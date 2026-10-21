//! OSC (UDP) : TouchOSC, QLab, une surface ou un autre logiciel pilote la console.
//!
//! ```text
//! /folie/exec/3/fader 0.75     fader de la place 3 de la page en cours (0 à 1)
//! /folie/exec/2.5/go           executor 5 de la page 2 ; aussi back, off, press
//! /folie/exec/3/flash 1        Flash enfoncé (1) ou relâché (0)
//! /folie/go   /folie/back      executor sélectionné
//! /folie/page 2   /folie/page/next   /folie/page/prev
//! /folie/blackout 1            1, 0, ou sans valeur pour basculer
//! /folie/offall   /folie/group 3   /folie/cmd "Fixture 1 At 50"
//! ```
//!
//! Une touche (go, back, off, press…) agit à l'appui : une valeur nulle (le relâchement
//! d'un bouton TouchOSC) est ignorée. Les clients reçoivent en retour, sur leur port de
//! retour, les mêmes adresses : `/folie/exec/N/fader`, `/name`, `/cue`, `/on`, et
//! `/folie/page`, `/folie/page/name`, `/folie/blackout`.
//!
//! Reaper (surface de contrôle OSC, fichier « Default ») annonce le marqueur qu'il vient de
//! passer : `/lastmarker/number/str "3"`. Si on le demande, le marqueur 3 fait Goto Cue 3
//! sur l'executor sélectionné : le spectacle se cale sur la timeline de Reaper.
//!
//! Pas de code : réseau local uniquement, et une liste d'adresses autorisées si l'on veut.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use folie_control::command::ExecutorRef;
use folie_control::{CommandError, Console, Outcome};
use folie_engine::Snapshot;
use folie_show::SLOTS_PER_PAGE;
use rosc::{OscMessage, OscPacket, OscType};

use crate::{Action, access, state::FADER_SLOTS};

/// Un client qui ne dit plus rien depuis ce temps ne reçoit plus le retour d'état.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(300);
/// Messages par paquet de retour : une vingtaine d'adresses courtes, bien sous les 1 500 octets.
const BUNDLE_MESSAGES: usize = 20;

/// Executor désigné par une adresse OSC : « 3 » (page en cours) ou « 2.5 ».
fn executor_ref(text: &str) -> Option<ExecutorRef> {
    let (page, slot) = match text.split_once('.') {
        Some((page, slot)) => (Some(page.parse().ok()?), slot.parse().ok()?),
        None => (None, text.parse().ok()?),
    };
    (page != Some(0) && (1..=SLOTS_PER_PAGE).contains(&slot)).then_some(ExecutorRef { page, slot })
}

fn number(arg: Option<&OscType>) -> Option<f32> {
    match arg? {
        OscType::Float(f) => Some(*f),
        OscType::Double(d) => Some(*d as f32),
        OscType::Int(i) => Some(*i as f32),
        OscType::Long(l) => Some(*l as f32),
        OscType::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Ce que demande un message, avant de savoir quelle page est affichée.
#[derive(Debug, Clone, PartialEq)]
enum Command {
    Exec { executor: ExecutorRef, key: Key },
    Go,
    Back,
    OffAll,
    Page(u32),
    PageStep(i32),
    Blackout(Option<bool>),
    Group(u32),
    Line(String),
    /// Marqueur passé par Reaper (son numéro).
    Marker(String),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Key {
    Fader(f32),
    Go,
    Back,
    Off,
    Press,
    Flash(bool),
}

/// Lit un message ; `None` s'il ne veut rien dire ou si c'est le relâchement d'une touche.
fn parse(message: &OscMessage) -> Option<Command> {
    let parts: Vec<&str> = message.addr.trim_start_matches('/').split('/').collect();
    let arg = message.args.first();
    // Touche : agit à l'appui, pas au relâchement.
    let pressed = number(arg).is_none_or(|v| v > 0.0);
    match parts.as_slice() {
        ["folie", "exec", n, what] => {
            let executor = executor_ref(n)?;
            let key = match *what {
                "fader" => Key::Fader(number(arg)?.clamp(0.0, 1.0)),
                "flash" => Key::Flash(pressed),
                "go" if pressed => Key::Go,
                "back" if pressed => Key::Back,
                "off" if pressed => Key::Off,
                "press" if pressed => Key::Press,
                _ => return None,
            };
            Some(Command::Exec { executor, key })
        }
        ["folie", "go"] => pressed.then_some(Command::Go),
        ["folie", "back"] => pressed.then_some(Command::Back),
        ["folie", "offall"] => pressed.then_some(Command::OffAll),
        ["folie", "page"] => Some(Command::Page(number(arg)?.round().max(1.0) as u32)),
        ["folie", "page", "next"] => pressed.then_some(Command::PageStep(1)),
        ["folie", "page", "prev"] => pressed.then_some(Command::PageStep(-1)),
        ["folie", "blackout"] => Some(Command::Blackout(number(arg).map(|v| v > 0.0))),
        ["folie", "group"] => Some(Command::Group(number(arg)?.round() as u32)),
        ["folie", "cmd"] => match arg? {
            OscType::String(line) => Some(Command::Line(line.clone())),
            _ => None,
        },
        ["lastmarker", "number", "str"] | ["lastmarker", "number"] => {
            let number = match arg? {
                OscType::String(text) => text.trim().to_string(),
                other => number(Some(other))?.round().to_string(),
            };
            // Hors des marqueurs, Reaper envoie un texte vide.
            (!number.is_empty()).then_some(Command::Marker(number))
        }
        _ => None,
    }
}

impl Command {
    /// `None` : marqueur Reaper alors qu'on ne les suit pas.
    fn into_action(self, console: &Console, markers: bool) -> Option<Action> {
        Some(match self {
            Self::Exec { executor, key } => {
                let executor = console.resolve_executor(executor);
                match key {
                    Key::Fader(level) => Action::Fader { executor, level },
                    Key::Go => Action::Go { executor: Some(executor) },
                    Key::Back => Action::Back { executor: Some(executor) },
                    Key::Off => Action::Off { executor },
                    Key::Press => Action::Press { executor },
                    Key::Flash(pressed) => Action::Flash { executor, pressed },
                }
            }
            Self::Go => Action::Go { executor: None },
            Self::Back => Action::Back { executor: None },
            Self::OffAll => Action::OffAll,
            Self::Page(page) => Action::Page { page },
            Self::PageStep(step) => Action::Page { page: console.page().saturating_add_signed(step).max(1) },
            Self::Blackout(on) => Action::Blackout { on: on.unwrap_or(!console.engine().snapshot().blackout) },
            Self::Group(number) => Action::Group { number },
            Self::Line(line) => Action::Command { line },
            Self::Marker(number) if markers => Action::Command { line: format!("Goto Cue {number}") },
            Self::Marker(_) => return None,
        })
    }
}

fn messages(packet: OscPacket, out: &mut Vec<OscMessage>) {
    match packet {
        OscPacket::Message(m) => out.push(m),
        OscPacket::Bundle(b) => b.content.into_iter().for_each(|p| messages(p, out)),
    }
}

/// Clients connus : adresse d'envoi et dernier message reçu.
type Clients = Arc<Mutex<HashMap<IpAddr, (SocketAddr, Instant)>>>;

/// Serveur OSC en marche ; il s'arrête quand on le lâche.
pub struct OscServer {
    port: u16,
    socket: UdpSocket,
    /// Port où les clients attendent le retour d'état ; 0 : celui d'où ils écrivent.
    reply_port: u16,
    requests: Receiver<Command>,
    /// Les marqueurs de Reaper font Goto Cue.
    markers: bool,
    /// Dernier marqueur suivi : Reaper peut le redire sans qu'on reparte.
    last_marker: Mutex<Option<String>>,
    clients: Clients,
    /// Dernières valeurs envoyées, pour n'envoyer que ce qui change.
    sent: Mutex<HashMap<String, OscType>>,
    /// Un nouveau client est arrivé : tout lui est renvoyé.
    resend: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl OscServer {
    /// Écoute en UDP sur `port` (0 : au choix du système). `allowed` : adresses autorisées
    /// (vide : tout le réseau local). `markers` : suivre les marqueurs de Reaper.
    /// `waker` est appelé à chaque message reçu.
    pub fn start(
        port: u16,
        reply_port: u16,
        allowed: Vec<IpAddr>,
        markers: bool,
        waker: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, port))?;
        socket.set_read_timeout(Some(Duration::from_millis(200)))?;
        let port = socket.local_addr()?.port();
        let receiver = socket.try_clone()?;
        let (tx, requests) = crossbeam_channel::unbounded();
        let clients = Clients::default();
        let stop = Arc::new(AtomicBool::new(false));
        let resend = Arc::new(AtomicBool::new(false));
        let thread = {
            let (clients, stop, resend) = (clients.clone(), stop.clone(), resend.clone());
            std::thread::Builder::new().name("folie-osc".into()).spawn(move || {
                let mut buffer = [0u8; rosc::decoder::MTU];
                while !stop.load(Ordering::Relaxed) {
                    let Ok((size, from)) = receiver.recv_from(&mut buffer) else { continue };
                    let ip = crate::canonical(from.ip());
                    if !access::is_local(ip) || !(allowed.is_empty() || allowed.contains(&ip)) {
                        log::warn!("OSC : {ip} refusé");
                        continue;
                    }
                    let Ok((_, packet)) = rosc::decoder::decode_udp(&buffer[..size]) else {
                        log::warn!("OSC : paquet illisible de {ip}");
                        continue;
                    };
                    let reply_to = if reply_port == 0 { from } else { SocketAddr::new(ip, reply_port) };
                    let known = clients.lock().unwrap_or_else(|e| e.into_inner()).insert(ip, (reply_to, Instant::now()));
                    if known.is_none() {
                        log::info!("OSC : {ip} connecté");
                        resend.store(true, Ordering::Relaxed);
                    }
                    let mut list = Vec::new();
                    messages(packet, &mut list);
                    for message in list {
                        match parse(&message) {
                            Some(command) => {
                                let _ = tx.send(command);
                            }
                            None => log::debug!("OSC : {} ignoré", message.addr),
                        }
                    }
                    waker();
                }
            })?
        };
        log::info!("OSC sur le port {port}");
        Ok(Self {
            port,
            socket,
            reply_port,
            requests,
            markers,
            last_marker: Mutex::default(),
            clients,
            sent: Mutex::default(),
            resend,
            stop,
            thread: Some(thread),
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn reply_port(&self) -> u16 {
        self.reply_port
    }

    /// Clients entendus ces dernières minutes.
    pub fn clients(&self) -> usize {
        let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        clients.retain(|_, (_, seen)| seen.elapsed() < CLIENT_TIMEOUT);
        clients.len()
    }

    /// Applique les messages arrivés depuis la dernière image (comme la télécommande web).
    pub fn handle(&self, console: &mut Console) -> Vec<Result<Outcome, CommandError>> {
        let mut results = Vec::new();
        for command in self.requests.try_iter() {
            if let Command::Marker(number) = &command {
                let mut last = self.last_marker.lock().unwrap_or_else(|e| e.into_inner());
                if last.as_ref() == Some(number) {
                    continue;
                }
                *last = Some(number.clone());
            }
            let Some(action) = command.into_action(console, self.markers) else { continue };
            let quiet = action.quiet();
            let result = action.apply(console);
            if !(quiet && result.is_ok()) {
                results.push(result);
            }
        }
        results
    }

    /// Retour d'état : envoie aux clients ce qui a changé depuis le dernier envoi
    /// (tout, quand un nouveau client arrive).
    pub fn publish(&self, console: &Console, snapshot: &Snapshot) {
        let targets: Vec<SocketAddr> = {
            let mut clients = self.clients.lock().unwrap_or_else(|e| e.into_inner());
            clients.retain(|_, (_, seen)| seen.elapsed() < CLIENT_TIMEOUT);
            clients.values().map(|(addr, _)| *addr).collect()
        };
        if targets.is_empty() {
            return;
        }
        let mut sent = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        if self.resend.swap(false, Ordering::Relaxed) {
            sent.clear();
        }
        let mut changed = Vec::new();
        for (addr, value) in feedback(console, snapshot) {
            if sent.get(&addr) != Some(&value) {
                sent.insert(addr.clone(), value.clone());
                changed.push(OscPacket::Message(OscMessage { addr, args: vec![value] }));
            }
        }
        if changed.is_empty() {
            return;
        }
        // Paquets groupés assez petits pour tenir dans une trame réseau (et dans le
        // tampon de réception des clients).
        for chunk in changed.chunks(BUNDLE_MESSAGES) {
            let bundle = rosc::OscBundle { timetag: rosc::OscTime { seconds: 0, fractional: 1 }, content: chunk.to_vec() };
            let Ok(bytes) = rosc::encoder::encode(&OscPacket::Bundle(bundle)) else { continue };
            for addr in &targets {
                if let Err(err) = self.socket.send_to(&bytes, addr) {
                    log::warn!("OSC : retour vers {addr} impossible ({err})");
                }
            }
        }
    }

}

impl Drop for OscServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Valeurs du retour d'état, par adresse.
fn feedback(console: &Console, snapshot: &Snapshot) -> Vec<(String, OscType)> {
    let mut out = Vec::new();
    for slot in 1..=SLOTS_PER_PAGE {
        let id = console.executor_at(slot);
        let status = snapshot.executors.get(&id);
        let (name, cue) = match console.executor(id) {
            Some((_, sequence)) => {
                let cue = status.and_then(|s| s.cue).and_then(|i| sequence.cues.iter().nth(i)).map_or_else(
                    || "éteint".to_string(),
                    |(n, c)| if c.name.is_empty() { n.to_string() } else { format!("{n} {}", c.name) },
                );
                (sequence.name.clone(), cue)
            }
            None => match console.executor_macro(id) {
                Some((number, m)) => (if m.name.is_empty() { format!("Macro {number}") } else { m.name.clone() }, String::new()),
                None => (String::new(), String::new()),
            },
        };
        let base = format!("/folie/exec/{slot}");
        if slot <= FADER_SLOTS {
            out.push((format!("{base}/fader"), OscType::Float(status.map_or(1.0, |s| s.level))));
        }
        out.push((format!("{base}/name"), OscType::String(name)));
        out.push((format!("{base}/cue"), OscType::String(cue)));
        out.push((format!("{base}/on"), OscType::Int(status.is_some_and(|s| s.cue.is_some()) as i32)));
    }
    let page = console.page();
    out.push(("/folie/page".into(), OscType::Int(page as i32)));
    out.push(("/folie/page/name".into(), OscType::String(console.page_name(page).to_string())));
    out.push(("/folie/blackout".into(), OscType::Int(snapshot.blackout as i32)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(addr: &str, args: Vec<OscType>) -> OscMessage {
        OscMessage { addr: addr.into(), args }
    }

    #[test]
    fn addresses() {
        let exec = |page, slot| ExecutorRef { page, slot };
        assert_eq!(
            parse(&message("/folie/exec/3/fader", vec![OscType::Float(1.5)])),
            Some(Command::Exec { executor: exec(None, 3), key: Key::Fader(1.0) })
        );
        assert_eq!(
            parse(&message("/folie/exec/2.5/go", vec![])),
            Some(Command::Exec { executor: exec(Some(2), 5), key: Key::Go })
        );
        assert_eq!(parse(&message("/folie/exec/3/go", vec![OscType::Float(0.0)])), None, "relâchement ignoré");
        assert_eq!(
            parse(&message("/folie/exec/3/flash", vec![OscType::Int(0)])),
            Some(Command::Exec { executor: exec(None, 3), key: Key::Flash(false) })
        );
        assert_eq!(parse(&message("/folie/exec/21/go", vec![])), None, "20 places par page");
        assert_eq!(parse(&message("/folie/page", vec![OscType::Int(2)])), Some(Command::Page(2)));
        assert_eq!(parse(&message("/folie/blackout", vec![])), Some(Command::Blackout(None)));
        assert_eq!(
            parse(&message("/folie/cmd", vec![OscType::String("Go".into())])),
            Some(Command::Line("Go".into()))
        );
        assert_eq!(parse(&message("/autre/chose", vec![])), None);
        let marker = |arg| parse(&message("/lastmarker/number/str", vec![arg]));
        assert_eq!(marker(OscType::String("3".into())), Some(Command::Marker("3".into())));
        assert_eq!(marker(OscType::String("".into())), None, "hors des marqueurs");
    }
}

//! Télécommande web : un téléphone ou une tablette du réseau local pilote la console.
//! L'OSC ([`osc`]) passe par les mêmes actions.
//!
//! Le serveur (axum, sur son propre fil) sert une page embarquée dans le binaire et
//! échange en WebSocket. Il ne touche jamais la console : les demandes des téléphones
//! arrivent par un canal et l'interface les applique à chaque image ([`RemoteServer::handle`]),
//! puis publie l'état ([`RemoteServer::publish`]), que le serveur pousse aux téléphones.
//!
//! Accès : réseau local uniquement, code PIN demandé à la connexion, et une adresse qui
//! se trompe trop souvent est mise à l'écart une minute.

mod access;
pub mod midi;
pub mod osc;
pub mod state;

use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread::JoinHandle;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State as Shared};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use crossbeam_channel::{Receiver, Sender};
use folie_control::{CommandError, Console, Outcome};
use folie_engine::{Command as EngineCommand, Snapshot};
use serde::Deserialize;
use tokio::sync::{oneshot, watch};

use access::Access;
pub use access::Role;

const PAGE: &str = include_str!("web/index.html");

/// Ce qu'un téléphone demande à la console.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    /// Une ligne de commande, comme au clavier de la console.
    Command {
        line: String,
    },
    /// Fader d'un executor, de 0 à 1.
    Fader {
        executor: u32,
        level: f32,
    },
    /// Touche principale de l'executor, selon son mode.
    Press {
        executor: u32,
    },
    /// Touche Flash (ou Swop) enfoncée ou relâchée.
    Flash {
        executor: u32,
        pressed: bool,
    },
    Go {
        executor: Option<u32>,
    },
    Back {
        executor: Option<u32>,
    },
    Off {
        executor: u32,
    },
    OffAll,
    Page {
        page: u32,
    },
    Group {
        number: u32,
    },
    Blackout {
        on: bool,
    },
    /// Goto Cue sur un executor (`None` : le sélectionné) ; numéro tel qu'écrit (« 3 », « 2.5 »).
    Goto {
        executor: Option<u32>,
        cue: String,
    },
}

impl Action {
    /// Avec le code « lecture », on ne fait que jouer les executors.
    pub fn allowed(&self, role: Role) -> bool {
        match role {
            Role::Full => true,
            Role::Playback => {
                matches!(self, Self::Press { .. } | Self::Flash { .. } | Self::Go { .. } | Self::Back { .. })
            }
        }
    }

    /// Gestes continus : on ne les annonce pas sur la ligne de commande de la console.
    fn quiet(&self) -> bool {
        matches!(self, Self::Fader { .. } | Self::Flash { .. })
    }

    fn apply(self, console: &mut Console) -> Result<Outcome, CommandError> {
        match self {
            Self::Command { line } => console.execute(&line),
            Self::Fader { executor, level } => console.set_fader(executor, level.clamp(0.0, 1.0) * 100.0),
            Self::Press { executor } => console.press(executor),
            Self::Flash { executor, pressed } => {
                console.flash(executor, pressed);
                Ok(info(format!("Flash executor {}", folie_show::executor_label(executor))))
            }
            Self::Go { executor } => console.go(executor),
            Self::Back { executor } => console.back(executor),
            Self::Off { executor } => console.off(Some(executor)),
            Self::OffAll => Ok(console.off_all()),
            Self::Page { page } => Ok(console.set_page(page.clamp(1, folie_show::MAX_PAGE))),
            Self::Group { number } => console.execute(&format!("Group {number}")),
            Self::Blackout { on } => {
                console.engine().send(EngineCommand::Blackout(on));
                Ok(info(if on { "Blackout" } else { "Fin du blackout" }))
            }
            Self::Goto { executor, cue } => match cue.parse() {
                Ok(number) => console.goto_on(executor, number),
                Err(_) => Err(CommandError::from(folie_control::command::SyntaxError(format!("numéro de cue illisible : {cue}")))),
            },
        }
    }
}

fn info(message: impl Into<String>) -> Outcome {
    Outcome { message: message.into(), show_changed: false }
}

/// Réponse renvoyée au seul téléphone qui a demandé.
#[derive(Debug, Clone, serde::Serialize)]
struct Reply {
    #[serde(rename = "type")]
    kind: &'static str,
    ok: bool,
    message: String,
}

struct Request {
    action: Action,
    reply: Option<oneshot::Sender<Reply>>,
}

/// Ce que partagent les connexions du serveur.
struct Hub {
    requests: Sender<Request>,
    state: watch::Receiver<Arc<str>>,
    stop: watch::Receiver<bool>,
    access: Access,
    clients: Arc<AtomicUsize>,
    waker: Box<dyn Fn() + Send + Sync>,
}

/// Serveur en marche ; il s'arrête quand on le lâche.
pub struct RemoteServer {
    addr: SocketAddr,
    requests: Receiver<Request>,
    state: watch::Sender<Arc<str>>,
    stop: watch::Sender<bool>,
    clients: Arc<AtomicUsize>,
    thread: Option<JoinHandle<()>>,
}

impl RemoteServer {
    /// Écoute sur toutes les interfaces IPv4, port `port` (0 : au choix du système).
    /// `full_pin` donne tout ; `playback_pin` ne permet que GO, Back et les touches des executors.
    /// `waker` est appelé à chaque demande d'un téléphone, pour réveiller l'interface.
    pub fn start(
        port: u16,
        full_pin: &str,
        playback_pin: &str,
        waker: impl Fn() + Send + Sync + 'static,
    ) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::UNSPECIFIED, port))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let (requests_tx, requests) = crossbeam_channel::unbounded();
        let (state, state_rx) = watch::channel(Arc::<str>::from(""));
        let (stop, stop_rx) = watch::channel(false);
        let clients = Arc::new(AtomicUsize::new(0));
        let hub = Arc::new(Hub {
            requests: requests_tx,
            state: state_rx,
            stop: stop_rx.clone(),
            access: Access::new(full_pin, playback_pin),
            clients: clients.clone(),
            waker: Box::new(waker),
        });
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        let thread = std::thread::Builder::new().name("folie-remote".into()).spawn(move || {
            runtime.block_on(async move {
                let listener = match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(err) => return log::error!("télécommande : {err}"),
                };
                let app = axum::Router::new()
                    .route("/", get(page))
                    .route("/ws", get(socket))
                    .with_state(hub)
                    .into_make_service_with_connect_info::<SocketAddr>();
                let mut stop = stop_rx;
                let shutdown = async move { stopped(&mut stop).await };
                if let Err(err) = axum::serve(listener, app).with_graceful_shutdown(shutdown).await {
                    log::error!("télécommande : {err}");
                }
            });
            runtime.shutdown_timeout(std::time::Duration::from_millis(300));
        })?;
        log::info!("télécommande web sur le port {}", addr.port());
        Ok(Self { addr, requests, state, stop, clients, thread: Some(thread) })
    }

    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Téléphones connectés (PIN accepté).
    pub fn clients(&self) -> usize {
        self.clients.load(Ordering::Relaxed)
    }

    /// Applique les demandes arrivées depuis la dernière image. Renvoie ce qui mérite
    /// d'être annoncé sur la console (les mouvements de fader ne le sont pas, sauf erreur).
    pub fn handle(&self, console: &mut Console) -> Vec<Result<Outcome, CommandError>> {
        let mut results = Vec::new();
        for request in self.requests.try_iter() {
            let quiet = request.action.quiet();
            let result = request.action.apply(console);
            if let Some(reply) = request.reply {
                let (ok, message) = match &result {
                    Ok(outcome) => (true, outcome.message.clone()),
                    Err(err) => (false, err.to_string()),
                };
                let _ = reply.send(Reply { kind: "reply", ok, message });
            }
            if !(quiet && result.is_ok()) {
                results.push(result);
            }
        }
        results
    }

    /// Publie l'état pour les téléphones (seulement s'il a changé et que quelqu'un écoute).
    pub fn publish(&self, console: &Console, snapshot: &Snapshot, show_name: &str) {
        if self.clients() == 0 {
            return;
        }
        let json = serde_json::to_string(&state::build(console, snapshot, show_name)).unwrap_or_default();
        self.state.send_if_modified(|current| {
            if **current == *json {
                return false;
            }
            *current = Arc::from(json);
            true
        });
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Rend la main quand le serveur s'arrête.
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|s| *s).await;
}

async fn page() -> Response {
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-store")], PAGE).into_response()
}

async fn socket(
    ws: WebSocketUpgrade,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Query(query): Query<HashMap<String, String>>,
    Shared(hub): Shared<Arc<Hub>>,
) -> Response {
    let ip = canonical(peer.ip());
    if !access::is_local(ip) {
        log::warn!("télécommande : {ip} refusé (hors du réseau local)");
        return (StatusCode::FORBIDDEN, "réseau local uniquement").into_response();
    }
    let pin = query.get("pin").cloned().unwrap_or_default();
    ws.on_upgrade(move |mut socket| async move {
        // Le refus passe par la WebSocket : un navigateur ne voit pas le code d'une
        // réponse HTTP à une demande de WebSocket.
        let role = match hub.access.check(ip, &pin) {
            Ok(role) => role,
            Err(refusal) => {
                // Une tentative toutes les demi-secondes au plus, même en insistant.
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let json = serde_json::json!({ "type": "refused", "reason": refusal.reason, "message": refusal.message });
                let _ = socket.send(Message::Text(json.to_string().into())).await;
                let _ = socket.send(Message::Close(None)).await;
                return;
            }
        };
        connection(socket, hub, ip, role).await
    })
}

/// Adresse IPv4 vue à travers une socket IPv6 (::ffff:192.168.1.20).
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
        v4 => v4,
    }
}

async fn connection(mut socket: WebSocket, hub: Arc<Hub>, ip: IpAddr, role: Role) {
    log::info!("télécommande : {ip} connecté ({role:?})");
    // La page n'affiche que ce que le code permet.
    let hello = serde_json::json!({ "type": "hello", "role": role }).to_string();
    if socket.send(Message::Text(hello.into())).await.is_err() {
        return;
    }
    hub.clients.fetch_add(1, Ordering::Relaxed);
    (hub.waker)();
    let mut state = hub.state.clone();
    let mut stop = hub.stop.clone();
    // Touches Flash tenues par ce téléphone : relâchées s'il disparaît.
    let mut held = BTreeSet::new();
    let mut pending: Vec<oneshot::Receiver<Reply>> = Vec::new();
    state.mark_changed();
    loop {
        tokio::select! {
            changed = state.changed() => {
                if changed.is_err() { break }
                let json = state.borrow_and_update().clone();
                // Rien de publié encore : l'interface n'a pas tourné depuis la connexion.
                if json.is_empty() { continue }
                if socket.send(Message::Text(json.as_ref().into())).await.is_err() { break }
            }
            message = socket.recv() => {
                let Some(Ok(message)) = message else { break };
                let Message::Text(text) = message else { continue };
                let action = match serde_json::from_str::<Action>(&text) {
                    Ok(action) => action,
                    Err(err) => {
                        log::warn!("télécommande : message illisible ({err})");
                        continue;
                    }
                };
                if !action.allowed(role) {
                    let reply = Reply { kind: "reply", ok: false, message: "Code lecture : réservé au code régie".into() };
                    let json = serde_json::to_string(&reply).unwrap_or_default();
                    if socket.send(Message::Text(json.into())).await.is_err() { break }
                    continue;
                }
                if let Action::Flash { executor, pressed } = action {
                    if pressed { held.insert(executor); } else { held.remove(&executor); }
                }
                let (reply, answer) = oneshot::channel();
                if hub.requests.send(Request { action, reply: Some(reply) }).is_err() { break }
                pending.push(answer);
                (hub.waker)();
            }
            _ = stopped(&mut stop) => break,
            _ = tokio::time::sleep(std::time::Duration::from_millis(20)), if !pending.is_empty() => {}
        }
        // Réponses de la console déjà prêtes, dans l'ordre des demandes.
        while let Some(first) = pending.first_mut() {
            match first.try_recv() {
                Ok(reply) => {
                    pending.remove(0);
                    let json = serde_json::to_string(&reply).unwrap_or_default();
                    if socket.send(Message::Text(json.into())).await.is_err() {
                        break;
                    }
                }
                Err(oneshot::error::TryRecvError::Empty) => break,
                Err(oneshot::error::TryRecvError::Closed) => {
                    pending.remove(0);
                }
            }
        }
    }
    for executor in held {
        let _ = hub.requests.send(Request { action: Action::Flash { executor, pressed: false }, reply: None });
    }
    hub.clients.fetch_sub(1, Ordering::Relaxed);
    (hub.waker)();
    log::info!("télécommande : {ip} déconnecté");
}

#[cfg(test)]
mod tests;

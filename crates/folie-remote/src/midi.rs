//! MIDI : un contrôleur (faders, boutons), un séquenceur (Reaper…) ou une régie pilote la
//! console, et Folie renvoie l'état au contrôleur (LED, faders motorisés).
//!
//! - Notes et CC sont associés à des actions (« apprentissage » : on touche la commande,
//!   elle est associée). Une note ou un CC ≥ 64 appuie, un relâchement (note off, CC < 64)
//!   relâche : utile au Flash.
//! - MIDI Show Control (MSC) : GO, STOP, RESUME, ALL_OFF… La liste de cues désigne
//!   l'executor (liste 2 : executor 2) ; sans liste, l'executor sélectionné.
//! - MIDI Timecode (MTC) : le temps reçu est tenu à jour, pour caler des cues dessus.
//!
//! Les ports sont ouverts par `midir` (CoreMIDI sur macOS, WinMM sous Windows ; un port
//! virtuel comme loopMIDI relie un séquenceur sous Windows).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crossbeam_channel::{Receiver, Sender, unbounded};
use folie_control::command::ExecutorRef;
use folie_control::{CommandError, Console, Outcome};
use folie_engine::Snapshot;
use serde::{Deserialize, Serialize};

use crate::Action;

/// Message MIDI reçu, déjà lu.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// Note : vélocité 0 = relâchée.
    Note { channel: u8, note: u8, velocity: u8 },
    Cc { channel: u8, number: u8, value: u8 },
    /// Message système exclusif complet (F0 … F7).
    SysEx(Vec<u8>),
    /// Quart d'image MTC (F1).
    QuarterFrame(u8),
}

/// Lit un message MIDI brut.
pub fn decode(bytes: &[u8]) -> Option<Event> {
    let status = *bytes.first()?;
    let channel = status & 0x0F;
    match status & 0xF0 {
        0x90 => Some(Event::Note { channel, note: *bytes.get(1)? & 0x7F, velocity: *bytes.get(2)? & 0x7F }),
        0x80 => Some(Event::Note { channel, note: *bytes.get(1)? & 0x7F, velocity: 0 }),
        0xB0 => Some(Event::Cc { channel, number: *bytes.get(1)? & 0x7F, value: *bytes.get(2)? & 0x7F }),
        0xF0 if status == 0xF0 => Some(Event::SysEx(bytes.to_vec())),
        0xF0 if status == 0xF1 => Some(Event::QuarterFrame(*bytes.get(1)?)),
        _ => None,
    }
}

/// Une commande du contrôleur : une note ou un CC, sur un canal (0 à 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Control {
    Note { channel: u8, number: u8 },
    Cc { channel: u8, number: u8 },
}

impl Control {
    pub fn label(&self) -> String {
        match self {
            Self::Note { channel, number } => format!("Note {} ({}) · canal {}", number, note_name(*number), channel + 1),
            Self::Cc { channel, number } => format!("CC {number} · canal {}", channel + 1),
        }
    }
}

fn note_name(note: u8) -> String {
    const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    format!("{}{}", NAMES[note as usize % 12], note as i32 / 12 - 1)
}

/// Ce que fait une commande. `slot` : place sur la page d'executors affichée (1 à 15).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Target {
    Fader { slot: u32 },
    Press { slot: u32 },
    Go { slot: u32 },
    Back { slot: u32 },
    Off { slot: u32 },
    Flash { slot: u32 },
    MainGo,
    MainBack,
    OffAll,
    PageNext,
    PagePrev,
    /// Encodeur sans fin (valeur relative autour de 64) : un cran à droite, page suivante.
    PageEncoder,
    Blackout,
    Group { number: u32 },
    Command { line: String },
}

impl Target {
    pub fn label(&self) -> String {
        match self {
            Self::Fader { slot } => format!("Fader de l'executor {slot}"),
            Self::Press { slot } => format!("Touche de l'executor {slot}"),
            Self::Go { slot } => format!("Go executor {slot}"),
            Self::Back { slot } => format!("Back executor {slot}"),
            Self::Off { slot } => format!("Off executor {slot}"),
            Self::Flash { slot } => format!("Flash executor {slot}"),
            Self::MainGo => "Go (executor sélectionné)".into(),
            Self::MainBack => "Back (executor sélectionné)".into(),
            Self::OffAll => "Tout éteindre".into(),
            Self::PageNext => "Page suivante".into(),
            Self::PagePrev => "Page précédente".into(),
            Self::PageEncoder => "Page (encodeur)".into(),
            Self::Blackout => "Blackout (bascule)".into(),
            Self::Group { number } => format!("Groupe {number}"),
            Self::Command { line } => format!("Commande « {line} »"),
        }
    }

    /// Place d'executor visée, pour les cibles qui en ont une.
    pub fn slot(&self) -> Option<u32> {
        match self {
            Self::Fader { slot } | Self::Press { slot } | Self::Go { slot } | Self::Back { slot } | Self::Off { slot } | Self::Flash { slot } => {
                Some(*slot)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mapping {
    pub control: Control,
    pub target: Target,
}

/// Réglages MIDI, gardés avec ceux de la machine (ils suivent le contrôleur, pas le show).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiSettings {
    pub enabled: bool,
    /// Ports d'entrée écoutés ; vide : tous.
    pub inputs: Vec<String>,
    /// Port de retour vers le contrôleur (LED, faders motorisés) ; vide : aucun.
    pub output: String,
    pub mappings: Vec<Mapping>,
    /// MIDI Show Control reçu.
    pub msc: bool,
    /// Identifiant MSC de Folie (0 à 126) ; 127 : répond à tous.
    pub msc_device: u8,
    /// Façon d'allumer les boutons du contrôleur.
    pub leds: Leds,
}

/// Retour sur les boutons : une note ou un CC (le plus courant), ou le message propre à
/// un contrôleur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Leds {
    #[default]
    Standard,
    /// Pads RGB du MiniLab 3 : couleur par message système Arturia.
    MiniLab3,
}

/// Couleur d'un pad du MiniLab 3 : ambre quand l'executor joue, veilleuse sinon.
fn minilab3_pad(note: u8, on: bool) -> Option<Vec<u8>> {
    let pad = match note {
        36..=43 => 0x34 + (note - 36),
        44..=51 => 0x44 + (note - 44),
        _ => return None,
    };
    let [r, g, b] = if on { [127, 70, 0] } else { [6, 4, 0] };
    Some(vec![0xF0, 0x00, 0x20, 0x6B, 0x7F, 0x42, 0x02, 0x01, 0x16, pad, r, g, b, 0xF7])
}

/// Message envoyé au contrôleur pour une valeur de retour.
fn feedback_message(leds: Leds, control: Control, value: u8) -> Vec<u8> {
    match (leds, control) {
        (Leds::MiniLab3, Control::Note { channel: 9, number }) => {
            if let Some(sysex) = minilab3_pad(number, value > 0) {
                return sysex;
            }
            vec![0x99, number, value]
        }
        (_, Control::Note { channel, number }) => vec![0x90 | channel, number, value],
        (_, Control::Cc { channel, number }) => vec![0xB0 | channel, number, value],
    }
}

impl Default for MidiSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            inputs: Vec::new(),
            output: String::new(),
            mappings: Vec::new(),
            msc: true,
            msc_device: 127,
            leds: Leds::Standard,
        }
    }
}

/// Profil d'un contrôleur : ses associations toutes prêtes.
pub struct Profile {
    pub name: &'static str,
    /// Ce qu'il faut savoir (réglages du contrôleur, limites).
    pub note: &'static str,
    pub mappings: Vec<Mapping>,
    pub leds: Leds,
}

/// Profils fournis. Les numéros sont ceux des réglages d'usine ; si un contrôleur a été
/// reconfiguré, « Apprendre » corrige une ligne.
pub fn profiles() -> Vec<Profile> {
    let cc = |number| Control::Cc { channel: 0, number };
    let pad = |number| Control::Note { channel: 9, number };
    let mut minilab = Vec::new();
    // Faders 1 à 4 : executors 1 à 4.
    for (slot, number) in (1..).zip([82, 83, 85, 17]) {
        minilab.push(Mapping { control: cc(number), target: Target::Fader { slot } });
    }
    // Encodeurs 1 à 6 : faders des executors 5 à 10 (les 7 et 8 restent libres).
    for (slot, number) in (5..).zip([74, 71, 76, 77, 93, 18]) {
        minilab.push(Mapping { control: cc(number), target: Target::Fader { slot } });
    }
    // Encodeur principal (tourné) : les pages d'executors.
    minilab.push(Mapping { control: cc(114), target: Target::PageEncoder });
    // Pads, banque A : touches des executors 1 à 8 ; banque B : leur Flash.
    for slot in 1..=8u32 {
        minilab.push(Mapping { control: pad(35 + slot as u8), target: Target::Press { slot } });
    }
    for slot in 1..=8u32 {
        minilab.push(Mapping { control: pad(43 + slot as u8), target: Target::Flash { slot } });
    }
    // Clavier (canal 1), touches blanches de la première octave (Do 48 sans décalage
    // d'octave) : executors à boutons 11 à 15, puis Go et Back de l'executor sélectionné.
    let key = |number| Control::Note { channel: 0, number };
    for (slot, number) in (11..).zip([48, 50, 52, 53, 55]) {
        minilab.push(Mapping { control: key(number), target: Target::Press { slot } });
    }
    minilab.push(Mapping { control: key(57), target: Target::MainGo });
    minilab.push(Mapping { control: key(59), target: Target::MainBack });
    vec![Profile {
        name: "Arturia MiniLab 3",
        note: "Réglages d'usine, programme Arturia (celui par défaut) : faders 1-4 et encodeurs 1-6 \
               sur les faders des executors 1 à 10, encodeur principal sur les pages, pads banque A \
               sur leurs touches, banque B sur leur Flash ; clavier, touches blanches de la \
               première octave (Do à Sol) sur les executors 11 à 15, La : Go, Si : Back. Les pads s'allument en ambre quand leur executor joue (port de retour : \
               MiniLab 3).",
        mappings: minilab,
        leds: Leds::MiniLab3,
    }]
}

/// La commande d'un message et son état : appuyée (ou valeur de 0 à 1).
fn control_of(event: &Event) -> Option<(Control, f32)> {
    match *event {
        Event::Note { channel, note, velocity } => Some((Control::Note { channel, number: note }, velocity as f32 / 127.0)),
        Event::Cc { channel, number, value } => Some((Control::Cc { channel, number }, value as f32 / 127.0)),
        _ => None,
    }
}

/// Action d'une association pour une valeur reçue (0 à 1). `None` : rien à faire
/// (relâchement d'une touche qui n'agit qu'à l'appui).
fn action_for(target: &Target, value: f32, console: &Console) -> Option<Action> {
    let pressed = value >= 0.5;
    let exec = |slot: u32| console.resolve_executor(ExecutorRef { page: None, slot });
    if *target == Target::PageEncoder {
        // Valeur relative : 65 et plus, vers la droite ; 63 et moins, vers la gauche.
        let step = (value * 127.0).round() as i32 - 64;
        let page = console.page() as i32 + step.signum();
        return (step != 0).then_some(Action::Page { page: page.max(1) as u32 });
    }
    Some(match target {
        Target::Fader { slot } => Action::Fader { executor: exec(*slot), level: value },
        Target::Flash { slot } => Action::Flash { executor: exec(*slot), pressed },
        _ if !pressed => return None,
        Target::Press { slot } => Action::Press { executor: exec(*slot) },
        Target::Go { slot } => Action::Go { executor: Some(exec(*slot)) },
        Target::Back { slot } => Action::Back { executor: Some(exec(*slot)) },
        Target::Off { slot } => Action::Off { executor: exec(*slot) },
        Target::MainGo => Action::Go { executor: None },
        Target::MainBack => Action::Back { executor: None },
        Target::OffAll => Action::OffAll,
        Target::PageNext => Action::Page { page: console.page().saturating_add(1) },
        Target::PageEncoder => return None,
        Target::PagePrev => Action::Page { page: console.page().saturating_sub(1).max(1) },
        Target::Blackout => Action::Blackout { on: !console.engine().snapshot().blackout },
        Target::Group { number } => Action::Group { number: *number },
        Target::Command { line } => Action::Command { line: line.clone() },
    })
}

/// Commande MSC (F0 7F <appareil> 02 <format> <commande> <données> F7).
#[derive(Debug, Clone, PartialEq)]
pub enum Msc {
    Go { cue: Option<String>, list: Option<u32> },
    Stop { list: Option<u32> },
    Resume { list: Option<u32> },
    AllOff,
}

/// Lit un message MSC destiné à `device` (127 : tous).
pub fn decode_msc(bytes: &[u8], device: u8) -> Option<Msc> {
    let [0xF0, 0x7F, target, 0x02, _format, command, rest @ ..] = bytes else { return None };
    if *target != 0x7F && device != 0x7F && *target != device {
        return None;
    }
    let data = rest.strip_suffix(&[0xF7]).unwrap_or(rest);
    // Numéro de cue, liste, chemin : textes ASCII séparés par 00.
    let mut fields = data.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).trim().to_string());
    let cue = fields.next().filter(|c| !c.is_empty());
    let list = fields.next().and_then(|l| l.parse().ok());
    match command {
        0x01 => Some(Msc::Go { cue, list }),
        0x02 => Some(Msc::Stop { list }),
        0x03 => Some(Msc::Resume { list }),
        0x0A => Some(Msc::AllOff),
        _ => None,
    }
}

fn msc_action(msc: Msc) -> Action {
    match msc {
        Msc::Go { cue: Some(cue), list } => Action::Goto { executor: list, cue },
        Msc::Go { cue: None, list } => Action::Go { executor: list },
        // Pas de pause dans Folie : STOP éteint la liste, RESUME la relance.
        Msc::Stop { list: Some(list) } => Action::Off { executor: list },
        Msc::Stop { list: None } => Action::OffAll,
        Msc::Resume { list } => Action::Go { executor: list },
        Msc::AllOff => Action::OffAll,
    }
}

/// Timecode reçu, en images, avec sa cadence.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Timecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub fps: f32,
}

impl Timecode {
    pub fn seconds(&self) -> f64 {
        self.hours as f64 * 3600.0 + self.minutes as f64 * 60.0 + self.seconds as f64 + self.frames as f64 / self.fps.max(1.0) as f64
    }

    pub fn label(&self) -> String {
        format!("{:02}:{:02}:{:02}:{:02}", self.hours, self.minutes, self.seconds, self.frames)
    }

    fn from_parts(hours_and_rate: u8, minutes: u8, seconds: u8, frames: u8) -> Self {
        let fps = match (hours_and_rate >> 5) & 0x03 {
            0 => 24.0,
            1 => 25.0,
            2 => 29.97,
            _ => 30.0,
        };
        Self { hours: hours_and_rate & 0x1F, minutes: minutes & 0x3F, seconds: seconds & 0x3F, frames: frames & 0x1F, fps }
    }
}

/// Assemble le timecode à partir des huit quarts d'image, ou d'une image complète.
#[derive(Debug, Default)]
pub struct MtcReader {
    pieces: [u8; 8],
    seen: u8,
}

impl MtcReader {
    /// Quart d'image ; rend le timecode quand le huitième arrive.
    pub fn quarter(&mut self, data: u8) -> Option<Timecode> {
        let index = (data >> 4) as usize & 0x07;
        self.pieces[index] = data & 0x0F;
        self.seen |= 1 << index;
        if index != 7 || self.seen != 0xFF {
            return None;
        }
        self.seen = 0;
        let p = self.pieces;
        let mut tc = Timecode::from_parts(p[6] | (p[7] << 4), p[4] | (p[5] << 4), p[2] | (p[3] << 4), p[0] | (p[1] << 4));
        // Les huit quarts couvrent deux images : la position est celle de la première.
        tc.frames = tc.frames.saturating_add(2);
        Some(tc)
    }

    /// Image complète : F0 7F 7F 01 01 hh mm ss ff F7.
    pub fn full_frame(bytes: &[u8]) -> Option<Timecode> {
        let [0xF0, 0x7F, _, 0x01, 0x01, hh, mm, ss, ff, ..] = bytes else { return None };
        Some(Timecode::from_parts(*hh, *mm, *ss, *ff))
    }
}

/// Ports MIDI présents : (entrées, sorties).
pub fn ports() -> (Vec<String>, Vec<String>) {
    let inputs = midir::MidiInput::new("Folie-DMX")
        .map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
        .unwrap_or_default();
    let outputs = midir::MidiOutput::new("Folie-DMX")
        .map(|m| m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect())
        .unwrap_or_default();
    (inputs, outputs)
}

/// MIDI en marche : les ports s'ouvrent au démarrage et se ferment quand on le lâche.
pub struct MidiServer {
    _inputs: Vec<midir::MidiInputConnection<()>>,
    output: Mutex<Option<midir::MidiOutputConnection>>,
    events: Receiver<Event>,
    sender: Sender<Event>,
    settings: MidiSettings,
    /// Apprentissage en cours : le prochain message est rendu au lieu d'agir.
    learning: Mutex<bool>,
    mtc: Mutex<MtcReader>,
    timecode: Arc<Mutex<Option<Timecode>>>,
    /// Dernière valeur envoyée à chaque commande du contrôleur.
    sent: Mutex<HashMap<Control, u8>>,
    /// Ports ouverts, pour l'affichage.
    pub opened: Vec<String>,
}

/// Ce que le MIDI a donné à cette image.
#[derive(Default)]
pub struct Handled {
    pub results: Vec<Result<Outcome, CommandError>>,
    /// Commande touchée pendant l'apprentissage.
    pub learned: Option<Control>,
}

impl MidiServer {
    pub fn start(settings: &MidiSettings) -> Result<Self, String> {
        let (sender, events) = unbounded();
        let mut connections = Vec::new();
        let mut opened = Vec::new();
        let probe = midir::MidiInput::new("Folie-DMX").map_err(|e| format!("MIDI indisponible : {e}"))?;
        for port in probe.ports() {
            let Ok(name) = probe.port_name(&port) else { continue };
            if !settings.inputs.is_empty() && !settings.inputs.contains(&name) {
                continue;
            }
            let mut input = midir::MidiInput::new("Folie-DMX").map_err(|e| e.to_string())?;
            // Le SysEx (MSC, image MTC complète) et le timecode sont filtrés par défaut.
            input.ignore(midir::Ignore::ActiveSense);
            let tx = sender.clone();
            match input.connect(&port, "Folie-DMX", move |_, bytes, _| {
                if let Some(event) = decode(bytes) {
                    let _ = tx.send(event);
                }
            }, ()) {
                Ok(connection) => {
                    connections.push(connection);
                    opened.push(name);
                }
                Err(e) => log::warn!("port MIDI {name} : {e}"),
            }
        }
        let output = if settings.output.is_empty() {
            None
        } else {
            let out = midir::MidiOutput::new("Folie-DMX").map_err(|e| e.to_string())?;
            let port = out.ports().into_iter().find(|p| out.port_name(p).ok().as_deref() == Some(settings.output.as_str()));
            match port {
                Some(port) => Some(out.connect(&port, "Folie-DMX").map_err(|e| format!("sortie {} : {e}", settings.output))?),
                None => return Err(format!("sortie MIDI « {} » absente", settings.output)),
            }
        };
        Ok(Self {
            _inputs: connections,
            output: Mutex::new(output),
            events,
            sender,
            settings: settings.clone(),
            learning: Mutex::new(false),
            mtc: Mutex::new(MtcReader::default()),
            timecode: Arc::new(Mutex::new(None)),
            sent: Mutex::new(HashMap::new()),
            opened,
        })
    }

    /// Message comme s'il venait d'un port (tests, ou un autre fil).
    pub fn inject(&self, bytes: &[u8]) {
        if let Some(event) = decode(bytes) {
            let _ = self.sender.send(event);
        }
    }

    /// Associations et MSC changés : pris en compte sans rouvrir les ports.
    pub fn update(&mut self, settings: &MidiSettings) {
        self.settings.mappings = settings.mappings.clone();
        self.settings.msc = settings.msc;
        self.settings.msc_device = settings.msc_device;
        self.settings.leds = settings.leds;
        // Le contrôleur reçoit à nouveau tout l'état.
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn learn(&self, on: bool) {
        *self.learning.lock().unwrap_or_else(|e| e.into_inner()) = on;
    }

    pub fn timecode(&self) -> Option<Timecode> {
        *self.timecode.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Applique les messages arrivés depuis la dernière image.
    pub fn handle(&self, console: &mut Console) -> Handled {
        let mut handled = Handled::default();
        for event in self.events.try_iter() {
            match &event {
                Event::QuarterFrame(data) => {
                    if let Some(tc) = self.mtc.lock().unwrap_or_else(|e| e.into_inner()).quarter(*data) {
                        *self.timecode.lock().unwrap_or_else(|e| e.into_inner()) = Some(tc);
                    }
                    continue;
                }
                Event::SysEx(bytes) => {
                    if let Some(tc) = MtcReader::full_frame(bytes) {
                        *self.timecode.lock().unwrap_or_else(|e| e.into_inner()) = Some(tc);
                    } else if self.settings.msc
                        && let Some(msc) = decode_msc(bytes, self.settings.msc_device)
                    {
                        handled.results.push(msc_action(msc).apply(console));
                    }
                    continue;
                }
                _ => {}
            }
            let Some((control, value)) = control_of(&event) else { continue };
            {
                let mut learning = self.learning.lock().unwrap_or_else(|e| e.into_inner());
                // On apprend à l'appui (ou au mouvement d'un fader), pas au relâchement.
                if *learning && (value > 0.0 || matches!(control, Control::Cc { .. })) {
                    *learning = false;
                    handled.learned = Some(control);
                    continue;
                }
            }
            for mapping in self.settings.mappings.iter().filter(|m| m.control == control) {
                let Some(action) = action_for(&mapping.target, value, console) else { continue };
                let quiet = action.quiet();
                let result = action.apply(console);
                if !(quiet && result.is_ok()) {
                    handled.results.push(result);
                }
            }
        }
        handled
    }

    /// Retour vers le contrôleur : ce qui a changé depuis le dernier envoi.
    pub fn publish(&self, console: &Console, snapshot: &Snapshot) {
        let mut output = self.output.lock().unwrap_or_else(|e| e.into_inner());
        let Some(out) = output.as_mut() else { return };
        let mut sent = self.sent.lock().unwrap_or_else(|e| e.into_inner());
        for (control, value) in feedback(&self.settings.mappings, console, snapshot) {
            if sent.get(&control) == Some(&value) {
                continue;
            }
            let message = feedback_message(self.settings.leds, control, value);
            if out.send(&message).is_ok() {
                sent.insert(control, value);
            }
        }
    }
}

/// Valeur à renvoyer à chaque commande associée : niveau du fader (0 à 127), LED
/// allumée (127) quand l'executor joue ou que le blackout est mis.
pub fn feedback(mappings: &[Mapping], console: &Console, snapshot: &Snapshot) -> Vec<(Control, u8)> {
    let mut out: Vec<(Control, u8)> = Vec::new();
    for mapping in mappings {
        let value = match &mapping.target {
            Target::Fader { slot } => {
                let status = snapshot.executors.get(&console.executor_at(*slot));
                (status.map_or(1.0, |s| s.level).clamp(0.0, 1.0) * 127.0).round() as u8
            }
            Target::Blackout => if snapshot.blackout { 127 } else { 0 },
            target => match target.slot() {
                Some(slot) => {
                    let on = snapshot.executors.get(&console.executor_at(slot)).is_some_and(|s| s.cue.is_some());
                    if on { 127 } else { 0 }
                }
                None => continue,
            },
        };
        // Une commande associée deux fois : la première décide.
        if !out.iter().any(|(c, _)| *c == mapping.control) {
            out.push((mapping.control, value));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_notes_cc_and_system_messages() {
        assert_eq!(decode(&[0x91, 60, 100]), Some(Event::Note { channel: 1, note: 60, velocity: 100 }));
        assert_eq!(decode(&[0x80, 60, 64]), Some(Event::Note { channel: 0, note: 60, velocity: 0 }), "note off");
        assert_eq!(decode(&[0xB2, 7, 127]), Some(Event::Cc { channel: 2, number: 7, value: 127 }));
        assert_eq!(decode(&[0xF1, 0x25]), Some(Event::QuarterFrame(0x25)));
        assert_eq!(decode(&[0xE0, 0, 64]), None, "pitch bend : pas encore");
        assert_eq!(Control::Note { channel: 0, number: 60 }.label(), "Note 60 (C4) · canal 1");
    }

    #[test]
    fn msc_messages() {
        // GO cue 3.5, liste 2, pour tous les appareils.
        let go = [0xF0, 0x7F, 0x7F, 0x02, 0x01, 0x01, b'3', b'.', b'5', 0x00, b'2', 0xF7];
        assert_eq!(decode_msc(&go, 1), Some(Msc::Go { cue: Some("3.5".into()), list: Some(2) }));
        assert_eq!(msc_action(decode_msc(&go, 1).unwrap()), Action::Goto { executor: Some(2), cue: "3.5".into() });
        // GO sans numéro : la cue suivante de l'executor sélectionné.
        let next = [0xF0, 0x7F, 0x05, 0x02, 0x01, 0x01, 0xF7];
        assert_eq!(decode_msc(&next, 5), Some(Msc::Go { cue: None, list: None }));
        assert_eq!(decode_msc(&next, 6), None, "pas pour nous");
        assert_eq!(decode_msc(&[0xF0, 0x7F, 0x7F, 0x02, 0x01, 0x0A, 0xF7], 0), Some(Msc::AllOff));
    }

    #[test]
    fn timecode_from_quarter_frames_and_full_frames() {
        // 01:02:03:04 à 25 i/s, en huit quarts.
        let (h, m, s, f) = (1u8 | (1 << 5), 2u8, 3u8, 4u8);
        let pieces = [f & 0x0F, f >> 4, s & 0x0F, s >> 4, m & 0x0F, m >> 4, h & 0x0F, h >> 4];
        let mut reader = MtcReader::default();
        let mut result = None;
        for (i, p) in pieces.iter().enumerate() {
            result = reader.quarter(((i as u8) << 4) | p);
        }
        let tc = result.unwrap();
        assert_eq!((tc.hours, tc.minutes, tc.seconds, tc.frames, tc.fps), (1, 2, 3, 6, 25.0));
        let full = MtcReader::full_frame(&[0xF0, 0x7F, 0x7F, 0x01, 0x01, 0x61, 0, 1, 12, 0xF7]).unwrap();
        assert_eq!(full.label(), "01:00:01:12");
        assert!((full.seconds() - (3601.0 + 12.0 / 30.0)).abs() < 1e-6, "30 i/s");
    }

    #[test]
    fn profiles_are_consistent() {
        for profile in profiles() {
            let mut controls: Vec<Control> = profile.mappings.iter().map(|m| m.control).collect();
            let count = controls.len();
            controls.sort_by_key(|c| format!("{c:?}"));
            controls.dedup();
            assert_eq!(controls.len(), count, "{} : une commande associée deux fois", profile.name);
            for mapping in &profile.mappings {
                if let Some(slot) = mapping.target.slot() {
                    assert!((1..=folie_show::SLOTS_PER_PAGE).contains(&slot), "{}", profile.name);
                }
            }
        }
        let minilab = &profiles()[0];
        assert_eq!(minilab.mappings.len(), 4 + 6 + 1 + 8 + 8 + 7);
        assert_eq!(minilab.mappings[27], Mapping { control: Control::Note { channel: 0, number: 48 }, target: Target::Press { slot: 11 } });
        assert_eq!(minilab.mappings[0], Mapping { control: Control::Cc { channel: 0, number: 82 }, target: Target::Fader { slot: 1 } });
        assert_eq!(minilab.mappings[10], Mapping { control: Control::Cc { channel: 0, number: 114 }, target: Target::PageEncoder });
        assert_eq!(minilab.mappings[11].control, Control::Note { channel: 9, number: 36 }, "pad 1, canal 10");
    }

    #[test]
    fn minilab3_pads_light_by_sysex() {
        let pad1 = Control::Note { channel: 9, number: 36 };
        assert_eq!(
            feedback_message(Leds::MiniLab3, pad1, 127),
            [0xF0, 0x00, 0x20, 0x6B, 0x7F, 0x42, 0x02, 0x01, 0x16, 0x34, 127, 70, 0, 0xF7]
        );
        let pad_b8 = Control::Note { channel: 9, number: 51 };
        assert_eq!(feedback_message(Leds::MiniLab3, pad_b8, 0)[9], 0x4B, "pad 8 de la banque B");
        assert_eq!(feedback_message(Leds::Standard, pad1, 127), [0x99, 36, 127]);
        assert_eq!(feedback_message(Leds::MiniLab3, Control::Cc { channel: 0, number: 82 }, 64), [0xB0, 82, 64]);
    }

    #[test]
    fn settings_round_trip() {
        let settings = MidiSettings {
            enabled: true,
            mappings: vec![
                Mapping { control: Control::Cc { channel: 0, number: 7 }, target: Target::Fader { slot: 1 } },
                Mapping { control: Control::Note { channel: 9, number: 36 }, target: Target::Command { line: "Go Executor 3".into() } },
            ],
            ..Default::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<MidiSettings>(&json).unwrap(), settings);
        assert_eq!(serde_json::from_str::<MidiSettings>("{}").unwrap(), MidiSettings::default());
    }
}

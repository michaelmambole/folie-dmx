//! Ce que voient les télécommandes : la page d'executors en cours, les groupes, le blackout.

use folie_control::Console;
use folie_engine::{ExecutorStatus, Snapshot};
use folie_show::{SLOTS_PER_PAGE, executor_label};
use serde::Serialize;

/// Places des executors à fader sur une page (les autres n'ont que des touches).
pub const FADER_SLOTS: u32 = 10;

#[derive(Debug, Serialize)]
pub struct State {
    #[serde(rename = "type")]
    kind: &'static str,
    show: String,
    page: u32,
    page_name: String,
    blackout: bool,
    /// Les commandes vont au programmer blind (rien ne part sur scène).
    blind: bool,
    /// Tempo des chenillards et des effets.
    tempo: f32,
    /// Fixtures dans le programmer en cours.
    programmer: usize,
    /// Executor visé par Go et Back sans numéro.
    selected: String,
    executors: Vec<ExecutorView>,
    groups: Vec<GroupView>,
    sounds: Vec<SoundView>,
    /// Le code lecture peut-il lancer les sons ?
    playback_sounds: bool,
    presets: Vec<PresetView>,
    macros: Vec<MacroView>,
    /// Timecode MIDI reçu (s, au dixième), s'il en arrive.
    timecode: Option<f64>,
}

#[derive(Debug, Serialize)]
struct PresetView {
    /// Famille, comme sur la console (« Couleur »).
    feature: &'static str,
    /// Numéro à taper : « 4.2 ».
    key: String,
    name: String,
}

#[derive(Debug, Serialize)]
struct MacroView {
    number: u32,
    name: String,
    running: bool,
}

/// Ce que l'interface sait des sons et que la console ignore : ce qui joue, les durées.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SoundsLive {
    /// Son -> position de lecture (s).
    pub positions: std::collections::HashMap<u32, f64>,
    /// Son -> durée jouée, de l'entrée à la sortie (s), une fois le fichier chargé.
    pub durations: std::collections::HashMap<u32, f64>,
}

#[derive(Debug, Serialize)]
struct SoundView {
    number: u32,
    name: String,
    group: u32,
    then: Option<u32>,
    looped: bool,
    duration: Option<f64>,
    /// En lecture : temps écoulé depuis l'entrée, au dixième (s).
    elapsed: Option<f64>,
}

impl State {
    pub fn add_sounds(&mut self, console: &Console, live: &SoundsLive, playback_sounds: bool) {
        let tenth = |t: f64| (t * 10.0).round() / 10.0;
        self.sounds = console
            .show
            .sounds
            .iter()
            .map(|(&number, sound)| SoundView {
                number,
                name: sound.name.clone(),
                group: sound.group,
                then: sound.then,
                looped: sound.looped,
                duration: live.durations.get(&number).copied().map(tenth),
                elapsed: live.positions.get(&number).map(|t| tenth((t - sound.start).max(0.0))),
            })
            .collect();
        self.playback_sounds = playback_sounds;
    }
}

#[derive(Debug, Serialize)]
struct ExecutorView {
    id: u32,
    label: String,
    fader: bool,
    /// « sequence », « macro » ou « empty ».
    kind: &'static str,
    name: String,
    mode: &'static str,
    level: f32,
    /// Ce que fait le fader : « Master », « Temp », « Vitesse », « Crossfade ».
    fader_function: &'static str,
    /// Valeur du fader telle que la console l'affiche (« 40 % », « × 0.80 »).
    value: String,
    /// Cue en cours (ou état), et suivante.
    now: String,
    next: String,
    on: bool,
    flashing: bool,
    /// Transition en cours : avancement de 0 à 1.
    progress: Option<f32>,
    fixed: bool,
    /// Temps suivi par les cues à temps TC : « son 3 », « TC » ; vide sinon.
    follows: String,
    /// Couleur de repère, « #rrggbb ».
    color: Option<String>,
}

#[derive(Debug, Serialize)]
struct GroupView {
    number: u32,
    name: String,
}

pub fn build(console: &Console, snapshot: &Snapshot, show_name: &str) -> State {
    let page = console.page();
    let executors = (1..=SLOTS_PER_PAGE)
        .map(|slot| {
            let id = console.executor_at(slot);
            executor_view(console, id, slot <= FADER_SLOTS, snapshot.executors.get(&id))
        })
        .collect();
    let groups = console.show.groups.iter().map(|(n, g)| GroupView { number: *n, name: g.name.clone() }).collect();
    // Presets dans l'ordre des familles de la console (Dimmer, Position, Gobo, Couleur…).
    let presets = folie_fixture::FeatureGroup::ALL
        .iter()
        .flat_map(|&feature| {
            console.show.presets.range((feature, 0)..=(feature, u32::MAX)).map(move |(&(f, n), p)| PresetView {
                feature: f.label(),
                key: format!("{}.{n}", f.preset_number()),
                name: p.name.clone(),
            })
        })
        .collect();
    let running = console.macros_in_progress();
    let macros = console
        .show
        .macros
        .iter()
        .map(|(&number, m)| MacroView {
            number,
            name: m.name.clone(),
            running: running.iter().any(|p| p.number == number),
        })
        .collect();
    State {
        kind: "state",
        show: show_name.to_string(),
        page,
        page_name: console.page_name(page).to_string(),
        blackout: snapshot.blackout,
        blind: console.blind(),
        tempo: (console.tempo() * 10.0).round() / 10.0,
        programmer: console.programmer().keys().map(|k| k.fixture).collect::<std::collections::BTreeSet<_>>().len(),
        selected: executor_label(console.selected_executor()),
        executors,
        groups,
        sounds: Vec::new(),
        playback_sounds: false,
        presets,
        macros,
        timecode: console.received_timecode().map(|t| (t * 10.0).floor() / 10.0),
    }
}

fn executor_view(console: &Console, id: u32, fader: bool, status: Option<&ExecutorStatus>) -> ExecutorView {
    let mut view = ExecutorView {
        id,
        label: executor_label(id),
        fader,
        kind: "empty",
        name: String::new(),
        mode: "",
        level: status.map_or(1.0, |s| s.level),
        fader_function: "Master",
        value: String::new(),
        now: "vide".into(),
        next: String::new(),
        on: false,
        flashing: status.is_some_and(|s| s.flashing),
        progress: None,
        fixed: false,
        follows: String::new(),
        color: None,
    };
    if let Some((number, m)) = console.executor_macro(id) {
        let running = console.macros_in_progress().iter().any(|p| p.number == number);
        view.kind = "macro";
        view.name = if m.name.is_empty() { format!("Macro {number}") } else { m.name.clone() };
        view.mode = "Macro";
        view.now = if running { "en cours".into() } else { format!("macro {number}") };
        view.on = running;
        return view;
    }
    let Some((x, sequence)) = console.executor(id) else {
        return view;
    };
    let current = status.and_then(|s| s.cue);
    let label = |i: usize| {
        sequence.cues.iter().nth(i).map(|(n, c)| if c.name.is_empty() { n.to_string() } else { format!("{n} {}", c.name) })
    };
    view.kind = "sequence";
    view.name = sequence.name.clone();
    view.mode = x.mode.label();
    view.fixed = x.options.fixed;
    view.on = current.is_some();
    view.now = current.and_then(label).unwrap_or_else(|| "éteint".into());
    view.next = match current.map_or(Some(0), |c| c.checked_add(1)).and_then(label) {
        Some(next) => next,
        None if sequence.cues.is_empty() => "aucune cue".into(),
        None => "fin".into(),
    };
    view.progress = status.filter(|s| s.in_transition()).map(|s| (s.elapsed / s.duration.max(f32::EPSILON)).clamp(0.0, 1.0));
    view.fader_function = x.options.fader.label();
    view.color = x.options.color.map(|[r, g, b]| format!("#{r:02x}{g:02x}{b:02x}"));
    if sequence.cues.values().any(|c| c.timecode.is_some()) {
        view.follows = match sequence.follows_sound {
            Some(n) => format!("son {n}"),
            None => "TC".into(),
        };
    }
    view.value = if x.options.fader == folie_show::FaderFunction::Speed {
        format!("× {:.2}", view.level * 2.0)
    } else {
        format!("{:.0} %", view.level * 100.0)
    };
    view
}

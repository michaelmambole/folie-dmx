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
    /// Executor visé par Go et Back sans numéro.
    selected: String,
    executors: Vec<ExecutorView>,
    groups: Vec<GroupView>,
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
    /// Cue en cours (ou état), et suivante.
    now: String,
    next: String,
    on: bool,
    flashing: bool,
    /// Transition en cours : avancement de 0 à 1.
    progress: Option<f32>,
    fixed: bool,
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
    State {
        kind: "state",
        show: show_name.to_string(),
        page,
        page_name: console.page_name(page).to_string(),
        blackout: snapshot.blackout,
        selected: executor_label(console.selected_executor()),
        executors,
        groups,
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
        now: "vide".into(),
        next: String::new(),
        on: false,
        flashing: status.is_some_and(|s| s.flashing),
        progress: None,
        fixed: false,
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
    view
}

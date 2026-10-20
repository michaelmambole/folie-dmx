//! Fenêtres Cues et Executors.

use eframe::egui::{self, Align, Color32, Layout, RichText};
use folie_control::{CommandError, Console, Outcome};
use folie_engine::{ExecutorStatus, Snapshot};
use folie_show::CueNumber;

type Action = Option<Result<Outcome, CommandError>>;

/// Executors affichés : une page de 10, comme une aile de fader.
pub const EXECUTORS: std::ops::RangeInclusive<u32> = 1..=10;
const CURRENT: Color32 = Color32::from_rgb(255, 170, 40);
const FADER_LENGTH: f32 = 170.0;

/// Nom de la cue en cours d'un executor, pour l'affichage.
fn current_cue_label(console: &Console, executor: u32, status: Option<&ExecutorStatus>) -> String {
    let index = status.and_then(|s| s.cue);
    let sequence = console.show.executors.get(&executor).and_then(|s| console.show.sequences.get(s));
    match (sequence, index) {
        (None, _) => "—".into(),
        (Some(_), None) => "éteint".into(),
        (Some(sequence), Some(i)) => sequence
            .cues
            .iter()
            .nth(i)
            .map(|(n, c)| if c.name.is_empty() { n.to_string() } else { format!("{n} {}", c.name) })
            .unwrap_or_default(),
    }
}

fn transport(ui: &mut egui::Ui, console: &Console, executor: u32) -> Action {
    let mut action = None;
    ui.horizontal(|ui| {
        if ui.button(RichText::new("Go").strong()).on_hover_text(format!("Go executor {executor}")).clicked() {
            action = Some(console.go(Some(executor)));
        }
        if ui.button("Back").on_hover_text(format!("Back executor {executor}")).clicked() {
            action = Some(console.back(Some(executor)));
        }
        if ui.button("Off").on_hover_text(format!("Éteindre l'executor {executor}")).clicked() {
            action = Some(console.off(Some(executor)));
        }
    });
    action
}

/// Liste des cues de la séquence de l'executor sélectionné.
pub fn cues_window(ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot) -> Action {
    let mut action = None;
    let executor = console.selected_executor();
    ui.horizontal(|ui| {
        let label = ui.label("Executor");
        let mut selected = executor;
        ui.add(egui::DragValue::new(&mut selected).range(EXECUTORS)).labelled_by(label.id);
        if selected != executor {
            action = Some(Ok(console.select_executor(selected)));
        }
        if let Some((_, sequence)) = console.selected_sequence() {
            ui.label(RichText::new(&sequence.name).strong());
        }
        ui.separator();
        if let Some(a) = transport(ui, console, executor) {
            action = Some(a);
        }
    });
    let status = snapshot.executors.get(&executor);
    if let Some(status) = status.filter(|s| s.cue.is_some()) {
        ui.add(egui::ProgressBar::new(status.progress).desired_height(6.0));
    }
    ui.separator();

    let Some((_, sequence)) = console.selected_sequence() else {
        ui.label("Aucune cue. Réglez des valeurs puis tapez « Store Cue », ou :");
        ui.monospace("Fixture 1 Thru 4 At 100   puis   Store Cue 1 \"Entrée\"");
        return action;
    };
    let current = status.and_then(|s| s.cue);
    let cues: Vec<(CueNumber, String, f32, f32)> =
        sequence.cues.iter().map(|(n, c)| (*n, c.name.clone(), c.fade, c.delay)).collect();

    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        egui::Grid::new("cues").num_columns(5).striped(true).spacing([14.0, 6.0]).show(ui, |ui| {
            ui.strong("N°");
            ui.strong("Nom");
            ui.strong("Fondu (s)");
            ui.strong("Délai (s)");
            ui.label("");
            ui.end_row();
            for (index, (number, name, fade, delay)) in cues.into_iter().enumerate() {
                let is_current = current == Some(index);
                let text = |t: String| {
                    let t = RichText::new(t);
                    if is_current { t.color(CURRENT).strong() } else { t }
                };
                // Un clic sur le numéro va directement à la cue.
                let goto = ui.selectable_label(is_current, text(number.to_string())).on_hover_text("Aller à cette cue");
                if goto.clicked() {
                    action = Some(console.goto(number));
                }
                ui.label(text(name));
                let (mut new_fade, mut new_delay) = (fade, delay);
                ui.add(egui::DragValue::new(&mut new_fade).range(0.0..=3600.0).speed(0.1).max_decimals(2))
                    .on_hover_text(format!("Fondu de la cue {number}"));
                ui.add(egui::DragValue::new(&mut new_delay).range(0.0..=3600.0).speed(0.1).max_decimals(2))
                    .on_hover_text(format!("Délai de la cue {number}"));
                if new_fade != fade || new_delay != delay {
                    action = Some(console.set_cue_times(number, new_fade, new_delay));
                }
                if ui.small_button("Supprimer").on_hover_text(format!("Supprimer la cue {number}")).clicked() {
                    action = Some(console.delete_cue(number));
                }
                ui.end_row();
            }
        });
    });
    action
}

/// Une aile de 10 executors : fader d'intensité, Go, Back, Off.
pub fn executors_window(ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot) -> Action {
    let mut action = None;
    egui::ScrollArea::horizontal().show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for executor in EXECUTORS {
                let status = snapshot.executors.get(&executor);
                let selected = console.selected_executor() == executor;
                ui.allocate_ui_with_layout(egui::vec2(96.0, FADER_LENGTH + 140.0), Layout::top_down(Align::Center), |ui| {
                    let title = RichText::new(format!("Exec {executor}")).strong();
                    let title = if selected { title.color(CURRENT) } else { title };
                    if ui.selectable_label(selected, title).on_hover_text("Sélectionner cet executor").clicked() {
                        action = Some(Ok(console.select_executor(executor)));
                    }
                    ui.small(current_cue_label(console, executor, status));

                    let has_sequence = console.show.executors.contains_key(&executor);
                    let mut level = status.map_or(100.0, |s| s.level * 100.0);
                    ui.spacing_mut().slider_width = FADER_LENGTH;
                    let fader = egui::Slider::new(&mut level, 0.0..=100.0).vertical().show_value(false);
                    let response = ui
                        .add_enabled(has_sequence, fader)
                        .on_hover_text(format!("Fader de l'executor {executor}"));
                    if response.changed() {
                        action = Some(console.set_fader(executor, level));
                    }
                    ui.monospace(format!("{level:.0} %"));
                    if has_sequence && let Some(a) = transport(ui, console, executor) {
                        action = Some(a);
                    }
                });
                ui.separator();
            }
        });
    });
    action
}

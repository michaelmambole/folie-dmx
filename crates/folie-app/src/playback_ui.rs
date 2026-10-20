//! Fenêtres Cues et Executors.

use std::collections::BTreeSet;

use eframe::egui::{self, Color32, RichText};
use folie_control::{CommandError, Console, Outcome};
use folie_engine::{ExecutorStatus, Snapshot};
use folie_show::{CueNumber, ExecutorMode};

type Action = Option<Result<Outcome, CommandError>>;

/// Executors à fader : une aile de 10.
pub const FADER_EXECUTORS: std::ops::RangeInclusive<u32> = 1..=10;
/// Executors à boutons, sans fader.
pub const BUTTON_EXECUTORS: std::ops::RangeInclusive<u32> = 11..=20;
const CURRENT: Color32 = Color32::from_rgb(255, 170, 40);
const ARMED: Color32 = Color32::from_rgb(200, 30, 30);
const FADER_LENGTH: f32 = 150.0;
const CARD_WIDTH: f32 = 112.0;
const MODE: Color32 = Color32::from_rgb(120, 170, 255);

/// Cadre d'un executor, de largeur fixe, contenu centré.
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::group(ui.style())
        .inner_margin(6.0)
        .show(ui, |ui| {
            ui.set_width(CARD_WIDTH);
            ui.vertical_centered(add).inner
        })
        .inner
}

fn cue_label(number: &CueNumber, name: &str) -> String {
    if name.is_empty() { number.to_string() } else { format!("{number} {name}") }
}

/// Cue en cours et cue suivante d'un executor.
fn cue_lines(console: &Console, executor: u32, status: Option<&ExecutorStatus>) -> (String, String) {
    let Some((_, sequence)) = console.executor(executor) else {
        return ("vide".into(), "Store puis clic ici".into());
    };
    let current = status.and_then(|s| s.cue);
    let label = |i: usize| sequence.cues.iter().nth(i).map(|(n, c)| cue_label(n, &c.name));
    let now = current.and_then(label).unwrap_or_else(|| "éteint".into());
    let next = match current.map_or(Some(0), |c| c.checked_add(1)).and_then(label) {
        Some(next) => format!("ensuite {next}"),
        None if sequence.cues.is_empty() => "aucune cue".into(),
        None => "fin de séquence".into(),
    };
    (now, next)
}

fn store_here(ui: &mut egui::Ui, console: &mut Console, executor: u32, store_armed: &mut bool) -> Action {
    let button = egui::Button::new(RichText::new("Enregistrer ici").strong().color(Color32::WHITE)).fill(ARMED);
    if ui.add_sized([CARD_WIDTH - 12.0, 24.0], button).on_hover_text(format!("Store executor {executor}")).clicked() {
        *store_armed = false;
        return Some(console.store_in_executor(executor));
    }
    None
}

/// Touches de l'executor selon son mode. Flash : actif tant que la touche est maintenue.
fn mode_buttons(ui: &mut egui::Ui, console: &Console, executor: u32, held: &mut BTreeSet<u32>) -> Action {
    let mode = console.executor(executor).map_or(ExecutorMode::Go, |(x, _)| x.mode);
    let mut action = None;
    match mode {
        ExecutorMode::Go => {
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Go").strong()).on_hover_text(format!("Go executor {executor}")).clicked() {
                    action = Some(console.press(executor));
                }
                if ui.button("Back").on_hover_text(format!("Back executor {executor}")).clicked() {
                    action = Some(console.back(Some(executor)));
                }
                if ui.button("Off").on_hover_text(format!("Éteindre l'executor {executor}")).clicked() {
                    action = Some(console.off(Some(executor)));
                }
            });
        }
        ExecutorMode::Flash => {
            let response = ui
                .add_sized([CARD_WIDTH - 12.0, 24.0], egui::Button::new(RichText::new("Flash").strong()))
                .on_hover_text(format!("Flash executor {executor} (maintenir)"));
            let down = response.is_pointer_button_down_on();
            if down && held.insert(executor) {
                console.flash(executor, true);
            } else if !down && held.remove(&executor) {
                console.flash(executor, false);
            }
            if response.clicked() && console.executor(executor).is_none() {
                action = Some(console.press(executor));
            }
        }
        ExecutorMode::Toggle => {
            let button = egui::Button::new(RichText::new("Bascule").strong());
            if ui.add_sized([CARD_WIDTH - 12.0, 24.0], button).on_hover_text(format!("Basculer l'executor {executor}")).clicked() {
                action = Some(console.press(executor));
            }
        }
    }
    action
}

/// Menu du clic droit : mode, nom, séquence, vider.
fn executor_menu(ui: &mut egui::Ui, console: &mut Console, executor: u32) -> Action {
    let mut action = None;
    let Some((x, sequence)) = console.executor(executor).map(|(x, s)| (*x, s.name.clone())) else {
        ui.label("Executor vide");
        ui.weak("Store, puis cliquez l'executor");
        return None;
    };
    ui.label(RichText::new("Mode").weak());
    for mode in ExecutorMode::ALL {
        if ui.add(egui::Button::selectable(x.mode == mode, mode.label())).clicked() {
            action = Some(console.set_executor_mode(executor, mode));
            ui.close();
        }
    }
    ui.separator();
    let id = egui::Id::unique(("rename-executor", executor));
    let mut name = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or(sequence);
    ui.horizontal(|ui| {
        let label = ui.label("Nom");
        let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0)).labelled_by(label.id);
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = Some(console.rename_executor(executor, name.clone()));
            ui.close();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, name));
    let sequences: Vec<(u32, String)> = console.show.sequences.iter().map(|(n, s)| (*n, s.name.clone())).collect();
    ui.menu_button("Jouer une autre séquence", |ui| {
        for (number, name) in sequences {
            if ui.add(egui::Button::selectable(x.sequence == number, format!("{number}. {name}"))).clicked() {
                action = Some(console.assign_sequence(executor, number));
                ui.close();
            }
        }
    });
    ui.separator();
    if ui.button("Vider l'executor").clicked() {
        action = Some(console.clear_executor(executor));
        ui.close();
    }
    action
}

/// En-tête d'un executor : numéro (clic = sélection), mode, nom de séquence, cues.
fn executor_header(ui: &mut egui::Ui, console: &mut Console, executor: u32, status: Option<&ExecutorStatus>) -> Action {
    let mut action = None;
    let selected = console.selected_executor() == executor;
    let mode = console.executor(executor).map(|(x, _)| x.mode.label());
    let name = console.executor(executor).map(|(_, s)| s.name.clone());
    ui.horizontal(|ui| {
        let title = RichText::new(executor.to_string()).strong();
        let title = if selected { title.color(CURRENT) } else { title };
        let response = ui
            .selectable_label(selected, title)
            .on_hover_text("Sélectionner (clic) · réglages (clic droit)");
        if response.clicked() {
            action = Some(Ok(console.select_executor(executor)));
        }
        response.context_menu(|ui| {
            if let Some(a) = executor_menu(ui, console, executor) {
                action = Some(a);
            }
        });
        if let Some(mode) = mode {
            ui.label(RichText::new(mode).size(12.0).color(MODE));
        }
    });
    ui.label(RichText::new(name.unwrap_or_else(|| " ".into())).size(12.0));
    let (now, next) = cue_lines(console, executor, status);
    let active = status.is_some_and(|s| s.cue.is_some());
    let now = RichText::new(now).size(13.0);
    ui.label(if active { now.color(CURRENT).strong() } else { now.weak() });
    ui.label(RichText::new(next).size(12.0).weak());
    if let Some(status) = status.filter(|s| s.cue.is_some() && s.progress < 1.0) {
        ui.add(egui::ProgressBar::new(status.progress).desired_height(4.0).desired_width(CARD_WIDTH - 12.0));
    }
    action
}

/// Liste des cues de la séquence de l'executor sélectionné.
pub fn cues_window(ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot, store_armed: &mut bool) -> Action {
    let mut action = None;
    let executor = console.selected_executor();
    ui.horizontal(|ui| {
        let label = ui.label("Executor");
        let mut selected = executor;
        ui.add(egui::DragValue::new(&mut selected).range(1..=*BUTTON_EXECUTORS.end())).labelled_by(label.id);
        if selected != executor {
            action = Some(Ok(console.select_executor(selected)));
        }
        if let Some((_, sequence)) = console.selected_sequence() {
            ui.label(RichText::new(&sequence.name).strong());
        }
    });
    let status = snapshot.executors.get(&executor);
    if let Some(status) = status.filter(|s| s.cue.is_some()) {
        ui.add(egui::ProgressBar::new(status.progress).desired_height(6.0));
    }
    if *store_armed {
        ui.colored_label(ARMED, "Store armé : cliquez un numéro pour compléter cette cue");
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
                let help = if *store_armed { "Compléter cette cue avec le programmer" } else { "Aller à cette cue" };
                let goto = ui.selectable_label(is_current, text(number.to_string())).on_hover_text(help);
                if goto.clicked() {
                    action = Some(if *store_armed {
                        *store_armed = false;
                        console.store_cue(Some(number), None, None)
                    } else {
                        console.goto(number)
                    });
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

/// Aile de 10 executors à fader, puis une rangée de 10 executors à boutons.
pub fn executors_window(
    ui: &mut egui::Ui,
    console: &mut Console,
    snapshot: &Snapshot,
    store_armed: &mut bool,
    held: &mut BTreeSet<u32>,
) -> Action {
    let mut action = None;
    let mut keep = |a: Action| {
        if a.is_some() {
            action = a;
        }
    };
    ui.horizontal(|ui| {
        crate::pools::store_button(ui, store_armed);
        ui.weak("Store, puis « Enregistrer ici » · clic droit sur un numéro : mode, nom, séquence");
    });
    ui.add_space(4.0);
    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            for executor in FADER_EXECUTORS {
                let status = snapshot.executors.get(&executor);
                card(ui, |ui| {
                    keep(executor_header(ui, console, executor, status));
                    if *store_armed {
                        keep(store_here(ui, console, executor, store_armed));
                    }
                    let has_sequence = console.executor(executor).is_some();
                    let mut level = status.map_or(100.0, |s| s.level * 100.0);
                    ui.spacing_mut().slider_width = FADER_LENGTH;
                    let fader = egui::Slider::new(&mut level, 0.0..=100.0).vertical().show_value(false);
                    let response =
                        ui.add_enabled(has_sequence, fader).on_hover_text(format!("Fader de l'executor {executor}"));
                    if response.changed() {
                        keep(Some(console.set_fader(executor, level)));
                    }
                    ui.monospace(format!("{level:.0} %"));
                    keep(mode_buttons(ui, console, executor, held));
                });
            }
        });
        ui.add_space(8.0);
        ui.label(RichText::new("Executors à boutons").weak());
        ui.horizontal_top(|ui| {
            for executor in BUTTON_EXECUTORS {
                let status = snapshot.executors.get(&executor);
                card(ui, |ui| {
                    keep(executor_header(ui, console, executor, status));
                    if *store_armed {
                        keep(store_here(ui, console, executor, store_armed));
                    } else {
                        keep(mode_buttons(ui, console, executor, held));
                    }
                });
            }
        });
    });
    action
}

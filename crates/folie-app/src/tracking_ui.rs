//! Fenêtre Tracking : les paramètres de la séquence, cue par cue, avec l'état
//! et l'origine de chaque valeur.

use eframe::egui::{self, Color32, RichText};
use folie_control::tracking::{TrackedValue, TrackingCell, TrackingRow, TrackingSheet};
use folie_control::{CommandError, Console, Outcome};
use folie_engine::Snapshot;
use folie_fixture::FeatureGroup;
use folie_show::{CueNumber, CueValue};

use crate::app::DisplayMode;
use crate::theme;

type Action = Option<Result<Outcome, CommandError>>;

const NAME_WIDTH: f32 = 190.0;
const CELL_WIDTH: f32 = 92.0;
const ROW_HEIGHT: f32 = 22.0;

/// Ce qu'on écrit dans une case et comment.
struct CellLook {
    text: String,
    color: Color32,
    /// Pastille de la famille du preset référencé.
    preset: Option<Color32>,
    tooltip: String,
}

fn cue_title(number: &CueNumber, name: &str) -> String {
    if name.is_empty() { number.to_string() } else { format!("{number} {name}") }
}

fn value_text(value: u16, display: DisplayMode) -> String {
    match display {
        DisplayMode::Percent => format!("{:.0}", value as f32 / u16::MAX as f32 * 100.0),
        DisplayMode::Dmx => (value >> 8).to_string(),
    }
}

fn look(console: &Console, sheet: &TrackingSheet, cell: &TrackingCell, cue: usize, display: DisplayMode, changes_only: bool) -> CellLook {
    let title = |i: usize| sheet.cues.get(i).map(|(n, name)| cue_title(n, name)).unwrap_or_default();
    let changed = cell.changed_here(cue);
    let mut look = match cell.value {
        TrackedValue::Empty => CellLook { text: String::new(), color: theme::MUTED, preset: None, tooltip: String::new() },
        TrackedValue::Released => CellLook {
            text: "relâché".into(),
            color: theme::DANGER,
            preset: None,
            tooltip: format!("Relâché dans la cue {}", title(cue)),
        },
        TrackedValue::Held { .. } if changes_only && !changed => {
            CellLook { text: String::new(), color: theme::MUTED, preset: None, tooltip: String::new() }
        }
        TrackedValue::Held { value, origin } => {
            let color = if changed { theme::TEXT } else { theme::DEFAULT };
            let tooltip = if changed {
                format!("Enregistrée dans la cue {}", title(cue))
            } else {
                format!("Suivie depuis la cue {}", title(origin))
            };
            match value {
                CueValue::Value(v) => CellLook { text: value_text(v, display), color, preset: None, tooltip },
                CueValue::Preset(family, number) => {
                    let name = console.show.presets.get(&(family, number)).map(|p| p.name.clone()).unwrap_or_default();
                    let reference = format!("{}.{number}", family.preset_number());
                    let dot = theme::family_color(family);
                    CellLook {
                        text: format!("P{reference}"),
                        color,
                        preset: Some(if changed { dot } else { dot.gamma_multiply(0.45) }),
                        tooltip: format!("{tooltip} · preset {reference} « {name} »"),
                    }
                }
                CueValue::Release => unreachable!("un relâchement n'est jamais tenu"),
            }
        }
    };
    if let Some(times) = cell.own_times.filter(|_| changed && cell.holds_value()) {
        look.tooltip = format!("{} · temps propres {}", look.tooltip, times_text(times));
    }
    if cell.effect {
        look.tooltip = if look.tooltip.is_empty() { "Animé par un effet".into() } else { format!("{} · animé par un effet", look.tooltip) };
    }
    look
}

/// « 4s » ou « 6s +1 » : temps propres affichés dans une case.
fn times_text((fade, delay): (f32, f32)) -> String {
    if delay > 0.0 { format!("{fade}s +{delay}") } else { format!("{fade}s") }
}

/// Une case dessinée : fond (teinté pour la cue en cours), trait violet si un effet l'anime,
/// temps propres en bleu, pastille de preset, valeur alignée à droite.
/// Une valeur enregistrée dans la cue se règle au clic droit.
fn cell(ui: &mut egui::Ui, look: &CellLook, c: &TrackingCell, current: bool, editable: bool) -> egui::Response {
    let sense = if editable { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(CELL_WIDTH, ROW_HEIGHT), sense);
    let painter = ui.painter();
    if current {
        painter.rect_filled(rect, 0.0, theme::EXECUTOR.gamma_multiply(0.10));
    }
    if editable && response.hovered() {
        painter.rect_stroke(rect.shrink(1.0), 3.0, egui::Stroke::new(1.0, theme::SURFACE_3), egui::StrokeKind::Inside);
    }
    painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, theme::SURFACE_2));
    if c.effect {
        let bar = egui::Rect::from_min_size(rect.min + egui::vec2(0.0, 3.0), egui::vec2(2.0, rect.height() - 6.0));
        painter.rect_filled(bar, 1.0, theme::EFFECT);
    }
    if let Some(times) = c.own_times.filter(|_| c.holds_value()) {
        let font = egui::FontId::proportional(10.0);
        painter.text(rect.left_center() + egui::vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, times_text(times), font, theme::INFO);
    }
    let font = if look.text == "relâché" { egui::FontId::proportional(11.0) } else { egui::FontId::monospace(12.0) };
    let galley = painter.layout_no_wrap(look.text.clone(), font, look.color);
    let text_pos = egui::pos2(rect.right() - 8.0 - galley.size().x, rect.center().y - galley.size().y / 2.0);
    if let Some(dot) = look.preset {
        let square = egui::Rect::from_center_size(egui::pos2(text_pos.x - 7.0, rect.center().y), egui::vec2(7.0, 7.0));
        painter.rect_filled(square, 2.0, dot);
    }
    painter.galley(text_pos, galley, look.color);
    let text = look.text.clone();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
    if look.tooltip.is_empty() { response } else { response.on_hover_text(&look.tooltip) }
}

/// Menu du clic droit d'une valeur : fondu et délai propres au paramètre dans cette cue.
fn param_times_menu(ui: &mut egui::Ui, console: &mut Console, number: CueNumber, row: &TrackingRow) -> Action {
    let (_, sequence) = console.selected_sequence()?;
    let cue = sequence.cues.get(&number).cloned()?;
    let family = console
        .show
        .patch
        .get(row.fixture)
        .and_then(|f| f.mode().parameters.iter().find(|p| p.name == row.parameter))
        .map_or(FeatureGroup::Control, |p| p.attribute.group());
    // Sans temps propre, le paramètre suit sa famille, sinon la cue.
    let base = cue.family_times.get(&family).map_or((cue.fade, cue.delay), |t| t.or((cue.fade, cue.delay)));
    let mut timing = cue.param_times.get(&row.fixture).and_then(|p| p.get(&row.parameter)).copied().unwrap_or_default();
    let mut action = None;
    ui.label(RichText::new(format!("{} · fixture {} · cue {number}", row.parameter, row.fixture)).family(theme::medium()));
    let mut changed = false;
    egui::Grid::new(("param-times", number, row.fixture, &row.parameter)).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        ui.label(RichText::new("Fondu").color(theme::MUTED));
        ui.horizontal(|ui| {
            let help = format!("Fondu de {}, fixture {}, cue {number}", row.parameter, row.fixture);
            changed |= crate::playback_ui::optional_time(ui, &mut timing.fade, base.0, help);
        });
        ui.end_row();
        ui.label(RichText::new("Délai").color(theme::MUTED));
        ui.horizontal(|ui| {
            let help = format!("Délai de {}, fixture {}, cue {number}", row.parameter, row.fixture);
            changed |= crate::playback_ui::optional_time(ui, &mut timing.delay, base.1, help);
        });
        ui.end_row();
    });
    if changed {
        action = Some(console.set_param_times(number, row.fixture, &row.parameter, timing));
    }
    let origin = if cue.family_times.contains_key(&family) { format!("temps de {}", family.label()) } else { "temps de la cue".into() };
    ui.label(RichText::new(format!("En gris : {origin}.")).size(11.0).color(theme::MUTED));
    action
}

fn legend(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 14.0;
        let sample = |ui: &mut egui::Ui, value: &str, color: Color32, label: &str| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                ui.label(RichText::new(value).monospace().size(12.0).color(color));
                ui.label(RichText::new(label).size(11.5).color(theme::MUTED));
            });
        };
        sample(ui, "100", theme::TEXT, "changée");
        sample(ui, "100", theme::DEFAULT, "suivie");
        ui.label(RichText::new("relâché").size(11.5).color(theme::DANGER));
        theme::chip(ui, theme::family_color(FeatureGroup::Color), "preset");
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 5.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(2.0, 12.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 1.0, theme::EFFECT);
            ui.label(RichText::new("effet").size(11.5).color(theme::MUTED));
        });
    });
}

pub fn tracking_window(ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot, display: DisplayMode) -> Action {
    let mut action = None;
    let changes_id = egui::Id::unique("tracking-changes-only");
    let mut changes_only = ui.data_mut(|d| *d.get_persisted_mut_or_default::<bool>(changes_id));
    let executor = console.selected_executor();
    let sheet = console.tracking_sheet(executor);

    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if let Some(a) = crate::playback_ui::executor_field(ui, console) {
                action = Some(a);
            }
            match console.selected_sequence() {
                Some((_, sequence)) => ui.label(RichText::new(&sequence.name).family(theme::medium()).size(15.0)),
                None => ui.label(RichText::new("aucune séquence").color(theme::MUTED)),
            };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut changes_only, "Seulement les changements")
                    .on_hover_text("Masque les valeurs suivies depuis une cue précédente");
                if let Some(sheet) = &sheet {
                    let text = format!("{} cues · {} paramètres", sheet.cues.len(), sheet.rows.len());
                    ui.label(RichText::new(text).size(12.0).color(theme::MUTED));
                }
            });
        });
    });
    ui.data_mut(|d| d.insert_persisted(changes_id, changes_only));
    ui.add_space(4.0);
    legend(ui);
    ui.add_space(4.0);

    let Some(sheet) = sheet.filter(|s| !s.cues.is_empty()) else {
        ui.add_space(8.0);
        ui.label(RichText::new("Aucune cue : enregistrez des cues sur cet executor pour suivre leurs valeurs.").color(theme::MUTED));
        return action;
    };
    let current = snapshot.executors.get(&executor).and_then(|s| s.cue);

    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
        // En-têtes : une colonne par cue, la cue en cours en ambre.
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(NAME_WIDTH, ROW_HEIGHT + 4.0), egui::Sense::hover());
            ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, "PARAMÈTRE", egui::FontId::proportional(11.0), theme::MUTED);
            for (index, (number, name)) in sheet.cues.iter().enumerate() {
                let is_current = current == Some(index);
                let (rect, response) = ui.allocate_exact_size(egui::vec2(CELL_WIDTH, ROW_HEIGHT + 4.0), egui::Sense::hover());
                let painter = ui.painter_at(rect);
                if is_current {
                    painter.rect_filled(rect, 0.0, theme::EXECUTOR.gamma_multiply(0.10));
                }
                let title = cue_title(number, name);
                let color = if is_current { theme::EXECUTOR } else { theme::MUTED };
                let font = egui::FontId::new(11.5, theme::medium());
                painter.text(rect.right_center() - egui::vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, &title, font, color);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &title));
            }
        });
        let mut fixture = None;
        for row in &sheet.rows {
            if fixture != Some(row.fixture) {
                fixture = Some(row.fixture);
                ui.add_space(6.0);
                ui.label(RichText::new(format!("{} · {}", row.fixture, row.fixture_name)).family(theme::medium()).size(12.5));
            }
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(NAME_WIDTH, ROW_HEIGHT), egui::Sense::hover());
                let painter = ui.painter();
                painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, theme::SURFACE_2));
                painter.text(
                    rect.left_center() + egui::vec2(10.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &row.parameter,
                    egui::FontId::proportional(12.5),
                    theme::MUTED,
                );
                for (index, c) in row.cells.iter().enumerate() {
                    let look = look(console, &sheet, c, index, display, changes_only);
                    let editable = matches!(c.value, TrackedValue::Held { origin, .. } if origin == index);
                    let response = cell(ui, &look, c, current == Some(index), editable);
                    if editable {
                        let number = sheet.cues[index].0;
                        let menu = egui::Popup::context_menu(&response)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
                        menu.show(|ui| {
                            if let Some(a) = param_times_menu(ui, console, number, row) {
                                action = Some(a);
                            }
                        });
                    }
                }
            });
        }
    });
    action
}

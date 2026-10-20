//! Pools de groupes et de presets : des grilles de cases numérotées, comme sur MA.
//! Un clic rappelle la case ; avec « Store » armé, un clic y enregistre.

use eframe::egui::{self, Color32, RichText, vec2};
use folie_control::command::Object;
use folie_control::{CommandError, Console, Outcome, SelectMode};
use folie_fixture::FeatureGroup;

const CELL: egui::Vec2 = vec2(70.0, 44.0);
const SPACING: f32 = 4.0;
/// Cases affichées au minimum, comme un pool de console.
const MIN_CELLS: usize = 48;
/// Rangées libres toujours visibles après la dernière case occupée.
const FREE_ROWS: usize = 2;
const ARMED: Color32 = Color32::from_rgb(200, 30, 30);

pub struct PoolsView;

/// Touche Store : armée, le prochain clic dans un pool, sur un executor ou une
/// cue enregistre au lieu de rappeler. Une seule pour toute l'application.
pub fn store_button(ui: &mut egui::Ui, armed: &mut bool) {
    let text = RichText::new("Store").strong();
    let button = if *armed { egui::Button::new(text.color(Color32::WHITE)).fill(ARMED) } else { egui::Button::new(text) };
    let help = "Armer, puis cliquer une case de pool, un executor ou une cue pour y enregistrer (Échap pour annuler)";
    if ui.add(button).on_hover_text(help).clicked() {
        *armed = !*armed;
    }
}

impl PoolsView {
    pub fn new() -> Self {
        Self
    }

/// Pool de groupes. Renvoie le résultat de l'action faite, s'il y en a une.
    pub fn groups(
        &mut self,
        ui: &mut egui::Ui,
        console: &mut Console,
        store_armed: &mut bool,
    ) -> Option<Result<Outcome, CommandError>> {
        store_button(ui, store_armed);
        ui.add_space(4.0);
        let groups: Vec<(u32, String, usize)> =
            console.show.groups.iter().map(|(n, g)| (*n, g.name.clone(), g.targets.len())).collect();
        let action = grid(ui, "groups", &groups, *store_armed)?;
        Some(match action {
            CellAction::Click(number) => {
                let add = ui.input(|i| i.modifiers.command);
                let result = if *store_armed {
                    console.store_group(number, None)
                } else {
                    console.select_group(number, if add { SelectMode::Add } else { SelectMode::Replace })
                };
                *store_armed = false;
                result
            }
            CellAction::Rename(number, name) => console.label(Object::Group(number), name),
            CellAction::Delete(number) => console.delete(Object::Group(number)),
        })
    }

    /// Pool de presets d'une famille, choisie dans la fenêtre.
    pub fn presets(
        &mut self,
        ui: &mut egui::Ui,
        console: &mut Console,
        family: &mut FeatureGroup,
        store_armed: &mut bool,
    ) -> Option<Result<Outcome, CommandError>> {
        store_button(ui, store_armed);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            for f in FeatureGroup::ALL {
                ui.selectable_value(family, f, format!("{} {}", f.preset_number(), f.label()));
            }
        });
        let current = *family;
        let presets: Vec<(u32, String, usize)> = console
            .show
            .presets
            .iter()
            .filter(|((f, _), _)| *f == current)
            .map(|((_, n), p)| (*n, p.name.clone(), p.values.len()))
            .collect();
        let action = grid(ui, ("presets", current), &presets, *store_armed)?;
        Some(match action {
            CellAction::Click(number) => {
                let result = if *store_armed {
                    console.store_preset(current, number, None)
                } else {
                    console.apply_preset(current, number)
                };
                *store_armed = false;
                result
            }
            CellAction::Rename(number, name) => console.label(Object::Preset(current, number), name),
            CellAction::Delete(number) => console.delete(Object::Preset(current, number)),
        })
    }
}

/// Ce que l'utilisateur a fait sur une case.
enum CellAction {
    Click(u32),
    Rename(u32, String),
    Delete(u32),
}

/// Menu du clic droit d'une case occupée : renommer, supprimer.
fn cell_menu(ui: &mut egui::Ui, id: egui::Id, number: u32, name: &str) -> Option<CellAction> {
    let mut action = None;
    // Le texte en cours de saisie est gardé tant que le menu est ouvert.
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| name.to_string());
    ui.horizontal(|ui| {
        let label = ui.label("Nom");
        let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(150.0)).labelled_by(label.id);
        if !edit.has_focus() && !edit.lost_focus() && text == name {
            edit.request_focus();
        }
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Renommer").clicked() || enter) && !text.trim().is_empty() {
            action = Some(CellAction::Rename(number, text.trim().to_string()));
        }
    });
    ui.separator();
    if ui.button("Supprimer").on_hover_text(format!("Supprimer la case {number}")).clicked() {
        action = Some(CellAction::Delete(number));
    }
    if action.is_some() {
        ui.data_mut(|d| d.remove::<String>(id));
        ui.close();
    } else {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    action
}

/// Grille de cases : au moins 48, et toujours deux rangées libres.
/// Une case libre n'est cliquable que pour enregistrer.
fn grid(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    items: &[(u32, String, usize)],
    armed: bool,
) -> Option<CellAction> {
    // Autant de colonnes que la largeur de la fenêtre le permet.
    let columns = (((ui.available_width() + SPACING) / (CELL.x + SPACING)) as usize).max(2);
    let last = items.iter().map(|(n, _, _)| *n).max().unwrap_or(0) as usize;
    let rows = (last.div_ceil(columns) + FREE_ROWS).max(MIN_CELLS.div_ceil(columns));
    let cells = rows * columns;
    let mut action = None;
    egui::Grid::new(id).spacing([SPACING, SPACING]).show(ui, |ui| {
        for number in 1..=cells as u32 {
            let item = items.iter().find(|(n, _, _)| *n == number);
            let (text, help) = match item {
                Some((_, name, count)) => (
                    RichText::new(format!("{number}\n{name}")).strong(),
                    format!("{name} ({count}) · clic droit : renommer, supprimer"),
                ),
                None => (RichText::new(number.to_string()).weak(), "Case libre : Store, puis clic".to_string()),
            };
            let button = egui::Button::new(text).min_size(CELL).wrap();
            let enabled = item.is_some() || armed;
            let response = ui.add_enabled(enabled, button).on_hover_text(help);
            if response.clicked() {
                action = Some(CellAction::Click(number));
            }
            if let Some((_, name, _)) = item {
                let menu_id = egui::Id::unique(("pool-rename", id, number));
                response.context_menu(|ui| {
                    if let Some(a) = cell_menu(ui, menu_id, number, name) {
                        action = Some(a);
                    }
                });
            }
            if (number as usize).is_multiple_of(columns) {
                ui.end_row();
            }
        }
    });
    action
}

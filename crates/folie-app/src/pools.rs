//! Pools de groupes et de presets : des grilles de cases numérotées, comme sur MA.
//! Un clic rappelle la case ; avec « Store » armé, un clic y enregistre.

use eframe::egui::{self, Color32, RichText, vec2};
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
        let number = grid(ui, "groups", &groups, *store_armed)?;
        let add = ui.input(|i| i.modifiers.command);
        let result = if *store_armed {
            console.store_group(number, None)
        } else {
            console.select_group(number, if add { SelectMode::Add } else { SelectMode::Replace })
        };
        *store_armed = false;
        Some(result)
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
        let number = grid(ui, ("presets", current), &presets, *store_armed)?;
        let result = if *store_armed {
            console.store_preset(current, number, None)
        } else {
            console.apply_preset(current, number)
        };
        *store_armed = false;
        Some(result)
    }
}

/// Grille de cases : les cases occupées, puis quelques cases libres.
/// Renvoie le numéro cliqué. Une case libre n'est cliquable que pour enregistrer.
fn grid(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, items: &[(u32, String, usize)], armed: bool) -> Option<u32> {
    // Autant de colonnes que la largeur de la fenêtre le permet.
    let columns = (((ui.available_width() + SPACING) / (CELL.x + SPACING)) as usize).max(2);
    let last = items.iter().map(|(n, _, _)| *n).max().unwrap_or(0) as usize;
    let rows = (last.div_ceil(columns) + FREE_ROWS).max(MIN_CELLS.div_ceil(columns));
    let cells = rows * columns;
    let mut clicked = None;
    egui::Grid::new(id).spacing([SPACING, SPACING]).show(ui, |ui| {
        for number in 1..=cells as u32 {
            let item = items.iter().find(|(n, _, _)| *n == number);
            let (text, help) = match item {
                Some((_, name, count)) => {
                    (RichText::new(format!("{number}\n{name}")).strong(), format!("{name} ({count})"))
                }
                None => (RichText::new(number.to_string()).weak(), "Case libre".to_string()),
            };
            let button = egui::Button::new(text).min_size(CELL).wrap();
            let enabled = item.is_some() || armed;
            if ui.add_enabled(enabled, button).on_hover_text(help).clicked() {
                clicked = Some(number);
            }
            if (number as usize).is_multiple_of(columns) {
                ui.end_row();
            }
        }
    });
    clicked
}

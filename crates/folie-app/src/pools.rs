//! Pools de groupes et de presets : une grille de cases numérotées, comme sur MA.
//! Un clic rappelle la case ; avec « Store » armé, un clic y enregistre.

use eframe::egui::{self, Color32, RichText, vec2};
use folie_control::{CommandError, Console, Outcome, SelectMode};
use folie_fixture::FeatureGroup;

const COLUMNS: usize = 4;
const CELL: egui::Vec2 = vec2(70.0, 44.0);
const ARMED: Color32 = Color32::from_rgb(200, 30, 30);

pub struct PoolsView {
    /// Le prochain clic dans un pool enregistre au lieu de rappeler.
    store_armed: bool,
    family: FeatureGroup,
}

impl PoolsView {
    pub fn new() -> Self {
        Self { store_armed: false, family: FeatureGroup::Color }
    }

    /// Renvoie le résultat de l'action faite, s'il y en a une.
    pub fn show(&mut self, ui: &mut egui::Ui, console: &mut Console) -> Option<Result<Outcome, CommandError>> {
        let mut result = None;
        ui.horizontal(|ui| {
            ui.heading("Pools");
            let text = RichText::new("Store").strong();
            let button = if self.store_armed {
                egui::Button::new(text.color(Color32::WHITE)).fill(ARMED)
            } else {
                egui::Button::new(text)
            };
            let help = "Armer, puis cliquer une case pour y enregistrer la sélection ou le programmer";
            if ui.add(button).on_hover_text(help).clicked() {
                self.store_armed = !self.store_armed;
            }
        });
        ui.add_space(4.0);

        ui.label(RichText::new("Groupes").strong());
        let groups: Vec<(u32, String, usize)> =
            console.show.groups.iter().map(|(n, g)| (*n, g.name.clone(), g.targets.len())).collect();
        if let Some(number) = grid(ui, "groups", &groups, self.store_armed) {
            let add = ui.input(|i| i.modifiers.command);
            result = Some(if self.store_armed {
                console.store_group(number, None)
            } else {
                console.select_group(number, if add { SelectMode::Add } else { SelectMode::Replace })
            });
            self.store_armed = false;
        }

        ui.add_space(10.0);
        ui.label(RichText::new("Presets").strong());
        ui.horizontal_wrapped(|ui| {
            for family in FeatureGroup::ALL {
                let label = format!("{} {}", family.preset_number(), family.label());
                ui.selectable_value(&mut self.family, family, label);
            }
        });
        let family = self.family;
        let presets: Vec<(u32, String, usize)> = console
            .show
            .presets
            .iter()
            .filter(|((f, _), _)| *f == family)
            .map(|((_, n), p)| (*n, p.name.clone(), p.values.len()))
            .collect();
        if let Some(number) = grid(ui, ("presets", family), &presets, self.store_armed) {
            result = Some(if self.store_armed {
                console.store_preset(family, number, None)
            } else {
                console.apply_preset(family, number)
            });
            self.store_armed = false;
        }
        result
    }
}

/// Grille de cases : les cases occupées, puis quelques cases libres.
/// Renvoie le numéro cliqué. Une case libre n'est cliquable que pour enregistrer.
fn grid(ui: &mut egui::Ui, id: impl std::hash::Hash + std::fmt::Debug, items: &[(u32, String, usize)], armed: bool) -> Option<u32> {
    let last = items.iter().map(|(n, _, _)| *n).max().unwrap_or(0) as usize;
    let cells = (last + COLUMNS).div_ceil(COLUMNS).max(2) * COLUMNS;
    let mut clicked = None;
    egui::Grid::new(id).spacing([4.0, 4.0]).show(ui, |ui| {
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
            if (number as usize).is_multiple_of(COLUMNS) {
                ui.end_row();
            }
        }
    });
    clicked
}

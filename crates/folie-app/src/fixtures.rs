use std::collections::BTreeSet;
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText, Sense, vec2};
use folie_control::{Console, SelectMode};
use folie_engine::Snapshot;
use folie_fixture::{
    Capability, CapabilityKind, FeatureGroup, FixtureId, Library, ParamKey, Parameter, Patch,
    PatchedFixture, Target,
};

use crate::app::DisplayMode;

/// Couleur des valeurs saisies dans le programmer (rouge, comme sur MA).
pub const PROGRAMMED: Color32 = Color32::from_rgb(255, 95, 95);
const NAME_WIDTH: f32 = 150.0;
const VALUE_WIDTH: f32 = 110.0;
const MENU_WIDTH: f32 = 210.0;

pub(crate) struct AddForm {
    fixture_type: usize,
    mode: usize,
    pub(crate) quantity: u32,
    pub(crate) first_id: u32,
    auto_address: bool,
    address: u16,
}

pub struct FixturesView {
    last_clicked: Option<Target>,
    /// Fixtures multi-instance dont les sous-fixtures sont affichées dans le patch.
    expanded: BTreeSet<FixtureId>,
    pub(crate) form: AddForm,
    /// Dernier résultat d'une opération de patch : (succès, message).
    message: Option<(bool, String)>,
}

impl FixturesView {
    pub fn new() -> Self {
        Self {
            last_clicked: None,
            expanded: BTreeSet::new(),
            form: AddForm {
                fixture_type: 0,
                mode: 0,
                quantity: 1,
                first_id: 1,
                auto_address: true,
                address: 1,
            },
            message: None,
        }
    }

    /// Liste du patch et formulaire d'ajout. Renvoie `true` si le patch a changé.
    pub fn patch_panel(&mut self, ui: &mut egui::Ui, library: &Library, console: &mut Console) -> bool {
        let mut changed = false;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            if let Some(ids) = self.add_form(ui, library, &mut console.show.patch) {
                console.select(ids.into_iter().map(Target::whole), SelectMode::Replace);
                changed = true;
            }
        });
        if let Some((ok, text)) = &self.message {
            let color = if *ok { Color32::LIGHT_GREEN } else { Color32::LIGHT_RED };
            ui.colored_label(color, text);
        }
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            if ui.button("Tout sélectionner").clicked() {
                let all: Vec<Target> = console.show.patch.fixtures().iter().map(|f| Target::whole(f.id)).collect();
                console.select(all, SelectMode::Replace);
            }
            if ui.button("Aucune").clicked() {
                console.clear_selection();
            }
            let can_delete = !console.selection().is_empty();
            if ui.add_enabled(can_delete, egui::Button::new("Supprimer")).clicked() {
                let ids: BTreeSet<FixtureId> = console.selection().iter().map(|t| t.fixture).collect();
                for id in ids {
                    console.show.patch.remove(id);
                }
                self.message = None;
                changed = true;
            }
        });
        ui.add_space(4.0);
        self.patch_list(ui, console);
        changed
    }

    /// Renvoie les numéros des fixtures ajoutées, le cas échéant.
    fn add_form(&mut self, ui: &mut egui::Ui, library: &Library, patch: &mut Patch) -> Option<Vec<FixtureId>> {
        if library.fixtures.is_empty() {
            ui.label("Bibliothèque vide.");
            return None;
        }
        let form = &mut self.form;
        form.fixture_type = form.fixture_type.min(library.fixtures.len() - 1);
        let fixture_type = library.fixtures[form.fixture_type].clone();
        form.mode = form.mode.min(fixture_type.modes.len() - 1);

        if !fixture_type.warnings.is_empty() {
            ui.small(format!("Modes non disponibles : {}", fixture_type.warnings.join(" ; ")))
                .on_hover_text("Fonctions du fichier pas encore gérées par Folie-DMX");
        }
        egui::Grid::new("add_form").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
            let label = ui.label("Type");
            egui::ComboBox::from_id_salt("add_type")
                .width(260.0)
                .selected_text(fixture_type.display_name())
                .show_ui(ui, |ui| {
                    for (i, t) in library.fixtures.iter().enumerate() {
                        if ui.selectable_value(&mut form.fixture_type, i, t.display_name()).changed() {
                            form.mode = 0;
                        }
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Mode");
            let mode_label = |m: &folie_fixture::Mode| format!("{} ({} canaux)", m.name, m.footprint);
            egui::ComboBox::from_id_salt("add_mode")
                .width(260.0)
                .selected_text(mode_label(&fixture_type.modes[form.mode]))
                .show_ui(ui, |ui| {
                    for (i, m) in fixture_type.modes.iter().enumerate() {
                        ui.selectable_value(&mut form.mode, i, mode_label(m));
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Quantité");
            ui.add(egui::DragValue::new(&mut form.quantity).range(1..=128)).labelled_by(label.id);
            ui.end_row();

            let label = ui.label("Fixture n°");
            ui.add(egui::DragValue::new(&mut form.first_id).range(1..=99_999)).labelled_by(label.id);
            ui.end_row();

            let address_label = ui.label("Adresse");
            ui.horizontal(|ui| {
                ui.checkbox(&mut form.auto_address, "auto");
                if form.auto_address {
                    let footprint = fixture_type.modes[form.mode].footprint;
                    let needed = footprint.saturating_mul(form.quantity as u16);
                    match patch.next_free_address(0, needed) {
                        Some(address) => {
                            form.address = address;
                            ui.monospace(format!("1.{address:03}"));
                        }
                        None => {
                            ui.colored_label(Color32::LIGHT_RED, "pas assez de place");
                        }
                    }
                } else {
                    ui.label("1.");
                    ui.add(egui::DragValue::new(&mut form.address).range(1..=512))
                        .labelled_by(address_label.id);
                }
            });
            ui.end_row();
        });

        if !ui.button("Patcher").clicked() {
            return None;
        }
        match self.add_fixtures(&fixture_type, patch) {
            Ok(ids) => {
                self.message = Some((true, format!("{} fixture(s) patchée(s)", ids.len())));
                self.form.first_id = patch.next_free_id().0;
                Some(ids)
            }
            Err(text) => {
                self.message = Some((false, text));
                None
            }
        }
    }

    /// Ajoute toutes les fixtures du formulaire, ou aucune en cas d'erreur.
    fn add_fixtures(
        &self,
        fixture_type: &Arc<folie_fixture::FixtureType>,
        patch: &mut Patch,
    ) -> Result<Vec<FixtureId>, String> {
        let form = &self.form;
        let footprint = fixture_type.modes[form.mode].footprint as u32;
        let mut candidate = patch.clone();
        let mut ids = Vec::new();
        for i in 0..form.quantity {
            let id = FixtureId(form.first_id + i);
            let address = form.address as u32 + i * footprint;
            let fixture = PatchedFixture {
                id,
                name: format!("{} {}", fixture_type.name, id),
                fixture_type: fixture_type.clone(),
                mode: form.mode,
                universe: 0,
                address: address.min(u16::MAX as u32) as u16,
            };
            candidate.add(fixture).map_err(|e| format!("Fixture {id} : {e}"))?;
            ids.push(id);
        }
        *patch = candidate;
        Ok(ids)
    }

    fn patch_list(&mut self, ui: &mut egui::Ui, console: &mut Console) {
        let mut clicked = None;
        // Lignes dans l'ordre d'affichage, pour la sélection Maj-clic.
        let mut rows: Vec<Target> = Vec::new();
        let selection = console.selection();
        let patch = &console.show.patch;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            egui::Grid::new("patch_list").num_columns(5).striped(true).spacing([10.0, 4.0]).show(
                ui,
                |ui| {
                    ui.label("");
                    ui.strong("N°");
                    ui.strong("Nom");
                    ui.strong("Mode");
                    ui.strong("Adresse");
                    ui.end_row();
                    for f in patch.fixtures() {
                        let heads = &f.mode().heads;
                        let expanded = self.expanded.contains(&f.id);
                        if heads.is_empty() {
                            ui.label("");
                        } else {
                            let (text, help) = if expanded {
                                ("−", "Masquer les sous-fixtures")
                            } else {
                                ("+", "Afficher les sous-fixtures")
                            };
                            if ui.small_button(text).on_hover_text(help).clicked() && !self.expanded.remove(&f.id)
                            {
                                self.expanded.insert(f.id);
                            }
                        }
                        let target = Target::whole(f.id);
                        if target_row(ui, selection, target, &f.name) {
                            clicked = Some(target);
                        }
                        rows.push(target);
                        ui.label(&f.mode().short_name);
                        ui.monospace(format!("{}.{:03}", f.universe + 1, f.address));
                        ui.end_row();

                        if !expanded {
                            continue;
                        }
                        for (head, key) in heads.iter().enumerate() {
                            ui.label("");
                            let target = Target { fixture: f.id, head: Some(head) };
                            if target_row(ui, selection, target, &format!("  cellule {key}")) {
                                clicked = Some(target);
                            }
                            rows.push(target);
                            ui.label("");
                            let first_offset = f
                                .mode()
                                .parameters
                                .iter()
                                .filter(|p| p.head == Some(head))
                                .flat_map(|p| p.offsets.iter().copied())
                                .min()
                                .unwrap_or(0);
                            ui.monospace(format!("{}.{:03}", f.universe + 1, f.address + first_offset));
                            ui.end_row();
                        }
                    }
                },
            );
            if patch.fixtures().is_empty() {
                ui.label("Aucune fixture patchée.");
            }
        });

        if let Some(target) = clicked {
            let modifiers = ui.input(|i| i.modifiers);
            let position = |t: Target| rows.iter().position(|r| *r == t);
            if modifiers.shift
                && let Some(a) = self.last_clicked.and_then(position)
                && let Some(b) = position(target)
            {
                // Dans le sens du clic : Maj-clic vers le haut sélectionne à l'envers.
                let range: Vec<Target> =
                    if a <= b { rows[a..=b].to_vec() } else { rows[b..=a].iter().rev().copied().collect() };
                console.select(range, SelectMode::Add);
            } else if modifiers.command {
                console.select([target], SelectMode::Toggle);
            } else {
                console.select([target], SelectMode::Replace);
            }
            self.last_clicked = Some(target);
        }
    }

    /// Réglage des attributs des fixtures sélectionnées.
    pub fn attribute_panel(
        &self,
        ui: &mut egui::Ui,
        console: &mut Console,
        snapshot: &Snapshot,
        display: DisplayMode,
    ) {
        let Some(&reference) = console.selection().first() else {
            ui.add_space(20.0);
            ui.label("Sélectionnez des fixtures dans le patch (Cmd-clic, Maj-clic pour plusieurs)");
            ui.label("ou tapez une commande : Fixture 1 Thru 4 At 50");
            return;
        };
        let Some(fixture) = console.show.patch.get(reference.fixture).cloned() else { return };
        let count = console.selection().len();

        ui.horizontal(|ui| {
            ui.heading(match (count, reference.head) {
                (1, None) => fixture.name.clone(),
                (1, Some(_)) => format!("{} · {reference}", fixture.name),
                (n, _) => format!("{n} sélectionnées"),
            });
            ui.label(RichText::new(fixture.fixture_type.display_name()).weak());
        });
        ui.horizontal(|ui| {
            if ui.button("Full").on_hover_text("Dimmer à 100 %, shutter ouvert").clicked() {
                console.full();
            }
            if ui.button("Relâcher").on_hover_text("Rend la sélection à ses valeurs par défaut").clicked() {
                console.release_selection();
            }
        });
        ui.separator();

        // Une ligne par fonction : les 10 « Dimmer N » d'un Sunstrip entier n'en font qu'une.
        let mut rows: Vec<(usize, Parameter)> = Vec::new();
        for (index, parameter) in console.show.patch.parameters_of(reference) {
            if !rows.iter().any(|(_, p)| p.base_name == parameter.base_name) {
                rows.push((index, parameter.clone()));
            }
        }
        let mut groups: Vec<FeatureGroup> = rows.iter().map(|(_, p)| p.attribute.group()).collect();
        groups.sort();
        groups.dedup();

        // Le curseur prend la place restante : le panneau central varie avec la fenêtre.
        let slider_width = (ui.available_width() - NAME_WIDTH - VALUE_WIDTH - MENU_WIDTH - 70.0).clamp(100.0, 320.0);
        egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
            for group in groups {
                ui.add_space(6.0);
                ui.label(RichText::new(group.label()).strong().size(15.0));
                egui::Grid::new(("attributes", group)).num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
                    for (index, parameter) in rows.iter().filter(|(_, p)| p.attribute.group() == group) {
                        let row = Row { index: *index, parameter, reference, snapshot, display, slider_width };
                        row.show(ui, console);
                        ui.end_row();
                    }
                });
            }
        });
    }
}

/// Ligne sélectionnable du patch (numéro et nom). Renvoie `true` si elle a été cliquée.
fn target_row(ui: &mut egui::Ui, selection: &[Target], target: Target, name: &str) -> bool {
    let selected = selection.contains(&target);
    let id_clicked = ui.selectable_label(selected, target.to_string()).clicked();
    let name_clicked = ui.selectable_label(selected, name).clicked();
    id_clicked || name_clicked
}

/// Une ligne de réglage d'attribut, pour la cible de référence de la sélection.
struct Row<'a> {
    index: usize,
    parameter: &'a Parameter,
    reference: Target,
    snapshot: &'a Snapshot,
    display: DisplayMode,
    slider_width: f32,
}

impl Row<'_> {
    fn show(&self, ui: &mut egui::Ui, console: &mut Console) {
        let parameter = self.parameter;
        let key = ParamKey { fixture: self.reference.fixture, parameter: self.index };
        let programmed = self.snapshot.attributes.get(&key).copied();
        let value = programmed.unwrap_or(parameter.default);

        // Fixture entière : nom de la fonction ; sous-fixture : nom complet ("Dimmer 3").
        let label = if self.reference.head.is_some() { &parameter.name } else { &parameter.base_name };
        let name = RichText::new(label);
        let name = if programmed.is_some() { name.color(PROGRAMMED) } else { name };
        // Largeurs fixes : les colonnes restent alignées d'un groupe d'attributs à l'autre.
        let name_label = fixed_width(ui, NAME_WIDTH, |ui| ui.label(name));

        let mut percent = value as f32 / u16::MAX as f32 * 100.0;
        ui.spacing_mut().slider_width = self.slider_width;
        let slider = egui::Slider::new(&mut percent, 0.0..=100.0).show_value(false);
        if ui.add(slider).labelled_by(name_label.id).changed() {
            console.set_like(parameter, (percent / 100.0 * u16::MAX as f32).round() as u16);
        }

        let mut text = match self.display {
            DisplayMode::Percent => format!("{percent:.1} %"),
            DisplayMode::Dmx if parameter.resolution_bits() > 8 => format!("{}/{}", value >> 8, value & 0xFF),
            DisplayMode::Dmx => (value >> 8).to_string(),
        };
        if let Some(cap) = parameter.capability_at(value)
            && let Some((start, end)) = cap.angle
        {
            let t = (value - cap.range.start()) as f32 / (cap.range.end() - cap.range.start()).max(1) as f32;
            text.push_str(&format!("  {:.0}°", start + (end - start) * t));
        }
        fixed_width(ui, VALUE_WIDTH, |ui| ui.monospace(text));

        self.capability_menu(ui, console, value, name_label.id);
    }

    fn capability_menu(&self, ui: &mut egui::Ui, console: &mut Console, value: u16, label: egui::Id) {
        let parameter = self.parameter;
        if parameter.capabilities.len() < 2 {
            ui.label("");
            return;
        }
        let current = parameter.capability_at(value);
        let mut chosen = None;
        ui.horizontal(|ui| {
            if let Some(rgb) = current.and_then(preset_color) {
                swatch(ui, rgb);
            }
            egui::ComboBox::from_id_salt(("capability", self.index))
                .width(MENU_WIDTH)
                .selected_text(current.map_or("—", |c| c.label.as_str()))
                .height(400.0)
                .show_ui(ui, |ui| {
                    for cap in parameter.capabilities.iter().filter(|c| !c.hidden) {
                        ui.horizontal(|ui| {
                            if let Some(rgb) = preset_color(cap) {
                                swatch(ui, rgb);
                            }
                            let selected = current.is_some_and(|c| c.range == cap.range);
                            if ui.selectable_label(selected, &cap.label).clicked() {
                                chosen = Some(cap.select_value());
                            }
                        });
                    }
                })
                .response
                .labelled_by(label);
        });
        if let Some(value) = chosen {
            console.set_like(parameter, value);
        }
    }
}

fn fixed_width(ui: &mut egui::Ui, width: f32, add: impl FnOnce(&mut egui::Ui) -> egui::Response) -> egui::Response {
    let size = vec2(width, ui.spacing().interact_size.y);
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    ui.allocate_ui_with_layout(size, layout, |ui| {
        ui.set_min_width(width);
        add(ui)
    })
    .inner
}

fn preset_color(cap: &Capability) -> Option<[u8; 3]> {
    match &cap.kind {
        CapabilityKind::ColorPreset { colors } => colors.first().copied(),
        _ => None,
    }
}

fn swatch(ui: &mut egui::Ui, [r, g, b]: [u8; 3]) {
    let (rect, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, Color32::from_rgb(r, g, b));
}

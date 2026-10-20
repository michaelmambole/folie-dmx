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
use crate::theme::{self, Source};

const NAME_WIDTH: f32 = 140.0;
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
    /// Couleur visée dans le sélecteur (teinte, saturation, luminosité).
    picker: [f32; 3],
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
            picker: [0.0, 0.0, 1.0],
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

    /// Le Programmer : réglage des attributs de la sélection.
    pub fn attribute_panel(
        &mut self,
        ui: &mut egui::Ui,
        console: &mut Console,
        snapshot: &Snapshot,
        display: DisplayMode,
    ) {
        let Some(&reference) = console.selection().first() else {
            ui.add_space(24.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Aucune sélection").size(18.0).family(theme::medium()));
                ui.add_space(4.0);
                ui.label(RichText::new("Cliquez des fixtures dans la fenêtre Patch, ou tapez une commande :").color(theme::MUTED));
                ui.monospace("Fixture 1 Thru 4 At 50");
            });
            return;
        };
        let Some(fixture) = console.show.patch.get(reference.fixture).cloned() else { return };

        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                let count = console.selection().len();
                let title = match (count, reference.head) {
                    (1, None) => fixture.name.clone(),
                    (1, Some(_)) => format!("{} · {reference}", fixture.name),
                    (n, _) => format!("{n} sélectionnées"),
                };
                ui.label(RichText::new(title).size(18.0).family(theme::medium()));
                ui.label(RichText::new(fixture.fixture_type.display_name()).color(theme::MUTED));
                ui.add_space(12.0);
                if ui.add(theme::primary_button("Full")).on_hover_text("Dimmer à 100 %, shutter ouvert").clicked() {
                    console.full();
                }
                if ui.button("Relâcher").on_hover_text("Rend la sélection à ses valeurs par défaut").clicked() {
                    console.release_selection();
                }
            });
            selection_chips(ui, console.selection());
        });
        ui.add_space(6.0);
        theme::legend(ui);
        ui.add_space(4.0);

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

        // Paramètres animés par un effet du programmer.
        let animated: BTreeSet<ParamKey> = console
            .programmer_effects()
            .iter()
            .flat_map(|e| folie_control::effects::instance(e, 0, &console.show.patch).lines)
            .flat_map(|l| l.params.into_iter().map(|(k, _)| k))
            .collect();

        let bar_width = (ui.available_width() - NAME_WIDTH - MENU_WIDTH - 60.0).clamp(140.0, 420.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for group in groups {
                theme::card(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    theme::section(ui, group.label());
                    if group == FeatureGroup::Color && has_color(&rows) {
                        let current = current_color(console, snapshot, &fixture, reference);
                        if let Some(rgb) = crate::color_picker::color_picker(ui, &mut self.picker, current) {
                            console.apply_color(rgb);
                        }
                        ui.add_space(8.0);
                    }
                    egui::Grid::new(("attributes", group)).num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                        for (index, parameter) in rows.iter().filter(|(_, p)| p.attribute.group() == group) {
                            let row = Row { index: *index, parameter, fixture: &fixture, reference, snapshot, display, bar_width };
                            row.show(ui, console, &animated);
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(6.0);
            }
        });
    }
}

/// Numéros des cibles sélectionnées, en pastilles jaunes ; au-delà de 16, un compteur.
fn selection_chips(ui: &mut egui::Ui, selection: &[Target]) {
    const SHOWN: usize = 16;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(4.0, 4.0);
        for target in selection.iter().take(SHOWN) {
            egui::Frame::new()
                .stroke(egui::Stroke::new(1.0, theme::SELECTION))
                .corner_radius(4)
                .inner_margin(egui::Margin::symmetric(6, 1))
                .show(ui, |ui| ui.label(RichText::new(target.to_string()).monospace().size(12.0).color(theme::SELECTION)));
        }
        if selection.len() > SHOWN {
            ui.label(RichText::new(format!("+{}", selection.len() - SHOWN)).color(theme::MUTED));
        }
    });
}

/// La fixture sait-elle faire de la couleur (mélange ou roue) ?
fn has_color(rows: &[(usize, Parameter)]) -> bool {
    rows.iter().any(|(_, p)| matches!(p.attribute, folie_fixture::Attribute::ColorMix(_) | folie_fixture::Attribute::ColorMacro))
}

/// Couleur produite en ce moment par la cible de référence, d'après la sortie DMX.
fn current_color(console: &Console, snapshot: &Snapshot, fixture: &PatchedFixture, target: Target) -> Option<[u8; 3]> {
    let values = console
        .show
        .patch
        .parameters_of(target)
        .into_iter()
        .filter_map(|(_, p)| live_value(snapshot, fixture, p).map(|v| (p.name.clone(), v)))
        .collect();
    console.target_color(target, &values)
}

/// Valeur réellement envoyée pour un paramètre, relue dans la sortie DMX.
fn live_value(snapshot: &Snapshot, fixture: &PatchedFixture, parameter: &Parameter) -> Option<u16> {
    let universe = snapshot.output.get(fixture.universe)?;
    let byte = |offset: u16| universe.get(fixture.address + offset).ok();
    match parameter.offsets.as_slice() {
        [coarse] => byte(*coarse).map(|v| v as u16 * 257),
        [coarse, fine, ..] => Some(((byte(*coarse)? as u16) << 8) | byte(*fine)? as u16),
        [] => None,
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
    fixture: &'a PatchedFixture,
    reference: Target,
    snapshot: &'a Snapshot,
    display: DisplayMode,
    bar_width: f32,
}

impl Row<'_> {
    fn show(&self, ui: &mut egui::Ui, console: &mut Console, animated: &BTreeSet<ParamKey>) {
        let parameter = self.parameter;
        let key = ParamKey { fixture: self.reference.fixture, parameter: self.index };
        let source = if animated.contains(&key) {
            Source::Effect
        } else if console.programmer().contains_key(&key) {
            Source::Programmer
        } else if self.snapshot.stage.contains_key(&key) {
            Source::Executor
        } else {
            Source::Default
        };
        // Ce qui sort vraiment (effets compris) ; à défaut, la valeur du programmer.
        let value = live_value(self.snapshot, self.fixture, parameter)
            .or_else(|| console.programmer().get(&key).copied())
            .unwrap_or(parameter.default);

        // Fixture entière : nom de la fonction ; sous-fixture : nom complet ("Dimmer 3").
        let label = if self.reference.head.is_some() { &parameter.name } else { &parameter.base_name };
        let name_color = if source == Source::Default { theme::TEXT } else { source.color() };
        let name_label = fixed_width(ui, NAME_WIDTH, |ui| {
            ui.add(egui::Label::new(RichText::new(label).color(name_color)).sense(Sense::click()))
        });

        let mut level = value as f32 / u16::MAX as f32;
        let text = self.value_text(value);
        let help = "Glisser : régler (Maj : fin) · clic : placer · Option-clic : relâcher";
        let bar = theme::value_bar(ui, &mut level, source.color(), &text, self.bar_width)
            .labelled_by(name_label.id)
            .on_hover_text(help);
        let alt = ui.input(|i| i.modifiers.alt);
        if alt && (bar.clicked() || name_label.clicked()) {
            console.release_like(parameter);
        } else if bar.changed() {
            console.set_like(parameter, (level * u16::MAX as f32).round() as u16);
        }
        self.capability_menu(ui, console, value, name_label.id);
    }

    /// Valeur lisible : degrés pour un angle, sinon % ou DMX selon l'affichage choisi.
    fn value_text(&self, value: u16) -> String {
        let parameter = self.parameter;
        if let Some(cap) = parameter.capability_at(value)
            && let Some((start, end)) = cap.angle
        {
            let t = (value - cap.range.start()) as f32 / (cap.range.end() - cap.range.start()).max(1) as f32;
            return format!("{:.0}°", start + (end - start) * t);
        }
        match self.display {
            DisplayMode::Percent => format!("{:.1} %", value as f32 / u16::MAX as f32 * 100.0),
            DisplayMode::Dmx if parameter.resolution_bits() > 8 => format!("{}/{}", value >> 8, value & 0xFF),
            DisplayMode::Dmx => (value >> 8).to_string(),
        }
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

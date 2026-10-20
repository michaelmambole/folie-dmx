use std::collections::{BTreeMap, BTreeSet};
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
    /// Cadres du Programmer repliés (Position, Couleur…), pour la session.
    collapsed: BTreeSet<FeatureGroup>,
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
            collapsed: BTreeSet::new(),
        }
    }

    /// Liste du patch et formulaire d'ajout. Renvoie `true` si le patch a changé.
    pub fn patch_panel(&mut self, ui: &mut egui::Ui, library: &Library, console: &mut Console) -> bool {
        let mut changed = false;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section(ui, "Ajouter des fixtures");
            if let Some(ids) = self.add_form(ui, library, &mut console.show.patch) {
                console.select(ids.into_iter().map(Target::whole), SelectMode::Replace);
                changed = true;
            }
            if let Some((ok, text)) = &self.message {
                ui.colored_label(if *ok { theme::GO } else { theme::DANGER }, text);
            }
        });
        ui.add_space(6.0);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            universe_map(ui, console);
        });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            theme::section(ui, "Patch");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let selected: BTreeSet<FixtureId> = console.selection().iter().map(|t| t.fixture).collect();
                let delete = if selected.is_empty() {
                    egui::Button::new("Supprimer")
                } else {
                    let text = RichText::new(format!("Supprimer ({})", selected.len())).color(Color32::from_rgb(255, 138, 132));
                    egui::Button::new(text).stroke(egui::Stroke::new(1.0, theme::DANGER))
                };
                let help = "Retire les fixtures sélectionnées du patch";
                if ui.add_enabled(!selected.is_empty(), delete).on_hover_text(help).clicked() {
                    for id in selected {
                        console.show.patch.remove(id);
                    }
                    self.message = None;
                    changed = true;
                }
                if ui.button("Aucune").clicked() {
                    console.clear_selection();
                }
                if ui.button("Tout sélectionner").clicked() {
                    let all: Vec<Target> = console.show.patch.fixtures().iter().map(|f| Target::whole(f.id)).collect();
                    console.select(all, SelectMode::Replace);
                }
            });
        });
        ui.add_space(2.0);
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
        let combo_width = (ui.available_width() - 80.0).clamp(200.0, 420.0);
        egui::Grid::new("add_form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            let label = ui.label(RichText::new("Type").color(theme::MUTED));
            egui::ComboBox::from_id_salt("add_type")
                .width(combo_width)
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

            let label = ui.label(RichText::new("Mode").color(theme::MUTED));
            let mode_label = |m: &folie_fixture::Mode| format!("{} ({} canaux)", m.name, m.footprint);
            egui::ComboBox::from_id_salt("add_mode")
                .width(combo_width)
                .selected_text(mode_label(&fixture_type.modes[form.mode]))
                .show_ui(ui, |ui| {
                    for (i, m) in fixture_type.modes.iter().enumerate() {
                        ui.selectable_value(&mut form.mode, i, mode_label(m));
                    }
                })
                .response
                .labelled_by(label.id);
            ui.end_row();
        });
        ui.add_space(4.0);

        let footprint = fixture_type.modes[form.mode].footprint;
        let mut patch_clicked = false;
        ui.horizontal(|ui| {
            let muted = |text: &str| RichText::new(text).color(theme::MUTED);
            let label = ui.label(muted("Quantité"));
            ui.add(egui::DragValue::new(&mut form.quantity).range(1..=128)).labelled_by(label.id);
            ui.add_space(6.0);
            let label = ui.label(muted("Fixture n°"));
            ui.add(egui::DragValue::new(&mut form.first_id).range(1..=99_999)).labelled_by(label.id);
            ui.add_space(6.0);
            let address_label = ui.label(muted("Adresse"));
            ui.checkbox(&mut form.auto_address, "auto");
            if form.auto_address {
                let needed = footprint.saturating_mul(form.quantity as u16);
                match patch.next_free_address(0, needed) {
                    Some(address) => {
                        form.address = address;
                        ui.monospace(format!("1.{address:03}"));
                    }
                    None => {
                        ui.colored_label(theme::DANGER, "pas assez de place");
                    }
                }
            } else {
                ui.label("1.");
                ui.add(egui::DragValue::new(&mut form.address).range(1..=512)).labelled_by(address_label.id);
            }
        });
        ui.horizontal(|ui| {
            let total = footprint as u32 * form.quantity;
            let text = format!("{} × {footprint} = {total} canaux", form.quantity);
            ui.label(RichText::new(text).size(11.5).color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                patch_clicked = ui.add(theme::primary_button("Patcher")).clicked();
            });
        });

        if !patch_clicked {
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
        // Fixtures (et cellules) qui ont des valeurs dans le programmer.
        let in_programmer: BTreeSet<Target> = console
            .programmer()
            .keys()
            .filter_map(|key| {
                let fixture = patch.get(key.fixture)?;
                let head = fixture.mode().parameters.get(key.parameter)?.head;
                Some([Target::whole(key.fixture), Target { fixture: key.fixture, head }])
            })
            .flatten()
            .collect();
        let range = |universe: usize, first: u16, count: u16| {
            let last = first + count.max(1) - 1;
            format!("{}.{first:03}–{last:03}", universe + 1)
        };
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            if patch.fixtures().is_empty() {
                ui.label(RichText::new("Aucune fixture patchée.").color(theme::MUTED));
            }
            for f in patch.fixtures() {
                let heads = &f.mode().heads;
                let expanded = self.expanded.contains(&f.id);
                let target = Target::whole(f.id);
                let line = PatchLine {
                    target,
                    name: &f.name,
                    detail: if heads.is_empty() {
                        f.fixture_type.manufacturer.clone()
                    } else {
                        format!("{} · {} cellules", f.fixture_type.manufacturer, heads.len())
                    },
                    mode: Some(format!("{} ch", f.mode().footprint)),
                    address: range(f.universe, f.address, f.mode().footprint),
                    selected: selection.contains(&target),
                    programmer: in_programmer.contains(&target),
                    expander: (!heads.is_empty()).then_some(expanded),
                };
                match line.show(ui) {
                    LineClick::Row => clicked = Some(target),
                    LineClick::Expander => {
                        if !self.expanded.remove(&f.id) {
                            self.expanded.insert(f.id);
                        }
                    }
                    LineClick::None => {}
                }
                rows.push(target);
                if !expanded {
                    continue;
                }
                for (head, key) in heads.iter().enumerate() {
                    let target = Target { fixture: f.id, head: Some(head) };
                    let offsets: Vec<u16> = f
                        .mode()
                        .parameters
                        .iter()
                        .filter(|p| p.head == Some(head))
                        .flat_map(|p| p.offsets.iter().copied())
                        .collect();
                    let first = offsets.iter().copied().min().unwrap_or(0);
                    let last = offsets.iter().copied().max().unwrap_or(0);
                    let line = PatchLine {
                        target,
                        name: &format!("cellule {key}"),
                        detail: String::new(),
                        mode: None,
                        address: range(f.universe, f.address + first, last - first + 1),
                        selected: selection.contains(&target),
                        programmer: in_programmer.contains(&target),
                        expander: None,
                    };
                    if line.show(ui) == LineClick::Row {
                        clicked = Some(target);
                    }
                    rows.push(target);
                }
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
                let help = "Marque la sélection « relâchée » : Store l'enregistre, et la séquence cesse de la tenir à cette cue";
                if ui.button("Release").on_hover_text(help).clicked() {
                    // Seule erreur possible : aucune sélection, et ce panneau n'apparaît qu'avec une sélection.
                    let _ = console.release_in_programmer();
                }
            });
            selection_chips(ui, console.selection());
            effect_chips(ui, console);
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

        // Paramètres animés par un effet du programmer : quel effet, et sa plage.
        let mut animated: BTreeMap<ParamKey, EffectMark> = BTreeMap::new();
        for effect in console.programmer_effects() {
            let instance = folie_control::effects::instance(effect, 0, &console.show.patch);
            for (line, played) in effect.lines.iter().zip(instance.lines) {
                for (key, _) in played.params {
                    let mark = EffectMark { name: effect.name.clone(), low: line.low, high: line.high, relative: line.relative };
                    animated.insert(key, mark);
                }
            }
        }

        let bar_width = (ui.available_width() - NAME_WIDTH - MENU_WIDTH - 60.0).clamp(140.0, 420.0);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for group in groups {
                theme::card(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    let open = !self.collapsed.contains(&group);
                    if theme::collapsible_section(ui, group.label(), open) && !self.collapsed.remove(&group) {
                        self.collapsed.insert(group);
                    }
                    if !open {
                        return;
                    }
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

/// Effets du programmer en pastilles violettes ; un clic arrête l'effet.
fn effect_chips(ui: &mut egui::Ui, console: &mut Console) {
    if console.programmer_effects().is_empty() {
        return;
    }
    let mut stop = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("Effets").size(11.5).color(theme::MUTED));
        for (index, effect) in console.programmer_effects().iter().enumerate() {
            let text = RichText::new(format!("{}  ×", effect.name)).size(12.0).color(theme::TEXT);
            let chip = egui::Button::new(text)
                .fill(theme::EFFECT.gamma_multiply(0.12))
                .stroke(egui::Stroke::new(1.0, theme::EFFECT));
            if ui.add(chip).on_hover_text(format!("Arrêter « {} »", effect.name)).clicked() {
                stop = Some(index);
            }
        }
    });
    if let Some(index) = stop {
        console.remove_effect(index);
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
#[derive(PartialEq)]
enum LineClick {
    None,
    Row,
    Expander,
}

/// Une ligne de la liste du patch : chevron des cellules, numéro, nom et marque,
/// mode, plage d'adresses, point rouge si le programmer tient des valeurs.
struct PatchLine<'a> {
    target: Target,
    name: &'a str,
    detail: String,
    mode: Option<String>,
    address: String,
    selected: bool,
    programmer: bool,
    /// `Some(déplié)` pour une fixture à cellules.
    expander: Option<bool>,
}

impl PatchLine<'_> {
    fn show(&self, ui: &mut egui::Ui) -> LineClick {
        let mut result = LineClick::None;
        let fill = if self.selected { theme::SELECTION.gamma_multiply(0.10) } else { Color32::TRANSPARENT };
        // Le fond de la ligne est enregistré avant son contenu : le chevron garde ses clics.
        let row = ui.scope_builder(egui::UiBuilder::new().id_salt(("patch-line", self.target)).sense(Sense::click()), |ui| {
            egui::Frame::new().fill(fill).corner_radius(theme::RADIUS).inner_margin(egui::Margin::symmetric(6, 3)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (rect, response) = ui.allocate_exact_size(vec2(14.0, 18.0), Sense::click());
                    if let Some(open) = self.expander {
                        let color = if response.hovered() { theme::TEXT } else { theme::MUTED };
                        let c = rect.center();
                        let points = if open {
                            vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
                        } else {
                            vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
                        };
                        ui.painter().add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
                        let help = if open { "Masquer les sous-fixtures" } else { "Afficher les sous-fixtures" };
                        let response = response.on_hover_text(help);
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, help));
                        if response.clicked() {
                            result = LineClick::Expander;
                        }
                    }
                    let (stroke, color) =
                        if self.selected { (theme::SELECTION, theme::SELECTION) } else { (theme::LINE, theme::TEXT) };
                    egui::Frame::new()
                        .fill(theme::SURFACE_2)
                        .stroke(egui::Stroke::new(1.0, stroke))
                        .corner_radius(5)
                        .inner_margin(egui::Margin::symmetric(4, 0))
                        .show(ui, |ui| {
                            let number = RichText::new(self.target.to_string()).monospace().color(color);
                            ui.add_sized([36.0, 18.0], egui::Label::new(number).selectable(false));
                        });
                    let name = RichText::new(self.name).color(if self.selected { theme::SELECTION } else { theme::TEXT });
                    ui.add(egui::Label::new(name).selectable(false).truncate());
                    if !self.detail.is_empty() {
                        ui.add(egui::Label::new(RichText::new(&self.detail).size(11.0).color(theme::MUTED)).selectable(false));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (dot, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        if self.programmer {
                            ui.painter().circle_filled(dot.center(), 3.5, theme::PROGRAMMER);
                        }
                        let address = RichText::new(&self.address).monospace().size(12.0).color(theme::MUTED);
                        ui.add_sized([92.0, 18.0], egui::Label::new(address).selectable(false));
                        if let Some(mode) = &self.mode {
                            egui::Frame::new()
                                .stroke(egui::Stroke::new(1.0, theme::LINE))
                                .corner_radius(4)
                                .inner_margin(egui::Margin::symmetric(5, 0))
                                .show(ui, |ui| {
                                    ui.add(egui::Label::new(RichText::new(mode).size(11.0).color(theme::MUTED)).selectable(false));
                                });
                        }
                    });
                });
            });
        });
        if row.response.clicked() && result == LineClick::None {
            result = LineClick::Row;
        }
        result
    }
}

/// Carte de l'univers 1 : chaque fixture occupe sa plage d'adresses, la sélection
/// en jaune. Clic : sélectionne la fixture (Cmd-clic : ajoute ou retire).
fn universe_map(ui: &mut egui::Ui, console: &mut Console) {
    let patch = &console.show.patch;
    let used: u32 = patch.fixtures().iter().filter(|f| f.universe == 0).map(|f| f.mode().footprint as u32).sum();
    ui.horizontal(|ui| {
        theme::section(ui, "Univers 1");
        let count = patch.fixtures().len();
        let text = format!("{used} / 512 canaux · {count} fixture{}", if count > 1 { "s" } else { "" });
        ui.label(RichText::new(text).size(11.5).color(theme::MUTED));
    });
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    ui.painter().rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let x = |channel: u32| rect.left() + rect.width() * channel as f32 / 512.0;
    let mut clicked = None;
    for f in patch.fixtures().iter().filter(|f| f.universe == 0) {
        let start = f.address as u32 - 1;
        let end = start + f.mode().footprint as u32;
        let segment = egui::Rect::from_x_y_ranges(x(start)..=(x(end) - 1.0).max(x(start) + 1.0), rect.y_range());
        let response = ui.interact(segment, ui.scope_id().with(("universe", f.id)), Sense::click());
        let selected = console.selection().iter().any(|t| t.fixture == f.id);
        let color = if selected {
            theme::SELECTION
        } else if response.hovered() {
            theme::MUTED
        } else {
            theme::DEFAULT
        };
        ui.painter().rect_filled(segment, 2.0, color);
        let last = f.address + f.mode().footprint - 1;
        let text = format!("{} · {} : 1.{:03}–{last:03}", f.id, f.name, f.address);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &text));
        if response.on_hover_text(&text).clicked() {
            clicked = Some(f.id);
        }
    }
    if let Some(id) = clicked {
        let mode = if ui.input(|i| i.modifiers.command) { SelectMode::Toggle } else { SelectMode::Replace };
        console.select([Target::whole(id)], mode);
    }
}

/// Effet du programmer qui anime un paramètre, pour l'afficher dans sa ligne.
struct EffectMark {
    name: String,
    low: f32,
    high: f32,
    relative: bool,
}

impl EffectMark {
    /// « Cercle · ±10 % », « Cercle · -5 à +10 % », « Vague · 0–100 % ».
    fn summary(&self) -> String {
        let range = match self.relative {
            true if self.low == -self.high => format!("±{} %", self.high.abs()),
            true => format!("{:+} à {:+} %", self.low, self.high),
            false => format!("{}–{} %", self.low, self.high),
        };
        format!("{} · {range}", self.name)
    }
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
    fn show(&self, ui: &mut egui::Ui, console: &mut Console, animated: &BTreeMap<ParamKey, EffectMark>) {
        let parameter = self.parameter;
        let key = ParamKey { fixture: self.reference.fixture, parameter: self.index };
        let effect = animated.get(&key);
        let source = if effect.is_some() {
            Source::Effect
        } else if console.programmer().contains_key(&key) || console.released().contains(&key) {
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
        let text = if console.released().contains(&key) { "relâché".to_string() } else { self.value_text(value) };
        let help = match effect {
            Some(e) if e.relative => "Effet relatif : glisser déplace le centre autour duquel il tourne · Option-clic : relâcher",
            _ => "Glisser : régler (Maj : fin) · clic : placer · Option-clic : relâcher",
        };
        // Plage de l'effet, en clair derrière la valeur ; relative, elle suit la valeur de base.
        let band = effect.map(|e| {
            let base = if e.relative {
                console.programmer().get(&key).copied().unwrap_or(parameter.default) as f32 / u16::MAX as f32
            } else {
                0.0
            };
            (base + e.low / 100.0, base + e.high / 100.0)
        });
        let bar = ui
            .horizontal(|ui| {
                let bar = theme::value_bar(ui, &mut level, source.color(), &text, self.bar_width, band)
                    .labelled_by(name_label.id)
                    .on_hover_text(help);
                if let Some(e) = effect {
                    ui.label(RichText::new(e.summary()).size(11.0).color(theme::EFFECT));
                }
                bar
            })
            .inner;
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

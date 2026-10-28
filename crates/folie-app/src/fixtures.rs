use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;

use eframe::egui::{self, Color32, Rect, RichText, Sense, pos2, vec2};
use folie_control::{Console, SelectMode};
use folie_engine::Snapshot;
use folie_fixture::{
    Capability, CapabilityKind, FeatureGroup, FixtureId, Library, ParamKey, Parameter, Patch,
    PatchedFixture, Target,
};

use crate::app::DisplayMode;
use crate::library_ui::{self, ImportDialog, ImportOutcome};
use crate::ofl_online::{self, OflSite, OnlineSearch, SearchOutcome};
use crate::theme::{self, Source};

const NAME_WIDTH: f32 = 140.0;
const MENU_WIDTH: f32 = 210.0;

pub(crate) struct AddForm {
    pub(crate) fixture_type: usize,
    mode: usize,
    pub(crate) quantity: u32,
    pub(crate) first_id: u32,
    pub(crate) auto_address: bool,
    pub(crate) address: u16,
}

pub struct FixturesView {
    last_clicked: Option<Target>,
    pub(crate) form: AddForm,
    /// Formulaire d'ajout déplié.
    form_open: bool,
    /// Dernier résultat d'une opération de patch : (succès, message).
    message: Option<(bool, String)>,
    /// Couleur visée dans le sélecteur (teinte, saturation, luminosité).
    picker: [f32; 3],
    /// Cadres du Programmer repliés (Position, Couleur…), pour la session.
    collapsed: BTreeSet<FeatureGroup>,
    /// Fiche en cours d'import.
    pub(crate) import: Option<ImportDialog>,
    /// Fiche tout juste importée : la bibliothèque est à recharger.
    imported: Option<String>,
    /// Recherche dans Open Fixture Library en cours.
    pub(crate) online: Option<OnlineSearch>,
    /// Le site OFL (un faux site dans les tests).
    pub(crate) ofl_source: Arc<dyn ofl_online::Source>,
}

impl FixturesView {
    pub fn new() -> Self {
        Self {
            last_clicked: None,
            form: AddForm {
                fixture_type: 0,
                mode: 0,
                quantity: 1,
                first_id: 1,
                auto_address: true,
                address: 1,
            },
            form_open: true,
            message: None,
            picker: [0.0, 0.0, 1.0],
            collapsed: BTreeSet::new(),
            import: None,
            imported: None,
            online: None,
            ofl_source: Arc::new(OflSite::new()),
        }
    }

    /// Préparation du patch : ajout, tableau des fixtures, changement d'adresse et
    /// suppression. Renvoie `true` si le patch a changé.
    pub fn patch_panel(
        &mut self,
        ui: &mut egui::Ui,
        library: &Library,
        user_dir: Option<&Path>,
        console: &mut Console,
    ) -> bool {
        let mut changed = false;
        if let Some(search) = self.online.as_mut() {
            match search.show(ui.ctx()) {
                SearchOutcome::Open => {}
                SearchOutcome::Cancelled => self.online = None,
                SearchOutcome::Chosen(json, manufacturer) => {
                    self.online = None;
                    match ImportDialog::from_json(json, None, library) {
                        Ok(dialog) => self.import = Some(dialog.with_manufacturer(&manufacturer)),
                        Err(e) => self.message = Some((false, e)),
                    }
                }
            }
        }
        if let (Some(dialog), Some(dir)) = (self.import.as_mut(), user_dir) {
            match dialog.show(ui.ctx(), library, dir) {
                ImportOutcome::Open => {}
                ImportOutcome::Cancelled => self.import = None,
                ImportOutcome::Imported(key) => {
                    self.import = None;
                    self.message = Some((true, format!("Fiche {key} rangée dans votre bibliothèque")));
                    self.imported = Some(key);
                }
            }
        }
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            if theme::collapsible_section(ui, "Ajouter des fixtures", self.form_open) {
                self.form_open = !self.form_open;
            }
            if self.form_open
                && let Some(ids) = self.add_form(ui, library, &mut console.show.patch)
            {
                console.select(ids.into_iter().map(Target::whole), SelectMode::Replace);
                changed = true;
            }
            if self.form_open {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    let unavailable = "Bibliothèque personnelle indisponible";
                    let online = ui
                        .add_enabled(user_dir.is_some(), egui::Button::new("Rechercher en ligne…"))
                        .on_hover_text("Cherche un appareil dans Open Fixture Library et importe sa fiche")
                        .on_disabled_hover_text(unavailable);
                    if online.clicked() {
                        self.online = Some(OnlineSearch::new(self.ofl_source.clone()));
                    }
                    let import = ui
                        .add_enabled(user_dir.is_some(), egui::Button::new("Importer un fichier…"))
                        .on_hover_text("Ajoute une fiche Open Fixture Library (.json) ou GDTF (.gdtf) à votre bibliothèque")
                        .on_disabled_hover_text(unavailable);
                    if import.clicked()
                        && let Some(path) = library_ui::pick_file()
                    {
                        self.begin_import(&path, library);
                    }
                });
            }
        });
        ui.add_space(6.0);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            universe_map(ui, console);
        });
        ui.add_space(6.0);

        // Fixtures entières sélectionnées, dans l'ordre de sélection.
        let mut selected: Vec<FixtureId> = Vec::new();
        for target in console.selection() {
            if !selected.contains(&target.fixture) {
                selected.push(target.fixture);
            }
        }
        let mut readdress = None;
        ui.horizontal(|ui| {
            theme::section(ui, "Patch");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let delete = if selected.is_empty() {
                    egui::Button::new("Supprimer")
                } else {
                    let text = RichText::new(format!("Supprimer ({})", selected.len())).color(Color32::from_rgb(255, 138, 132));
                    egui::Button::new(text).stroke(egui::Stroke::new(1.0, theme::DANGER))
                };
                let help = "Retire les fixtures sélectionnées du patch";
                if ui.add_enabled(!selected.is_empty(), delete).on_hover_text(help).clicked() {
                    for id in &selected {
                        console.show.patch.remove(*id);
                    }
                    self.message = None;
                    changed = true;
                }
                let help = "Nouvelle adresse de départ pour la sélection (aussi : clic droit sur une ligne)";
                let button = ui.add_enabled(!selected.is_empty(), egui::Button::new("Changer l'adresse…")).on_hover_text(help);
                if let Some(&first) = selected.first()
                    && let Some(fixture) = console.show.patch.get(first)
                {
                    let popup = egui::Popup::menu(&button);
                    readdress = address_menu(ui, popup, None, selected.clone(), fixture);
                }
            });
        });
        if let Some((ok, text)) = &self.message {
            ui.colored_label(if *ok { theme::GO } else { theme::DANGER }, text);
        }
        ui.add_space(2.0);
        readdress = readdress.or(self.patch_table(ui, console, &selected));
        if let Some(request) = readdress {
            changed |= self.readdress(ui, console, request);
        }
        changed
    }

    /// Applique un changement d'adresse validé dans un menu. Renvoie `true` s'il a eu lieu.
    fn readdress(&mut self, ui: &egui::Ui, console: &mut Console, (menu_id, ids, universe, address): Readdress) -> bool {
        match console.readdress(&ids, universe, address) {
            Ok(()) => {
                let at = format!("{}.{address:03}", universe + 1);
                let text = match ids.as_slice() {
                    [id] => format!("Fixture {id} déplacée en {at}"),
                    _ => format!("{} fixtures déplacées à la suite à partir de {at}", ids.len()),
                };
                self.message = Some((true, text));
                ui.data_mut(|d| d.remove::<AddressField>(menu_id));
                egui::Popup::close_all(ui.ctx());
                true
            }
            Err(e) => {
                ui.data_mut(|d| d.get_temp_mut_or_default::<AddressField>(menu_id).error = Some(format!("Impossible : {e}")));
                false
            }
        }
    }

    /// Fiche tout juste importée, une seule fois : l'appelant recharge la bibliothèque
    /// puis la sélectionne avec [`Self::select_type`].
    pub fn take_imported(&mut self) -> Option<String> {
        self.imported.take()
    }

    pub fn select_type(&mut self, library: &Library, key: &str) {
        if let Some(index) = library.fixtures.iter().position(|f| f.key == key) {
            self.form.fixture_type = index;
            self.form.mode = 0;
            self.form_open = true;
        }
    }

    /// Ouvre la boîte d'import pour ce fichier (le bouton passe par le sélecteur de fichier).
    pub fn begin_import(&mut self, path: &Path, library: &Library) {
        match ImportDialog::from_file(path, library) {
            Ok(dialog) => self.import = Some(dialog),
            Err(e) => self.message = Some((false, e)),
        }
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
                .selected_text(type_label(library, &fixture_type))
                .show_ui(ui, |ui| {
                    for (i, t) in library.fixtures.iter().enumerate() {
                        if ui.selectable_value(&mut form.fixture_type, i, type_label(library, t)).changed() {
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
        // Conflit signalé avant de patcher : numéro déjà pris, adresses occupées.
        let mut candidate = patch.clone();
        let preview = self.add_fixtures(&fixture_type, &mut candidate);
        let mut patch_clicked = false;
        ui.horizontal(|ui| {
            let total = footprint as u32 * self.form.quantity;
            match &preview {
                Ok(_) => {
                    let text = format!("{} × {footprint} = {total} canaux", self.form.quantity);
                    ui.label(RichText::new(text).size(11.5).color(theme::MUTED));
                }
                Err(e) => {
                    ui.label(RichText::new(format!("Conflit : {e}")).size(11.5).color(theme::DANGER));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let button = theme::primary_button("Patcher").min_size(egui::vec2(110.0, 28.0));
                let help = match &preview {
                    Ok(_) => "Ajouter ces fixtures au patch",
                    Err(_) => "Conflit : changez le numéro, l'adresse ou la quantité",
                };
                patch_clicked = ui.add_enabled(preview.is_ok(), button).on_hover_text(help).clicked();
            });
        });

        if !patch_clicked {
            return None;
        }
        let ids = preview.ok()?;
        *patch = candidate;
        let plural = if ids.len() > 1 { "s" } else { "" };
        self.message = Some((true, format!("{} fixture{plural} patchée{plural}", ids.len())));
        self.form.first_id = patch.next_free_id().0;
        Some(ids)
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

    /// Tableau du patch : une ligne par fixture. Clic, Maj-clic, ⌘-clic : sélection ;
    /// clic droit : changer l'adresse. Renvoie le changement d'adresse validé, le cas échéant.
    fn patch_table(&mut self, ui: &mut egui::Ui, console: &mut Console, selected: &[FixtureId]) -> Option<Readdress> {
        let mut clicked = None;
        let mut readdress = None;
        let mut rows: Vec<Target> = Vec::new();
        let patch = &console.show.patch;
        // Le nom passe avant le type, qui se coupe le premier.
        let free = ui.available_width() - PATCH_FIXED;
        let name = (free * 0.5).max(130.0);
        let columns = (name, (free - name).max(80.0));
        patch_header(ui, columns);
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if patch.fixtures().is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("Aucune fixture patchée.").color(theme::MUTED));
            }
            for f in patch.fixtures() {
                let response = patch_row(ui, f, selected.contains(&f.id), columns);
                // Clic droit sur une fixture sélectionnée : toute la sélection, sinon elle seule.
                let ids = if selected.contains(&f.id) { selected.to_vec() } else { vec![f.id] };
                let popup = egui::Popup::context_menu(&response);
                if let Some(request) = address_menu(ui, popup, Some(f.id), ids, f) {
                    readdress = Some(request);
                }
                if response.clicked() {
                    clicked = Some(Target::whole(f.id));
                }
                rows.push(Target::whole(f.id));
            }
        });
        if let Some(target) = clicked {
            click_select(console, &rows, &mut self.last_clicked, target, ui.input(|i| i.modifiers));
        }
        readdress
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
                ui.label(RichText::new("Cliquez des fixtures dans la fenêtre Fixtures, ou tapez une commande :").color(theme::MUTED));
                ui.monospace("Fixture 1 Thru 4 At 50");
            });
            return;
        };
        let Some(fixture) = console.show.patch.get(reference.fixture).cloned() else { return };
        blind_banner(ui, console);

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
                if ui.button("Relâcher").on_hover_text("Rend la sélection à ses valeurs par défaut (R)").clicked() {
                    console.release_selection();
                }
                let help = "Marque la sélection « relâchée » : Store l'enregistre, et la séquence cesse de la tenir à cette cue (Maj-R)";
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

/// En blind, un bandeau bleu sur toute la largeur : ce qu'on règle ne sort pas.
pub(crate) fn blind_banner(ui: &mut egui::Ui, console: &Console) {
    if !console.blind() {
        return;
    }
    egui::Frame::new()
        .fill(theme::INFO.gamma_multiply(0.22))
        .stroke(egui::Stroke::new(1.0, theme::INFO))
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::symmetric(10, 5))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("BLIND").family(theme::medium()).extra_letter_spacing(0.6).color(Color32::WHITE));
                let text = "Programmer blind : rien de ce que vous réglez ne sort, Store l'enregistre. La scène garde le programmer live.";
                ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::TEXT)).wrap());
            });
        });
    ui.add_space(6.0);
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
pub(crate) fn selection_chips(ui: &mut egui::Ui, selection: &[Target]) {
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
        .filter_map(|(i, p)| shown_value(console, snapshot, fixture, i, p).map(|v| (p.name.clone(), v)))
        .collect();
    console.target_color(target, &values)
}

/// Valeur à montrer pour un paramètre : en blind, celle du programmer blind, qui ne
/// sort pas ; sinon ce qui sort vraiment.
pub(crate) fn shown_value(
    console: &Console,
    snapshot: &Snapshot,
    fixture: &PatchedFixture,
    index: usize,
    parameter: &Parameter,
) -> Option<u16> {
    let key = ParamKey { fixture: fixture.id, parameter: index };
    match console.programmer().get(&key) {
        Some(value) if console.blind() => Some(*value),
        _ => live_value(snapshot, fixture, parameter),
    }
}

/// Valeur réellement envoyée pour un paramètre, relue dans la sortie DMX.
pub(crate) fn live_value(snapshot: &Snapshot, fixture: &PatchedFixture, parameter: &Parameter) -> Option<u16> {
    let universe = snapshot.output.get(fixture.universe)?;
    let byte = |offset: u16| universe.get(fixture.address + offset).ok();
    match parameter.offsets.as_slice() {
        [coarse] => byte(*coarse).map(|v| v as u16 * 257),
        [coarse, fine, ..] => Some(((byte(*coarse)? as u16) << 8) | byte(*fine)? as u16),
        [] => None,
    }
}

/// Sélection au clic dans une liste de lignes : Maj-clic prend la plage depuis le
/// dernier clic (dans le sens du geste), ⌘-clic ajoute ou retire, sinon remplace.
pub(crate) fn click_select(
    console: &mut Console,
    rows: &[Target],
    last_clicked: &mut Option<Target>,
    target: Target,
    modifiers: egui::Modifiers,
) {
    let position = |t: Target| rows.iter().position(|r| *r == t);
    if modifiers.shift
        && let Some(a) = last_clicked.and_then(position)
        && let Some(b) = position(target)
    {
        // Dans le sens du clic : Maj-clic vers le haut sélectionne à l'envers.
        let range: Vec<Target> = if a <= b { rows[a..=b].to_vec() } else { rows[b..=a].iter().rev().copied().collect() };
        console.select(range, SelectMode::Add);
    } else if modifiers.command {
        console.select([target], SelectMode::Toggle);
    } else {
        console.select([target], SelectMode::Replace);
    }
    *last_clicked = Some(target);
}

/// D'où vient la valeur d'un paramètre : effet, programmer (ou relâché), executor, défaut.
pub(crate) fn source_of(console: &Console, snapshot: &Snapshot, key: ParamKey, animated: bool) -> Source {
    if animated {
        Source::Effect
    } else if console.programmer().contains_key(&key) || console.released().contains(&key) {
        Source::Programmer
    } else if snapshot.stage.contains_key(&key) {
        Source::Executor
    } else {
        Source::Default
    }
}

/// Valeur lisible : degrés pour un angle, sinon % ou DMX selon l'affichage choisi.
pub(crate) fn value_text(parameter: &Parameter, value: u16, display: DisplayMode) -> String {
    if let Some(cap) = parameter.capability_at(value)
        && let Some((start, end)) = cap.angle
    {
        let t = (value - cap.range.start()) as f32 / (cap.range.end() - cap.range.start()).max(1) as f32;
        return format!("{:.0}°", start + (end - start) * t);
    }
    match display {
        DisplayMode::Percent => format!("{:.1} %", value as f32 / u16::MAX as f32 * 100.0),
        DisplayMode::Dmx if parameter.resolution_bits() > 8 => format!("{}/{}", value >> 8, value & 0xFF),
        DisplayMode::Dmx => (value >> 8).to_string(),
    }
}

/// Changement d'adresse validé dans un menu : (menu, fixtures, univers, adresse).
type Readdress = (egui::Id, Vec<FixtureId>, usize, u16);

/// Saisie en cours dans un menu « Adresse ».
#[derive(Clone, Default)]
struct AddressField {
    text: Option<String>,
    error: Option<String>,
}

/// Menu de changement d'adresse (clic droit sur une ligne, ou touche « Changer l'adresse… ») :
/// nouvelle adresse de départ, les fixtures à la suite dans l'ordre donné.
/// `fixture` est la première : son adresse actuelle est proposée. `salt` : la ligne du
/// clic droit, ou `None` pour la touche.
fn address_menu(
    ui: &mut egui::Ui,
    popup: egui::Popup<'_>,
    salt: Option<FixtureId>,
    ids: Vec<FixtureId>,
    fixture: &PatchedFixture,
) -> Option<Readdress> {
    let id = egui::Id::unique(("patch-address", salt));
    let mut result = None;
    // Le menu reste ouvert pendant la saisie, et en cas de conflit pour corriger.
    let open = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        theme::menu_fields(ui);
        let mut field = ui.data_mut(|d| d.get_temp::<AddressField>(id)).unwrap_or_default();
        let current = format!("{}.{:03}", fixture.universe + 1, fixture.address);
        let first_time = field.text.is_none();
        let text = field.text.get_or_insert(current);
        let title = match ids.as_slice() {
            [one] => format!("Fixture {one}"),
            _ => format!("{} fixtures sélectionnées, à la suite", ids.len()),
        };
        ui.label(RichText::new(title).family(theme::medium()));
        ui.horizontal(|ui| {
            let label = ui.label("Adresse");
            let edit = ui.add(egui::TextEdit::singleline(text).desired_width(70.0)).labelled_by(label.id);
            if first_time {
                edit.request_focus();
            }
            if edit.changed() {
                field.error = None;
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Changer l'adresse").clicked() || enter {
                match parse_address(text) {
                    Ok((universe, address)) => result = Some((id, ids.clone(), universe, address)),
                    Err(e) => field.error = Some(e),
                }
            }
        });
        ui.label(RichText::new("101 ou 1.101 (univers.adresse)").size(11.0).color(theme::MUTED));
        if let Some(error) = &field.error {
            ui.colored_label(theme::DANGER, error);
        }
        ui.data_mut(|d| d.insert_temp(id, field));
    });
    if open.is_none() {
        ui.data_mut(|d| d.remove::<AddressField>(id));
    }
    result
}

/// « 101 » ou « 1.101 » : univers en base 0 et adresse de départ.
fn parse_address(text: &str) -> Result<(usize, u16), String> {
    let invalid = || format!("Adresse invalide : « {} » (exemple : 101 ou 1.101)", text.trim());
    let (universe, address) = match text.trim().split_once('.') {
        Some((universe, address)) => (universe.trim().parse::<usize>().map_err(|_| invalid())?, address),
        None => (1, text),
    };
    let address = address.trim().parse::<u16>().ok().filter(|a| (1..=512).contains(a)).ok_or_else(invalid)?;
    if universe != 1 {
        return Err("Un seul univers pour l'instant : 1.xxx".into());
    }
    Ok((0, address))
}

/// Colonnes du tableau du patch ; le nom et le type se partagent ce qui reste.
const PATCH_NUMBER: f32 = 52.0;
const PATCH_MODE: f32 = 104.0;
const PATCH_UNIVERSE: f32 = 44.0;
const PATCH_ADDRESS: f32 = 84.0;
const PATCH_CHANNELS: f32 = 40.0;
const PATCH_FIXED: f32 = PATCH_NUMBER + PATCH_MODE + PATCH_UNIVERSE + PATCH_ADDRESS + PATCH_CHANNELS + 16.0;

/// `(name, type)` : largeurs du nom et du type.
fn patch_header(ui: &mut egui::Ui, (name, type_width): (f32, f32)) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let painter = ui.painter();
    let columns = [
        ("N°", PATCH_NUMBER),
        ("Nom", name),
        ("Type", type_width),
        ("Mode", PATCH_MODE),
        ("Univ.", PATCH_UNIVERSE),
        ("Adresse", PATCH_ADDRESS),
        ("Ch", PATCH_CHANNELS),
    ];
    let mut x = rect.left() + 6.0;
    for (title, width) in columns {
        painter.text(pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, title, egui::FontId::proportional(11.0), theme::MUTED);
        x += width;
    }
    painter.hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0, theme::LINE));
}

/// Une ligne du tableau du patch, dessinée d'un bloc et cliquable en entier.
fn patch_row(ui: &mut egui::Ui, f: &PatchedFixture, selected: bool, (name, type_width): (f32, f32)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
    let last = f.address + f.mode().footprint.max(1) - 1;
    let label = format!("{} · {} · {}.{:03}–{last:03}", f.id, f.name, f.universe + 1, f.address);
    response.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Row, true, selected, &label));
    let painter = ui.painter_at(rect);
    let fill = if selected {
        theme::SELECTION.gamma_multiply(0.10)
    } else if response.hovered() {
        theme::SURFACE_1
    } else {
        Color32::TRANSPARENT
    };
    painter.rect_filled(rect.shrink2(vec2(0.0, 1.0)), 4.0, fill);
    painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, theme::SURFACE_2));
    let y = rect.center().y;
    let mono = egui::FontId::monospace(12.0);
    let body = egui::FontId::proportional(13.0);
    let small = egui::FontId::proportional(12.0);
    let (stroke, color) = if selected { (theme::SELECTION, theme::SELECTION) } else { (theme::LINE, theme::TEXT) };

    let mut x = rect.left() + 6.0;
    let number = Rect::from_min_size(pos2(x, y - 9.0), vec2(PATCH_NUMBER - 14.0, 18.0));
    painter.rect(number, 5.0, theme::SURFACE_2, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    painter.text(number.center(), egui::Align2::CENTER_CENTER, f.id.to_string(), mono.clone(), color);
    x += PATCH_NUMBER;
    let mut cell = |text: String, width: f32, font: egui::FontId, color: Color32| {
        let clip = Rect::from_min_size(pos2(x, rect.top()), vec2(width - 8.0, rect.height()));
        painter.with_clip_rect(clip).text(pos2(x, y), egui::Align2::LEFT_CENTER, text, font, color);
        x += width;
    };
    cell(f.name.clone(), name, body, if selected { theme::SELECTION } else { theme::TEXT });
    cell(f.fixture_type.display_name(), type_width, small.clone(), theme::MUTED);
    cell(f.mode().name.clone(), PATCH_MODE, small, theme::MUTED);
    cell((f.universe + 1).to_string(), PATCH_UNIVERSE, mono.clone(), theme::TEXT);
    cell(format!("{:03}–{last:03}", f.address), PATCH_ADDRESS, mono.clone(), theme::TEXT);
    cell(f.mode().footprint.to_string(), PATCH_CHANNELS, mono, theme::MUTED);
    response.on_hover_text("Clic : sélectionner · clic droit : changer l'adresse")
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
        let source = source_of(console, self.snapshot, key, effect.is_some());
        // Ce qui sort vraiment (effets compris) ; à défaut, la valeur du programmer.
        let value = shown_value(console, self.snapshot, self.fixture, self.index, parameter)
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

    fn value_text(&self, value: u16) -> String {
        value_text(self.parameter, value, self.display)
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

/// Nom d'un type dans la liste, avec la mention des fiches de l'utilisateur.
fn type_label(library: &Library, fixture: &folie_fixture::FixtureType) -> String {
    if library.is_user(&fixture.key) {
        format!("{} (ma fiche)", fixture.display_name())
    } else {
        fixture.display_name()
    }
}

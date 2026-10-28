//! Éditeur de fiches : modifier une fiche (canaux, plages, modes, cellules) ou en créer
//! une. Il enregistre toujours dans la bibliothèque de l'utilisateur ; une fiche livrée
//! modifiée y est copiée et remplace l'originale.

use std::path::Path;

use eframe::egui::{self, Color32, RichText};
use folie_fixture::edit::{CAPABILITY_TYPES, ChannelDoc, EMITTER_COLORS, FixtureDoc, SHUTTER_EFFECTS, Slot};
use folie_fixture::library::{self, slug};
use folie_fixture::{Library, ofl};

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Channels,
    Modes,
    Cells,
}

/// Une fiche ouverte dans l'éditeur.
pub struct Session {
    pub doc: FixtureDoc,
    pub manufacturer: String,
    /// Partie « modèle » de la clé : `rush-mh-6-wash`.
    pub model: String,
    /// Clé d'où vient la fiche ; `None` pour une fiche nouvelle.
    origin: Option<String>,
    /// Texte tel qu'à l'ouverture ou au dernier enregistrement.
    saved_json: String,
    tab: Tab,
    channel: usize,
    template: usize,
    mode: usize,
    /// Nom en cours de saisie du canal sélectionné (appliqué à la validation).
    name_edit: Option<(usize, String)>,
    replace: bool,
    message: Option<(bool, String)>,
}

impl Session {
    fn new(doc: FixtureDoc, manufacturer: String, model: String, origin: Option<String>) -> Self {
        let saved_json = if origin.is_some() { doc.to_json() } else { String::new() };
        Self {
            doc,
            manufacturer,
            model,
            origin,
            saved_json,
            tab: Tab::Channels,
            channel: 0,
            template: 0,
            mode: 0,
            name_edit: None,
            replace: false,
            message: None,
        }
    }

    pub fn key(&self) -> String {
        format!("{}/{}", slug(&self.manufacturer), slug(&self.model))
    }

    pub fn dirty(&self) -> bool {
        self.doc.to_json() != self.saved_json
    }
}

#[derive(Default)]
pub struct FixtureEditor {
    pub session: Option<Session>,
    /// Fiche choisie dans la liste d'ouverture.
    pick: usize,
}

impl FixtureEditor {
    /// Ouvre une fiche de la bibliothèque.
    pub fn open(&mut self, library: &Library, key: &str) -> Result<(), String> {
        let fixture = library.get(key).ok_or_else(|| format!("fiche {key} introuvable"))?;
        let doc = FixtureDoc::from_json(&fixture.ofl_source)?;
        let model = key.split_once('/').map(|(_, m)| m.to_string()).unwrap_or_default();
        self.session = Some(Session::new(doc, fixture.manufacturer.clone(), model, Some(key.to_string())));
        Ok(())
    }

    pub fn new_fixture(&mut self) {
        let doc = FixtureDoc::new("Nouvelle fiche");
        self.session = Some(Session::new(doc, String::new(), "nouvelle-fiche".into(), None));
    }

    /// Affiche l'éditeur. Rend la clé de la fiche enregistrée, le cas échéant.
    pub fn show(&mut self, ui: &mut egui::Ui, library: &Library, user_dir: Option<&Path>) -> Option<String> {
        let Some(session) = self.session.as_mut() else {
            self.chooser(ui, library);
            return None;
        };
        let mut saved = None;
        let mut close = false;
        header(ui, session, library);
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            for (tab, label) in [(Tab::Channels, "Canaux"), (Tab::Modes, "Modes"), (Tab::Cells, "Cellules")] {
                ui.selectable_value(&mut session.tab, tab, label);
            }
        });
        ui.separator();

        // Vérification en direct : ce que Folie comprend de la fiche telle qu'elle est.
        let json = session.doc.to_json();
        let check = ofl::parse(&json, &session.key(), &session.manufacturer);

        // Le contenu de l'onglet, puis le bas de la fenêtre (état et boutons) toujours visible.
        let body = egui::vec2(ui.available_width(), (ui.available_height() - 86.0).max(220.0));
        ui.allocate_ui(body, |ui| {
            ui.set_min_size(body);
            match session.tab {
                Tab::Channels => channels_tab(ui, session),
                Tab::Modes => modes_tab(ui, session),
                Tab::Cells => cells_tab(ui, session),
            }
        });
        ui.separator();
        ui.add_space(4.0);
        match &check {
            Ok(fixture) => {
                let modes: Vec<String> =
                    fixture.modes.iter().map(|m| format!("{} : {} canaux", m.name, m.footprint)).collect();
                let text = if fixture.warnings.is_empty() {
                    format!("Fiche valide · {}", modes.join(" · "))
                } else {
                    format!("{} · ignoré : {}", modes.join(" · "), fixture.warnings.join(" ; "))
                };
                ui.label(RichText::new(text).size(11.5).color(if fixture.warnings.is_empty() { theme::GO } else { theme::MUTED }));
            }
            Err(e) => {
                ui.colored_label(theme::DANGER, format!("Fiche invalide : {e}"));
            }
        }
        if let Some((ok, text)) = &session.message {
            ui.colored_label(if *ok { theme::GO } else { theme::DANGER }, text);
        }
        let key = session.key();
        let other = library.get(&key).is_some() && session.origin.as_deref() != Some(key.as_str());
        ui.horizontal(|ui| {
            let can_save = check.is_ok() && user_dir.is_some() && (!other || session.replace);
            let save = ui
                .add_enabled(can_save, egui::Button::new(RichText::new("Enregistrer").strong()))
                .on_hover_text("Range la fiche dans votre bibliothèque");
            if save.clicked()
                && let Some(dir) = user_dir
            {
                // Même clé que la fiche ouverte : on la remplace, c'est le but.
                match library::install(dir, &session.manufacturer, &session.model, &json, true) {
                    Ok(key) => {
                        session.saved_json = json.clone();
                        session.origin = Some(key.clone());
                        session.replace = false;
                        session.message = Some((true, format!("Enregistrée dans votre bibliothèque sous {key}")));
                        saved = Some(key);
                    }
                    Err(e) => session.message = Some((false, e)),
                }
            }
            if other {
                let which = if library.is_user(&key) { "votre fiche" } else { "la fiche livrée" };
                ui.checkbox(&mut session.replace, format!("Remplacer {which} {key}"));
            }
            let close_label = if session.dirty() { "Fermer sans enregistrer" } else { "Fermer" };
            if ui.button(close_label).clicked() {
                close = true;
            }
        });
        ui.add_space(2.0);
        if close {
            self.session = None;
        }
        saved
    }

    fn chooser(&mut self, ui: &mut egui::Ui, library: &Library) {
        ui.add_space(8.0);
        ui.heading("Éditeur de fiches");
        ui.label(RichText::new("Modifier une fiche de la bibliothèque, ou en créer une.").color(theme::MUTED));
        ui.add_space(8.0);
        if !library.fixtures.is_empty() {
            self.pick = self.pick.min(library.fixtures.len() - 1);
            ui.horizontal(|ui| {
                let label = ui.label("Fiche");
                egui::ComboBox::from_id_salt("editor_pick")
                    .width(320.0)
                    .selected_text(library.fixtures[self.pick].display_name())
                    .show_ui(ui, |ui| {
                        for (i, f) in library.fixtures.iter().enumerate() {
                            ui.selectable_value(&mut self.pick, i, f.display_name());
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if ui.button("Modifier").clicked() {
                    let key = library.fixtures[self.pick].key.clone();
                    let _ = self.open(library, &key);
                }
            });
        }
        ui.add_space(6.0);
        if ui.button("Nouvelle fiche").clicked() {
            self.new_fixture();
        }
    }
}

fn header(ui: &mut egui::Ui, session: &mut Session, library: &Library) {
    egui::Grid::new("editor_header").num_columns(4).spacing([10.0, 6.0]).show(ui, |ui| {
        let label = ui.label("Constructeur");
        ui.add_sized([200.0, 22.0], egui::TextEdit::singleline(&mut session.manufacturer).hint_text("Martin"))
            .labelled_by(label.id);
        let label = ui.label("Nom");
        ui.add_sized([260.0, 22.0], egui::TextEdit::singleline(&mut session.doc.name)).labelled_by(label.id);
        ui.end_row();
        let label = ui.label("Modèle (fichier)");
        ui.add_sized([200.0, 22.0], egui::TextEdit::singleline(&mut session.model))
            .labelled_by(label.id)
            .on_hover_text("Nom du fichier dans la bibliothèque : le garder pour corriger la fiche, le changer pour en faire une autre");
        let label = ui.label("Nom court");
        ui.add_sized([260.0, 22.0], egui::TextEdit::singleline(&mut session.doc.short_name)).labelled_by(label.id);
        ui.end_row();
        let label = ui.label("Catégories");
        let mut categories = session.doc.categories.join(", ");
        if ui
            .add_sized([200.0, 22.0], egui::TextEdit::singleline(&mut categories).hint_text("Moving Head, Color Changer"))
            .labelled_by(label.id)
            .changed()
        {
            session.doc.categories =
                categories.split(',').map(str::trim).filter(|c| !c.is_empty()).map(String::from).collect();
        }
        let key = session.key();
        let origin = match &session.origin {
            Some(o) if *o == key && library.is_user(o) => "votre fiche",
            Some(o) if *o == key => "remplacera la fiche livrée",
            Some(_) => "nouvelle fiche, l'originale reste",
            None => "nouvelle fiche",
        };
        ui.label(RichText::new(format!("{key} · {origin}")).monospace().size(11.5).color(theme::MUTED));
        ui.end_row();
    });
}

/// Liste à gauche, avec ses boutons. Rend l'action demandée.
enum ListAction {
    Add,
    Duplicate,
    Remove,
}

fn side_list(ui: &mut egui::Ui, id: &str, names: &[String], selected: &mut usize, add: &str) -> Option<ListAction> {
    let mut action = None;
    ui.vertical(|ui| {
        ui.set_width(200.0);
        egui::ScrollArea::vertical().id_salt(id).max_height(ui.available_height() - 70.0).show(ui, |ui| {
            for (i, name) in names.iter().enumerate() {
                ui.selectable_value(selected, i, name);
            }
            if names.is_empty() {
                ui.label(RichText::new("(aucun)").color(theme::MUTED));
            }
        });
        ui.add_space(4.0);
        if ui.button(add).clicked() {
            action = Some(ListAction::Add);
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!names.is_empty(), egui::Button::new("Dupliquer")).clicked() {
                action = Some(ListAction::Duplicate);
            }
            let remove = egui::Button::new(RichText::new("Supprimer").color(Color32::from_rgb(255, 138, 132)));
            if ui.add_enabled(!names.is_empty(), remove).clicked() {
                action = Some(ListAction::Remove);
            }
        });
    });
    action
}

fn channels_tab(ui: &mut egui::Ui, session: &mut Session) {
    let names: Vec<String> = session.doc.channels.iter().map(|c| c.name.clone()).collect();
    session.channel = session.channel.min(names.len().saturating_sub(1));
    ui.horizontal_top(|ui| {
        match side_list(ui, "editor_channels", &names, &mut session.channel, "Nouveau canal") {
            Some(ListAction::Add) => {
                let name = session.doc.free_name("Canal");
                session.doc.channels.push(ChannelDoc::new(&name));
                session.channel = session.doc.channels.len() - 1;
            }
            Some(ListAction::Duplicate) => {
                let mut copy = session.doc.channels[session.channel].clone();
                copy.name = session.doc.free_name(&format!("{} copie", copy.name));
                let bytes = copy.fine.len() + 1;
                copy.fine.clear();
                copy.set_bytes(bytes);
                session.doc.channels.insert(session.channel + 1, copy);
                session.channel += 1;
            }
            Some(ListAction::Remove) => {
                session.doc.remove_channel(session.channel);
                session.channel = session.channel.saturating_sub(1);
            }
            None => {}
        }
        ui.separator();
        ui.vertical(|ui| {
            if session.doc.channels.is_empty() {
                ui.label("Ajoutez un canal.");
                return;
            }
            // Nom : appliqué à la validation, pour suivre dans les modes.
            let index = session.channel;
            let current = session.doc.channels[index].name.clone();
            let mut buffer = match &session.name_edit {
                Some((i, text)) if *i == index => text.clone(),
                _ => current.clone(),
            };
            ui.horizontal(|ui| {
                let label = ui.label("Nom du canal");
                let edit = ui.add(egui::TextEdit::singleline(&mut buffer).desired_width(220.0)).labelled_by(label.id);
                if edit.lost_focus() && buffer != current {
                    match session.doc.rename_channel(index, &buffer) {
                        Ok(()) => session.message = None,
                        Err(e) => session.message = Some((false, e)),
                    }
                    session.name_edit = None;
                } else {
                    session.name_edit = Some((index, buffer.clone()));
                }
                ui.label(RichText::new("utilisé dans les modes sous ce nom").size(11.0).color(theme::MUTED));
            });
            let channel = &mut session.doc.channels[index];
            channel_editor(ui, channel, "channel");
        });
    });
}

fn bits_label(bytes: usize) -> &'static str {
    match bytes {
        1 => "8 bits",
        2 => "16 bits",
        _ => "24 bits",
    }
}

/// Résolution, valeur par défaut et plages d'un canal (ou d'un canal modèle).
fn channel_editor(ui: &mut egui::Ui, channel: &mut ChannelDoc, salt: &str) {
    ui.horizontal(|ui| {
        let label = ui.label("Résolution");
        let bytes = channel.fine.len() + 1;
        let mut chosen = bytes;
        egui::ComboBox::from_id_salt((salt, "bits"))
            .selected_text(bits_label(bytes))
            .show_ui(ui, |ui| {
                for b in 1..=3 {
                    ui.selectable_value(&mut chosen, b, bits_label(b));
                }
            })
            .response
            .labelled_by(label.id)
            .on_hover_text("16 bits : un canal fin suit le canal (réglage précis du pan, du dimmer…)");
        if chosen != bytes {
            set_resolution(channel, chosen);
        }
        ui.add_space(10.0);
        let label = ui.label("Par défaut");
        ui.add(egui::TextEdit::singleline(&mut channel.default_value).desired_width(70.0).hint_text("0"))
            .labelled_by(label.id)
            .on_hover_text("Valeur DMX, ou pourcentage (« 100% »)");
    });
    ui.add_space(6.0);
    theme::section(ui, "Plages");
    let max = channel.max_value();
    let mut remove = None;
    egui::ScrollArea::vertical().id_salt((salt, "caps")).max_height(ui.available_height() - 40.0).show(ui, |ui| {
        egui::Grid::new((salt, "caps_grid")).num_columns(6).spacing([8.0, 4.0]).striped(true).show(ui, |ui| {
            for title in ["De", "À", "Type", "Détail", "Commentaire", ""] {
                ui.label(RichText::new(title).size(11.0).color(theme::MUTED));
            }
            ui.end_row();
            for (i, cap) in channel.capabilities.iter_mut().enumerate() {
                ui.add(egui::DragValue::new(&mut cap.start).range(0..=max)).on_hover_text("Début de la plage (valeur DMX)");
                ui.add(egui::DragValue::new(&mut cap.end).range(0..=max)).on_hover_text("Fin de la plage (valeur DMX)");
                let mut kind = cap.kind.clone();
                egui::ComboBox::from_id_salt((salt, "kind", i))
                    .width(150.0)
                    .selected_text(type_label(&kind))
                    .show_ui(ui, |ui| {
                        for t in CAPABILITY_TYPES {
                            ui.selectable_value(&mut kind, t.to_string(), type_label(t));
                        }
                    })
                    .response
                    .on_hover_text("Ce que fait la plage");
                cap.set_kind(&kind);
                ui.horizontal(|ui| detail_editor(ui, cap, (salt, i)));
                ui.add_sized([190.0, 20.0], egui::TextEdit::singleline(&mut cap.comment))
                    .on_hover_text("Commentaire affiché dans Folie (nom de la couleur, du gobo…)");
                if ui.small_button("Retirer").on_hover_text("Retire cette plage").clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    });
    if let Some(i) = remove {
        channel.capabilities.remove(i);
    }
    ui.horizontal(|ui| {
        if ui.button("Ajouter une plage").on_hover_text("Coupe la dernière plage en deux").clicked() {
            channel.split_last();
        }
        if let Some(problem) = range_problem(channel) {
            ui.colored_label(theme::DANGER, problem);
        }
    });
}

/// Change le nombre d'octets en gardant les plages au même endroit de la course.
fn set_resolution(channel: &mut ChannelDoc, bytes: usize) {
    let from_bits = channel.resolution.unwrap_or(8);
    let to_bits = 8 * bytes as u32;
    channel.set_bytes(bytes);
    if from_bits == to_bits {
        return;
    }
    let shift = |v: u32, end: bool| -> u32 {
        if to_bits > from_bits {
            let up = to_bits - from_bits;
            if end { (v << up) | ((1 << up) - 1) } else { v << up }
        } else {
            v >> (from_bits - to_bits)
        }
    };
    for cap in &mut channel.capabilities {
        cap.start = shift(cap.start, false);
        cap.end = shift(cap.end, true);
    }
    if let Ok(value) = channel.default_value.trim().parse::<u32>() {
        channel.default_value = shift(value, false).to_string();
    }
    channel.resolution = if to_bits == 8 { None } else { Some(to_bits) };
}

/// Trou ou chevauchement entre plages, pour le signaler.
fn range_problem(channel: &ChannelDoc) -> Option<String> {
    let mut caps: Vec<_> = channel.capabilities.iter().collect();
    caps.sort_by_key(|c| c.start);
    let mut next = 0u32;
    for cap in caps {
        if cap.end < cap.start {
            return Some(format!("plage {}–{} à l'envers", cap.start, cap.end));
        }
        if cap.start > next {
            return Some(format!("trou de {next} à {}", cap.start - 1));
        }
        if cap.start < next {
            return Some(format!("chevauchement à {}", cap.start));
        }
        next = cap.end + 1;
    }
    (next <= channel.max_value()).then(|| format!("rien de {next} à {}", channel.max_value()))
}

fn type_label(kind: &str) -> &str {
    match kind {
        "NoFunction" => "Aucune fonction",
        "Intensity" => "Intensité",
        "ColorIntensity" => "Couleur (émetteur)",
        "ColorPreset" => "Couleur fixe",
        "ColorTemperature" => "Température de couleur",
        "ShutterStrobe" => "Shutter / strobe",
        "Pan" => "Pan",
        "Tilt" => "Tilt",
        "PanContinuous" => "Pan continu",
        "TiltContinuous" => "Tilt continu",
        "PanTiltSpeed" => "Vitesse pan/tilt",
        "WheelSlot" => "Position de roue",
        "WheelRotation" => "Rotation de roue",
        "WheelSlotRotation" => "Rotation de gobo",
        "Zoom" => "Zoom",
        "Focus" => "Focus",
        "Iris" => "Iris",
        "Prism" => "Prisme",
        "Frost" => "Frost",
        "Effect" => "Effet",
        "EffectSpeed" => "Vitesse d'effet",
        "Maintenance" => "Maintenance",
        "Generic" => "Générique",
        other => other,
    }
}

fn detail_editor(ui: &mut egui::Ui, cap: &mut folie_fixture::edit::CapabilityDoc, id: impl std::hash::Hash + std::fmt::Debug + Copy) {
    match cap.kind.as_str() {
        "ColorIntensity" => {
            let mut color = cap.text("color");
            egui::ComboBox::from_id_salt((id, "color"))
                .width(110.0)
                .selected_text(&color)
                .show_ui(ui, |ui| {
                    for c in EMITTER_COLORS {
                        ui.selectable_value(&mut color, c.to_string(), *c);
                    }
                })
                .response
                .on_hover_text("Couleur de l'émetteur");
            cap.set_text("color", &color);
        }
        "ColorPreset" => {
            let current = cap.extra.get("colors").and_then(|c| c.get(0)?.as_str().map(String::from)).unwrap_or_default();
            let mut rgb = parse_hex(&current).unwrap_or([255, 255, 255]);
            if ui.color_edit_button_srgb(&mut rgb).on_hover_text("Couleur affichée").changed() || current.is_empty() {
                let hex = format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
                cap.extra.insert("colors".into(), serde_json::json!([hex]));
            }
        }
        "ShutterStrobe" => {
            let mut effect = cap.text("shutterEffect");
            egui::ComboBox::from_id_salt((id, "shutter"))
                .width(110.0)
                .selected_text(&effect)
                .show_ui(ui, |ui| {
                    for e in SHUTTER_EFFECTS {
                        ui.selectable_value(&mut effect, e.to_string(), *e);
                    }
                })
                .response
                .on_hover_text("Effet du shutter");
            cap.set_text("shutterEffect", &effect);
        }
        "Pan" | "Tilt" => {
            let degrees = |cap: &folie_fixture::edit::CapabilityDoc, key: &str| {
                cap.text(key).trim_end_matches("deg").trim().parse::<f32>().unwrap_or(0.0)
            };
            let (mut from, mut to) = (degrees(cap, "angleStart"), degrees(cap, "angleEnd"));
            let a = ui.add(egui::DragValue::new(&mut from).suffix("°")).on_hover_text("Angle au début de la plage");
            let b = ui.add(egui::DragValue::new(&mut to).suffix("°")).on_hover_text("Angle à la fin de la plage");
            if a.changed() || b.changed() {
                cap.set_text("angleStart", &format!("{from}deg"));
                cap.set_text("angleEnd", &format!("{to}deg"));
            }
        }
        "WheelSlot" => {
            let mut slot = cap.extra.get("slotNumber").and_then(|v| v.as_f64()).unwrap_or(1.0) as u32;
            if ui.add(egui::DragValue::new(&mut slot).range(1..=64).prefix("n° ")).on_hover_text("Emplacement sur la roue").changed() {
                cap.extra.insert("slotNumber".into(), serde_json::json!(slot));
            }
        }
        _ => {
            ui.label("");
        }
    }
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let s = text.strip_prefix('#')?;
    let v = u32::from_str_radix(s, 16).ok().filter(|_| s.len() == 6)?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn modes_tab(ui: &mut egui::Ui, session: &mut Session) {
    let names: Vec<String> = session.doc.modes.iter().map(|m| m.name.clone()).collect();
    session.mode = session.mode.min(names.len().saturating_sub(1));
    ui.horizontal_top(|ui| {
        match side_list(ui, "editor_modes", &names, &mut session.mode, "Nouveau mode") {
            Some(ListAction::Add) => {
                let mut n = session.doc.modes.len() + 1;
                while session.doc.modes.iter().any(|m| m.name == format!("Mode {n}")) {
                    n += 1;
                }
                session.doc.modes.push(folie_fixture::edit::ModeDoc {
                    name: format!("Mode {n}"),
                    short_name: String::new(),
                    channels: Vec::new(),
                    extra: Default::default(),
                });
                session.mode = session.doc.modes.len() - 1;
            }
            Some(ListAction::Duplicate) => {
                session.doc.duplicate_mode(session.mode);
                session.mode += 1;
            }
            Some(ListAction::Remove) => {
                session.doc.modes.remove(session.mode);
                session.mode = session.mode.saturating_sub(1);
            }
            None => {}
        }
        ui.separator();
        ui.vertical(|ui| {
            if session.doc.modes.is_empty() {
                ui.label("Ajoutez un mode.");
                return;
            }
            let available = session.doc.slot_names();
            let has_cells = !session.doc.templates.is_empty() && !session.doc.cells.is_empty();
            let mode_index = session.mode;
            let mut add_cells = None;
            {
                let mode = &mut session.doc.modes[mode_index];
                ui.horizontal(|ui| {
                    let label = ui.label("Nom du mode");
                    ui.add(egui::TextEdit::singleline(&mut mode.name).desired_width(200.0)).labelled_by(label.id);
                    let label = ui.label("Nom court");
                    ui.add(egui::TextEdit::singleline(&mut mode.short_name).desired_width(90.0)).labelled_by(label.id);
                });
                ui.add_space(6.0);
                theme::section(ui, "Canaux, dans l'ordre DMX");
                let mut action: Option<(usize, i32)> = None;
                egui::ScrollArea::vertical().id_salt("mode_slots").max_height(ui.available_height() - 70.0).show(ui, |ui| {
                    egui::Grid::new("mode_slots_grid").num_columns(4).spacing([8.0, 3.0]).striped(true).show(ui, |ui| {
                        let count = mode.channels.len();
                        for (i, slot) in mode.channels.iter().enumerate() {
                            ui.label(RichText::new(format!("{}", i + 1)).monospace().color(theme::MUTED));
                            match slot {
                                Slot::Channel(name) if available.contains(name) => ui.label(name),
                                Slot::Channel(name) => ui.colored_label(theme::DANGER, format!("{name} (inconnu)")),
                                Slot::Empty => ui.label(RichText::new("(case vide)").color(theme::MUTED)),
                                Slot::Insert(_) => ui.label(RichText::new("(canaux de matrice insérés)").color(theme::MUTED)),
                            };
                            ui.horizontal(|ui| {
                                if ui.add_enabled(i > 0, egui::Button::new("Monter").small()).clicked() {
                                    action = Some((i, -1));
                                }
                                if ui.add_enabled(i + 1 < count, egui::Button::new("Descendre").small()).clicked() {
                                    action = Some((i, 1));
                                }
                            });
                            if ui.small_button("Retirer").clicked() {
                                action = Some((i, 0));
                            }
                            ui.end_row();
                        }
                    });
                });
                match action {
                    Some((i, 0)) => {
                        mode.channels.remove(i);
                    }
                    Some((i, delta)) => mode.channels.swap(i, (i as i32 + delta) as usize),
                    None => {}
                }
                ui.horizontal(|ui| {
                    let mut chosen: Option<Slot> = None;
                    let label = ui.label("Ajouter au mode");
                    egui::ComboBox::from_id_salt("mode_add")
                        .width(220.0)
                        .selected_text("choisir un canal…")
                        .show_ui(ui, |ui| {
                            for name in &available {
                                if ui.selectable_label(false, name).clicked() {
                                    chosen = Some(Slot::Channel(name.clone()));
                                }
                            }
                            if ui.selectable_label(false, "(case vide)").clicked() {
                                chosen = Some(Slot::Empty);
                            }
                        })
                        .response
                        .labelled_by(label.id)
                        .on_hover_text("Ajoute un canal à la fin du mode");
                    if let Some(slot) = chosen {
                        mode.channels.push(slot);
                    }
                    if has_cells {
                        if ui.button("Ajouter les cellules (cellule par cellule)").clicked() {
                            add_cells = Some(true);
                        }
                        if ui.button("(canal par canal)").on_hover_text("Tous les Dimmer, puis tous les Red…").clicked() {
                            add_cells = Some(false);
                        }
                    }
                });
            }
            if let Some(by_cell) = add_cells {
                session.doc.add_cells_to_mode(mode_index, by_cell);
            }
        });
    });
}

fn cells_tab(ui: &mut egui::Ui, session: &mut Session) {
    ui.label(
        RichText::new(
            "Pour un appareil à plusieurs cellules (barre, matrice) : un canal modèle par fonction, \
             répété pour chaque cellule. « $pixelKey » dans son nom est remplacé par le nom de la cellule.",
        )
        .color(theme::MUTED),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let label = ui.label("Nombre de cellules");
        let mut count = session.doc.cells.len();
        if ui.add(egui::DragValue::new(&mut count).range(0..=256)).labelled_by(label.id).changed() {
            session.doc.set_cell_count(count);
        }
        if !session.doc.cells.is_empty() {
            ui.label(RichText::new(format!("cellules : {}", session.doc.cells.join(", "))).size(11.5).color(theme::MUTED));
        }
    });
    ui.add_space(6.0);
    let names: Vec<String> = session.doc.templates.iter().map(|c| c.name.clone()).collect();
    session.template = session.template.min(names.len().saturating_sub(1));
    ui.horizontal_top(|ui| {
        match side_list(ui, "editor_templates", &names, &mut session.template, "Nouveau canal modèle") {
            Some(ListAction::Add) => {
                let name = session.doc.free_name("Dimmer $pixelKey");
                let mut channel = ChannelDoc::new(&name);
                channel.capabilities[0].kind = "Intensity".into();
                session.doc.templates.push(channel);
                session.template = session.doc.templates.len() - 1;
            }
            Some(ListAction::Duplicate) => {
                let mut copy = session.doc.templates[session.template].clone();
                copy.name = session.doc.free_name(&copy.name.replace("$pixelKey", "copie $pixelKey"));
                session.doc.templates.insert(session.template + 1, copy);
                session.template += 1;
            }
            Some(ListAction::Remove) => {
                session.doc.templates.remove(session.template);
                session.template = session.template.saturating_sub(1);
            }
            None => {}
        }
        ui.separator();
        ui.vertical(|ui| {
            let Some(template) = session.doc.templates.get_mut(session.template) else {
                ui.label("Ajoutez un canal modèle.");
                return;
            };
            ui.horizontal(|ui| {
                let label = ui.label("Nom du canal modèle");
                ui.add(egui::TextEdit::singleline(&mut template.name).desired_width(220.0)).labelled_by(label.id);
                if !template.name.contains("$pixelKey") {
                    ui.colored_label(theme::DANGER, "le nom doit contenir $pixelKey");
                }
            });
            let bytes = template.fine.len() + 1;
            template.fine.clear();
            template.set_bytes(bytes);
            channel_editor(ui, template, "template");
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 8 bits -> 16 bits -> 8 bits : les plages restent au même endroit de la course.
    #[test]
    fn resolution_changes_keep_the_ranges() {
        let mut channel = ChannelDoc::new("Shutter");
        channel.split_last();
        channel.capabilities[0].end = 9;
        channel.capabilities[1].start = 10;
        channel.capabilities[1].end = 255;
        channel.default_value = "10".into();
        set_resolution(&mut channel, 2);
        assert_eq!(channel.fine, ["Shutter fine"]);
        assert_eq!(channel.resolution, Some(16));
        assert_eq!((channel.capabilities[0].end, channel.capabilities[1].start), (2559, 2560));
        assert_eq!(channel.capabilities[1].end, 65_535);
        assert_eq!(channel.default_value, "2560");
        assert_eq!(range_problem(&channel), None);
        set_resolution(&mut channel, 1);
        assert_eq!((channel.capabilities[0].end, channel.capabilities[1].start, channel.capabilities[1].end), (9, 10, 255));
        assert!(channel.fine.is_empty() && channel.resolution.is_none());
    }

    #[test]
    fn range_problems_are_named() {
        let mut channel = ChannelDoc::new("Canal");
        channel.capabilities[0].end = 100;
        assert_eq!(range_problem(&channel).as_deref(), Some("rien de 101 à 255"));
        channel.split_last();
        channel.capabilities[1].start = 40;
        channel.capabilities[1].end = 255;
        assert_eq!(range_problem(&channel).as_deref(), Some("chevauchement à 40"));
    }
}

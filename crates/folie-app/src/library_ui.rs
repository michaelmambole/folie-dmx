//! Import d'une fiche dans la bibliothèque de l'utilisateur : aperçu de ce que Folie
//! en comprend, constructeur et modèle à confirmer, puis rangement.

use std::path::Path;

use eframe::egui::{self, Key, RichText};
use folie_fixture::library::{self, slug};
use folie_fixture::{FixtureType, Library, ofl};

use crate::theme;

/// Fiche choisie, en attente de confirmation.
pub struct ImportDialog {
    json: String,
    preview: FixtureType,
    manufacturer: String,
    model: String,
    replace: bool,
    error: Option<String>,
}

/// Ce que la boîte a donné à cette image.
pub enum ImportOutcome {
    Open,
    Cancelled,
    /// Clé de la fiche rangée.
    Imported(String),
}

impl ImportDialog {
    pub fn from_file(path: &Path, library: &Library) -> Result<Self, String> {
        let json = std::fs::read_to_string(path).map_err(|e| format!("{} : {e}", path.display()))?;
        let folder = path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
        Self::from_json(json, folder.as_deref(), library)
    }

    /// `folder` : dossier du fichier ; dans une copie de la bibliothèque OFL, c'est le constructeur.
    pub fn from_json(json: String, folder: Option<&str>, library: &Library) -> Result<Self, String> {
        let preview = ofl::parse(&json, "import/apercu", "").map_err(|e| format!("Fiche illisible : {e}"))?;
        let manufacturer = folder
            .map(slug)
            .and_then(|key| library.fixtures.iter().find(|f| f.key.starts_with(&format!("{key}/"))))
            .map(|f| f.manufacturer.clone())
            .unwrap_or_default();
        let model = preview.name.clone();
        Ok(Self { json, preview, manufacturer, model, replace: false, error: None })
    }

    /// Constructeur connu d'avance (fiche venue de la recherche en ligne).
    pub fn with_manufacturer(mut self, name: &str) -> Self {
        if !name.is_empty() {
            self.manufacturer = name.to_string();
        }
        self
    }

    pub fn key(&self) -> String {
        format!("{}/{}", slug(&self.manufacturer), slug(&self.model))
    }


    /// Range la fiche ; l'erreur reste affichée dans la boîte.
    pub fn confirm(&mut self, user_dir: &Path) -> Option<String> {
        match library::install(user_dir, &self.manufacturer, &self.model, &self.json, self.replace) {
            Ok(key) => Some(key),
            Err(e) => {
                self.error = Some(e);
                None
            }
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, library: &Library, user_dir: &Path) -> ImportOutcome {
        let mut outcome = ImportOutcome::Open;
        let key = self.key();
        let existing = library.get(&key).map(|_| library.is_user(&key));
        let ready = !slug(&self.manufacturer).is_empty() && !slug(&self.model).is_empty();
        // Une fiche de même clé n'est remplacée que si on l'a demandé.
        let allowed = ready && (existing.is_none() || self.replace);
        let response = egui::Modal::new(egui::Id::unique("import_fixture")).show(ctx, |ui| {
            ui.set_width(440.0);
            ui.heading("Importer une fiche");
            ui.add_space(4.0);
            ui.label(RichText::new(&self.preview.name).strong());
            for mode in &self.preview.modes {
                let heads = if mode.heads.len() > 1 { format!(", {} cellules", mode.heads.len()) } else { String::new() };
                ui.label(
                    RichText::new(format!("{} : {} canaux{heads}", mode.name, mode.footprint)).color(theme::MUTED),
                );
            }
            if !self.preview.warnings.is_empty() {
                ui.label(
                    RichText::new(format!("Ignoré : {}", self.preview.warnings.join(" ; ")))
                        .size(11.5)
                        .color(theme::MUTED),
                );
            }
            ui.add_space(8.0);
            egui::Grid::new("import_form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                let label = ui.label("Constructeur");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.manufacturer).desired_width(220.0).hint_text("Robe"))
                        .labelled_by(label.id);
                    let known = library.manufacturers();
                    egui::ComboBox::from_id_salt("import_known_manufacturer")
                        .selected_text("Connus")
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            for name in known {
                                if ui.selectable_label(self.manufacturer == name, &name).clicked() {
                                    self.manufacturer = name;
                                }
                            }
                        })
                        .response
                        .on_hover_text("Constructeurs déjà dans la bibliothèque");
                });
                ui.end_row();
                let label = ui.label("Modèle");
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut self.model).desired_width(220.0))
                    .labelled_by(label.id);
                if edit.lost_focus()
                    && ui.input(|i| i.key_pressed(Key::Enter))
                    && allowed
                    && let Some(key) = self.confirm(user_dir)
                {
                    outcome = ImportOutcome::Imported(key);
                }
                ui.end_row();
            });
            ui.label(RichText::new(format!("Rangée sous {key}")).monospace().size(11.5).color(theme::MUTED));
            if let Some(user) = existing {
                let which = if user { "votre fiche" } else { "la fiche livrée avec Folie" };
                ui.checkbox(&mut self.replace, format!("Remplacer {which} de même nom"));
            }
            if let Some(error) = &self.error {
                ui.colored_label(theme::DANGER, error);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.add_enabled(allowed, egui::Button::new(RichText::new("Importer").strong())).clicked()
                    && let Some(key) = self.confirm(user_dir)
                {
                    outcome = ImportOutcome::Imported(key);
                }
                if ui.button("Annuler").clicked() {
                    outcome = ImportOutcome::Cancelled;
                }
            });
        });
        if response.should_close() && matches!(outcome, ImportOutcome::Open) {
            outcome = ImportOutcome::Cancelled;
        }
        outcome
    }
}

/// Choix du fichier à importer.
pub fn pick_file() -> Option<std::path::PathBuf> {
    rfd::FileDialog::new().add_filter("Fiche Open Fixture Library", &["json"]).pick_file()
}

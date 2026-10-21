//! Fenêtre « À propos » : logo, version, auteur, briques utilisées et leurs licences.

use eframe::egui::{self, RichText};

use super::FolieApp;
use crate::theme;

/// Briques libres sur lesquelles repose Folie-DMX : (nom, rôle, licence).
const CREDITS: [(&str, &str, &str); 9] = [
    ("egui, eframe", "interface", "MIT ou Apache-2.0"),
    ("axum, tokio", "télécommande web", "MIT"),
    ("rosc", "OSC", "MIT ou Apache-2.0"),
    ("serialport", "Enttec USB Pro", "MPL-2.0"),
    ("qrcodegen", "QR code", "MIT"),
    ("if-addrs", "adresses réseau", "MIT ou BSD-3-Clause"),
    ("Open Fixture Library", "fiches de projecteurs", "MIT"),
    ("Inter", "police", "SIL OFL 1.1"),
    ("JetBrains Mono", "police à chasse fixe", "SIL OFL 1.1"),
];

impl FolieApp {
    pub(super) fn about_dialog(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let mut close = false;
        let response = egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                crate::brand::wordmark(ui, 34.0);
                ui.add_space(4.0);
                ui.label(RichText::new(format!("Folie-DMX {}", env!("CARGO_PKG_VERSION"))).monospace().color(theme::MUTED));
                ui.add_space(8.0);
                ui.label("Console lumière inspirée des standards de l'industrie.");
                ui.label(RichText::new("Créée par Michaël Mambole").color(theme::MUTED));
            });
            ui.add_space(12.0);
            theme::section(ui, "Construite avec");
            egui::Grid::new("credits").num_columns(3).spacing([12.0, 3.0]).show(ui, |ui| {
                for (name, role, license) in CREDITS {
                    ui.label(RichText::new(name).size(12.5));
                    ui.label(RichText::new(role).size(12.0).color(theme::MUTED));
                    ui.label(RichText::new(license).size(12.0).color(theme::MUTED));
                    ui.end_row();
                }
            });
            if let Some(dir) = self.file.storage.dir() {
                ui.add_space(12.0);
                theme::section(ui, "Réglages et sauvegarde automatique");
                ui.horizontal(|ui| {
                    let path = dir.display().to_string();
                    ui.add(egui::Label::new(RichText::new(&path).monospace().size(11.5)).truncate().selectable(true));
                    if cfg!(target_os = "macos") && ui.button("Afficher").on_hover_text("Ouvrir le dossier dans le Finder").clicked() {
                        let _ = std::process::Command::new("open").arg(dir).spawn();
                    }
                });
            }
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                if ui.button("Fermer").clicked() {
                    close = true;
                }
            });
        });
        if close || response.should_close() {
            self.about_open = false;
        }
    }
}

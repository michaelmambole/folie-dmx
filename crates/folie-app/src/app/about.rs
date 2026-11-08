//! Fenêtre « À propos » : logo, version, auteur, briques utilisées et leurs licences.

use eframe::egui::{self, RichText};

use super::FolieApp;
use crate::theme;

/// Briques libres sur lesquelles repose Folie-DMX : (nom, rôle, licence).
const CREDITS: [(&str, &str, &str); 16] = [
    ("egui, eframe", "interface", "MIT ou Apache-2.0"),
    ("serialport", "Enttec USB Pro", "MPL-2.0"),
    ("cpal", "entrée et sortie audio", "Apache-2.0"),
    ("Symphonia", "lecture des fichiers son", "MPL-2.0"),
    ("Rubato", "rééchantillonnage", "MIT ou Apache-2.0"),
    ("midir", "MIDI", "MIT"),
    ("rosc", "OSC", "MIT ou Apache-2.0"),
    ("axum, tokio", "télécommande web", "MIT"),
    ("ureq", "recherche de fiches en ligne", "MIT ou Apache-2.0"),
    ("roxmltree, zip", "import GDTF", "MIT"),
    ("qrcodegen", "QR code", "MIT"),
    ("if-addrs", "adresses réseau", "MIT ou BSD-3-Clause"),
    ("Open Fixture Library", "fiches de projecteurs", "MIT"),
    ("BlenderDMX", "fiches GDTF d'essai", "GPL-3.0"),
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
            ui.set_width(440.0);
            ui.vertical_centered(|ui| {
                ui.add_space(8.0);
                crate::brand::wordmark(ui, 34.0);
                ui.add_space(4.0);
                ui.label(RichText::new(format!("Folie-DMX {}", env!("CARGO_PKG_VERSION"))).monospace().color(theme::MUTED));
                ui.add_space(8.0);
                ui.label("Console lumière inspirée des standards de l'industrie.");
                ui.label(
                    RichText::new("DMX (Enttec USB Pro, Art-Net, sACN) · fiches OFL et GDTF · MIDI, MSC, timecode · OSC · sons")
                        .size(12.0)
                        .color(theme::MUTED),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Créée par Michaël Mambole").color(theme::MUTED));
                ui.label(RichText::new("Logiciel libre, licence GPL-3.0 ou ultérieure").size(12.0).color(theme::MUTED));
            });
            ui.add_space(12.0);
            theme::section(ui, "Construite avec");
            // La liste défile sur un petit écran (portable de 13 pouces).
            let room = (ctx.content_rect().height() - 420.0).clamp(140.0, 440.0);
            egui::ScrollArea::vertical().max_height(room).show(ui, |ui| {
                egui::Grid::new("credits").num_columns(3).spacing([12.0, 3.0]).show(ui, |ui| {
                    for (name, role, license) in CREDITS {
                        ui.label(RichText::new(name).size(12.5));
                        ui.label(RichText::new(role).size(12.0).color(theme::MUTED));
                        ui.label(RichText::new(license).size(12.0).color(theme::MUTED));
                        ui.end_row();
                    }
                });
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

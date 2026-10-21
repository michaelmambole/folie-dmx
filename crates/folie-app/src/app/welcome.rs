//! Accueil de la fenêtre principale tant que rien n'est sélectionné : le logo, la
//! prochaine chose à faire, et les raccourcis à portée de main.

use eframe::egui::{self, RichText};

use super::FolieApp;
use super::file::Pending;
use crate::theme;
use crate::workspace::WindowKind;

/// Touches de la console, hors fenêtres.
const KEYS: [(&str, &str); 5] = [
    ("S", "Store"),
    ("B", "Blackout"),
    ("R", "Relâcher la sélection"),
    ("Maj R", "Release (pour enregistrer)"),
    ("Échap", "Désarmer Store"),
];

/// « Cmd 1 » pour la fenêtre Patch (la police n'a pas le symbole de la touche Commande).
fn window_shortcut(kind: WindowKind) -> String {
    let Some(key) = kind.key() else { return "menu".into() };
    let digit = format!("{key:?}").trim_start_matches("Num").to_string();
    if cfg!(target_os = "macos") { format!("Cmd {digit}") } else { format!("Ctrl {digit}") }
}

/// Touche dessinée : texte à chasse fixe dans un cadre.
fn keycap(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::SURFACE_2)
        .stroke(egui::Stroke::new(1.0, theme::LINE))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.5));
        });
}

impl FolieApp {
    pub(super) fn welcome(&mut self, ui: &mut egui::Ui) {
        let patched = !self.console.show.patch.fixtures().is_empty();
        ui.add_space((ui.available_height() * 0.12).min(80.0));
        ui.vertical_centered(|ui| {
            crate::brand::wordmark(ui, 44.0);
            ui.add_space(18.0);
            if patched {
                ui.label(RichText::new("Aucune sélection").size(18.0).family(theme::medium()));
                ui.add_space(4.0);
                ui.label(RichText::new("Cliquez des fixtures dans la fenêtre Patch, ou tapez une commande :").color(theme::MUTED));
                ui.label(RichText::new("Fixture 1 Thru 4 At 50").monospace().color(theme::PROGRAMMER));
            } else {
                ui.label(RichText::new("Commencez par patcher vos projecteurs").size(18.0).family(theme::medium()));
                ui.add_space(4.0);
                ui.label(RichText::new("Choisissez leur type, leur nombre et leur adresse dans la fenêtre Patch.").color(theme::MUTED));
            }
            ui.add_space(14.0);
            let ctx = ui.ctx().clone();
            ui.horizontal(|ui| {
                // Les trois touches côte à côte, centrées.
                let width = 3.0 * 150.0 + 2.0 * ui.spacing().item_spacing.x;
                ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
                let size = egui::vec2(150.0, 30.0);
                let patch = if patched { egui::Button::new("Ouvrir le Patch") } else { theme::primary_button("Ouvrir le Patch") };
                if ui.add_sized(size, patch).on_hover_text(window_shortcut(WindowKind::Patch)).clicked() {
                    self.windows.open(WindowKind::Patch);
                }
                if ui.add_sized(size, egui::Button::new("Ouvrir un show…")).clicked() {
                    self.request(&ctx, Pending::Open);
                }
                if ui.add_sized(size, egui::Button::new("Nouveau show")).clicked() {
                    self.request(&ctx, Pending::New);
                }
            });
        });
        ui.add_space(28.0);

        // Raccourcis : les fenêtres (clic : ouvrir) et les touches de la console.
        ui.horizontal(|ui| {
            let width = 2.0 * 240.0 + 40.0;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            ui.vertical(|ui| {
                ui.set_width(240.0);
                theme::section(ui, "Fenêtres");
                egui::Grid::new("welcome-windows").striped(false).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    for kind in WindowKind::ALL {
                        keycap(ui, &window_shortcut(kind));
                        let link = egui::Button::new(RichText::new(kind.title()).color(theme::TEXT)).frame(false);
                        if ui.add(link).on_hover_text(format!("Ouvrir ou fermer la fenêtre {}", kind.title())).clicked() {
                            self.windows.toggle(kind);
                        }
                        ui.end_row();
                    }
                });
            });
            ui.add_space(40.0);
            ui.vertical(|ui| {
                ui.set_width(240.0);
                theme::section(ui, "Touches");
                egui::Grid::new("welcome-keys").striped(false).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (key, what) in KEYS {
                        keycap(ui, key);
                        ui.label(RichText::new(what).color(theme::MUTED));
                        ui.end_row();
                    }
                });
            });
        });
    }
}

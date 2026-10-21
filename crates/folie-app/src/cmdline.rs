//! Ligne de commande façon grandMA, avec historique (flèches haut et bas).

use eframe::egui::{self, Key, RichText};

use crate::theme;
use folie_control::{CommandError, Console, Outcome};

pub struct CommandLine {
    pub input: String,
    history: Vec<String>,
    /// Position dans l'historique pendant la navigation aux flèches.
    browsing: Option<usize>,
    /// Résultat de la dernière action : (succès, message).
    pub feedback: Option<(bool, String)>,
}

impl CommandLine {
    pub fn new() -> Self {
        Self { input: String::new(), history: Vec::new(), browsing: None, feedback: None }
    }

    /// Affiche le résultat d'une action, qu'elle vienne de la ligne de commande ou d'ailleurs.
    /// Renvoie `true` si le show a changé.
    pub fn report(&mut self, result: Result<Outcome, CommandError>) -> bool {
        match result {
            Ok(outcome) => {
                self.feedback = Some((true, outcome.message));
                outcome.show_changed
            }
            Err(err) => {
                self.feedback = Some((false, err.to_string()));
                false
            }
        }
    }

    /// Renvoie `true` si une commande a modifié le show.
    pub fn show(&mut self, ui: &mut egui::Ui, console: &mut Console) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            // Invite rouge, comme le programmer : c'est ici qu'on lui parle.
            let prompt = RichText::new("Folie>").family(egui::FontFamily::Monospace).size(14.0).color(theme::PROGRAMMER);
            ui.add(egui::Label::new(prompt).selectable(false)).on_hover_text("Ligne de commande");
            let edit = egui::TextEdit::singleline(&mut self.input)
                .hint_text("Fixture 1 Thru 4 At 50")
                .font(egui::FontId::monospace(15.0))
                .margin(egui::Margin::symmetric(8, 5))
                .background_color(theme::BG)
                .desired_width((ui.available_width() * 0.55).clamp(320.0, 760.0));
            let response = ui
                .scope(|ui| {
                    let widgets = &mut ui.visuals_mut().widgets;
                    widgets.inactive.bg_stroke = egui::Stroke::new(1.0, theme::PROGRAMMER.gamma_multiply(0.5));
                    widgets.hovered.bg_stroke = egui::Stroke::new(1.0, theme::PROGRAMMER.gamma_multiply(0.8));
                    ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, theme::PROGRAMMER);
                    ui.add(edit)
                })
                .inner;
            // Le nom accessible reste « Commande » (tests, lecteurs d'écran), pas « Folie> ».
            ui.ctx().accesskit_node_builder(response.id, |node| node.set_label("Commande"));

            if response.has_focus() {
                self.browse_history(ui);
            }
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let line = self.input.trim().to_string();
                if !line.is_empty() {
                    changed = self.report(console.execute(&line));
                    if self.history.last() != Some(&line) {
                        self.history.push(line);
                    }
                    self.input.clear();
                }
                self.browsing = None;
                // On enchaîne les commandes sans reprendre la souris.
                response.request_focus();
            }

            if let Some((ok, text)) = &self.feedback {
                let color = if *ok { theme::GO } else { theme::PROGRAMMER };
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(color)).truncate());
            }
        });
        changed
    }

    fn browse_history(&mut self, ui: &egui::Ui) {
        if self.history.is_empty() {
            return;
        }
        let (up, down) = ui.input(|i| (i.key_pressed(Key::ArrowUp), i.key_pressed(Key::ArrowDown)));
        let last = self.history.len() - 1;
        if up {
            let i = self.browsing.map_or(last, |i| i.saturating_sub(1));
            self.browsing = Some(i);
            self.input = self.history[i].clone();
        } else if down {
            match self.browsing {
                Some(i) if i < last => {
                    self.browsing = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                }
                _ => {
                    self.browsing = None;
                    self.input.clear();
                }
            }
        }
    }
}

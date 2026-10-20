//! Ligne de commande façon grandMA, avec historique (flèches haut et bas).

use eframe::egui::{self, Color32, Key, RichText};
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
            let label = ui.label(RichText::new("Commande").strong());
            let edit = egui::TextEdit::singleline(&mut self.input)
                .hint_text("Fixture 1 Thru 4 At 50")
                .font(egui::TextStyle::Monospace)
                .desired_width(480.0);
            let response = ui.add(edit).labelled_by(label.id);

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
                let color = if *ok { Color32::LIGHT_GREEN } else { Color32::LIGHT_RED };
                ui.colored_label(color, text);
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

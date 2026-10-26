//! Ligne de commande façon grandMA, avec historique (flèches haut et bas)
//! et complétion par Tab.

use eframe::egui::{self, Key, RichText};
use egui::text::{CCursor, CCursorRange};

use crate::theme;
use folie_control::completion::Completion;
use folie_control::{CommandError, Console, Outcome};

pub struct CommandLine {
    pub input: String,
    history: Vec<String>,
    /// Position dans l'historique pendant la navigation aux flèches.
    browsing: Option<usize>,
    /// Résultat de la dernière action : (succès, message).
    pub feedback: Option<(bool, String)>,
    /// Complétion en cours, tant que la ligne reste celle que Tab a écrite.
    tab: Option<TabCompletion>,
}

struct TabCompletion {
    /// Ligne telle qu'elle était au premier Tab.
    base: String,
    completion: Completion,
    /// Possibilité affichée, une fois qu'on les fait défiler.
    index: Option<usize>,
    /// Ligne écrite par le dernier Tab : si elle change, on repart de zéro.
    written: String,
}

impl CommandLine {
    pub fn new() -> Self {
        Self { input: String::new(), history: Vec::new(), browsing: None, feedback: None, tab: None }
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
        if self.tab.as_ref().is_some_and(|tab| tab.written != self.input) {
            self.tab = None;
        }
        let mut indent = 0.0;
        ui.horizontal(|ui| {
            // Invite rouge, comme le programmer : c'est ici qu'on lui parle.
            let prompt = RichText::new("Folie>").family(egui::FontFamily::Monospace).size(14.0).color(theme::PROGRAMMER);
            let prompt = ui.add(egui::Label::new(prompt).selectable(false)).on_hover_text("Ligne de commande");
            indent = prompt.rect.right() - ui.min_rect().left() + ui.spacing().item_spacing.x;
            let edit = egui::TextEdit::singleline(&mut self.input)
                .hint_text("Fixture 1 Thru 4 At 50")
                // Tab complète au lieu de passer au widget suivant.
                .lock_focus(true)
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
                let (tab, backwards) = ui.input(|i| (i.key_pressed(Key::Tab), i.modifiers.shift));
                if tab {
                    self.complete(console, backwards);
                    // Curseur en fin de ligne, prêt pour la suite.
                    let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
                    let end = CCursor::new(self.input.chars().count());
                    state.cursor.set_char_range(Some(CCursorRange::one(end)));
                    state.store(ui.ctx(), response.id);
                }
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
                self.tab = None;
                // On enchaîne les commandes sans reprendre la souris.
                response.request_focus();
            }

            if let Some((ok, text)) = &self.feedback {
                let color = if *ok { theme::GO } else { theme::PROGRAMMER };
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(color)).truncate());
            }
        });
        self.show_candidates(ui, indent);
        changed
    }

    /// Premier Tab : complète jusqu'au préfixe commun. Les suivants font défiler les possibilités.
    fn complete(&mut self, console: &Console, backwards: bool) {
        let tab = match self.tab.take() {
            Some(mut tab) => {
                let count = tab.completion.candidates.len();
                let i = match (tab.index, backwards) {
                    (None, false) => 0,
                    (None, true) => count - 1,
                    (Some(i), false) => (i + 1) % count,
                    (Some(i), true) => (i + count - 1) % count,
                };
                tab.index = Some(i);
                tab.written = tab.completion.apply(&tab.base, &tab.completion.candidates[i].text);
                tab
            }
            None => {
                let completion = console.complete(&self.input);
                if completion.candidates.is_empty() {
                    return;
                }
                let written = completion.apply(&self.input, &completion.common);
                TabCompletion { base: std::mem::take(&mut self.input), completion, index: None, written }
            }
        };
        self.input = tab.written.clone();
        // Une seule possibilité : rien à faire défiler.
        if tab.completion.candidates.len() > 1 {
            self.tab = Some(tab);
        }
    }

    /// Possibilités sous la ligne, celle choisie en rouge.
    fn show_candidates(&self, ui: &mut egui::Ui, indent: f32) {
        let Some(tab) = &self.tab else { return };
        ui.horizontal_wrapped(|ui| {
            ui.add_space(indent);
            ui.spacing_mut().item_spacing.x = 14.0;
            for (i, candidate) in tab.completion.candidates.iter().enumerate() {
                let text = RichText::new(&candidate.label).family(egui::FontFamily::Monospace).size(12.5);
                let text = if tab.index == Some(i) { text.color(theme::PROGRAMMER).strong() } else { text.weak() };
                ui.add(egui::Label::new(text).selectable(false));
            }
        });
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

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    use folie_engine::EngineConfig;
    use folie_fixture::FeatureGroup;
    use folie_show::{Group, Preset};

    use super::*;

    struct Line {
        command_line: CommandLine,
        console: Console,
    }

    fn harness() -> Harness<'static, Line> {
        let mut console = Console::new(folie_engine::spawn(EngineConfig { universes: 1, rate_hz: 50.0 }, Vec::new()));
        for (number, name) in [(1, "Cour"), (2, "Jardin"), (3, "Contres")] {
            console.show.groups.insert(number, Group { name: name.into(), targets: Vec::new() });
        }
        for (number, name) in [(1, "Rouge"), (2, "Bleu")] {
            console.show.presets.insert((FeatureGroup::Color, number), Preset { name: name.into(), ..Default::default() });
        }
        let line = Line { command_line: CommandLine::new(), console };
        let mut harness = Harness::builder().with_size(egui::vec2(900.0, 90.0)).build_ui_state(
            |ui, line: &mut Line| {
                line.command_line.show(ui, &mut line.console);
            },
            line,
        );
        theme::apply(&harness.ctx);
        harness.run_steps(2);
        harness
    }

    fn tab(harness: &mut Harness<'_, Line>) {
        harness.key_press(Key::Tab);
        harness.run_steps(2);
    }

    #[test]
    fn tab_completes_and_keeps_focus() {
        let mut harness = harness();
        let input = harness.get_by_role_and_label(Role::TextInput, "Commande");
        input.focus();
        input.type_text("fi");
        harness.run_steps(1);
        tab(&mut harness);
        assert_eq!(harness.state().command_line.input, "Fixture ");
        assert!(harness.get_by_role_and_label(Role::TextInput, "Commande").is_focused(), "Tab ne quitte pas la ligne");

        // Plusieurs groupes : la liste s'affiche, chaque Tab de plus en choisit un.
        harness.state_mut().command_line.input.clear();
        harness.run_steps(1);
        harness.get_by_role_and_label(Role::TextInput, "Commande").type_text("Group ");
        harness.run_steps(1);
        tab(&mut harness);
        harness.get_by_label("2 Jardin");
        assert_eq!(harness.state().command_line.input, "Group ");
        tab(&mut harness);
        assert_eq!(harness.state().command_line.input, "Group 1 ");
        tab(&mut harness);
        assert_eq!(harness.state().command_line.input, "Group 2 ");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("command-completion.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.key_press_modifiers(egui::Modifiers::SHIFT, Key::Tab);
        harness.run_steps(2);
        assert_eq!(harness.state().command_line.input, "Group 1 ", "Maj-Tab revient en arrière");

        // On continue de taper : la liste disparaît, et la suite se complète à son tour.
        harness.get_by_role_and_label(Role::TextInput, "Commande").type_text("at");
        harness.run_steps(2);
        assert_eq!(harness.state().command_line.input, "Group 1 at", "le curseur était en fin de ligne");
        assert!(harness.query_by_label("2 Jardin").is_none());
        harness.key_press(Key::Enter);
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::TextInput, "Commande").type_text("Preset 4.");
        harness.run_steps(1);
        tab(&mut harness);
        harness.get_by_label("4.1 Rouge");
        harness.get_by_label("4.2 Bleu");
    }
}

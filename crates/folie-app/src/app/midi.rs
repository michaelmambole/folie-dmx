//! Fenêtre MIDI : ports, associations commande -> action avec apprentissage, MSC et
//! timecode reçu. Les réglages suivent la machine (le contrôleur branché), pas le show.

use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use folie_control::{CommandError, Outcome};
use folie_remote::midi::{Mapping, MidiServer, Target, ports};

use super::FolieApp;
use crate::theme;

/// État de la fenêtre MIDI (pas enregistré).
#[derive(Default)]
pub struct MidiView {
    /// Ports présents : (entrées, sorties), relus à la demande.
    ports: Option<(Vec<String>, Vec<String>)>,
    /// Apprentissage en cours : `None` pour une nouvelle association, sinon son rang.
    learning: Option<Option<usize>>,
}

/// Les cibles proposées, avec leur nom dans le menu.
fn target_kinds() -> Vec<(&'static str, Target)> {
    vec![
        ("Fader d'executor", Target::Fader { slot: 1 }),
        ("Touche d'executor", Target::Press { slot: 1 }),
        ("Go d'executor", Target::Go { slot: 1 }),
        ("Back d'executor", Target::Back { slot: 1 }),
        ("Off d'executor", Target::Off { slot: 1 }),
        ("Flash d'executor", Target::Flash { slot: 1 }),
        ("Go (sélectionné)", Target::MainGo),
        ("Back (sélectionné)", Target::MainBack),
        ("Tout éteindre", Target::OffAll),
        ("Page suivante", Target::PageNext),
        ("Page précédente", Target::PagePrev),
        ("Blackout", Target::Blackout),
        ("Groupe", Target::Group { number: 1 }),
        ("Ligne de commande", Target::Command { line: String::new() }),
    ]
}

fn kind_name(target: &Target) -> &'static str {
    let kinds = target_kinds();
    kinds
        .iter()
        .find(|(_, t)| std::mem::discriminant(t) == std::mem::discriminant(target))
        .map_or("?", |(name, _)| name)
}

impl FolieApp {
    /// Ouvre ou ferme les ports selon les réglages, et les enregistre.
    pub(crate) fn midi_changed(&mut self) {
        self.midi = None;
        self.midi_view.learning = None;
        if self.midi_settings.enabled {
            match MidiServer::start(&self.midi_settings) {
                Ok(server) => self.midi = Some(server),
                Err(err) => {
                    self.midi_settings.enabled = false;
                    self.error = Some(format!("MIDI : {err}"));
                }
            }
        }
        self.file.storage.set_midi(&self.midi_settings);
    }

    /// Associations changées : le serveur les reprend, elles sont enregistrées.
    fn midi_mappings_changed(&mut self) {
        if let Some(midi) = &mut self.midi {
            midi.update(&self.midi_settings);
        }
        self.file.storage.set_midi(&self.midi_settings);
    }

    #[cfg(test)]
    pub(crate) fn midi_mappings_changed_for_tests(&mut self) {
        self.midi_mappings_changed();
    }

    /// Messages arrivés depuis la dernière image, puis retour vers le contrôleur.
    pub(super) fn midi_tick(&mut self) -> Vec<Result<Outcome, CommandError>> {
        let Some(midi) = &self.midi else { return Vec::new() };
        let handled = midi.handle(&mut self.console);
        midi.publish(&self.console, &self.snapshot);
        if let Some(control) = handled.learned
            && let Some(learning) = self.midi_view.learning.take()
        {
            match learning.and_then(|i| self.midi_settings.mappings.get_mut(i)) {
                Some(mapping) => mapping.control = control,
                None => self.midi_settings.mappings.push(Mapping { control, target: Target::Fader { slot: 1 } }),
            }
            self.midi_mappings_changed();
        }
        handled.results
    }

    pub(super) fn midi_window(&mut self, ui: &mut egui::Ui) {
        let (inputs, outputs) = self.midi_view.ports.get_or_insert_with(ports).clone();
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            theme::card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let mut enabled = self.midi_settings.enabled;
                    if ui.checkbox(&mut enabled, RichText::new("MIDI actif").strong()).changed() {
                        self.midi_settings.enabled = enabled;
                        self.midi_changed();
                    }
                    if ui.button("Relire les ports").on_hover_text("Après avoir branché un contrôleur").clicked() {
                        self.midi_view.ports = None;
                        if self.midi_settings.enabled {
                            self.midi_changed();
                        }
                    }
                    if let Some(midi) = &self.midi {
                        let text = match midi.opened.len() {
                            0 => "aucun port ouvert".to_string(),
                            n => format!("{n} port(s) ouvert(s)"),
                        };
                        ui.label(RichText::new(text).size(11.5).color(theme::MUTED));
                    }
                });
                ui.add_space(4.0);
                theme::section(ui, "Entrées");
                if inputs.is_empty() {
                    ui.label(RichText::new("Aucune entrée MIDI : branchez un contrôleur, puis Relire les ports.").color(theme::MUTED));
                }
                let mut ports_changed = false;
                for name in &inputs {
                    let mut on = self.midi_settings.inputs.is_empty() || self.midi_settings.inputs.contains(name);
                    if ui.checkbox(&mut on, name).changed() {
                        // Liste vide = tous : on la remplit avant d'en retirer un.
                        if self.midi_settings.inputs.is_empty() {
                            self.midi_settings.inputs = inputs.clone();
                        }
                        self.midi_settings.inputs.retain(|n| n != name);
                        if on {
                            self.midi_settings.inputs.push(name.clone());
                        }
                        if self.midi_settings.inputs.len() == inputs.len() {
                            self.midi_settings.inputs.clear();
                        }
                        ports_changed = true;
                    }
                }
                ui.horizontal(|ui| {
                    let label = ui.label("Retour vers");
                    let current = if self.midi_settings.output.is_empty() { "aucun".to_string() } else { self.midi_settings.output.clone() };
                    let mut chosen = self.midi_settings.output.clone();
                    egui::ComboBox::from_id_salt("midi_output")
                        .width(260.0)
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut chosen, String::new(), "aucun");
                            for name in &outputs {
                                ui.selectable_value(&mut chosen, name.clone(), name);
                            }
                        })
                        .response
                        .labelled_by(label.id)
                        .on_hover_text("LED des boutons et faders motorisés du contrôleur");
                    if chosen != self.midi_settings.output {
                        self.midi_settings.output = chosen;
                        ports_changed = true;
                    }
                });
                if ports_changed {
                    if self.midi_settings.enabled {
                        self.midi_changed();
                    } else {
                        self.file.storage.set_midi(&self.midi_settings);
                    }
                }
            });
            ui.add_space(6.0);
            theme::card(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::section(ui, "Associations");
                self.mappings_table(ui);
            });
            ui.add_space(6.0);
            theme::card(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::section(ui, "Régie : MIDI Show Control et timecode");
                ui.horizontal(|ui| {
                    let mut changed = ui
                        .checkbox(&mut self.midi_settings.msc, "MIDI Show Control")
                        .on_hover_text("GO, STOP, RESUME, ALL_OFF ; la liste de cues désigne l'executor (liste 2 : executor 2)")
                        .changed();
                    let label = ui.label("Identifiant");
                    changed |= ui
                        .add(egui::DragValue::new(&mut self.midi_settings.msc_device).range(0..=127))
                        .labelled_by(label.id)
                        .on_hover_text("127 : répond à tous les messages")
                        .changed();
                    if changed {
                        self.midi_mappings_changed();
                    }
                });
                let timecode = self.midi.as_ref().and_then(MidiServer::timecode);
                let text = match timecode {
                    Some(tc) => format!("Timecode reçu : {} ({} i/s)", tc.label(), tc.fps),
                    None => "Timecode : rien reçu".to_string(),
                };
                ui.label(RichText::new(text).monospace().color(if timecode.is_some() { theme::GO } else { theme::MUTED }));
                if timecode.is_some() {
                    ui.ctx().request_repaint_after(Duration::from_millis(40));
                }
            });
        });
    }

    fn mappings_table(&mut self, ui: &mut egui::Ui) {
        let mut remove = None;
        let mut changed = false;
        let learning = self.midi_view.learning;
        egui::Grid::new("midi_mappings").num_columns(4).spacing([8.0, 4.0]).striped(true).show(ui, |ui| {
            for (i, mapping) in self.midi_settings.mappings.iter_mut().enumerate() {
                let text = if learning == Some(Some(i)) { "touchez une commande…".to_string() } else { mapping.control.label() };
                ui.label(RichText::new(text).monospace().size(12.0));
                ui.horizontal(|ui| changed |= target_editor(ui, &mut mapping.target, i));
                if ui.small_button("Apprendre").on_hover_text("Associe la prochaine commande touchée").clicked() {
                    self.midi_view.learning = Some(Some(i));
                }
                let remove_text = RichText::new("Retirer").color(Color32::from_rgb(255, 138, 132));
                if ui.small_button(remove_text).clicked() {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
        if self.midi_settings.mappings.is_empty() {
            ui.label(RichText::new("Aucune association.").color(theme::MUTED));
        }
        if let Some(i) = remove {
            self.midi_settings.mappings.remove(i);
            changed = true;
        }
        if changed {
            self.midi_mappings_changed();
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            match self.midi_view.learning {
                Some(_) => {
                    ui.spinner();
                    ui.label("Touchez un bouton ou un fader du contrôleur…");
                    if ui.button("Annuler").clicked() {
                        self.midi_view.learning = None;
                    }
                }
                None => {
                    let enabled = self.midi.is_some();
                    let add = ui
                        .add_enabled(enabled, egui::Button::new("Nouvelle association"))
                        .on_hover_text("Touchez ensuite la commande du contrôleur, puis choisissez l'action")
                        .on_disabled_hover_text("Activez le MIDI d'abord");
                    if add.clicked() {
                        self.midi_view.learning = Some(None);
                    }
                }
            }
        });
        if let Some(midi) = &self.midi {
            midi.learn(self.midi_view.learning.is_some());
        }
        if self.midi_view.learning.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
    }
}

/// Type d'action et son réglage (place, numéro, ligne). Rend `true` s'il a changé.
fn target_editor(ui: &mut egui::Ui, target: &mut Target, row: usize) -> bool {
    let before = target.clone();
    let mut chosen = None;
    egui::ComboBox::from_id_salt(("midi_target", row))
        .width(170.0)
        .selected_text(kind_name(target))
        .show_ui(ui, |ui| {
            for (name, kind) in target_kinds() {
                let same = std::mem::discriminant(&kind) == std::mem::discriminant(target);
                if ui.selectable_label(same, name).clicked() && !same {
                    chosen = Some(kind);
                }
            }
        })
        .response
        .on_hover_text("Ce que fait la commande");
    if let Some(mut kind) = chosen {
        // On garde la place d'executor en changeant de type d'action.
        if let (Some(slot), Some(new_slot)) = (target.slot(), slot_mut(&mut kind)) {
            *new_slot = slot;
        }
        *target = kind;
    }
    if let Some(slot) = slot_mut(target) {
        ui.add(egui::DragValue::new(slot).range(1..=folie_show::SLOTS_PER_PAGE).prefix("n° "))
            .on_hover_text("Place sur la page d'executors affichée");
    }
    match target {
        Target::Group { number } => {
            ui.add(egui::DragValue::new(number).range(1..=9999)).on_hover_text("Numéro du groupe");
        }
        Target::Command { line } => {
            ui.add(egui::TextEdit::singleline(line).desired_width(180.0).hint_text("Go Executor 3"))
                .on_hover_text("Ligne de commande exécutée à l'appui");
        }
        _ => {}
    }
    *target != before
}

fn slot_mut(target: &mut Target) -> Option<&mut u32> {
    match target {
        Target::Fader { slot } | Target::Press { slot } | Target::Go { slot } | Target::Back { slot } | Target::Off { slot } | Target::Flash { slot } => {
            Some(slot)
        }
        _ => None,
    }
}

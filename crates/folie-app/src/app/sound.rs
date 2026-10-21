//! Fenêtre Son : entrée audio, vu-mètres par bande, beat et tempo, et les réactions
//! des executors (enregistrées dans le show).

use std::time::Duration;

use eframe::egui::{self, Color32, RichText};
use folie_audio::input::{AudioInput, input_devices};
use folie_show::executor_label;
use folie_show::sound::{Band, Reaction, SoundReaction};

use super::FolieApp;
use crate::theme;

/// Le voyant de beat reste allumé ce temps-là.
const BEAT_FLASH: Duration = Duration::from_millis(120);

/// Barre de niveau horizontale, de 0 à 1.
fn meter(ui: &mut egui::Ui, label: &str, value: f32, color: Color32) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(62.0, 16.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(62.0);
            ui.label(RichText::new(label).size(12.0).color(theme::MUTED));
        });
        let width = ui.available_width().min(320.0);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 10.0), egui::Sense::hover());
        let painter = ui.painter();
        painter.rect_filled(rect, 3.0, theme::BG);
        let filled = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * value.clamp(0.0, 1.0), rect.height()));
        painter.rect_filled(filled, 3.0, color);
        response.widget_info(|| egui::WidgetInfo::slider(false, f64::from(value) * 100.0, label));
    });
}

/// Champ de 0 à 100 % pour une valeur de 0 à 1.
fn percent(value: &mut f32) -> egui::DragValue<'_> {
    egui::DragValue::from_get_set(move |set: Option<f64>| {
        if let Some(x) = set {
            *value = (x / 100.0).clamp(0.0, 1.0) as f32;
        }
        f64::from(*value) * 100.0
    })
    .range(0.0..=100.0)
    .suffix(" %")
    .max_decimals(0)
}

fn reaction_kinds() -> [Reaction; 3] {
    [Reaction::Step { every: 1 }, Reaction::Fader { band: Band::Bass, min: 0.0, max: 1.0 }, Reaction::Flash { every: 1 }]
}

impl FolieApp {
    /// Démarre ou arrête l'entrée selon les réglages, et les enregistre.
    pub(crate) fn sound_changed(&mut self) {
        self.audio = None;
        let s = &self.sound_settings;
        if s.enabled {
            let tx = self.console.engine().sender();
            match AudioInput::start(s.device.as_deref(), s.channel, s.gain, self.console.show.sound.clone(), tx) {
                Ok(input) => {
                    self.audio_setup = self.console.show.sound.clone();
                    self.audio = Some(input);
                }
                Err(err) => {
                    self.sound_settings.enabled = false;
                    self.error = Some(format!("Son : {err}"));
                }
            }
        }
        self.file.storage.set_sound(&self.sound_settings);
    }

    /// À chaque image : les réactions du show changées sont passées à l'entrée audio.
    pub(super) fn sound_tick(&mut self) {
        if let Some(audio) = &self.audio
            && self.audio_setup != self.console.show.sound
        {
            self.audio_setup = self.console.show.sound.clone();
            audio.set_setup(&self.audio_setup);
        }
    }

    pub(super) fn sound_window(&mut self, ui: &mut egui::Ui) {
        if self.audio_devices.is_empty() {
            self.audio_devices = input_devices();
        }
        egui::ScrollArea::vertical().show(ui, |ui| {
            self.sound_input(ui);
            ui.add_space(6.0);
            self.sound_meters(ui);
            ui.add_space(6.0);
            self.sound_reactions(ui);
        });
    }

    fn sound_input(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.sound_settings.clone();
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section(ui, "Entrée");
            egui::Grid::new("sound-input").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                let label = ui.label(RichText::new("Appareil").color(theme::MUTED));
                let current = settings.device.clone().unwrap_or_else(|| "Entrée par défaut".into());
                egui::ComboBox::from_id_salt("sound-device")
                    .selected_text(current)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.device, None, "Entrée par défaut");
                        for device in &self.audio_devices {
                            let text = format!("{} ({} canaux)", device.name, device.channels);
                            ui.selectable_value(&mut settings.device, Some(device.name.clone()), text);
                        }
                    })
                    .response
                    .labelled_by(label.id);
                ui.end_row();

                let label = ui.label(RichText::new("Canal").color(theme::MUTED));
                let channels = settings
                    .device
                    .as_ref()
                    .and_then(|name| self.audio_devices.iter().find(|d| &d.name == name))
                    .or(self.audio_devices.first())
                    .map_or(2, |d| d.channels);
                let channel_text = |c: Option<u16>| c.map_or("Tous, mélangés".to_string(), |c| format!("Canal {}", c + 1));
                egui::ComboBox::from_id_salt("sound-channel")
                    .selected_text(channel_text(settings.channel))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut settings.channel, None, channel_text(None));
                        for c in 0..channels {
                            ui.selectable_value(&mut settings.channel, Some(c), channel_text(Some(c)));
                        }
                    })
                    .response
                    .labelled_by(label.id);
                ui.end_row();

                let label = ui.label(RichText::new("Gain").color(theme::MUTED));
                ui.add(egui::DragValue::new(&mut settings.gain).range(0.1..=40.0).speed(0.05).max_decimals(1).prefix("× "))
                    .labelled_by(label.id)
                    .on_hover_text("Pour une entrée trop faible ; ensuite, chaque bande se règle seule de 0 à 100 %");
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if self.audio.is_some() {
                    let off = egui::Button::new(RichText::new("Couper le son").family(theme::medium()).color(Color32::WHITE)).fill(theme::DANGER);
                    if ui.add(off).clicked() {
                        settings.enabled = false;
                    }
                    if let Some(audio) = &self.audio {
                        ui.label(RichText::new(format!("Écoute : {}", audio.device)).size(12.0).color(theme::GO));
                    }
                } else if ui.add(theme::primary_button("Écouter")).on_hover_text("Ouvrir l'entrée audio (macOS demande l'accès au micro la première fois)").clicked() {
                    settings.enabled = true;
                }
                if ui.button("Rechercher").on_hover_text("Relire la liste des entrées").clicked() {
                    self.audio_devices = input_devices();
                }
            });
        });
        if settings != self.sound_settings {
            // Le gain se change à chaud ; le reste rouvre l'entrée.
            let only_gain = settings.enabled == self.sound_settings.enabled
                && settings.device == self.sound_settings.device
                && settings.channel == self.sound_settings.channel;
            self.sound_settings = settings;
            match &self.audio {
                Some(audio) if only_gain => {
                    audio.set_gain(self.sound_settings.gain);
                    self.file.storage.set_sound(&self.sound_settings);
                }
                _ => self.sound_changed(),
            }
        }
    }

    fn sound_meters(&mut self, ui: &mut egui::Ui) {
        let meter_state = self.audio.as_ref().map(AudioInput::meter).unwrap_or_default();
        let mut tempo = self.console.show.sound.tempo;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section(ui, "Analyse");
            let levels = meter_state.levels;
            meter(ui, "Niveau", levels.level, theme::TEXT);
            meter(ui, "Graves", levels.bass, theme::PROGRAMMER);
            meter(ui, "Médiums", levels.mid, theme::EXECUTOR);
            meter(ui, "Aigus", levels.high, theme::INFO);
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let lit = meter_state.last_beat.is_some_and(|t| t.elapsed() < BEAT_FLASH);
                let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 8.0, if lit { theme::GO } else { theme::SURFACE_3 });
                ui.label(RichText::new("Beat").color(theme::MUTED));
                ui.add_space(12.0);
                let detected = meter_state.bpm.map_or("—".to_string(), |b| format!("{b:.1} BPM"));
                ui.label(RichText::new(format!("Tempo détecté {detected}")).monospace().size(12.5));
                ui.add_space(12.0);
                ui.label(RichText::new(format!("Tempo du show {:.1}", self.snapshot.tempo)).monospace().size(12.5).color(theme::MUTED));
            });
            let help = "Les effets et chenillards calés sur le tempo suivent la musique ; le tempo enregistré dans le show ne change pas";
            ui.checkbox(&mut tempo, "Le tempo suit le beat").on_hover_text(help);
        });
        if tempo != self.console.show.sound.tempo {
            self.console.show.sound.tempo = tempo;
            if !tempo {
                // Retour au tempo du show.
                self.console.engine().send(folie_engine::Command::SetTempo(self.console.show.tempo));
            }
            self.mark_dirty();
        }
        if self.audio.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(30));
        }
    }

    fn sound_reactions(&mut self, ui: &mut egui::Ui) {
        let mut reactions = self.console.show.sound.reactions.clone();
        let mut remove = None;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::section(ui, "Réactions des executors");
            if reactions.is_empty() {
                ui.label(RichText::new("Aucune : ajoutez-en une pour l'executor sélectionné.").size(12.0).color(theme::MUTED));
            }
            for (i, r) in reactions.iter_mut().enumerate() {
                ui.push_id(i, |ui| {
                    ui.horizontal(|ui| {
                        let name = self.console.executor(r.executor).map(|(_, s)| s.name.clone()).unwrap_or_else(|| "vide".into());
                        let label = ui.label(RichText::new("Exec").color(theme::MUTED));
                        ui.add(
                            egui::DragValue::new(&mut r.executor)
                                .range(1..=folie_show::executor_id(folie_show::MAX_PAGE, folie_show::SLOTS_PER_PAGE))
                                .custom_formatter(|n, _| executor_label(n as u32))
                                .custom_parser(|text| {
                                    let (page, slot) = text.trim().split_once('.').map_or(Some((1, text.trim().parse().ok()?)), |(p, s)| {
                                        Some((p.parse().ok()?, s.parse().ok()?))
                                    })?;
                                    Some(f64::from(folie_show::executor_id(page, slot)))
                                }),
                        )
                        .labelled_by(label.id)
                        .on_hover_text(format!("Executor {} : {name}", executor_label(r.executor)));
                        egui::ComboBox::from_id_salt("kind")
                            .selected_text(r.reaction.label())
                            .width(140.0)
                            .show_ui(ui, |ui| {
                                for kind in reaction_kinds() {
                                    let same = std::mem::discriminant(&kind) == std::mem::discriminant(&r.reaction);
                                    if ui.selectable_label(same, kind.label()).clicked() && !same {
                                        r.reaction = kind;
                                    }
                                }
                            });
                        match &mut r.reaction {
                            Reaction::Step { every } | Reaction::Flash { every } => {
                                ui.label(RichText::new("tous les").color(theme::MUTED));
                                ui.add(egui::DragValue::new(every).range(1..=32));
                                ui.label(RichText::new(if *every > 1 { "beats" } else { "beat" }).color(theme::MUTED));
                            }
                            Reaction::Fader { band, min, max } => {
                                egui::ComboBox::from_id_salt("band").selected_text(band.label()).width(80.0).show_ui(ui, |ui| {
                                    for b in Band::ALL {
                                        ui.selectable_value(band, b, b.label());
                                    }
                                });
                                ui.add(percent(min)).on_hover_text("Fader quand la bande est au plus bas");
                                ui.label(RichText::new("à").color(theme::MUTED));
                                ui.add(percent(max)).on_hover_text("Fader quand la bande est au plus haut");
                            }
                        }
                        let delete = egui::Button::new(RichText::new("×").size(15.0).color(theme::MUTED)).frame(false);
                        if ui.add(delete).on_hover_text("Retirer cette réaction").clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            ui.add_space(4.0);
            let executor = self.console.selected_executor();
            let help = format!("Pas sur le beat pour l'executor sélectionné ({})", executor_label(executor));
            if ui.button("Ajouter une réaction").on_hover_text(help).clicked() {
                reactions.push(SoundReaction { executor, reaction: Reaction::Step { every: 1 } });
            }
        });
        if let Some(i) = remove {
            reactions.remove(i);
        }
        if reactions != self.console.show.sound.reactions {
            self.console.show.sound.reactions = reactions;
            self.mark_dirty();
        }
    }
}

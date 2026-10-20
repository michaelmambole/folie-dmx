//! Fenêtre Effets : tempo, modèles, réglage des effets du programmer.

use eframe::egui::{self, RichText};
use folie_control::{CommandError, Console, Outcome};
use folie_show::{EffectRate, Waveform, effects::templates};

type Action = Option<Result<Outcome, CommandError>>;

pub fn effects_window(ui: &mut egui::Ui, console: &mut Console) -> Action {
    let mut action = None;

    ui.horizontal(|ui| {
        let label = ui.label(RichText::new("Tempo").strong());
        let mut bpm = console.tempo();
        let field = egui::DragValue::new(&mut bpm).range(1.0..=999.0).speed(0.5).max_decimals(1).suffix(" BPM");
        if ui.add(field).labelled_by(label.id).changed() {
            action = Some(Ok(console.set_tempo(bpm)));
        }
        if ui.button(RichText::new("Tap").strong()).on_hover_text("Taper en rythme pour régler le tempo").clicked()
            && let Some(outcome) = console.tap_tempo()
        {
            action = Some(Ok(outcome));
        }
    });
    ui.separator();

    ui.label(RichText::new("Modèles").strong());
    ui.small("Sélectionnez des fixtures puis cliquez un modèle (ou tapez « Effect 2 »).");
    ui.horizontal_wrapped(|ui| {
        for (i, template) in templates().iter().enumerate() {
            let number = i as u32 + 1;
            if ui.button(format!("{number} {}", template.name)).clicked() {
                action = Some(console.apply_effect(number));
            }
        }
    });
    ui.separator();

    ui.label(RichText::new("En cours dans le programmer").strong());
    if console.programmer_effects().is_empty() {
        ui.weak("Aucun effet. Store Cue les enregistre dans une cue ; Clear les arrête.");
    }
    let mut changed = false;
    let mut remove = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (index, effect) in console.programmer_effects_mut().iter_mut().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&effect.name).strong());
                    ui.weak(format!("{} cible(s)", effect.targets.len()));
                    if ui.small_button("Arrêter").on_hover_text(format!("Arrêter « {} »", effect.name)).clicked() {
                        remove = Some(index);
                    }
                });
                changed |= effect_settings(ui, index, effect);
            });
        }
    });
    if let Some(index) = remove {
        console.remove_effect(index);
    } else if changed {
        console.effects_changed();
    }
    action
}

/// Réglages d'un effet. Renvoie `true` si quelque chose a changé.
fn effect_settings(ui: &mut egui::Ui, index: usize, effect: &mut folie_show::Effect) -> bool {
    let mut changed = false;
    egui::Grid::new(("effect", index)).num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        let label = ui.label("Vitesse");
        ui.horizontal(|ui| {
            let synced = matches!(effect.rate, EffectRate::Beats(_));
            let mode = if synced { "Tempo" } else { "Hz" };
            egui::ComboBox::from_id_salt(("rate", index))
                .selected_text(mode)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(synced, "Tempo (temps par cycle)").clicked() && !synced {
                        effect.rate = EffectRate::Beats(4.0);
                        changed = true;
                    }
                    if ui.selectable_label(!synced, "Hz (cycles par seconde)").clicked() && synced {
                        effect.rate = EffectRate::Hz(0.5);
                        changed = true;
                    }
                })
                .response
                .labelled_by(label.id);
            let (value, suffix) = match &mut effect.rate {
                EffectRate::Beats(beats) => (beats, " temps"),
                EffectRate::Hz(hz) => (hz, " Hz"),
            };
            changed |= ui
                .add(egui::DragValue::new(value).range(0.05..=64.0).speed(0.05).max_decimals(2).suffix(suffix))
                .on_hover_text("Durée d'un cycle (tempo) ou fréquence (Hz)")
                .changed();
        });
        ui.end_row();

        let label = ui.label("Éventail");
        changed |= ui
            .add(egui::DragValue::new(&mut effect.spread).range(0.0..=720.0).suffix("°"))
            .labelled_by(label.id)
            .changed();
        ui.end_row();
        let label = ui.label("Ailes");
        changed |= ui.add(egui::DragValue::new(&mut effect.wings).range(1..=8)).labelled_by(label.id).changed();
        ui.end_row();
        let label = ui.label("Blocs");
        changed |= ui.add(egui::DragValue::new(&mut effect.blocks).range(1..=16)).labelled_by(label.id).changed();
        ui.end_row();
    });

    for (line_index, line) in effect.lines.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let name = line.attribute.label();
            ui.label(RichText::new(&name).strong());
            egui::ComboBox::from_id_salt(("waveform", index, line_index))
                .selected_text(line.waveform.label())
                .show_ui(ui, |ui| {
                    for w in Waveform::ALL {
                        changed |= ui.selectable_value(&mut line.waveform, w, w.label()).changed();
                    }
                })
                .response
                .on_hover_text(format!("Forme d'onde : {name}"));
            changed |= ui
                .add(egui::DragValue::new(&mut line.low).range(0.0..=100.0).prefix("bas ").suffix(" %"))
                .on_hover_text(format!("Valeur basse : {name}"))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(&mut line.high).range(0.0..=100.0).prefix("haut ").suffix(" %"))
                .on_hover_text(format!("Valeur haute : {name}"))
                .changed();
            changed |= ui
                .add(egui::DragValue::new(&mut line.phase).range(-360.0..=360.0).prefix("phase ").suffix("°"))
                .on_hover_text(format!("Décalage : {name}"))
                .changed();
        });
    }
    changed
}

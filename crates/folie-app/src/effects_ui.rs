//! Fenêtre Effets : tempo, modèles, réglage des effets du programmer.
//! Le violet y est la couleur des effets, comme partout ailleurs.

use eframe::egui::{self, Color32, RichText, Sense, vec2};
use folie_control::{CommandError, Console, Outcome};
use folie_show::{Effect, EffectRate, Waveform, effects::templates};

use crate::theme;

type Action = Option<Result<Outcome, CommandError>>;

const TILE: egui::Vec2 = egui::vec2(124.0, 58.0);

/// Courbe d'une forme d'onde sur une période, dans `rect`.
fn paint_wave(painter: &egui::Painter, rect: egui::Rect, wave: Waveform, color: Color32) {
    let engine = folie_control::effects::waveform(wave);
    let points: Vec<egui::Pos2> = (0..=48)
        .map(|i| {
            let t = i as f64 / 48.0;
            let level = engine.level(t) as f32;
            egui::pos2(rect.left() + rect.width() * t as f32, rect.bottom() - rect.height() * level)
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, color)));
}

/// Carte du tempo : BPM en grand, voyant violet sur chaque temps, touche Tap.
fn tempo_card(ui: &mut egui::Ui, console: &mut Console) -> Action {
    let mut action = None;
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let label = ui.label(RichText::new("TEMPO").size(11.0).color(theme::MUTED).extra_letter_spacing(0.8));
            let mut bpm = console.tempo();
            let edited = ui.scope(|ui| {
                ui.style_mut().override_font_id = Some(egui::FontId::monospace(20.0));
                let field = egui::DragValue::new(&mut bpm).range(1.0..=999.0).speed(0.5).max_decimals(1);
                ui.add(field).labelled_by(label.id).changed()
            });
            if edited.inner {
                action = Some(Ok(console.set_tempo(bpm)));
            }
            ui.label(RichText::new("BPM").color(theme::MUTED));
            // Voyant : allumé au début de chaque temps.
            let beat = (ui.input(|i| i.time) * console.tempo() as f64 / 60.0).fract();
            let (rect, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            let lit = beat < 0.2;
            let color = if lit { theme::EFFECT } else { theme::EFFECT.gamma_multiply(0.25) };
            ui.painter().circle_filled(rect.center(), 5.0, color);
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let tap = egui::Button::new(RichText::new("Tap").family(theme::medium())).min_size(vec2(64.0, 26.0));
                if ui.add(tap).on_hover_text("Taper en rythme pour régler le tempo").clicked()
                    && let Some(outcome) = console.tap_tempo()
                {
                    action = Some(Ok(outcome));
                }
            });
        });
    });
    action
}

/// Case d'un modèle : numéro, nom, aperçu de son onde. Clic : lancer sur la sélection.
fn template_tile(ui: &mut egui::Ui, number: u32, template: &Effect) -> bool {
    let (rect, response) = ui.allocate_exact_size(TILE, Sense::click());
    let painter = ui.painter_at(rect.expand(1.0));
    let hovered = response.hovered();
    painter.rect_filled(rect, 8.0, if hovered { theme::SURFACE_3 } else { theme::SURFACE_2 });
    let stroke = if hovered { theme::EFFECT } else { theme::LINE };
    painter.rect_stroke(rect, 8.0, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    let inner = rect.shrink2(vec2(8.0, 6.0));
    painter.text(inner.left_top(), egui::Align2::LEFT_TOP, number.to_string(), egui::FontId::proportional(11.0), theme::MUTED);
    painter.text(
        inner.left_top() + vec2(0.0, 14.0),
        egui::Align2::LEFT_TOP,
        &template.name,
        egui::FontId::new(13.0, theme::medium()),
        theme::TEXT,
    );
    if let Some(line) = template.lines.first() {
        let wave = egui::Rect::from_min_size(egui::pos2(inner.right() - 46.0, inner.bottom() - 12.0), vec2(46.0, 12.0));
        paint_wave(&painter, wave, line.waveform, theme::EFFECT);
    }
    let label = format!("{number} {}", template.name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    response.on_hover_text("Lancer sur la sélection").clicked()
}

/// Plage basse–haute d'une ligne, de 0 à 100 % : bande violette dont on glisse
/// le bord le plus proche. Renvoie `true` si la plage a changé.
fn range_bar(ui: &mut egui::Ui, id: egui::Id, low: &mut f32, high: &mut f32, width: f32) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(width, 14.0), Sense::click_and_drag());
    let value_at = |x: f32| ((x - rect.left()) / rect.width() * 100.0).clamp(0.0, 100.0);
    let mut changed = false;
    if let Some(pos) = response.interact_pointer_pos() {
        // Bord saisi au début du geste : on le garde jusqu'au relâchement.
        let edge_is_high = if response.drag_started() || response.clicked() {
            let v = value_at(pos.x);
            let high_edge = (v - *high).abs() < (v - *low).abs();
            ui.data_mut(|d| d.insert_temp(id, high_edge));
            high_edge
        } else {
            ui.data_mut(|d| d.get_temp::<bool>(id)).unwrap_or(true)
        };
        let v = value_at(pos.x).round();
        let target = if edge_is_high { &mut *high } else { &mut *low };
        if *target != v {
            *target = v;
            changed = true;
        }
    }
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, theme::SURFACE_2);
    let (a, b) = (low.min(*high), low.max(*high));
    let band = egui::Rect::from_x_y_ranges(
        rect.left() + rect.width() * a / 100.0..=rect.left() + rect.width() * b / 100.0,
        rect.y_range(),
    );
    painter.rect_filled(band, 3.0, theme::EFFECT.gamma_multiply(0.55));
    if response.hovered() || response.dragged() {
        painter.rect_stroke(rect, 3.0, egui::Stroke::new(1.0, theme::MUTED), egui::StrokeKind::Inside);
    }
    let text = format!("Plage {a:.0}–{b:.0} %");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, &text));
    changed
}

/// Carte d'un effet en cours : nom, cibles, Arrêter, réglages, une ligne par attribut.
/// Renvoie (changé, arrêter).
fn effect_card(ui: &mut egui::Ui, index: usize, effect: &mut Effect) -> (bool, bool) {
    let mut stop = false;
    let mut changed = false;
    let frame = egui::Frame::new().fill(theme::SURFACE_1).stroke(egui::Stroke::new(1.0, theme::LINE)).inner_margin(egui::Margin { left: 14, right: 12, top: 10, bottom: 10 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new(&effect.name).family(theme::medium()));
            let count = effect.targets.len();
            ui.label(RichText::new(format!("{count} cible{}", if count > 1 { "s" } else { "" })).size(11.5).color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let text = RichText::new("Arrêter").color(Color32::from_rgb(255, 138, 132));
                let button = egui::Button::new(text).stroke(egui::Stroke::new(1.0, theme::DANGER));
                stop = ui.add(button).on_hover_text(format!("Arrêter « {} »", effect.name)).clicked();
            });
        });
        ui.add_space(4.0);
        changed |= effect_settings(ui, index, effect);
    });
    // Liseré violet à gauche : c'est un effet.
    let r = frame.response.rect;
    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(3.0, r.height())), 0.0, theme::EFFECT);
    (changed, stop)
}

pub fn effects_window(ui: &mut egui::Ui, console: &mut Console) -> Action {
    let mut action = tempo_card(ui, console);
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        theme::section(ui, "Modèles");
        ui.label(RichText::new("Sélectionnez des fixtures puis cliquez un modèle (ou tapez « Effect 2 »)").size(11.5).color(theme::MUTED));
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (i, template) in templates().iter().enumerate() {
            let number = i as u32 + 1;
            if template_tile(ui, number, template) {
                action = Some(console.apply_effect(number));
            }
        }
    });
    ui.add_space(10.0);

    theme::section(ui, "En cours dans le programmer");
    if console.programmer_effects().is_empty() {
        ui.label(RichText::new("Aucun effet. Store Cue les enregistre dans une cue ; Clear les arrête.").color(theme::MUTED));
    }
    let mut changed = false;
    let mut remove = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (index, effect) in console.programmer_effects_mut().iter_mut().enumerate() {
            let (c, stop) = effect_card(ui, index, effect);
            changed |= c;
            if stop {
                remove = Some(index);
            }
            ui.add_space(6.0);
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
fn effect_settings(ui: &mut egui::Ui, index: usize, effect: &mut Effect) -> bool {
    let mut changed = false;
    let muted = |text: &str| RichText::new(text).color(theme::MUTED);
    ui.horizontal_wrapped(|ui| {
        let label = ui.label(muted("Vitesse"));
        let synced = matches!(effect.rate, EffectRate::Beats(_));
        let mode = if synced { "Tempo" } else { "Hz" };
        egui::ComboBox::from_id_salt(("rate", index))
            .selected_text(mode)
            .width(70.0)
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
        ui.add_space(8.0);
        let label = ui.label(muted("Éventail"));
        changed |= ui.add(egui::DragValue::new(&mut effect.spread).range(0.0..=720.0).suffix("°")).labelled_by(label.id).changed();
        ui.add_space(8.0);
        let label = ui.label(muted("Ailes"));
        changed |= ui.add(egui::DragValue::new(&mut effect.wings).range(1..=8)).labelled_by(label.id).changed();
        ui.add_space(8.0);
        let label = ui.label(muted("Blocs"));
        changed |= ui.add(egui::DragValue::new(&mut effect.blocks).range(1..=16)).labelled_by(label.id).changed();
    });
    ui.add_space(4.0);

    for (line_index, line) in effect.lines.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let name = line.attribute.label();
            ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new(&name).family(theme::medium())).truncate());
            egui::ComboBox::from_id_salt(("waveform", index, line_index))
                .selected_text(line.waveform.label())
                .width(120.0)
                .show_ui(ui, |ui| {
                    for w in Waveform::ALL {
                        changed |= ui.selectable_value(&mut line.waveform, w, w.label()).changed();
                    }
                })
                .response
                .on_hover_text(format!("Forme d'onde : {name}"));
            let id = ui.make_persistent_id(("range-edge", index, line_index));
            let width = (ui.available_width() - 250.0).clamp(80.0, 260.0);
            changed |= range_bar(ui, id, &mut line.low, &mut line.high, width);
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

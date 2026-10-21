//! Fenêtre Effets : tempo, modèles, réglage des effets du programmer.
//! Le violet y est la couleur des effets, comme partout ailleurs.

use eframe::egui::{self, Color32, RichText, Sense, vec2};
use folie_control::{CommandError, Console, Outcome};
use folie_show::{Effect, EffectRate, Waveform, effects::templates};

use crate::theme;

type Action = Option<Result<Outcome, CommandError>>;

const TILE: egui::Vec2 = egui::vec2(124.0, 58.0);

/// Courbe d'une forme d'onde dans `rect` : une période, ou huit tirages pour l'aléatoire.
fn paint_wave(painter: &egui::Painter, rect: egui::Rect, wave: Waveform, duty: f32, color: Color32) {
    let engine = folie_control::effects::waveform(wave);
    let cycles = if wave == Waveform::Random { 8.0 } else { 1.0 };
    let points: Vec<egui::Pos2> = (0..=96)
        .map(|i| {
            let t = i as f64 / 96.0;
            let level = engine.level_with(t * cycles, duty as f64 / 100.0, 1) as f32;
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

/// Case d'un modèle ou de Mes effets : numéro, nom, aperçu de son onde. Clic : lancer sur la sélection.
fn template_tile(ui: &mut egui::Ui, number: u32, template: &Effect) -> egui::Response {
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
        paint_wave(&painter, wave, line.waveform, line.duty, theme::EFFECT);
    }
    let label = format!("{number} {}", template.name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    response.on_hover_text("Lancer sur la sélection")
}

/// Menu du clic droit d'une case de Mes effets : renommer, supprimer.
fn pool_menu(ui: &mut egui::Ui, console: &mut Console, number: u32, name: &str) -> Action {
    theme::menu_fields(ui);
    let id = egui::Id::unique(("rename-pool-effect", number));
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| name.to_string());
    let mut action = None;
    ui.horizontal(|ui| {
        let label = ui.label("Nom");
        let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(150.0)).labelled_by(label.id);
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Renommer").clicked() || enter) && !text.trim().is_empty() {
            action = Some(console.rename_pool_effect(number, text.trim().to_string()));
        }
    });
    if ui.button("Supprimer").on_hover_text(format!("Retirer « {name} » de Mes effets")).clicked() {
        action = Some(console.delete_pool_effect(number));
    }
    if action.is_some() {
        ui.data_mut(|d| d.remove::<String>(id));
        ui.close();
    } else {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    action
}

/// Ce que l'on tient pendant un glissé sur la plage.
#[derive(Clone, Copy, PartialEq)]
enum Grip {
    Low,
    High,
    Band,
}

/// Geste en cours sur une plage : ce qui est tenu, et la plage au début du geste.
#[derive(Clone, Copy)]
struct RangeDrag {
    grip: Grip,
    low: f32,
    high: f32,
    start_x: f32,
}

/// Plage basse–haute d'une ligne : bande violette avec une poignée à chaque bord.
/// Glisser une poignée déplace ce bord, glisser la bande la décale sans changer sa largeur,
/// Option-glisser une poignée élargit la bande de façon symétrique ; un clic ne fait rien.
/// De 0 à 100 %, ou de −100 à +100 % autour de la valeur en cours (trait gris) pour une
/// ligne relative. Renvoie `true` si la plage a changé.
fn range_bar(ui: &mut egui::Ui, id: egui::Id, low: &mut f32, high: &mut f32, relative: bool, width: f32) -> bool {
    const GRIP: f32 = 6.0;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 22.0), Sense::drag());
    let track = egui::Rect::from_center_size(rect.center(), vec2(rect.width(), 14.0));
    let min = if relative { -100.0 } else { 0.0 };
    let span = 100.0 - min;
    let x_of = |v: f32| track.left() + track.width() * (v - min) / span;
    let value_per_point = span / track.width();
    let near = |x: f32, v: f32| (x - x_of(v)).abs() <= GRIP;
    let grip_at = |x: f32, low: f32, high: f32| {
        if near(x, high) && (x >= x_of(high) || !near(x, low)) {
            Some(Grip::High)
        } else if near(x, low) {
            Some(Grip::Low)
        } else if x > x_of(low) && x < x_of(high) {
            Some(Grip::Band)
        } else {
            None
        }
    };

    let mut changed = false;
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        match grip_at(pos.x, *low, *high) {
            Some(grip) => {
                ui.data_mut(|d| d.insert_temp(id, RangeDrag { grip, low: *low, high: *high, start_x: pos.x }));
            }
            None => ui.data_mut(|d| d.remove::<RangeDrag>(id)),
        }
    }
    let drag = ui.data_mut(|d| d.get_temp::<RangeDrag>(id)).filter(|_| response.dragged());
    if let (Some(drag), Some(pos)) = (drag, response.interact_pointer_pos()) {
        let delta = ((pos.x - drag.start_x) * value_per_point).round();
        let symmetric = ui.input(|i| i.modifiers.alt);
        let (new_low, new_high) = match drag.grip {
            Grip::Band => {
                // Décalée en entier, sans sortir de la barre.
                let delta = delta.clamp(min - drag.low, 100.0 - drag.high);
                (drag.low + delta, drag.high + delta)
            }
            Grip::High if symmetric => {
                let center = (drag.low + drag.high) / 2.0;
                let half = (drag.high + delta - center).max(0.0).min(100.0 - center).min(center - min);
                (center - half, center + half)
            }
            Grip::Low if symmetric => {
                let center = (drag.low + drag.high) / 2.0;
                let half = (center - (drag.low + delta)).max(0.0).min(100.0 - center).min(center - min);
                (center - half, center + half)
            }
            Grip::High => (drag.low, (drag.high + delta).clamp(drag.low, 100.0)),
            Grip::Low => ((drag.low + delta).clamp(min, drag.high), drag.high),
        };
        if (new_low, new_high) != (*low, *high) {
            (*low, *high) = (new_low, new_high);
            changed = true;
        }
    }
    if response.drag_stopped() {
        ui.data_mut(|d| d.remove::<RangeDrag>(id));
    }

    // Curseur : flèche gauche-droite sur une poignée, main sur la bande.
    let held = drag.map(|d| d.grip);
    let hovered = response.hover_pos().and_then(|p| grip_at(p.x, *low, *high));
    match held.or(hovered) {
        Some(Grip::Low | Grip::High) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
        Some(Grip::Band) if held.is_some() => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
        Some(Grip::Band) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
        None => {}
    }

    let painter = ui.painter();
    painter.rect_filled(track, 3.0, theme::SURFACE_2);
    let band = egui::Rect::from_x_y_ranges(x_of(*low)..=x_of(*high), track.y_range());
    painter.rect_filled(band, 3.0, theme::EFFECT.gamma_multiply(0.45));
    if relative {
        // La valeur en cours, au centre : l'effet tourne autour.
        let center = egui::Rect::from_center_size(egui::pos2(x_of(0.0), track.center().y), vec2(2.0, track.height() + 6.0));
        painter.rect_filled(center, 1.0, theme::MUTED);
    }
    for (value, grip) in [(*low, Grip::Low), (*high, Grip::High)] {
        let active = held == Some(grip) || (held.is_none() && hovered == Some(grip));
        let handle = egui::Rect::from_center_size(egui::pos2(x_of(value), rect.center().y), vec2(8.0, 20.0));
        painter.rect_filled(handle, 3.0, if active { egui::Color32::WHITE } else { theme::TEXT });
        painter.rect_stroke(handle, 3.0, egui::Stroke::new(1.0, theme::EFFECT), egui::StrokeKind::Inside);
    }

    // Pendant le glissé, la valeur au-dessus de ce que l'on tient.
    if let Some(grip) = held {
        let sign = |v: f32| if relative { format!("{v:+}") } else { format!("{v}") };
        let (text, x) = match grip {
            Grip::Low => (format!("{} %", sign(*low)), x_of(*low)),
            Grip::High => (format!("{} %", sign(*high)), x_of(*high)),
            Grip::Band => (format!("{} à {} %", sign(*low), sign(*high)), (x_of(*low) + x_of(*high)) / 2.0),
        };
        let layer = egui::LayerId::new(egui::Order::Tooltip, id.with("valeur"));
        let painter = ui.ctx().layer_painter(layer);
        let galley = painter.layout_no_wrap(text, egui::FontId::monospace(11.0), theme::TEXT);
        let tip = egui::Rect::from_center_size(egui::pos2(x, rect.top() - 10.0), galley.size() + vec2(8.0, 2.0));
        painter.rect_filled(tip, 4.0, theme::SURFACE_3);
        painter.rect_stroke(tip, 4.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside);
        painter.galley(tip.min + vec2(4.0, 1.0), galley, theme::TEXT);
    }

    let (a, b) = (*low, *high);
    let text = if relative { format!("Plage {a:+.0} à {b:+.0} % autour de la valeur") } else { format!("Plage {a:.0}–{b:.0} %") };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, &text));
    changed
}

/// Ce qu'on a demandé sur la carte d'un effet en cours.
#[derive(Default)]
struct CardActions {
    changed: bool,
    stop: bool,
    to_pool: bool,
}

/// Carte d'un effet en cours : nom, cibles, Au pool, Arrêter, réglages, une ligne par attribut.
fn effect_card(ui: &mut egui::Ui, index: usize, effect: &mut Effect) -> CardActions {
    let mut stop = false;
    let mut to_pool = false;
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
                let help = "Enregistrer cet effet dans Mes effets, pour le relancer sur une autre sélection";
                to_pool = ui.button("Au pool").on_hover_text(help).clicked();
            });
        });
        ui.add_space(4.0);
        changed |= effect_settings(ui, index, effect);
    });
    // Liseré violet à gauche : c'est un effet.
    let r = frame.response.rect;
    ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(3.0, r.height())), 0.0, theme::EFFECT);
    CardActions { changed, stop, to_pool }
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
            if template_tile(ui, number, template).clicked() {
                action = Some(console.apply_effect(number));
            }
        }
    });
    ui.add_space(8.0);

    ui.horizontal(|ui| {
        theme::section(ui, "Mes effets");
        ui.label(RichText::new("Clic : lancer sur la sélection · clic droit : renommer, supprimer").size(11.5).color(theme::MUTED));
    });
    let pool: Vec<(u32, Effect)> = console.show.effect_pool.iter().map(|(n, e)| (*n, e.clone())).collect();
    if pool.is_empty() {
        ui.label(RichText::new("Vide : « Au pool » sur un effet en cours l'enregistre ici.").color(theme::MUTED));
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (number, effect) in &pool {
            let response = template_tile(ui, *number, effect);
            if response.clicked() {
                action = Some(console.apply_pool_effect(*number));
            }
            egui::Popup::context_menu(&response).show(|ui| {
                if let Some(a) = pool_menu(ui, console, *number, &effect.name) {
                    action = Some(a);
                }
            });
        }
    });
    ui.add_space(10.0);

    theme::section(ui, "En cours dans le programmer");
    if console.programmer_effects().is_empty() {
        ui.label(RichText::new("Aucun effet. Store Cue les enregistre dans une cue ; Clear les arrête.").color(theme::MUTED));
    }
    let mut changed = false;
    let mut remove = None;
    let mut to_pool = None;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for (index, effect) in console.programmer_effects_mut().iter_mut().enumerate() {
            let card = effect_card(ui, index, effect);
            changed |= card.changed;
            if card.stop {
                remove = Some(index);
            }
            if card.to_pool {
                to_pool = Some(index);
            }
            ui.add_space(6.0);
        }
    });
    if let Some(index) = to_pool {
        action = Some(console.store_effect_in_pool(index));
    }
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
        ui.add_space(8.0);
        ui.label(muted("Fondus"));
        fn seconds<'a>(value: &'a mut f32, prefix: &str) -> egui::DragValue<'a> {
            egui::DragValue::new(value).range(0.0..=3600.0).speed(0.1).max_decimals(2).prefix(prefix).suffix(" s")
        }
        changed |= ui
            .add(seconds(&mut effect.fade_in, "entrée "))
            .on_hover_text("Secondes pour que l'effet grandisse à son lancement")
            .changed();
        changed |= ui
            .add(seconds(&mut effect.fade_out, "sortie "))
            .on_hover_text("Secondes pour qu'il s'efface à son arrêt, au lieu de couper net")
            .changed();
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
            if line.waveform == Waveform::Square {
                changed |= ui
                    .add(egui::DragValue::new(&mut line.duty).range(1.0..=99.0).prefix("allumé ").suffix(" %"))
                    .on_hover_text(format!("Part du cycle où le carré est allumé : {name}"))
                    .changed();
            }
            // Absolu (de bas en haut) ou relatif (autour de la valeur en cours).
            let mut relative = line.relative;
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                // Violet : c'est un réglage d'effet.
                ui.visuals_mut().selection.bg_fill = theme::EFFECT.gamma_multiply(0.3);
                ui.visuals_mut().selection.stroke = egui::Stroke::new(1.0, theme::EFFECT);
                ui.selectable_value(&mut relative, false, "Absolu").on_hover_text(format!("{name} : entre deux valeurs fixes"));
                ui.selectable_value(&mut relative, true, "Relatif").on_hover_text(format!("{name} : autour de la valeur en cours"));
            });
            if relative != line.relative {
                line.relative = relative;
                (line.low, line.high) = if relative { (-10.0, 10.0) } else { (0.0, 100.0) };
                changed = true;
            }
            let id = ui.make_persistent_id(("range-edge", index, line_index));
            let width = (ui.available_width() - 250.0).clamp(80.0, 260.0);
            changed |= range_bar(ui, id, &mut line.low, &mut line.high, line.relative, width);
            let (range, low, high) =
                if line.relative { (-100.0..=100.0, "de ", "à ") } else { (0.0..=100.0, "bas ", "haut ") };
            // En relatif, le signe est toujours écrit : « de -10 % à +10 % ».
            fn field<'a>(value: &'a mut f32, range: std::ops::RangeInclusive<f32>, prefix: &str, signed: bool) -> egui::DragValue<'a> {
                let field = egui::DragValue::new(value).range(range).prefix(prefix).suffix(" %");
                if signed { field.custom_formatter(|v, _| format!("{v:+}")) } else { field }
            }
            let signed = line.relative;
            changed |=
                ui.add(field(&mut line.low, range.clone(), low, signed)).on_hover_text(format!("Valeur basse : {name}")).changed();
            changed |= ui.add(field(&mut line.high, range, high, signed)).on_hover_text(format!("Valeur haute : {name}")).changed();
            changed |= ui
                .add(egui::DragValue::new(&mut line.phase).range(-360.0..=360.0).prefix("phase ").suffix("°"))
                .on_hover_text(format!("Décalage : {name}"))
                .changed();
        });
    }
    changed
}

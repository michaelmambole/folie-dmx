//! Identité visuelle de Folie-DMX : palette, polices, tailles, espacements.
//!
//! Une couleur a toujours le même sens, dans toutes les fenêtres :
//! rouge = programmer, ambre = executor, violet = effet, gris = valeur par
//! défaut, jaune = sélection, vert = action principale (GO).

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke, TextStyle,
};

// --- Fonds, du plus profond au plus proche -------------------------------
pub const BG: Color32 = Color32::from_rgb(20, 21, 24);
pub const SURFACE_1: Color32 = Color32::from_rgb(28, 30, 34);
pub const SURFACE_2: Color32 = Color32::from_rgb(36, 39, 44);
pub const SURFACE_3: Color32 = Color32::from_rgb(46, 50, 57);
pub const LINE: Color32 = Color32::from_rgb(48, 52, 58);

// --- Texte ---------------------------------------------------------------
pub const TEXT: Color32 = Color32::from_rgb(232, 230, 225);
pub const MUTED: Color32 = Color32::from_rgb(139, 143, 150);

// --- Sens -----------------------------------------------------------------
pub const PROGRAMMER: Color32 = Color32::from_rgb(255, 90, 82);
pub const EXECUTOR: Color32 = Color32::from_rgb(240, 166, 58);
pub const EFFECT: Color32 = Color32::from_rgb(167, 139, 250);
pub const DEFAULT: Color32 = Color32::from_rgb(91, 96, 104);
pub const SELECTION: Color32 = Color32::from_rgb(255, 209, 102);
pub const GO: Color32 = Color32::from_rgb(63, 182, 139);
pub const DANGER: Color32 = Color32::from_rgb(214, 54, 48);
pub const INFO: Color32 = Color32::from_rgb(60, 130, 230);

/// Coins arrondis : contrôles, cartes.
pub const RADIUS: u8 = 6;
pub const CARD_RADIUS: u8 = 10;

/// D'où vient une valeur : c'est ce que dit sa couleur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Programmer,
    Effect,
    Executor,
    Default,
}

impl Source {
    pub fn color(self) -> Color32 {
        match self {
            Self::Programmer => PROGRAMMER,
            Self::Effect => EFFECT,
            Self::Executor => EXECUTOR,
            Self::Default => DEFAULT,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Programmer => "Programmer",
            Self::Effect => "Effet",
            Self::Executor => "Executor",
            Self::Default => "Par défaut",
        }
    }

    pub const ALL: [Source; 4] = [Self::Programmer, Self::Executor, Self::Effect, Self::Default];
}

/// Police des titres et des boutons importants.
pub fn medium() -> FontFamily {
    FontFamily::Name("medium".into())
}

fn fonts() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let add = |fonts: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut fonts, "inter", include_bytes!("../assets/fonts/Inter-Regular.ttf"));
    add(&mut fonts, "inter-medium", include_bytes!("../assets/fonts/Inter-Medium.ttf"));
    add(&mut fonts, "jetbrains-mono", include_bytes!("../assets/fonts/JetBrainsMono-Regular.ttf"));

    // Nos polices d'abord ; celles d'egui restent derrière pour les caractères rares.
    let fallback = |fonts: &FontDefinitions, family: FontFamily| fonts.families.get(&family).cloned().unwrap_or_default();
    let proportional = fallback(&fonts, FontFamily::Proportional);
    let monospace = fallback(&fonts, FontFamily::Monospace);
    fonts.families.insert(FontFamily::Proportional, [vec!["inter".to_string()], proportional.clone()].concat());
    fonts.families.insert(FontFamily::Monospace, [vec!["jetbrains-mono".to_string()], monospace].concat());
    fonts.families.insert(medium(), [vec!["inter-medium".to_string()], proportional].concat());
    fonts
}

fn widget(fill: Color32, stroke: Color32, text: Color32) -> egui::style::WidgetVisuals {
    egui::style::WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, stroke),
        corner_radius: CornerRadius::same(RADIUS),
        fg_stroke: Stroke::new(1.0, text),
        expansion: 0.0,
    }
}

/// Applique l'identité visuelle à toute l'application.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.set_fonts(fonts());
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = [
            (TextStyle::Heading, FontId::new(18.0, medium())),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        ]
        .into();

        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(8.0, 6.0);
        spacing.button_padding = egui::vec2(10.0, 4.0);
        spacing.interact_size = egui::vec2(40.0, 24.0);
        spacing.window_margin = Margin::same(10);
        spacing.menu_margin = Margin::same(8);

        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = SURFACE_1;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.extreme_bg_color = Color32::from_rgb(14, 15, 17);
        v.faint_bg_color = SURFACE_1;
        v.code_bg_color = SURFACE_2;
        v.override_text_color = None;
        v.hyperlink_color = INFO;
        v.striped = true;
        v.selection.bg_fill = SELECTION.gamma_multiply(0.28);
        v.selection.stroke = Stroke::new(1.0, SELECTION);
        v.widgets.noninteractive = widget(SURFACE_1, LINE, MUTED);
        v.widgets.inactive = widget(SURFACE_2, LINE, TEXT);
        v.widgets.hovered = widget(SURFACE_3, Color32::from_rgb(70, 75, 84), TEXT);
        v.widgets.active = widget(Color32::from_rgb(56, 61, 69), SELECTION, TEXT);
        v.widgets.open = widget(SURFACE_3, LINE, TEXT);
    });
}

/// Carte : un groupe d'éléments sur un fond légèrement surélevé.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SURFACE_1)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, add)
        .inner
}

/// Dans un menu, egui dessine les boutons sans fond ni cadre : les champs, cases à cocher
/// et touches y ressembleraient à du texte. À appeler en tête de menu pour les rendre visibles.
pub fn menu_fields(ui: &mut egui::Ui) {
    let visuals = ui.visuals_mut();
    visuals.button_frame = true;
    let inactive = &mut visuals.widgets.inactive;
    inactive.weak_bg_fill = BG;
    inactive.bg_fill = BG;
    inactive.bg_stroke = Stroke::new(1.0, LINE);
}

/// Titre de section discret, en capitales : « POSITION ».
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(egui::RichText::new(title.to_uppercase()).size(11.0).color(MUTED).extra_letter_spacing(0.8));
}

/// Titre de section repliable : chevron et titre en capitales, toute la ligne est cliquable.
/// Renvoie `true` quand on a cliqué (l'appelant bascule l'état).
pub fn collapsible_section(ui: &mut egui::Ui, title: &str, open: bool) -> bool {
    let text = title.to_uppercase();
    let galley = egui::WidgetText::from(egui::RichText::new(&text).size(11.0).extra_letter_spacing(0.8)).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        f32::INFINITY,
        egui::TextStyle::Body,
    );
    let height = galley.size().y.max(16.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), egui::Sense::click());
    let color = if response.hovered() { TEXT } else { MUTED };
    // Chevron dessiné : les triangles ne sont pas dans les polices embarquées.
    let center = egui::pos2(rect.left() + 5.0, rect.center().y);
    let points = if open {
        vec![center + egui::vec2(-4.0, -2.0), center + egui::vec2(4.0, -2.0), center + egui::vec2(0.0, 3.0)]
    } else {
        vec![center + egui::vec2(-2.0, -4.0), center + egui::vec2(3.0, 0.0), center + egui::vec2(-2.0, 4.0)]
    };
    let painter = ui.painter();
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    painter.galley(egui::pos2(rect.left() + 16.0, rect.center().y - galley.size().y / 2.0), galley, color);
    let response = response.on_hover_text(if open { "Replier" } else { "Déplier" });
    response.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::DisclosureTriangle, true, open, &text));
    response.clicked()
}

/// Pastille de couleur avec un libellé, pour les légendes.
pub fn chip(ui: &mut egui::Ui, color: Color32, label: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 2.0, color);
        ui.label(egui::RichText::new(label).size(12.0).color(MUTED));
    });
}

/// Légende des couleurs de source.
pub fn legend(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        for source in Source::ALL {
            chip(ui, source.color(), source.label());
            ui.add_space(6.0);
        }
    });
}

/// Touche de jeu de la barre du haut (STORE, CLEAR, BLACKOUT) : capitales, cadre coloré.
pub fn key_button(text: &str, stroke: Color32, fill: Color32, color: Color32) -> egui::Button<'_> {
    let text = egui::RichText::new(text).family(medium()).size(12.5).extra_letter_spacing(0.6).color(color);
    egui::Button::new(text).fill(fill).stroke(Stroke::new(1.0, stroke)).min_size(egui::vec2(78.0, 26.0))
}

/// Pastille d'état de la barre du haut : un point (vert actif, rouge en erreur, gris
/// sinon) et un libellé discret. Cliquable.
pub fn status_chip(ui: &mut egui::Ui, label: &str, state: Option<Color32>) -> egui::Response {
    let text = egui::RichText::new(label).size(12.0).color(if state.is_some() { TEXT } else { MUTED });
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    const DOT: f32 = 7.0;
    const PAD: f32 = 7.0;
    let size = egui::vec2(PAD + DOT + 6.0 + galley.size().x + PAD, 24.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, RADIUS, SURFACE_2);
    }
    let dot = egui::pos2(rect.left() + PAD + DOT / 2.0, rect.center().y);
    painter.circle_filled(dot, DOT / 2.0, state.unwrap_or(DEFAULT));
    let text_pos = egui::pos2(rect.left() + PAD + DOT + 6.0, rect.center().y - galley.size().y / 2.0);
    painter.galley(text_pos, galley, TEXT);
    let label = label.to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, &label));
    response
}

/// Bouton d'action principale (GO) : plein, vert.
pub fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).family(medium()).color(Color32::from_rgb(10, 40, 30))).fill(GO)
}

/// Glisser relatif ancré : la valeur suit la distance parcourue depuis le début du geste,
/// pas la valeur relue à chaque image. Celle-ci revient du moteur, qui tourne à son propre
/// rythme : y ajouter le déplacement de l'image perdait les pas où il n'avait pas encore
/// répondu (valeur saccadée, plus lente que la souris). Maj (fin) se prend ou se lâche
/// en cours de geste sans saut. `along` donne la position utile du pointeur (x ou -y).
fn anchored_drag(ui: &egui::Ui, response: &egui::Response, value: f32, length: f32, along: fn(egui::Pos2) -> f32) -> Option<f32> {
    #[derive(Clone, Copy)]
    struct Anchor {
        value: f32,
        pointer: f32,
        fine: bool,
        last: f32,
    }
    let id = response.id.with("anchored_drag");
    if !response.dragged() {
        ui.data_mut(|d| d.remove::<Anchor>(id));
        return None;
    }
    let pointer = along(response.interact_pointer_pos()?);
    let fine = ui.input(|i| i.modifiers.shift);
    let anchor = match ui.data(|d| d.get_temp::<Anchor>(id)) {
        // Début du geste : on part de la valeur affichée, déplacement déjà fait compris.
        None => Anchor { value: value + along(response.drag_delta().to_pos2()) / length, pointer, fine, last: value },
        // Maj change : on repart de la dernière valeur donnée, à l'endroit du pointeur.
        Some(a) if a.fine != fine => Anchor { value: a.last, pointer, fine, last: a.last },
        Some(a) => a,
    };
    let scale = if fine { 0.1 } else { 1.0 };
    let next = (anchor.value + (pointer - anchor.pointer) / length * scale).clamp(0.0, 1.0);
    ui.data_mut(|d| d.insert_temp(id, Anchor { last: next, ..anchor }));
    Some(next)
}

/// Barre de valeur réglable, de 0 à 1 : le niveau est rempli dans la couleur de
/// sa source et la valeur lisible est écrite dedans.
/// Glisser : réglage relatif (Maj : fin) ; clic : valeur directe ; molette : ajustement.
/// `band` : plage (0 à 1) dessinée en clair derrière le niveau, celle d'un effet par exemple.
pub fn value_bar(
    ui: &mut egui::Ui,
    value: &mut f32,
    color: Color32,
    text: &str,
    width: f32,
    band: Option<(f32, f32)>,
) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(egui::vec2(width, 20.0), egui::Sense::click_and_drag());
    let before = *value;
    // Option-clic : réservé à l'appelant (relâcher la valeur), la barre ne bouge pas.
    let alt = ui.input(|i| i.modifiers.alt);
    if let Some(dragged) = anchored_drag(ui, &response, *value, rect.width(), |p| p.x).filter(|_| !alt) {
        *value = dragged;
    } else if !alt
        && response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        *value = (pos.x - rect.left()) / rect.width();
    }
    if response.hovered() && !alt {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        *value += scroll / 2000.0;
    }
    *value = value.clamp(0.0, 1.0);
    if *value != before {
        response.mark_changed();
    }

    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, SURFACE_2);
    if let Some((low, high)) = band {
        let (low, high) = (low.min(high).clamp(0.0, 1.0), low.max(high).clamp(0.0, 1.0));
        let x = |t: f32| rect.left() + rect.width() * t;
        let range = egui::Rect::from_x_y_ranges(x(low)..=x(high), rect.y_range());
        painter.rect_filled(range, 4.0, color.gamma_multiply(0.22));
    }
    let filled = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * *value, rect.height()));
    painter.rect_filled(filled, 4.0, color.gamma_multiply(0.85));
    if response.hovered() || response.dragged() {
        painter.rect_stroke(rect, 4.0, Stroke::new(1.0, Color32::from_rgb(90, 96, 106)), egui::StrokeKind::Inside);
    }
    painter.text(
        rect.right_center() - egui::vec2(6.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        text,
        FontId::monospace(12.0),
        TEXT,
    );
    let shown = *value;
    response.widget_info(|| egui::WidgetInfo::slider(true, shown as f64, text));
    response
}

/// Fader vertical dessiné : piste, niveau rempli dans la couleur de la source,
/// curseur. Glisser : réglage relatif (Maj : fin) ; clic : valeur directe ; molette.
pub fn fader(ui: &mut egui::Ui, level: &mut f32, color: Color32, enabled: bool, size: egui::Vec2) -> egui::Response {
    let sense = if enabled { egui::Sense::click_and_drag() } else { egui::Sense::hover() };
    let (rect, mut response) = ui.allocate_exact_size(size, sense);
    let before = *level;
    if enabled {
        if let Some(dragged) = anchored_drag(ui, &response, *level, rect.height(), |p| -p.y) {
            *level = dragged;
        } else if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            *level = (rect.bottom() - pos.y) / rect.height();
        }
        if response.hovered() {
            *level += ui.input(|i| i.smooth_scroll_delta.y) / 1500.0;
        }
        *level = level.clamp(0.0, 1.0);
        if *level != before {
            response.mark_changed();
        }
    }

    let painter = ui.painter();
    painter.rect_filled(rect, 6.0, if enabled { SURFACE_2 } else { SURFACE_1 });
    if enabled {
        let fill_top = rect.bottom() - rect.height() * *level;
        let filled = egui::Rect::from_min_max(egui::pos2(rect.left(), fill_top), rect.max);
        painter.rect_filled(filled, 6.0, color.gamma_multiply(0.55));
        // Curseur : une barre claire à la hauteur du niveau.
        let cap_y = fill_top.clamp(rect.top() + 5.0, rect.bottom() - 5.0);
        let cap = egui::Rect::from_center_size(egui::pos2(rect.center().x, cap_y), egui::vec2(rect.width() - 6.0, 9.0));
        let cap_color = if response.dragged() || response.hovered() { Color32::WHITE } else { TEXT };
        painter.rect_filled(cap, 3.0, cap_color);
    }
    painter.rect_stroke(rect, 6.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    let shown = *level;
    response.widget_info(|| egui::WidgetInfo::slider(enabled, shown as f64 * 100.0, "Fader"));
    response
}

/// Couleur repère d'une famille de presets.
pub fn family_color(family: folie_fixture::FeatureGroup) -> Color32 {
    use folie_fixture::FeatureGroup as F;
    match family {
        F::Dimmer => Color32::from_rgb(245, 211, 122),
        F::Position => Color32::from_rgb(79, 195, 181),
        F::Gobo => Color32::from_rgb(232, 121, 166),
        F::Color => Color32::from_rgb(255, 120, 90),
        F::Beam => Color32::from_rgb(240, 138, 93),
        F::Focus => Color32::from_rgb(106, 168, 255),
        F::Control => Color32::from_rgb(154, 160, 168),
    }
}

/// LED : un petit voyant rond, éteint s'il n'a pas de couleur.
pub fn led(painter: &egui::Painter, center: egui::Pos2, color: Option<Color32>) {
    match color {
        Some(color) => {
            painter.circle_filled(center, 4.0, color);
            painter.circle_stroke(center, 4.0, Stroke::new(1.0, color.gamma_multiply(0.5)));
        }
        None => {
            painter.circle_stroke(center, 3.5, Stroke::new(1.0, Color32::from_rgb(70, 75, 84)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// Un réglage dont la valeur relue ne suit pas (le moteur n'a pas encore répondu) :
    /// le glisser va quand même exactement où va la souris, Maj compris.
    #[test]
    fn drag_follows_the_pointer_even_when_the_value_lags() {
        struct State {
            shown: f32,
            sent: Vec<f32>,
        }
        let mut harness = Harness::builder().with_size(egui::vec2(300.0, 60.0)).build_ui_state(
            |ui, state: &mut State| {
                let mut value = state.shown;
                if value_bar(ui, &mut value, PROGRAMMER, "Dimmer", 200.0, None).changed() {
                    state.sent.push(value);
                }
                // La valeur relue n'est jamais mise à jour : le pire des retards.
            },
            State { shown: 0.25, sent: Vec::new() },
        );
        harness.run();
        let bar = harness.get_by_role_and_label(egui::accesskit::Role::Slider, "Dimmer").rect();
        let y = bar.center().y;
        let x = |v: f32| bar.left() + bar.width() * v;
        let press = |x: f32, pressed: bool, modifiers| egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let glide = |h: &mut Harness<'_, State>, from: f32, to: f32, modifiers| {
            for i in 1..=10 {
                h.event_modifiers(egui::Event::PointerMoved(egui::pos2(from + (to - from) * i as f32 / 10.0, y)), modifiers);
                h.run_steps(1);
            }
        };
        harness.hover_at(egui::pos2(x(0.25), y));
        harness.run_steps(1);
        harness.event(press(x(0.25), true, egui::Modifiers::NONE));
        harness.run_steps(1);
        glide(&mut harness, x(0.25), x(0.75), egui::Modifiers::NONE);
        let last = *harness.state().sent.last().unwrap();
        assert!((last - 0.75).abs() < 0.01, "la valeur suit la souris : {last}");
        // Maj en cours de geste : dix fois plus fin, sans saut.
        glide(&mut harness, x(0.75), x(0.95), egui::Modifiers::SHIFT);
        let last = *harness.state().sent.last().unwrap();
        assert!((last - 0.77).abs() < 0.01, "Maj : fin, à partir de la valeur atteinte : {last}");
        harness.event(press(x(0.95), false, egui::Modifiers::SHIFT));
        harness.run_steps(1);
    }
}

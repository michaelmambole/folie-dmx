use std::collections::BTreeSet;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use folie_dmx::{CHANNELS_PER_UNIVERSE, Universe};
use folie_engine::ChannelOverrides;

use crate::app::DisplayMode;
use crate::theme;

const COLUMNS: usize = 32;
const ROWS: usize = CHANNELS_PER_UNIVERSE / COLUMNS;
/// Canal occupé par une fixture : légèrement bleuté pour voir le patch d'un coup d'œil.
pub const PATCHED: Color32 = Color32::from_rgb(38, 48, 61);

/// Ce que montre la grille, en plus des valeurs envoyées.
pub struct Marks<'a> {
    /// `labels[i]` décrit ce que pilote le canal i+1 (fixture et paramètre), s'il est patché.
    pub labels: &'a [Option<String>],
    /// Premier canal de chaque fixture.
    pub starts: &'a BTreeSet<u16>,
    /// Canaux forcés depuis les faders.
    pub overrides: &'a ChannelOverrides,
}

/// Grille des 512 canaux tels qu'ils sortent réellement : le niveau en jauge neutre,
/// les canaux patchés bleutés, un trait au début de chaque fixture, les canaux forcés
/// cerclés de rouge et le canal sélectionné de jaune. Renvoie le canal cliqué.
pub fn show(ui: &mut egui::Ui, output: &Universe, marks: &Marks<'_>, display: DisplayMode, selected: Option<u16>) -> Option<u16> {
    let available = ui.available_size();
    let cell_w = (available.x / COLUMNS as f32).floor().max(26.0);
    let cell_h = (available.y / ROWS as f32).floor().clamp(22.0, cell_w);
    let size = vec2(cell_w * COLUMNS as f32, cell_h * ROWS as f32);

    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter_at(rect);

    let channel_at = |pos: egui::Pos2| -> Option<u16> {
        if !rect.contains(pos) {
            return None;
        }
        let col = ((pos.x - rect.left()) / cell_w) as usize;
        let row = ((pos.y - rect.top()) / cell_h) as usize;
        (col < COLUMNS && row < ROWS).then(|| (row * COLUMNS + col + 1) as u16)
    };

    for (index, &value) in output.as_slice().iter().enumerate() {
        let channel = index as u16 + 1;
        let min = pos2(rect.left() + (index % COLUMNS) as f32 * cell_w, rect.top() + (index / COLUMNS) as f32 * cell_h);
        let cell = Rect::from_min_size(min, vec2(cell_w, cell_h)).shrink(1.0);

        let patched = marks.labels.get(index).is_some_and(Option::is_some);
        painter.rect_filled(cell, 3.0, if patched { PATCHED } else { theme::SURFACE_2 });
        // Jauge : le niveau monte du bas de la case.
        let level = value as f32 / 255.0;
        if value > 0 {
            let top = cell.bottom() - cell.height() * level;
            let gauge = Rect::from_min_max(pos2(cell.left(), top), cell.max);
            painter.rect_filled(gauge, 3.0, theme::TEXT.gamma_multiply(0.55));
        }
        if marks.starts.contains(&channel) {
            let bar = Rect::from_min_size(cell.min, vec2(2.0, cell.height()));
            painter.rect_filled(bar, 1.0, theme::MUTED);
        }
        if marks.overrides.get(index).is_some_and(Option::is_some) {
            painter.rect_stroke(cell, 3.0, Stroke::new(1.0, theme::DANGER), StrokeKind::Inside);
        }
        if selected == Some(channel) {
            painter.rect_stroke(cell, 3.0, Stroke::new(2.0, theme::SELECTION), StrokeKind::Inside);
        }

        painter.text(cell.left_top() + vec2(3.0, 1.0), Align2::LEFT_TOP, channel.to_string(), FontId::proportional(8.5), theme::MUTED);
        if value > 0 {
            // Texte sombre quand la jauge passe derrière lui.
            let ink = if level > 0.35 { theme::BG } else { theme::TEXT };
            painter.text(cell.right_bottom() - vec2(3.0, 1.0), Align2::RIGHT_BOTTOM, display.format(value), FontId::monospace(10.5), ink);
        }
    }

    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Grid, true, "Canaux de l'univers 1"));
    let response = match response.hover_pos().and_then(channel_at) {
        Some(channel) => {
            let value = output.get(channel).unwrap_or(0);
            let mut text = format!("Canal {channel} : {value} DMX ({} %)", DisplayMode::Percent.format(value));
            if let Some(Some(label)) = marks.labels.get(channel as usize - 1) {
                text = format!("{text}\n{label}");
            }
            if marks.overrides.get(channel as usize - 1).is_some_and(Option::is_some) {
                text = format!("{text}\nforcé depuis les faders");
            }
            response.on_hover_text(text)
        }
        None => response,
    };

    if response.clicked() { response.interact_pointer_pos().and_then(channel_at) } else { None }
}

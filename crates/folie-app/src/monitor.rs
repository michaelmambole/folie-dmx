use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use folie_dmx::{CHANNELS_PER_UNIVERSE, Universe};

use crate::app::DisplayMode;

const COLUMNS: usize = 32;
const ROWS: usize = CHANNELS_PER_UNIVERSE / COLUMNS;
const EMPTY: Color32 = Color32::from_rgb(46, 46, 52);
/// Canal occupé par une fixture : légèrement bleuté pour voir le patch d'un coup d'œil.
const EMPTY_PATCHED: Color32 = Color32::from_rgb(40, 52, 72);
const AMBER: Color32 = Color32::from_rgb(255, 170, 40);

/// Grille des 512 canaux tels qu'ils sortent réellement.
/// `labels[i]` décrit ce que pilote le canal i+1 (fixture et paramètre), s'il est patché.
/// Renvoie le canal cliqué, s'il y en a un.
pub fn show(
    ui: &mut egui::Ui,
    output: &Universe,
    labels: &[Option<String>],
    display: DisplayMode,
    selected: Option<u16>,
) -> Option<u16> {
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
        let min = pos2(
            rect.left() + (index % COLUMNS) as f32 * cell_w,
            rect.top() + (index / COLUMNS) as f32 * cell_h,
        );
        let cell = Rect::from_min_size(min, vec2(cell_w, cell_h)).shrink(1.0);

        let base = if labels.get(index).is_some_and(Option::is_some) { EMPTY_PATCHED } else { EMPTY };
        painter.rect_filled(cell, 2.0, blend(base, AMBER, value as f32 / 255.0));
        if selected == Some(channel) {
            painter.rect_stroke(cell, 2.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        }

        let ink = if value > 150 { Color32::BLACK } else { Color32::from_gray(200) };
        painter.text(
            cell.left_top() + vec2(2.0, 1.0),
            Align2::LEFT_TOP,
            channel.to_string(),
            FontId::proportional(8.0),
            ink.gamma_multiply(0.6),
        );
        if value > 0 {
            painter.text(
                cell.center_bottom() - vec2(0.0, 2.0),
                Align2::CENTER_BOTTOM,
                display.format(value),
                FontId::monospace(11.0),
                ink,
            );
        }
    }

    let response = match response.hover_pos().and_then(channel_at) {
        Some(channel) => {
            let value = output.get(channel).unwrap_or(0);
            let mut text =
                format!("Canal {channel} : {value} DMX ({} %)", DisplayMode::Percent.format(value));
            if let Some(Some(label)) = labels.get(channel as usize - 1) {
                text = format!("{text}\n{label}");
            }
            response.on_hover_text(text)
        }
        None => response,
    };

    if response.clicked() {
        response.interact_pointer_pos().and_then(channel_at)
    } else {
        None
    }
}

fn blend(from: Color32, to: Color32, t: f32) -> Color32 {
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(mix(from.r(), to.r()), mix(from.g(), to.g()), mix(from.b(), to.b()))
}

use eframe::egui::{self, RichText, vec2};
use folie_dmx::{CHANNELS_PER_UNIVERSE, Universe};
use folie_engine::{ChannelOverrides, Command, EngineHandle};

use crate::app::DisplayMode;
use crate::theme;

pub const FADERS_PER_PAGE: usize = 16;
const PAGES: usize = CHANNELS_PER_UNIVERSE / FADERS_PER_PAGE;
const FADER_SIZE: egui::Vec2 = egui::vec2(30.0, 150.0);

/// Banque de faders, une page de 16 canaux à la fois, dessinés comme ceux des executors.
/// Un fader affiche la valeur réellement envoyée ; le bouger force le canal (en rouge).
pub fn show(
    ui: &mut egui::Ui,
    page: &mut usize,
    overrides: &ChannelOverrides,
    output: &Universe,
    display: DisplayMode,
    selected: Option<u16>,
    engine: &EngineHandle,
) {
    let forced: Vec<u16> = (1..=CHANNELS_PER_UNIVERSE as u16).filter(|c| overrides[*c as usize - 1].is_some()).collect();
    ui.horizontal(|ui| {
        theme::section(ui, "Faders");
        if ui.add_enabled(*page > 0, egui::Button::new("‹")).on_hover_text("Page précédente").clicked() {
            *page -= 1;
        }
        ui.label(RichText::new(format!("Page {}", *page + 1)).monospace());
        if ui.add_enabled(*page + 1 < PAGES, egui::Button::new("›")).on_hover_text("Page suivante").clicked() {
            *page += 1;
        }
        let first = *page * FADERS_PER_PAGE + 1;
        ui.label(RichText::new(format!("canaux {first}–{}", first + FADERS_PER_PAGE - 1)).color(theme::MUTED));
        ui.label(RichText::new("Double-clic : plein ou zéro · clic droit : relâcher").size(11.5).color(theme::MUTED));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !forced.is_empty() {
                let text = RichText::new(format!("Relâcher les canaux forcés ({})", forced.len()))
                    .color(egui::Color32::from_rgb(255, 138, 132));
                let button = egui::Button::new(text).stroke(egui::Stroke::new(1.0, theme::DANGER));
                if ui.add(button).on_hover_text("Rend tous les canaux forcés au moteur").clicked() {
                    for channel in &forced {
                        engine.send(Command::ReleaseChannel { universe: 0, channel: *channel });
                    }
                }
            }
        });
    });
    ui.add_space(6.0);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let first = *page * FADERS_PER_PAGE + 1;
        for channel in first..first + FADERS_PER_PAGE {
            let channel = channel as u16;
            let size = vec2(FADER_SIZE.x + 8.0, FADER_SIZE.y + 44.0);
            ui.allocate_ui_with_layout(size, egui::Layout::top_down(egui::Align::Center), |ui| {
                let forced = overrides[channel as usize - 1].is_some();
                fader(ui, channel, output, forced, display, selected == Some(channel), engine);
            });
        }
    });
}

fn fader(
    ui: &mut egui::Ui,
    channel: u16,
    output: &Universe,
    forced: bool,
    display: DisplayMode,
    is_selected: bool,
    engine: &EngineHandle,
) {
    let value = output.get(channel).unwrap_or(0);
    let value_color = if value > 0 { theme::TEXT } else { theme::MUTED };
    ui.label(RichText::new(display.format(value)).monospace().size(12.0).color(value_color));

    let mut level = value as f32 / 255.0;
    let color = if forced { theme::DANGER } else { theme::TEXT };
    let response = theme::fader(ui, &mut level, color, true, vec2(FADER_SIZE.x, FADER_SIZE.y));
    let name = format!("Canal {channel}");
    response.widget_info(|| egui::WidgetInfo::slider(true, level as f64 * 100.0, &name));
    let response = response.on_hover_text(if forced { format!("{name} : forcé (clic droit pour relâcher)") } else { name });
    if response.secondary_clicked() {
        engine.send(Command::ReleaseChannel { universe: 0, channel });
    } else if response.double_clicked() {
        engine.set_channel(0, channel, if value == 255 { 0 } else { 255 });
    } else if response.changed() {
        engine.set_channel(0, channel, (level * 255.0).round() as u8);
    }

    let color = if forced {
        egui::Color32::from_rgb(255, 138, 132)
    } else if is_selected {
        theme::SELECTION
    } else {
        theme::MUTED
    };
    ui.label(RichText::new(channel.to_string()).monospace().size(12.0).color(color));
}

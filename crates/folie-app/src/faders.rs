use eframe::egui::{self, Align, Color32, Layout, RichText};
use folie_dmx::{CHANNELS_PER_UNIVERSE, Universe};
use folie_engine::{ChannelOverrides, Command, EngineHandle};

use crate::app::DisplayMode;

pub const FADERS_PER_PAGE: usize = 16;
const PAGES: usize = CHANNELS_PER_UNIVERSE / FADERS_PER_PAGE;
const FADER_WIDTH: f32 = 54.0;
const FADER_LENGTH: f32 = 170.0;

/// Banque de faders, une page de 16 canaux à la fois.
/// Un fader affiche la valeur réellement envoyée ; le bouger force le canal.
pub fn show(
    ui: &mut egui::Ui,
    page: &mut usize,
    overrides: &ChannelOverrides,
    output: &Universe,
    display: DisplayMode,
    selected: Option<u16>,
    engine: &EngineHandle,
) {
    ui.horizontal(|ui| {
        page_selector(ui, page);
        ui.separator();

        ui.spacing_mut().slider_width = FADER_LENGTH;
        let first = *page * FADERS_PER_PAGE + 1;
        for channel in first..first + FADERS_PER_PAGE {
            let channel = channel as u16;
            let size = egui::vec2(FADER_WIDTH, FADER_LENGTH + 44.0);
            ui.allocate_ui_with_layout(size, Layout::top_down(Align::Center), |ui| {
                let forced = overrides[channel as usize - 1].is_some();
                fader(ui, channel, output, forced, display, selected == Some(channel), engine);
            });
        }
    });
}

fn page_selector(ui: &mut egui::Ui, page: &mut usize) {
    ui.vertical(|ui| {
        ui.set_width(90.0);
        ui.label(RichText::new(format!("Page {}", *page + 1)).strong());
        let first = *page * FADERS_PER_PAGE + 1;
        ui.label(format!("canaux {first}–{}", first + FADERS_PER_PAGE - 1));
        ui.horizontal(|ui| {
            if ui.add_enabled(*page > 0, egui::Button::new("‹")).on_hover_text("Page précédente").clicked() {
                *page -= 1;
            }
            if ui.add_enabled(*page + 1 < PAGES, egui::Button::new("›")).on_hover_text("Page suivante").clicked() {
                *page += 1;
            }
        });
        ui.add_space(8.0);
        ui.small("Double-clic :\nplein / zéro\nClic droit :\nrelâcher");
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
    let mut value = output.get(channel).unwrap_or(0);
    ui.monospace(display.format(value));

    let response = ui
        .add(egui::Slider::new(&mut value, 0..=255).vertical().show_value(false))
        .on_hover_text(format!("Canal {channel}"));
    if response.secondary_clicked() {
        engine.send(Command::ReleaseChannel { universe: 0, channel });
    } else if response.double_clicked() {
        value = if value == 255 { 0 } else { 255 };
        engine.set_channel(0, channel, value);
    } else if response.changed() {
        engine.set_channel(0, channel, value);
    }

    let mut label = RichText::new(channel.to_string()).monospace();
    if forced {
        label = label.color(Color32::from_rgb(255, 95, 95));
    }
    if is_selected {
        label = label.strong().underline();
    }
    let response = ui.label(label);
    if forced {
        response.on_hover_text("Canal forcé (clic droit pour relâcher)");
    }
}

mod app;
mod brand;
mod cmdline;
mod color_picker;
mod effects_ui;
mod faders;
mod fixture_sheet;
mod fixtures;
mod monitor;
mod network;
mod playback_ui;
mod pools;
mod screens;
mod storage;
mod stage_ui;
mod theme;
mod tracking_ui;
mod workspace;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let storage = storage::Storage::for_user();
    // La fenêtre principale reprend sa place de la dernière fois.
    let main = storage.workspace().map(|w| w.main).unwrap_or_default();
    let main = screens::rescue(main, &screens::displays(), 0);
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("Folie-DMX")
        .with_inner_size(main.size().unwrap_or(eframe::egui::vec2(1100.0, 760.0)))
        .with_min_inner_size([700.0, 480.0])
        .with_fullscreen(main.fullscreen)
        .with_icon(std::sync::Arc::new(brand::icon(512)));
    if let Some(position) = main.position() {
        viewport = viewport.with_position(position);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Folie-DMX", options, Box::new(|cc| Ok(Box::new(app::FolieApp::new(cc, storage)))))
}

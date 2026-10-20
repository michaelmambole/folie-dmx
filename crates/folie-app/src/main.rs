mod app;
mod faders;
mod fixtures;
mod monitor;
mod storage;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Folie-DMX")
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("Folie-DMX", options, Box::new(|cc| Ok(Box::new(app::FolieApp::new(cc, storage::Storage::for_user())))))
}

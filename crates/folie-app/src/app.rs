use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use folie_dmx::NullOutput;
use folie_dmx::enttec::{self, PortCandidate, UsbPro};
use folie_dmx::CHANNELS_PER_UNIVERSE;
use folie_engine::{Command, EngineConfig, EngineHandle, OutputBinding, Snapshot};
use folie_fixture::library::LoadError;
use folie_fixture::{Library, Patch};

use folie_control::Console;

use crate::cmdline::CommandLine;
use crate::fixtures::FixturesView;
use crate::pools::PoolsView;
use crate::storage::Storage;

mod file;
mod windows;

/// Rafraîchissement de l'interface : largement suffisant pour suivre le moteur à 44 Hz.
const UI_REFRESH: Duration = Duration::from_millis(33);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayMode {
    Percent,
    Dmx,
}

impl DisplayMode {
    pub fn format(self, value: u8) -> String {
        match self {
            Self::Percent => format!("{:.0}", value as f32 / 255.0 * 100.0),
            Self::Dmx => value.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum OutputChoice {
    Virtual,
    Port(String),
}

/// Mesure la fréquence réelle du moteur à partir du compteur de trames.
struct RateMeter {
    since: Instant,
    frame: u64,
    hz: f64,
}

impl RateMeter {
    fn update(&mut self, frame: u64) {
        let elapsed = self.since.elapsed().as_secs_f64();
        if elapsed >= 0.5 {
            self.hz = frame.saturating_sub(self.frame) as f64 / elapsed;
            self.since = Instant::now();
            self.frame = frame;
        }
    }
}

/// Dossier de la bibliothèque : `FOLIE_LIBRARY`, sinon `library/` du dépôt.
fn library_dir() -> PathBuf {
    std::env::var_os("FOLIE_LIBRARY")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library")))
}

/// Pour chaque canal de l'univers 1 : « Fixture N (nom) — paramètre », s'il est patché.
fn channel_labels(patch: &Patch) -> Vec<Option<String>> {
    let mut labels = vec![None; CHANNELS_PER_UNIVERSE];
    for f in patch.fixtures().iter().filter(|f| f.universe == 0) {
        for parameter in &f.mode().parameters {
            for (level, offset) in parameter.offsets.iter().enumerate() {
                let suffix = if level == 0 { "" } else { " (fin)" };
                if let Some(slot) = labels.get_mut(f.address as usize - 1 + *offset as usize) {
                    let number = match parameter.head {
                        Some(head) => format!("{}.{}", f.id, head + 1),
                        None => f.id.to_string(),
                    };
                    *slot = Some(format!("Fixture {number} ({}) — {}{suffix}", f.name, parameter.name));
                }
            }
        }
    }
    labels
}

pub struct FolieApp {
    /// Show, sélection et programmer : tout passe par la console.
    library: Library,
    library_errors: Vec<LoadError>,
    console: Console,
    pools: PoolsView,
    command_line: CommandLine,
    channel_labels: Vec<Option<String>>,
    file: file::FileState,
    windows: windows::WindowsState,
    fixtures_view: FixturesView,
    snapshot: Snapshot,
    ports: Vec<PortCandidate>,
    output: OutputChoice,
    error: Option<String>,
    display: DisplayMode,
    fader_page: usize,
    selected: Option<u16>,
    rate: RateMeter,
}

impl FolieApp {
    pub fn new(cc: &eframe::CreationContext<'_>, storage: Storage) -> Self {
        // Toujours sombre : une console est utilisée en régie, dans le noir.
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        let engine = folie_engine::spawn(
            EngineConfig::default(),
            vec![OutputBinding { universe: 0, output: Box::new(NullOutput) }],
        );
        let (library, library_errors) = Library::load_dir(&library_dir());
        for err in &library_errors {
            log::error!("{} : {}", err.path.display(), err.message);
        }
        let console = Console::new(engine);
        let workspace = storage.workspace();
        let layouts = storage.layouts();
        let mut app = Self {
            snapshot: console.engine().snapshot(),
            console,
            pools: PoolsView::new(),
            command_line: CommandLine::new(),
            library,
            library_errors,
            channel_labels: vec![None; CHANNELS_PER_UNIVERSE],
            file: file::FileState::new(storage),
            windows: windows::WindowsState::new(workspace, layouts),
            fixtures_view: FixturesView::new(),
            ports: Vec::new(),
            output: OutputChoice::Virtual,
            error: None,
            display: DisplayMode::Percent,
            fader_page: 0,
            selected: None,
            rate: RateMeter { since: Instant::now(), frame: 0, hz: 0.0 },
        };
        app.refresh_ports();
        app.startup();
        app
    }

    fn refresh_ports(&mut self) {
        match enttec::find_ports() {
            Ok(ports) => self.ports = ports,
            Err(err) => self.error = Some(format!("Recherche des ports : {err}")),
        }
    }

    fn engine(&self) -> &EngineHandle {
        self.console.engine()
    }

    fn patch_changed(&mut self) {
        self.console.patch_changed();
        self.channel_labels = channel_labels(&self.console.show.patch);
    }

    fn toggle_blackout(&mut self) {
        self.engine().send(Command::Blackout(!self.snapshot.blackout));
    }

    fn select_output(&mut self, choice: OutputChoice) {
        let output: Box<dyn folie_dmx::DmxOutput> = match &choice {
            OutputChoice::Virtual => Box::new(NullOutput),
            OutputChoice::Port(path) => match UsbPro::open(path) {
                Ok(pro) => Box::new(pro),
                Err(err) => {
                    self.error = Some(format!("Ouverture de {path} : {err}"));
                    return;
                }
            },
        };
        self.engine().send(Command::SetOutputs(vec![OutputBinding { universe: 0, output }]));
        self.output = choice;
        self.error = None;
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Folie-DMX").strong());
            self.file_menu(ui);
            self.windows_menu(ui);
            ui.separator();

            let output_label = ui.label("Sortie :");
            let selected_text = match &self.output {
                OutputChoice::Virtual => "Virtuelle".to_string(),
                OutputChoice::Port(path) => path.clone(),
            };
            let mut choice = None;
            egui::ComboBox::from_id_salt("output")
                .selected_text(selected_text)
                .width(280.0)
                .show_ui(ui, |ui| {
                    let virtual_selected = self.output == OutputChoice::Virtual;
                    if ui.selectable_label(virtual_selected, "Virtuelle (aucune interface)").clicked() {
                        choice = Some(OutputChoice::Virtual);
                    }
                    for port in &self.ports {
                        let mut label = port.path.clone();
                        if port.is_ftdi {
                            label = format!("{label}  [FTDI]");
                        }
                        let is_selected = self.output == OutputChoice::Port(port.path.clone());
                        if ui.selectable_label(is_selected, label).clicked() {
                            choice = Some(OutputChoice::Port(port.path.clone()));
                        }
                    }
                })
                .response
                .labelled_by(output_label.id);
            if ui.button("Rechercher").on_hover_text("Rechercher les interfaces branchées").clicked() {
                self.refresh_ports();
            }
            if let Some(choice) = choice.filter(|c| *c != self.output) {
                self.select_output(choice);
            }

            ui.separator();
            let blackout = self.snapshot.blackout;
            let (fill, text) = if blackout {
                (Color32::from_rgb(200, 30, 30), Color32::WHITE)
            } else {
                (ui.visuals().widgets.inactive.weak_bg_fill, ui.visuals().text_color())
            };
            let button = egui::Button::new(RichText::new("BLACKOUT").strong().color(text)).fill(fill);
            if ui.add(button).on_hover_text("Coupe toute la sortie (B)").clicked() {
                self.toggle_blackout();
            }
            let clear_help = "Vide le programmer : fixtures à leurs valeurs par défaut, canaux forcés relâchés";
            if ui.button("Clear").on_hover_text(clear_help).clicked() {
                let result = self.console.execute("ClearAll");
                self.command_line.report(result);
            }

            ui.separator();
            ui.label("Affichage :");
            ui.selectable_value(&mut self.display, DisplayMode::Percent, "%");
            ui.selectable_value(&mut self.display, DisplayMode::Dmx, "DMX");
        });
    }

    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let snap = &self.snapshot;
            let show = format!("{}{}", self.file.show_name(), if self.file.dirty { " •" } else { "" });
            ui.label(RichText::new(show).strong());
            ui.separator();
            ui.monospace(format!("trame {:>8}", snap.frame));
            ui.separator();
            ui.monospace(format!("{:>4.1} Hz", self.rate.hz));
            ui.separator();
            ui.label(format!("Sortie : {}", snap.output_names.join(", ")));
            ui.separator();
            let errors = RichText::new(format!("erreurs {}", snap.output_errors));
            ui.label(if snap.output_errors > 0 { errors.color(Color32::LIGHT_RED) } else { errors });
            ui.label(format!("retards {}", snap.late_frames));
            ui.separator();
            ui.label(format!("{} fixtures patchées", self.console.show.patch.fixtures().len()));
            if !self.library_errors.is_empty() {
                let details: Vec<String> = self
                    .library_errors
                    .iter()
                    .map(|e| format!("{} : {}", e.path.display(), e.message))
                    .collect();
                ui.colored_label(
                    Color32::LIGHT_RED,
                    format!("{} fichier(s) de bibliothèque ignoré(s)", self.library_errors.len()),
                )
                .on_hover_text(details.join("\n"));
            }
            if let Some(err) = &self.error {
                ui.separator();
                ui.colored_label(Color32::LIGHT_RED, err);
            }
        });
    }
}

impl eframe::App for FolieApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.snapshot = self.engine().snapshot();
        self.rate.update(self.snapshot.frame);

        let ctx = ui.ctx().clone();
        self.global_shortcuts(&ctx);
        if ctx.input(|i| i.viewport().close_requested()) {
            self.save_workspace_if_changed(true);
        }
        self.file_tick(&ctx);

        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(4.0);
        });
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        let mut command_changed = false;
        egui::Panel::bottom("command").show(ui, |ui| {
            ui.add_space(4.0);
            command_changed = self.command_line.show(ui, &mut self.console);
            ui.add_space(4.0);
        });
        if command_changed {
            self.mark_dirty();
        }
        egui::CentralPanel::default().show(ui, |ui| {
            self.fixtures_view.attribute_panel(ui, &mut self.console, &self.snapshot, self.display);
        });
        self.show_windows(&ctx);

        self.file_modals(&ctx);
        self.layout_dialog(&ctx);
        ctx.request_repaint_after(UI_REFRESH);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui::accesskit::Role;
    use crate::workspace::{Placement, WindowKind, WindowState, Workspace};
    use folie_fixture::FeatureGroup;
    use egui_kittest::kittest::{By, Queryable};

    /// Fenêtres simulées à droite du Programmer, pour ne pas masquer ses réglages.
    fn test_workspace() -> Workspace {
        let window = |kind, x: f32, y: f32, w: f32, h: f32| WindowState {
            kind,
            placement: Placement::at_size(egui::pos2(x, y), egui::vec2(w, h)),
            preset_family: None,
        };
        Workspace {
            main: Placement::default(),
            windows: vec![
                window(WindowKind::Patch, 1000.0, 40.0, 480.0, 760.0),
                window(WindowKind::Groups, 1520.0, 40.0, 340.0, 360.0),
                window(WindowKind::Presets, 1520.0, 430.0, 340.0, 420.0),
            ],
        }
    }

    fn harness(storage: Storage) -> Harness<'static, FolieApp> {
        harness_with(storage, test_workspace(), egui::vec2(1900.0, 900.0))
    }

    fn harness_with(storage: Storage, workspace: Workspace, size: egui::Vec2) -> Harness<'static, FolieApp> {
        let mut harness = Harness::builder().with_size(size).build_eframe(move |cc| {
            let mut app = FolieApp::new(cc, storage);
            app.windows = windows::WindowsState::new(Some(workspace), Default::default());
            app
        });
        harness.run_steps(2);
        harness
    }

    /// Traite les clics en attente, laisse au moteur le temps d'exécuter les
    /// commandes envoyées, puis rafraîchit l'interface avec son nouvel état.
    fn settle(harness: &mut Harness<'_, FolieApp>) {
        harness.run_steps(2);
        std::thread::sleep(Duration::from_millis(60));
        harness.run_steps(2);
    }

    #[test]
    fn patch_four_mh6_and_program_them() {
        let mut harness = harness(Storage::disabled());

        // Type : la bibliothèque est triée, la MH 2 vient en premier.
        harness.get_by_role_and_label(Role::ComboBox, "Type").click();
        harness.run_steps(2);
        harness.get_by_label("Martin Rush MH 6 Wash").click();
        harness.state_mut().fixtures_view.form.quantity = 4;
        harness.run_steps(2);
        harness.get_by_label("Patcher").click();
        settle(&mut harness);

        let app = harness.state();
        let addresses: Vec<u16> = app.console.show.patch.fixtures().iter().map(|f| f.address).collect();
        assert_eq!(addresses, [1, 15, 29, 43]);
        assert_eq!(app.console.selection().len(), 4);
        assert_eq!(app.snapshot.patch.fixtures().len(), 4, "le moteur a reçu le patch");

        // Valeurs par défaut : shutter ouvert (12), pan/tilt au centre, zoom 128.
        let out = &app.snapshot.output[0];
        assert_eq!(out.get(1), Ok(12));
        assert_eq!(out.get(10), Ok(128));
        assert_eq!(out.get(9), Ok(128));

        harness.get_by_label("Full").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::ComboBox, "Color Wheel").click();
        harness.run_steps(2);
        harness.get_by_label("Color 12 (R0 G255 B0 W0)").click();
        settle(&mut harness);

        let out = &harness.state().snapshot.output[0];
        for base in [0, 14, 28, 42] {
            assert_eq!(out.get(base + 2), Ok(255), "dimmer fixture à {}", base + 1);
            assert_eq!(out.get(base + 3), Ok(255), "dimmer fin");
            assert_eq!(out.get(base + 8), Ok(66), "couleur 12 sur la roue simulée");
        }

        // FOLIE_UI_CAPTURE=<dossier> enregistre une image de l'écran pour vérifier la mise en page.
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("fixtures.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        harness.get_by_label("Clear").click();
        settle(&mut harness);
        let out = &harness.state().snapshot.output[0];
        assert_eq!((out.get(2), out.get(8)), (Ok(0), Ok(0)));
    }

    #[test]
    fn sunstrip_sub_fixtures() {
        let mut harness = harness(Storage::disabled());

        harness.get_by_role_and_label(Role::ComboBox, "Type").click();
        harness.run_steps(2);
        harness.get_by_label("Showtec Sunstrip Active MKII").click();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::ComboBox, "Mode").click();
        harness.run_steps(2);
        harness.get_by_label("Individual (10 canaux)").click();
        let form = &mut harness.state_mut().fixtures_view.form;
        form.quantity = 2;
        form.first_id = 10;
        harness.run_steps(2);
        harness.get_by_label("Patcher").click();
        settle(&mut harness);
        let addresses: Vec<u16> = harness.state().console.show.patch.fixtures().iter().map(|f| f.address).collect();
        assert_eq!(addresses, [1, 11]);

        // Ouvre la fixture 10 et ne sélectionne que sa cellule 3.
        harness.get_all_by_label("+").next().unwrap().click();
        harness.run_steps(2);
        harness.get_by_label("10.3").click();
        settle(&mut harness);
        harness.get_by_label("Full").click();
        settle(&mut harness);
        let out = &harness.state().snapshot.output[0];
        let lamps: Vec<u8> = (1..=10).map(|ch| out.get(ch).unwrap()).collect();
        assert_eq!(lamps, [0, 0, 255, 0, 0, 0, 0, 0, 0, 0]);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("sunstrip.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // La fixture 11 entière : ses 10 lampes d'un coup.
        harness.get_by_label("11").click();
        settle(&mut harness);
        harness.get_by_label("Full").click();
        settle(&mut harness);
        let out = &harness.state().snapshot.output[0];
        assert!((11..=20).all(|ch| out.get(ch) == Ok(255)));
        assert_eq!(out.get(1), Ok(0));
    }


    /// Patche `quantity` fixtures du type donné par le formulaire, mode par défaut.
    fn patch_with_form(harness: &mut Harness<'_, FolieApp>, fixture_type: &str, quantity: u32) {
        harness.get_by_role_and_label(Role::ComboBox, "Type").click();
        harness.run_steps(2);
        harness.get_by_label(fixture_type).click();
        harness.state_mut().fixtures_view.form.quantity = quantity;
        harness.run_steps(2);
        harness.get_by_label("Patcher").click();
        settle(harness);
    }

    #[test]
    fn saved_show_reopens_at_startup() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("donnees");
        let show_path = dir.path().join("concert.folie");

        let mut first = harness(Storage::at(&data));
        patch_with_form(&mut first, "Martin Rush MH 6 Wash", 2);
        assert!(first.state().file.dirty);
        assert!(first.state_mut().save_to(&dir.path().join("concert")), "extension ajoutée");
        assert!(show_path.exists());
        assert!(!first.state().file.dirty);
        drop(first);

        let second = harness(Storage::at(&data));
        let app = second.state();
        assert_eq!(app.file.path.as_deref(), Some(show_path.as_path()));
        assert_eq!(app.console.show.patch.fixtures().len(), 2);
        assert!(!app.file.dirty);
        assert_eq!(app.fixtures_view.form.first_id, 3, "le formulaire propose un numéro libre");
    }

    #[test]
    fn unsaved_changes_are_recovered_after_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("donnees");

        let mut first = harness(Storage::at(&data));
        patch_with_form(&mut first, "Showtec Sunstrip Active MKII", 3);
        first.state_mut().autosave_now();
        drop(first); // fermeture brutale : rien n'a été enregistré

        let mut second = harness(Storage::at(&data));
        assert!(second.state().file.recovery.is_some());
        assert!(second.state().console.show.patch.fixtures().is_empty(), "rien n'est chargé sans accord");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("recovery.png");
            second.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        second.get_by_label("Récupérer").click();
        settle(&mut second);
        let app = second.state();
        assert_eq!(app.console.show.patch.fixtures().len(), 3);
        assert!(app.file.dirty, "les modifications récupérées restent à enregistrer");
        assert_eq!(app.snapshot.patch.fixtures().len(), 3, "le moteur a reçu le patch");
    }

    /// Tape une commande dans la ligne de commande et la valide.
    fn command(harness: &mut Harness<'_, FolieApp>, line: &str) {
        let input = harness.get_by_role_and_label(Role::TextInput, "Commande");
        input.focus();
        input.type_text(line);
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        settle(harness);
    }

    fn output(harness: &Harness<'_, FolieApp>, channel: u16) -> u8 {
        harness.state().snapshot.output[0].get(channel).unwrap()
    }

    #[test]
    fn command_line_groups_and_presets_pools() {
        let mut harness = harness(Storage::disabled());
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 4);
        harness.get_by_label("Aucune").click();
        harness.run_steps(2);

        command(&mut harness, "fi 1 th 2 at 50");
        assert_eq!((output(&harness, 2), output(&harness, 16), output(&harness, 30)), (128, 128, 0));
        assert_eq!(harness.state().command_line.input, "", "la ligne est vidée après exécution");
        harness.get_by_label("Dimmer à 50 % sur 2 cibles");

        // Erreur expliquée, sans rien casser.
        command(&mut harness, "Fixture 99");
        harness.get_by_label("Fixture 99 n'est pas patchée");

        command(&mut harness, r#"Store Group 1 "Face""#);
        assert!(harness.state().file.dirty);
        command(&mut harness, "ClearAll");
        assert!(harness.state().console.selection().is_empty());

        // Rappel du groupe depuis son pool.
        harness.get_by_label("1\nFace").click();
        settle(&mut harness);
        assert_eq!(harness.state().console.selection().len(), 2);

        // Preset de dimmer enregistré avec Store armé, puis rappelé d'un clic.
        harness.get_by_label("Full").click();
        settle(&mut harness);
        harness.get_by_label("1 Dimmer").click();
        harness.get_all_by_label("Store").last().unwrap().click(); // celui de la fenêtre Presets
        harness.run_steps(2);
        harness.get_all_by_label("1").last().unwrap().click(); // case 1 du pool de presets
        settle(&mut harness);
        let preset = &harness.state().console.show.presets[&(FeatureGroup::Dimmer, 1)];
        assert_eq!(preset.values.len(), 2);

        command(&mut harness, "ClearAll");
        assert_eq!(output(&harness, 2), 0);
        harness.get_by_label("1\nDimmer 1").click();
        settle(&mut harness);
        assert_eq!((output(&harness, 2), output(&harness, 16)), (255, 255));
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("pools.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
    }

    #[test]
    fn window_layouts_from_the_menu_and_shortcut() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = harness(Storage::at(dir.path()));

        harness.get_by_label("Fenêtres").click();
        harness.run_steps(2);
        harness.get_by_label("Enregistrer la disposition…").click();
        harness.run_steps(2);
        let name = harness.get_by_role_and_label(Role::TextInput, "Nom");
        name.focus();
        name.type_text("Programmation");
        harness.run_steps(1);
        harness.get_by_label("Enregistrer").click();
        harness.run_steps(2);
        assert_eq!(harness.state().windows.layouts[&1].name, "Programmation");

        // On ferme les groupes depuis le menu…
        harness.get_by_label("Fenêtres").click();
        harness.run_steps(2);
        // L'entrée du menu porte aussi son raccourci : « Groupes ⌘2 ».
        harness.get(By::new().role(Role::Button).label_contains("Groupes")).click();
        harness.run_steps(2);
        assert!(!harness.state().windows.workspace.is_open(WindowKind::Groups));

        // … et ⌥⌘1 les rouvre avec le reste de la disposition.
        harness.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, egui::Key::Num1);
        harness.run_steps(2);
        assert!(harness.state().windows.workspace.is_open(WindowKind::Groups));
        harness.get_by_label("Disposition 1 « Programmation » rappelée");

        // Les dispositions survivent au redémarrage.
        drop(harness);
        let again = harness_restored(Storage::at(dir.path()));
        assert_eq!(again.state().windows.layouts[&1].name, "Programmation");
    }

    /// Comme `harness`, mais en gardant l'espace de travail et les dispositions enregistrés.
    fn harness_restored(storage: Storage) -> Harness<'static, FolieApp> {
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1900.0, 900.0))
            .build_eframe(move |cc| FolieApp::new(cc, storage));
        harness.run_steps(2);
        harness
    }

    #[test]
    fn cues_and_executors_windows() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        let window = |kind, x: f32, y: f32, w: f32, h: f32| WindowState {
            kind,
            placement: Placement::at_size(egui::pos2(x, y), egui::vec2(w, h)),
            preset_family: None,
        };
        workspace.windows.push(window(WindowKind::Cues, 1520.0, 40.0, 560.0, 380.0));
        workspace.windows.push(window(WindowKind::Executors, 1000.0, 860.0, 1100.0, 380.0));
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2200.0, 1300.0));
        harness.get_by_label("Aucune cue. Réglez des valeurs puis tapez « Store Cue », ou :");

        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, r#"Store Cue 1 Fade 0 "Face""#);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 2 At 100");
        command(&mut harness, r#"Store Cue Fade 0 "Contre""#);
        command(&mut harness, "ClearAll");
        assert!(harness.state().file.dirty, "les cues sont à enregistrer dans le show");
        assert_eq!(output(&harness, 2), 0);

        // Go depuis la fenêtre Cues : la cue 1, puis la 2 (la 1 reste suivie).
        harness.get_all_by_label("Go").next().unwrap().click();
        settle(&mut harness);
        assert_eq!((output(&harness, 2), output(&harness, 16)), (255, 0));
        harness.get_by_label("1 Face"); // cue en cours affichée dans les executors
        harness.get_all_by_label("Go").next().unwrap().click();
        settle(&mut harness);
        assert_eq!((output(&harness, 2), output(&harness, 16)), (255, 255));
        harness.get_all_by_label("Back").next().unwrap().click();
        settle(&mut harness);
        assert_eq!(output(&harness, 16), 0);

        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("cues.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
    }
}
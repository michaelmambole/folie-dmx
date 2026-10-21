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
    /// Touche Store armée : le prochain clic sur un pool, un executor ou une cue enregistre.
    store_armed: bool,
    /// Executors dont la touche Flash est maintenue.
    flash_held: std::collections::BTreeSet<u32>,
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
        // Identité visuelle : sombre (régie), polices et couleurs de Folie-DMX.
        crate::theme::apply(&cc.egui_ctx);
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
            store_armed: false,
            flash_held: Default::default(),
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
            self.file_status(ui);
            ui.separator();
            crate::pools::store_button(ui, &mut self.store_armed);
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
            ui.label(RichText::new(self.file.show_name()).strong());
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
    use folie_fixture::{FeatureGroup, FixtureId, Target};
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
        harness_with(storage, test_workspace(), egui::vec2(1900.0, 1200.0))
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
        harness.get_by_label("Supprimer (4)"); // la touche dit combien de fixtures elle retirerait
        harness.get_by_label("56 / 512 canaux · 4 fixtures");

        // Carte de l'univers : un clic sélectionne la fixture, Cmd-clic en ajoute une.
        harness.get_by_label("2 · Rush MH 6 Wash 2 : 1.015–028").click();
        harness.run_steps(2);
        assert_eq!(harness.state().console.selection(), [Target::whole(FixtureId(2))]);
        harness.get_by_label("3 · Rush MH 6 Wash 3 : 1.029–042").click_modifiers(egui::Modifiers::COMMAND);
        harness.run_steps(2);
        assert_eq!(harness.state().console.selection().len(), 2);
        harness.get_by_label("Supprimer (2)");
        harness.get_by_label("Tout sélectionner").click();
        harness.run_steps(2);

        harness.get_by_label("Full").click();
        settle(&mut harness);
        // Option-clic sur la barre du Dimmer : relâché sur les 4, le shutter reste ouvert.
        harness.get_by_role_and_label(Role::Slider, "Dimmer").click_modifiers(egui::Modifiers::ALT);
        settle(&mut harness);
        assert_eq!(harness.state().snapshot.output[0].get(2), Ok(0), "dimmer relâché");
        assert_eq!(harness.state().snapshot.output[0].get(1), Ok(8), "shutter toujours ouvert");
        harness.get_by_label("Full").click();
        settle(&mut harness);
        harness.get_by_role_and_label(Role::ComboBox, "Color Wheel").click();
        harness.run_steps(2);
        harness.get_by_label("Color 5 (R255 G245 B0 W0)").click(); // entièrement visible dans la liste
        settle(&mut harness);

        let out = &harness.state().snapshot.output[0];
        for base in [0, 14, 28, 42] {
            assert_eq!(out.get(base + 2), Ok(255), "dimmer fixture à {}", base + 1);
            assert_eq!(out.get(base + 3), Ok(255), "dimmer fin");
            assert_eq!(out.get(base + 8), Ok(31), "couleur 5 sur la roue simulée (DMX 31 à 35)");
        }

        // FOLIE_UI_CAPTURE=<dossier> enregistre une image de l'écran pour vérifier la mise en page.
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("fixtures.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Les cadres se replient d'un clic sur leur titre, et se déplient de même.
        harness.get_by_role_and_label(Role::DisclosureTriangle, "POSITION").click();
        harness.run_steps(2);
        assert!(harness.query_by_role_and_label(Role::Slider, "Pan").is_none(), "Position repliée");
        harness.get_by_role_and_label(Role::Slider, "Zoom"); // les autres cadres restent ouverts
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("fixtures-collapsed.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_role_and_label(Role::DisclosureTriangle, "POSITION").click();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::Slider, "Pan");

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
        harness.get_all_by_label("Afficher les sous-fixtures").next().unwrap().click();
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
        harness.get_all_by_label("11").next().unwrap().click(); // la ligne du patch, avant les cases de pool
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
        first.run_steps(2);
        first.get_by_label("Enregistré");

        // Une modification : la barre du haut le signale, et un clic enregistre.
        command(&mut first, "Fixture 1 Thru 2");
        command(&mut first, r#"Store Group 1 "Tout""#);
        assert!(first.state().file.dirty);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("unsaved.png");
            first.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        first.get_by_label("Non enregistré").click();
        first.run_steps(2);
        assert!(!first.state().file.dirty, "enregistré dans concert.folie, sans dialogue");
        first.get_by_label("Enregistré");
        drop(first);

        let second = harness(Storage::at(&data));
        let app = second.state();
        assert_eq!(app.file.path.as_deref(), Some(show_path.as_path()));
        assert_eq!(app.console.show.patch.fixtures().len(), 2);
        assert!(!app.file.dirty);
        assert!(app.console.show.groups.contains_key(&1), "le groupe enregistré par le clic");
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

        // Clic droit sur la case : renommer (le nom actuel est déjà dans le champ).
        harness.get_by_label("1\nFace").click_secondary();
        harness.run_steps(2);
        let field = harness.get_by_role_and_label(Role::TextInput, "Nom");
        field.focus();
        field.type_text(" cour");
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert_eq!(harness.state().console.show.groups[&1].name, "Face cour");
        assert!(harness.state().file.dirty);

        // … et supprimer, puis recréer pour la suite du test.
        harness.get_by_label("1\nFace cour").click_secondary();
        harness.run_steps(2);
        harness.get_all_by_label("Supprimer").last().unwrap().click(); // celui du menu, pas du patch
        harness.run_steps(2);
        assert!(harness.state().console.show.groups.is_empty());
        command(&mut harness, r#"Store Group 1 "Face""#);

        // Preset de dimmer enregistré avec Store armé, puis rappelé d'un clic.
        // Le Patch, devant le pool de presets, est fermé (⌘1) pour ne pas recevoir les clics.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1);
        harness.run_steps(2);
        harness.get_by_label("Full").click();
        settle(&mut harness);
        harness.get_by_label("1 Dimmer").click();
        harness.get_all_by_label("Store").last().unwrap().click(); // celui de la fenêtre Presets
        harness.run_steps(2);
        harness.get_all(By::new().role(Role::Button).label("1")).last().unwrap().click(); // case 1 du pool de presets
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

        // Les raccourcis de fenêtre basculent : ⌘2 rouvre les Groupes, ⌘2 les referme.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
        harness.run_steps(2);
        assert!(harness.state().windows.workspace.is_open(WindowKind::Groups));
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num2);
        harness.run_steps(2);
        assert!(!harness.state().windows.workspace.is_open(WindowKind::Groups));

        // En AZERTY, la touche 4 donne « ' » : ⌘4 arrive comme une apostrophe, sur la touche physique 4.
        let azerty_4 = egui::Event::Key {
            key: egui::Key::Quote,
            physical_key: Some(egui::Key::Num4),
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let was_open = harness.state().windows.workspace.is_open(WindowKind::Output);
        harness.event(azerty_4.clone());
        harness.run_steps(2);
        assert_ne!(harness.state().windows.workspace.is_open(WindowKind::Output), was_open, "⌘4 en AZERTY");
        harness.event(azerty_4);
        harness.run_steps(2);
        assert_eq!(harness.state().windows.workspace.is_open(WindowKind::Output), was_open);

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
        workspace.windows.push(window(WindowKind::Executors, 900.0, 820.0, 1350.0, 700.0));
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1600.0));
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

        // Bouton Go de l'executor 1 (le mot « Go » apparaît aussi comme étiquette de mode).
        let go = |h: &Harness<'_, FolieApp>| h.get_all(By::new().role(Role::Button).label("Go")).next().unwrap().click();
        go(&harness);
        settle(&mut harness);
        assert_eq!((output(&harness, 2), output(&harness, 16)), (255, 0));
        harness.get_by_label("1 Face"); // cue en cours affichée dans les executors
        harness.get_by_label("› 2 Contre");
        go(&harness);
        settle(&mut harness);
        assert_eq!((output(&harness, 2), output(&harness, 16)), (255, 255));
        harness.get_all(By::new().role(Role::Button).label("Back")).next().unwrap().click();
        settle(&mut harness);
        assert_eq!(output(&harness, 16), 0);

        // Store armé, puis « Enregistrer ici » sur l'executor 2, encore vide.
        command(&mut harness, "Fixture 2 At 100");
        harness.get_all_by_label("Store").next().unwrap().click(); // celui de la barre du haut
        harness.run_steps(2);
        harness.get_all_by_label("Enregistrer ici").nth(1).unwrap().click();
        settle(&mut harness);
        assert!(!harness.state().store_armed, "Store se désarme après usage");
        assert_eq!(harness.state().console.show.executors[&2].sequence, 2);
        harness.get_by_label("Cue 1 enregistrée sur l'executor 2 (fondu 2 s)");

        // Clic droit sur le numéro de l'executor 1 : options.
        harness.get_all(By::new().role(Role::Button).label("1")).find(|n| n.rect().min.y > 800.0).unwrap().click_secondary();
        harness.run_steps(2);
        harness.get_by_label("Boucler en fin de séquence").click();
        harness.run_steps(2);
        assert!(harness.state().console.show.executors[&1].options.wrap);
        harness.get_by_label("Go · boucle"); // mode et options sur la carte
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("executor-menu.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Fenêtre Cues, sur la séquence de l'executor 1 : cue 1 en cours après le Back.
        harness.key_press(egui::Key::Escape);
        harness.state_mut().console.select_executor(1);
        settle(&mut harness);
        // Les numéros de cue, à distinguer de ceux du patch et des executors.
        let in_cues = |r: egui::Rect| r.min.x > 1520.0 && r.max.y < 420.0;
        let cue_number = |h: &Harness<'_, FolieApp>, label: &str| {
            h.get_all(By::new().role(Role::Button).label(label)).find(|n| in_cues(n.rect())).unwrap().rect()
        };
        harness.get_by_label("EN COURS");
        harness.get_by_label("SUIVANTE");
        assert_eq!(harness.get_all_by_label("1 fixture").count(), 2, "contenu de chaque cue");
        assert!(cue_number(&harness, "2").min.y > cue_number(&harness, "1").min.y);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("cues.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Clic sur un numéro : Goto.
        harness.get_all(By::new().role(Role::Button).label("2")).find(|n| in_cues(n.rect())).unwrap().click();
        settle(&mut harness);
        assert_eq!(output(&harness, 16), 255, "Goto cue 2");

        // Fondu et délai réglés depuis la liste. Ces champs sont nommés par leur valeur :
        // on les retrouve par leur infobulle, qui est leur description.
        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        harness.get(described("Fondu de la cue 2")).set_value_accesskit(1.5);
        settle(&mut harness);
        harness.get(described("Délai de la cue 2")).set_value_accesskit(0.5);
        settle(&mut harness);
        let cue = |h: &Harness<'_, FolieApp>, n: u32| h.state().console.show.sequences[&1].cues[&folie_show::CueNumber::whole(n)].clone();
        assert_eq!((cue(&harness, 2).fade, cue(&harness, 2).delay), (1.5, 0.5));

        // Clic droit sur la ligne : renommer.
        harness.get_by_label("Contre").click_secondary();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::TextInput, "Nom").type_text(" bis");
        harness.run_steps(1);
        harness.get_by_label("Renommer").click();
        settle(&mut harness);
        assert_eq!(cue(&harness, 2).name, "Contre bis");

        // Clic droit, temps par famille : le Dimmer de la cue 2 en 3 s, affiché sur la ligne.
        harness.get_by_label("Contre bis").click_secondary();
        harness.run_steps(2);
        harness.get(described("Fondu de Dimmer, cue 2")).set_value_accesskit(3.0);
        harness.run_steps(2);
        assert_eq!(cue(&harness, 2).family_times[&folie_fixture::FeatureGroup::Dimmer].fade, Some(3.0));
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("cues-family-times.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.get_by_label("Dimmer 3 s +0.5"); // le délai reste celui de la cue

        // Bloquer, puis débloquer, depuis le même menu.
        harness.get_by_label("Contre bis").click_secondary();
        harness.run_steps(2);
        harness.get_by_label("Bloquer").click();
        harness.run_steps(2);
        assert!(cue(&harness, 2).values.contains_key(&folie_fixture::FixtureId(1)), "la cue 2 tient la fixture 1 en dur");
        harness.get_by_label("Débloquer").click();
        harness.run_steps(2);
        assert!(!cue(&harness, 2).values.contains_key(&folie_fixture::FixtureId(1)));
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);

        // Touche S : arme Store, la rappuyer le désarme.
        harness.key_press(egui::Key::S);
        harness.run_steps(2);
        assert!(harness.state().store_armed, "S arme Store");
        harness.key_press(egui::Key::S);
        harness.run_steps(2);
        assert!(!harness.state().store_armed, "S le désarme");

        // Store armé : les numéros de cue complètent la cue avec le programmer.
        // Pendant la saisie d'une commande, S est une lettre comme une autre.
        harness.get_by_role_and_label(Role::TextInput, "Commande").focus();
        harness.run_steps(1);
        harness.key_press(egui::Key::S);
        harness.run_steps(2);
        assert!(!harness.state().store_armed, "S tapé dans la ligne de commande n'arme pas Store");
        command(&mut harness, "Fixture 2 At 50");
        harness.key_press(egui::Key::Escape); // quitte la ligne de commande
        harness.run_steps(2);
        harness.key_press(egui::Key::S);
        harness.run_steps(2);
        harness.get_by_label("Store armé : cliquez un numéro pour compléter cette cue");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("cues-store.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_all(By::new().role(Role::Button).label("1")).find(|n| in_cues(n.rect())).unwrap().click();
        settle(&mut harness);
        assert!(!harness.state().store_armed, "Store se désarme après usage");
        assert_eq!(cue(&harness, 1).values.len(), 2, "la cue 1 tient maintenant les deux fixtures");
        harness.get_by_label("2 fixtures");
        command(&mut harness, "ClearAll");

        // Suppression.
        harness.get(described("Supprimer la cue 2")).click();
        settle(&mut harness);
        assert!(!harness.state().console.show.sequences[&1].cues.contains_key(&folie_show::CueNumber::whole(2)));
        harness.get_by_label("1 cue");
    }

    #[test]
    fn effects_window() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Effects,
            placement: Placement::at_size(egui::pos2(1520.0, 40.0), egui::vec2(620.0, 700.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2200.0, 1000.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 4);

        harness.get_by_label("3 Vague").click();
        settle(&mut harness);
        assert_eq!(harness.state().console.programmer_effects().len(), 1);
        harness.get_by_label("Effet « Vague » sur 4 cible(s)");
        assert_eq!(harness.state().snapshot.running_effects, 1);

        harness.get_by_label("Tap").click();
        harness.run_steps(1);
        harness.get_by_label("Tap").click();
        settle(&mut harness);
        assert!(harness.state().console.tempo() > 120.0, "deux taps rapprochés : tempo rapide");
        assert!(harness.state().file.dirty, "le tempo est enregistré dans le show");

        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("effects.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        harness.get_by_label("Plage 0–100 %"); // plage du Dimmer de la vague, en violet

        harness.get_by_label("Arrêter").click();
        settle(&mut harness);
        assert!(harness.state().console.programmer_effects().is_empty());
        assert_eq!(harness.state().snapshot.running_effects, 0);

        // Le Cercle tourne autour de la position en cours : ses deux lignes sont relatives.
        harness.get_by_label("4 Cercle").click();
        settle(&mut harness);
        assert_eq!(harness.get_all_by_label("Plage -10 à +10 % autour de la valeur").count(), 2);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("effects-relative.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        // Le Pan repasse en absolu : de 0 à 100 %.
        harness.get_all_by_label("Absolu").next().unwrap().click();
        harness.run_steps(2);
        let pan = harness.state().console.programmer_effects()[0].lines[0].clone();
        assert!(!pan.relative && pan.low == 0.0 && pan.high == 100.0, "{pan:?}");
        harness.get_by_label("Plage 0–100 %");

        // Le Programmer montre l'effet de chaque paramètre et permet de l'arrêter.
        harness.get_by_label("Cercle · 0–100 %"); // le Pan, repassé en absolu
        harness.get_by_label("Cercle · ±10 %"); // le Tilt, toujours relatif
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("programmer-effects.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Cercle  ×").click();
        settle(&mut harness);
        assert!(harness.state().console.programmer_effects().is_empty(), "arrêté depuis le Programmer");
        assert!(harness.query_by_label("Cercle · ±10 %").is_none());

        // Chenillard : son carré n'est allumé que 20 % du cycle. Puis le Scintillement, aléatoire.
        harness.get_by_label("2 Chenillard").click();
        settle(&mut harness);
        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        harness.get(described("Part du cycle où le carré est allumé : Dimmer")).set_value_accesskit(20.0);
        harness.run_steps(2);
        assert_eq!(harness.state().console.programmer_effects()[0].lines[0].duty, 20.0);
        harness.get_by_label("7 Scintillement").click();
        settle(&mut harness);
        let sparkle = &harness.state().console.programmer_effects()[1];
        assert_eq!(sparkle.lines[0].waveform, folie_show::Waveform::Random);
        // Fondu de sortie du Chenillard : 2 s pour s'effacer.
        let fade_out = By::new().predicate(|n| n.description().as_deref() == Some("Secondes pour qu'il s'efface à son arrêt, au lieu de couper net"));
        harness.get_all(fade_out).next().unwrap().set_value_accesskit(2.0);
        harness.run_steps(2);
        assert_eq!(harness.state().console.programmer_effects()[0].fade_out, 2.0);

        // Mes effets : le Chenillard réglé, enregistré, renommé, puis relancé d'un clic.
        harness.get_by_label("Vide : « Au pool » sur un effet en cours l'enregistre ici.");
        harness.get_all_by_label("Au pool").next().unwrap().click();
        harness.run_steps(2);
        assert!(harness.state().file.dirty, "Mes effets sont enregistrés dans le show");
        harness.get_by_label("1 Chenillard").click_secondary();
        harness.run_steps(2);
        let name = harness.get_by_role_and_label(Role::TextInput, "Nom");
        name.focus();
        name.type_text(" court");
        harness.run_steps(1);
        harness.get_by_label("Renommer").click();
        harness.run_steps(2);
        let pool = &harness.state().console.show.effect_pool;
        assert_eq!((pool[&1].name.as_str(), pool[&1].lines[0].duty, pool[&1].fade_out), ("Chenillard court", 20.0, 2.0));
        harness.get_by_label("1 Chenillard court").click();
        settle(&mut harness);
        assert_eq!(harness.state().console.programmer_effects().len(), 3);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("effects-pool.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("effects-duty-random.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
    }

    #[test]
    fn executor_pages_and_fixed_executors() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Executors,
            placement: Placement::at_size(egui::pos2(900.0, 820.0), egui::vec2(1350.0, 700.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1600.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, r#"Store Cue 1 Fade 0 "Face""#);
        command(&mut harness, "ClearAll");
        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        let executor_number = |h: &Harness<'_, FolieApp>, label: &str| {
            h.query_all(By::new().role(Role::Button).label(label)).any(|n| n.rect().min.y > 800.0)
        };

        // Page suivante : les places de la page 2 sont vides.
        harness.get(described("Page suivante (Page +)")).click();
        harness.run_steps(2);
        assert_eq!(harness.state().console.page(), 2);
        assert!(executor_number(&harness, "2.1") && executor_number(&harness, "2.20"));
        assert!(!executor_number(&harness, "1"), "l'executor 1 n'est pas sur la page 2");

        // Store sur la place 3 de la page 2.
        command(&mut harness, "Fixture 2 At 100");
        harness.key_press(egui::Key::Escape);
        harness.key_press(egui::Key::S);
        harness.run_steps(2);
        harness.get_all_by_label("Enregistrer ici").nth(2).unwrap().click();
        settle(&mut harness);
        harness.get_by_label("Cue 1 enregistrée sur l'executor 2.3 (fondu 2 s)");
        command(&mut harness, "ClearAll");
        assert!(harness.state().console.show.executors.contains_key(&103));

        // L'executor 1, fixé, reste à sa place sur la page 2.
        command(&mut harness, "Fix Executor 1.1");
        assert!(executor_number(&harness, "1"), "fixé : présent sur la page 2");
        harness.get_by_label("Go · fixe");

        // Nom de la page, enregistré dans le show.
        let name = harness.get_by_role_and_label(Role::TextInput, "Nom de la page");
        name.focus();
        name.type_text("Acte 2");
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        settle(&mut harness);
        assert_eq!(harness.state().console.page_name(2), "Acte 2");
        assert!(harness.state().file.dirty);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("executor-pages.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Retour à la page 1 : l'executor 2.3 n'y est plus, l'executor 1 si.
        harness.get(described("Page précédente (Page -)")).click();
        harness.run_steps(2);
        assert_eq!(harness.state().console.page(), 1);
        assert!(executor_number(&harness, "1") && !executor_number(&harness, "2.3"));
    }

    #[test]
    fn executor_options_menu_chaser_and_release() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Executors,
            placement: Placement::at_size(egui::pos2(900.0, 700.0), egui::vec2(1350.0, 700.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1600.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, r#"Store Cue 1 Fade 0 "Jardin""#);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 2 At 100");
        command(&mut harness, r#"Store Cue Fade 0 "Cour""#);
        command(&mut harness, "ClearAll");

        // Clic droit sur le numéro de l'executor 1 : les deux colonnes d'options.
        harness.get_all(By::new().role(Role::Button).label("1")).find(|n| n.rect().min.y > 700.0).unwrap().click_secondary();
        harness.run_steps(2);
        for label in ["Chenillard (les cues défilent seules)", "Aller-retour", "Ooo : s'éteint quand tout est repris", "Kill", "Swop"] {
            harness.get_all_by_label(label).next().unwrap().click();
            harness.run_steps(2);
        }
        harness.get_all_by_label("Kill").nth(1).unwrap().click(); // celui des protections
        harness.run_steps(2);
        harness.get_by_label("= 0,50 s par pas");
        let x = harness.state().console.show.executors[&1];
        assert_eq!(x.mode, folie_show::ExecutorMode::Swop, "le dernier mode cliqué");
        assert!(x.options.chaser && x.options.ooo);
        assert!(x.options.kill_protect, "« Kill » de la colonne Protections");
        assert_eq!(x.options.chaser_settings.direction, folie_show::ChaserDirection::Bounce);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("executor-options.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.get_by_label("Swop · chenillard 120 · Ooo");

        // Le chenillard tourne : les deux cues défilent.
        let options = folie_show::ExecutorOptions { chaser: true, ..Default::default() };
        harness.state_mut().console.set_executor_options(1, options).unwrap();
        harness.state_mut().console.set_executor_mode(1, folie_show::ExecutorMode::Go).unwrap();
        settle(&mut harness);
        harness.state_mut().console.press(1).unwrap();
        settle(&mut harness);
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..12 {
            settle(&mut harness);
            seen.extend(harness.state().snapshot.executors[&1].cue);
        }
        assert_eq!(seen.len(), 2, "deux pas joués");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("executor-chaser.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Release depuis la fenêtre Programmer.
        command(&mut harness, "Fixture 1");
        harness.get_by_label("Release").click();
        harness.run_steps(2);
        assert!(!harness.state().console.released().is_empty());
    }

    #[test]
    fn tracking_window_shows_origins() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Tracking,
            placement: Placement::at_size(egui::pos2(1400.0, 40.0), egui::vec2(760.0, 560.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2200.0, 1000.0));
        harness.get_by_label("Aucune cue : enregistrez des cues sur cet executor pour suivre leurs valeurs.");
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, r#"Store Cue 1 Fade 0 "Face""#);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 2 At 50");
        command(&mut harness, r#"Store Cue 2 Fade 0 "Contre""#);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 30");
        command(&mut harness, r#"Store Preset 1.1 "Demi""#);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1");
        command(&mut harness, "Preset 1.1");
        command(&mut harness, r#"Store Cue 3 Fade 0 "Demi""#);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Goto Cue 2");
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1); // ferme le Patch, qui la recouvre
        settle(&mut harness);

        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        harness.get_by_label("2 Contre"); // en-tête de colonne
        harness.get_by_label("1 · Rush MH 6 Wash 1");
        // Dimmer de la fixture 1 : enregistré en cue 1, suivi en cue 2, preset en cue 3.
        harness.get(described("Enregistrée dans la cue 1 Face"));
        assert!(harness.query_all(described("Suivie depuis la cue 1 Face")).count() >= 1);
        harness.get(described("Enregistrée dans la cue 3 Demi · preset 1.1 « Demi »"));
        harness.get_by_label("P1.1");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("tracking.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Clic droit sur une valeur enregistrée : temps propres du paramètre dans la cue.
        harness.get(described("Enregistrée dans la cue 1 Face")).click_secondary();
        harness.run_steps(2);
        harness.get(described("Fondu de Dimmer, fixture 1, cue 1")).set_value_accesskit(4.0);
        harness.run_steps(2);
        harness.get(described("Délai de Dimmer, fixture 1, cue 1")).set_value_accesskit(1.0);
        harness.run_steps(2);
        let cue1 = harness.state().console.show.sequences[&1].cues[&folie_show::CueNumber::whole(1)].clone();
        let timing = cue1.param_times[&folie_fixture::FixtureId(1)]["Dimmer"];
        assert_eq!((timing.fade, timing.delay), (Some(4.0), Some(1.0)));
        assert!(harness.state().file.dirty);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("tracking-times.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        // « × » : le paramètre reprend le temps de la cue.
        harness.get_all(described("Reprendre le temps de la cue")).next().unwrap().click(); // celui du fondu
        harness.run_steps(2);
        let cue1 = harness.state().console.show.sequences[&1].cues[&folie_show::CueNumber::whole(1)].clone();
        assert_eq!(cue1.param_times[&folie_fixture::FixtureId(1)]["Dimmer"].fade, None);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);

        // Une valeur suivie, réglée dans la cue 2 : elle y est enregistrée, puis retirée.
        let value_of = |h: &Harness<'_, FolieApp>, cue: u32, fixture: u32| {
            let cue = &h.state().console.show.sequences[&1].cues[&folie_show::CueNumber::whole(cue)];
            cue.values.get(&FixtureId(fixture)).and_then(|p| p.get("Dimmer")).copied()
        };
        harness.get(described("Suivie depuis la cue 1 Face")).click_secondary();
        harness.run_steps(2);
        harness.get_by_label("Changer la valeur l'enregistre dans cette cue.");
        harness.get(described("Valeur de Dimmer, fixture 1, cue 2")).set_value_accesskit(40.0);
        harness.run_steps(2);
        assert_eq!(value_of(&harness, 2, 1), Some(folie_show::CueValue::Value(26_214)), "40 %");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("tracking-value.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Retirer de la cue").click();
        harness.run_steps(2);
        assert_eq!(value_of(&harness, 2, 1), None, "la cue 2 reprend la valeur suivie");
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);

        // Un preset choisi dans le menu, puis un relâchement.
        harness.get(described("Enregistrée dans la cue 1 Face · temps propres 0s +1")).click_secondary();
        harness.run_steps(2);
        harness.get(described("Preset de Dimmer, fixture 1, cue 1")).click();
        harness.run_steps(2);
        harness.get_by_label("1.1 Demi").click();
        harness.run_steps(2);
        assert_eq!(value_of(&harness, 1, 1), Some(folie_show::CueValue::Preset(FeatureGroup::Dimmer, 1)));
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.get(described("Suivie depuis la cue 2 Contre")).click_secondary();
        harness.run_steps(2);
        harness.get_by_label("Relâcher ici").click();
        harness.run_steps(2);
        assert_eq!(value_of(&harness, 3, 2), Some(folie_show::CueValue::Release));
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);

        // Block : la cue 2 écrit en dur ce qu'elle suivait, repéré par un « B ».
        command(&mut harness, "Block Cue 2");
        let blocked = By::new().predicate(|n| n.description().is_some_and(|d| d.starts_with("Bloquée dans la cue 2 Contre")));
        assert_eq!(harness.query_all(blocked).count(), 1, "le Dimmer de la fixture 1, suivi depuis la cue 1");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("tracking-block.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        command(&mut harness, "Unblock Cue 2");
        let blocked = By::new().predicate(|n| n.description().is_some_and(|d| d.starts_with("Bloquée dans la cue 2")));
        assert_eq!(harness.query_all(blocked).count(), 0);
        // La cue 3 répète le preset 1.1 que la cue 1 tient désormais : elle est repérée aussi.
        harness.get(By::new().predicate(|n| n.description().is_some_and(|d| d.starts_with("Bloquée dans la cue 3 Demi"))));

        // Seulement les changements : les valeurs suivies disparaissent.
        harness.get_by_label("Seulement les changements").click();
        harness.run_steps(2);
        assert_eq!(harness.query_all(described("Suivie depuis la cue 1 Face")).count(), 0);
        harness.get(described("Enregistrée dans la cue 1 Face · preset 1.1 « Demi » · temps propres 0s +1"));
    }

    #[test]
    fn dmx_output_window_forces_and_releases_channels() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Output,
            placement: Placement::at_size(egui::pos2(40.0, 820.0), egui::vec2(1100.0, 720.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2000.0, 1600.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 100");
        settle(&mut harness);
        harness.get_by_label("Canaux de l'univers 1");
        harness.get_by_label("virtuelle"); // nom de la sortie donné par le moteur

        // Page 2 des faders (canaux 17 à 32), puis le canal 20 forcé d'un clic au milieu du fader.
        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        harness.get(described("Page suivante")).click();
        harness.run_steps(2);
        harness.get_by_label("canaux 17–32");
        harness.get_by_role_and_label(Role::Slider, "Canal 20").click();
        settle(&mut harness);
        let forced = output(&harness, 20);
        assert!((120..=135).contains(&forced), "canal 20 forcé vers la moitié : {forced}");
        assert!(harness.state().snapshot.channel_overrides[0][19].is_some());
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("output.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        harness.get_by_label("Relâcher les canaux forcés (1)").click();
        settle(&mut harness);
        assert!(harness.state().snapshot.channel_overrides[0][19].is_none(), "canal rendu au moteur");
        assert!(harness.query_by_label("Relâcher les canaux forcés (1)").is_none());
    }

    #[test]
    fn one_effect_stops_from_the_cue_menu() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Cues,
            placement: Placement::at_size(egui::pos2(1520.0, 40.0), egui::vec2(620.0, 480.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1000.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 Thru 2 At 50");
        command(&mut harness, "Effect 3");
        command(&mut harness, "Effect 4");
        command(&mut harness, r#"Store Cue 1 Fade 0 "Ouverture""#);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 80");
        command(&mut harness, r#"Store Cue 2 Fade 0 "Suite""#);
        command(&mut harness, "ClearAll");
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1); // ferme le Patch
        harness.run_steps(2);

        harness.get_by_label("Suite").click_secondary();
        harness.run_steps(2);
        assert_eq!(harness.get_all_by_label("suivi depuis la cue 1").count(), 2, "Vague et Cercle");
        harness.get_all_by_label("Arrêter ici").nth(1).unwrap().click(); // le Cercle
        harness.run_steps(2);
        let stopped = |h: &Harness<'_, FolieApp>| {
            h.state().console.show.sequences[&1].cues[&folie_show::CueNumber::whole(2)].stopped_effects.clone()
        };
        assert_eq!(stopped(&harness).into_iter().collect::<Vec<_>>(), [(folie_show::CueNumber::whole(1), 1)]);
        harness.get_by_label("arrêté ici");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("cue-effects.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Reprendre").click();
        harness.run_steps(2);
        assert!(stopped(&harness).is_empty());
    }

    #[test]
    fn transition_bar_shows_delay_without_moving_the_layout() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Executors,
            placement: Placement::at_size(egui::pos2(900.0, 820.0), egui::vec2(1000.0, 700.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2000.0, 1600.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 1);
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, "Store Cue 1 Fade 3");
        command(&mut harness, "ClearAll");
        harness.state_mut().console.set_cue_times(folie_show::CueNumber::whole(1), 3.0, 2.0).unwrap();
        settle(&mut harness);

        let go = |h: &Harness<'_, FolieApp>| h.get_all(By::new().role(Role::Button).label("Go")).next().unwrap().rect();
        let before = go(&harness);
        harness.get_all(By::new().role(Role::Button).label("Go")).next().unwrap().click();
        settle(&mut harness);
        let status = harness.state().snapshot.executors[&1].clone();
        assert!(status.in_transition() && status.elapsed < status.delay, "encore dans le délai : {status:?}");
        assert_eq!(go(&harness), before, "la carte ne bouge pas au Go");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("delay.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
    }

    #[test]
    fn presets_pool_shows_colors_and_states() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Presets,
            placement: Placement::at_size(egui::pos2(1500.0, 40.0), egui::vec2(700.0, 480.0)),
            preset_family: Some("color".into()),
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1000.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        let console = &mut harness.state_mut().console;
        let parameter = |console: &Console, name: &str| {
            let fixture = console.show.patch.fixtures()[0].clone();
            fixture.mode().parameters.iter().find(|p| p.name == name).unwrap().clone()
        };
        let store = |console: &mut Console, line: &str, color: &str, number: u32, name: &str| {
            console.execute(line).unwrap();
            let p = parameter(console, color);
            console.set_like(&p, u16::MAX);
            console.store_preset(FeatureGroup::Color, number, Some(name.into())).unwrap();
            console.execute("ClearAll").unwrap();
        };
        store(console, "Fixture 1 Thru 2", "Red", 1, "Rouge");
        store(console, "Fixture 1", "Blue", 2, "Bleu");
        store(console, "Fixture 1 Thru 2", "Green", 3, "Vert");
        // « Bleu » joué par l'executor 1, « Vert » dans le programmer, fixture 1 sélectionnée.
        console.execute("Fixture 1").unwrap();
        console.execute("Preset 4.2").unwrap();
        console.execute("Store Cue 1 Fade 0").unwrap();
        console.execute("ClearAll").unwrap();
        console.execute("Go").unwrap();
        console.execute("Fixture 2").unwrap();
        console.execute("Preset 4.3").unwrap();
        settle(&mut harness);
        harness.get_by_label("1\nRouge");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("presets-pool.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
    }

    #[test]
    fn color_picker_sets_the_selection() {
        let mut harness = harness(Storage::disabled());
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        harness.get_by_label("Ambre").click();
        settle(&mut harness);
        let out = |h: &Harness<'_, FolieApp>, ch| h.state().snapshot.output[0].get(ch).unwrap();
        // MH 6 : Red 4, Green 5, Blue 6, White 7 (et 18 à 21 pour la deuxième).
        assert_eq!((out(&harness, 4), out(&harness, 5), out(&harness, 6), out(&harness, 7)), (255, 128, 0, 0));
        assert_eq!(out(&harness, 18), 255, "toute la sélection");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("color-picker.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
    }
}
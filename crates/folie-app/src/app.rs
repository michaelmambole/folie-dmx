use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText};
use folie_dmx::NullOutput;
use folie_dmx::artnet::ArtNet;
use folie_dmx::enttec::{self, PortCandidate, UsbPro};
use folie_dmx::CHANNELS_PER_UNIVERSE;
use folie_engine::{Command, EngineConfig, EngineHandle, OutputBinding, Snapshot};
use folie_fixture::library::LoadError;
use folie_fixture::{Library, Patch};

use folie_control::Console;

use crate::cmdline::CommandLine;
use crate::fixtures::FixturesView;
use crate::pools::PoolsView;
use crate::storage::{ArtNetSettings, OscSettings, RemoteSettings, SacnSettings, SoundSettings, Storage};
use crate::theme;

mod about;
mod file;
mod remote;
mod sound;
mod welcome;
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

/// Sortie choisie au lancement : la première interface FTDI détectée (très probablement
/// une Enttec), sinon la sortie virtuelle.
fn startup_output(ports: &[PortCandidate]) -> OutputChoice {
    ports.iter().find(|p| p.is_ftdi).map_or(OutputChoice::Virtual, |p| OutputChoice::Port(p.path.clone()))
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

/// Dossier de la bibliothèque : `FOLIE_LIBRARY`, sinon celui de la version distribuée
/// (`library/` à côté de l'exécutable sous Windows, `Contents/Resources/library` dans l'app
/// macOS), sinon `library/` du dépôt.
fn library_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("FOLIE_LIBRARY") {
        return PathBuf::from(dir);
    }
    let beside_exe = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from));
    beside_exe
        .into_iter()
        .flat_map(|dir| [dir.join("library"), dir.join("../Resources/library")])
        .find(|dir| dir.is_dir())
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
    fixture_sheet: crate::fixture_sheet::FixtureSheet,
    snapshot: Snapshot,
    ports: Vec<PortCandidate>,
    output: OutputChoice,
    /// Sortie Art-Net, envoyée en plus de `output`.
    pub(crate) artnet: ArtNetSettings,
    /// Sortie sACN, envoyée en plus de `output`.
    pub(crate) sacn: SacnSettings,
    pub(crate) remote_settings: RemoteSettings,
    /// Serveur de la télécommande web, quand elle est active.
    pub(crate) remote: Option<folie_remote::RemoteServer>,
    pub(crate) osc_settings: OscSettings,
    /// Serveur OSC, quand il est actif.
    pub(crate) osc: Option<folie_remote::osc::OscServer>,
    /// Adresses de cet ordinateur, montrées pour la télécommande et l'OSC.
    addresses: crate::network::AddressCache,
    /// Fenêtre « À propos » ouverte.
    about_open: bool,
    pub(crate) sound_settings: SoundSettings,
    /// Entrée audio en marche, et les réactions qu'elle applique.
    pub(crate) audio: Option<folie_audio::input::AudioInput>,
    audio_setup: folie_show::sound::SoundSetup,
    audio_devices: Vec<folie_audio::input::InputDevice>,
    /// Dernier total d'erreurs de sortie, et quand il a augmenté.
    output_errors: (u64, Option<Instant>),
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
        let artnet = storage.artnet();
        let sacn = storage.sacn();
        let remote_settings = storage.remote();
        let osc_settings = storage.osc();
        let sound_settings = storage.sound();
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
            fixture_sheet: Default::default(),
            ports: Vec::new(),
            output: OutputChoice::Virtual,
            artnet,
            sacn,
            remote_settings,
            remote: None,
            osc_settings,
            osc: None,
            addresses: Default::default(),
            about_open: false,
            sound_settings,
            audio: None,
            audio_setup: Default::default(),
            audio_devices: Vec::new(),
            output_errors: (0, None),
            error: None,
            display: DisplayMode::Percent,
            fader_page: 0,
            selected: None,
            rate: RateMeter { since: Instant::now(), frame: 0, hz: 0.0 },
        };
        app.refresh_ports();
        // Les tests ne doivent jamais ouvrir l'interface réelle branchée sur la machine.
        let choice = if cfg!(test) { OutputChoice::Virtual } else { startup_output(&app.ports) };
        if choice != OutputChoice::Virtual || app.artnet.enabled || app.sacn.enabled {
            app.apply_outputs(choice);
        }
        if app.remote_settings.enabled {
            app.remote_changed(&cc.egui_ctx);
        }
        if app.osc_settings.enabled {
            app.osc_changed(&cc.egui_ctx);
        }
        if app.sound_settings.enabled {
            app.sound_changed();
        }
        #[cfg(all(target_os = "macos", not(test)))]
        file::route_quit_to_main_window(cc);
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
        self.apply_outputs(choice);
    }

    /// Sorties du moteur : la sortie choisie, plus l'Art-Net s'il est actif. Une adresse
    /// Art-Net illisible désactive l'Art-Net et le dit, sans toucher à l'autre sortie.
    fn apply_outputs(&mut self, choice: OutputChoice) {
        let primary: Box<dyn folie_dmx::DmxOutput> = match &choice {
            OutputChoice::Virtual => Box::new(NullOutput),
            OutputChoice::Port(path) => match UsbPro::open(path) {
                Ok(pro) => Box::new(pro),
                Err(err) => {
                    self.error = Some(format!("Ouverture de {path} : {err}"));
                    return;
                }
            },
        };
        let mut outputs = vec![OutputBinding { universe: 0, output: primary }];
        self.error = None;
        if self.artnet.enabled {
            let opened = self
                .artnet
                .destination
                .trim()
                .parse::<std::net::IpAddr>()
                .map_err(|_| format!("Art-Net : adresse « {} » illisible", self.artnet.destination))
                .and_then(|ip| ArtNet::open(ip, self.artnet.universe).map_err(|err| format!("Art-Net : {err}")));
            match opened {
                Ok(artnet) => outputs.push(OutputBinding { universe: 0, output: Box::new(artnet) }),
                Err(err) => {
                    self.artnet.enabled = false;
                    self.error = Some(err);
                }
            }
        }
        if self.sacn.enabled {
            let destination = self.sacn.destination.trim();
            let target = if destination.is_empty() {
                Ok(None)
            } else {
                destination.parse::<std::net::IpAddr>().map(Some).map_err(|_| format!("sACN : adresse « {destination} » illisible"))
            };
            let opened = target.and_then(|ip| {
                folie_dmx::sacn::Sacn::open(self.sacn.universe, ip, self.sacn.priority).map_err(|err| format!("sACN : {err}"))
            });
            match opened {
                Ok(sacn) => outputs.push(OutputBinding { universe: 0, output: Box::new(sacn) }),
                Err(err) => {
                    self.sacn.enabled = false;
                    self.error = Some(err);
                }
            }
        }
        self.engine().send(Command::SetOutputs(outputs));
        self.output = choice;
        self.file.storage.set_artnet(&self.artnet);
        self.file.storage.set_sacn(&self.sacn);
    }

    /// Les réglages Art-Net ou sACN ont changé : on reconstruit les sorties.
    pub(crate) fn artnet_changed(&mut self) {
        let choice = self.output.clone();
        self.apply_outputs(choice);
    }

    /// Barre du haut en trois zones : le show et les menus à gauche, les touches de jeu au
    /// centre, les états (sortie, Art-Net, télécommande) et l'affichage à droite.
    fn top_bar(&mut self, ui: &mut egui::Ui) {
        const HEIGHT: f32 = 32.0;
        // Trois touches de 78 points et leurs écarts.
        let keys_width = 3.0 * 78.0 + 2.0 * ui.spacing().item_spacing.x;
        let row = egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), HEIGHT));
        let zone = |rect: egui::Rect, layout: egui::Layout| egui::UiBuilder::new().max_rect(rect).layout(layout);
        let centered = egui::Align::Center;
        ui.scope_builder(zone(row, egui::Layout::left_to_right(centered)), |ui| {
            let logo = crate::brand::mark_widget(ui, 24.0).interact(egui::Sense::click());
            if logo.on_hover_text("À propos de Folie-DMX").clicked() {
                self.about_open = true;
            }
            self.file_status(ui);
            ui.add_space(4.0);
            self.file_menu(ui);
            self.windows_menu(ui);
        });
        let keys = egui::Rect::from_center_size(row.center(), egui::vec2(keys_width, HEIGHT));
        ui.scope_builder(zone(keys, egui::Layout::left_to_right(centered)), |ui| self.play_keys(ui));
        ui.scope_builder(zone(row, egui::Layout::right_to_left(centered)), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            self.display_switch(ui);
            self.remote_button(ui);
            // L'Art-Net part en plus de la sortie choisie ; il se règle dans la fenêtre Sortie DMX.
            if self.artnet.enabled {
                let help = format!("Art-Net vers {}, univers {} : réglages dans la fenêtre Sortie DMX", self.artnet.destination, self.artnet.universe);
                if theme::status_chip(ui, "Art-Net", Some(theme::GO)).on_hover_text(help).clicked() {
                    self.windows.open(crate::workspace::WindowKind::Output);
                }
            }
            if self.sacn.enabled {
                let to = if self.sacn.destination.trim().is_empty() { "multicast".to_string() } else { self.sacn.destination.clone() };
                let help = format!("sACN univers {} ({to}) : réglages dans la fenêtre Sortie DMX", self.sacn.universe);
                if theme::status_chip(ui, "sACN", Some(theme::GO)).on_hover_text(help).clicked() {
                    self.windows.open(crate::workspace::WindowKind::Output);
                }
            }
            self.output_chip(ui);
        });
    }

    /// STORE, CLEAR, BLIND et BLACKOUT : les touches qu'on joue, au centre.
    fn play_keys(&mut self, ui: &mut egui::Ui) {
        crate::pools::store_key(ui, &mut self.store_armed);
        let clear_help = "Vide le programmer : fixtures à leurs valeurs par défaut, canaux forcés relâchés";
        if ui.add(theme::key_button("CLEAR", theme::LINE, theme::SURFACE_2, theme::TEXT)).on_hover_text(clear_help).clicked() {
            let result = self.console.execute("ClearAll");
            self.command_line.report(result);
        }
        let blind = self.console.blind();
        let button = if blind {
            theme::key_button("BLIND", theme::INFO, theme::INFO, Color32::WHITE)
        } else {
            theme::key_button("BLIND", theme::INFO, theme::INFO.gamma_multiply(0.14), Color32::from_rgb(140, 185, 245))
        };
        let help = if blind {
            "Blind actif : le programmer ne sort pas. Cliquer pour revenir au programmer live (commande Blind)"
        } else {
            "Programmer à part, qui ne sort pas : préparer sans toucher à la scène (commande Blind)"
        };
        if ui.add(button).on_hover_text(help).clicked() {
            let result = self.console.execute("Blind");
            self.command_line.report(result);
        }
        let blackout = self.snapshot.blackout;
        let button = if blackout {
            theme::key_button("BLACKOUT", theme::DANGER, theme::DANGER, Color32::WHITE)
        } else {
            theme::key_button("BLACKOUT", theme::DANGER, theme::DANGER.gamma_multiply(0.18), Color32::from_rgb(255, 138, 132))
        };
        let help = if blackout { "Sortie coupée : cliquer pour rallumer (B)" } else { "Coupe toute la sortie (B)" };
        if ui.add(button).on_hover_text(help).clicked() {
            self.toggle_blackout();
        }
    }

    /// Sortie choisie, en pastille : un clic ouvre la liste des interfaces.
    fn output_chip(&mut self, ui: &mut egui::Ui) {
        let unplugged = !self.snapshot.output_offline.is_empty();
        let (label, help) = match &self.output {
            OutputChoice::Virtual => ("Virtuelle".to_string(), "Sortie virtuelle : aucune interface DMX".to_string()),
            OutputChoice::Port(path) if unplugged => {
                ("USB Pro débranchée".to_string(), format!("Enttec USB Pro sur {path} : reconnexion automatique dès son retour"))
            }
            OutputChoice::Port(path) => ("USB Pro".to_string(), format!("Enttec USB Pro sur {path}")),
        };
        let state = match &self.output {
            _ if unplugged => Some(theme::DANGER),
            OutputChoice::Virtual => None,
            OutputChoice::Port(_) => Some(theme::GO),
        };
        let response = theme::status_chip(ui, &label, state).on_hover_text(format!("{help} ; clic : choisir la sortie"));
        let mut choice = None;
        egui::Popup::menu(&response).show(|ui| {
            theme::section(ui, "Sortie DMX");
            if ui.selectable_label(self.output == OutputChoice::Virtual, "Virtuelle (aucune interface)").clicked() {
                choice = Some(OutputChoice::Virtual);
            }
            for port in &self.ports {
                let text = if port.is_ftdi { format!("{}  [FTDI]", port.path) } else { port.path.clone() };
                if ui.selectable_label(self.output == OutputChoice::Port(port.path.clone()), text).clicked() {
                    choice = Some(OutputChoice::Port(port.path.clone()));
                }
            }
            ui.separator();
            if ui.button("Rechercher les interfaces").clicked() {
                self.refresh_ports();
            }
        });
        if let Some(choice) = choice.filter(|c| *c != self.output) {
            self.select_output(choice);
        }
    }

    /// Valeurs en % ou en DMX.
    fn display_switch(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            // Dans une disposition de droite à gauche : DMX d'abord.
            ui.selectable_value(&mut self.display, DisplayMode::Dmx, RichText::new("DMX").monospace().size(12.0))
                .on_hover_text("Valeurs en DMX (0 à 255)");
            ui.selectable_value(&mut self.display, DisplayMode::Percent, RichText::new("%").monospace().size(12.0))
                .on_hover_text("Valeurs en pourcentage");
        });
    }

    /// Barre d'état : un message d'erreur s'il y en a un, l'essentiel à droite.
    fn status_bar(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for name in &self.snapshot.output_offline {
                let text = format!("{name} débranchée : reconnexion automatique dès qu'elle revient");
                ui.label(RichText::new(text).size(12.0).color(theme::DANGER));
            }
            if let Some(err) = &self.error {
                ui.label(RichText::new(err).size(12.0).color(theme::DANGER));
            }
            if !self.library_errors.is_empty() {
                let details: Vec<String> =
                    self.library_errors.iter().map(|e| format!("{} : {}", e.path.display(), e.message)).collect();
                let text = format!("{} fichier(s) de bibliothèque ignoré(s)", self.library_errors.len());
                ui.label(RichText::new(text).size(12.0).color(theme::DANGER)).on_hover_text(details.join("\n"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                let snap = &self.snapshot;
                let small = |text: String, color| RichText::new(text).monospace().size(11.0).color(color);
                let output = if !snap.output_offline.is_empty() {
                    small("sortie débranchée".into(), theme::DANGER)
                } else if self.output_errors.1.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
                    small(format!("{} erreur(s) de sortie", snap.output_errors), theme::DANGER)
                } else {
                    small("sortie OK".into(), theme::GO)
                };
                let help = format!("Sortie : {} ; {} erreur(s) depuis le lancement", snap.output_names.join(", "), snap.output_errors);
                ui.label(output).on_hover_text(help);
                if snap.late_frames > 0 {
                    ui.label(small(format!("{} trame(s) en retard", snap.late_frames), theme::EXECUTOR))
                        .on_hover_text("Trames émises après leur échéance : l'ordinateur est très chargé");
                }
                ui.label(small(format!("{:.0} Hz", self.rate.hz), theme::MUTED)).on_hover_text(format!("Trame {}", snap.frame));
                let fixtures = self.console.show.patch.fixtures().len();
                let plural = if fixtures > 1 { "s" } else { "" };
                ui.label(small(format!("{fixtures} fixture{plural} patchée{plural}"), theme::MUTED));
            });
        });
    }
}

impl eframe::App for FolieApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let was_offline = std::mem::take(&mut self.snapshot.output_offline);
        self.snapshot = self.engine().snapshot();
        if self.snapshot.output_errors > self.output_errors.0 {
            self.output_errors = (self.snapshot.output_errors, Some(Instant::now()));
        }
        // Une interface revenue : on le dit sur la ligne de commande.
        for name in was_offline.iter().filter(|n| !self.snapshot.output_offline.contains(n)) {
            self.command_line.feedback = Some((true, format!("{name} reconnectée")));
        }
        self.rate.update(self.snapshot.frame);
        // Les macros en cours avancent à chaque image (leurs « Wait » comptent ici).
        for result in self.console.tick_macros(Instant::now()) {
            if self.command_line.report(result) {
                self.mark_dirty();
            }
        }
        // Commandes des cues qui viennent de partir (OSC vers Reaper…).
        for result in self.console.tick_cue_commands(&self.snapshot) {
            if self.command_line.report(result) {
                self.mark_dirty();
            }
        }

        let ctx = ui.ctx().clone();
        self.remote_tick();
        self.sound_tick();
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
            if self.console.selection().is_empty() {
                self.welcome(ui);
            } else {
                self.fixtures_view.attribute_panel(ui, &mut self.console, &self.snapshot, self.display);
            }
        });
        self.show_windows(&ctx);

        self.file_modals(&ctx);
        self.layout_dialog(&ctx);
        self.about_dialog(&ctx);
        ctx.request_repaint_after(UI_REFRESH);
    }

    fn on_exit(&mut self) {
        self.file_exit();
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
                window(WindowKind::Patch, 1000.0, 40.0, 500.0, 560.0),
                window(WindowKind::Fixtures, 1000.0, 650.0, 500.0, 500.0),
                window(WindowKind::Groups, 1520.0, 40.0, 340.0, 360.0),
                window(WindowKind::Presets, 1520.0, 430.0, 340.0, 420.0),
            ],
        }
    }

    #[test]
    fn startup_picks_the_first_ftdi_interface() {
        let port = |path: &str, is_ftdi| PortCandidate { path: path.into(), is_ftdi, description: None, serial_number: None };
        assert_eq!(startup_output(&[]), OutputChoice::Virtual);
        assert_eq!(startup_output(&[port("/dev/cu.Bluetooth", false)]), OutputChoice::Virtual, "pas une interface DMX");
        assert_eq!(
            startup_output(&[port("/dev/cu.Bluetooth", false), port("/dev/cu.usbserial-EN1", true)]),
            OutputChoice::Port("/dev/cu.usbserial-EN1".into())
        );
    }

    fn harness(storage: Storage) -> Harness<'static, FolieApp> {
        harness_with(storage, test_workspace(), egui::vec2(1960.0, 1200.0))
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

        // R : Relâcher, le programmer lâche la sélection ; Maj-R : Release, pour Store.
        assert!(!harness.state().console.programmer().is_empty());
        harness.key_press(egui::Key::R);
        harness.run_steps(2);
        assert!(harness.state().console.programmer().is_empty(), "R relâche la sélection");
        harness.get_by_label("4 cible(s) relâchée(s) : le programmer ne les tient plus");
        harness.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::R);
        harness.run_steps(2);
        assert!(!harness.state().console.released().is_empty(), "Maj-R marque la sélection relâchée");
        // Dans la ligne de commande (qui garde la main après Entrée), R reste une lettre.
        command(&mut harness, "Fixture 1 At 50");
        harness.key_press(egui::Key::R);
        harness.run_steps(2);
        assert!(!harness.state().console.programmer().is_empty(), "R tapé dans la commande ne relâche rien");
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);

        harness.get_by_label("CLEAR").click();
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


    /// Une ligne de la fenêtre Fixtures, par son numéro (« 2 », « 10.3 »).
    fn sheet_row<'h>(harness: &'h Harness<'_, FolieApp>, number: &'h str) -> egui_kittest::Node<'h> {
        harness.get(By::new().role(Role::Row).label(number))
    }

    fn selection(harness: &Harness<'_, FolieApp>) -> Vec<String> {
        harness.state().console.selection().iter().map(|t| t.to_string()).collect()
    }

    /// Patche des Sunstrip en mode Individual (10 cellules) à partir du numéro donné.
    fn patch_sunstrips(harness: &mut Harness<'_, FolieApp>, quantity: u32, first_id: u32) {
        harness.get_by_role_and_label(Role::ComboBox, "Type").click();
        harness.run_steps(2);
        harness.get_by_label("Showtec Sunstrip Active MKII").click();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::ComboBox, "Mode").click();
        harness.run_steps(2);
        harness.get_by_label("Individual (10 canaux)").click();
        let form = &mut harness.state_mut().fixtures_view.form;
        form.quantity = quantity;
        form.first_id = first_id;
        harness.run_steps(2);
        harness.get_by_label("Patcher").click();
        settle(harness);
    }

    #[test]
    fn fixture_sheet_shows_live_values_and_selects() {
        use crate::fixture_sheet::{animated, live_row};
        use crate::theme::Source;
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Fixtures,
            placement: Placement::at_size(egui::pos2(1000.0, 620.0), egui::vec2(900.0, 560.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(1960.0, 1200.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 4);
        patch_sunstrips(&mut harness, 1, 10);
        command(&mut harness, "ClearAll");

        // 1 : tenue par l'executor ; 2 : dans le programmer ; 3 : un cercle ; 4 : strobe.
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, "Store Cue 1 Fade 0");
        command(&mut harness, "ClearAll");
        command(&mut harness, "Go Executor 1");
        command(&mut harness, "Fixture 2 At 50");
        command(&mut harness, "Fixture 3");
        command(&mut harness, "Effect 4");
        command(&mut harness, "Fixture 4");
        {
            let console = &mut harness.state_mut().console;
            let fixture = console.show.patch.get(FixtureId(4)).unwrap().clone();
            let shutter = fixture.mode().parameters.iter().find(|p| p.attribute == folie_fixture::Attribute::Shutter).unwrap().clone();
            let strobe = shutter
                .capabilities
                .iter()
                .find(|c| matches!(&c.kind, folie_fixture::CapabilityKind::Shutter { effect, .. } if effect == "Strobe"))
                .expect("plage de strobe")
                .select_value();
            console.set_like(&shutter, strobe);
        }
        command(&mut harness, "Fixture 10.3 At 100");
        settle(&mut harness);

        let app = harness.state();
        let effects = animated(&app.console, &app.snapshot);
        let row = |target| live_row(&app.console, &app.snapshot, target, &effects, DisplayMode::Percent);
        let one = row(Target::whole(FixtureId(1)));
        let intensity = one.intensity.clone().unwrap();
        assert_eq!((intensity.value, intensity.source), (1.0, Source::Executor));
        assert_eq!(one.color.as_ref().map(|c| (c.value, c.source)), Some(([255, 255, 255], Source::Default)), "blanc par défaut");
        assert_eq!(one.pan.as_ref().map(|p| (p.value.as_str(), p.source)), Some(("0°", Source::Default)));
        assert!(one.zoom.is_some() && one.strobe.is_none() && one.effects.is_empty());
        let two = row(Target::whole(FixtureId(2))).intensity.unwrap();
        assert!((two.value - 0.5).abs() < 0.01 && two.source == Source::Programmer, "{two:?}");
        let three = row(Target::whole(FixtureId(3)));
        assert_eq!((three.pan.unwrap().source, three.tilt.unwrap().source), (Source::Effect, Source::Effect));
        assert_eq!(three.effects.len(), 1, "le nom du cercle");
        assert_eq!(row(Target::whole(FixtureId(4))).strobe, Some(Source::Programmer));
        // Sunstrip : la fixture entière montre sa lampe la plus haute, la cellule la sienne, sans couleur.
        let strip = row(Target::whole(FixtureId(10)));
        assert_eq!(strip.intensity.map(|i| (i.value, i.source)), Some((1.0, Source::Programmer)));
        assert!(strip.color.is_none() && strip.pan.is_none());
        let cell = |head| row(Target { fixture: FixtureId(10), head: Some(head) }).intensity.unwrap().value;
        assert_eq!((cell(2), cell(3)), (1.0, 0.0));

        // Sélection : clic, Maj-clic (plage), Cmd-clic (retire).
        harness.get_by_label("Aucune").click();
        harness.run_steps(2);
        sheet_row(&harness, "2").click();
        harness.run_steps(2);
        assert_eq!(selection(&harness), ["2"]);
        sheet_row(&harness, "4").click_modifiers(egui::Modifiers::SHIFT);
        harness.run_steps(2);
        assert_eq!(selection(&harness), ["2", "3", "4"]);
        sheet_row(&harness, "3").click_modifiers(egui::Modifiers::COMMAND);
        harness.run_steps(2);
        assert_eq!(selection(&harness), ["2", "4"]);

        // Glisser de la 3 vers la 1 : sélection dans le sens du geste.
        let (from, to) = (sheet_row(&harness, "3").rect().center(), sheet_row(&harness, "1").rect().center());
        harness.hover_at(from);
        harness.run_steps(1);
        harness.drag_at(from);
        harness.run_steps(1);
        for step in 1..=4 {
            harness.hover_at(from + (to - from) * step as f32 / 4.0);
            harness.run_steps(1);
        }
        assert_eq!(selection(&harness), ["3", "2", "1"]);
        harness.drop_at(to);
        harness.run_steps(2);
        assert_eq!(selection(&harness), ["3", "2", "1"], "le relâcher garde la sélection");

        // Sous-fixtures dépliées, puis une seule cellule sélectionnée.
        harness.get_by_label("Afficher les sous-fixtures").click();
        harness.run_steps(2);
        sheet_row(&harness, "10.3").click();
        harness.run_steps(2);
        assert_eq!(selection(&harness), ["10.3"]);
        harness.get(By::new().role(Role::Row).label("10.10"));
        sheet_row(&harness, "1").click_modifiers(egui::Modifiers::COMMAND);
        sheet_row(&harness, "2").click_modifiers(egui::Modifiers::COMMAND);
        harness.run_steps(2);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let image = harness.render().expect("rendu");
            let window = image::imageops::crop_imm(&image, 990, 600, 920, 600).to_image();
            window.save(std::path::Path::new(&dir).join("fixture-sheet.png")).expect("écriture de l'image");
        }

        // Filtres : par type, puis par groupe.
        let shown = |h: &Harness<'_, FolieApp>| {
            ["1", "2", "3", "4", "10"].into_iter().filter(|n| h.query(By::new().role(Role::Row).label(n)).is_some()).collect::<Vec<_>>()
        };
        harness.get(By::new().role(Role::Button).label("Showtec Sunstrip Active MKII")).click();
        harness.run_steps(2);
        assert_eq!(shown(&harness), ["10"]);
        harness.get_by_label("Tous").click();
        harness.run_steps(2);
        harness.get_by_label("Aucune").click();
        harness.run_steps(2);
        command(&mut harness, "Fixture 3 + 1");
        command(&mut harness, r#"Store Group 1 "Cour""#);
        harness.get_by_role_and_label(Role::ComboBox, "Groupe").click();
        harness.run_steps(2);
        harness.get_by_label("1 Cour").click();
        harness.run_steps(2);
        assert_eq!(shown(&harness), ["1", "3"]);
        harness.get_by_label("Tout sélectionner").click();
        harness.run_steps(2);
        assert_eq!(selection(&harness), ["1", "3"], "seulement les fixtures affichées");

        // Cmd-F ferme la fenêtre, puis la rouvre.
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
        harness.run_steps(2);
        assert!(!harness.state().windows.workspace.is_open(WindowKind::Fixtures));
        assert!(harness.query(By::new().role(Role::Row).label("1")).is_none());
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::F);
        harness.run_steps(2);
        sheet_row(&harness, "1");
    }

    #[test]
    fn blind_key_shows_the_blind_programmer_without_output() {
        use crate::fixture_sheet::{animated, live_row};
        use crate::theme::Source;
        const BANNER: &str =
            "Programmer blind : rien de ce que vous réglez ne sort, Store l'enregistre. La scène garde le programmer live.";
        let mut harness = harness(Storage::disabled());
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 100");
        assert!(harness.query_by_label(BANNER).is_none());

        harness.get(By::new().role(Role::Button).label("BLIND")).click(); // la touche, pas le bandeau
        settle(&mut harness);
        assert!(harness.state().console.blind());
        assert_eq!(harness.get_all_by_label(BANNER).count(), 2, "au-dessus du Programmer et dans la fenêtre Fixtures");
        command(&mut harness, "Fixture 2 At 50");
        assert_eq!((output(&harness, 2), output(&harness, 16)), (255, 0), "rien ne sort du blind");
        // La feuille montre le programmer blind là où il a des valeurs, la sortie ailleurs.
        let app = harness.state();
        let effects = animated(&app.console, &app.snapshot);
        let intensity =
            |id| live_row(&app.console, &app.snapshot, Target::whole(FixtureId(id)), &effects, DisplayMode::Percent).intensity.unwrap();
        let two = intensity(2);
        assert!((two.value - 0.5).abs() < 0.01 && two.source == Source::Programmer, "{two:?}");
        assert_eq!(intensity(1).value, 1.0, "le live, toujours sur scène");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("blind.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        harness.get(By::new().role(Role::Button).label("BLIND")).click(); // la touche, pas le bandeau
        settle(&mut harness);
        assert!(!harness.state().console.blind());
        assert!(harness.query_by_label(BANNER).is_none());
        harness.get_by_label("Blind coupé : retour au programmer live (1 élément repris)");
        assert_eq!((output(&harness, 2), output(&harness, 16)), (255, 0));
    }

    /// Remplace le texte du champ « Adresse » du menu ouvert, puis valide par Entrée.
    fn type_address(harness: &mut Harness<'_, FolieApp>, text: &str) {
        harness.get_by_role_and_label(Role::TextInput, "Adresse").focus();
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        harness.get_by_role_and_label(Role::TextInput, "Adresse").type_text(text);
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        settle(harness);
    }

    /// Le Patch en grand, et la fenêtre Fixtures dessous, sans les pools.
    fn patch_harness() -> Harness<'static, FolieApp> {
        let window = |kind, y: f32, h: f32| WindowState {
            kind,
            placement: Placement::at_size(egui::pos2(1000.0, y), egui::vec2(700.0, h)),
            preset_family: None,
        };
        let workspace = Workspace {
            main: Placement::default(),
            windows: vec![window(WindowKind::Patch, 40.0, 580.0), window(WindowKind::Fixtures, 680.0, 480.0)],
        };
        harness_with(Storage::disabled(), workspace, egui::vec2(1960.0, 1200.0))
    }

    #[test]
    fn readdress_from_the_patch_table() {
        let mut harness = patch_harness();
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 4);
        harness.get_by_label("Aucune").click();
        harness.run_steps(2);
        command(&mut harness, "Fixture 2 At 50");
        harness.state_mut().file.dirty = false;

        // Clic droit sur la ligne de la fixture 2 : son adresse actuelle est dans le champ.
        harness.get_by_label("2 · Rush MH 6 Wash 2 · 1.015–028").click_secondary();
        harness.run_steps(2);
        assert_eq!(harness.get_by_role_and_label(Role::TextInput, "Adresse").value().as_deref(), Some("1.015"));
        harness.get_by_label("Fixture 2");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("patch-address-menu.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Conflit : expliqué dans le menu, qui reste ouvert, et rien ne bouge.
        type_address(&mut harness, "1.020");
        harness.get_by_label("Impossible : chevauche la fixture 3");
        assert_eq!(harness.state().console.show.patch.get(FixtureId(2)).unwrap().address, 15);
        type_address(&mut harness, "2.001");
        harness.get_by_label("Un seul univers pour l'instant : 1.xxx");

        // Adresse libre : la fixture garde son numéro et ses valeurs, qui sortent à la nouvelle adresse.
        type_address(&mut harness, "201");
        assert_eq!(harness.state().console.show.patch.get(FixtureId(2)).unwrap().address, 201);
        assert!(harness.query_by_role_and_label(Role::TextInput, "Adresse").is_none(), "menu refermé");
        harness.get_by_label("Fixture 2 déplacée en 1.201");
        harness.get_by_label("2 · Rush MH 6 Wash 2 · 1.201–214"); // le tableau suit
        assert_eq!((output(&harness, 202), output(&harness, 16)), (128, 0));
        assert!(harness.state().file.dirty);

        // Plusieurs fixtures sélectionnées : à la suite, dans l'ordre de sélection.
        command(&mut harness, "Fixture 4 + 3");
        harness.get_by_label("3 · Rush MH 6 Wash 3 · 1.029–042").click_secondary();
        harness.run_steps(2);
        harness.get_by_label("2 fixtures sélectionnées, à la suite");
        type_address(&mut harness, "301");
        let address = |h: &Harness<'_, FolieApp>, id| h.state().console.show.patch.get(FixtureId(id)).unwrap().address;
        assert_eq!((address(&harness, 4), address(&harness, 3), address(&harness, 1)), (301, 315, 1));
        harness.get_by_label("2 fixtures déplacées à la suite à partir de 1.301");

        // La touche « Changer l'adresse… » fait de même pour la sélection.
        harness.get_by_label("Aucune").click();
        harness.run_steps(2);
        command(&mut harness, "Fixture 1");
        harness.get_by_label("Changer l'adresse…").click();
        harness.run_steps(2);
        harness.get_by_label("Fixture 1");
        type_address(&mut harness, "401");
        assert_eq!(address(&harness, 1), 401);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let image = harness.render().expect("rendu");
            let window = image::imageops::crop_imm(&image, 990, 20, 720, 620).to_image();
            window.save(std::path::Path::new(&dir).join("patch-table.png")).expect("écriture de l'image");
        }

        // Supprimer : la sélection quitte le patch.
        harness.get_by_label("Supprimer (1)").click();
        settle(&mut harness);
        assert!(harness.state().console.show.patch.get(FixtureId(1)).is_none());
        assert!(harness.query_by_label("1 · Rush MH 6 Wash 1 · 1.401–414").is_none());
    }

    #[test]
    fn add_form_warns_about_conflicts_before_patching() {
        let mut harness = patch_harness();
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        // Adresse choisie à la main, sur la fixture 1 : le conflit est dit, Patcher est grisé.
        let form = &mut harness.state_mut().fixtures_view.form;
        form.auto_address = false;
        form.address = 10;
        harness.run_steps(2);
        harness.get_by_label("Conflit : Fixture 3 : chevauche la fixture 1");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let image = harness.render().expect("rendu");
            let window = image::imageops::crop_imm(&image, 990, 20, 720, 320).to_image();
            window.save(std::path::Path::new(&dir).join("patch-conflict.png")).expect("écriture de l'image");
        }
        harness.get_by_label("Patcher").click();
        settle(&mut harness);
        assert_eq!(harness.state().console.show.patch.fixtures().len(), 2, "rien n'est patché");
        // Numéro déjà pris.
        let form = &mut harness.state_mut().fixtures_view.form;
        form.address = 100;
        form.first_id = 2;
        harness.run_steps(2);
        harness.get_by_label("Conflit : Fixture 2 : la fixture 2 existe déjà");
        // Place libre : on peut patcher.
        harness.state_mut().fixtures_view.form.first_id = 3;
        harness.run_steps(2);
        harness.get_by_label("2 × 14 = 28 canaux");
        harness.get_by_label("Patcher").click();
        settle(&mut harness);
        assert_eq!(harness.state().console.show.patch.get(FixtureId(3)).unwrap().address, 100);
        harness.get_by_label("2 fixtures patchées");
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

    /// Bouton rouge de la fenêtre principale. Sur macOS, ⌘Q et « Quitter Folie-DMX »
    /// y sont renvoyés (`route_quit_to_main_window`).
    fn close_main_window(harness: &mut Harness<'_, FolieApp>) {
        let root = harness.input_mut().viewports.entry(egui::ViewportId::ROOT).or_default();
        root.events.push(egui::ViewportEvent::Close);
        harness.run_steps(2);
    }

    fn has_autosave(data: &std::path::Path) -> bool {
        Storage::at(data).recovery().is_some()
    }

    /// Un show enregistré dans `concert.folie`, deux Rush patchées.
    fn saved_concert(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
        let data = dir.join("donnees");
        let show_path = dir.join("concert.folie");
        let mut harness = harness(Storage::at(&data));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        assert!(harness.state_mut().save_to(&show_path));
        (data, show_path)
    }

    #[test]
    fn closing_without_changes_leaves_nothing_to_recover() {
        let dir = tempfile::tempdir().unwrap();
        let (data, show_path) = saved_concert(dir.path());

        let mut first = harness(Storage::at(&data));
        assert!(!first.state().windows.workspace.windows.is_empty(), "des fenêtres secondaires sont ouvertes");
        // Une modification recopiée, puis enregistrée : la copie de secours part avec.
        command(&mut first, "Fixture 1 Thru 2");
        command(&mut first, r#"Store Group 1 "Tout""#);
        first.state_mut().autosave_now();
        assert!(has_autosave(&data));
        first.get_by_label("Non enregistré").click();
        first.run_steps(2);
        assert!(!has_autosave(&data));

        close_main_window(&mut first);
        assert!(first.state().file.pending.is_none(), "rien à demander");
        assert!(first.query_by_label("Modifications non enregistrées").is_none());
        first.state_mut().file_exit();
        drop(first);

        let next = harness(Storage::at(&data));
        assert!(next.state().file.recovery.is_none(), "aucune question après une fermeture normale");
        assert_eq!(next.state().file.path.as_deref(), Some(show_path.as_path()));
        assert!(next.state().console.show.groups.contains_key(&1));

        // Un show neuf jamais touché se ferme aussi sans rien demander.
        let mut fresh = harness(Storage::disabled());
        close_main_window(&mut fresh);
        assert!(fresh.state().file.pending.is_none());
    }

    #[test]
    fn closing_with_changes_asks_first() {
        let dir = tempfile::tempdir().unwrap();
        let (data, show_path) = saved_concert(dir.path());

        let mut first = harness(Storage::at(&data));
        command(&mut first, "Fixture 1 Thru 2");
        command(&mut first, r#"Store Group 1 "Tout""#);
        first.state_mut().autosave_now();

        // Échap (comme un clic à côté) revient au show, sans rien perdre.
        close_main_window(&mut first);
        first.get_by_label("Modifications non enregistrées");
        first.key_press(egui::Key::Escape);
        first.run_steps(2);
        assert!(first.query_by_label("Modifications non enregistrées").is_none());
        assert!(first.state().file.dirty);
        assert!(!first.state().file.closing);
        assert!(has_autosave(&data), "la copie de secours reste tant qu'on n'a pas choisi");

        // Ne pas enregistrer : la copie de secours est effacée, même si l'arrêt passe ensuite par on_exit.
        close_main_window(&mut first);
        first.get_by_label("Ne pas enregistrer").click();
        first.run_steps(2);
        assert!(first.state().file.closing);
        first.state_mut().file_exit();
        assert!(!has_autosave(&data));
        drop(first);

        let mut next = harness(Storage::at(&data));
        assert!(next.state().file.recovery.is_none());
        assert!(next.state().console.show.groups.is_empty(), "la modification abandonnée n'est pas là");

        // Enregistrer : écrit dans concert.folie, puis ferme.
        command(&mut next, "Fixture 1 Thru 2");
        command(&mut next, r#"Store Group 2 "Face""#);
        close_main_window(&mut next);
        next.get_by_label("Enregistrer").click();
        next.run_steps(2);
        assert!(next.state().file.closing);
        assert!(!has_autosave(&data));
        drop(next);
        let reopened = folie_show::load(&show_path, &harness(Storage::disabled()).state().library).unwrap();
        assert!(reopened.show.groups.contains_key(&2));
    }

    #[test]
    fn quitting_from_the_dock_keeps_changes_for_next_launch() {
        // « Quitter » du Dock et la fermeture de session ne demandent rien : seul on_exit est appelé.
        let dir = tempfile::tempdir().unwrap();
        let (data, _) = saved_concert(dir.path());

        let mut first = harness(Storage::at(&data));
        command(&mut first, "Fixture 1 Thru 2");
        command(&mut first, r#"Store Group 1 "Tout""#);
        assert!(!has_autosave(&data), "pas encore recopiée");
        first.state_mut().file_exit();
        assert!(has_autosave(&data), "recopiée à l'arrêt : rien ne se perd");
        drop(first);

        let mut next = harness(Storage::at(&data));
        next.get_by_label("Récupérer").click();
        settle(&mut next);
        assert!(next.state().console.show.groups.contains_key(&1));
    }

    #[test]
    fn recovered_show_is_complete_and_keeps_its_path() {
        let dir = tempfile::tempdir().unwrap();
        let (data, show_path) = saved_concert(dir.path());

        let mut first = harness(Storage::at(&data));
        command(&mut first, "Fixture 1 Thru 2");
        command(&mut first, r#"Store Group 1 "Tout""#);
        command(&mut first, "ClearAll");
        command(&mut first, "Fixture 1 At 100");
        command(&mut first, r#"Store Cue 1 Fade 0 "Face""#);
        command(&mut first, "ClearAll");
        command(&mut first, "Fixture 2 At 30");
        command(&mut first, r#"Store Preset 1.1 "Demi""#);
        command(&mut first, "ClearAll");
        let expected = first.state().console.show.clone();
        assert!(!expected.groups.is_empty() && !expected.presets.is_empty());
        assert!(!expected.sequences.is_empty() && !expected.executors.is_empty());
        first.state_mut().autosave_now();
        drop(first); // plantage

        let mut second = harness(Storage::at(&data));
        second.get_by_label("Session non enregistrée");
        second.get_by_label("Folie-DMX s'est fermé sans enregistrer des modifications de « concert.folie ».");
        second.get_by_label("Récupérer").click();
        settle(&mut second);
        assert!(second.query_by_label("Session non enregistrée").is_none());
        let app = second.state();
        assert_eq!(app.console.show, expected, "patch, groupes, presets, cues et executors");
        assert_eq!(app.file.path.as_deref(), Some(show_path.as_path()), "le show d'origine");
        assert!(app.file.dirty, "reste à enregistrer");
        second.get_by_label("Non enregistré");

        // Fermer sans choisir : on demande, la copie reste.
        close_main_window(&mut second);
        assert_eq!(second.state().file.pending, Some(file::Pending::Quit));
        second.get_by_label("Enregistrer").click();
        second.run_steps(2);
        assert!(!has_autosave(&data));
        drop(second);

        let third = harness(Storage::at(&data));
        assert!(third.state().file.recovery.is_none(), "la question ne revient plus");
        assert_eq!(third.state().console.show, expected);
    }

    #[test]
    fn ignoring_the_recovery_reopens_the_saved_show() {
        let dir = tempfile::tempdir().unwrap();
        let (data, show_path) = saved_concert(dir.path());

        let mut first = harness(Storage::at(&data));
        command(&mut first, "Fixture 1 Thru 2");
        command(&mut first, r#"Store Group 1 "Tout""#);
        first.state_mut().autosave_now();
        drop(first);

        let mut second = harness(Storage::at(&data));
        second.get_by_label("Ignorer les supprime définitivement.");
        second.get_by_label("Ignorer").click();
        settle(&mut second);
        assert!(second.query_by_label("Session non enregistrée").is_none());
        assert!(!has_autosave(&data), "la copie de secours est supprimée");
        let app = second.state();
        assert_eq!(app.file.path.as_deref(), Some(show_path.as_path()));
        assert_eq!(app.console.show.patch.fixtures().len(), 2, "le dernier show enregistré");
        assert!(app.console.show.groups.is_empty());
        assert!(!app.file.dirty);
        drop(second);

        let third = harness(Storage::at(&data));
        assert!(third.state().file.recovery.is_none());
    }

    #[test]
    fn the_recovery_question_cannot_be_dismissed_by_accident() {
        let dir = tempfile::tempdir().unwrap();
        let (data, _) = saved_concert(dir.path());
        let mut first = harness(Storage::at(&data));
        command(&mut first, "Fixture 1 Thru 2");
        command(&mut first, r#"Store Group 1 "Tout""#);
        first.state_mut().autosave_now();
        drop(first);
        let recovered = std::fs::read(Storage::at(&data).recovery().unwrap().path).unwrap();

        let mut second = harness(Storage::at(&data));
        // Échap, un clic à côté, ⌘N : la question reste posée.
        second.key_press(egui::Key::Escape);
        second.run_steps(2);
        let outside = egui::pos2(5.0, 600.0);
        second.event(egui::Event::PointerMoved(outside));
        for pressed in [true, false] {
            let button = egui::PointerButton::Primary;
            second.event(egui::Event::PointerButton { pos: outside, button, pressed, modifiers: egui::Modifiers::NONE });
        }
        second.run_steps(2);
        second.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::N);
        second.run_steps(2);
        second.get_by_label("Session non enregistrée");
        assert!(second.state().file.recovery.is_some());

        // Même 30 s plus tard, la copie à récupérer n'est pas écrasée par le show vide.
        second.state_mut().mark_dirty();
        second.state_mut().autosave_due();
        second.run_steps(2);
        assert_eq!(std::fs::read(Storage::at(&data).recovery().unwrap().path).unwrap(), recovered);

        // Quitter sans répondre : la question reviendra.
        second.state_mut().file.dirty = false;
        close_main_window(&mut second);
        second.state_mut().file_exit();
        assert!(has_autosave(&data));
        drop(second);
        let third = harness(Storage::at(&data));
        assert!(third.state().file.recovery.is_some());
    }

    #[test]
    fn autosave_is_written_only_for_real_changes() {
        let dir = tempfile::tempdir().unwrap();
        let (data, _) = saved_concert(dir.path());

        let mut harness = harness(Storage::at(&data));
        let written = |harness: &mut Harness<'_, FolieApp>| {
            harness.state_mut().autosave_due();
            harness.run_steps(2);
            has_autosave(&data)
        };
        assert!(!harness.state().file.dirty, "ouvrir un show ne le modifie pas");
        assert!(!written(&mut harness));

        // Programmer et fenêtres ne font pas partie du show.
        command(&mut harness, "Fixture 1 At 50");
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1);
        harness.run_steps(2);
        assert!(!harness.state().file.dirty);
        assert!(!written(&mut harness));

        // Marqué modifié sans l'être (ou modification défaite) : rien à recopier.
        harness.state_mut().mark_dirty();
        assert!(!written(&mut harness));
        assert!(!harness.state().file.dirty, "plus rien à enregistrer");

        // Une vraie modification, elle, est recopiée.
        command(&mut harness, "Fixture 1 Thru 2");
        command(&mut harness, r#"Store Group 1 "Tout""#);
        assert!(written(&mut harness));
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
        // (l'accueil a aussi un lien « Groupes », plus bas dans la fenêtre).
        let menu_entry = harness.get_all(By::new().role(Role::Button).label_contains("Groupes")).min_by(|a, b| a.rect().min.y.total_cmp(&b.rect().min.y));
        menu_entry.unwrap().click();
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
        harness.get_by_label("STORE").click(); // celui de la barre du haut
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
        // La plage du Pan à la souris : poignée droite, bande entière, Option pour la symétrie.
        let bar = harness.get_all_by_label("Plage -10 à +10 % autour de la valeur").next().unwrap().rect();
        let x = |v: f32| bar.left() + bar.width() * (v + 100.0) / 200.0;
        let y = bar.center().y;
        let gesture = |h: &mut Harness<'_, FolieApp>, from: f32, to: f32, modifiers: egui::Modifiers| {
            h.hover_at(egui::pos2(from, y));
            h.run_steps(1);
            let press = |x: f32, pressed| egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            h.event_modifiers(press(from, true), modifiers);
            h.run_steps(1);
            for i in 1..=8 {
                h.event_modifiers(egui::Event::PointerMoved(egui::pos2(from + (to - from) * i as f32 / 8.0, y)), modifiers);
                h.run_steps(1);
            }
            h.event_modifiers(press(to, false), modifiers);
            h.run_steps(2);
        };
        let pan = |h: &Harness<'_, FolieApp>| {
            let line = &h.state().console.programmer_effects()[0].lines[0];
            (line.low, line.high)
        };
        gesture(&mut harness, x(10.0), x(30.0), egui::Modifiers::NONE);
        assert_eq!(pan(&harness), (-10.0, 30.0), "seule la poignée droite a bougé");
        gesture(&mut harness, x(0.0), x(-20.0), egui::Modifiers::NONE);
        assert_eq!(pan(&harness), (-30.0, 10.0), "la bande entière, même largeur");
        gesture(&mut harness, x(10.0), x(20.0), egui::Modifiers::ALT);
        assert_eq!(pan(&harness), (-40.0, 20.0), "symétrique autour du centre de la bande (-10)");
        gesture(&mut harness, x(80.0), x(80.0), egui::Modifiers::NONE);
        assert_eq!(pan(&harness), (-40.0, 20.0), "un clic dans le vide ne fait rien");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            // Pendant un glissé de la poignée droite : la valeur s'affiche au-dessus.
            let hold = |x: f32, pressed| egui::Event::PointerButton {
                pos: egui::pos2(x, y),
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            harness.hover_at(egui::pos2(x(20.0), y));
            harness.run_steps(1);
            harness.event(hold(x(20.0), true));
            harness.run_steps(1);
            for i in 1..=6 {
                harness.hover_at(egui::pos2(x(20.0 + 4.0 * i as f32), y));
                harness.run_steps(1);
            }
            let path = std::path::Path::new(&dir).join("range-drag.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
            harness.event(hold(x(44.0), false));
            harness.run_steps(2);
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

    /// Actif, éteint, en fondu : l'état de chaque executor se lit de loin.
    #[test]
    fn executor_state_at_a_glance() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Executors,
            placement: Placement::at_size(egui::pos2(900.0, 820.0), egui::vec2(1350.0, 700.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1600.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 4);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, r#"Store Cue 1 Fade 0 "Face""#);
        for (fixture, executor) in [(2, 2), (3, 3), (4, 11)] {
            command(&mut harness, "ClearAll");
            command(&mut harness, &format!("Fixture {fixture} At 100"));
            command(&mut harness, &format!("Store Executor {executor}"));
        }
        command(&mut harness, "ClearAll");
        command(&mut harness, "Go Executor 1");
        command(&mut harness, "Go Executor 11");
        // L'executor 3 part sur son fondu de 2 s : la capture le prend en route.
        command(&mut harness, "Go Executor 3");

        let executors = &harness.state().snapshot.executors;
        assert!(executors[&1].cue.is_some() && executors[&11].cue.is_some());
        assert!(executors[&3].in_transition(), "fondu en cours");
        assert!(executors.get(&2).is_none_or(|s| s.cue.is_none()), "l'executor 2 reste éteint");
        assert_eq!(harness.get_all_by_label("› fin de séquence").count(), 3, "executors 1, 3 et 11");
        harness.get_by_label("› 1"); // l'executor 2, éteint
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let image = harness.render().expect("rendu");
            let window = image::imageops::crop_imm(&image, 900, 820, 1350, 700).to_image();
            window.save(std::path::Path::new(&dir).join("executors-state.png")).expect("écriture de l'image");
        }
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
    fn artnet_output_from_the_dmx_window() {
        let dir = tempfile::tempdir().unwrap();
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Output,
            placement: Placement::at_size(egui::pos2(40.0, 820.0), egui::vec2(1100.0, 720.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::at(dir.path()), workspace, egui::vec2(2000.0, 1600.0));
        assert!(harness.query_by_label("Art-Net").is_none(), "pas de pastille tant qu'il est inactif");

        // Destination : ce Mac, puis Actif.
        harness.get_by_role_and_label(Role::TextInput, "Destination").focus();
        harness.run_steps(1);
        harness.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::A]); // tout le texte
        harness.run_steps(1);
        harness.get_by_role_and_label(Role::TextInput, "Destination").type_text("127.0.0.1");
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert_eq!(harness.state().artnet.destination, "127.0.0.1");
        harness.get_all_by_label("Actif").next().unwrap().click(); // la première : Art-Net
        settle(&mut harness);
        assert!(harness.state().artnet.enabled);
        let names = harness.state().snapshot.output_names.clone();
        assert_eq!(names, ["virtuelle", "Art-Net 127.0.0.1 (0:0:0)"], "en plus de la sortie choisie");
        harness.get_by_label("Art-Net"); // pastille de la barre du haut
        assert!(Storage::at(dir.path()).artnet().enabled, "gardé pour la prochaine fois");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("artnet.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Une adresse illisible : l'Art-Net se coupe et le dit.
        harness.state_mut().artnet.destination = "pas une adresse".into();
        harness.state_mut().artnet_changed();
        settle(&mut harness);
        assert!(!harness.state().artnet.enabled);
        assert_eq!(harness.state().snapshot.output_names, ["virtuelle"]);
        harness.get_by_label("Art-Net : adresse « pas une adresse » illisible");

        // sACN, univers 1 en multicast, en plus de la sortie virtuelle.
        harness.get_all_by_label("Actif").nth(1).unwrap().click();
        settle(&mut harness);
        assert!(harness.state().sacn.enabled);
        assert_eq!(harness.state().snapshot.output_names, ["virtuelle", "sACN univers 1 (multicast)"]);
        harness.get_by_label("sACN"); // pastille de la barre du haut
        assert!(Storage::at(dir.path()).sacn().enabled, "gardé pour la prochaine fois");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("sacn.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.state_mut().sacn.destination = "node ?".into();
        harness.state_mut().artnet_changed();
        settle(&mut harness);
        assert!(!harness.state().sacn.enabled);
        harness.get_by_label("sACN : adresse « node ? » illisible");
    }

    #[test]
    fn web_remote_from_the_top_bar() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = harness(Storage::at(dir.path()));
        harness.state_mut().remote_settings.port = 0; // port libre, choisi par le système
        assert!(harness.state().remote.is_none(), "coupée par défaut");

        harness.get_by_label("Télécommande").click();
        harness.run_steps(2);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("telecommande-coupee.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Activer la télécommande").click();
        settle(&mut harness);
        let port = harness.state().remote.as_ref().expect("serveur démarré").port();
        let pin = harness.state().remote_settings.pin.clone();
        let playback_pin = harness.state().remote_settings.playback_pin.clone();
        for code in [&pin, &playback_pin] {
            assert!(code.len() == 4 && code.chars().all(|c| c.is_ascii_digit()), "{code}");
        }
        assert_ne!(pin, playback_pin);
        assert!(Storage::at(dir.path()).remote().enabled, "gardé pour la prochaine fois");
        harness.get_by_label(&pin);
        harness.get_by_label(&playback_pin);
        harness.get_by_label("Aucun appareil connecté");

        // Un téléphone : il change de page d'executors, la console suit et le lui renvoie.
        let (mut phone, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}/ws?pin={pin}")).expect("connexion");
        phone.send(tungstenite::Message::text(r#"{"type":"page","page":2}"#)).unwrap();
        for _ in 0..50 {
            settle(&mut harness);
            if harness.state().console.page() == 2 {
                break;
            }
        }
        assert_eq!(harness.state().console.page(), 2);
        let state = loop {
            let message = phone.read().unwrap();
            let Ok(text) = message.to_text() else { continue };
            let json: serde_json::Value = serde_json::from_str(text).unwrap();
            if json["type"] == "state" && json["page"] == 2 {
                break json;
            }
        };
        assert_eq!(state["show"], "Sans titre");
        settle(&mut harness);
        harness.get_by_label("Télécommande · 1");
        harness.get_by_label("1 appareil connecté");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("telecommande.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Couper : le téléphone est déconnecté, le réglage retenu.
        harness.get_by_label("Couper la télécommande").click();
        settle(&mut harness);
        assert!(harness.state().remote.is_none());
        assert!(!Storage::at(dir.path()).remote().enabled);
        let closed = (0..20).any(|_| phone.read().is_err());
        assert!(closed, "téléphone déconnecté");
    }

    #[test]
    fn osc_from_the_top_bar() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = harness(Storage::at(dir.path()));
        // Ports choisis par le système ; retour vers le port d'où le client écrit.
        harness.state_mut().osc_settings.port = 0;
        harness.state_mut().osc_settings.reply_port = 0;
        harness.get_by_label("Télécommande").click();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::TextInput, "Autorisés").focus();
        harness.run_steps(1);
        harness.get_by_role_and_label(Role::TextInput, "Autorisés").type_text("127.0.0.1, 192.168.1.30");
        harness.run_steps(1);
        harness.key_press(egui::Key::Enter);
        harness.run_steps(2);
        assert_eq!(harness.state().osc_settings.allowed, "127.0.0.1, 192.168.1.30");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("osc-coupe.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Suivre les marqueurs de Reaper").click();
        harness.run_steps(2);
        assert!(harness.state().osc_settings.reaper_markers);
        harness.get_by_label("Activer l'OSC").click();
        settle(&mut harness);
        let port = harness.state().osc.as_ref().expect("OSC démarré").port();
        assert!(Storage::at(dir.path()).osc().enabled, "gardé pour la prochaine fois");

        // Un client : page 3, puis le retour d'état lui arrive.
        let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let packet = rosc::OscPacket::Message(rosc::OscMessage { addr: "/folie/page".into(), args: vec![rosc::OscType::Int(3)] });
        client.send_to(&rosc::encoder::encode(&packet).unwrap(), ("127.0.0.1", port)).unwrap();
        for _ in 0..50 {
            settle(&mut harness);
            if harness.state().console.page() == 3 {
                break;
            }
        }
        assert_eq!(harness.state().console.page(), 3);
        let mut buffer = [0u8; rosc::decoder::MTU];
        // Le premier envoi peut encore dire page 1 : on lit jusqu'à la page 3.
        let mut pages = Vec::new();
        for _ in 0..40 {
            let Ok((size, _)) = client.recv_from(&mut buffer) else { break };
            let Ok((_, rosc::OscPacket::Bundle(bundle))) = rosc::decoder::decode_udp(&buffer[..size]) else { continue };
            pages.extend(bundle.content.into_iter().filter_map(|p| match p {
                rosc::OscPacket::Message(m) if m.addr == "/folie/page" => m.args.first().cloned(),
                _ => None,
            }));
            if pages.last() == Some(&rosc::OscType::Int(3)) {
                break;
            }
        }
        assert_eq!(pages.last(), Some(&rosc::OscType::Int(3)), "{pages:?}");
        settle(&mut harness);
        harness.get_by_label("1 client entendu");
        harness.get_by_label("Télécommande · 1");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("osc.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Couper l'OSC").click();
        settle(&mut harness);
        assert!(harness.state().osc.is_none());

        // Une adresse autorisée illisible : l'OSC ne démarre pas et le dit.
        harness.state_mut().osc_settings.allowed = "192.168.1".into();
        harness.state_mut().osc_settings.enabled = true;
        let ctx = harness.ctx.clone();
        harness.state_mut().osc_changed(&ctx);
        settle(&mut harness);
        assert!(harness.state().osc.is_none());
        harness.get_by_label("OSC : adresse autorisée « 192.168.1 » illisible");
    }

    #[test]
    fn cue_command_sends_osc_to_reaper() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Cues,
            placement: Placement::at_size(egui::pos2(1520.0, 40.0), egui::vec2(560.0, 380.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1400.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 1);
        command(&mut harness, "Fixture 1 At 100");
        command(&mut harness, r#"Store Cue 1 Fade 0 "Ouverture""#);
        command(&mut harness, "ClearAll");

        // Destination Reaper toute prête, puis dirigée vers un faux Reaper pour le test.
        harness.get_by_label("Télécommande").click();
        harness.run_steps(2);
        harness.get_by_label("Ajouter Reaper (127.0.0.1:8000)").click();
        settle(&mut harness);
        assert_eq!(harness.state().console.show.osc_destinations["Reaper"], "127.0.0.1:8000");
        assert!(harness.state().file.dirty, "enregistrée dans le show");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("osc-destinations.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        let reaper = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        reaper.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let address = reaper.local_addr().unwrap().to_string();
        harness.state_mut().console.set_osc_destination("Reaper", &address).unwrap();

        // Clic droit sur la cue : sa commande.
        harness.get_by_label("Ouverture").click_secondary();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::TextInput, "Au départ").focus();
        harness.run_steps(1);
        harness.get_by_role_and_label(Role::TextInput, "Au départ").type_text(r#"Osc "Reaper" "/marker" 1; Osc "Reaper" "/play""#);
        harness.run_steps(2);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("cue-command.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Valider").click();
        settle(&mut harness);
        let number = folie_show::CueNumber::whole(1);
        assert_eq!(harness.state().console.show.sequences[&1].cues[&number].command, r#"Osc "Reaper" "/marker" 1; Osc "Reaper" "/play""#);
        harness.key_press(egui::Key::Escape);
        harness.run_steps(2);
        harness.get_by_label("CMD"); // repère sur la ligne de la cue

        // GO : Reaper reçoit le marqueur puis la lecture.
        command(&mut harness, "Go");
        settle(&mut harness);
        settle(&mut harness);
        let mut buffer = [0u8; 1024];
        let mut received = Vec::new();
        for _ in 0..2 {
            let (size, _) = reaper.recv_from(&mut buffer).expect("message OSC");
            let (_, rosc::OscPacket::Message(m)) = rosc::decoder::decode_udp(&buffer[..size]).unwrap() else { panic!() };
            received.push(m.addr);
        }
        assert_eq!(received, ["/marker", "/play"]);
        harness.get_by_label("OSC vers Reaper /play");
    }

    #[test]
    fn about_from_the_file_menu() {
        let dir = tempfile::tempdir().unwrap();
        let mut harness = harness(Storage::at(dir.path()));
        harness.get_by_label("Fichier").click();
        harness.run_steps(2);
        harness.get_all_by_label("À propos de Folie-DMX").last().unwrap().click(); // l'entrée du menu, pas le logo
        harness.run_steps(2);
        harness.get_by_label(&format!("Folie-DMX {}", env!("CARGO_PKG_VERSION")));
        harness.get_by_label("Console lumière inspirée des standards de l'industrie.");
        harness.get_by_label("Créée par Michaël Mambole");
        harness.get_by_label("Open Fixture Library");
        harness.get_by_label(&dir.path().display().to_string());
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("a-propos.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Fermer").click();
        harness.run_steps(2);
        assert!(!harness.state().about_open);
    }

    /// Fenêtre Son, sans ouvrir l'entrée audio (pas de micro pendant les tests).
    #[test]
    fn sound_window_reactions() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Sound,
            placement: Placement::at_size(egui::pos2(1500.0, 40.0), egui::vec2(620.0, 620.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2200.0, 1300.0));
        harness.get_by_label("Aucune : ajoutez-en une pour l'executor sélectionné.");
        harness.get_by_label("Ajouter une réaction").click();
        settle(&mut harness);
        let sound = |h: &Harness<'_, FolieApp>| h.state().console.show.sound.clone();
        assert_eq!(sound(&harness).reactions, [folie_show::sound::SoundReaction {
            executor: 1,
            reaction: folie_show::sound::Reaction::Step { every: 1 },
        }]);
        assert!(harness.state().file.dirty, "enregistré dans le show");

        // Pas sur le beat -> Fader au niveau, graves par défaut.
        // La liste du type de réaction affiche sa valeur, pas un libellé.
        harness.get(By::new().predicate(|n| n.value().as_deref() == Some("Pas sur le beat"))).click();
        harness.run_steps(2);
        harness.get_all_by_label("Fader au niveau").last().unwrap().click();
        settle(&mut harness);
        assert!(matches!(
            sound(&harness).reactions[0].reaction,
            folie_show::sound::Reaction::Fader { band: folie_show::sound::Band::Bass, .. }
        ));
        harness.get_by_label("Le tempo suit le beat").click();
        settle(&mut harness);
        assert!(sound(&harness).tempo);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("son.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        harness.get(described("Retirer cette réaction")).click();
        settle(&mut harness);
        assert!(sound(&harness).reactions.is_empty());
        assert!(harness.state().audio.is_none(), "l'entrée n'a pas été ouverte");
    }

    /// Interface simulée qu'on débranche et rebranche.
    struct Pluggable(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl folie_dmx::DmxOutput for Pluggable {
        fn name(&self) -> &str {
            "Enttec USB Pro (essai)"
        }
        fn send(&mut self, _universe: &folie_dmx::Universe) -> Result<(), folie_dmx::OutputError> {
            Ok(())
        }
        fn connected(&self) -> bool {
            self.0.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    #[test]
    fn unplugged_interface_is_shown_until_it_returns() {
        use std::sync::atomic::Ordering;
        let mut harness = harness(Storage::disabled());
        let plugged = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        harness.state_mut().output = OutputChoice::Port("/dev/cu.essai".into());
        let output = Box::new(Pluggable(plugged.clone()));
        harness.state().engine().send(Command::SetOutputs(vec![OutputBinding { universe: 0, output }]));
        settle(&mut harness);
        harness.get_by_label("USB Pro");

        plugged.store(false, Ordering::Relaxed);
        settle(&mut harness);
        harness.get_by_label("USB Pro débranchée");
        harness.get_by_label("Enttec USB Pro (essai) débranchée : reconnexion automatique dès qu'elle revient");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("usb-debranchee.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        plugged.store(true, Ordering::Relaxed);
        settle(&mut harness);
        harness.get_by_label("USB Pro");
        harness.get_by_label("Enttec USB Pro (essai) reconnectée");
    }

    #[test]
    fn stage_draws_a_sunstrip_as_a_bar_of_cells() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Stage,
            placement: Placement::at_size(egui::pos2(1520.0, 40.0), egui::vec2(900.0, 560.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2500.0, 1000.0));
        harness.get_by_role_and_label(Role::ComboBox, "Type").click();
        harness.run_steps(2);
        harness.get_by_label("Showtec Sunstrip Active MKII").click();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::ComboBox, "Mode").click();
        harness.run_steps(2);
        harness.get_by_label("Individual (10 canaux)").click();
        harness.run_steps(2);
        harness.get_by_label("Patcher").click();
        settle(&mut harness);
        command(&mut harness, "Clear"); // le patch vient de sélectionner tout le Sunstrip
        command(&mut harness, "Fixture 1.1 At 100");
        command(&mut harness, "Fixture 1.4 At 40");
        command(&mut harness, "Clear");
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1); // ferme le Patch
        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        harness.get(described("Vue Face")).click();
        settle(&mut harness);

        let cell = |h: &Harness<'_, FolieApp>, n: u32| h.get_by_label(&format!("Projecteur 1.{n}")).rect().center().x;
        assert!(cell(&harness, 1) > cell(&harness, 10), "cellule 1 à droite vue de la salle, comme sur l'appareil");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("stage-sunstrip.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Projecteur 1.3").click();
        harness.run_steps(2);
        assert_eq!(harness.state().console.selection(), [Target { fixture: FixtureId(1), head: Some(2) }], "un clic sur une cellule la sélectionne");
    }

    #[test]
    fn stage_view_shows_beams_and_selects() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Stage,
            placement: Placement::at_size(egui::pos2(1520.0, 40.0), egui::vec2(900.0, 560.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2500.0, 1000.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 4);
        command(&mut harness, "ClearAll");
        command(&mut harness, "Fixture 1 Thru 4 At 100");
        // La MH 6 est une LED RGBW : sans couleur, elle n'éclaire pas. Blanc sur les trois premières.
        command(&mut harness, r#"Fixture 1 Thru 3 Attribute "White" At 100"#);
        command(&mut harness, r#"Fixture 2 Attribute "Tilt" At 75"#);
        command(&mut harness, r#"Fixture 3 Attribute "Pan" At 70"#);
        command(&mut harness, r#"Attribute "Tilt" At 70"#);
        command(&mut harness, r#"Fixture 4 Attribute "Red" At 100"#);
        command(&mut harness, "Clear"); // vide la sélection, garde le programmer
        harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Num1); // ferme le Patch
        settle(&mut harness);

        harness.get_by_label("Vue de la scène");
        for n in 1..=4 {
            harness.get_by_label(&format!("Projecteur {n} · Rush MH 6 Wash {n}"));
        }
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("stage.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Projecteur 2 · Rush MH 6 Wash 2").click();
        harness.run_steps(2);
        assert_eq!(harness.state().console.selection(), [Target::whole(FixtureId(2))], "un clic sélectionne");

        let described = |text: &'static str| By::new().predicate(move |n| n.description().as_deref() == Some(text));
        harness.get(described("Vue Dessus")).click();
        harness.run_steps(2);
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("stage-top.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }

        // Placer : la carte Position règle le projecteur sélectionné (la 2).
        harness.get_by_label("Placer").click();
        harness.run_steps(2);
        harness.get_by_label("2 · Rush MH 6 Wash 2");
        harness.get_by_role_and_label(Role::SpinButton, "Hauteur").set_value_accesskit(3.0);
        harness.run_steps(2);
        harness.get_by_label("Au sol").click();
        harness.run_steps(2);
        let place = |h: &Harness<'_, FolieApp>, n: u32| h.state().console.show.stage.get(&FixtureId(n)).copied();
        let two = place(&harness, 2).expect("placée");
        assert_eq!((two.y, two.floor), (3.0, true));
        assert!(harness.state().file.dirty, "le plan est enregistré dans le show");

        // À la souris, en vue de dessus : la 3 glisse vers cour, puis Maj-glisser la monte.
        let block = harness.get_by_label("Projecteur 3 · Rush MH 6 Wash 3").rect().center();
        let drag = |h: &mut Harness<'_, FolieApp>, to: egui::Pos2, modifiers: egui::Modifiers| {
            let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
            h.hover_at(block);
            h.run_steps(1);
            h.event_modifiers(button(block, true), modifiers);
            h.run_steps(1);
            for i in 1..=6 {
                h.event_modifiers(egui::Event::PointerMoved(block + (to - block) * (i as f32 / 6.0)), modifiers);
                h.run_steps(1);
            }
            h.event_modifiers(button(to, false), modifiers);
            h.run_steps(2);
        };
        let auto_x = crate::stage_ui::auto_x(4, 2); // place de la 3 sur la perche
        drag(&mut harness, block + egui::vec2(120.0, 0.0), egui::Modifiers::NONE);
        let three = place(&harness, 3).expect("placée à la souris");
        assert!(three.x > auto_x + 1.0, "vers cour : {three:?}");
        assert!((three.y - 5.0).abs() < 1e-3, "même hauteur");
        assert_eq!(harness.state().console.selection(), [Target::whole(FixtureId(3))], "le projecteur glissé est sélectionné");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("stage-placing.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.remove_cursor(); // l'infobulle du bloc porte le même nom
        harness.run_steps(2);
        let block = harness.get_all_by_label("Projecteur 3 · Rush MH 6 Wash 3").next().unwrap().rect().center();
        let drag_up = |h: &mut Harness<'_, FolieApp>| {
            let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::SHIFT };
            h.hover_at(block);
            h.run_steps(1);
            h.event_modifiers(button(block, true), egui::Modifiers::SHIFT);
            h.run_steps(1);
            for i in 1..=5 {
                h.event_modifiers(egui::Event::PointerMoved(block - egui::vec2(0.0, 10.0 * i as f32)), egui::Modifiers::SHIFT);
                h.run_steps(1);
            }
            h.event_modifiers(button(block - egui::vec2(0.0, 50.0), false), egui::Modifiers::SHIFT);
            h.run_steps(2);
        };
        drag_up(&mut harness);
        assert!(place(&harness, 3).unwrap().y > 5.2, "Maj-glisser vers le haut : plus haut");

        harness.get_by_label("Replacer sur la perche").click();
        harness.run_steps(2);
        assert!(place(&harness, 3).is_none(), "rendue à la perche");
    }

    #[test]
    fn macros_pool_editor_and_executor() {
        let mut workspace = test_workspace();
        workspace.windows.retain(|w| w.kind == WindowKind::Patch);
        workspace.windows.push(WindowState {
            kind: WindowKind::Macros,
            placement: Placement::at_size(egui::pos2(1520.0, 40.0), egui::vec2(560.0, 520.0)),
            preset_family: None,
        });
        workspace.windows.push(WindowState {
            kind: WindowKind::Executors,
            placement: Placement::at_size(egui::pos2(900.0, 820.0), egui::vec2(1350.0, 700.0)),
            preset_family: None,
        });
        let mut harness = harness_with(Storage::disabled(), workspace, egui::vec2(2300.0, 1600.0));
        patch_with_form(&mut harness, "Martin Rush MH 6 Wash", 2);
        command(&mut harness, "ClearAll");
        harness.key_press(egui::Key::Escape); // quitte la ligne de commande

        // Store, puis une case libre : la macro 1 est créée et son éditeur s'ouvre.
        harness.key_press(egui::Key::S);
        harness.run_steps(2);
        let in_pool = |n: &egui_kittest::Node<'_>| n.rect().min.x > 1500.0 && n.rect().max.y < 600.0;
        harness.get_all(By::new().role(Role::Button).label("1")).find(in_pool).unwrap().click();
        harness.run_steps(2);
        assert!(harness.state().console.show.macros.contains_key(&1));
        let lines = harness.get_by_role_and_label(Role::MultilineTextInput, "Une commande par ligne · « Wait 2 » attend 2 secondes");
        lines.focus();
        lines.type_text("Fixture 1 Thru 2\nAt 100\nGo Executor 21");
        harness.run_steps(2);
        harness.get_by_label("Ligne 3 : executor invalide : 21 (place 1 à 20, ou page.place)");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("macros.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        // On corrige la dernière ligne, puis Enregistrer.
        for _ in 0..2 {
            harness.key_press(egui::Key::Backspace);
        }
        harness.get_by_role_and_label(Role::MultilineTextInput, "Une commande par ligne · « Wait 2 » attend 2 secondes").type_text("1");
        harness.run_steps(2);
        assert!(harness.query_by_label_contains("Ligne 3").is_none(), "plus d'erreur");
        harness.get_by_label("Enregistrer").click();
        harness.run_steps(2);
        assert_eq!(harness.state().console.show.macros[&1].lines, ["Fixture 1 Thru 2", "At 100", "Go Executor 1"]);

        // Un clic sur la case lance la macro.
        harness.get_all(By::new().role(Role::Button).label("1\nMacro 1")).find(in_pool).unwrap().click();
        settle(&mut harness);
        assert_eq!(output(&harness, 2), 255, "At 100 sur la fixture 1");
        assert!(harness.state().console.selection().len() == 2);

        // L'executor 3 lance la macro : touche verte « Macro ».
        command(&mut harness, "ClearAll");
        command(&mut harness, r#"Label Macro 1 "Ouverture""#);
        harness.key_press(egui::Key::Escape);
        harness.state_mut().console.set_executor_macro(3, Some(1)).unwrap();
        harness.run_steps(2);
        harness.get_by_label("Macro 1"); // sur la carte de l'executor, sous son nom
        harness.get(By::new().predicate(|n| n.description().as_deref() == Some("Lancer la macro 1 « Ouverture »"))).click();
        settle(&mut harness);
        assert_eq!(harness.state().console.selection().len(), 2, "la touche de l'executor a lancé la macro");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("macro-executor.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
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

        // Gélatines : nuancier Lee par défaut, recherche, puis Rosco.
        harness.get_by_label("Gélatines").click();
        harness.run_steps(2);
        harness.get_by_label("Lee 201 Full C.T. Blue");
        assert!(harness.query_by_label("Rosco R80 Primary Blue").is_none(), "Lee d'abord");
        harness.get_by_label("Lee 106 Primary Red").click();
        settle(&mut harness);
        assert_eq!((out(&harness, 4), out(&harness, 5), out(&harness, 6)), (240, 0, 0));
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let path = std::path::Path::new(&dir).join("gelatines.png");
            harness.render().expect("rendu").save(&path).expect("écriture de l'image");
        }
        harness.get_by_label("Rosco").click();
        harness.run_steps(2);
        harness.get_by_role_and_label(Role::TextInput, "Chercher").focus();
        harness.run_steps(1);
        harness.get_by_role_and_label(Role::TextInput, "Chercher").type_text("r80");
        harness.run_steps(2);
        assert!(harness.query_by_label("Rosco R83 Medium Blue").is_none(), "filtré");
        harness.get_by_label("Rosco R80 Primary Blue").click();
        settle(&mut harness);
        assert_eq!((out(&harness, 4), out(&harness, 5), out(&harness, 6)), (0, 80, 255));
    }
}
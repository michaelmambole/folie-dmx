//! Menu Fichier : nouveau, ouvrir, enregistrer, sauvegarde automatique et récupération.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Key, KeyboardShortcut, Modifiers, RichText, ViewportCommand};
use folie_show::Show;

use super::FolieApp;
use crate::storage::{Recovery, Storage};
use crate::theme;

/// Les modifications non enregistrées sont recopiées à cet intervalle.
const AUTOSAVE_EVERY: Duration = Duration::from_secs(30);

const SAVE: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::S);
const SAVE_AS: KeyboardShortcut =
    KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::S);
const OPEN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::O);
const NEW: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::N);

/// Action qui ferait perdre les modifications : on demande d'abord.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Pending {
    New,
    Open,
    Quit,
}

pub(super) struct FileState {
    pub(super) storage: Storage,
    pub(super) path: Option<PathBuf>,
    pub(super) dirty: bool,
    /// Show tel qu'il est sur le disque ; `None` s'il est inconnu (modifications récupérées).
    saved: Option<Show>,
    last_autosave: Instant,
    pub(super) pending: Option<Pending>,
    pub(super) recovery: Option<Recovery>,
    /// Compte rendu à afficher : titre et lignes (avertissements, erreur).
    report: Option<(String, Vec<String>)>,
    /// Fermeture déjà confirmée : ne plus l'intercepter.
    pub(super) closing: bool,
    title: String,
    /// Dernier enregistrement sur disque, pour le confirmer quelques secondes.
    saved_at: Option<Instant>,
}

impl FileState {
    pub(super) fn new(storage: Storage) -> Self {
        Self {
            storage,
            path: None,
            dirty: false,
            saved: Some(Show::default()),
            last_autosave: Instant::now(),
            pending: None,
            recovery: None,
            report: None,
            closing: false,
            title: String::new(),
            saved_at: None,
        }
    }

    /// Plus rien n'est à récupérer. Tant qu'on n'a pas répondu à la question de
    /// récupération, la copie de secours reste : seul « Ignorer » l'efface.
    fn clear_autosave(&self) {
        if self.recovery.is_none() {
            self.storage.clear_autosave();
        }
    }

    pub(super) fn show_name(&self) -> String {
        self.path
            .as_deref()
            .and_then(Path::file_name)
            .map_or_else(|| "Sans titre".into(), |n| n.to_string_lossy().into_owned())
    }
}

fn dialog() -> rfd::FileDialog {
    rfd::FileDialog::new().add_filter("Show Folie-DMX", &[folie_show::EXTENSION])
}

impl FolieApp {
    fn current_show(&self) -> Show {
        self.console.show.clone()
    }

    fn apply_show(&mut self, show: Show) {
        self.console.load_show(show);
        self.fixtures_view.form.first_id = self.console.show.patch.next_free_id().0;
        self.channel_labels = super::channel_labels(&self.console.show.patch);
    }

    /// Le show diffère-t-il de celui du disque ? Une modification défaite (ou un
    /// `mark_dirty` de trop) ne compte pas : rien à enregistrer ni à recopier.
    fn has_changes(&mut self) -> bool {
        if self.file.dirty && self.file.saved.as_ref() == Some(&self.console.show) {
            self.file.dirty = false;
            self.file.clear_autosave();
        }
        self.file.dirty
    }

    pub(super) fn mark_dirty(&mut self) {
        if !self.file.dirty {
            self.file.dirty = true;
            // Première modification : la prochaine copie de secours part dans 30 s.
            self.file.last_autosave = Instant::now();
        }
    }

    /// Au démarrage : propose de récupérer une session non enregistrée, sinon
    /// rouvre le dernier show.
    pub(super) fn startup(&mut self) {
        if let Some(recovery) = self.file.storage.recovery() {
            self.file.recovery = Some(recovery);
        } else if let Some(path) = self.file.storage.last_show() {
            self.open_path(&path);
        }
    }

    pub(super) fn open_path(&mut self, path: &Path) -> bool {
        match folie_show::load(path, &self.library) {
            Ok(loaded) => {
                self.file.saved = Some(loaded.show.clone());
                self.apply_show(loaded.show);
                self.file.path = Some(path.to_path_buf());
                self.file.dirty = false;
                self.file.storage.set_last_show(Some(path));
                self.file.clear_autosave();
                if !loaded.warnings.is_empty() {
                    let title = format!("{} ouvert avec des avertissements", self.file.show_name());
                    self.file.report = Some((title, loaded.warnings));
                }
                true
            }
            Err(err) => {
                let title = "Ouverture impossible".to_string();
                self.file.report = Some((title, vec![format!("{} : {err}", path.display())]));
                false
            }
        }
    }

    pub(super) fn save_to(&mut self, path: &Path) -> bool {
        let mut path = path.to_path_buf();
        if path.extension().is_none() {
            path.set_extension(folie_show::EXTENSION);
        }
        let show = self.current_show();
        match folie_show::save(&show, &path) {
            Ok(()) => {
                self.file.saved = Some(show);
                self.file.storage.set_last_show(Some(&path));
                self.file.clear_autosave();
                self.file.path = Some(path);
                self.file.dirty = false;
                self.file.saved_at = Some(Instant::now());
                true
            }
            Err(err) => {
                self.file.report = Some(("Enregistrement impossible".into(), vec![err.to_string()]));
                false
            }
        }
    }

    fn save(&mut self) -> bool {
        match self.file.path.clone() {
            Some(path) => self.save_to(&path),
            None => self.save_as(),
        }
    }

    fn save_as(&mut self) -> bool {
        let name = match &self.file.path {
            Some(_) => self.file.show_name(),
            None => format!("show.{}", folie_show::EXTENSION),
        };
        match dialog().set_file_name(name).save_file() {
            Some(path) => self.save_to(&path),
            None => false,
        }
    }

    fn new_show(&mut self) {
        self.apply_show(Show::default());
        self.file.saved = Some(Show::default());
        self.file.path = None;
        self.file.dirty = false;
        self.file.storage.set_last_show(None);
        self.file.clear_autosave();
    }

    /// Lance l'action, ou demande d'abord quoi faire des modifications en cours.
    pub(super) fn request(&mut self, ctx: &egui::Context, action: Pending) {
        if self.has_changes() {
            self.file.pending = Some(action);
        } else {
            self.perform(ctx, action);
        }
    }

    fn perform(&mut self, ctx: &egui::Context, action: Pending) {
        match action {
            Pending::New => self.new_show(),
            Pending::Open => {
                if let Some(path) = dialog().pick_file() {
                    self.open_path(&path);
                }
            }
            Pending::Quit => {
                // Rien n'est à récupérer : l'utilisateur a choisi.
                self.file.clear_autosave();
                self.file.closing = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    pub(super) fn file_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.menu_button("Fichier", |ui| {
            let item = |ui: &mut egui::Ui, text: &str, shortcut: &KeyboardShortcut| {
                ui.add(egui::Button::new(text).shortcut_text(ui.ctx().format_shortcut(shortcut))).clicked()
            };
            if item(ui, "Nouveau", &NEW) {
                self.request(&ctx, Pending::New);
            }
            if item(ui, "Ouvrir…", &OPEN) {
                self.request(&ctx, Pending::Open);
            }
            ui.separator();
            if item(ui, "Enregistrer", &SAVE) {
                self.save();
            }
            if item(ui, "Enregistrer sous…", &SAVE_AS) {
                self.save_as();
            }
            ui.separator();
            if ui.button("À propos de Folie-DMX").clicked() {
                self.about_open = true;
            }
        });
    }

    pub(super) fn file_shortcuts(&mut self, ctx: &egui::Context) {
        if self.file.recovery.is_some() {
            // D'abord répondre à la question de récupération.
            return;
        }
        // Le plus spécifique d'abord : Cmd+Maj+S ne doit pas déclencher Cmd+S.
        if ctx.input_mut(|i| i.consume_shortcut(&SAVE_AS)) {
            self.save_as();
        } else if ctx.input_mut(|i| i.consume_shortcut(&SAVE)) {
            self.save();
        } else if ctx.input_mut(|i| i.consume_shortcut(&OPEN)) {
            self.request(ctx, Pending::Open);
        } else if ctx.input_mut(|i| i.consume_shortcut(&NEW)) {
            self.request(ctx, Pending::New);
        }
    }

    /// Nom du show dans la barre du haut, et son état dessous : « Non enregistré » en ambre
    /// (un clic enregistre), ou « Enregistré », plus visible quelques secondes après.
    pub(super) fn file_status(&mut self, ui: &mut egui::Ui) {
        let shortcut = ui.ctx().format_shortcut(&SAVE);
        let name = self.file.show_name();
        let path = self.file.path.as_deref().map(|p| p.display().to_string()).unwrap_or_else(|| "Jamais enregistré".into());
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.add(egui::Label::new(RichText::new(name).family(theme::medium()).size(13.0)).truncate()).on_hover_text(path);
            if self.file.dirty {
                let text = RichText::new("Non enregistré").size(11.0).color(theme::EXECUTOR);
                let button = egui::Button::new(text).frame(false);
                if ui.add(button).on_hover_text(format!("Enregistrer le show ({shortcut})")).clicked() {
                    self.save();
                }
            } else if self.file.path.is_some() {
                const CONFIRM: Duration = Duration::from_secs(3);
                let recent = self.file.saved_at.is_some_and(|t| t.elapsed() < CONFIRM);
                ui.label(RichText::new("Enregistré").size(11.0).color(if recent { theme::GO } else { theme::MUTED }));
                if recent {
                    ui.ctx().request_repaint_after(CONFIRM);
                }
            } else {
                ui.label(RichText::new("Nouveau show").size(11.0).color(theme::MUTED));
            }
        });
    }

    /// À appeler à chaque image : copie de secours, fermeture, titre de la fenêtre.
    pub(super) fn file_tick(&mut self, ctx: &egui::Context) {
        // Pas de copie pendant la question de récupération : elle écraserait celle à récupérer.
        if self.file.dirty
            && self.file.recovery.is_none()
            && self.file.last_autosave.elapsed() >= AUTOSAVE_EVERY
            && self.has_changes()
        {
            self.autosave_now();
        }

        if ctx.input(|i| i.viewport().close_requested()) && !self.file.closing {
            if self.has_changes() {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                self.file.pending = Some(Pending::Quit);
            } else {
                self.file.clear_autosave();
            }
        }

        let title = format!(
            "Folie-DMX — {}{}",
            self.file.show_name(),
            if self.file.dirty { " •" } else { "" }
        );
        if title != self.file.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.file.title = title;
        }
    }

    pub(super) fn autosave_now(&mut self) {
        self.file.last_autosave = Instant::now();
        let show = self.current_show();
        if let Err(err) = self.file.storage.write_autosave(&show, self.file.path.as_deref()) {
            log::error!("sauvegarde automatique impossible : {err}");
        }
    }

    /// À l'arrêt du programme, quelle qu'en soit la cause. « Quitter » du Dock ou la
    /// fermeture de session ne passent pas par la confirmation : les modifications
    /// sont alors recopiées, pour être proposées au prochain lancement.
    pub(super) fn file_exit(&mut self) {
        if self.file.closing || self.file.recovery.is_some() {
            return;
        }
        if self.has_changes() {
            self.autosave_now();
        } else {
            self.file.clear_autosave();
        }
    }

    #[cfg(test)]
    pub(super) fn autosave_due(&mut self) {
        self.file.last_autosave = Instant::now() - AUTOSAVE_EVERY;
    }

    pub(super) fn recover(&mut self) {
        let Some(recovery) = self.file.recovery.take() else { return };
        match folie_show::load(&recovery.path, &self.library) {
            Ok(loaded) => {
                self.apply_show(loaded.show);
                // Les modifications récupérées restent à enregistrer, dans le show d'origine.
                self.file.path = recovery.original;
                self.file.saved = None;
                self.file.dirty = true;
                self.file.last_autosave = Instant::now();
            }
            Err(err) => {
                self.file.report = Some(("Récupération impossible".into(), vec![err.to_string()]));
                if let Some(path) = self.file.storage.last_show() {
                    self.open_path(&path);
                }
            }
        }
    }

    pub(super) fn file_modals(&mut self, ctx: &egui::Context) {
        if let Some(recovery) = &self.file.recovery {
            let original = recovery
                .original
                .as_deref()
                .and_then(Path::file_name)
                .map_or_else(|| "un show sans titre".into(), |n| format!("« {} »", n.to_string_lossy()));
            let mut choice = None;
            // Échap et un clic à côté ne font rien : il faut choisir, rien ne se perd par mégarde.
            egui::Modal::new(egui::Id::unique("recovery")).show(ctx, |ui| {
                ui.heading("Session non enregistrée");
                ui.label(format!(
                    "Folie-DMX s'est fermé sans enregistrer des modifications de {original}."
                ));
                ui.label(RichText::new("Ignorer les supprime définitivement.").color(theme::MUTED));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Récupérer").strong()).clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Ignorer").clicked() {
                        choice = Some(false);
                    }
                });
            });
            match choice {
                Some(true) => self.recover(),
                Some(false) => {
                    self.file.recovery = None;
                    self.file.clear_autosave();
                    if let Some(path) = self.file.storage.last_show() {
                        self.open_path(&path);
                    }
                }
                None => {}
            }
        }

        if let Some(action) = self.file.pending {
            let mut choice = None;
            let response = egui::Modal::new(egui::Id::unique("unsaved")).show(ctx, |ui| {
                ui.heading("Modifications non enregistrées");
                ui.label(format!("Enregistrer les modifications de « {} » ?", self.file.show_name()));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Enregistrer").strong()).clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Ne pas enregistrer").clicked() {
                        choice = Some(false);
                    }
                    if ui.button("Annuler").clicked() {
                        self.file.pending = None;
                    }
                });
            });
            if response.should_close() {
                self.file.pending = None;
            }
            match choice {
                Some(true) => {
                    self.file.pending = None;
                    if self.save() {
                        self.perform(ctx, action);
                    }
                }
                Some(false) => {
                    // Le show ne redevient « propre » que si l'action aboutit.
                    self.file.pending = None;
                    self.perform(ctx, action);
                }
                None => {}
            }
        }

        if let Some((title, lines)) = &self.file.report {
            let mut close = false;
            let response = egui::Modal::new(egui::Id::unique("report")).show(ctx, |ui| {
                ui.heading(title);
                for line in lines {
                    ui.label(line);
                }
                ui.add_space(8.0);
                close = ui.button("OK").clicked();
            });
            if close || response.should_close() {
                self.file.report = None;
            }
        }
    }
}

/// Sur macOS, ⌘Q et « Quitter Folie-DMX » appellent `terminate:` : l'application
/// s'arrête sans `close_requested`, donc sans proposer d'enregistrer. Ils font
/// plutôt comme le bouton rouge de la fenêtre principale, qui demande.
#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn route_quit_to_main_window(cc: &eframe::CreationContext<'_>) {
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::{NSApplication, NSView};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let Some(mtm) = MainThreadMarker::new() else { return };
    let Ok(handle) = cc.window_handle() else { return };
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else { return };
    // SAFETY : winit donne la vue de la fenêtre principale, qui vit autant que l'application.
    let view: &NSView = unsafe { handle.ns_view.cast().as_ref() };
    let Some(window) = view.window() else { return };
    let Some(menu) = NSApplication::sharedApplication(mtm).mainMenu() else { return };
    let target: &AnyObject = &window;
    for submenu in menu.itemArray().iter().filter_map(|item| item.submenu()) {
        for item in submenu.itemArray().iter().filter(|item| item.action() == Some(sel!(terminate:))) {
            // SAFETY : la fenêtre répond à `performClose:` ; la cible d'un menu n'est pas retenue,
            // mais winit garde la fenêtre principale jusqu'à la fin.
            unsafe {
                item.setTarget(Some(target));
                item.setAction(Some(sel!(performClose:)));
            }
        }
    }
}

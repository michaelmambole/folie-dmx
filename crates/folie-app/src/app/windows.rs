//! Fenêtres séparées (Patch, Groupes, Presets, Sortie DMX), à la manière d'une
//! station de travail audio : chacune est une vraie fenêtre du système.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Key, KeyboardShortcut, Modifiers, Pos2, Rect, Vec2, ViewportBuilder, ViewportCommand, ViewportId,
};

use super::FolieApp;
use crate::screens;
use crate::workspace::{LAYOUT_SLOTS, Layout, Placement, WindowKind, Workspace, layout_key};
use crate::{effects_ui, faders, monitor, playback_ui, theme, tracking_ui};

/// Les déplacements de fenêtres sont enregistrés au plus à cette fréquence.
const SAVE_WORKSPACE_EVERY: Duration = Duration::from_secs(5);

pub(super) struct WindowsState {
    pub(super) workspace: Workspace,
    /// Placement demandé à l'ouverture : il sert à construire la fenêtre et ne
    /// suit pas ses déplacements (egui repositionnerait sinon la fenêtre en boucle).
    spawn: BTreeMap<WindowKind, Placement>,
    saved: Workspace,
    last_save: Instant,
    /// Aucun espace de travail enregistré : la disposition de départ reste à calculer.
    first_launch: bool,
    /// Fenêtres déjà créées : une fenêtre qui apparaît passe au premier plan.
    created: BTreeSet<WindowKind>,
    pub(super) layouts: BTreeMap<u8, Layout>,
    /// Dernière disposition rappelée ou enregistrée, proposée à la mise à jour.
    pub(super) current_layout: Option<u8>,
    /// Placement à appliquer à la fenêtre principale (rappel d'une disposition).
    pending_main: Option<Placement>,
    /// Boîte « Enregistrer la disposition » ouverte : (case, nom).
    pub(super) save_dialog: Option<(u8, String)>,
}

impl WindowsState {
    pub(super) fn new(workspace: Option<Workspace>, layouts: BTreeMap<u8, Layout>) -> Self {
        let first_launch = workspace.is_none();
        let workspace = workspace.unwrap_or_else(Workspace::first_launch);
        let mut state = Self {
            saved: workspace.clone(),
            workspace: Workspace::default(),
            spawn: BTreeMap::new(),
            last_save: Instant::now(),
            first_launch,
            created: BTreeSet::new(),
            layouts,
            current_layout: None,
            pending_main: None,
            save_dialog: None,
        };
        state.use_workspace(workspace, &screens::displays());
        state.pending_main = None; // la fenêtre principale est déjà placée au lancement
        state
    }

    /// Adopte un espace de travail, en ramenant les fenêtres dont l'écran est absent.
    fn use_workspace(&mut self, mut workspace: Workspace, displays: &[Rect]) {
        for (index, window) in workspace.windows.iter_mut().enumerate() {
            window.placement = screens::rescue(window.placement, displays, index + 1);
        }
        workspace.main = screens::rescue(workspace.main, displays, 0);
        self.spawn = workspace.windows.iter().map(|w| (w.kind, w.placement)).collect();
        self.pending_main = Some(workspace.main);
        self.workspace = workspace;
    }

    pub(super) fn recall(&mut self, slot: u8, displays: &[Rect]) -> Result<String, String> {
        let layout = self.layouts.get(&slot).cloned().ok_or_else(|| format!("aucune disposition en case {slot}"))?;
        self.use_workspace(layout.workspace, displays);
        self.first_launch = false;
        self.current_layout = Some(slot);
        Ok(format!("Disposition {slot} « {} » rappelée", layout.name))
    }

    pub(super) fn store(&mut self, slot: u8, name: String) -> String {
        let message = match self.layouts.get(&slot) {
            Some(old) => format!("Disposition {slot} « {name} » remplace « {} »", old.name),
            None => format!("Disposition {slot} « {name} » enregistrée"),
        };
        self.layouts.insert(slot, Layout { name, workspace: self.workspace.clone() });
        self.current_layout = Some(slot);
        message
    }

    /// Disposition de départ : le Programmer à gauche, les fenêtres en colonne à droite.
    fn first_layout(&mut self, ctx: &egui::Context, monitor: Vec2) {
        const MARGIN: f32 = 12.0;
        const TOP: f32 = 40.0; // sous la barre de menus
        const TITLE: f32 = 28.0;
        let column = WindowKind::Patch.default_size().x;
        let height = monitor.y - TOP - MARGIN;
        let main_width = (monitor.x - column - 3.0 * MARGIN).max(700.0);
        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(MARGIN, TOP)));
        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(main_width, height - TITLE)));

        let x = MARGIN + main_width + MARGIN;
        let open: Vec<WindowKind> = self.workspace.windows.iter().map(|w| w.kind).collect();
        let share = (height - open.len() as f32 * (TITLE + MARGIN)) / open.len().max(1) as f32;
        let mut y = TOP;
        for kind in open {
            // Le patch prend plus de place que les pools.
            let h = if kind == WindowKind::Patch { share * 1.4 } else { share * 0.8 };
            self.spawn.insert(kind, Placement::at_size(egui::pos2(x, y), egui::vec2(column, h)));
            y += h + TITLE + MARGIN;
        }
        self.first_launch = false;
    }

    pub(super) fn toggle(&mut self, kind: WindowKind) {
        if self.workspace.is_open(kind) {
            self.workspace.close(kind);
        } else {
            self.open(kind);
        }
    }

    pub(super) fn open(&mut self, kind: WindowKind) {
        if !self.workspace.is_open(kind) {
            self.workspace.open(kind);
            self.spawn.entry(kind).or_default();
        }
    }
}

/// Place les fenêtres sans position connue : en colonne à droite de la fenêtre
/// principale si l'écran le permet, sinon en cascade par-dessus.
struct AutoPlacer {
    main: Rect,
    monitor: Vec2,
    next_y: f32,
    cascade: f32,
}

impl AutoPlacer {
    const GAP: f32 = 12.0;

    fn place(&mut self, size: Vec2) -> Pos2 {
        let x = self.main.max.x + Self::GAP;
        if x + size.x <= self.monitor.x && self.next_y + size.y <= self.monitor.y {
            let position = egui::pos2(x, self.next_y);
            self.next_y += size.y + Self::GAP + 28.0; // barre de titre
            return position;
        }
        self.cascade += 40.0;
        self.main.min + egui::vec2(self.cascade, self.cascade)
    }
}

fn viewport_id(kind: WindowKind) -> ViewportId {
    ViewportId::from_hash_of(("folie-window", kind))
}

fn builder(kind: WindowKind, placement: &Placement) -> ViewportBuilder {
    let mut builder = ViewportBuilder::default()
        .with_title(format!("{} — Folie-DMX", kind.title()))
        .with_inner_size(placement.size().unwrap_or_else(|| kind.default_size()))
        .with_min_inner_size([280.0, 200.0])
        .with_fullscreen(placement.fullscreen);
    if let Some(position) = placement.position() {
        builder = builder.with_position(position);
    }
    builder
}

/// Raccourci sur une touche de chiffre, reconnue aussi par sa place sur le clavier :
/// en AZERTY, la touche 4 donne « ' » et la 8 « ! », qu'egui lit comme une apostrophe
/// et un point d'exclamation, jamais comme un 4 ou un 8.
fn consume_digit(ctx: &egui::Context, modifiers: Modifiers, digit: Key) -> bool {
    ctx.input_mut(|i| {
        let before = i.events.len();
        i.events.retain(|event| {
            !matches!(event,
                egui::Event::Key { key, physical_key, modifiers: m, pressed: true, .. }
                if (*key == digit || *physical_key == Some(digit)) && m.matches_logically(modifiers))
        });
        i.events.len() != before
    })
}

impl FolieApp {
    /// Raccourcis valables dans toutes les fenêtres : chaque fenêtre système
    /// reçoit ses propres touches, il faut donc les traiter partout.
    pub(super) fn global_shortcuts(&mut self, ctx: &egui::Context) {
        self.file_shortcuts(ctx);
        // ⌥⌘1 avant ⌘1 : egui accepte une touche Option en trop, ⌘1 avalerait ⌥⌘1.
        for slot in LAYOUT_SLOTS {
            if consume_digit(ctx, Modifiers::COMMAND | Modifiers::ALT, layout_key(slot)) {
                self.recall_layout(slot);
            }
        }
        for kind in WindowKind::ALL {
            // Bascule, comme dans le menu Fenêtres : ouvre si fermée, ferme si ouverte.
            if consume_digit(ctx, Modifiers::COMMAND, kind.key()) {
                self.windows.toggle(kind);
            }
        }
        if self.store_armed && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.store_armed = false;
        }
        if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::B)) {
            self.toggle_blackout();
        }
        // S : la touche Store, comme sur grandMA2 onPC.
        if !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::S)) {
            self.store_armed = !self.store_armed;
        }
    }

    fn recall_layout(&mut self, slot: u8) {
        let feedback = self.windows.recall(slot, &screens::displays());
        self.command_line.feedback = Some((feedback.is_ok(), feedback.unwrap_or_else(|e| e)));
    }

    fn store_layout(&mut self, slot: u8, name: String) {
        let message = self.windows.store(slot, name);
        self.file.storage.set_layouts(&self.windows.layouts);
        self.command_line.feedback = Some((true, message));
    }

    pub(super) fn windows_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Fenêtres", |ui| {
            for kind in WindowKind::ALL {
                let shortcut = ui.ctx().format_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, kind.key()));
                let open = self.windows.workspace.is_open(kind);
                let button = egui::Button::selectable(open, kind.title()).shortcut_text(shortcut);
                if ui.add(button).clicked() {
                    self.windows.toggle(kind);
                }
            }

            ui.separator();
            ui.label(egui::RichText::new("Dispositions").weak());
            let layouts: Vec<(u8, String)> =
                self.windows.layouts.iter().map(|(slot, l)| (*slot, l.name.clone())).collect();
            for (slot, name) in &layouts {
                let shortcut = KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::ALT, layout_key(*slot));
                let current = self.windows.current_layout == Some(*slot);
                let button = egui::Button::selectable(current, format!("{slot}. {name}"))
                    .shortcut_text(ui.ctx().format_shortcut(&shortcut));
                if ui.add(button).clicked() {
                    self.recall_layout(*slot);
                }
            }
            if let Some(slot) = self.windows.current_layout
                && let Some(name) = self.windows.layouts.get(&slot).map(|l| l.name.clone())
                && ui.button(format!("Mettre à jour « {name} »")).clicked()
            {
                self.store_layout(slot, name);
            }
            if ui.button("Enregistrer la disposition…").clicked() {
                let free = LAYOUT_SLOTS.clone().find(|s| !self.windows.layouts.contains_key(s)).unwrap_or(1);
                self.windows.save_dialog = Some((free, String::new()));
            }
            if !layouts.is_empty() {
                ui.menu_button("Supprimer une disposition", |ui| {
                    for (slot, name) in &layouts {
                        if ui.button(format!("{slot}. {name}")).clicked() {
                            self.windows.layouts.remove(slot);
                            if self.windows.current_layout == Some(*slot) {
                                self.windows.current_layout = None;
                            }
                            self.file.storage.set_layouts(&self.windows.layouts);
                        }
                    }
                });
            }
        });
    }

    /// Boîte « Enregistrer la disposition » : case de 1 à 9 et nom.
    pub(super) fn layout_dialog(&mut self, ctx: &egui::Context) {
        let Some((mut slot, mut name)) = self.windows.save_dialog.take() else { return };
        let mut done = None;
        let response = egui::Modal::new(egui::Id::unique("layout_dialog")).show(ctx, |ui| {
            ui.heading("Enregistrer la disposition");
            ui.label("Fenêtres ouvertes, positions et tailles.");
            ui.add_space(6.0);
            egui::Grid::new("layout_form").num_columns(2).show(ui, |ui| {
                let label = ui.label("Case");
                ui.add(egui::DragValue::new(&mut slot).range(LAYOUT_SLOTS)).labelled_by(label.id);
                ui.end_row();
                let label = ui.label("Nom");
                let edit = ui.add(egui::TextEdit::singleline(&mut name).hint_text("Programmation")).labelled_by(label.id);
                if edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    done = Some(true);
                }
                ui.end_row();
            });
            if let Some(old) = self.windows.layouts.get(&slot) {
                ui.colored_label(egui::Color32::YELLOW, format!("Remplacera « {} »", old.name));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(egui::RichText::new("Enregistrer").strong()).clicked() {
                    done = Some(true);
                }
                if ui.button("Annuler").clicked() {
                    done = Some(false);
                }
            });
        });
        if response.should_close() && done.is_none() {
            done = Some(false);
        }
        match done {
            Some(true) => {
                let name = if name.trim().is_empty() { format!("Disposition {slot}") } else { name.trim().to_string() };
                self.store_layout(slot, name);
            }
            Some(false) => {}
            None => self.windows.save_dialog = Some((slot, name)),
        }
    }

    /// Affiche toutes les fenêtres ouvertes et enregistre l'espace de travail s'il a changé.
    pub(super) fn show_windows(&mut self, ctx: &egui::Context) {
        if let Some(main) = self.windows.pending_main.take() {
            if let Some(position) = main.position() {
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(position));
            }
            if let Some(size) = main.size() {
                ctx.send_viewport_cmd(ViewportCommand::InnerSize(size));
            }
            ctx.send_viewport_cmd(ViewportCommand::Fullscreen(main.fullscreen));
            self.windows.workspace.main = main;
        } else if let Some(main) = Placement::capture(ctx) {
            self.windows.workspace.main = main;
        }
        let main = ctx.input(|i| i.viewport().outer_rect);
        let monitor = ctx.input(|i| i.viewport().monitor_size);
        let mut placer = main.zip(monitor).map(|(main, monitor)| AutoPlacer {
            main,
            monitor,
            next_y: main.min.y,
            cascade: 0.0,
        });
        if self.windows.first_launch && !ctx.embed_viewports() {
            match monitor {
                Some(monitor) => self.windows.first_layout(ctx, monitor),
                None => return, // taille d'écran encore inconnue : on attend l'image suivante
            }
        }
        let kinds: Vec<WindowKind> = self.windows.workspace.windows.iter().map(|w| w.kind).collect();
        self.windows.created.retain(|k| kinds.contains(k));
        for kind in kinds {
            let mut spawn = self.windows.spawn.get(&kind).copied().unwrap_or_default();
            if spawn.position.is_none() && !ctx.embed_viewports() {
                // On attend de connaître la fenêtre principale pour placer la nouvelle à côté.
                let Some(placer) = placer.as_mut() else { continue };
                let size = spawn.size().unwrap_or_else(|| kind.default_size());
                spawn = Placement { position: Some(placer.place(size).into()), size: Some(size.into()), ..spawn };
                self.windows.spawn.insert(kind, spawn);
            }
            let id = viewport_id(kind);
            if ctx.embed_viewports() {
                // Pas de fenêtres système (tests, plateformes sans multi-fenêtres) :
                // fenêtres internes, placées comme demandé.
                let size = spawn.size().unwrap_or_else(|| kind.default_size());
                let mut window = egui::Window::new(kind.title()).id(egui::Id::unique(id)).default_size(size);
                if let Some(position) = spawn.position() {
                    window = window.default_pos(position);
                }
                window.show(ctx, |ui| self.window_contents(ui, kind));
            } else {
                if self.windows.created.insert(kind) {
                    ctx.send_viewport_cmd_to(id, ViewportCommand::Focus);
                }
                ctx.show_viewport_immediate(id, builder(kind, &spawn), |ui, _| {
                    let ctx = ui.ctx().clone();
                    self.global_shortcuts(&ctx);
                    if ctx.input(|i| i.viewport().close_requested()) {
                        self.windows.workspace.close(kind);
                        return;
                    }
                    if let Some(placement) = Placement::capture(&ctx)
                        && let Some(window) = self.windows.workspace.window_mut(kind)
                    {
                        window.placement = placement;
                    }
                    egui::CentralPanel::default().show(ui, |ui| self.window_contents(ui, kind));
                });
            }
        }
        self.save_workspace_if_changed(false);
    }

    pub(super) fn save_workspace_if_changed(&mut self, now: bool) {
        let state = &mut self.windows;
        if state.workspace != state.saved && (now || state.last_save.elapsed() >= SAVE_WORKSPACE_EVERY) {
            self.file.storage.set_workspace(&state.workspace);
            state.saved = state.workspace.clone();
            state.last_save = Instant::now();
        }
    }

    fn window_contents(&mut self, ui: &mut egui::Ui, kind: WindowKind) {
        match kind {
            WindowKind::Patch => {
                if self.fixtures_view.patch_panel(ui, &self.library, &mut self.console) {
                    self.patch_changed();
                    self.mark_dirty();
                }
            }
            WindowKind::Groups => {
                let result = egui::ScrollArea::vertical()
                    .show(ui, |ui| self.pools.groups(ui, &mut self.console, &mut self.store_armed))
                    .inner;
                self.report(result);
            }
            WindowKind::Presets => {
                let Some(window) = self.windows.workspace.window_mut(kind) else { return };
                let mut family = window.family();
                let result = egui::ScrollArea::vertical()
                    .show(ui, |ui| self.pools.presets(ui, &mut self.console, &self.snapshot, &mut family, &mut self.store_armed))
                    .inner;
                if let Some(window) = self.windows.workspace.window_mut(kind) {
                    window.preset_family = Some(family.key().to_string());
                }
                self.report(result);
            }
            WindowKind::Output => self.output_contents(ui),
            WindowKind::Cues => {
                let result = playback_ui::cues_window(ui, &mut self.console, &self.snapshot, &mut self.store_armed);
                self.report(result);
            }
            WindowKind::Executors => {
                let result = playback_ui::executors_window(
                    ui,
                    &mut self.console,
                    &self.snapshot,
                    &mut self.store_armed,
                    &mut self.flash_held,
                );
                self.report(result);
            }
            WindowKind::Effects => {
                let result = effects_ui::effects_window(ui, &mut self.console);
                self.report(result);
            }
            WindowKind::Tracking => {
                let result = tracking_ui::tracking_window(ui, &mut self.console, &self.snapshot, self.display);
                self.report(result);
            }
        }
    }

    fn output_contents(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("faders").resizable(false).show(ui, |ui| {
            ui.add_space(6.0);
            faders::show(
                ui,
                &mut self.fader_page,
                &self.snapshot.channel_overrides[0],
                &self.snapshot.output[0],
                self.display,
                self.selected,
                self.console.engine(),
            );
            ui.add_space(6.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            self.output_status(ui);
            ui.add_space(6.0);
            let starts: BTreeSet<u16> =
                self.console.show.patch.fixtures().iter().filter(|f| f.universe == 0).map(|f| f.address).collect();
            let marks = monitor::Marks {
                labels: &self.channel_labels,
                starts: &starts,
                overrides: &self.snapshot.channel_overrides[0],
            };
            let output = &self.snapshot.output[0];
            let clicked = egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| monitor::show(ui, output, &marks, self.display, self.selected))
                .inner;
            if let Some(channel) = clicked {
                self.selected = Some(channel);
                self.fader_page = (channel as usize - 1) / faders::FADERS_PER_PAGE;
            }
        });
    }

    /// Carte d'état de la sortie : interface, fréquence, erreurs, légende de la grille.
    fn output_status(&mut self, ui: &mut egui::Ui) {
        let snapshot = self.snapshot.clone();
        let mut artnet = self.artnet.clone();
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::section(ui, "Sortie");
                ui.label(snapshot.output_names.join(", "));
                ui.label(egui::RichText::new(format!("{:.1} Hz", self.rate.hz)).monospace().color(theme::MUTED));
                let errors = egui::RichText::new(format!("erreurs {}", snapshot.output_errors));
                ui.label(if snapshot.output_errors > 0 { errors.color(theme::DANGER) } else { errors.color(theme::MUTED) });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::chip(ui, theme::DANGER, "forcé");
                    theme::chip(ui, theme::TEXT, "niveau");
                    theme::chip(ui, monitor::PATCHED, "patché");
                });
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                theme::section(ui, "Art-Net");
                ui.checkbox(&mut artnet.enabled, "Actif").on_hover_text("Envoyer aussi l'univers en Art-Net, en plus de la sortie choisie");
                let label = ui.label(egui::RichText::new("Destination").color(theme::MUTED));
                let id = egui::Id::unique("artnet-destination");
                let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| artnet.destination.clone());
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut text).desired_width(130.0).font(egui::TextStyle::Monospace))
                    .labelled_by(label.id)
                    .on_hover_text("Adresse d'un node, ou de diffusion : 255.255.255.255, 2.255.255.255…");
                if edit.lost_focus() {
                    artnet.destination = text.trim().to_string();
                    ui.data_mut(|d| d.remove::<String>(id));
                } else if edit.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, text));
                }
                let label = ui.label(egui::RichText::new("Univers").color(theme::MUTED));
                ui.add(egui::DragValue::new(&mut artnet.universe).range(0..=folie_dmx::artnet::MAX_UNIVERSE)).labelled_by(label.id);
                let (net, subnet, universe) = folie_dmx::artnet::split_universe(artnet.universe);
                let detail = format!("net {net} · sous-réseau {subnet} · univers {universe}");
                ui.label(egui::RichText::new(detail).size(11.5).color(theme::MUTED));
            });
        });
        if artnet != self.artnet {
            self.artnet = artnet;
            self.artnet_changed();
        }
    }

    /// Affiche le résultat d'une action dans la ligne de commande ; le show est
    /// marqué modifié si l'action l'a changé.
    fn report(&mut self, result: Option<Result<folie_control::Outcome, folie_control::CommandError>>) {
        if let Some(result) = result
            && self.command_line.report(result)
        {
            self.mark_dirty();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_and_recall_layouts() {
        let mut state = WindowsState::new(Some(Workspace::first_launch()), BTreeMap::new());
        state.store(1, "Programmation".into());

        state.workspace.close(WindowKind::Groups);
        state.open(WindowKind::Output);
        // La sortie DMX était sur le vidéoprojecteur, à droite du portable.
        state.workspace.window_mut(WindowKind::Output).unwrap().placement =
            Placement::at_size(egui::pos2(3000.0, 100.0), egui::vec2(800.0, 600.0));
        assert!(state.store(2, "Spectacle".into()).contains("enregistrée"));
        assert!(state.store(2, "Spectacle".into()).contains("remplace"));

        let laptop = Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1440.0, 900.0));
        state.recall(1, &[laptop]).unwrap();
        let open: Vec<WindowKind> = state.workspace.windows.iter().map(|w| w.kind).collect();
        assert_eq!(open, [WindowKind::Patch, WindowKind::Groups, WindowKind::Presets]);
        assert_eq!(state.current_layout, Some(1));

        // Vidéoprojecteur débranché : la fenêtre revient sur le portable.
        state.recall(2, &[laptop]).unwrap();
        let output = state.spawn[&WindowKind::Output].position().unwrap();
        assert!(laptop.contains(output), "{output:?}");

        assert!(state.recall(7, &[laptop]).is_err());
    }
}

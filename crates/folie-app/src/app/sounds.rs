//! Fenêtre Sons : le pool de sons du show (import, réglages, forme d'onde avec les points
//! d'entrée et de sortie, écoute) et la sortie audio.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use folie_control::timecode::TimeSource;
use folie_show::media::Sound;

use super::FolieApp;
use crate::playback_sound::{ClipState, PEAK_BLOCK};
use crate::theme;

/// État de la fenêtre Sons (pas enregistré).
#[derive(Default)]
pub struct SoundsView {
    pub selected: Option<u32>,
    devices: Option<Vec<String>>,
    /// Crêtes affichées, qui retombent doucement.
    meter: [f32; 2],
    /// Sons en lecture à l'image précédente.
    playing: Vec<u32>,
    /// Partie de la forme d'onde affichée (zoom).
    pub zoom: Option<WaveZoom>,
}

/// Partie affichée de la forme d'onde d'un son : à partir de `from`, sur `span` secondes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WaveZoom {
    pub sound: u32,
    pub from: f64,
    pub span: f64,
}

impl WaveZoom {
    /// Plus petite durée affichée.
    const MIN_SPAN: f64 = 0.02;

    fn whole(sound: u32, duration: f64) -> Self {
        Self { sound, from: 0.0, span: duration }
    }

    /// Zoom de `factor` (> 1 : on s'approche) autour du temps `at`, qui reste sous la souris.
    fn zoom(&mut self, factor: f64, at: f64, duration: f64) {
        let span = (self.span / factor).clamp(Self::MIN_SPAN.min(duration), duration);
        self.from = at - (at - self.from) * span / self.span;
        self.span = span;
        self.clamp(duration);
    }

    /// Montre la plage `from..to`, avec un peu de marge.
    fn frame(&mut self, from: f64, to: f64, duration: f64) {
        let margin = (to - from) * 0.05;
        self.span = (to - from + 2.0 * margin).clamp(Self::MIN_SPAN.min(duration), duration);
        self.from = from - margin;
        self.clamp(duration);
    }

    fn clamp(&mut self, duration: f64) {
        self.span = self.span.min(duration);
        self.from = self.from.clamp(0.0, (duration - self.span).max(0.0));
    }

    fn is_whole(&self, duration: f64) -> bool {
        self.span >= duration - 1e-9
    }
}

/// Durée lisible : « 2:31.4 ».
fn duration_text(seconds: f64) -> String {
    let tenths = (seconds.max(0.0) * 10.0).round() as u64;
    format!("{}:{:02}.{}", tenths / 600, (tenths / 10) % 60, tenths % 10)
}

/// Fichiers audio reconnus, pour le sélecteur et le glisser-déposer.
const AUDIO_EXTENSIONS: [&str; 9] = ["wav", "aif", "aiff", "flac", "mp3", "ogg", "m4a", "aac", "mp4"];

fn is_audio(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Fichiers audio parmi ceux déposés : un dossier donne ses sons (sous-dossiers compris),
/// par ordre alphabétique. Rend aussi le nombre de fichiers laissés de côté.
pub(crate) fn audio_files(dropped: &[PathBuf]) -> (Vec<PathBuf>, usize) {
    fn walk(path: &std::path::Path, found: &mut Vec<PathBuf>, ignored: &mut usize, depth: usize) {
        if path.is_dir() {
            let Ok(entries) = std::fs::read_dir(path) else { return };
            let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            children.sort();
            for child in children {
                // Fichiers cachés (.DS_Store…) et dossiers trop profonds : ignorés sans bruit.
                if child.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) || depth > 8 {
                    continue;
                }
                walk(&child, found, ignored, depth + 1);
            }
        } else if is_audio(path) {
            found.push(path.to_path_buf());
        } else {
            *ignored += 1;
        }
    }
    let (mut found, mut ignored) = (Vec::new(), 0);
    for path in dropped {
        walk(path, &mut found, &mut ignored, 0);
    }
    (found, ignored)
}

impl FolieApp {
    /// Lecture : demandes de la console passées à la carte son, fichiers chargés.
    pub(super) fn sound_playback_tick(&mut self, ctx: &egui::Context) {
        let requests = self.console.take_sound_requests();
        for problem in self.playback.handle(requests, &self.console.show.sounds) {
            self.command_line.feedback = Some((false, problem));
        }
        if self.playback.is_open() {
            self.playback.sync(&self.console.show.sounds);
            ctx.request_repaint_after(Duration::from_millis(40));
        }
        // La lecture des sons fait partir les cues des séquences qui les suivent.
        let positions = self.playback.positions();
        for number in std::mem::take(&mut self.sounds_view.playing) {
            if !positions.contains_key(&number) {
                self.console.forget_time(TimeSource::Sound(number));
            }
        }
        for (&number, &position) in &positions {
            for outcome in self.console.time_tick(TimeSource::Sound(number), position) {
                self.command_line.feedback = Some((true, outcome.message));
            }
        }
        self.sounds_view.playing = positions.keys().copied().collect();
    }

    /// Une commande des boutons de la fenêtre, annoncée comme à la ligne de commande.
    fn run_line(&mut self, line: &str) {
        let result = self.console.execute(line);
        if self.command_line.report(result) {
            self.mark_dirty();
        }
    }

    /// Ajoute des fichiers au pool, aux premiers numéros libres. Rend le premier ajouté.
    pub(crate) fn import_sounds(&mut self, files: &[PathBuf]) -> Option<u32> {
        let mut first = None;
        for file in files {
            let number = (1..).find(|n| !self.console.show.sounds.contains_key(n)).unwrap_or(1);
            self.console.show.sounds.insert(number, Sound::from_file(file));
            first.get_or_insert(number);
        }
        if first.is_some() {
            self.mark_dirty();
            self.playback.open();
        }
        first
    }

    /// Déplace un son dans la liste : renuméroté, avec tout ce qui le désigne (cues,
    /// macros, séquences), et ce qui joue suit.
    pub(crate) fn move_sound(&mut self, number: u32, index: usize) {
        let map = self.console.move_sound(number, index);
        if map.is_empty() {
            return;
        }
        let renumber = |n: u32| map.get(&n).copied().unwrap_or(n);
        self.playback.renumber(&map);
        for n in &mut self.sounds_view.playing {
            *n = renumber(*n);
        }
        self.sounds_view.selected = self.sounds_view.selected.map(renumber);
        if let Some(zoom) = &mut self.sounds_view.zoom {
            zoom.sound = renumber(zoom.sound);
        }
        let new = renumber(number);
        let message = if map.len() > 1 { format!("Son {number} devenu le son {new}, les suivants renumérotés") } else { format!("Son {number} devenu le son {new}") };
        self.command_line.feedback = Some((true, message));
        self.mark_dirty();
    }

    pub(super) fn sounds_window(&mut self, ui: &mut egui::Ui) {
        // La sortie s'ouvre quand on vient régler les sons.
        if !self.playback.is_open() && self.playback.error.is_none() {
            self.playback.open();
        }
        if !self.playback.preview.device.is_empty() && self.playback.preview.error.is_none() {
            self.playback.open_preview();
        }
        // Fichiers ou dossiers glissés sur la fenêtre.
        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        if !dropped.is_empty() {
            let (files, ignored) = audio_files(&dropped);
            let count = files.len();
            if let Some(first) = self.import_sounds(&files) {
                self.sounds_view.selected = Some(first);
            }
            let message = match (count, ignored) {
                (0, _) => (false, "Aucun son reconnu (WAV, AIFF, FLAC, MP3, OGG, M4A)".to_string()),
                (n, 0) => (true, format!("{n} son(s) ajouté(s)")),
                (n, i) => (true, format!("{n} son(s) ajouté(s), {i} fichier(s) ignoré(s)")),
            };
            self.command_line.feedback = Some(message);
        }
        let hovering = ui.input(|i| !i.raw.hovered_files.is_empty());
        let (positions, peaks) = self.playback.status();
        for (shown, peak) in self.sounds_view.meter.iter_mut().zip(peaks) {
            *shown = (*shown * 0.85).max(peak);
        }
        self.sound_output_bar(ui);
        ui.add_space(6.0);
        let numbers: Vec<u32> = self.console.show.sounds.keys().copied().collect();
        if self.sounds_view.selected.is_none_or(|n| !numbers.contains(&n)) {
            self.sounds_view.selected = numbers.first().copied();
        }
        if hovering {
            let rect = ui.available_rect_before_wrap();
            ui.painter().rect_filled(rect, theme::RADIUS, theme::INFO.gamma_multiply(0.18));
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Déposez les sons ici (fichiers ou dossiers)",
                egui::FontId::proportional(16.0),
                theme::TEXT,
            );
        }
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(220.0);
                self.sound_list(ui, &positions);
            });
            ui.separator();
            ui.vertical(|ui| match self.sounds_view.selected {
                Some(number) => self.sound_editor(ui, number, positions.get(&number).copied()),
                None => {
                    ui.add_space(20.0);
                    ui.label(
                        RichText::new("Aucun son. Importez ou glissez ici des fichiers : WAV, AIFF, FLAC, MP3, OGG, M4A.")
                            .color(theme::MUTED),
                    );
                }
            });
        });
    }

    fn sound_output_bar(&mut self, ui: &mut egui::Ui) {
        let devices = self.sounds_view.devices.get_or_insert_with(folie_audio::player::output_devices).clone();
        ui.horizontal(|ui| {
            let label = ui.label("Salle");
            let current = if self.playback.device.is_empty() { "celle du système".to_string() } else { self.playback.device.clone() };
            let mut chosen = self.playback.device.clone();
            egui::ComboBox::from_id_salt("sound_output")
                .width(240.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, String::new(), "celle du système");
                    for name in &devices {
                        ui.selectable_value(&mut chosen, name.clone(), name);
                    }
                })
                .response
                .labelled_by(label.id)
                .on_hover_text("Carte son, sortie intégrée, ou BlackHole pour passer par un autre logiciel");
            if chosen != self.playback.device {
                self.playback.set_device(chosen);
                self.file.storage.set_sound_output(&self.playback.device);
            }
            if ui.button("Tout arrêter").on_hover_text("Sound Off").clicked() {
                self.run_line("Sound Off");
            }
            meter(ui, self.sounds_view.meter);
            if let Some(name) = self.playback.device_name() {
                ui.label(RichText::new(format!("ouverte : {name}")).size(11.0).color(theme::MUTED));
            }
        });
        ui.horizontal(|ui| {
            let label = ui.label("Pré-écoute");
            let current = if self.playback.preview.device.is_empty() { "aucune".to_string() } else { self.playback.preview.device.clone() };
            let mut chosen = self.playback.preview.device.clone();
            egui::ComboBox::from_id_salt("sound_preview_output")
                .width(240.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, String::new(), "aucune");
                    for name in &devices {
                        ui.selectable_value(&mut chosen, name.clone(), name);
                    }
                })
                .response
                .labelled_by(label.id)
                .on_hover_text("Casque : « Écouter » y joue, la salle n'entend rien. Prise casque du Mac, autre carte son…");
            if chosen != self.playback.preview.device {
                self.playback.set_preview_device(chosen);
                self.file.storage.set_sound_preview_output(&self.playback.preview.device);
            }
            match self.playback.preview_device_name() {
                Some(name) if name == self.playback.device_name().unwrap_or_default() && !self.playback.silent_output() => {
                    ui.label(RichText::new("même sortie que la salle : la salle entendra").size(11.0).color(theme::EXECUTOR));
                }
                Some(name) => {
                    ui.label(RichText::new(format!("ouverte : {name}")).size(11.0).color(theme::MUTED));
                }
                None => {
                    ui.label(RichText::new("sans pré-écoute, « Écouter » joue en salle").size(11.0).color(theme::MUTED));
                }
            }
        });
        if let Some(e) = &self.playback.error {
            ui.colored_label(theme::DANGER, format!("Sortie audio : {e}"));
        }
        if let Some(e) = &self.playback.preview.error {
            ui.colored_label(theme::DANGER, format!("Pré-écoute : {e}"));
        }
    }

    fn sound_list(&mut self, ui: &mut egui::Ui, positions: &std::collections::HashMap<u32, f64>) {
        // Glisser un son à une autre place de la liste : (son déplacé, place visée).
        let mut moved: Option<(u32, usize)> = None;
        egui::ScrollArea::vertical().id_salt("sound_list").max_height(ui.available_height() - 70.0).show(ui, |ui| {
            let count = self.console.show.sounds.len();
            for (place, (number, sound)) in self.console.show.sounds.iter().enumerate() {
                let playing = positions.contains_key(number);
                let duration = match self.playback.clip(sound) {
                    Some(ClipState::Ready { clip, .. }) => duration_text(clip.duration()),
                    Some(ClipState::Loading) => "…".into(),
                    Some(ClipState::Failed(_)) => "illisible".into(),
                    None => String::new(),
                };
                let group = if sound.group == 0 { String::new() } else { format!("  G{}", sound.group) };
                let text = RichText::new(format!("{number}  {}  {duration}{group}", sound.name))
                    .color(if playing { theme::GO } else { theme::TEXT });
                let selected = self.sounds_view.selected == Some(*number);
                let row = ui.selectable_label(selected, text).interact(Sense::drag());
                let row = if playing { row.on_hover_text("En lecture") } else { row };
                if row.clicked() {
                    self.sounds_view.selected = Some(*number);
                }
                row.dnd_set_drag_payload(*number);
                if row.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                }
                // Au-dessus ou au-dessous de la ligne survolée, selon la moitié.
                let below = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| p.y > row.rect.center().y);
                if row.dnd_hover_payload::<u32>().is_some_and(|d| *d != *number) {
                    let y = if below { row.rect.bottom() } else { row.rect.top() };
                    ui.painter().hline(row.rect.x_range(), y, Stroke::new(2.0, theme::SELECTION));
                }
                if let Some(dragged) = row.dnd_release_payload::<u32>().filter(|d| **d != *number) {
                    let from = self.console.show.sounds.keys().position(|n| n == &*dragged).unwrap_or(0);
                    let target = place + usize::from(below);
                    let index = if from < target { target - 1 } else { target };
                    moved = Some((*dragged, index.min(count - 1)));
                }
            }
        });
        if let Some((number, index)) = moved {
            self.move_sound(number, index);
        }
        ui.add_space(4.0);
        let help = "Ou glissez des fichiers (ou un dossier) sur cette fenêtre. Ils seront copiés à côté du show à l'enregistrement";
        if ui.button("Importer des sons…").on_hover_text(help).clicked()
            && let Some(files) = rfd::FileDialog::new().add_filter("Son", &AUDIO_EXTENSIONS).pick_files()
            && let Some(first) = self.import_sounds(&files)
        {
            self.sounds_view.selected = Some(first);
        }
        if let Some(number) = self.sounds_view.selected {
            let remove = egui::Button::new(RichText::new("Supprimer").color(Color32::from_rgb(255, 138, 132)));
            if ui.add(remove).on_hover_text(format!("Retire le son {number} du show")).clicked() {
                let _ = self.console.execute(&format!("Sound {number} Off"));
                self.console.show.sounds.remove(&number);
                self.sounds_view.selected = None;
                self.mark_dirty();
            }
        }
    }

    fn sound_editor(&mut self, ui: &mut egui::Ui, number: u32, position: Option<f64>) {
        let Some(sound) = self.console.show.sounds.get(&number).cloned() else { return };
        let mut edited = sound.clone();
        // Clic sur la forme d'onde : pré-écoute depuis ce point.
        let mut listen_from = None;
        let state = self.playback.clip(&sound);
        let duration = match state {
            Some(ClipState::Ready { clip, .. }) => Some(clip.duration()),
            _ => None,
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Son {number}")).strong());
            let label = ui.label("Nom");
            ui.add(egui::TextEdit::singleline(&mut edited.name).desired_width(220.0)).labelled_by(label.id);
        });
        let file = sound.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        ui.label(RichText::new(file).size(11.5).color(theme::MUTED)).on_hover_text(sound.file.display().to_string());
        ui.add_space(4.0);
        match state {
            Some(ClipState::Ready { clip, peaks }) => {
                let duration = clip.duration().max(1e-6);
                let zoom = match self.sounds_view.zoom {
                    Some(z) if z.sound == number => z,
                    _ => WaveZoom::whole(number, duration),
                };
                let mut zoom = zoom;
                zoom.clamp(duration);
                ui.horizontal(|ui| {
                    let whole = zoom.is_whole(duration);
                    let center = zoom.from + zoom.span / 2.0;
                    if ui.add_enabled(zoom.span > WaveZoom::MIN_SPAN, egui::Button::new("Zoom avant").small()).on_hover_text("Ou Cmd + molette, ou pincer, sur la forme d'onde").clicked() {
                        zoom.zoom(2.0, center, duration);
                    }
                    if ui.add_enabled(!whole, egui::Button::new("Zoom arrière").small()).clicked() {
                        zoom.zoom(0.5, center, duration);
                    }
                    if ui.add_enabled(!whole, egui::Button::new("Tout").small()).on_hover_text("Tout le fichier").clicked() {
                        zoom = WaveZoom::whole(number, duration);
                    }
                    if ui.add(egui::Button::new("Entrée-sortie").small()).on_hover_text("Cadrer la partie jouée").clicked() {
                        zoom.frame(edited.start, edited.end.unwrap_or(duration), duration);
                    }
                    if !zoom.is_whole(duration) {
                        let text = format!("{} à {}", duration_text(zoom.from), duration_text(zoom.from + zoom.span));
                        ui.label(RichText::new(text).size(11.0).monospace().color(theme::MUTED));
                    }
                });
                let previewing = self.playback.preview_position().filter(|(n, _)| *n == number).map(|(_, t)| t);
                let heads = Heads { show: position, preview: previewing, click_to_listen: self.playback.has_preview() };
                if let Some(t) = waveform(ui, clip, peaks, &mut zoom, &mut edited, heads) {
                    listen_from = Some(t);
                }
                self.sounds_view.zoom = Some(zoom);
            }
            Some(ClipState::Failed(e)) => {
                ui.colored_label(theme::DANGER, format!("Fichier illisible : {e}"));
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Chargement du fichier…");
                });
            }
        }
        ui.add_space(4.0);
        let end_max = duration.unwrap_or(36_000.0);
        egui::Grid::new("sound_settings").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
            let label = ui.label("Entrée");
            ui.add(egui::DragValue::new(&mut edited.start).range(0.0..=end_max).speed(0.05).suffix(" s").max_decimals(2))
                .labelled_by(label.id);
            let label = ui.label("Sortie");
            let mut end = edited.end.unwrap_or(end_max);
            let response = ui
                .add(egui::DragValue::new(&mut end).range(0.0..=end_max).speed(0.05).suffix(" s").max_decimals(2))
                .labelled_by(label.id)
                .on_hover_text("Fin de la lecture (par défaut, la fin du fichier)");
            if response.changed() {
                edited.end = (duration.is_none_or(|d| end < d - 0.005)).then_some(end);
            }
            ui.end_row();
            let label = ui.label("Volume");
            ui.horizontal(|ui| {
                let response = ui
                    .add(egui::Slider::new(&mut edited.gain_db, -60.0..=12.0).suffix(" dB").max_decimals(1))
                    .labelled_by(label.id)
                    .on_hover_text("Option-clic ou double-clic : retour à 0 dB");
                // Retour à 0 dB comme sur un fader de console : double-clic ou Option-clic,
                // et un cran à 0 dB quand on glisse.
                // Le curseur d'egui ne capte que le glissé : le double-clic se lit sur le pointeur.
                let (alt, double) = ui.input(|i| (i.modifiers.alt, i.pointer.button_double_clicked(egui::PointerButton::Primary)));
                let reset = (double && response.hovered()) || (alt && (response.changed() || response.dragged()));
                let detent = response.dragged() && edited.gain_db.abs() < 0.75;
                if reset || detent {
                    edited.gain_db = 0.0;
                }
                if ui.add_enabled(edited.gain_db != 0.0, egui::Button::new("0 dB").small()).on_hover_text("Volume d'origine du fichier").clicked() {
                    edited.gain_db = 0.0;
                }
            });
            ui.checkbox(&mut edited.looped, "En boucle");
            ui.label("");
            ui.end_row();
            let label = ui.label("Fondu d'entrée");
            ui.add(egui::DragValue::new(&mut edited.fade_in).range(0.0..=60.0).speed(0.05).suffix(" s").max_decimals(2))
                .labelled_by(label.id);
            let label = ui.label("Fondu de sortie");
            ui.add(egui::DragValue::new(&mut edited.fade_out).range(0.0..=60.0).speed(0.05).suffix(" s").max_decimals(2))
                .labelled_by(label.id)
                .on_hover_text("Avant le point de sortie");
            ui.end_row();
            let label = ui.label("Groupe");
            ui.add(
                egui::DragValue::new(&mut edited.group)
                    .range(0..=99)
                    .speed(0.1)
                    .custom_formatter(|g, _| if g < 0.5 { "aucun".into() } else { format!("{g:.0}") })
                    .custom_parser(|t| match t.trim() {
                        "" | "aucun" | "-" => Some(0.0),
                        t => t.parse::<u32>().ok().map(f64::from),
                    }),
            )
            .labelled_by(label.id)
            .on_hover_text("Comme sur un sampleur : un son qui part coupe net ceux de son groupe");
            let others = self.console.show.sounds.iter().filter(|(n, s)| **n != number && edited.group != 0 && s.group == edited.group).count();
            if others > 0 {
                ui.label(RichText::new(format!("coupe {others} autre(s) son(s)")).size(11.0).color(theme::MUTED));
            }
            ui.end_row();
        });
        if edited.end.is_some_and(|e| e <= edited.start) {
            edited.end = None;
        }
        ui.add_space(6.0);
        let preview = self.playback.has_preview();
        let previewing = self.playback.preview_position().filter(|(n, _)| *n == number).map(|(_, t)| t);
        ui.horizontal(|ui| {
            let playing = position.is_some();
            if preview {
                // Avec une pré-écoute, « Écouter » reste dans le casque ; la salle a son bouton.
                if ui.button(RichText::new("Écouter").strong()).on_hover_text("Dans la pré-écoute : la salle n'entend rien").clicked() {
                    listen_from = Some(-1.0);
                }
                if ui.button("En salle").on_hover_text(format!("Sound {number}")).clicked() {
                    self.run_line(&format!("Sound {number}"));
                }
            } else if ui.button(RichText::new("Écouter").strong()).on_hover_text(format!("Sound {number}, en salle")).clicked() {
                self.run_line(&format!("Sound {number}"));
            }
            let stop = egui::Button::new("Arrêter");
            if ui.add_enabled(playing || previewing.is_some(), stop).on_hover_text(format!("Pré-écoute, et en salle : Sound {number} Off Fade 1")).clicked() {
                if previewing.is_some() {
                    self.playback.stop_preview();
                }
                if playing {
                    self.run_line(&format!("Sound {number} Off Fade 1"));
                }
            }
            if let Some(t) = position {
                ui.label(RichText::new(duration_text(t)).monospace().color(theme::GO)).on_hover_text("En salle");
            }
            if let Some(t) = previewing {
                ui.label(RichText::new(format!("casque {}", duration_text(t))).monospace().color(theme::EXECUTOR));
            }
            ui.label(
                RichText::new(format!("Dans une cue : clic droit, Au départ : Sound {number}"))
                    .size(11.0)
                    .color(theme::MUTED),
            );
        });
        if let Some(from) = listen_from {
            let from = (from >= 0.0).then_some(from);
            if let Some(problem) = self.playback.preview_sound(number, from, &self.console.show.sounds) {
                self.command_line.feedback = Some((false, problem));
            }
        }
        if edited != sound {
            if edited.gain_db != sound.gain_db {
                self.playback.set_gain(number, edited.gain_db);
                self.playback.set_preview_gain(number, edited.gain_db);
            }
            self.console.show.sounds.insert(number, edited);
            self.mark_dirty();
        }
    }
}

/// Vu-mètre de sortie, gauche et droite.
fn meter(ui: &mut egui::Ui, peaks: [f32; 2]) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(120.0, 14.0), Sense::hover());
    let painter = ui.painter();
    for (i, peak) in peaks.iter().enumerate() {
        let row = egui::Rect::from_min_size(rect.min + egui::vec2(0.0, i as f32 * 7.0), egui::vec2(rect.width(), 5.0));
        painter.rect_filled(row, 2.0, theme::BG);
        let width = row.width() * peak.clamp(0.0, 1.0);
        let color = if *peak >= 0.99 { theme::DANGER } else { theme::GO };
        painter.rect_filled(egui::Rect::from_min_size(row.min, egui::vec2(width, row.height())), 2.0, color);
    }
    response.widget_info(|| egui::WidgetInfo::slider(false, f64::from(peaks[0].max(peaks[1])) * 100.0, "Niveau de sortie"));
}

/// Crête de la tranche `from..to` (en images) : par les crêtes gardées si la tranche
/// est large, dans le fichier lui-même sinon.
fn peak_between(clip: &folie_audio::player::Clip, blocks: &[f32], from: usize, to: usize) -> f32 {
    let to = to.min(clip.frames()).max(from + 1);
    if to - from > 2 * PEAK_BLOCK && !blocks.is_empty() {
        let last = blocks.len() - 1;
        blocks[(from / PEAK_BLOCK).min(last)..=((to - 1) / PEAK_BLOCK).min(last)].iter().fold(0.0f32, |m, p| m.max(*p))
    } else {
        let samples = &clip.samples;
        samples.get(from * 2..(to * 2).min(samples.len())).map_or(0.0, |s| s.iter().fold(0.0f32, |m, v| m.max(v.abs())))
    }
}

/// Têtes de lecture de la forme d'onde : en salle, dans la pré-écoute.
struct Heads {
    show: Option<f64>,
    preview: Option<f64>,
    /// Un clic fait écouter depuis ce point (seulement s'il y a une pré-écoute).
    click_to_listen: bool,
}

/// Forme d'onde : la partie jouée en clair, les points d'entrée et de sortie à la souris,
/// la tête de lecture. Zoom : Cmd + molette ou pincer, autour de la souris ; défilement par
/// la molette horizontale, ou en glissant ailleurs que sur une poignée.
fn waveform(
    ui: &mut egui::Ui,
    clip: &folie_audio::player::Clip,
    blocks: &[f32],
    zoom: &mut WaveZoom,
    sound: &mut Sound,
    heads: Heads,
) -> Option<f64> {
    let duration = clip.duration().max(1e-6);
    let width = ui.available_width().clamp(200.0, 900.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 90.0), Sense::click_and_drag());
    let t_of_x = |zoom: &WaveZoom, x: f32| zoom.from + ((x - rect.left()) / rect.width()) as f64 * zoom.span;

    // Zoom et défilement, souris au-dessus.
    if response.hovered() {
        let (factor, scroll, pointer) = ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta.x, i.pointer.hover_pos()));
        if factor != 1.0
            && let Some(pos) = pointer
        {
            let at = t_of_x(zoom, pos.x);
            zoom.zoom(f64::from(factor), at, duration);
        }
        if scroll != 0.0 {
            zoom.from -= f64::from(scroll / rect.width()) * zoom.span;
            zoom.clamp(duration);
        }
    }
    let x_at = |zoom: &WaveZoom, t: f64| rect.left() + ((t - zoom.from) / zoom.span) as f32 * rect.width();
    let end = sound.end.unwrap_or(duration);

    // Glisser : la poignée (entrée ou sortie) la plus proche suit la souris. Zoomé, on ne
    // prend une poignée que tout près ; ailleurs, on fait défiler.
    let handle_id = response.id.with("handle");
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let (to_start, to_end) = ((pos.x - x_at(zoom, sound.start)).abs(), (pos.x - x_at(zoom, end)).abs());
        let grab = if zoom.is_whole(duration) || to_start.min(to_end) <= 12.0 { Some(to_start <= to_end) } else { None };
        ui.data_mut(|d| d.insert_temp(handle_id, grab));
    }
    if response.dragged()
        && let Some(pos) = response.interact_pointer_pos()
    {
        match ui.data(|d| d.get_temp::<Option<bool>>(handle_id)).flatten() {
            Some(start_handle) => {
                // Arrondi au centième, ou à l'image près quand on est très près.
                let step = if zoom.span < 2.0 { 1.0 / clip.rate.max(1) as f64 } else { 0.01 };
                let t = ((t_of_x(zoom, pos.x).clamp(0.0, duration) / step).round() * step).min(duration);
                if start_handle {
                    sound.start = t.min(end - 0.01).max(0.0);
                } else {
                    let t = t.max(sound.start + 0.01);
                    sound.end = (t < duration - 0.005).then_some(t);
                }
            }
            None => {
                zoom.from -= f64::from(response.drag_delta().x / rect.width()) * zoom.span;
                zoom.clamp(duration);
            }
        }
    }

    let x_of = |t: f64| x_at(zoom, t);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::BG);
    let middle = rect.center().y;
    let half = rect.height() / 2.0 - 4.0;
    // Une colonne par point d'écran.
    let columns = (rect.width().round() as usize).max(1);
    let rate = clip.rate.max(1) as f64;
    for c in 0..columns {
        let t0 = zoom.from + c as f64 / columns as f64 * zoom.span;
        let t1 = zoom.from + (c + 1) as f64 / columns as f64 * zoom.span;
        let peak = peak_between(clip, blocks, (t0 * rate) as usize, (t1 * rate).ceil() as usize);
        let t = (t0 + t1) / 2.0;
        let inside = t >= sound.start && t <= end;
        let color = if inside { theme::INFO } else { theme::INFO.gamma_multiply(0.25) };
        let x = rect.left() + (c as f32 + 0.5) / columns as f32 * rect.width();
        let h = (peak.clamp(0.0, 1.0) * half).max(0.5);
        painter.line_segment([egui::pos2(x, middle - h), egui::pos2(x, middle + h)], Stroke::new(rect.width() / columns as f32 + 0.3, color));
    }
    for (t, label) in [(sound.start, "entrée"), (end, "sortie")] {
        let x = x_of(t);
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(2.0, theme::SELECTION));
        painter.text(egui::pos2(x + 3.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(10.5), theme::SELECTION);
    }
    for (t, color) in [(heads.show, theme::GO), (heads.preview, theme::EXECUTOR)] {
        if let Some(t) = t {
            let x = x_of(t);
            painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(1.5, color));
        }
    }
    // Zoomé : où l'on est dans le fichier, en bas.
    if !zoom.is_whole(duration) {
        let bar = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.bottom() - 3.0), egui::vec2(rect.width(), 3.0));
        let left = bar.left() + (zoom.from / duration) as f32 * bar.width();
        let w = ((zoom.span / duration) as f32 * bar.width()).max(4.0);
        painter.rect_filled(egui::Rect::from_min_size(egui::pos2(left, bar.top()), egui::vec2(w, bar.height())), 1.5, theme::MUTED);
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, "Forme d'onde : glisser les points d'entrée et de sortie"));
    let response = if heads.click_to_listen { response.on_hover_text("Clic : écouter d'ici dans la pré-écoute") } else { response };
    if heads.click_to_listen
        && response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        return Some(t_of_x(zoom, pos.x).clamp(0.0, duration));
    }
    None
}

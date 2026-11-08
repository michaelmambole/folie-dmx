//! Fenêtre Sons : le pool de sons du show (import, réglages, forme d'onde avec les points
//! d'entrée et de sortie, écoute) et la sortie audio.

use std::path::PathBuf;
use std::time::Duration;

use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use folie_control::timecode::TimeSource;
use folie_show::media::Sound;

use super::FolieApp;
use crate::playback_sound::ClipState;
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

    pub(super) fn sounds_window(&mut self, ui: &mut egui::Ui) {
        // La sortie s'ouvre quand on vient régler les sons.
        if !self.playback.is_open() && self.playback.error.is_none() {
            self.playback.open();
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
            let label = ui.label("Sortie");
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
        if let Some(e) = &self.playback.error {
            ui.colored_label(theme::DANGER, format!("Sortie audio : {e}"));
        }
    }

    fn sound_list(&mut self, ui: &mut egui::Ui, positions: &std::collections::HashMap<u32, f64>) {
        egui::ScrollArea::vertical().id_salt("sound_list").max_height(ui.available_height() - 70.0).show(ui, |ui| {
            for (number, sound) in &self.console.show.sounds {
                let playing = positions.contains_key(number);
                let duration = match self.playback.clip(sound) {
                    Some(ClipState::Ready { clip, .. }) => duration_text(clip.duration()),
                    Some(ClipState::Loading) => "…".into(),
                    Some(ClipState::Failed(_)) => "illisible".into(),
                    None => String::new(),
                };
                let text = RichText::new(format!("{number}  {}  {duration}", sound.name))
                    .color(if playing { theme::GO } else { theme::TEXT });
                let selected = self.sounds_view.selected == Some(*number);
                let row = ui.selectable_label(selected, text);
                let row = if playing { row.on_hover_text("En lecture") } else { row };
                if row.clicked() {
                    self.sounds_view.selected = Some(*number);
                }
            }
        });
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
            Some(ClipState::Ready { peaks, .. }) => {
                waveform(ui, peaks, duration.unwrap_or(1.0), &mut edited, position);
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
            ui.add(egui::Slider::new(&mut edited.gain_db, -60.0..=12.0).suffix(" dB").max_decimals(1)).labelled_by(label.id);
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
        });
        if edited.end.is_some_and(|e| e <= edited.start) {
            edited.end = None;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let playing = position.is_some();
            if ui.button(RichText::new("Écouter").strong()).on_hover_text(format!("Sound {number}")).clicked() {
                self.run_line(&format!("Sound {number}"));
            }
            if ui.add_enabled(playing, egui::Button::new("Arrêter")).on_hover_text(format!("Sound {number} Off Fade 1")).clicked() {
                self.run_line(&format!("Sound {number} Off Fade 1"));
            }
            if let Some(t) = position {
                ui.label(RichText::new(duration_text(t)).monospace().color(theme::GO));
            }
            ui.label(
                RichText::new(format!("Dans une cue : clic droit, Au départ : Sound {number}"))
                    .size(11.0)
                    .color(theme::MUTED),
            );
        });
        if edited != sound {
            if edited.gain_db != sound.gain_db {
                self.playback.set_gain(number, edited.gain_db);
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

/// Forme d'onde : la partie jouée en clair, les points d'entrée et de sortie à la souris,
/// la tête de lecture.
fn waveform(ui: &mut egui::Ui, peaks: &[f32], duration: f64, sound: &mut Sound, position: Option<f64>) {
    let width = ui.available_width().clamp(200.0, 900.0);
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 90.0), Sense::click_and_drag());
    let x_of = |t: f64| rect.left() + (t / duration.max(1e-6)) as f32 * rect.width();
    let t_of = |x: f32| (((x - rect.left()) / rect.width()) as f64 * duration).clamp(0.0, duration);
    let end = sound.end.unwrap_or(duration);

    // Glisser : la poignée la plus proche (entrée ou sortie) suit la souris.
    let handle_id = response.id.with("handle");
    if response.drag_started()
        && let Some(pos) = response.interact_pointer_pos()
    {
        let nearer_start = (pos.x - x_of(sound.start)).abs() <= (pos.x - x_of(end)).abs();
        ui.data_mut(|d| d.insert_temp(handle_id, nearer_start));
    }
    if response.dragged()
        && let (Some(pos), Some(start_handle)) = (response.interact_pointer_pos(), ui.data(|d| d.get_temp::<bool>(handle_id)))
    {
        let t = (t_of(pos.x) * 100.0).round() / 100.0;
        if start_handle {
            sound.start = t.min(end - 0.01).max(0.0);
        } else {
            let t = t.max(sound.start + 0.01);
            sound.end = (t < duration - 0.005).then_some(t);
        }
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4.0, theme::BG);
    let middle = rect.center().y;
    let half = rect.height() / 2.0 - 4.0;
    let columns = peaks.len().max(1);
    for (i, peak) in peaks.iter().enumerate() {
        let x = rect.left() + (i as f32 + 0.5) / columns as f32 * rect.width();
        let t = (i as f64 + 0.5) / columns as f64 * duration;
        let inside = t >= sound.start && t <= end;
        let color = if inside { theme::INFO } else { theme::INFO.gamma_multiply(0.25) };
        let h = (peak.clamp(0.0, 1.0) * half).max(0.5);
        painter.line_segment([egui::pos2(x, middle - h), egui::pos2(x, middle + h)], Stroke::new(rect.width() / columns as f32 + 0.3, color));
    }
    for (t, label) in [(sound.start, "entrée"), (end, "sortie")] {
        let x = x_of(t);
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(2.0, theme::SELECTION));
        painter.text(egui::pos2(x + 3.0, rect.top() + 2.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(10.5), theme::SELECTION);
    }
    if let Some(t) = position {
        let x = x_of(t);
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], Stroke::new(1.5, theme::GO));
    }
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Slider, true, "Forme d'onde : glisser les points d'entrée et de sortie"));
}

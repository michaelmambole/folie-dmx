//! Lecture de sons : fichiers décodés en mémoire (WAV, AIFF, FLAC, MP3, OGG, AAC/M4A),
//! mis à la fréquence de la sortie, puis mélangés en stéréo, chacun avec son point
//! d'entrée et de sortie, son volume, ses fondus et sa boucle.
//!
//! Le mélangeur est séparé de la carte son : il se teste en lui demandant des blocs.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, unbounded};

/// Un son prêt à jouer : stéréo entrelacé, à la fréquence de la sortie.
#[derive(Clone)]
pub struct Clip {
    pub samples: Arc<Vec<f32>>,
    pub rate: u32,
}

impl Clip {
    pub fn frames(&self) -> usize {
        self.samples.len() / 2
    }

    pub fn duration(&self) -> f64 {
        self.frames() as f64 / self.rate.max(1) as f64
    }

    /// Crêtes par tranche (gauche et droite mêlées), pour dessiner la forme d'onde.
    pub fn peaks(&self, columns: usize) -> Vec<f32> {
        let frames = self.frames();
        if columns == 0 || frames == 0 {
            return Vec::new();
        }
        (0..columns)
            .map(|c| {
                let from = c * frames / columns;
                let to = ((c + 1) * frames / columns).max(from + 1).min(frames);
                self.samples[from * 2..to * 2].iter().fold(0.0f32, |m, s| m.max(s.abs()))
            })
            .collect()
    }
}

/// Décode un fichier en stéréo entrelacé, à sa fréquence d'origine.
pub fn decode(path: &Path) -> Result<(Vec<f32>, u32), String> {
    use symphonia::core::codecs::audio::AudioDecoderOptions;
    use symphonia::core::errors::Error;
    use symphonia::core::formats::probe::Hint;
    use symphonia::core::formats::{FormatOptions, TrackType};
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;

    let file = std::fs::File::open(path).map_err(|e| format!("{} : {e}", path.display()))?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, stream, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| format!("format non reconnu : {e}"))?;
    let track = format.default_track(TrackType::Audio).ok_or("aucune piste audio")?;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or("piste audio illisible")?.clone();
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .map_err(|e| format!("codec non pris en charge : {e}"))?;
    let rate = params.sample_rate.ok_or("fréquence d'échantillonnage inconnue")?;

    let mut stereo = Vec::new();
    let mut buffer: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(packet)) => packet,
            Ok(None) => break,
            Err(Error::IoError(_)) => break,
            Err(e) => return Err(format!("lecture : {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(Error::DecodeError(_)) => continue,
            Err(e) => return Err(format!("décodage : {e}")),
        };
        let channels = decoded.spec().channels().count().max(1);
        buffer.resize(decoded.samples_interleaved(), 0.0);
        decoded.copy_to_slice_interleaved(&mut buffer);
        for frame in buffer.chunks_exact(channels) {
            // Mono : la même chose des deux côtés ; plus de deux canaux : les deux premiers.
            let (left, right) = if channels == 1 { (frame[0], frame[0]) } else { (frame[0], frame[1]) };
            stereo.push(left);
            stereo.push(right);
        }
    }
    if stereo.is_empty() {
        return Err("fichier vide".into());
    }
    Ok((stereo, rate))
}

/// Remet un son stéréo à une autre fréquence (rééchantillonnage FFT de qualité).
pub fn resample(stereo: Vec<f32>, from: u32, to: u32) -> Result<Vec<f32>, String> {
    use rubato::audioadapter_buffers::owned::InterleavedOwned;
    use rubato::{Fft, FixedSync, Resampler};
    if from == to || stereo.is_empty() {
        return Ok(stereo);
    }
    let frames = stereo.len() / 2;
    let input = InterleavedOwned::new_from(stereo, 2, frames).map_err(|e| e.to_string())?;
    let mut resampler = Fft::<f32>::new(from as usize, to as usize, 1024, 2, FixedSync::Both).map_err(|e| e.to_string())?;
    let output = resampler.process_all(&input, frames, None).map_err(|e| e.to_string())?;
    Ok(output.take_data())
}

/// Charge un fichier pour une sortie à `rate` Hz.
pub fn load(path: &Path, rate: u32) -> Result<Clip, String> {
    let (stereo, from) = decode(path)?;
    let samples = resample(stereo, from, rate)?;
    Ok(Clip { samples: Arc::new(samples), rate })
}

/// Comment jouer un son.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaySettings {
    /// Point d'entrée, en secondes depuis le début du fichier.
    pub start: f64,
    /// Point de sortie ; `None` : la fin du fichier.
    pub end: Option<f64>,
    /// Volume, en dB (0 : tel quel).
    pub gain_db: f32,
    /// Fondu d'entrée et de sortie, en secondes.
    pub fade_in: f32,
    pub fade_out: f32,
    pub looped: bool,
}

impl Default for PlaySettings {
    fn default() -> Self {
        Self { start: 0.0, end: None, gain_db: 0.0, fade_in: 0.0, fade_out: 0.0, looped: false }
    }
}

pub fn db_to_gain(db: f32) -> f32 {
    if db <= -80.0 { 0.0 } else { 10f32.powf(db / 20.0) }
}

/// Ce qu'on demande au mélangeur.
pub enum Command {
    /// Joue un son (un son déjà en lecture sous ce numéro repart du début).
    Play { id: u32, clip: Clip, settings: PlaySettings },
    /// Arrête un son, en fondu de `fade` secondes (0 : net).
    Stop { id: u32, fade: f32 },
    StopAll { fade: f32 },
    /// Change le volume d'un son en cours.
    Gain { id: u32, gain_db: f32 },
    /// Les sons ont changé de numéro (ancien -> nouveau) : ceux en cours suivent.
    Renumber(HashMap<u32, u32>),
}

struct Voice {
    id: u32,
    clip: Clip,
    /// Image en cours, dans le fichier.
    position: usize,
    start: usize,
    end: usize,
    gain: f32,
    /// Enveloppe des fondus, de 0 à 1.
    envelope: f32,
    /// Variation de l'enveloppe par image : montée (fondu d'entrée) ou descente (arrêt).
    ramp: f32,
    fade_out: usize,
    looped: bool,
    stopping: bool,
}

/// État visible de l'extérieur : où en est chaque son, et les crêtes de sortie.
#[derive(Debug, Clone, Default)]
pub struct PlayerStatus {
    /// Son -> position dans le fichier, en secondes.
    pub positions: HashMap<u32, f64>,
    /// Crêtes de sortie, gauche et droite, depuis le dernier relevé.
    pub peaks: [f32; 2],
}

/// Mélangeur : les sons en cours, rendus bloc par bloc.
pub struct Mixer {
    rate: u32,
    voices: Vec<Voice>,
}

impl Mixer {
    pub fn new(rate: u32) -> Self {
        Self { rate, voices: Vec::new() }
    }

    fn frames(&self, seconds: f64) -> usize {
        (seconds.max(0.0) * self.rate as f64).round() as usize
    }

    pub fn apply(&mut self, command: Command) {
        match command {
            Command::Play { id, clip, settings } => {
                self.voices.retain(|v| v.id != id);
                let length = clip.frames();
                let start = self.frames(settings.start).min(length);
                let end = settings.end.map_or(length, |e| self.frames(e)).clamp(start, length);
                let fade_in = self.frames(settings.fade_in as f64);
                self.voices.push(Voice {
                    id,
                    clip,
                    position: start,
                    start,
                    end,
                    gain: db_to_gain(settings.gain_db),
                    envelope: if fade_in == 0 { 1.0 } else { 0.0 },
                    ramp: if fade_in == 0 { 0.0 } else { 1.0 / fade_in as f32 },
                    fade_out: self.frames(settings.fade_out as f64),
                    looped: settings.looped,
                    stopping: false,
                });
            }
            Command::Stop { id, fade } => self.stop(|v| v.id == id, fade),
            Command::StopAll { fade } => self.stop(|_| true, fade),
            Command::Gain { id, gain_db } => {
                for voice in self.voices.iter_mut().filter(|v| v.id == id) {
                    voice.gain = db_to_gain(gain_db);
                }
            }
            Command::Renumber(map) => {
                for voice in &mut self.voices {
                    voice.id = map.get(&voice.id).copied().unwrap_or(voice.id);
                }
            }
        }
    }

    fn stop(&mut self, which: impl Fn(&Voice) -> bool, fade: f32) {
        let frames = self.frames(fade as f64);
        if frames == 0 {
            self.voices.retain(|v| !which(v));
            return;
        }
        for voice in self.voices.iter_mut().filter(|v| which(v)) {
            voice.stopping = true;
            // Descente depuis le niveau actuel, en `fade` secondes depuis le plein.
            voice.ramp = -1.0 / frames as f32;
        }
    }

    pub fn is_playing(&self, id: u32) -> bool {
        self.voices.iter().any(|v| v.id == id)
    }

    /// Positions des sons en cours, en secondes dans le fichier.
    pub fn positions(&self) -> HashMap<u32, f64> {
        self.voices.iter().map(|v| (v.id, v.position as f64 / self.rate as f64)).collect()
    }

    /// Ajoute les sons en cours dans `out` (entrelacé, `channels` canaux) et rend les crêtes.
    pub fn render(&mut self, out: &mut [f32], channels: usize) -> [f32; 2] {
        let channels = channels.max(1);
        out.iter_mut().for_each(|s| *s = 0.0);
        let mut peaks = [0.0f32; 2];
        for voice in &mut self.voices {
            for frame in out.chunks_exact_mut(channels) {
                if voice.position >= voice.end {
                    if voice.looped && voice.end > voice.start && !voice.stopping {
                        voice.position = voice.start;
                    } else {
                        voice.stopping = true;
                        voice.envelope = 0.0;
                        break;
                    }
                }
                voice.envelope = (voice.envelope + voice.ramp).clamp(0.0, 1.0);
                if voice.stopping && voice.envelope <= 0.0 {
                    break;
                }
                // Fondu de sortie automatique avant le point de sortie (sans boucle).
                let remaining = voice.end - voice.position;
                let tail = if !voice.looped && voice.fade_out > 0 && remaining < voice.fade_out {
                    remaining as f32 / voice.fade_out as f32
                } else {
                    1.0
                };
                let level = voice.gain * voice.envelope * tail;
                let left = voice.clip.samples[voice.position * 2] * level;
                let right = voice.clip.samples[voice.position * 2 + 1] * level;
                if channels == 1 {
                    frame[0] += (left + right) * 0.5;
                } else {
                    frame[0] += left;
                    frame[1] += right;
                }
                voice.position += 1;
            }
        }
        self.voices.retain(|v| !(v.stopping && v.envelope <= 0.0));
        for frame in out.chunks_exact(channels) {
            peaks[0] = peaks[0].max(frame[0].abs());
            peaks[1] = peaks[1].max(frame[channels.min(2) - 1].abs());
        }
        peaks
    }
}

/// Sorties audio présentes.
pub fn output_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.output_devices()
        .map(|list| list.filter_map(|d| d.description().ok().map(|desc| desc.name().to_string())).collect())
        .unwrap_or_default()
}

/// La carte son qui joue : le mélangeur tourne dans son fil à elle.
pub struct Player {
    commands: Sender<Command>,
    status: Arc<Mutex<PlayerStatus>>,
    rate: u32,
    pub device: String,
    _stream: cpal::Stream,
}

impl Player {
    /// Ouvre une sortie (`None` : celle du système), à la fréquence `rate` si elle la
    /// connaît (pour jouer les mêmes fichiers qu'une autre sortie), sinon à la sienne.
    pub fn start(device: Option<&str>, rate: Option<u32>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = match device {
            Some(name) => host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.description().ok().is_some_and(|desc| desc.name() == name))
                .ok_or_else(|| format!("sortie « {name} » absente"))?,
            None => host.default_output_device().ok_or("aucune sortie audio")?,
        };
        let name = device.description().map(|d| d.name().to_string()).unwrap_or_default();
        let default = device.default_output_config().map_err(|e| format!("sortie {name} : {e}"))?;
        let mut config: cpal::StreamConfig = default.into();
        if let Some(wanted) = rate.filter(|r| *r != config.sample_rate)
            && let Ok(mut ranges) = device.supported_output_configs()
            && ranges.any(|r| r.channels() == config.channels && r.min_sample_rate() <= wanted && wanted <= r.max_sample_rate())
        {
            config.sample_rate = wanted;
        }
        let rate = config.sample_rate;
        let channels = config.channels as usize;
        let (commands, receiver): (Sender<Command>, Receiver<Command>) = unbounded();
        let status = Arc::new(Mutex::new(PlayerStatus::default()));
        let shared = status.clone();
        let mut mixer = Mixer::new(rate);
        let stream = device
            .build_output_stream(
                config,
                move |out: &mut [f32], _| {
                    for command in receiver.try_iter() {
                        mixer.apply(command);
                    }
                    let peaks = mixer.render(out, channels);
                    // Jamais d'attente dans le fil audio : relevé sauté si l'interface lit.
                    if let Ok(mut status) = shared.try_lock() {
                        status.positions = mixer.positions();
                        status.peaks = [status.peaks[0].max(peaks[0]), status.peaks[1].max(peaks[1])];
                    }
                },
                |err| log::warn!("sortie audio : {err}"),
                None,
            )
            .map_err(|e| format!("sortie {name} : {e}"))?;
        stream.play().map_err(|e| format!("sortie {name} : {e}"))?;
        Ok(Self { commands, status, rate, device: name, _stream: stream })
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    /// Positions des sons en cours (sans toucher aux crêtes).
    pub fn positions(&self) -> HashMap<u32, f64> {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).positions.clone()
    }

    /// Positions des sons en cours, et crêtes depuis le dernier appel (remises à zéro).
    pub fn status(&self) -> PlayerStatus {
        let mut status = self.status.lock().unwrap_or_else(|e| e.into_inner());
        let copy = status.clone();
        status.peaks = [0.0; 2];
        copy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un son de `frames` images à 1.0 à gauche et 0.5 à droite, à 100 Hz.
    fn clip(frames: usize) -> Clip {
        Clip { samples: Arc::new([1.0, 0.5].repeat(frames)), rate: 100 }
    }

    fn render(mixer: &mut Mixer, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * 2];
        mixer.render(&mut out, 2);
        out
    }

    #[test]
    fn plays_from_in_to_out_points() {
        let mut mixer = Mixer::new(100);
        let settings = PlaySettings { start: 0.1, end: Some(0.3), ..Default::default() };
        mixer.apply(Command::Play { id: 1, clip: clip(100), settings });
        assert_eq!(mixer.positions()[&1], 0.1);
        let out = render(&mut mixer, 30);
        assert_eq!(&out[..4], [1.0, 0.5, 1.0, 0.5]);
        assert_eq!(out[19 * 2], 1.0, "20 images, de 0,1 s à 0,3 s");
        assert_eq!(out[20 * 2], 0.0, "plus rien après le point de sortie");
        assert!(!mixer.is_playing(1), "le son s'est arrêté seul");
    }

    #[test]
    fn fades_volume_and_stop() {
        let mut mixer = Mixer::new(100);
        let settings = PlaySettings { fade_in: 0.1, gain_db: -6.0, ..Default::default() };
        mixer.apply(Command::Play { id: 1, clip: clip(100), settings });
        let out = render(&mut mixer, 20);
        let gain = db_to_gain(-6.0);
        assert!((out[0] - gain * 0.1).abs() < 1e-4, "fondu d'entrée : {}", out[0]);
        assert!((out[9 * 2] - gain).abs() < 1e-4, "plein volume au bout de 0,1 s");
        // Arrêt en 0,1 s : il descend puis s'arrête.
        mixer.apply(Command::Stop { id: 1, fade: 0.1 });
        let out = render(&mut mixer, 20);
        assert!(out[0] < gain && out[0] > 0.0);
        assert_eq!(out[15 * 2], 0.0);
        assert!(!mixer.is_playing(1));
    }

    #[test]
    fn automatic_fade_out_before_the_end_and_loop() {
        let mut mixer = Mixer::new(100);
        let settings = PlaySettings { fade_out: 0.1, ..Default::default() };
        mixer.apply(Command::Play { id: 1, clip: clip(20), settings });
        let out = render(&mut mixer, 20);
        assert_eq!(out[0], 1.0);
        assert!((out[15 * 2] - 0.5).abs() < 1e-4, "à mi-fondu de sortie : {}", out[15 * 2]);
        // En boucle, il repart au point d'entrée.
        let settings = PlaySettings { start: 0.05, looped: true, ..Default::default() };
        mixer.apply(Command::Play { id: 2, clip: clip(20), settings });
        render(&mut mixer, 25);
        assert!(mixer.is_playing(2));
        assert!((mixer.positions()[&2] - 0.15).abs() < 1e-9, "15 images après la boucle à 0,05 s");
    }

    #[test]
    fn several_sounds_mix_and_mono_output() {
        let mut mixer = Mixer::new(100);
        mixer.apply(Command::Play { id: 1, clip: clip(10), settings: PlaySettings::default() });
        mixer.apply(Command::Play { id: 2, clip: clip(10), settings: PlaySettings::default() });
        let mut out = vec![0.0; 4];
        let peaks = mixer.render(&mut out, 1);
        assert_eq!(out[0], 1.5, "deux sons (1 + 0,5) / 2 chacun, additionnés");
        assert_eq!(peaks, [1.5, 1.5]);
        mixer.apply(Command::StopAll { fade: 0.0 });
        assert!(!mixer.is_playing(1) && !mixer.is_playing(2));
    }

    /// Un vrai fichier : WAV mono 16 bits à 44,1 kHz, décodé en stéréo et passé à 48 kHz.
    #[test]
    fn decodes_and_resamples_a_wav_file() {
        let dir = std::env::temp_dir().join(format!("folie-son-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bip.wav");
        let rate = 44_100u32;
        let samples: Vec<i16> =
            (0..rate).map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 16_000.0) as i16).collect();
        let mut wav = Vec::new();
        let data_len = samples.len() as u32 * 2;
        wav.extend(b"RIFF");
        wav.extend((36 + data_len).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(1u16.to_le_bytes()); // PCM
        wav.extend(1u16.to_le_bytes()); // mono
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 2).to_le_bytes());
        wav.extend(2u16.to_le_bytes());
        wav.extend(16u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend(data_len.to_le_bytes());
        for s in &samples {
            wav.extend(s.to_le_bytes());
        }
        std::fs::write(&path, wav).unwrap();

        let (stereo, from) = decode(&path).unwrap();
        assert_eq!(from, 44_100);
        assert_eq!(stereo.len(), 2 * 44_100, "mono -> stéréo");
        assert_eq!(stereo[200], stereo[201], "les deux côtés pareils");
        let clip = load(&path, 48_000).unwrap();
        assert!((clip.duration() - 1.0).abs() < 0.01, "une seconde à 48 kHz : {}", clip.duration());
        let loudest = clip.samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!((loudest - 16_000.0 / 32_768.0).abs() < 0.05, "niveau gardé : {loudest}");
        assert_eq!(clip.peaks(10).len(), 10);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(decode(&dir.join("absent.wav")).is_err());
    }
}

//! Entrée audio (micro, carte son) : un canal ou le mélange de tous, analysé dans le fil
//! audio lui-même ; les réactions partent de là vers le moteur.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use crossbeam_channel::Sender;
use folie_engine::Command;
use folie_show::sound::SoundSetup;

use crate::analysis::{Analyzer, Hop, Levels};
use crate::reactor::Reactor;

/// Un appareil d'entrée.
#[derive(Debug, Clone, PartialEq)]
pub struct InputDevice {
    pub name: String,
    pub channels: u16,
}

/// Appareils d'entrée du système, celui par défaut en premier.
pub fn input_devices() -> Vec<InputDevice> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.description().ok()).map(|d| d.name().to_string());
    let mut list: Vec<InputDevice> = host
        .input_devices()
        .map(|devices| {
            devices
                .filter_map(|d| {
                    let name = d.description().ok()?.name().to_string();
                    let channels = d.default_input_config().ok()?.channels();
                    Some(InputDevice { name, channels })
                })
                .collect()
        })
        .unwrap_or_default();
    list.sort_by_key(|d| Some(&d.name) != default.as_ref());
    list
}

/// Ce que l'interface affiche : niveaux, beats, tempo.
#[derive(Debug, Clone, Copy, Default)]
pub struct Meter {
    pub levels: Levels,
    pub beats: u64,
    pub bpm: Option<f32>,
    pub last_beat: Option<Instant>,
}

/// Entrée en marche ; elle s'arrête quand on la lâche (les flashs en cours sont relâchés).
pub struct AudioInput {
    _stream: cpal::Stream,
    meter: Arc<Mutex<Meter>>,
    setup: Arc<Mutex<SoundSetup>>,
    setup_version: Arc<AtomicU64>,
    gain: Arc<AtomicU32>,
    pub device: String,
    pub channels: u16,
}

impl AudioInput {
    /// `device` : nom de l'appareil (`None` : celui par défaut). `channel` : canal à suivre,
    /// à partir de 0 (`None` : le mélange de tous).
    pub fn start(
        device: Option<&str>,
        channel: Option<u16>,
        gain: f32,
        setup: SoundSetup,
        tx: Sender<Command>,
    ) -> Result<Self, String> {
        let host = cpal::default_host();
        let found = match device {
            Some(name) => host
                .input_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.description().is_ok_and(|desc| desc.name() == name)),
            None => host.default_input_device(),
        };
        let device = found.ok_or_else(|| match device {
            Some(name) => format!("entrée « {name} » introuvable"),
            None => "aucune entrée audio".to_string(),
        })?;
        let name = device.description().map(|d| d.name().to_string()).unwrap_or_else(|_| "entrée".into());
        let config = device.default_input_config().map_err(|e| format!("{name} : {e}"))?;
        let channels = config.channels();
        let sample_rate = config.sample_rate() as f32;

        let meter = Arc::new(Mutex::new(Meter::default()));
        let shared_setup = Arc::new(Mutex::new(setup.clone()));
        let setup_version = Arc::new(AtomicU64::new(0));
        let shared_gain = Arc::new(AtomicU32::new(gain.to_bits()));
        let mut worker = Worker {
            analyzer: Analyzer::new(sample_rate),
            reactor: Reactor::new(tx, setup),
            meter: meter.clone(),
            setup: shared_setup.clone(),
            setup_version: setup_version.clone(),
            seen_version: 0,
            gain: shared_gain.clone(),
            channels: channels as usize,
            channel: channel.map(usize::from).filter(|c| *c < channels as usize),
            mono: Vec::new(),
            hops: Vec::new(),
            beats: 0,
        };
        let error = move |err: cpal::Error| log::warn!("entrée audio : {err}");
        let stream_config: cpal::StreamConfig = config.config();
        let stream = match config.sample_format() {
            SampleFormat::F32 => device.build_input_stream(stream_config, move |data: &[f32], _: &_| worker.run(data), error, None),
            SampleFormat::I16 => device.build_input_stream(stream_config, move |data: &[i16], _: &_| worker.run(data), error, None),
            SampleFormat::I32 => device.build_input_stream(stream_config, move |data: &[i32], _: &_| worker.run(data), error, None),
            SampleFormat::U16 => device.build_input_stream(stream_config, move |data: &[u16], _: &_| worker.run(data), error, None),
            other => return Err(format!("{name} : format {other:?} non géré")),
        }
        .map_err(|e| format!("{name} : {e}"))?;
        stream.play().map_err(|e| format!("{name} : {e}"))?;
        log::info!("entrée audio : {name}, {channels} canaux, {sample_rate} Hz");
        Ok(Self { _stream: stream, meter, setup: shared_setup, setup_version, gain: shared_gain, device: name, channels })
    }

    pub fn meter(&self) -> Meter {
        *self.meter.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Nouvelles réactions (le show a changé).
    pub fn set_setup(&self, setup: &SoundSetup) {
        *self.setup.lock().unwrap_or_else(|e| e.into_inner()) = setup.clone();
        self.setup_version.fetch_add(1, Ordering::Release);
    }

    pub fn set_gain(&self, gain: f32) {
        self.gain.store(gain.to_bits(), Ordering::Relaxed);
    }
}

/// Ce qui tourne dans le fil audio. Il ne bloque jamais : les verrous sont seulement tentés.
struct Worker {
    analyzer: Analyzer,
    reactor: Reactor,
    meter: Arc<Mutex<Meter>>,
    setup: Arc<Mutex<SoundSetup>>,
    setup_version: Arc<AtomicU64>,
    seen_version: u64,
    gain: Arc<AtomicU32>,
    channels: usize,
    channel: Option<usize>,
    mono: Vec<f32>,
    hops: Vec<Hop>,
    beats: u64,
}

impl Worker {
    fn run<T: SizedSample>(&mut self, data: &[T])
    where
        f32: FromSample<T>,
    {
        let version = self.setup_version.load(Ordering::Acquire);
        if version != self.seen_version
            && let Ok(setup) = self.setup.try_lock()
        {
            self.reactor.set_setup(setup.clone());
            self.seen_version = version;
        }
        self.analyzer.set_gain(f32::from_bits(self.gain.load(Ordering::Relaxed)));
        self.mono.clear();
        for frame in data.chunks(self.channels.max(1)) {
            let sample = match self.channel {
                Some(c) => frame.get(c).map_or(0.0, |s| s.to_sample::<f32>()),
                None => frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / frame.len() as f32,
            };
            self.mono.push(sample);
        }
        let hops = &mut self.hops;
        hops.clear();
        self.analyzer.push(&self.mono, |hop| hops.push(hop));
        let bpm = self.analyzer.bpm();
        for hop in &self.hops {
            self.reactor.on_hop(hop, bpm);
            if hop.beat {
                self.beats += 1;
            }
        }
        if let Some(last) = self.hops.last()
            && let Ok(mut meter) = self.meter.try_lock()
        {
            let beat = self.hops.iter().any(|h| h.beat);
            *meter = Meter {
                levels: last.levels,
                beats: self.beats,
                bpm,
                last_beat: if beat { Some(Instant::now()) } else { meter.last_beat },
            };
        }
    }
}

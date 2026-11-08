//! Lecture des sons du show : la sortie audio (ouverte à la demande), les fichiers
//! chargés en arrière-plan, et les commandes `Sound` passées à la carte son.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use folie_audio::player::{self, Clip, Command, Mixer, PlaySettings, Player};
use folie_control::sounds::SoundRequest;
use folie_show::media::Sound;

/// Où part le son : la carte son, ou (tests) un mélangeur muet qu'on interroge.
pub enum Output {
    Device(Player),
    Silent(Mixer),
}

/// Un fichier du show, chargé ou en cours de chargement.
pub enum ClipState {
    Loading,
    /// `peaks` : une crête par bloc de [`PEAK_BLOCK`] images.
    Ready { clip: Clip, peaks: Vec<f32> },
    Failed(String),
}

/// Images par crête gardée pour la forme d'onde : assez fin pour zoomer (au-delà, la
/// forme d'onde lit le fichier lui-même).
pub const PEAK_BLOCK: usize = 256;

pub struct SoundPlayback {
    output: Option<Output>,
    /// Sortie choisie ; vide : celle du système.
    pub device: String,
    pub error: Option<String>,
    /// Fichier -> état (un même fichier n'est chargé qu'une fois).
    clips: HashMap<PathBuf, ClipState>,
    sender: Sender<(PathBuf, Result<Clip, String>)>,
    receiver: Receiver<(PathBuf, Result<Clip, String>)>,
    /// Sons demandés pendant leur chargement : joués dès qu'ils sont prêts.
    pending: Vec<u32>,
    /// Muet (tests) : aucune carte son n'est ouverte.
    silent: bool,
}

/// Coupure d'un son par un autre de son groupe : juste assez pour ne pas claquer.
const CHOKE_FADE: f32 = 0.005;

/// Fréquence du mélangeur muet.
const SILENT_RATE: u32 = 48_000;

impl SoundPlayback {
    pub fn new(device: String) -> Self {
        let (sender, receiver) = channel();
        Self { output: None, device, error: None, clips: HashMap::new(), sender, receiver, pending: Vec::new(), silent: false }
    }

    /// Sans carte son : tout se joue dans un mélangeur qu'on peut interroger.
    pub fn silent() -> Self {
        Self { silent: true, ..Self::new(String::new()) }
    }

    pub fn is_open(&self) -> bool {
        self.output.is_some()
    }

    pub fn device_name(&self) -> Option<&str> {
        match &self.output {
            Some(Output::Device(player)) => Some(&player.device),
            Some(Output::Silent(_)) => Some("muet"),
            None => None,
        }
    }

    fn rate(&self) -> Option<u32> {
        match &self.output {
            Some(Output::Device(player)) => Some(player.rate()),
            Some(Output::Silent(_)) => Some(SILENT_RATE),
            None => None,
        }
    }

    /// Ouvre la sortie si besoin. Une autre fréquence oblige à recharger les fichiers.
    pub fn open(&mut self) -> bool {
        if self.output.is_some() {
            return true;
        }
        let output = if self.silent {
            Output::Silent(Mixer::new(SILENT_RATE))
        } else {
            match Player::start(Some(self.device.as_str()).filter(|d| !d.is_empty())) {
                Ok(player) => Output::Device(player),
                Err(e) => {
                    self.error = Some(e);
                    return false;
                }
            }
        };
        self.output = Some(output);
        self.error = None;
        self.clips.clear();
        true
    }

    /// Change de sortie : les sons en cours s'arrêtent.
    pub fn set_device(&mut self, device: String) {
        self.device = device;
        self.output = None;
        self.clips.clear();
        self.open();
    }

    fn send(&mut self, command: Command) {
        match &mut self.output {
            Some(Output::Device(player)) => player.send(command),
            Some(Output::Silent(mixer)) => mixer.apply(command),
            None => {}
        }
    }

    pub fn clip(&self, sound: &Sound) -> Option<&ClipState> {
        self.clips.get(&sound.file)
    }

    /// Charge (en arrière-plan) les fichiers du show pas encore prêts, et range ceux
    /// qui viennent de l'être. Ne fait rien tant que la sortie n'est pas ouverte.
    pub fn sync(&mut self, sounds: &BTreeMap<u32, Sound>) {
        let Some(rate) = self.rate() else { return };
        for (path, result) in self.receiver.try_iter() {
            let state = match result {
                Ok(clip) => {
                    let peaks = clip.peaks(clip.frames().div_ceil(PEAK_BLOCK));
                    ClipState::Ready { clip, peaks }
                }
                Err(e) => ClipState::Failed(e),
            };
            self.clips.insert(path, state);
        }
        for sound in sounds.values() {
            if sound.file.as_os_str().is_empty() || self.clips.contains_key(&sound.file) {
                continue;
            }
            self.clips.insert(sound.file.clone(), ClipState::Loading);
            let (path, sender) = (sound.file.clone(), self.sender.clone());
            std::thread::spawn(move || {
                let result = player::load(&path, rate);
                let _ = sender.send((path, result));
            });
        }
        // Sons demandés pendant leur chargement : ceux qui sont prêts partent.
        let pending = std::mem::take(&mut self.pending);
        for number in pending {
            let _ = self.play(number, sounds);
        }
    }

    /// Joue les demandes de la console. Rend les problèmes à signaler.
    pub fn handle(&mut self, requests: Vec<SoundRequest>, sounds: &BTreeMap<u32, Sound>) -> Vec<String> {
        let mut problems = Vec::new();
        if requests.is_empty() || !self.open() {
            if let (false, Some(e)) = (requests.is_empty(), &self.error) {
                problems.push(format!("Son : {e}"));
            }
            return problems;
        }
        for request in requests {
            match request {
                SoundRequest::Play(number) => {
                    if let Some(problem) = self.play(number, sounds) {
                        problems.push(problem);
                    }
                }
                SoundRequest::Stop { number, fade } => {
                    self.pending.retain(|n| *n != number);
                    self.send(Command::Stop { id: number, fade });
                }
                SoundRequest::StopAll { fade } => {
                    self.pending.clear();
                    self.send(Command::StopAll { fade });
                }
            }
        }
        problems
    }

    fn play(&mut self, number: u32, sounds: &BTreeMap<u32, Sound>) -> Option<String> {
        let sound = sounds.get(&number)?;
        let clip = match self.clips.get(&sound.file) {
            Some(ClipState::Ready { clip, .. }) => clip.clone(),
            Some(ClipState::Failed(e)) => return Some(format!("Son {number} : {e}")),
            // Pas encore chargé : il partira dès qu'il le sera.
            // (le chargement est lancé par `sync`, à l'image suivante).
            Some(ClipState::Loading) | None => {
                if !self.pending.contains(&number) {
                    self.pending.push(number);
                }
                return None;
            }
        };
        let settings = PlaySettings {
            start: sound.start,
            end: sound.end,
            gain_db: sound.gain_db,
            fade_in: sound.fade_in,
            fade_out: sound.fade_out,
            looped: sound.looped,
        };
        // Groupe d'exclusion : les autres sons du groupe sont coupés net (un rien de fondu
        // pour éviter le clic), comme sur un sampleur.
        if sound.group != 0 {
            for (&other, _) in sounds.iter().filter(|(n, s)| **n != number && s.group == sound.group) {
                self.pending.retain(|n| *n != other);
                self.send(Command::Stop { id: other, fade: CHOKE_FADE });
            }
        }
        self.send(Command::Play { id: number, clip, settings });
        None
    }

    /// Les sons ont changé de numéro : ceux en cours ou attendus suivent.
    pub fn renumber(&mut self, map: &HashMap<u32, u32>) {
        for n in &mut self.pending {
            *n = map.get(n).copied().unwrap_or(*n);
        }
        self.send(Command::Renumber(map.clone()));
    }

    /// Volume changé pendant la lecture.
    pub fn set_gain(&mut self, number: u32, gain_db: f32) {
        self.send(Command::Gain { id: number, gain_db });
    }

    /// Sons en cours -> position dans le fichier (s).
    pub fn positions(&self) -> HashMap<u32, f64> {
        match &self.output {
            Some(Output::Device(player)) => player.positions(),
            Some(Output::Silent(mixer)) => mixer.positions(),
            None => HashMap::new(),
        }
    }

    /// Sons en cours -> position dans le fichier (s), et crêtes de sortie.
    pub fn status(&mut self) -> (HashMap<u32, f64>, [f32; 2]) {
        match &mut self.output {
            Some(Output::Device(player)) => {
                let status = player.status();
                (status.positions, status.peaks)
            }
            Some(Output::Silent(mixer)) => (mixer.positions(), [0.0; 2]),
            None => (HashMap::new(), [0.0; 2]),
        }
    }

    /// Mélangeur muet (tests) : on y fait avancer le temps.
    #[cfg(test)]
    pub fn silent_mixer(&mut self) -> Option<&mut Mixer> {
        match &mut self.output {
            Some(Output::Silent(mixer)) => Some(mixer),
            _ => None,
        }
    }
}

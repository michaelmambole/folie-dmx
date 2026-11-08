//! Lecture des sons du show : la sortie audio (ouverte à la demande), les fichiers
//! chargés en arrière-plan, et les commandes `Sound` passées à la carte son.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use folie_audio::player::{self, Clip, Command, Mixer, PREVIEW, PlaySettings, Player};
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
    /// Sorties du mélangeur muet (tests).
    pub silent_channels: usize,
    /// Pré-écoute (casque) : une autre paire de sorties de la carte de la salle, ou une
    /// autre carte. La salle ne l'entend pas et le show ne la voit pas (ni cues qui
    /// suivent le son, ni groupes d'exclusion).
    pub preview: Preview,
}

/// Fichier remis à la fréquence de la pré-écoute.
type Prepared = (PathBuf, ClipState);

/// La sortie de pré-écoute.
#[derive(Default)]
pub struct Preview {
    /// Sortie choisie ; vide : pas de pré-écoute.
    pub device: String,
    /// Paire de sorties : 0 pour 1-2, 1 pour 3-4…
    pub pair: u32,
    /// Même carte que la salle : la pré-écoute passe par son mélangeur, sur sa paire.
    shared: bool,
    output: Option<Output>,
    pub error: Option<String>,
    /// Fichiers remis à la fréquence de la pré-écoute, quand ce n'est pas celle de la salle.
    clips: HashMap<PathBuf, ClipState>,
    channel: Option<(Sender<Prepared>, Receiver<Prepared>)>,
    /// Demandée pendant la préparation du fichier : (son, à partir de).
    pending: Option<(u32, Option<f64>)>,
    /// Fréquence de la pré-écoute muette (tests) ; 0 : celle de la salle muette.
    pub silent_rate: u32,
}

/// Départ de la pré-écoute au milieu d'un son : un rien de fondu, pour ne pas claquer.
const PREVIEW_FADE: f32 = 0.005;

/// Coupure d'un son par un autre de son groupe : juste assez pour ne pas claquer.
const CHOKE_FADE: f32 = 0.005;

/// Fréquence du mélangeur muet.
const SILENT_RATE: u32 = 48_000;

impl SoundPlayback {
    pub fn new(device: String) -> Self {
        let (sender, receiver) = channel();
        Self {
            output: None,
            device,
            error: None,
            clips: HashMap::new(),
            sender,
            receiver,
            pending: Vec::new(),
            silent: false,
            silent_channels: 2,
            preview: Preview::default(),
        }
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

    fn output_channels(&self, output: &Option<Output>) -> usize {
        match output {
            Some(Output::Device(player)) => player.channels(),
            Some(Output::Silent(_)) => self.silent_channels,
            None => 0,
        }
    }

    /// Sorties de la carte de la salle (0 : fermée).
    pub fn channels(&self) -> usize {
        self.output_channels(&self.output)
    }

    /// Sorties de la carte de pré-écoute (0 : fermée).
    pub fn preview_channels(&self) -> usize {
        if self.preview.shared { self.channels() } else { self.output_channels(&self.preview.output) }
    }

    /// La pré-écoute passe-t-elle par la carte de la salle ?
    pub fn preview_shares_output(&self) -> bool {
        self.preview.shared
    }

    /// Ouvre la sortie si besoin. Une autre fréquence oblige à recharger les fichiers.
    pub fn open(&mut self) -> bool {
        if self.output.is_some() {
            return true;
        }
        let output = if self.silent {
            Output::Silent(Mixer::new(SILENT_RATE))
        } else {
            match Player::start(Some(self.device.as_str()).filter(|d| !d.is_empty()), None) {
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
        // La pré-écoute suit la fréquence de la salle.
        self.close_preview();
        self.open_preview();
    }

    /// Choisit la sortie de pré-écoute (vide : aucune).
    pub fn set_preview_device(&mut self, device: String) {
        self.preview.device = device;
        self.close_preview();
        self.open_preview();
    }

    /// Choisit la paire de sorties de la pré-écoute ; ce qui y joue s'arrête.
    pub fn set_preview_pair(&mut self, pair: u32) {
        self.stop_preview();
        self.preview.pair = pair;
    }

    fn close_preview(&mut self) {
        self.stop_preview();
        self.preview.shared = false;
        self.preview.output = None;
        self.preview.clips.clear();
        self.preview.pending = None;
        self.preview.error = None;
    }

    /// Une pré-écoute est-elle réglée (et ouverte) ?
    pub fn has_preview(&self) -> bool {
        self.preview.shared || self.preview.output.is_some()
    }

    pub fn preview_device_name(&self) -> Option<&str> {
        if self.preview.shared {
            return self.device_name();
        }
        match &self.preview.output {
            Some(Output::Device(player)) => Some(&player.device),
            Some(Output::Silent(_)) => Some("muet"),
            None => None,
        }
    }

    /// Ouvre la pré-écoute si elle est réglée, à la fréquence de la salle si elle peut.
    pub fn open_preview(&mut self) -> bool {
        if self.has_preview() {
            return true;
        }
        if self.preview.device.is_empty() {
            return false;
        }
        // Même carte que la salle : un seul flux, la pré-écoute sur sa paire.
        if self.open() && (self.device_name() == Some(self.preview.device.as_str()) || self.device == self.preview.device) {
            self.preview.shared = true;
            self.preview.error = None;
            return true;
        }
        let output = if self.silent {
            Output::Silent(Mixer::new(if self.preview.silent_rate == 0 { SILENT_RATE } else { self.preview.silent_rate }))
        } else {
            match Player::start(Some(self.preview.device.as_str()), self.rate()) {
                Ok(player) => Output::Device(player),
                Err(e) => {
                    self.preview.error = Some(e);
                    return false;
                }
            }
        };
        self.preview.output = Some(output);
        self.preview.error = None;
        true
    }

    fn preview_rate(&self) -> Option<u32> {
        if self.preview.shared {
            return self.rate();
        }
        match &self.preview.output {
            Some(Output::Device(player)) => Some(player.rate()),
            Some(Output::Silent(_)) => Some(if self.preview.silent_rate == 0 { SILENT_RATE } else { self.preview.silent_rate }),
            None => None,
        }
    }

    fn send_preview(&mut self, command: Command) {
        if self.preview.shared {
            return self.send(command);
        }
        match &mut self.preview.output {
            Some(Output::Device(player)) => player.send(command),
            Some(Output::Silent(mixer)) => mixer.apply(command),
            None => {}
        }
    }

    /// Écoute un son dans la pré-écoute, depuis son entrée ou depuis `from` (s). Un seul
    /// son à la fois : le précédent s'arrête. Rend un problème à signaler.
    pub fn preview_sound(&mut self, number: u32, from: Option<f64>, sounds: &BTreeMap<u32, Sound>) -> Option<String> {
        if !self.open_preview() {
            return self.preview.error.as_ref().map(|e| format!("Pré-écoute : {e}"));
        }
        let sound = sounds.get(&number)?;
        let clip = match self.clips.get(&sound.file) {
            Some(ClipState::Ready { clip, .. }) => clip.clone(),
            Some(ClipState::Failed(e)) => return Some(format!("Son {number} : {e}")),
            Some(ClipState::Loading) | None => {
                self.preview.pending = Some((number, from));
                return None;
            }
        };
        let rate = self.preview_rate()?;
        let clip = if clip.rate == rate {
            clip
        } else {
            // Autre fréquence : le fichier est refait pour la pré-écoute, en arrière-plan.
            match self.preview.clips.get(&sound.file) {
                Some(ClipState::Ready { clip, .. }) => clip.clone(),
                Some(ClipState::Failed(e)) => return Some(format!("Pré-écoute du son {number} : {e}")),
                Some(ClipState::Loading) => {
                    self.preview.pending = Some((number, from));
                    return None;
                }
                None => {
                    self.preview.clips.insert(sound.file.clone(), ClipState::Loading);
                    self.preview.pending = Some((number, from));
                    let (sender, _) = self.preview.channel.get_or_insert_with(channel);
                    let (sender, path) = (sender.clone(), sound.file.clone());
                    std::thread::spawn(move || {
                        let state = match player::resample(clip.samples.to_vec(), clip.rate, rate) {
                            Ok(samples) => ClipState::Ready { clip: Clip { samples: std::sync::Arc::new(samples), rate }, peaks: Vec::new() },
                            Err(e) => ClipState::Failed(e),
                        };
                        let _ = sender.send((path, state));
                    });
                    return None;
                }
            }
        };
        let mut settings = PlaySettings {
            start: sound.start,
            end: sound.end,
            gain_db: sound.gain_db,
            fade_in: sound.fade_in,
            fade_out: sound.fade_out,
            looped: sound.looped,
            pair: self.preview.pair as usize,
        };
        if let Some(from) = from {
            settings.start = from.max(0.0);
            settings.fade_in = PREVIEW_FADE;
            // Écouter après la sortie : jusqu'au bout du fichier.
            if settings.end.is_some_and(|e| e <= from) {
                settings.end = None;
            }
        }
        self.preview.pending = None;
        self.send_preview(Command::StopPreview { fade: PREVIEW_FADE });
        self.send_preview(Command::Play { id: PREVIEW | number, clip, settings });
        None
    }

    /// Volume changé pendant la pré-écoute.
    pub fn set_preview_gain(&mut self, number: u32, gain_db: f32) {
        self.send_preview(Command::Gain { id: PREVIEW | number, gain_db });
    }

    pub fn stop_preview(&mut self) {
        self.preview.pending = None;
        self.send_preview(Command::StopPreview { fade: PREVIEW_FADE });
    }

    /// Son en pré-écoute et sa position (s).
    pub fn preview_position(&self) -> Option<(u32, f64)> {
        let output = if self.preview.shared { &self.output } else { &self.preview.output };
        all_positions(output).into_iter().find(|(id, _)| id & PREVIEW != 0).map(|(id, t)| (id & !PREVIEW, t))
    }

    /// Pré-écoute muette (tests) : le mélangeur de la salle quand elle le partage.
    #[cfg(test)]
    pub fn silent_preview(&mut self) -> Option<&mut Mixer> {
        let output = if self.preview.shared { &mut self.output } else { &mut self.preview.output };
        match output {
            Some(Output::Silent(mixer)) => Some(mixer),
            _ => None,
        }
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
        if let Some((_, receiver)) = &self.preview.channel {
            for (path, state) in receiver.try_iter() {
                self.preview.clips.insert(path, state);
            }
        }
        if let Some((number, from)) = self.preview.pending.take()
            && let Some(problem) = self.preview_sound(number, from, sounds)
        {
            self.preview.error = Some(problem);
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
            pair: sound.pair as usize,
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
        // La pré-écoute porte les mêmes numéros, marqués.
        let marked: HashMap<u32, u32> = map.iter().map(|(old, new)| (PREVIEW | old, PREVIEW | new)).collect();
        if self.preview.shared {
            self.send(Command::Renumber(map.iter().map(|(o, n)| (*o, *n)).chain(marked).collect()));
        } else {
            self.send(Command::Renumber(map.clone()));
            self.send_preview(Command::Renumber(marked));
        }
        if let Some((n, _)) = &mut self.preview.pending {
            *n = map.get(n).copied().unwrap_or(*n);
        }
    }

    /// Volume changé pendant la lecture.
    pub fn set_gain(&mut self, number: u32, gain_db: f32) {
        self.send(Command::Gain { id: number, gain_db });
    }

    /// Sons en cours en salle -> position dans le fichier (s).
    pub fn positions(&self) -> HashMap<u32, f64> {
        let mut positions = all_positions(&self.output);
        positions.retain(|id, _| id & PREVIEW == 0);
        positions
    }

    /// Sons en cours en salle -> position dans le fichier (s), et crêtes de sortie.
    pub fn status(&mut self) -> (HashMap<u32, f64>, [f32; 2]) {
        let (mut positions, peaks) = match &mut self.output {
            Some(Output::Device(player)) => {
                let status = player.status();
                (status.positions, status.peaks)
            }
            Some(Output::Silent(mixer)) => (mixer.positions(), [0.0; 2]),
            None => (HashMap::new(), [0.0; 2]),
        };
        positions.retain(|id, _| id & PREVIEW == 0);
        (positions, peaks)
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

/// Tous les sons en cours d'une sortie, pré-écoute comprise.
fn all_positions(output: &Option<Output>) -> HashMap<u32, f64> {
    match output {
        Some(Output::Device(player)) => player.positions(),
        Some(Output::Silent(mixer)) => mixer.positions(),
        None => HashMap::new(),
    }
}

//! Lecture des séquences par les executors : transitions, fondus, délais.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use folie_fixture::ParamKey;

use crate::effects::EffectInstance;

/// État complet d'une cue, tracking déjà appliqué par la console : tout ce que
/// la séquence tient à ce moment-là, pas seulement ce qui change.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CueState {
    /// « 1.5 Entrée », pour l'affichage.
    pub label: String,
    pub values: HashMap<ParamKey, u16>,
    /// Secondes.
    pub fade: f32,
    /// Secondes.
    pub delay: f32,
    /// Temps propres de certains paramètres, (fondu, délai) en secondes ; les autres
    /// suivent ceux de la cue.
    pub times: HashMap<ParamKey, (f32, f32)>,
    /// Effets qui tournent pendant cette cue (déjà suivis depuis les cues précédentes).
    pub effects: Vec<EffectInstance>,
}

/// Cues jouées par chaque executor, dans l'ordre de la séquence.
pub type Sequences = HashMap<u32, Arc<Vec<CueState>>>;

/// Ce que fait le fader d'un executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FaderMode {
    /// Dose l'intensité.
    #[default]
    Master,
    /// Dose l'intensité, et l'executor ne joue que tant que le fader est levé.
    Temp,
    /// L'intensité reste pleine ; le fader règle la vitesse des effets et du chenillard (50 % = normale).
    Speed,
    /// L'intensité reste pleine ; le fader fait la transition vers la cue suivante à la main.
    Crossfade,
}

/// Ordre dans lequel un chenillard parcourt ses cues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChaserDirection {
    #[default]
    Forward,
    Backward,
    Bounce,
    Random,
}

/// Chenillard : les cues défilent seules.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaserOptions {
    /// Pas par minute ; `None` : le tempo du show.
    pub bpm: Option<f32>,
    /// Fondu entre deux pas, en fraction du pas.
    pub step_fade: f32,
    pub direction: ChaserDirection,
}

/// Cue jouée quand on rallume un executor éteint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Restart {
    #[default]
    First,
    Current,
    Next,
}

/// Options de fonctionnement d'un executor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaybackOptions {
    /// Après la dernière cue, Go revient à la première.
    pub wrap: bool,
    /// Lever le fader depuis 0 lance la cue 1.
    pub auto_start: bool,
    /// Fader à 0 : l'executor s'éteint.
    pub auto_stop: bool,
    pub fader: FaderMode,
    /// Durée de l'extinction (Off), en secondes.
    pub off_time: f32,
    /// Durée du Back ; `None` : le fondu de la cue visée.
    pub back_time: Option<f32>,
    /// Sur un même paramètre, la priorité la plus haute l'emporte.
    pub priority: i8,
    /// Ignoré par « Tout éteindre ».
    pub protected: bool,
    pub chaser: Option<ChaserOptions>,
    pub restart: Restart,
    /// S'éteint quand d'autres executors ont repris tout ce qu'il tient.
    pub ooo: bool,
    /// Prépositionne dans le noir ce que la cue suivante va allumer.
    pub mib: bool,
    pub swop_protect: bool,
    pub kill_protect: bool,
}

impl Default for PlaybackOptions {
    fn default() -> Self {
        Self {
            wrap: false,
            auto_start: false,
            auto_stop: false,
            fader: FaderMode::Master,
            off_time: 0.0,
            back_time: None,
            priority: 0,
            protected: false,
            chaser: None,
            restart: Restart::First,
            ooo: false,
            mib: false,
            swop_protect: false,
            kill_protect: false,
        }
    }
}

/// Déclenchement d'une transition : quand, dans quel ordre, avec quel minutage.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Transition {
    pub(crate) now: Instant,
    /// Ordre de déclenchement, pour la priorité LTP.
    pub(crate) activated: u64,
    pub(crate) timing: Timing,
}

/// Minutage d'une transition.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Timing {
    /// Fondu et délai de la cue visée.
    Cue,
    /// Tout de suite (flash).
    Snap,
    /// Fondu de cette durée, sans délai (Off, Back, pas de chenillard).
    Fixed(f32),
    /// Fondu mené par le fader (crossfade), qui part de cette extrémité (0 ou 1).
    Manual(f32),
}

/// Ce que l'interface voit d'un executor.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ExecutorStatus {
    /// Index de la cue en cours dans la séquence, `None` si l'executor est éteint.
    pub cue: Option<usize>,
    /// Avancement de la transition en cours, de 0 à 1.
    pub progress: f32,
    /// Niveau du fader, de 0 à 1.
    pub level: f32,
    pub cue_count: usize,
    /// Touche Flash maintenue : pleine intensité, quel que soit le fader.
    pub flashing: bool,
    /// Délai de la transition en cours, en secondes.
    pub delay: f32,
    /// Fondu de la transition en cours, en secondes.
    pub fade: f32,
    /// Temps écoulé depuis le début de la transition, en secondes.
    pub elapsed: f32,
    /// Durée totale de la transition, temps propres des paramètres compris.
    pub duration: f32,
    /// Chenillard en marche : avancement du pas en cours, de 0 à 1.
    pub step: Option<f32>,
    /// Crossfade mené par le fader : avancement, de 0 à 1.
    pub manual: Option<f32>,
}

impl ExecutorStatus {
    /// Une transition (délai ou fondu) est en cours.
    pub fn in_transition(&self) -> bool {
        self.manual.is_some() || self.elapsed < self.duration
    }
}

pub(crate) struct Playback {
    cue: Option<usize>,
    /// Valeurs tenues au moment où la transition a commencé.
    from: HashMap<ParamKey, u16>,
    started: Instant,
    fade: f32,
    delay: f32,
    /// Temps propres des paramètres pour la transition en cours.
    times: HashMap<ParamKey, (f32, f32)>,
    pub(crate) level: f32,
    /// Ordre de déclenchement : en LTP, le dernier executor lancé gagne.
    pub(crate) activated: u64,
    pub(crate) flashing: bool,
    /// Touche Swop maintenue : les autres executors s'éteignent.
    pub(crate) swopping: bool,
    pub(crate) options: PlaybackOptions,
    /// Dernière cue jouée, pour la reprise après extinction.
    last: Option<usize>,
    /// Chenillard : avancement du pas en cours, de 0 à 1.
    pub(crate) step_phase: f64,
    /// Aller-retour : on redescend la séquence.
    backwards: bool,
    /// Tirage du hasard (xorshift).
    seed: u64,
    /// Crossfade : avancement mené par le fader.
    manual: Option<f32>,
    /// Extrémité d'où le fader est parti (0 ou 1).
    manual_origin: f32,
}

impl Playback {
    pub(crate) fn new() -> Self {
        Self {
            cue: None,
            from: HashMap::new(),
            started: Instant::now(),
            fade: 0.0,
            delay: 0.0,
            times: HashMap::new(),
            level: 1.0,
            activated: 0,
            flashing: false,
            swopping: false,
            options: PlaybackOptions::default(),
            last: None,
            step_phase: 0.0,
            backwards: false,
            seed: 0x9E37_79B9_7F4A_7C15,
            manual: None,
            manual_origin: 0.0,
        }
    }

    /// Niveau effectif de l'intensité : pleine pendant un flash, ou si le fader ne la dose pas.
    pub(crate) fn intensity_level(&self) -> f32 {
        if self.flashing || matches!(self.options.fader, FaderMode::Speed | FaderMode::Crossfade) { 1.0 } else { self.level }
    }

    /// Cue de départ quand on rallume l'executor, selon le mode de reprise.
    pub(crate) fn start_cue(&self, len: usize) -> Option<usize> {
        let last = len.checked_sub(1)?;
        let looping = self.options.wrap || self.options.chaser.is_some();
        Some(match (self.options.restart, self.last) {
            (Restart::First, _) | (_, None) => 0,
            (Restart::Current, Some(c)) => c.min(last),
            (Restart::Next, Some(c)) if c < last => c + 1,
            (Restart::Next, Some(_)) if looping => 0,
            (Restart::Next, Some(_)) => last,
        })
    }

    /// Pas suivant du chenillard (`forward` : vers l'avant de son sens), `None` s'il est éteint.
    pub(crate) fn chaser_next(&mut self, len: usize, forward: bool) -> Option<usize> {
        let (c, direction) = (self.cue?, self.options.chaser?.direction);
        if len < 2 {
            return Some(0);
        }
        Some(match direction {
            ChaserDirection::Forward if forward => (c + 1) % len,
            ChaserDirection::Backward if !forward => (c + 1) % len,
            ChaserDirection::Forward | ChaserDirection::Backward => (c + len - 1) % len,
            ChaserDirection::Bounce => {
                // Au bout, on repart dans l'autre sens ; Back recule sans changer de sens.
                let down = self.backwards == forward;
                let down = if down && c == 0 || !down && c + 1 >= len { !down } else { down };
                if forward {
                    self.backwards = down;
                }
                if down { c - 1 } else { c + 1 }
            }
            ChaserDirection::Random => {
                self.seed ^= self.seed << 13;
                self.seed ^= self.seed >> 7;
                self.seed ^= self.seed << 17;
                // Jamais deux fois le même pas de suite.
                let r = (self.seed % (len as u64 - 1)) as usize;
                if r >= c { r + 1 } else { r }
            }
        })
    }

    /// Repart du début d'un pas, dans le sens normal.
    pub(crate) fn reset_chaser(&mut self) {
        self.step_phase = 0.0;
        self.backwards = false;
    }

    /// Crossfade : avancement mené par le fader. Arrivé à l'autre extrémité, la transition est finie.
    pub(crate) fn set_manual(&mut self, level: f32) {
        if self.manual.is_none() {
            return;
        }
        let progress = (level - self.manual_origin).abs().min(1.0);
        if progress >= 1.0 {
            self.manual = None;
            self.times.clear();
        } else {
            self.manual = Some(progress);
        }
    }

    pub(crate) fn in_manual(&self) -> bool {
        self.manual.is_some()
    }

    /// Multiplicateur de vitesse des effets : seul le fader « Vitesse » le change.
    pub(crate) fn effect_speed(&self) -> f64 {
        if self.options.fader == FaderMode::Speed { self.level as f64 * 2.0 } else { 1.0 }
    }

    /// Avancement, de 0 à 1, d'un fondu de `fade` secondes après `delay` secondes.
    fn progress_with(&self, now: Instant, fade: f32, delay: f32) -> f32 {
        if let Some(manual) = self.manual {
            return manual;
        }
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32() - delay;
        if elapsed < 0.0 {
            0.0
        } else if fade <= 0.0 {
            1.0
        } else {
            (elapsed / fade).min(1.0)
        }
    }

    /// Avancement au temps de la cue.
    fn progress(&self, now: Instant) -> f32 {
        self.progress_with(now, self.fade, self.delay)
    }

    /// Avancement d'un paramètre : son temps propre, sinon celui de la cue.
    fn progress_of(&self, key: &ParamKey, now: Instant) -> f32 {
        match self.times.get(key) {
            Some(&(fade, delay)) => self.progress_with(now, fade, delay),
            None => self.progress(now),
        }
    }

    /// Durée totale de la transition : le plus long des temps, propres ou de la cue.
    fn duration(&self) -> f32 {
        self.times.values().map(|(fade, delay)| fade + delay).fold(self.fade + self.delay, f32::max)
    }

    /// Valeurs tenues par l'executor à cet instant, avant son fader.
    /// `is_intensity` dit si un paramètre se mélange en HTP : il part alors de 0
    /// et y retourne. Les autres partent de `under` (ce qui est sur scène sous
    /// cet executor) et y reviennent en fondu.
    pub(crate) fn values(
        &self,
        cues: &[CueState],
        now: Instant,
        is_intensity: impl Fn(&ParamKey) -> bool,
        under: impl Fn(&ParamKey) -> u16,
    ) -> HashMap<ParamKey, u16> {
        let t = self.progress(now);
        let lerp = |a: u16, b: u16, t: f32| (a as f32 + (b as f32 - a as f32) * t).round() as u16;
        let target = self.cue.and_then(|c| cues.get(c)).map(|c| &c.values);
        let mut values = HashMap::new();
        if let Some(target) = target {
            for (key, &b) in target {
                let a = match self.from.get(key) {
                    Some(&a) => a,
                    None if is_intensity(key) => 0,
                    None => under(key),
                };
                values.insert(*key, lerp(a, b, self.progress_of(key, now)));
            }
        }
        // Ce que la nouvelle cue ne tient plus revient en fondu : l'intensité vers 0,
        // le reste vers ce qui est dessous. Une fois le fondu fini, on le rend.
        for (key, &a) in &self.from {
            if target.is_some_and(|t| t.contains_key(key)) {
                continue;
            }
            if is_intensity(key) {
                values.insert(*key, lerp(a, 0, t));
            } else if t < 1.0 {
                values.insert(*key, lerp(a, under(key), t));
            }
        }
        self.move_in_black(&mut values, cues, now, &is_intensity, &under);
        values
    }

    /// MIB : une fois la transition finie, les fixtures éteintes par cette cue et que
    /// la suivante allume prennent d'avance ses autres valeurs, au fondu de la suivante.
    fn move_in_black(
        &self,
        values: &mut HashMap<ParamKey, u16>,
        cues: &[CueState],
        now: Instant,
        is_intensity: impl Fn(&ParamKey) -> bool,
        under: impl Fn(&ParamKey) -> u16,
    ) {
        if !self.options.mib || self.options.chaser.is_some() || self.manual.is_some() {
            return;
        }
        let Some(c) = self.cue else { return };
        let next = if c + 1 < cues.len() { c + 1 } else if self.options.wrap && c > 0 { 0 } else { return };
        let next = &cues[next];
        let since = now.saturating_duration_since(self.started).as_secs_f32() - self.duration();
        if since < 0.0 {
            return;
        }
        let t = if next.fade <= 0.0 { 1.0 } else { (since / next.fade).min(1.0) };
        // Éteintes : la cue tient leur intensité, à 0. Allumées ensuite : une intensité au-dessus de 0.
        let (mut held, mut lit) = (HashSet::new(), HashSet::new());
        for (key, v) in values.iter().filter(|(k, _)| is_intensity(k)) {
            held.insert(key.fixture);
            if *v > 0 {
                lit.insert(key.fixture);
            }
        }
        let lit_next: HashSet<_> = next.values.iter().filter(|(k, v)| is_intensity(k) && **v > 0).map(|(k, _)| k.fixture).collect();
        let lerp = |a: u16, b: u16| (a as f32 + (b as f32 - a as f32) * t).round() as u16;
        for (key, &b) in &next.values {
            let dark = held.contains(&key.fixture) && !lit.contains(&key.fixture);
            if dark && lit_next.contains(&key.fixture) && !is_intensity(key) {
                let a = values.get(key).copied().unwrap_or_else(|| under(key));
                values.insert(*key, lerp(a, b));
            }
        }
    }

    /// Démarre la transition vers une cue (ou vers rien pour `None`).
    pub(crate) fn go_to(
        &mut self,
        cue: Option<usize>,
        cues: &[CueState],
        transition: Transition,
        is_intensity: impl Fn(&ParamKey) -> bool,
        under: impl Fn(&ParamKey) -> u16,
    ) {
        let Transition { now, activated, timing } = transition;
        self.from = self.values(cues, now, is_intensity, under);
        self.cue = cue;
        if cue.is_some() {
            self.last = cue;
        }
        let target = cue.and_then(|c| cues.get(c));
        (self.fade, self.delay) = match timing {
            Timing::Cue => (target.map_or(0.0, |c| c.fade), target.map_or(0.0, |c| c.delay)),
            Timing::Snap | Timing::Manual(_) => (0.0, 0.0),
            Timing::Fixed(seconds) => (seconds.max(0.0), 0.0),
        };
        (self.manual, self.manual_origin) = match timing {
            Timing::Manual(origin) => (Some(0.0), origin),
            _ => (None, 0.0),
        };
        // Les temps propres ne valent que pour une transition au temps de la cue (Go, Goto).
        self.times = match (timing, target) {
            (Timing::Cue, Some(target)) => target.times.clone(),
            _ => HashMap::new(),
        };
        self.started = now;
        self.activated = activated;
    }

    /// Assert : l'executor reprend la main sur ce qu'il tient, en partant de ce qui est
    /// sur scène, au temps de sa cue en cours.
    pub(crate) fn assert(
        &mut self,
        cues: &[CueState],
        transition: Transition,
        is_intensity: impl Fn(&ParamKey) -> bool,
        under: impl Fn(&ParamKey) -> u16,
    ) {
        let Some(target) = self.cue.and_then(|c| cues.get(c)) else { return };
        let Transition { now, activated, .. } = transition;
        let current = self.values(cues, now, &is_intensity, &under);
        self.from = current.into_iter().map(|(k, v)| if is_intensity(&k) { (k, v) } else { (k, under(&k)) }).collect();
        (self.fade, self.delay, self.times) = (target.fade, target.delay, target.times.clone());
        self.manual = None;
        self.started = now;
        self.activated = activated;
    }

    pub(crate) fn current(&self) -> Option<usize> {
        self.cue
    }

    /// Une fois la transition finie, on oublie les valeurs de départ.
    pub(crate) fn settle(&mut self, now: Instant) {
        if self.manual.is_none() && now.saturating_duration_since(self.started).as_secs_f32() >= self.duration() {
            self.from.clear();
        }
    }

    /// La séquence a changé (cue modifiée, supprimée…) : on reste sur une cue valide.
    pub(crate) fn clamp(&mut self, len: usize) {
        if self.cue.is_some_and(|c| c >= len) {
            self.cue = len.checked_sub(1);
        }
    }

    pub(crate) fn status(&self, now: Instant, cue_count: usize) -> ExecutorStatus {
        ExecutorStatus {
            cue: self.cue,
            progress: self.progress(now),
            level: self.level,
            cue_count,
            flashing: self.flashing,
            delay: self.delay,
            fade: self.fade,
            elapsed: now.saturating_duration_since(self.started).as_secs_f32(),
            duration: self.duration(),
            step: (self.options.chaser.is_some() && self.cue.is_some()).then_some(self.step_phase as f32),
            manual: self.manual,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use folie_fixture::FixtureId;

    use super::*;

    fn key(parameter: usize) -> ParamKey {
        ParamKey { fixture: FixtureId(1), parameter }
    }

    fn cue(values: &[(usize, u16)], fade: f32, delay: f32) -> CueState {
        CueState {
            label: String::new(),
            values: values.iter().map(|(p, v)| (key(*p), *v)).collect(),
            fade,
            delay,
            times: HashMap::new(),
            effects: Vec::new(),
        }
    }

    /// Paramètre 0 = dimmer (HTP), paramètre 1 = pan (LTP).
    fn intensity(k: &ParamKey) -> bool {
        k.parameter == 0
    }

    /// Sous l'executor : le pan est au centre (valeur par défaut).
    fn under(_: &ParamKey) -> u16 {
        32_768
    }

    #[test]
    fn parameters_follow_their_own_times() {
        // Cue 2 : fondu de 2 s, mais le pan a 4 s de fondu après 1 s de délai.
        let mut second = cue(&[(0, 60_000), (1, 40_000)], 2.0, 0.0);
        second.times.insert(key(1), (4.0, 1.0));
        let cues = vec![cue(&[(0, 0), (1, 0)], 0.0, 0.0), second];
        let start = Instant::now();
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let mut pb = Playback::new();
        pb.go_to(Some(0), &cues, Transition { now: start, activated: 1, timing: Timing::Cue }, intensity, under);
        pb.settle(at(0.1));

        pb.go_to(Some(1), &cues, Transition { now: at(1.0), activated: 2, timing: Timing::Cue }, intensity, under);
        let v = pb.values(&cues, at(2.0), intensity, under);
        assert_eq!(v[&key(0)], 30_000, "dimmer au temps de la cue : mi-fondu");
        assert_eq!(v[&key(1)], 0, "pan encore dans son délai");
        let v = pb.values(&cues, at(4.0), intensity, under);
        assert_eq!(v[&key(0)], 60_000);
        assert_eq!(v[&key(1)], 20_000, "pan à mi-fondu (1 s de délai + 2 s sur 4)");
        let status = pb.status(at(4.0), cues.len());
        assert!(status.in_transition(), "la transition dure jusqu'au dernier fondu");
        assert_eq!(status.duration, 5.0);
        pb.settle(at(4.0));
        assert_eq!(pb.values(&cues, at(4.0), intensity, under)[&key(1)], 20_000, "rien n'est oublié avant la fin");
        assert_eq!(pb.values(&cues, at(6.5), intensity, under)[&key(1)], 40_000);

        // Back, Off, temps fixe : les temps propres ne s'appliquent pas.
        pb.go_to(Some(1), &cues, Transition { now: at(7.0), activated: 3, timing: Timing::Snap }, intensity, under);
        assert_eq!(pb.status(at(7.0), cues.len()).duration, 0.0);
    }

    #[test]
    fn fades_delays_and_releases() {
        let cues = vec![cue(&[(0, 60_000), (1, 10_000)], 2.0, 0.0), cue(&[(1, 30_000)], 2.0, 1.0)];
        let start = Instant::now();
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let mut pb = Playback::new();

        pb.go_to(Some(0), &cues, Transition { now: start, activated: 1, timing: Timing::Cue }, intensity, under);
        let v = pb.values(&cues, at(1.0), intensity, under);
        assert_eq!(v[&key(0)], 30_000, "dimmer à mi-fondu depuis 0");
        assert_eq!(v[&key(1)], 21_384, "pan nouveau : fondu depuis le centre (32768) vers 10000");

        pb.go_to(Some(1), &cues, Transition { now: at(2.0), activated: 2, timing: Timing::Cue }, intensity, under);
        let v = pb.values(&cues, at(2.5), intensity, under);
        assert_eq!(v[&key(1)], 10_000, "délai de 1 s : rien ne bouge encore");
        assert_eq!(v[&key(0)], 60_000, "le dimmer ne bouge pas non plus pendant le délai");
        let v = pb.values(&cues, at(4.0), intensity, under);
        assert_eq!(v[&key(1)], 20_000, "pan à mi-fondu");
        assert_eq!(v[&key(0)], 30_000, "dimmer non tenu par la cue 2 : il s'éteint en fondu");
        let v = pb.values(&cues, at(6.0), intensity, under);
        assert_eq!(v.get(&key(0)), Some(&0));
        pb.settle(at(6.0));
        assert!(!pb.values(&cues, at(6.0), intensity, under).contains_key(&key(0)), "oublié une fois fini");
    }

    #[test]
    fn ltp_fades_in_from_and_out_to_what_is_below() {
        let cues = vec![cue(&[(1, 0)], 2.0, 0.0)];
        let start = Instant::now();
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let mut pb = Playback::new();
        pb.go_to(Some(0), &cues, Transition { now: start, activated: 1, timing: Timing::Cue }, intensity, under);
        assert_eq!(pb.values(&cues, at(1.0), intensity, under)[&key(1)], 16_384, "entrée : à mi-chemin du centre vers 0");

        pb.settle(at(2.0));
        pb.go_to(None, &cues, Transition { now: at(2.0), activated: 2, timing: Timing::Fixed(2.0) }, intensity, under);
        assert_eq!(pb.values(&cues, at(3.0), intensity, under)[&key(1)], 16_384, "sortie : retour en fondu vers le centre");
        assert!(!pb.values(&cues, at(4.0), intensity, under).contains_key(&key(1)), "rendu au niveau inférieur");
    }

    #[test]
    fn restart_modes_and_chaser_directions() {
        let cues = vec![cue(&[(0, 1)], 0.0, 0.0), cue(&[(0, 2)], 0.0, 0.0), cue(&[(0, 3)], 0.0, 0.0)];
        let now = Instant::now();
        let mut pb = Playback::new();
        assert_eq!(pb.start_cue(3), Some(0), "jamais joué : la première");
        assert_eq!(pb.start_cue(0), None);
        pb.go_to(Some(1), &cues, Transition { now, activated: 1, timing: Timing::Snap }, intensity, under);
        pb.go_to(None, &cues, Transition { now, activated: 2, timing: Timing::Snap }, intensity, under);
        assert_eq!(pb.start_cue(3), Some(0), "reprise sur la première");
        pb.options.restart = Restart::Current;
        assert_eq!(pb.start_cue(3), Some(1), "reprise où il s'était arrêté");
        pb.options.restart = Restart::Next;
        assert_eq!(pb.start_cue(3), Some(2));
        pb.last = Some(2);
        assert_eq!(pb.start_cue(3), Some(2), "fin de séquence sans boucle : on y reste");
        pb.options.wrap = true;
        assert_eq!(pb.start_cue(3), Some(0));

        let mut chase = |direction, forward: &[bool]| {
            pb.options.chaser = Some(ChaserOptions { bpm: Some(120.0), step_fade: 0.0, direction });
            pb.cue = Some(0);
            pb.reset_chaser();
            forward
                .iter()
                .map(|f| {
                    let next = pb.chaser_next(3, *f).unwrap();
                    pb.cue = Some(next);
                    next
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(chase(ChaserDirection::Forward, &[true, true, true, false]), [1, 2, 0, 2]);
        assert_eq!(chase(ChaserDirection::Backward, &[true, true, false]), [2, 1, 2]);
        assert_eq!(chase(ChaserDirection::Bounce, &[true, true, true, true, true, false]), [1, 2, 1, 0, 1, 0]);
        let random = chase(ChaserDirection::Random, &[true; 20]);
        assert!(random.windows(2).all(|w| w[0] != w[1] && w[1] < 3), "hasard sans répétition : {random:?}");
    }

    #[test]
    fn crossfade_is_led_by_the_fader() {
        let cues = vec![cue(&[(0, 0), (1, 0)], 5.0, 1.0), cue(&[(0, 60_000), (1, 40_000)], 5.0, 1.0)];
        let now = Instant::now();
        let mut pb = Playback::new();
        pb.go_to(Some(0), &cues, Transition { now, activated: 1, timing: Timing::Snap }, intensity, under);
        pb.go_to(Some(1), &cues, Transition { now, activated: 2, timing: Timing::Manual(1.0) }, intensity, under);
        pb.set_manual(0.75);
        let v = pb.values(&cues, now, intensity, under);
        assert_eq!((v[&key(0)], v[&key(1)]), (15_000, 10_000), "fader descendu d'un quart, sans délai ni fondu de cue");
        assert!(pb.status(now, 2).in_transition());
        pb.set_manual(0.0);
        assert!(!pb.in_manual(), "arrivé en bas : transition finie");
        assert_eq!(pb.values(&cues, now, intensity, under)[&key(1)], 40_000);
    }

    #[test]
    fn move_in_black_presets_the_next_cue() {
        // Cue 1 : fixture éteinte, pan à 0 ; cue 2 : elle s'allume, pan à 40000 en 2 s.
        let cues = vec![cue(&[(0, 0), (1, 0)], 0.0, 0.0), cue(&[(0, 60_000), (1, 40_000)], 2.0, 0.0)];
        let start = Instant::now();
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let mut pb = Playback::new();
        pb.go_to(Some(0), &cues, Transition { now: start, activated: 1, timing: Timing::Cue }, intensity, under);
        assert_eq!(pb.values(&cues, at(1.0), intensity, under)[&key(1)], 0, "sans MIB, le pan attend le Go");

        pb.options.mib = true;
        let v = pb.values(&cues, at(1.0), intensity, under);
        assert_eq!((v[&key(0)], v[&key(1)]), (0, 20_000), "MIB : le pan bouge dans le noir, au fondu de la cue 2");
        assert_eq!(pb.values(&cues, at(3.0), intensity, under)[&key(1)], 40_000);

        // Au Go, le pan est déjà en place : seul le dimmer monte.
        pb.go_to(Some(1), &cues, Transition { now: at(3.0), activated: 2, timing: Timing::Cue }, intensity, under);
        let v = pb.values(&cues, at(4.0), intensity, under);
        assert_eq!((v[&key(0)], v[&key(1)]), (30_000, 40_000));

        // Fixture allumée dans la cue en cours : pas de MIB.
        let lit = vec![cue(&[(0, 10_000), (1, 0)], 0.0, 0.0), cues[1].clone()];
        pb.go_to(Some(0), &lit, Transition { now: at(5.0), activated: 3, timing: Timing::Snap }, intensity, under);
        assert_eq!(pb.values(&lit, at(7.0), intensity, under)[&key(1)], 0);
    }

    #[test]
    fn assert_fades_back_from_what_is_on_stage() {
        let cues = vec![cue(&[(0, 60_000), (1, 10_000)], 2.0, 0.0)];
        let start = Instant::now();
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let mut pb = Playback::new();
        pb.go_to(Some(0), &cues, Transition { now: start, activated: 1, timing: Timing::Snap }, intensity, under);
        // Sur scène, un autre executor a mis le pan au centre (32768).
        pb.assert(&cues, Transition { now: at(1.0), activated: 5, timing: Timing::Cue }, intensity, under);
        assert_eq!(pb.activated, 5, "repasse devant en LTP");
        let v = pb.values(&cues, at(2.0), intensity, under);
        assert_eq!(v[&key(1)], 21_384, "pan à mi-chemin du centre vers sa valeur");
        assert_eq!(v[&key(0)], 60_000, "l'intensité ne bouge pas");
    }
}

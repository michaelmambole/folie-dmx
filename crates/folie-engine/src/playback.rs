//! Lecture des séquences par les executors : transitions, fondus, délais.

use std::collections::HashMap;
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
    /// L'intensité reste pleine ; le fader règle la vitesse des effets (50 % = normale).
    Speed,
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
    /// Fondu de cette durée, sans délai (Off, Back).
    Fixed(f32),
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
}

impl ExecutorStatus {
    /// Une transition (délai ou fondu) est en cours.
    pub fn in_transition(&self) -> bool {
        self.elapsed < self.delay + self.fade
    }
}

pub(crate) struct Playback {
    cue: Option<usize>,
    /// Valeurs tenues au moment où la transition a commencé.
    from: HashMap<ParamKey, u16>,
    started: Instant,
    fade: f32,
    delay: f32,
    pub(crate) level: f32,
    /// Ordre de déclenchement : en LTP, le dernier executor lancé gagne.
    pub(crate) activated: u64,
    pub(crate) flashing: bool,
    pub(crate) options: PlaybackOptions,
}

impl Playback {
    pub(crate) fn new() -> Self {
        Self {
            cue: None,
            from: HashMap::new(),
            started: Instant::now(),
            fade: 0.0,
            delay: 0.0,
            level: 1.0,
            activated: 0,
            flashing: false,
            options: PlaybackOptions::default(),
        }
    }

    /// Niveau effectif de l'intensité : pleine pendant un flash, ou si le fader règle la vitesse.
    pub(crate) fn intensity_level(&self) -> f32 {
        if self.flashing || self.options.fader == FaderMode::Speed { 1.0 } else { self.level }
    }

    /// Multiplicateur de vitesse des effets : seul le fader « Vitesse » le change.
    pub(crate) fn effect_speed(&self) -> f64 {
        if self.options.fader == FaderMode::Speed { self.level as f64 * 2.0 } else { 1.0 }
    }

    fn progress(&self, now: Instant) -> f32 {
        let elapsed = now.saturating_duration_since(self.started).as_secs_f32() - self.delay;
        if elapsed < 0.0 {
            0.0
        } else if self.fade <= 0.0 {
            1.0
        } else {
            (elapsed / self.fade).min(1.0)
        }
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
        let lerp = |a: u16, b: u16| (a as f32 + (b as f32 - a as f32) * t).round() as u16;
        let target = self.cue.and_then(|c| cues.get(c)).map(|c| &c.values);
        let mut values = HashMap::new();
        if let Some(target) = target {
            for (key, &b) in target {
                let a = match self.from.get(key) {
                    Some(&a) => a,
                    None if is_intensity(key) => 0,
                    None => under(key),
                };
                values.insert(*key, lerp(a, b));
            }
        }
        // Ce que la nouvelle cue ne tient plus revient en fondu : l'intensité vers 0,
        // le reste vers ce qui est dessous. Une fois le fondu fini, on le rend.
        for (key, &a) in &self.from {
            if target.is_some_and(|t| t.contains_key(key)) {
                continue;
            }
            if is_intensity(key) {
                values.insert(*key, lerp(a, 0));
            } else if t < 1.0 {
                values.insert(*key, lerp(a, under(key)));
            }
        }
        values
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
        let target = cue.and_then(|c| cues.get(c));
        (self.fade, self.delay) = match timing {
            Timing::Cue => (target.map_or(0.0, |c| c.fade), target.map_or(0.0, |c| c.delay)),
            Timing::Snap => (0.0, 0.0),
            Timing::Fixed(seconds) => (seconds.max(0.0), 0.0),
        };
        self.started = now;
        self.activated = activated;
    }

    pub(crate) fn current(&self) -> Option<usize> {
        self.cue
    }

    /// Une fois la transition finie, on oublie les valeurs de départ.
    pub(crate) fn settle(&mut self, now: Instant) {
        if self.progress(now) >= 1.0 {
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
}
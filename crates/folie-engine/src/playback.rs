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
        }
    }

    /// Niveau effectif de l'intensité : pleine pendant un flash.
    pub(crate) fn intensity_level(&self) -> f32 {
        if self.flashing { 1.0 } else { self.level }
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
    /// `is_intensity` dit si un paramètre se mélange en HTP (il part alors de 0
    /// ou y retourne) ou en LTP (il change franchement en fin de fondu).
    pub(crate) fn values(
        &self,
        cues: &[CueState],
        now: Instant,
        is_intensity: impl Fn(&ParamKey) -> bool,
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
                    None => b,
                };
                values.insert(*key, lerp(a, b));
            }
        }
        // Ce que la nouvelle cue ne tient plus : l'intensité s'éteint en fondu,
        // le reste est rendu au niveau inférieur en fin de transition.
        for (key, &a) in &self.from {
            if target.is_some_and(|t| t.contains_key(key)) {
                continue;
            }
            if is_intensity(key) {
                values.insert(*key, lerp(a, 0));
            } else if t < 1.0 {
                values.insert(*key, a);
            }
        }
        values
    }

    /// Démarre la transition vers une cue (ou vers rien pour `None`).
    /// `snap` ignore les temps de la cue (flash : tout de suite).
    pub(crate) fn go_to(
        &mut self,
        cue: Option<usize>,
        cues: &[CueState],
        now: Instant,
        activated: u64,
        snap: bool,
        is_intensity: impl Fn(&ParamKey) -> bool,
    ) {
        self.from = self.values(cues, now, is_intensity);
        self.cue = cue;
        let target = cue.and_then(|c| cues.get(c)).filter(|_| !snap);
        self.fade = target.map_or(0.0, |c| c.fade);
        self.delay = target.map_or(0.0, |c| c.delay);
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

    #[test]
    fn fades_delays_and_releases() {
        let cues = vec![cue(&[(0, 60_000), (1, 10_000)], 2.0, 0.0), cue(&[(1, 30_000)], 2.0, 1.0)];
        let start = Instant::now();
        let at = |s: f32| start + Duration::from_secs_f32(s);
        let mut pb = Playback::new();

        pb.go_to(Some(0), &cues, start, 1, false, intensity);
        let v = pb.values(&cues, at(1.0), intensity);
        assert_eq!(v[&key(0)], 30_000, "dimmer à mi-fondu depuis 0");
        assert_eq!(v[&key(1)], 10_000, "pan nouveau : pris tel quel");

        pb.go_to(Some(1), &cues, at(2.0), 2, false, intensity);
        let v = pb.values(&cues, at(2.5), intensity);
        assert_eq!(v[&key(1)], 10_000, "délai de 1 s : rien ne bouge encore");
        assert_eq!(v[&key(0)], 60_000, "le dimmer ne bouge pas non plus pendant le délai");
        let v = pb.values(&cues, at(4.0), intensity);
        assert_eq!(v[&key(1)], 20_000, "pan à mi-fondu");
        assert_eq!(v[&key(0)], 30_000, "dimmer non tenu par la cue 2 : il s'éteint en fondu");
        let v = pb.values(&cues, at(6.0), intensity);
        assert_eq!(v.get(&key(0)), Some(&0));
        pb.settle(at(6.0));
        assert!(!pb.values(&cues, at(6.0), intensity).contains_key(&key(0)), "oublié une fois fini");
    }
}

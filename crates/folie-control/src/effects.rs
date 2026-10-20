//! Effets du programmer et tempo.

use std::time::{Duration, Instant};

use folie_engine::{Command as EngineCommand, EffectInstance, EffectLine as EngineLine, Rate, Waveform as EngineWave};
use folie_fixture::{ParamKey, Patch};
use folie_show::{Effect, EffectRate, Waveform, effects::templates};

use crate::{CommandError, Console, Outcome, invalid};

/// Identifiants des effets du programmer, à l'écart de ceux des cues.
const PROGRAMMER_IDS: u64 = 1 << 62;

/// Au-delà, un tap recommence une nouvelle mesure du tempo.
const TAP_TIMEOUT: Duration = Duration::from_secs(2);

/// Forme d'onde du moteur, qui sait la calculer.
pub fn waveform(w: Waveform) -> EngineWave {
    match w {
        Waveform::Sine => EngineWave::Sine,
        Waveform::RampUp => EngineWave::RampUp,
        Waveform::RampDown => EngineWave::RampDown,
        Waveform::Square => EngineWave::Square,
        Waveform::Triangle => EngineWave::Triangle,
    }
}

fn percent(p: f32) -> u16 {
    (p.clamp(0.0, 100.0) / 100.0 * u16::MAX as f32).round() as u16
}

/// Traduit un effet en paramètres concrets : pour chaque cible, les paramètres
/// de l'attribut visé, avec la phase de la cible et celle de la ligne.
pub fn instance(effect: &Effect, id: u64, patch: &Patch) -> EffectInstance {
    let lines = effect
        .lines
        .iter()
        .map(|line| {
            let params = effect
                .targets
                .iter()
                .enumerate()
                .flat_map(|(index, target)| {
                    let phase = effect.phase_of(index) + line.phase / 360.0;
                    patch
                        .parameters_of(*target)
                        .into_iter()
                        .filter(|(_, p)| p.attribute == line.attribute)
                        .map(move |(parameter, _)| (ParamKey { fixture: target.fixture, parameter }, phase))
                })
                .collect();
            // Relatif : écarts signés autour de la valeur en cours ; sinon valeurs de 0 à 100 %.
            let level = |p: f32| {
                if line.relative { (p.clamp(-100.0, 100.0) / 100.0 * u16::MAX as f32).round() as i32 } else { percent(p) as i32 }
            };
            EngineLine { waveform: waveform(line.waveform), low: level(line.low), high: level(line.high), relative: line.relative, params }
        })
        .collect();
    let rate = match effect.rate {
        EffectRate::Hz(hz) => Rate::Hz(hz),
        EffectRate::Beats(beats) => Rate::Beats(beats),
    };
    EffectInstance { id, rate, lines }
}

/// Mesure du tempo par taps successifs.
#[derive(Debug, Default)]
pub struct TapTempo {
    taps: Vec<Instant>,
}

impl TapTempo {
    /// Enregistre un tap ; renvoie le tempo dès qu'il y a deux taps rapprochés.
    pub fn tap(&mut self, now: Instant) -> Option<f32> {
        if self.taps.last().is_some_and(|last| now.saturating_duration_since(*last) > TAP_TIMEOUT) {
            self.taps.clear();
        }
        self.taps.push(now);
        // Les 4 derniers intervalles suffisent et suivent un tempo qui bouge.
        let start = self.taps.len().saturating_sub(5);
        let taps = &self.taps[start..];
        if taps.len() < 2 {
            return None;
        }
        let span = taps[taps.len() - 1].saturating_duration_since(taps[0]).as_secs_f32();
        let bpm = 60.0 * (taps.len() - 1) as f32 / span;
        bpm.is_finite().then(|| bpm.clamp(1.0, 999.0))
    }
}

impl Console {
    pub fn programmer_effects(&self) -> &[Effect] {
        &self.effects
    }

    /// Pour la fenêtre Effets ; appeler [`Console::effects_changed`] ensuite.
    pub fn programmer_effects_mut(&mut self) -> &mut Vec<Effect> {
        &mut self.effects
    }

    pub fn effects_changed(&self) {
        let instances = self
            .effects
            .iter()
            .enumerate()
            .map(|(i, e)| instance(e, PROGRAMMER_IDS + i as u64, &self.show.patch))
            .collect();
        self.engine.send(EngineCommand::SetProgrammerEffects(instances));
    }

    /// Lance l'effet modèle n° `number` (à partir de 1) sur la sélection.
    pub fn apply_effect(&mut self, number: u32) -> Result<Outcome, CommandError> {
        let all = templates();
        let Some(template) = number.checked_sub(1).and_then(|i| all.get(i as usize)) else {
            return invalid(format!("l'effet {number} n'existe pas (1 à {})", all.len()));
        };
        if self.selection.is_empty() {
            return invalid("aucune sélection");
        }
        let mut effect = template.clone();
        effect.targets = self.selection.clone();
        let name = effect.name.clone();
        self.effects.push(effect);
        self.effects_changed();
        self.selection_used = true;
        Ok(Outcome::info(format!("Effet « {name} » sur {} cible(s)", self.selection.len())))
    }

    pub fn remove_effect(&mut self, index: usize) {
        if index < self.effects.len() {
            self.effects.remove(index);
            self.effects_changed();
        }
    }

    pub fn tempo(&self) -> f32 {
        self.show.tempo
    }

    pub fn set_tempo(&mut self, bpm: f32) -> Outcome {
        self.show.tempo = bpm.clamp(1.0, 999.0);
        self.engine.send(EngineCommand::SetTempo(self.show.tempo));
        Outcome::changed(format!("Tempo {:.1} BPM", self.show.tempo))
    }

    pub fn tap_tempo(&mut self) -> Option<Outcome> {
        let bpm = self.taps.tap(Instant::now())?;
        Some(self.set_tempo(bpm))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_tempo_measures_and_resets() {
        let mut tap = TapTempo::default();
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        assert_eq!(tap.tap(at(0)), None);
        assert_eq!(tap.tap(at(500)), Some(120.0));
        assert_eq!(tap.tap(at(1000)), Some(120.0));
        // Longue pause : on repart à zéro.
        assert_eq!(tap.tap(at(5000)), None);
        let bpm = tap.tap(at(5400)).unwrap();
        assert!((bpm - 150.0).abs() < 0.01, "{bpm}");
    }
}

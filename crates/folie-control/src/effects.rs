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
        Waveform::Random => EngineWave::Random,
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
            EngineLine { waveform: waveform(line.waveform), low: level(line.low), high: level(line.high), relative: line.relative, duty: line.duty.clamp(1.0, 99.0) / 100.0, params }
        })
        .collect();
    let rate = match effect.rate {
        EffectRate::Hz(hz) => Rate::Hz(hz),
        EffectRate::Beats(beats) => Rate::Beats(beats),
    };
    EffectInstance { id, rate, lines, fade_in: effect.fade_in, fade_out: effect.fade_out }
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
        self.send_programmer(EngineCommand::SetProgrammerEffects(instances));
    }

    /// Lance l'effet modèle n° `number` (à partir de 1) sur la sélection.
    pub fn apply_effect(&mut self, number: u32) -> Result<Outcome, CommandError> {
        let all = templates();
        let Some(template) = number.checked_sub(1).and_then(|i| all.get(i as usize)) else {
            return invalid(format!("l'effet {number} n'existe pas (1 à {})", all.len()));
        };
        self.launch(template.clone())
    }

    /// Lance un effet (modèle ou du pool) sur la sélection, dans le programmer.
    fn launch(&mut self, mut effect: Effect) -> Result<Outcome, CommandError> {
        if self.selection.is_empty() {
            return invalid("aucune sélection");
        }
        effect.targets = self.selection.clone();
        let name = effect.name.clone();
        self.effects.push(effect);
        self.effects_changed();
        self.selection_used = true;
        Ok(Outcome::info(format!("Effet « {name} » sur {} cible(s)", self.selection.len())))
    }

    /// « Au pool » : l'effet du programmer, sans ses cibles, dans la première case libre.
    pub fn store_effect_in_pool(&mut self, index: usize) -> Result<Outcome, CommandError> {
        let Some(effect) = self.effects.get(index) else { return invalid("cet effet n'est plus dans le programmer") };
        let mut effect = effect.clone();
        effect.targets.clear();
        let number = (1..).find(|n| !self.show.effect_pool.contains_key(n)).expect("toujours une case libre");
        let name = effect.name.clone();
        self.show.effect_pool.insert(number, effect);
        Ok(Outcome::changed(format!("Effet « {name} » enregistré dans Mes effets ({number})")))
    }

    /// Relance un effet du pool sur la sélection.
    pub fn apply_pool_effect(&mut self, number: u32) -> Result<Outcome, CommandError> {
        let Some(effect) = self.show.effect_pool.get(&number).cloned() else {
            return invalid(format!("Mes effets : la case {number} est vide"));
        };
        self.launch(effect)
    }

    pub fn rename_pool_effect(&mut self, number: u32, name: String) -> Result<Outcome, CommandError> {
        let Some(effect) = self.show.effect_pool.get_mut(&number) else {
            return invalid(format!("Mes effets : la case {number} est vide"));
        };
        effect.name = name.clone();
        Ok(Outcome::changed(format!("Mes effets {number} renommé « {name} »")))
    }

    pub fn delete_pool_effect(&mut self, number: u32) -> Result<Outcome, CommandError> {
        match self.show.effect_pool.remove(&number) {
            Some(effect) => Ok(Outcome::changed(format!("Effet « {} » retiré de Mes effets", effect.name))),
            None => invalid(format!("Mes effets : la case {number} est vide")),
        }
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

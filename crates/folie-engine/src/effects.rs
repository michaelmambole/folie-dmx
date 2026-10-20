//! Générateurs d'effets : un attribut oscille entre une valeur basse et une
//! valeur haute, avec un décalage de phase propre à chaque fixture.

use std::collections::HashMap;

use folie_fixture::ParamKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waveform {
    Sine,
    RampUp,
    RampDown,
    Square,
    Triangle,
}

impl Waveform {
    /// Niveau entre 0 et 1 pour une phase entre 0 et 1.
    pub fn level(self, phase: f64) -> f64 {
        let p = phase.rem_euclid(1.0);
        match self {
            // Commence en bas, comme une respiration.
            Self::Sine => 0.5 - 0.5 * (p * std::f64::consts::TAU).cos(),
            Self::RampUp => p,
            Self::RampDown => 1.0 - p,
            Self::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Triangle => 1.0 - (2.0 * p - 1.0).abs(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rate {
    /// Cycles par seconde.
    Hz(f32),
    /// Durée d'un cycle en temps, calée sur le tempo : 1 = un cycle par temps.
    Beats(f32),
}

impl Rate {
    pub fn hz(self, bpm: f32) -> f64 {
        match self {
            Self::Hz(hz) => hz.max(0.0) as f64,
            Self::Beats(beats) if beats > 0.0 => bpm.max(0.0) as f64 / 60.0 / beats as f64,
            Self::Beats(_) => 0.0,
        }
    }
}

/// Une ligne d'effet : une forme d'onde appliquée à des paramètres, chacun
/// avec son décalage de phase (0 à 1).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectLine {
    pub waveform: Waveform,
    /// Valeurs basse et haute sur 16 bits ; relatives, ce sont des écarts (signés).
    pub low: i32,
    pub high: i32,
    /// Relatif : l'effet oscille autour de la valeur qui est sous lui (programmer, cue,
    /// valeur par défaut) au lieu d'aller d'une valeur fixe à une autre.
    pub relative: bool,
    pub params: Vec<(ParamKey, f32)>,
}

/// Effet prêt à jouer. `id` est stable tant que l'effet est le même : la
/// phase continue alors sans à-coup quand on retouche un réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectInstance {
    pub id: u64,
    pub rate: Rate,
    pub lines: Vec<EffectLine>,
}

impl EffectInstance {
    /// Valeurs de l'effet à cette phase. `base` donne la valeur sous l'effet, autour de
    /// laquelle tournent les lignes relatives.
    pub fn values(&self, phase: f64, base: impl Fn(&ParamKey) -> u16) -> HashMap<ParamKey, u16> {
        let mut values = HashMap::new();
        for line in &self.lines {
            let (low, high) = (line.low as f64, line.high as f64);
            for (key, offset) in &line.params {
                let level = line.waveform.level(phase + *offset as f64);
                let mut value = low + (high - low) * level;
                if line.relative {
                    value += base(key) as f64;
                }
                values.insert(*key, value.round().clamp(0.0, u16::MAX as f64) as u16);
            }
        }
        values
    }
}

/// Phase de chaque effet en cours, avancée à chaque trame.
#[derive(Debug, Default)]
pub(crate) struct Phases {
    phases: HashMap<u64, f64>,
    used: Vec<u64>,
}

impl Phases {
    /// Avance un effet de `dt` secondes et renvoie sa phase.
    pub(crate) fn advance(&mut self, effect: &EffectInstance, dt: f64, bpm: f32) -> f64 {
        self.used.push(effect.id);
        let phase = self.phases.entry(effect.id).or_insert(0.0);
        *phase = (*phase + dt * effect.rate.hz(bpm)).rem_euclid(1.0);
        *phase
    }

    /// Nombre d'effets qui ont tourné pendant cette trame.
    pub(crate) fn running(&self) -> usize {
        let mut used = self.used.clone();
        used.sort_unstable();
        used.dedup();
        used.len()
    }

    /// Oublie les effets qui n'ont pas tourné pendant cette trame.
    pub(crate) fn end_frame(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.phases.retain(|id, _| used.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use folie_fixture::FixtureId;

    use super::*;

    #[test]
    fn waveforms() {
        assert!((Waveform::Sine.level(0.0)).abs() < 1e-9);
        assert!((Waveform::Sine.level(0.5) - 1.0).abs() < 1e-9);
        assert_eq!(Waveform::RampUp.level(0.25), 0.25);
        assert_eq!(Waveform::RampDown.level(0.25), 0.75);
        assert_eq!((Waveform::Square.level(0.2), Waveform::Square.level(0.7)), (1.0, 0.0));
        assert_eq!(Waveform::Triangle.level(0.5), 1.0);
        assert_eq!(Waveform::RampUp.level(1.25), 0.25, "la phase boucle");
    }

    #[test]
    fn rate_follows_tempo() {
        assert_eq!(Rate::Hz(2.0).hz(90.0), 2.0);
        assert_eq!(Rate::Beats(1.0).hz(120.0), 2.0, "un cycle par temps à 120 BPM");
        assert_eq!(Rate::Beats(4.0).hz(120.0), 0.5);
    }

    #[test]
    fn phase_spread_and_continuity() {
        let key = |p| ParamKey { fixture: FixtureId(p), parameter: 0 };
        let effect = EffectInstance {
            id: 7,
            rate: Rate::Hz(1.0),
            lines: vec![EffectLine {
                waveform: Waveform::RampUp,
                low: 0,
                high: 1000,
                relative: false,
                params: vec![(key(1), 0.0), (key(2), 0.5)],
            }],
        };
        let mut phases = Phases::default();
        let phase = phases.advance(&effect, 0.25, 120.0);
        let v = effect.values(phase, |_| 0);
        assert_eq!((v[&key(1)], v[&key(2)]), (250, 750), "la 2e fixture a une demi-période d'avance");

        // Relatif : autour de la valeur sous l'effet, sans sortir de 0–65535.
        let around = EffectInstance {
            lines: vec![EffectLine { low: -2000, high: 2000, relative: true, ..effect.lines[0].clone() }],
            ..effect.clone()
        };
        let v = around.values(0.25, |k| if k.fixture.0 == 1 { 30_000 } else { 65_000 });
        assert_eq!(v[&key(1)], 29_000, "un quart de rampe : -1000 autour de 30000");
        assert_eq!(v[&key(2)], 65_535, "borné en haut");

        // Changer la vitesse ne fait pas sauter la phase : elle repart d'où elle était.
        let faster = EffectInstance { rate: Rate::Hz(2.0), ..effect.clone() };
        let phase = phases.advance(&faster, 0.125, 120.0);
        assert!((phase - 0.5).abs() < 1e-9);
        phases.end_frame();
        phases.end_frame();
        assert!(phases.phases.is_empty(), "effet arrêté : sa phase est oubliée");
    }
}

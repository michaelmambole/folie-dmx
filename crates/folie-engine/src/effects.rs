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
    pub low: u16,
    pub high: u16,
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
    pub fn values(&self, phase: f64) -> HashMap<ParamKey, u16> {
        let mut values = HashMap::new();
        for line in &self.lines {
            let (low, high) = (line.low as f64, line.high as f64);
            for (key, offset) in &line.params {
                let level = line.waveform.level(phase + *offset as f64);
                values.insert(*key, (low + (high - low) * level).round() as u16);
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
                params: vec![(key(1), 0.0), (key(2), 0.5)],
            }],
        };
        let mut phases = Phases::default();
        let phase = phases.advance(&effect, 0.25, 120.0);
        let v = effect.values(phase);
        assert_eq!((v[&key(1)], v[&key(2)]), (250, 750), "la 2e fixture a une demi-période d'avance");

        // Changer la vitesse ne fait pas sauter la phase : elle repart d'où elle était.
        let faster = EffectInstance { rate: Rate::Hz(2.0), ..effect.clone() };
        let phase = phases.advance(&faster, 0.125, 120.0);
        assert!((phase - 0.5).abs() < 1e-9);
        phases.end_frame();
        phases.end_frame();
        assert!(phases.phases.is_empty(), "effet arrêté : sa phase est oubliée");
    }
}

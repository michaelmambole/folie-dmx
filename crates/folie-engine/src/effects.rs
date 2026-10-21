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
    /// Une valeur tirée au hasard à chaque cycle, gardée jusqu'au suivant.
    Random,
}

/// Nombre pseudo-aléatoire de 0 à 1, toujours le même pour la même graine.
fn noise(seed: u64) -> f64 {
    // splitmix64
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) as f64 / u64::MAX as f64
}

impl Waveform {
    /// Niveau entre 0 et 1 pour une phase (en cycles), carré allumé la moitié du cycle.
    pub fn level(self, phase: f64) -> f64 {
        self.level_with(phase, 0.5, 0)
    }

    /// Niveau entre 0 et 1. `duty` : part du cycle où le carré est allumé ; `seed` :
    /// propre à chaque paramètre, pour que l'aléatoire diffère d'un projecteur à l'autre.
    pub fn level_with(self, phase: f64, duty: f64, seed: u64) -> f64 {
        let p = phase.rem_euclid(1.0);
        match self {
            // Commence en bas, comme une respiration.
            Self::Sine => 0.5 - 0.5 * (p * std::f64::consts::TAU).cos(),
            Self::RampUp => p,
            Self::RampDown => 1.0 - p,
            Self::Square => {
                if p < duty {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Triangle => 1.0 - (2.0 * p - 1.0).abs(),
            Self::Random => noise(seed ^ (phase.floor() as i64 as u64).wrapping_mul(0x2545_F491_4F6C_DD1D)),
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
    /// Part du cycle où un carré est allumé, de 0 à 1.
    pub duty: f32,
    pub params: Vec<(ParamKey, f32)>,
}

/// Effet prêt à jouer. `id` est stable tant que l'effet est le même : la
/// phase continue alors sans à-coup quand on retouche un réglage.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectInstance {
    pub id: u64,
    pub rate: Rate,
    pub lines: Vec<EffectLine>,
    /// Secondes pour que l'effet grandisse depuis ce qui est sous lui, à son lancement.
    pub fade_in: f32,
    /// Secondes pour qu'il s'efface quand il s'arrête, au lieu de couper net.
    pub fade_out: f32,
}

impl EffectInstance {
    /// Valeurs de l'effet à cette phase. `base` donne la valeur sous l'effet : les lignes
    /// relatives tournent autour, et `size` (0 à 1, pendant les fondus) y ramène l'effet.
    pub fn values(&self, phase: f64, size: f64, base: impl Fn(&ParamKey) -> u16) -> HashMap<ParamKey, u16> {
        let mut values = HashMap::new();
        for line in &self.lines {
            let (low, high) = (line.low as f64, line.high as f64);
            for (key, offset) in &line.params {
                let seed = ((key.fixture.0 as u64) << 20) ^ key.parameter as u64;
                let level = line.waveform.level_with(phase + *offset as f64, line.duty as f64, seed);
                let under = base(key) as f64;
                let mut value = low + (high - low) * level;
                if line.relative {
                    value += under;
                }
                let value = under + (value - under) * size.clamp(0.0, 1.0);
                values.insert(*key, value.round().clamp(0.0, u16::MAX as f64) as u16);
            }
        }
        values
    }

    /// Taille de l'effet `age` secondes après son lancement.
    fn size_at(&self, age: f64) -> f64 {
        if self.fade_in > 0.0 { (age / self.fade_in as f64).min(1.0) } else { 1.0 }
    }
}

/// Un effet en cours : sa phase (en cycles) et depuis combien de temps il tourne.
#[derive(Debug, Clone, Copy, Default)]
struct Running {
    phase: f64,
    age: f64,
}

/// Un effet qui vient de s'arrêter et s'efface.
#[derive(Debug, Clone)]
struct Ending {
    effect: EffectInstance,
    phase: f64,
    /// Taille au moment de l'arrêt.
    size: f64,
    /// Secondes de fondu de sortie restantes.
    left: f64,
}

/// Phases et fondus des effets, couche par couche : chaque executor, et le programmer.
#[derive(Debug, Default)]
pub(crate) struct Phases {
    running: HashMap<u64, Running>,
    /// Effets de chaque couche à la trame précédente, pour voir lesquels disparaissent.
    layers: HashMap<u64, Vec<EffectInstance>>,
    ending: HashMap<u64, Vec<Ending>>,
    used: Vec<u64>,
}

impl Phases {
    /// Fait avancer les effets d'une couche (`owner`) : `dt_effect` pour la phase (le fader
    /// Vitesse le change), `dt` pour les fondus. Renvoie chaque effet à jouer, avec sa phase et
    /// sa taille, y compris ceux qui viennent de s'arrêter et s'effacent. À appeler à chaque
    /// trame, même sans effet, pour que leurs fondus de sortie aient lieu.
    pub(crate) fn layer(
        &mut self,
        owner: u64,
        effects: &[EffectInstance],
        dt_effect: f64,
        dt: f64,
        bpm: f32,
    ) -> Vec<(EffectInstance, f64, f64)> {
        let mut out = Vec::new();
        let ending = self.ending.entry(owner).or_default();
        for effect in effects {
            self.used.push(effect.id);
            // Relancé pendant qu'il s'effaçait : il reprend où il en était.
            ending.retain(|e| e.effect.id != effect.id);
            // La phase compte les cycles sans revenir à 0 : l'aléatoire change à chaque cycle.
            let running = self.running.entry(effect.id).or_default();
            running.phase += dt_effect * effect.rate.hz(bpm);
            running.age += dt;
            out.push((effect.clone(), running.phase, effect.size_at(running.age)));
        }
        if let Some(previous) = self.layers.insert(owner, effects.to_vec()) {
            for effect in previous.into_iter().filter(|p| p.fade_out > 0.0 && !effects.iter().any(|e| e.id == p.id)) {
                let running = self.running.get(&effect.id).copied().unwrap_or_default();
                let size = effect.size_at(running.age);
                let left = effect.fade_out as f64;
                ending.push(Ending { effect, phase: running.phase, size, left });
            }
        }
        for e in ending.iter_mut() {
            e.left -= dt;
            e.phase += dt_effect * e.effect.rate.hz(bpm);
            if e.left > 0.0 {
                let size = e.size * e.left / e.effect.fade_out as f64;
                out.push((e.effect.clone(), e.phase, size));
            }
        }
        ending.retain(|e| e.left > 0.0);
        out
    }

    /// Nombre d'effets qui ont tourné pendant cette trame, en comptant ceux qui s'effacent.
    pub(crate) fn running(&self) -> usize {
        let mut used = self.used.clone();
        used.sort_unstable();
        used.dedup();
        used.len() + self.ending.values().map(Vec::len).sum::<usize>()
    }

    /// Oublie les effets qui n'ont pas tourné pendant cette trame.
    pub(crate) fn end_frame(&mut self) {
        let used = std::mem::take(&mut self.used);
        self.running.retain(|id, _| used.contains(id));
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
        assert_eq!(Waveform::Square.level_with(0.3, 0.2, 0), 0.0, "carré allumé 20 % du cycle");
        assert_eq!(Waveform::Square.level_with(0.1, 0.2, 0), 1.0);
        // Aléatoire : constant pendant un cycle, différent au suivant et d'une graine à l'autre.
        let random = |phase: f64, seed: u64| Waveform::Random.level_with(phase, 0.5, seed);
        assert_eq!(random(3.1, 7), random(3.9, 7));
        assert_ne!(random(3.1, 7), random(4.1, 7));
        assert_ne!(random(3.1, 7), random(3.1, 8));
        assert!((0..100).map(|c| random(c as f64, 1)).all(|v| (0.0..=1.0).contains(&v)));
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
                duty: 0.5,
                params: vec![(key(1), 0.0), (key(2), 0.5)],
            }],
            fade_in: 0.0,
            fade_out: 0.0,
        };
        let mut phases = Phases::default();
        let phase = phases.layer(1, std::slice::from_ref(&effect), 0.25, 0.25, 120.0)[0].1;
        let v = effect.values(phase, 1.0, |_| 0);
        assert_eq!((v[&key(1)], v[&key(2)]), (250, 750), "la 2e fixture a une demi-période d'avance");

        // Relatif : autour de la valeur sous l'effet, sans sortir de 0–65535.
        let around = EffectInstance {
            lines: vec![EffectLine { low: -2000, high: 2000, relative: true, ..effect.lines[0].clone() }],
            ..effect.clone()
        };
        let v = around.values(0.25, 1.0, |k| if k.fixture.0 == 1 { 30_000 } else { 65_000 });
        assert_eq!(v[&key(1)], 29_000, "un quart de rampe : -1000 autour de 30000");
        assert_eq!(v[&key(2)], 65_535, "borné en haut");

        // Changer la vitesse ne fait pas sauter la phase : elle repart d'où elle était.
        let faster = EffectInstance { rate: Rate::Hz(2.0), ..effect.clone() };
        let phase = phases.layer(1, std::slice::from_ref(&faster), 0.125, 0.125, 120.0)[0].1;
        assert!((phase - 0.5).abs() < 1e-9);
        phases.end_frame();
        phases.layer(1, &[], 0.1, 0.1, 120.0);
        phases.end_frame();
        assert!(phases.running.is_empty(), "effet arrêté : sa phase est oubliée");
    }

    #[test]
    fn effects_fade_in_and_out() {
        let key = ParamKey { fixture: FixtureId(1), parameter: 0 };
        let effect = EffectInstance {
            id: 3,
            rate: Rate::Hz(0.0),
            lines: vec![EffectLine {
                waveform: Waveform::Square,
                low: 60_000,
                high: 60_000,
                relative: false,
                duty: 0.5,
                params: vec![(key, 0.0)],
            }],
            fade_in: 2.0,
            fade_out: 4.0,
        };
        let value = |played: &[(EffectInstance, f64, f64)]| {
            let (e, phase, size) = &played[0];
            e.values(*phase, *size, |_| 20_000)[&key]
        };
        let mut phases = Phases::default();
        let played = phases.layer(9, std::slice::from_ref(&effect), 1.0, 1.0, 120.0);
        assert_eq!(value(&played), 40_000, "mi-entrée : à mi-chemin de 20000 vers 60000");
        phases.end_frame();
        let played = phases.layer(9, std::slice::from_ref(&effect), 1.0, 1.0, 120.0);
        assert_eq!(value(&played), 60_000, "entré");
        phases.end_frame();

        // Arrêté : il s'efface en 4 s au lieu de couper.
        let played = phases.layer(9, &[], 1.0, 1.0, 120.0);
        assert_eq!(value(&played), 50_000, "3 s de sortie restantes sur 4");
        assert_eq!(phases.running(), 1, "compté tant qu'il s'efface");
        phases.end_frame();
        for _ in 0..3 {
            phases.layer(9, &[], 1.0, 1.0, 120.0);
            phases.end_frame();
        }
        assert!(phases.layer(9, &[], 1.0, 1.0, 120.0).is_empty(), "effacé");
        assert_eq!(phases.running(), 0);
    }
}

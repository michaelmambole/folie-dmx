//! Analyse du son, par petits paquets (~10 ms) : niveau global et par bande, beat, tempo.
//!
//! Volontairement simple et sans FFT : trois filtres (graves sous 150 Hz, médiums jusqu'à
//! 2 kHz, aigus au-dessus), un contrôle de gain automatique par bande (chaque bande va de
//! 0 à 1 quel que soit le volume d'entrée), et un beat détecté sur les graves quand leur
//! énergie dépasse nettement sa moyenne de la dernière seconde.

use std::collections::VecDeque;
use std::f32::consts::PI;

/// Échantillons par paquet d'analyse : ~10 ms à 48 kHz.
const HOP: usize = 512;
/// Beats au plus rapprochés : 250 ms, soit 240 BPM.
const MIN_BEAT_GAP: f64 = 0.25;
/// Plage de tempo retenue (les intervalles sont ramenés dedans en doublant ou divisant).
const MIN_BPM: f64 = 70.0;
const MAX_BPM: f64 = 180.0;
/// En dessous, c'est le silence : pas de beat, niveaux à zéro.
const SILENCE: f32 = 1e-4;

/// Filtre du second ordre (formules RBJ).
#[derive(Clone, Copy)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn new(kind: Kind, frequency: f32, sample_rate: f32) -> Self {
        let w = 2.0 * PI * frequency / sample_rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let (b, a0, a) = match kind {
            Kind::LowPass => ([(1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0], 1.0 + alpha, [-2.0 * cos, 1.0 - alpha]),
            Kind::HighPass => ([(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0], 1.0 + alpha, [-2.0 * cos, 1.0 - alpha]),
        };
        Self { b: b.map(|v| v / a0), a: a.map(|v| v / a0), x: [0.0; 2], y: [0.0; 2] }
    }

    fn run(&mut self, input: f32) -> f32 {
        let out = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [out, self.y[0]];
        out
    }
}

#[derive(Clone, Copy)]
enum Kind {
    LowPass,
    HighPass,
}

/// Une bande : son énergie, ramenée de 0 à 1 par un gain automatique.
#[derive(Clone, Copy, Default)]
struct Band {
    /// Crête qui redescend lentement : la référence du gain automatique.
    peak: f32,
    /// Valeur affichée, montée rapide et descente douce.
    value: f32,
}

impl Band {
    /// `rms` du paquet ; `hop` en secondes.
    fn update(&mut self, rms: f32, hop: f32) -> f32 {
        // La crête redescend de moitié en 3 s : le gain s'adapte à un morceau plus calme.
        self.peak = rms.max(self.peak * 0.5f32.powf(hop / 3.0)).max(SILENCE);
        let target = if rms <= SILENCE { 0.0 } else { (rms / self.peak).clamp(0.0, 1.0) };
        // Montée immédiate, descente en ~150 ms : la lumière suit les attaques sans trembler.
        if target > self.value {
            self.value = target;
        } else {
            self.value += (target - self.value) * (1.0 - (-hop / 0.15).exp());
        }
        self.value
    }
}

/// Niveaux de 0 à 1.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Levels {
    pub level: f32,
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
}

/// Résultat d'un paquet.
#[derive(Debug, Clone, Copy)]
pub struct Hop {
    pub levels: Levels,
    pub beat: bool,
    /// Temps écoulé depuis le début de l'analyse, en secondes.
    pub time: f64,
}

pub struct Analyzer {
    sample_rate: f32,
    gain: f32,
    bass_filter: Biquad,
    mid_low: Biquad,
    mid_high: Biquad,
    high_filter: Biquad,
    buffer: Vec<[f32; 4]>,
    bands: [Band; 4],
    /// Énergie des graves sur la dernière seconde, pour la détection de beat.
    history: VecDeque<f32>,
    previous_energy: f32,
    samples: u64,
    beats: VecDeque<f64>,
    bpm: Option<f32>,
}

impl Analyzer {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            gain: 1.0,
            bass_filter: Biquad::new(Kind::LowPass, 150.0, sample_rate),
            mid_low: Biquad::new(Kind::HighPass, 150.0, sample_rate),
            mid_high: Biquad::new(Kind::LowPass, 2000.0, sample_rate),
            high_filter: Biquad::new(Kind::HighPass, 2000.0, sample_rate),
            buffer: Vec::with_capacity(HOP),
            bands: [Band::default(); 4],
            history: VecDeque::new(),
            previous_energy: 0.0,
            samples: 0,
            beats: VecDeque::new(),
            bpm: None,
        }
    }

    /// Gain d'entrée (1 : tel quel), avant le gain automatique : sert surtout à sortir
    /// une entrée trop faible du seuil de silence.
    pub fn set_gain(&mut self, gain: f32) {
        self.gain = gain.max(0.0);
    }

    /// Tempo estimé à partir des derniers beats, s'il est assez régulier.
    pub fn bpm(&self) -> Option<f32> {
        self.bpm
    }

    /// Analyse des échantillons mono ; `on_hop` est appelé à chaque paquet complet.
    pub fn push(&mut self, samples: &[f32], mut on_hop: impl FnMut(Hop)) {
        for &sample in samples {
            let s = sample * self.gain;
            let bass = self.bass_filter.run(s);
            let mid = self.mid_high.run(self.mid_low.run(s));
            let high = self.high_filter.run(s);
            self.buffer.push([s, bass, mid, high]);
            self.samples += 1;
            if self.buffer.len() == HOP {
                let hop = self.analyse();
                self.buffer.clear();
                on_hop(hop);
            }
        }
    }

    fn analyse(&mut self) -> Hop {
        let n = self.buffer.len() as f32;
        let mut energy = [0.0f32; 4];
        for frame in &self.buffer {
            for (e, v) in energy.iter_mut().zip(frame) {
                *e += v * v;
            }
        }
        let energy = energy.map(|e| e / n);
        let hop = n / self.sample_rate;
        let rms = energy.map(f32::sqrt);
        let mut values = [0.0f32; 4];
        for (i, band) in self.bands.iter_mut().enumerate() {
            values[i] = band.update(rms[i], hop);
        }
        let time = self.samples as f64 / self.sample_rate as f64;
        let beat = self.detect_beat(energy[1], time);
        Hop { levels: Levels { level: values[0], bass: values[1], mid: values[2], high: values[3] }, beat, time }
    }

    /// Beat : les graves montent et dépassent nettement leur moyenne récente.
    fn detect_beat(&mut self, energy: f32, time: f64) -> bool {
        let seconds = self.sample_rate / HOP as f32;
        let average = if self.history.is_empty() { 0.0 } else { self.history.iter().sum::<f32>() / self.history.len() as f32 };
        self.history.push_back(energy);
        while self.history.len() as f32 > seconds {
            self.history.pop_front();
        }
        let rising = energy > self.previous_energy;
        self.previous_energy = energy;
        let loud = energy.sqrt() > SILENCE * 10.0;
        let spaced = self.beats.back().is_none_or(|last| time - last >= MIN_BEAT_GAP);
        let beat = rising && loud && spaced && energy > average * 1.5;
        if beat {
            self.beats.push_back(time);
            while self.beats.len() > 12 {
                self.beats.pop_front();
            }
            self.estimate_tempo();
        }
        beat
    }

    /// Médiane des écarts entre beats, ramenés entre 70 et 180 BPM.
    fn estimate_tempo(&mut self) {
        let mut intervals: Vec<f64> = self
            .beats
            .iter()
            .zip(self.beats.iter().skip(1))
            .map(|(a, b)| {
                let mut gap = b - a;
                while gap > 60.0 / MIN_BPM {
                    gap /= 2.0;
                }
                while gap < 60.0 / MAX_BPM {
                    gap *= 2.0;
                }
                gap
            })
            .collect();
        if intervals.len() < 4 {
            return;
        }
        intervals.sort_by(f64::total_cmp);
        let median = intervals[intervals.len() / 2];
        let bpm = (60.0 / median) as f32;
        // Petits écarts lissés, grands écarts (nouveau morceau) pris d'un coup.
        self.bpm = Some(match self.bpm {
            Some(old) if (old - bpm).abs() < 4.0 => old + (bpm - old) * 0.3,
            _ => bpm,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48_000.0;

    /// Grosse caisse de synthèse (60 Hz amorti) tous les `period` s, sur un fond de
    /// charleston (bruit aigu faible), pendant `seconds`.
    fn kicks(period: f32, seconds: f32) -> Vec<f32> {
        let mut seed = 1u32;
        let mut noise = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        (0..(seconds * RATE) as usize)
            .map(|i| {
                let t = i as f32 / RATE;
                let since = t % period;
                let kick = (2.0 * PI * 60.0 * since).sin() * (-since * 18.0).exp() * 0.8;
                kick + noise() * 0.02
            })
            .collect()
    }

    #[test]
    fn finds_the_beat_and_tempo_of_a_kick_drum() {
        let mut analyzer = Analyzer::new(RATE);
        let mut beats = Vec::new();
        let mut max_bass = 0.0f32;
        analyzer.push(&kicks(0.5, 8.0), |hop| {
            if hop.beat {
                beats.push(hop.time);
            }
            max_bass = max_bass.max(hop.levels.bass);
        });
        // 16 coups en 8 s ; le tout premier peut être manqué (pas encore de moyenne).
        assert!((15..=16).contains(&beats.len()), "{beats:?}");
        for pair in beats.windows(2) {
            assert!((pair[1] - pair[0] - 0.5).abs() < 0.03, "écart irrégulier : {pair:?}");
        }
        let bpm = analyzer.bpm().expect("tempo trouvé");
        assert!((bpm - 120.0).abs() < 1.5, "{bpm}");
        assert!(max_bass > 0.9, "les graves montent à fond sur les coups : {max_bass}");
    }

    #[test]
    fn tempo_is_folded_into_a_usable_range() {
        // 60 BPM (un coup par seconde) : compté comme 120, plus utile pour les effets.
        let mut analyzer = Analyzer::new(RATE);
        analyzer.push(&kicks(1.0, 10.0), |_| {});
        let bpm = analyzer.bpm().expect("tempo trouvé");
        assert!((bpm - 120.0).abs() < 1.5, "{bpm}");
    }

    #[test]
    fn silence_gives_nothing() {
        let mut analyzer = Analyzer::new(RATE);
        let mut any = false;
        analyzer.push(&vec![0.0; RATE as usize * 2], |hop| any |= hop.beat || hop.levels.level > 0.0);
        assert!(!any);
        assert!(analyzer.bpm().is_none());
    }
}

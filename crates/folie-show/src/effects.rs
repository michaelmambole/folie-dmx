//! Effets : un ou plusieurs attributs qui oscillent entre deux valeurs, avec
//! un décalage de phase réparti sur les fixtures (éventail, ailes, blocs).

use folie_fixture::{Attribute, Target};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Waveform {
    Sine,
    RampUp,
    RampDown,
    Square,
    Triangle,
    Random,
}

impl Waveform {
    pub const ALL: [Waveform; 6] = [Self::Sine, Self::RampUp, Self::RampDown, Self::Square, Self::Triangle, Self::Random];

    pub fn key(self) -> &'static str {
        match self {
            Self::Sine => "sine",
            Self::RampUp => "ramp_up",
            Self::RampDown => "ramp_down",
            Self::Square => "square",
            Self::Triangle => "triangle",
            Self::Random => "random",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|w| w.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Sine => "Sinus",
            Self::RampUp => "Rampe montante",
            Self::RampDown => "Rampe descendante",
            Self::Square => "Carré",
            Self::Triangle => "Triangle",
            Self::Random => "Aléatoire",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EffectRate {
    /// Cycles par seconde.
    Hz(f32),
    /// Durée d'un cycle en temps, calée sur le tempo du show.
    Beats(f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct EffectLine {
    pub attribute: Attribute,
    pub waveform: Waveform,
    /// Valeur basse, en pourcentage.
    pub low: f32,
    /// Valeur haute, en pourcentage.
    pub high: f32,
    /// Décalage propre à la ligne, en degrés (ex. tilt à 90° du pan pour un cercle).
    pub phase: f32,
    /// Relatif : `low` et `high` sont des écarts (−100 à +100 %) autour de la valeur en cours.
    pub relative: bool,
    /// Part du cycle où un carré est allumé, en pourcentage (rapport cyclique).
    pub duty: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Effect {
    pub name: String,
    pub rate: EffectRate,
    pub lines: Vec<EffectLine>,
    /// Décalage réparti sur les fixtures, en degrés : 0 = toutes ensemble,
    /// 360 = une vague qui parcourt la sélection.
    pub spread: f32,
    /// L'éventail est reflété en miroir sur ce nombre d'ailes.
    pub wings: u32,
    /// Nombre de fixtures consécutives qui partagent la même phase.
    pub blocks: u32,
    /// Fixtures concernées, dans l'ordre de sélection.
    pub targets: Vec<Target>,
}

impl Effect {
    /// Décalage de phase (0 à 1) de la n-ième cible.
    pub fn phase_of(&self, index: usize) -> f32 {
        let count = self.targets.len().max(1);
        let blocks = self.blocks.max(1) as usize;
        let group = index / blocks;
        let groups = count.div_ceil(blocks);
        let wings = (self.wings.max(1) as usize).min(groups);
        let per_wing = groups.div_ceil(wings);
        let mut position = group % per_wing;
        if (group / per_wing) % 2 == 1 {
            position = per_wing - 1 - position; // aile en miroir
        }
        self.spread / 360.0 * position as f32 / per_wing as f32
    }
}

/// Rapport cyclique d'un carré : allumé la moitié du cycle.
pub const DEFAULT_DUTY: f32 = 50.0;

fn line(attribute: Attribute, waveform: Waveform, low: f32, high: f32, phase: f32) -> EffectLine {
    EffectLine { attribute, waveform, low, high, phase, relative: false, duty: DEFAULT_DUTY }
}

/// Ligne relative : de `low` à `high` (en %) autour de la valeur en cours.
fn around(attribute: Attribute, waveform: Waveform, low: f32, high: f32, phase: f32) -> EffectLine {
    EffectLine { relative: true, ..line(attribute, waveform, low, high, phase) }
}

/// Effets modèles proposés dans la fenêtre Effets : `Effect 1` à `Effect 6`.
pub fn templates() -> Vec<Effect> {
    let effect = |name: &str, rate, spread, lines| Effect {
        name: name.into(),
        rate,
        lines,
        spread,
        wings: 1,
        blocks: 1,
        targets: Vec::new(),
    };
    let color = |c: &str| Attribute::ColorMix(c.into());
    vec![
        effect("Sinus dimmer", EffectRate::Beats(4.0), 0.0, vec![line(Attribute::Dimmer, Waveform::Sine, 0.0, 100.0, 0.0)]),
        effect("Chenillard", EffectRate::Beats(1.0), 360.0, vec![line(Attribute::Dimmer, Waveform::Square, 0.0, 100.0, 0.0)]),
        effect("Vague", EffectRate::Beats(4.0), 360.0, vec![line(Attribute::Dimmer, Waveform::Sine, 0.0, 100.0, 0.0)]),
        effect(
            "Cercle",
            EffectRate::Beats(4.0),
            0.0,
            // Autour de la position en cours, comme sur grandMA2.
            vec![
                around(Attribute::Pan, Waveform::Sine, -10.0, 10.0, 0.0),
                around(Attribute::Tilt, Waveform::Sine, -10.0, 10.0, 90.0),
            ],
        ),
        effect(
            "Arc-en-ciel",
            EffectRate::Beats(8.0),
            0.0,
            vec![
                line(color("Red"), Waveform::Sine, 0.0, 100.0, 0.0),
                line(color("Green"), Waveform::Sine, 0.0, 100.0, 120.0),
                line(color("Blue"), Waveform::Sine, 0.0, 100.0, 240.0),
            ],
        ),
        effect("Pulse", EffectRate::Beats(1.0), 0.0, vec![line(Attribute::Dimmer, Waveform::RampDown, 0.0, 100.0, 0.0)]),
        effect("Scintillement", EffectRate::Hz(8.0), 0.0, vec![line(Attribute::Dimmer, Waveform::Random, 40.0, 100.0, 0.0)]),
    ]
}

// --- Format de fichier ---------------------------------------------------

#[derive(Serialize, Deserialize)]
pub(crate) struct EffectEntry {
    name: String,
    /// {"hz": 1.0} ou {"beats": 4.0}
    rate: RateEntry,
    lines: Vec<LineEntry>,
    spread: f32,
    wings: u32,
    blocks: u32,
    targets: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum RateEntry {
    Hz(f32),
    Beats(f32),
}

#[derive(Serialize, Deserialize)]
struct LineEntry {
    attribute: String,
    waveform: String,
    low: f32,
    high: f32,
    phase: f32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    relative: bool,
    #[serde(default = "default_duty", skip_serializing_if = "is_default_duty")]
    duty: f32,
}

fn default_duty() -> f32 {
    DEFAULT_DUTY
}

fn is_default_duty(duty: &f32) -> bool {
    *duty == DEFAULT_DUTY
}

impl EffectEntry {
    pub(crate) fn from_effect(effect: &Effect) -> Self {
        Self {
            name: effect.name.clone(),
            rate: match effect.rate {
                EffectRate::Hz(hz) => RateEntry::Hz(hz),
                EffectRate::Beats(beats) => RateEntry::Beats(beats),
            },
            lines: effect
                .lines
                .iter()
                .map(|l| LineEntry {
                    attribute: l.attribute.key(),
                    waveform: l.waveform.key().into(),
                    low: l.low,
                    high: l.high,
                    phase: l.phase,
                    relative: l.relative,
                    duty: l.duty,
                })
                .collect(),
            spread: effect.spread,
            wings: effect.wings,
            blocks: effect.blocks,
            targets: effect.targets.iter().map(Target::to_string).collect(),
        }
    }

    /// Les lignes et cibles illisibles sont écartées ; `None` s'il ne reste rien.
    pub(crate) fn into_effect(self, keep: impl Fn(Target) -> bool) -> Option<Effect> {
        let lines: Vec<EffectLine> = self
            .lines
            .into_iter()
            .filter_map(|l| {
                let range = if l.relative { -100.0..=100.0 } else { 0.0..=100.0 };
                Some(EffectLine {
                    attribute: Attribute::from_key(&l.attribute)?,
                    waveform: Waveform::from_key(&l.waveform)?,
                    low: l.low.clamp(*range.start(), *range.end()),
                    high: l.high.clamp(*range.start(), *range.end()),
                    phase: l.phase,
                    relative: l.relative,
                    duty: l.duty.clamp(1.0, 99.0),
                })
            })
            .collect();
        let targets: Vec<Target> = self.targets.iter().filter_map(|t| t.parse().ok()).filter(|t| keep(*t)).collect();
        (!lines.is_empty() && !targets.is_empty()).then(|| Effect {
            name: self.name,
            rate: match self.rate {
                RateEntry::Hz(hz) => EffectRate::Hz(hz),
                RateEntry::Beats(beats) => EffectRate::Beats(beats),
            },
            lines,
            spread: self.spread,
            wings: self.wings.max(1),
            blocks: self.blocks.max(1),
            targets,
        })
    }
}

#[cfg(test)]
mod tests {
    use folie_fixture::FixtureId;

    use super::*;

    fn with_targets(n: u32, spread: f32, wings: u32, blocks: u32) -> Effect {
        let mut effect = templates()[2].clone();
        effect.targets = (1..=n).map(|i| Target::whole(FixtureId(i))).collect();
        effect.spread = spread;
        effect.wings = wings;
        effect.blocks = blocks;
        effect
    }

    #[test]
    fn spread_wings_and_blocks() {
        let phases = |e: &Effect| (0..e.targets.len()).map(|i| e.phase_of(i)).collect::<Vec<_>>();
        assert_eq!(phases(&with_targets(4, 360.0, 1, 1)), [0.0, 0.25, 0.5, 0.75]);
        assert_eq!(phases(&with_targets(4, 0.0, 1, 1)), [0.0; 4]);
        // Deux ailes : le centre et les bords se répondent en miroir.
        assert_eq!(phases(&with_targets(4, 360.0, 2, 1)), [0.0, 0.5, 0.5, 0.0]);
        // Blocs de 2 : les fixtures vont par paires.
        assert_eq!(phases(&with_targets(4, 360.0, 1, 2)), [0.0, 0.0, 0.5, 0.5]);
    }

    #[test]
    fn file_round_trip() {
        let effect = with_targets(3, 180.0, 1, 1);
        let json = serde_json::to_string(&EffectEntry::from_effect(&effect)).unwrap();
        let entry: EffectEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.into_effect(|_| true), Some(effect.clone()));
        let entry: EffectEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.into_effect(|_| false), None, "plus aucune cible : l'effet disparaît");
    }
}

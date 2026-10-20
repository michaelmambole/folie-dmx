//! Séquences de cues et executors.
//!
//! Une cue ne mémorise que ce qui change (tracking) : son état complet se
//! déduit des cues précédentes. Ce calcul est fait par la console.

use std::collections::BTreeMap;

use folie_fixture::{FeatureGroup, FixtureId};

use crate::effects::Effect;

/// Temps de fondu d'une nouvelle cue, en secondes.
pub const DEFAULT_FADE: f32 = 2.0;

/// Numéro de cue au millième : « 1.5 » = 1500. Tri et égalité exacts, et on
/// peut toujours insérer une cue entre deux autres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CueNumber(pub u32);

impl CueNumber {
    pub fn whole(n: u32) -> Self {
        Self(n * 1000)
    }

    /// Numéro entier suivant : après 3.5, la cue 4.
    pub fn next_whole(self) -> Self {
        Self((self.0 / 1000 + 1) * 1000)
    }
}

impl std::fmt::Display for CueNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (whole, frac) = (self.0 / 1000, self.0 % 1000);
        if frac == 0 {
            write!(f, "{whole}")
        } else {
            let frac = format!("{frac:03}");
            write!(f, "{whole}.{}", frac.trim_end_matches('0'))
        }
    }
}

impl std::str::FromStr for CueNumber {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let invalid = || format!("numéro de cue invalide : « {s} »");
        let (whole, frac) = s.trim().split_once('.').unwrap_or((s.trim(), ""));
        if frac.len() > 3 || !frac.chars().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        let whole: u32 = whole.parse().map_err(|_| invalid())?;
        let frac: u32 = if frac.is_empty() { 0 } else { format!("{frac:0<3}").parse().map_err(|_| invalid())? };
        whole.checked_mul(1000).and_then(|w| w.checked_add(frac)).map(Self).ok_or_else(invalid)
    }
}

/// Ce qu'une cue mémorise pour un paramètre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CueValue {
    Value(u16),
    /// Référence vivante : modifier le preset modifie la cue.
    Preset(FeatureGroup, u32),
    /// Le paramètre n'est plus tenu par la séquence à partir de cette cue.
    Release,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub name: String,
    /// Durée du fondu, en secondes.
    pub fade: f32,
    /// Attente avant le fondu, en secondes.
    pub delay: f32,
    /// Fixture -> nom du paramètre -> valeur. Seulement ce qui change.
    pub values: BTreeMap<FixtureId, BTreeMap<String, CueValue>>,
    /// `None` : les effets des cues précédentes continuent (tracking).
    /// `Some` : ils sont remplacés par ceux-ci.
    pub effects: Option<Vec<Effect>>,
}

impl Cue {
    pub fn new(name: String) -> Self {
        Self { name, fade: DEFAULT_FADE, delay: 0.0, values: BTreeMap::new(), effects: None }
    }
}

/// Comportement de la touche principale d'un executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ExecutorMode {
    /// Go, Back, Off : on avance dans la séquence.
    #[default]
    Go,
    /// Allumé tant que la touche est maintenue, à pleine intensité.
    Flash,
    /// Un appui allume, le suivant éteint.
    Toggle,
}

impl ExecutorMode {
    pub const ALL: [ExecutorMode; 3] = [Self::Go, Self::Flash, Self::Toggle];

    pub fn key(self) -> &'static str {
        match self {
            Self::Go => "go",
            Self::Flash => "flash",
            Self::Toggle => "toggle",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Go => "Go",
            Self::Flash => "Flash",
            Self::Toggle => "Bascule",
        }
    }
}

/// Ce que fait le fader d'un executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FaderFunction {
    /// Dose l'intensité.
    #[default]
    Master,
    /// Dose l'intensité ; l'executor ne joue que tant que le fader est levé.
    Temp,
    /// L'intensité reste pleine ; le fader règle la vitesse des effets.
    Speed,
}

impl FaderFunction {
    pub const ALL: [FaderFunction; 3] = [Self::Master, Self::Temp, Self::Speed];

    pub fn key(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Temp => "temp",
            Self::Speed => "speed",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Master => "Master",
            Self::Temp => "Temp",
            Self::Speed => "Vitesse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

impl Priority {
    pub const ALL: [Priority; 3] = [Self::Low, Self::Normal, Self::High];

    pub fn key(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Low => "Basse",
            Self::Normal => "Normale",
            Self::High => "Haute",
        }
    }

    pub fn level(self) -> i8 {
        match self {
            Self::Low => -1,
            Self::Normal => 0,
            Self::High => 1,
        }
    }
}

/// Options de fonctionnement d'un executor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExecutorOptions {
    /// Après la dernière cue, Go revient à la première.
    pub wrap: bool,
    /// Lever le fader depuis 0 lance la cue 1.
    pub auto_start: bool,
    /// Fader à 0 : l'executor s'éteint.
    pub auto_stop: bool,
    pub fader: FaderFunction,
    /// Durée de l'extinction, en secondes (0 = instantanée).
    pub off_time: f32,
    /// Durée du Back ; `None` : le fondu de la cue visée.
    pub back_time: Option<f32>,
    pub priority: Priority,
    /// Ignoré par « Tout éteindre ».
    pub protected: bool,
    /// Fixé : reste à sa place sur toutes les pages (touche Fix de grandMA2).
    pub fixed: bool,
}

impl Default for ExecutorOptions {
    fn default() -> Self {
        Self {
            wrap: false,
            auto_start: false,
            auto_stop: false,
            fader: FaderFunction::Master,
            off_time: 0.0,
            back_time: None,
            priority: Priority::Normal,
            protected: false,
            fixed: false,
        }
    }
}

/// Places d'executors sur une page : 1 à 10 avec fader, 11 à 20 en boutons.
pub const SLOTS_PER_PAGE: u32 = 20;
/// Écart de numérotation entre deux pages : l'executor 5 de la page 2 porte le numéro 105.
/// La page 1 garde les numéros 1 à 20, ceux des shows enregistrés avant les pages.
const PAGE_STRIDE: u32 = 100;
/// Dernière page possible.
pub const MAX_PAGE: u32 = 99;

/// Numéro d'executor d'une place sur une page.
pub fn executor_id(page: u32, slot: u32) -> u32 {
    (page.max(1) - 1) * PAGE_STRIDE + slot
}

/// Page et place d'un numéro d'executor.
pub fn executor_place(id: u32) -> (u32, u32) {
    let id = id.max(1);
    ((id - 1) / PAGE_STRIDE + 1, (id - 1) % PAGE_STRIDE + 1)
}

/// Nom affiché d'un executor : « 5 » sur la page 1, « 2.5 » ailleurs.
pub fn executor_label(id: u32) -> String {
    match executor_place(id) {
        (1, slot) => slot.to_string(),
        (page, slot) => format!("{page}.{slot}"),
    }
}

/// Un executor : la séquence qu'il joue, le comportement de sa touche et ses options.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Executor {
    pub sequence: u32,
    pub mode: ExecutorMode,
    pub options: ExecutorOptions,
}

impl Executor {
    pub fn new(sequence: u32) -> Self {
        Self { sequence, mode: ExecutorMode::Go, options: ExecutorOptions::default() }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Sequence {
    pub name: String,
    pub cues: BTreeMap<CueNumber, Cue>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cue_numbers() {
        let n: CueNumber = "1.5".parse().unwrap();
        assert_eq!(n, CueNumber(1500));
        assert_eq!(n.to_string(), "1.5");
        assert_eq!("12".parse::<CueNumber>().unwrap().to_string(), "12");
        assert_eq!("2.25".parse::<CueNumber>().unwrap().to_string(), "2.25");
        assert_eq!(n.next_whole(), CueNumber::whole(2));
        assert!("1.2345".parse::<CueNumber>().is_err());
        assert!("x".parse::<CueNumber>().is_err());
        assert!(CueNumber::whole(1) < "1.1".parse().unwrap());
    }

    #[test]
    fn executor_pages() {
        assert_eq!(executor_id(1, 5), 5, "la page 1 garde les anciens numéros");
        assert_eq!(executor_id(2, 5), 105);
        assert_eq!(executor_place(105), (2, 5));
        assert_eq!(executor_place(20), (1, 20));
        assert_eq!(executor_label(5), "5");
        assert_eq!(executor_label(120), "2.20");
    }
}

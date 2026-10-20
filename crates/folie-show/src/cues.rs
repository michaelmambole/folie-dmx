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

/// Temps propre d'un paramètre ou d'une famille dans une cue ; `None` : celui de la cue.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ParamTiming {
    pub fade: Option<f32>,
    pub delay: Option<f32>,
}

impl ParamTiming {
    pub fn is_empty(&self) -> bool {
        self.fade.is_none() && self.delay.is_none()
    }

    /// (fondu, délai), les temps absents pris dans `base`.
    pub fn or(&self, base: (f32, f32)) -> (f32, f32) {
        (self.fade.unwrap_or(base.0), self.delay.unwrap_or(base.1))
    }
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
    /// Temps propres par famille (Position en 4 s…), pour les paramètres de la cue.
    pub family_times: BTreeMap<FeatureGroup, ParamTiming>,
    /// Temps propres d'un paramètre d'une fixture : ils l'emportent sur ceux de sa famille.
    pub param_times: BTreeMap<FixtureId, BTreeMap<String, ParamTiming>>,
}

impl Cue {
    pub fn new(name: String) -> Self {
        Self {
            name,
            fade: DEFAULT_FADE,
            delay: 0.0,
            values: BTreeMap::new(),
            effects: None,
            family_times: BTreeMap::new(),
            param_times: BTreeMap::new(),
        }
    }

    /// (fondu, délai) d'un paramètre : son temps propre, sinon celui de sa famille, sinon celui de la cue.
    pub fn times_of(&self, fixture: FixtureId, parameter: &str, family: FeatureGroup) -> (f32, f32) {
        let base = (self.fade, self.delay);
        let base = self.family_times.get(&family).map_or(base, |t| t.or(base));
        self.param_times.get(&fixture).and_then(|p| p.get(parameter)).map_or(base, |t| t.or(base))
    }

    /// Le paramètre ou la famille a un temps propre.
    pub fn has_own_times(&self, fixture: FixtureId, parameter: &str, family: FeatureGroup) -> bool {
        self.family_times.contains_key(&family) || self.param_times.get(&fixture).is_some_and(|p| p.contains_key(parameter))
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
    /// Comme Flash, mais les autres executors s'éteignent tant que la touche est maintenue.
    Swop,
    /// Go, et tous les autres executors s'éteignent.
    Kill,
}

impl ExecutorMode {
    pub const ALL: [ExecutorMode; 5] = [Self::Go, Self::Flash, Self::Toggle, Self::Swop, Self::Kill];

    /// La touche agit tant qu'elle est maintenue.
    pub fn is_held(self) -> bool {
        matches!(self, Self::Flash | Self::Swop)
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Go => "go",
            Self::Flash => "flash",
            Self::Toggle => "toggle",
            Self::Swop => "swop",
            Self::Kill => "kill",
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
            Self::Swop => "Swop",
            Self::Kill => "Kill",
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
    /// L'intensité reste pleine ; le fader règle la vitesse des effets et du chenillard.
    Speed,
    /// L'intensité reste pleine ; le fader fait à la main la transition vers la cue
    /// suivante, en montant puis en descendant (A/B).
    Crossfade,
}

impl FaderFunction {
    pub const ALL: [FaderFunction; 4] = [Self::Master, Self::Temp, Self::Speed, Self::Crossfade];

    pub fn key(self) -> &'static str {
        match self {
            Self::Master => "master",
            Self::Temp => "temp",
            Self::Speed => "speed",
            Self::Crossfade => "crossfade",
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
            Self::Crossfade => "Crossfade",
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

/// Ordre dans lequel un chenillard parcourt ses cues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ChaserDirection {
    #[default]
    Forward,
    Backward,
    /// Aller-retour.
    Bounce,
    Random,
}

impl ChaserDirection {
    pub const ALL: [ChaserDirection; 4] = [Self::Forward, Self::Backward, Self::Bounce, Self::Random];

    pub fn key(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
            Self::Bounce => "bounce",
            Self::Random => "random",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|d| d.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Forward => "Avant",
            Self::Backward => "Arrière",
            Self::Bounce => "Aller-retour",
            Self::Random => "Hasard",
        }
    }
}

/// Réglages du chenillard : les cues défilent seules, à vitesse régulière.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChaserSettings {
    /// Pas par minute.
    pub bpm: f32,
    /// Suit le tempo du show au lieu de `bpm`.
    pub follow_tempo: bool,
    /// Fondu entre deux pas, en fraction de la durée d'un pas (0 à 1).
    pub step_fade: f32,
    pub direction: ChaserDirection,
}

impl Default for ChaserSettings {
    fn default() -> Self {
        Self { bpm: 120.0, follow_tempo: false, step_fade: 0.0, direction: ChaserDirection::Forward }
    }
}

/// Cue jouée quand on rallume un executor éteint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RestartMode {
    #[default]
    First,
    /// Celle où il s'était arrêté.
    Current,
    /// Celle d'après.
    Next,
}

impl RestartMode {
    pub const ALL: [RestartMode; 3] = [Self::First, Self::Current, Self::Next];

    pub fn key(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Current => "current",
            Self::Next => "next",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.key() == key)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::First => "1re cue",
            Self::Current => "Cue en cours",
            Self::Next => "Suivante",
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
    /// Les cues défilent seules.
    pub chaser: bool,
    /// Réglages gardés même chenillard coupé.
    pub chaser_settings: ChaserSettings,
    pub restart: RestartMode,
    /// Off on Overwritten : s'éteint quand d'autres executors ont repris tout ce qu'il tient.
    pub ooo: bool,
    /// Move in Black : les fixtures éteintes prennent d'avance leur position de la cue suivante.
    pub mib: bool,
    /// Ignore le Swop des autres executors.
    pub swop_protect: bool,
    /// Ignore le Kill des autres executors.
    pub kill_protect: bool,
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
            chaser: false,
            chaser_settings: ChaserSettings::default(),
            restart: RestartMode::First,
            ooo: false,
            mib: false,
            swop_protect: false,
            kill_protect: false,
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
    fn parameter_times_override_family_and_cue() {
        let mut cue = Cue::new("Contre".into());
        cue.fade = 2.0;
        cue.family_times.insert(FeatureGroup::Position, ParamTiming { fade: Some(4.0), delay: None });
        cue.param_times.entry(FixtureId(1)).or_default().insert("Tilt".into(), ParamTiming { fade: None, delay: Some(1.0) });
        assert_eq!(cue.times_of(FixtureId(1), "Dimmer", FeatureGroup::Dimmer), (2.0, 0.0), "temps de la cue");
        assert_eq!(cue.times_of(FixtureId(1), "Pan", FeatureGroup::Position), (4.0, 0.0), "temps de la famille");
        assert_eq!(cue.times_of(FixtureId(1), "Tilt", FeatureGroup::Position), (4.0, 1.0), "délai propre, fondu de la famille");
        assert_eq!(cue.times_of(FixtureId(2), "Tilt", FeatureGroup::Position), (4.0, 0.0));
        assert!(cue.has_own_times(FixtureId(1), "Tilt", FeatureGroup::Position));
        assert!(!cue.has_own_times(FixtureId(1), "Dimmer", FeatureGroup::Dimmer));
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

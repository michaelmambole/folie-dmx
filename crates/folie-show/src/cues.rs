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
}

//! Réaction au son, enregistrée dans le show : ce que fait chaque executor sur la musique.

use serde::{Deserialize, Serialize};

/// Bande de fréquences suivie par un fader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    /// Tout le spectre.
    #[default]
    Level,
    Bass,
    Mid,
    High,
}

impl Band {
    pub const ALL: [Band; 4] = [Band::Level, Band::Bass, Band::Mid, Band::High];

    pub fn label(self) -> &'static str {
        match self {
            Self::Level => "Niveau",
            Self::Bass => "Graves",
            Self::Mid => "Médiums",
            Self::High => "Aigus",
        }
    }
}

/// Ce que fait un executor sur le son.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reaction {
    /// Go tous les `every` beats (un chenillard avance d'un pas).
    Step { every: u32 },
    /// Le fader suit une bande, entre `min` et `max` (0 à 1).
    Fader { band: Band, min: f32, max: f32 },
    /// Flash bref tous les `every` beats.
    Flash { every: u32 },
}

impl Reaction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Step { .. } => "Pas sur le beat",
            Self::Fader { .. } => "Fader au niveau",
            Self::Flash { .. } => "Flash sur le beat",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SoundReaction {
    /// Numéro d'executor (voir `executor_id`).
    pub executor: u32,
    #[serde(flatten)]
    pub reaction: Reaction,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundSetup {
    /// Le tempo du show suit le beat détecté.
    pub tempo: bool,
    pub reactions: Vec<SoundReaction>,
}

impl SoundSetup {
    pub fn is_empty(&self) -> bool {
        !self.tempo && self.reactions.is_empty()
    }
}

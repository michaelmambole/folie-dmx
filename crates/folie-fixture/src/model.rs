use std::ops::RangeInclusive;

/// Familles d'attributs, dans l'ordre d'affichage (même découpage que sur une console MA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FeatureGroup {
    Dimmer,
    Position,
    Color,
    Gobo,
    Beam,
    Focus,
    Control,
}

impl FeatureGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::Dimmer => "Dimmer",
            Self::Position => "Position",
            Self::Color => "Couleur",
            Self::Gobo => "Gobo",
            Self::Beam => "Faisceau",
            Self::Focus => "Focus",
            Self::Control => "Contrôle",
        }
    }

    pub const ALL: [FeatureGroup; 7] = [
        Self::Dimmer,
        Self::Position,
        Self::Gobo,
        Self::Color,
        Self::Beam,
        Self::Focus,
        Self::Control,
    ];

    /// Numéro du type de preset sur grandMA2 : `Preset 4.1` = couleur n° 1.
    pub fn preset_number(self) -> u32 {
        match self {
            Self::Dimmer => 1,
            Self::Position => 2,
            Self::Gobo => 3,
            Self::Color => 4,
            Self::Beam => 5,
            Self::Focus => 6,
            Self::Control => 7,
        }
    }

    pub fn from_preset_number(number: u32) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.preset_number() == number)
    }

    /// Identifiant stable utilisé dans les fichiers de show.
    pub fn key(self) -> &'static str {
        match self {
            Self::Dimmer => "dimmer",
            Self::Position => "position",
            Self::Gobo => "gobo",
            Self::Color => "color",
            Self::Beam => "beam",
            Self::Focus => "focus",
            Self::Control => "control",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.key() == key)
    }
}

/// Fonction normalisée d'un paramètre, indépendante du modèle d'appareil.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Attribute {
    Dimmer,
    Shutter,
    Pan,
    Tilt,
    PanTiltSpeed,
    /// Composante de mélange additif ou soustractif : "Red", "White", "Cyan"…
    ColorMix(String),
    /// Roue de couleurs réelle ou simulée, macros de couleurs.
    ColorMacro,
    ColorTemperature,
    Gobo,
    Prism,
    Zoom,
    Focus,
    Iris,
    Effect,
    Control,
    Other(String),
}

impl Attribute {
    pub fn group(&self) -> FeatureGroup {
        match self {
            Self::Dimmer | Self::Shutter => FeatureGroup::Dimmer,
            Self::Pan | Self::Tilt | Self::PanTiltSpeed => FeatureGroup::Position,
            Self::ColorMix(_) | Self::ColorMacro | Self::ColorTemperature => FeatureGroup::Color,
            Self::Gobo => FeatureGroup::Gobo,
            Self::Prism | Self::Effect => FeatureGroup::Beam,
            Self::Zoom | Self::Focus | Self::Iris => FeatureGroup::Focus,
            Self::Control | Self::Other(_) => FeatureGroup::Control,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CapabilityKind {
    NoFunction,
    Intensity,
    /// Effet d'obturateur OFL : "Open", "Closed", "Strobe", "RampUp"…
    Shutter { effect: String, random: bool },
    ColorIntensity(String),
    ColorPreset { colors: Vec<[u8; 3]> },
    ColorTemperature,
    Pan,
    Tilt,
    PanTiltSpeed,
    Zoom,
    Focus,
    Iris,
    Gobo,
    Prism,
    Effect,
    Maintenance,
    /// Type OFL pas encore interprété : conservé tel quel.
    Other(String),
}

/// Une plage de valeurs d'un paramètre et ce qu'elle fait.
#[derive(Debug, Clone, PartialEq)]
pub struct Capability {
    /// Plage en valeurs 16 bits.
    pub range: RangeInclusive<u16>,
    pub kind: CapabilityKind,
    pub label: String,
    /// Plage technique à ne pas proposer dans les menus (ex. « Open » de transition).
    pub hidden: bool,
    /// Angle physique au début et à la fin de la plage, en degrés.
    pub angle: Option<(f32, f32)>,
}

impl Capability {
    /// Valeur à envoyer pour choisir cette plage : son début, aligné sur un pas DMX 8 bits.
    pub fn select_value(&self) -> u16 {
        let start = *self.range.start();
        start | (start >> 8)
    }
}

/// Un paramètre logique d'un mode : un ou plusieurs canaux DMX consécutifs
/// (grossier, fin…) pilotés par une seule valeur.
#[derive(Debug, Clone, PartialEq)]
pub struct Parameter {
    /// Nom complet, ex. "Dimmer 3" pour la cellule 3 d'un Sunstrip.
    pub name: String,
    /// Nom de la fonction sans la cellule, ex. "Dimmer". Égal à `name` hors matrice.
    pub base_name: String,
    /// Sous-fixture pilotée (index dans `Mode::heads`), `None` pour l'appareil entier.
    pub head: Option<usize>,
    pub attribute: Attribute,
    /// Décalages des canaux dans le mode (base 0), poids fort en premier.
    pub offsets: Vec<u16>,
    pub default: u16,
    pub capabilities: Vec<Capability>,
}

impl Parameter {
    pub fn resolution_bits(&self) -> u32 {
        8 * self.offsets.len() as u32
    }

    /// Plage contenant `value`, s'il y en a une.
    pub fn capability_at(&self, value: u16) -> Option<&Capability> {
        self.capabilities.iter().find(|c| c.range.contains(&value))
    }

    /// Écrit `value` dans les canaux de l'appareil (`dmx` commence à son adresse).
    pub fn write(&self, value: u16, dmx: &mut [u8]) {
        let bytes = value.to_be_bytes();
        for (level, &offset) in self.offsets.iter().enumerate() {
            if let Some(slot) = dmx.get_mut(offset as usize) {
                *slot = bytes.get(level).copied().unwrap_or(0);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Mode {
    pub name: String,
    pub short_name: String,
    /// Nombre de canaux DMX occupés.
    pub footprint: u16,
    /// Noms des sous-fixtures (cellules d'une matrice), vide pour un appareil simple.
    pub heads: Vec<String>,
    pub parameters: Vec<Parameter>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureType {
    /// Identifiant stable : "constructeur/modèle", ex. "martin/rush-mh-6-wash".
    pub key: String,
    pub manufacturer: String,
    pub name: String,
    pub short_name: String,
    pub categories: Vec<String>,
    pub modes: Vec<Mode>,
    /// Modes du fichier écartés à l'import, avec la raison.
    pub warnings: Vec<String>,
    /// Texte OFL d'origine : il est recopié dans le show pour qu'il reste lisible
    /// sur une machine dont la bibliothèque est différente.
    pub ofl_source: String,
}

impl FixtureType {
    pub fn display_name(&self) -> String {
        format!("{} {}", self.manufacturer, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(offsets: Vec<u16>) -> Parameter {
        Parameter {
            name: "Pan".into(),
            base_name: "Pan".into(),
            head: None,
            attribute: Attribute::Pan,
            offsets,
            default: 0,
            capabilities: Vec::new(),
        }
    }

    #[test]
    fn writes_8_and_16_bit() {
        let mut dmx = [0u8; 4];
        param(vec![1]).write(200 * 257, &mut dmx);
        assert_eq!(dmx, [0, 200, 0, 0]);

        param(vec![2, 3]).write(0x1234, &mut dmx);
        assert_eq!(dmx, [0, 200, 0x12, 0x34]);
    }
}

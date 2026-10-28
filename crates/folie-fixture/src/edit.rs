//! Fiche OFL modifiable : ce que l'éditeur de fiches change (canaux, plages, modes,
//! cellules), le reste du fichier (auteurs, liens, dimensions, extensions de Folie…)
//! étant recopié tel quel à l'enregistrement.

use serde_json::{Map, Value, json};

/// Types de plage OFL proposés, dans l'ordre du menu.
pub const CAPABILITY_TYPES: &[&str] = &[
    "NoFunction",
    "Intensity",
    "ColorIntensity",
    "ColorPreset",
    "ColorTemperature",
    "ShutterStrobe",
    "Pan",
    "Tilt",
    "PanContinuous",
    "TiltContinuous",
    "PanTiltSpeed",
    "WheelSlot",
    "WheelRotation",
    "WheelSlotRotation",
    "Zoom",
    "Focus",
    "Iris",
    "Prism",
    "Frost",
    "Effect",
    "EffectSpeed",
    "Maintenance",
    "Generic",
];

/// Couleurs d'émetteur OFL (`ColorIntensity`).
pub const EMITTER_COLORS: &[&str] =
    &["Red", "Green", "Blue", "White", "Warm White", "Cold White", "Amber", "UV", "Lime", "Cyan", "Magenta", "Yellow", "Indigo"];

/// Effets de shutter OFL (`ShutterStrobe`).
pub const SHUTTER_EFFECTS: &[&str] = &["Open", "Closed", "Strobe", "Pulse", "RampUp", "RampDown", "RampUpDown", "Lightning", "Spikes", "Burst"];

#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityDoc {
    pub start: u32,
    pub end: u32,
    pub kind: String,
    pub comment: String,
    /// Champs propres au type : `color`, `colors`, `shutterEffect`, `angleStart`…
    pub extra: Map<String, Value>,
}

impl CapabilityDoc {
    pub fn text(&self, key: &str) -> String {
        self.extra.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    pub fn set_text(&mut self, key: &str, value: &str) {
        if value.is_empty() {
            self.extra.remove(key);
        } else {
            self.extra.insert(key.into(), json!(value));
        }
    }

    /// Champs qui ne valent que pour un type : on les retire quand le type change.
    pub fn set_kind(&mut self, kind: &str) {
        if self.kind == kind {
            return;
        }
        self.kind = kind.to_string();
        let keep: &[&str] = match kind {
            "ColorIntensity" => &["color"],
            "ColorPreset" => &["colors", "colorsStart", "colorsEnd"],
            "ShutterStrobe" => &["shutterEffect", "speed", "speedStart", "speedEnd", "randomTiming"],
            "Pan" | "Tilt" => &["angle", "angleStart", "angleEnd"],
            "WheelSlot" => &["slotNumber", "wheel"],
            _ => &[],
        };
        self.extra.retain(|key, _| keep.contains(&key.as_str()));
        if kind == "ColorIntensity" && !self.extra.contains_key("color") {
            self.extra.insert("color".into(), json!("Red"));
        }
        if kind == "ShutterStrobe" && !self.extra.contains_key("shutterEffect") {
            self.extra.insert("shutterEffect".into(), json!("Strobe"));
        }
    }

    fn from_json(value: &Value) -> Self {
        let mut extra = value.as_object().cloned().unwrap_or_default();
        let range = extra.remove("dmxRange");
        let [start, end] = range
            .and_then(|r| Some([r.get(0)?.as_u64()? as u32, r.get(1)?.as_u64()? as u32]))
            .unwrap_or([0, 255]);
        let kind = extra.remove("type").and_then(|t| t.as_str().map(String::from)).unwrap_or_else(|| "Generic".into());
        let comment = extra.remove("comment").and_then(|c| c.as_str().map(String::from)).unwrap_or_default();
        Self { start, end, kind, comment, extra }
    }

    fn to_json(&self, with_range: bool) -> Value {
        let mut map = Map::new();
        if with_range {
            map.insert("dmxRange".into(), json!([self.start, self.end]));
        }
        map.insert("type".into(), json!(self.kind));
        if !self.comment.is_empty() {
            map.insert("comment".into(), json!(self.comment));
        }
        map.extend(self.extra.clone());
        Value::Object(map)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDoc {
    pub name: String,
    /// Noms des canaux fins (un par octet supplémentaire).
    pub fine: Vec<String>,
    /// `Some(16)` : valeurs exprimées sur 16 bits ; `None` : celles du fichier (8 bits le plus souvent).
    pub resolution: Option<u32>,
    /// Valeur par défaut, en valeur DMX ou en pourcentage (« 100% ») ; vide : aucune.
    pub default_value: String,
    pub capabilities: Vec<CapabilityDoc>,
    /// Autres champs (`folieTurnsOff`, `highlightValue`, `constant`…).
    pub extra: Map<String, Value>,
}

impl ChannelDoc {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            fine: Vec::new(),
            resolution: None,
            default_value: String::new(),
            capabilities: vec![CapabilityDoc {
                start: 0,
                end: 255,
                kind: "Generic".into(),
                comment: String::new(),
                extra: Map::new(),
            }],
            extra: Map::new(),
        }
    }

    /// Plus grande valeur DMX des plages : 255, 65535…
    pub fn max_value(&self) -> u32 {
        match self.resolution {
            Some(16) => 65_535,
            Some(24) => 16_777_215,
            _ => 255,
        }
    }

    /// Nombre d'octets : 1 (8 bits), 2 (16 bits)… Les noms fins suivent le nom du canal.
    pub fn set_bytes(&mut self, bytes: usize) {
        let bytes = bytes.max(1);
        while self.fine.len() + 1 > bytes {
            self.fine.pop();
        }
        while self.fine.len() + 1 < bytes {
            let level = self.fine.len() + 1;
            self.fine.push(fine_name(&self.name, level));
        }
    }

    /// Ajoute une plage après la dernière, en coupant celle-ci en deux.
    pub fn split_last(&mut self) {
        let Some(last) = self.capabilities.last_mut() else {
            self.capabilities.push(CapabilityDoc { start: 0, end: self.max_value(), kind: "Generic".into(), comment: String::new(), extra: Map::new() });
            return;
        };
        if last.end <= last.start {
            return;
        }
        let middle = last.start + (last.end - last.start) / 2;
        let mut next = last.clone();
        next.start = middle + 1;
        last.end = middle;
        next.comment.clear();
        self.capabilities.push(next);
    }

    fn from_json(name: &str, value: &Value) -> Self {
        let mut extra = value.as_object().cloned().unwrap_or_default();
        let fine = extra
            .remove("fineChannelAliases")
            .and_then(|v| v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()))
            .unwrap_or_default();
        let resolution = extra.remove("dmxValueResolution").and_then(|r| match r.as_str()? {
            "8bit" => Some(8),
            "16bit" => Some(16),
            "24bit" => Some(24),
            _ => None,
        });
        let default_value = match extra.remove("defaultValue") {
            Some(Value::String(text)) => text,
            Some(Value::Number(n)) => n.to_string(),
            _ => String::new(),
        };
        let capabilities = match (extra.remove("capability"), extra.remove("capabilities")) {
            (_, Some(Value::Array(list))) => list.iter().map(CapabilityDoc::from_json).collect(),
            (Some(single), _) => {
                let mut cap = CapabilityDoc::from_json(&single);
                cap.start = 0;
                cap.end = match resolution {
                    Some(16) => 65_535,
                    _ => 255,
                };
                vec![cap]
            }
            _ => Vec::new(),
        };
        Self { name: name.to_string(), fine, resolution, default_value, capabilities, extra }
    }

    fn to_json(&self) -> Value {
        let mut map = Map::new();
        if !self.fine.is_empty() {
            map.insert("fineChannelAliases".into(), json!(self.fine));
        }
        if let Some(bits) = self.resolution {
            map.insert("dmxValueResolution".into(), json!(format!("{bits}bit")));
        }
        let default = self.default_value.trim();
        if let Ok(value) = default.parse::<u64>() {
            map.insert("defaultValue".into(), json!(value));
        } else if !default.is_empty() {
            map.insert("defaultValue".into(), json!(default));
        }
        // Une seule plage couvrant tout : la forme courte d'OFL.
        match self.capabilities.as_slice() {
            [only] if only.start == 0 && only.end == self.max_value() => {
                map.insert("capability".into(), only.to_json(false));
            }
            caps => {
                map.insert("capabilities".into(), Value::Array(caps.iter().map(|c| c.to_json(true)).collect()));
            }
        }
        map.extend(self.extra.clone());
        Value::Object(map)
    }
}

/// Une case d'un mode : un canal par son nom, une case vide, ou une insertion de
/// canaux de matrice (gardée telle quelle).
#[derive(Debug, Clone, PartialEq)]
pub enum Slot {
    Channel(String),
    Empty,
    Insert(Value),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModeDoc {
    pub name: String,
    pub short_name: String,
    pub channels: Vec<Slot>,
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FixtureDoc {
    pub name: String,
    pub short_name: String,
    pub categories: Vec<String>,
    pub channels: Vec<ChannelDoc>,
    /// Canaux modèles des cellules (« Dimmer $pixelKey »).
    pub templates: Vec<ChannelDoc>,
    /// Noms des cellules, dans l'ordre (matrice à une dimension).
    pub cells: Vec<String>,
    pub modes: Vec<ModeDoc>,
    /// Matrice d'origine quand elle n'est pas à une dimension : recopiée telle quelle.
    matrix: Option<Value>,
    extra: Map<String, Value>,
}

pub fn fine_name(name: &str, level: usize) -> String {
    if level == 1 { format!("{name} fine") } else { format!("{name} fine^{level}") }
}

const SCHEMA: &str = "https://raw.githubusercontent.com/OpenLightingProject/open-fixture-library/master/schemas/fixture.json";

impl FixtureDoc {
    /// Fiche vide, prête à remplir : un canal Dimmer, un mode.
    pub fn new(name: &str) -> Self {
        let mut dimmer = ChannelDoc::new("Dimmer");
        dimmer.capabilities[0].kind = "Intensity".into();
        Self {
            name: name.to_string(),
            short_name: String::new(),
            categories: vec!["Dimmer".into()],
            channels: vec![dimmer],
            templates: Vec::new(),
            cells: Vec::new(),
            modes: vec![ModeDoc {
                name: "Mode 1".into(),
                short_name: String::new(),
                channels: vec![Slot::Channel("Dimmer".into())],
                extra: Map::new(),
            }],
            matrix: None,
            extra: Map::new(),
        }
    }

    pub fn from_json(json: &str) -> Result<Self, String> {
        let value: Value = serde_json::from_str(json).map_err(|e| format!("JSON invalide : {e}"))?;
        let mut extra = value.as_object().cloned().ok_or("la fiche n'est pas un objet JSON")?;
        let take_str = |extra: &mut Map<String, Value>, key: &str| {
            extra.remove(key).and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
        };
        let name = take_str(&mut extra, "name");
        let short_name = take_str(&mut extra, "shortName");
        let categories = extra
            .remove("categories")
            .and_then(|c| c.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()))
            .unwrap_or_default();
        let channels_of = |value: Option<Value>| -> Vec<ChannelDoc> {
            value
                .and_then(|v| v.as_object().cloned())
                .map(|m| m.iter().map(|(name, def)| ChannelDoc::from_json(name, def)).collect())
                .unwrap_or_default()
        };
        let channels = channels_of(extra.remove("availableChannels"));
        let templates = channels_of(extra.remove("templateChannels"));
        let matrix = extra.remove("matrix");
        let one_dimension = matrix
            .as_ref()
            .and_then(|m| m.get("pixelKeys")?.as_array().cloned())
            .filter(|planes| planes.len() == 1 && planes[0].as_array().is_some_and(|rows| rows.len() == 1));
        let (cells, matrix) = match (one_dimension, matrix) {
            (Some(planes), Some(m)) if m.as_object().is_some_and(|o| o.len() == 1) => {
                let keys = planes[0][0].as_array().cloned().unwrap_or_default();
                (keys.iter().filter_map(|k| k.as_str().map(String::from)).collect(), None)
            }
            (_, matrix) => (Vec::new(), matrix),
        };
        let modes = extra
            .remove("modes")
            .and_then(|m| m.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|mode| {
                let mut extra = mode.as_object().cloned().unwrap_or_default();
                let name = take_str(&mut extra, "name");
                let short_name = take_str(&mut extra, "shortName");
                let channels = extra
                    .remove("channels")
                    .and_then(|c| c.as_array().cloned())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|slot| match slot {
                        Value::String(name) => Slot::Channel(name),
                        Value::Null => Slot::Empty,
                        other => Slot::Insert(other),
                    })
                    .collect();
                ModeDoc { name, short_name, channels, extra }
            })
            .collect();
        Ok(Self { name, short_name, categories, channels, templates, cells, modes, matrix, extra })
    }

    pub fn to_json(&self) -> String {
        let mut map = Map::new();
        map.insert("$schema".into(), json!(SCHEMA));
        map.insert("name".into(), json!(self.name));
        if !self.short_name.is_empty() {
            map.insert("shortName".into(), json!(self.short_name));
        }
        map.insert("categories".into(), json!(self.categories));
        let mut extra = self.extra.clone();
        extra.remove("$schema");
        map.extend(extra);
        let channels = |list: &[ChannelDoc]| -> Value {
            Value::Object(list.iter().map(|c| (c.name.clone(), c.to_json())).collect())
        };
        if let Some(matrix) = &self.matrix {
            map.insert("matrix".into(), matrix.clone());
        } else if !self.cells.is_empty() {
            map.insert("matrix".into(), json!({ "pixelKeys": [[self.cells]] }));
        }
        map.insert("availableChannels".into(), channels(&self.channels));
        if !self.templates.is_empty() {
            map.insert("templateChannels".into(), channels(&self.templates));
        }
        let modes: Vec<Value> = self
            .modes
            .iter()
            .map(|mode| {
                let mut m = Map::new();
                m.insert("name".into(), json!(mode.name));
                if !mode.short_name.is_empty() {
                    m.insert("shortName".into(), json!(mode.short_name));
                }
                let slots: Vec<Value> = mode
                    .channels
                    .iter()
                    .map(|slot| match slot {
                        Slot::Channel(name) => json!(name),
                        Slot::Empty => Value::Null,
                        Slot::Insert(value) => value.clone(),
                    })
                    .collect();
                m.insert("channels".into(), Value::Array(slots));
                m.extend(mode.extra.clone());
                Value::Object(m)
            })
            .collect();
        map.insert("modes".into(), Value::Array(modes));
        serde_json::to_string_pretty(&Value::Object(map)).unwrap_or_default()
    }

    /// Noms utilisables dans un mode : canaux et leurs fins, canaux de cellule concrets.
    pub fn slot_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for channel in &self.channels {
            names.push(channel.name.clone());
            names.extend(channel.fine.iter().cloned());
        }
        for template in &self.templates {
            for cell in &self.cells {
                let concrete = |n: &str| n.replace("$pixelKey", cell);
                names.push(concrete(&template.name));
                names.extend(template.fine.iter().map(|f| concrete(f)));
            }
        }
        names
    }

    /// Le nom est-il déjà pris par un canal (ou un de ses fins) ?
    pub fn name_taken(&self, name: &str) -> bool {
        self.channels.iter().chain(&self.templates).any(|c| c.name == name || c.fine.iter().any(|f| f == name))
    }

    /// Nom libre à partir d'une base : « Canal », « Canal 2 »…
    pub fn free_name(&self, base: &str) -> String {
        if !self.name_taken(base) {
            return base.to_string();
        }
        (2..).map(|n| format!("{base} {n}")).find(|n| !self.name_taken(n)).unwrap()
    }

    /// Renomme un canal (et ses fins), dans les modes aussi.
    pub fn rename_channel(&mut self, index: usize, new_name: &str) -> Result<(), String> {
        let new_name = new_name.trim();
        let old = self.channels.get(index).ok_or("canal inconnu")?.clone();
        if new_name.is_empty() {
            return Err("un canal doit avoir un nom".into());
        }
        if new_name != old.name && self.name_taken(new_name) {
            return Err(format!("« {new_name} » existe déjà"));
        }
        let mut renames = vec![(old.name.clone(), new_name.to_string())];
        let channel = &mut self.channels[index];
        channel.name = new_name.to_string();
        for (level, fine) in channel.fine.iter_mut().enumerate() {
            let renamed = fine_name(new_name, level + 1);
            renames.push((std::mem::replace(fine, renamed.clone()), renamed));
        }
        for mode in &mut self.modes {
            for slot in &mut mode.channels {
                if let Slot::Channel(name) = slot
                    && let Some((_, to)) = renames.iter().find(|(from, _)| from == name)
                {
                    *name = to.clone();
                }
            }
        }
        Ok(())
    }

    /// Retire un canal, et ses cases dans les modes.
    pub fn remove_channel(&mut self, index: usize) {
        if index >= self.channels.len() {
            return;
        }
        let channel = self.channels.remove(index);
        let gone: Vec<&String> = std::iter::once(&channel.name).chain(&channel.fine).collect();
        for mode in &mut self.modes {
            mode.channels.retain(|slot| !matches!(slot, Slot::Channel(name) if gone.contains(&name)));
        }
    }

    /// Copie d'un mode, ajoutée juste après lui.
    pub fn duplicate_mode(&mut self, index: usize) {
        let Some(mode) = self.modes.get(index) else { return };
        let mut copy = mode.clone();
        let base = format!("{} (copie)", mode.name);
        copy.name = base.clone();
        let mut n = 2;
        while self.modes.iter().any(|m| m.name == copy.name) {
            copy.name = format!("{base} {n}");
            n += 1;
        }
        copy.short_name.clear();
        self.modes.insert(index + 1, copy);
    }

    /// Fixe le nombre de cellules : noms « 1 », « 2 »… pour les nouvelles.
    pub fn set_cell_count(&mut self, count: usize) {
        self.cells.truncate(count);
        while self.cells.len() < count {
            let mut n = self.cells.len() + 1;
            while self.cells.contains(&n.to_string()) {
                n += 1;
            }
            self.cells.push(n.to_string());
        }
    }

    /// Ajoute à un mode les canaux de toutes les cellules : cellule par cellule
    /// (`by_cell`), ou canal modèle par canal modèle.
    pub fn add_cells_to_mode(&mut self, mode: usize, by_cell: bool) {
        let Some(mode) = self.modes.get_mut(mode) else { return };
        let names = |template: &ChannelDoc, cell: &str| {
            std::iter::once(&template.name).chain(&template.fine).map(|n| n.replace("$pixelKey", cell)).collect::<Vec<_>>()
        };
        if by_cell {
            for cell in &self.cells {
                for template in &self.templates {
                    mode.channels.extend(names(template, cell).into_iter().map(Slot::Channel));
                }
            }
        } else {
            for template in &self.templates {
                for cell in &self.cells {
                    mode.channels.extend(names(template, cell).into_iter().map(Slot::Channel));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ofl;
    use std::path::Path;

    /// Lire puis réécrire une fiche ne change rien à ce que Folie en comprend.
    #[test]
    fn every_library_fixture_survives_a_round_trip() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let mut count = 0;
        for manufacturer in std::fs::read_dir(root).unwrap().flatten().filter(|e| e.path().is_dir()) {
            for file in std::fs::read_dir(manufacturer.path()).unwrap().flatten() {
                let json = std::fs::read_to_string(file.path()).unwrap();
                let before = ofl::parse(&json, "a/b", "M").unwrap();
                let doc = FixtureDoc::from_json(&json).unwrap();
                let rewritten = doc.to_json();
                let after = ofl::parse(&rewritten, "a/b", "M").unwrap_or_else(|e| panic!("{:?} : {e}", file.path()));
                assert_eq!(before.modes, after.modes, "{:?}", file.path());
                assert_eq!(FixtureDoc::from_json(&rewritten).unwrap(), doc, "stable : {:?}", file.path());
                // Ce que l'éditeur ne touche pas est gardé (auteurs, liens…).
                let original: Value = serde_json::from_str(&json).unwrap();
                let saved: Value = serde_json::from_str(&rewritten).unwrap();
                for key in ["meta", "links", "physical"] {
                    assert_eq!(original.get(key), saved.get(key), "{key} : {:?}", file.path());
                }
                count += 1;
            }
        }
        assert!(count >= 6);
    }

    #[test]
    fn create_a_fixture_from_scratch() {
        let mut doc = FixtureDoc::new("Par maison");
        let mut red = ChannelDoc::new("Red");
        red.capabilities[0].set_kind("ColorIntensity");
        doc.channels.push(red);
        doc.modes[0].channels.push(Slot::Channel("Red".into()));
        // Dimmer en 16 bits, avec un mode qui utilise le fin.
        doc.channels[0].set_bytes(2);
        doc.channels[0].resolution = Some(16);
        doc.channels[0].capabilities[0].end = 65_535;
        doc.modes[0].channels.insert(1, Slot::Channel("Dimmer fine".into()));
        let fixture = ofl::parse(&doc.to_json(), "maison/par", "Maison").unwrap();
        let mode = &fixture.modes[0];
        assert_eq!(mode.footprint, 3);
        assert_eq!(mode.parameters[0].offsets, [0, 1]);
        assert_eq!(mode.parameters[1].attribute, crate::Attribute::ColorMix("Red".into()));
    }

    #[test]
    fn renaming_and_removing_follow_the_modes() {
        let mut doc = FixtureDoc::new("Essai");
        doc.channels[0].set_bytes(2);
        doc.modes[0].channels.push(Slot::Channel("Dimmer fine".into()));
        doc.rename_channel(0, "Intensité").unwrap();
        assert_eq!(doc.modes[0].channels, [Slot::Channel("Intensité".into()), Slot::Channel("Intensité fine".into())]);
        doc.channels.push(ChannelDoc::new("Autre"));
        assert!(doc.rename_channel(1, "Intensité").is_err(), "nom déjà pris");
        doc.remove_channel(0);
        assert!(doc.modes[0].channels.is_empty());
    }

    #[test]
    fn duplicated_mode_and_cells() {
        let mut doc = FixtureDoc::new("Barre");
        doc.duplicate_mode(0);
        assert_eq!(doc.modes[1].name, "Mode 1 (copie)");
        let mut template = ChannelDoc::new("Dimmer $pixelKey");
        template.capabilities[0].kind = "Intensity".into();
        doc.templates.push(template);
        doc.set_cell_count(4);
        doc.modes[1].channels.clear();
        doc.add_cells_to_mode(1, true);
        let fixture = ofl::parse(&doc.to_json(), "essai/barre", "Essai").unwrap();
        assert_eq!(fixture.modes[1].heads.len(), 4);
        assert_eq!(fixture.modes[1].footprint, 4);
        assert!(doc.slot_names().contains(&"Dimmer 3".to_string()));
    }

    #[test]
    fn changing_a_capability_type_drops_foreign_fields() {
        let mut cap = CapabilityDoc { start: 0, end: 255, kind: "Pan".into(), comment: String::new(), extra: Map::new() };
        cap.set_text("angleStart", "0deg");
        cap.set_kind("ColorIntensity");
        assert_eq!(cap.text("angleStart"), "");
        assert_eq!(cap.text("color"), "Red");
    }
}

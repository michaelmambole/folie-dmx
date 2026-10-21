//! Fichier de show Folie-DMX (`.folie`).
//!
//! Un show est un document JSON lisible et versionné. Il embarque une copie de
//! chaque fiche de fixture qu'il utilise : il s'ouvre donc à l'identique sur
//! une machine dont la bibliothèque est différente ou incomplète.
//!
//! La lecture est tolérante : une fixture qui ne peut pas être restaurée est
//! écartée avec un avertissement, le reste du show est chargé.

pub mod cues;
pub mod effects;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use folie_fixture::{FeatureGroup, FixtureId, FixtureType, Library, Patch, PatchedFixture, Target, ofl};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub use cues::{
    ChaserDirection, ChaserSettings, Cue, CueNumber, CueValue, DEFAULT_FADE, Executor, ExecutorMode, ExecutorOptions, FaderFunction,
    MAX_PAGE, ParamTiming, Priority, RestartMode, SLOTS_PER_PAGE, Sequence, executor_id, executor_label, executor_place,
};
pub use effects::{Effect, EffectLine, EffectRate, Waveform};

use effects::EffectEntry;

/// Tempo d'un nouveau show.
pub const DEFAULT_TEMPO: f32 = 120.0;

pub const FORMAT: &str = "folie-show";
/// Version du format écrite par ce programme. À incrémenter à chaque changement
/// incompatible, avec une conversion depuis les versions précédentes.
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "folie";

#[derive(Debug, Error)]
pub enum ShowError {
    #[error("{path} : {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("fichier illisible : {0}")]
    Json(#[from] serde_json::Error),
    #[error("ce fichier n'est pas un show Folie-DMX")]
    NotAShow,
    #[error("show enregistré par une version plus récente de Folie-DMX (format {0})")]
    TooNew(u32),
}

/// Contenu d'un show.
#[derive(Debug, Clone, PartialEq)]
pub struct Show {
    pub patch: Patch,
    /// Groupes par numéro dans le pool.
    pub groups: BTreeMap<u32, Group>,
    /// Presets par (famille, numéro) : `Preset 4.1` = (Couleur, 1).
    pub presets: BTreeMap<(FeatureGroup, u32), Preset>,
    /// Séquences (listes de cues) par numéro.
    pub sequences: BTreeMap<u32, Sequence>,
    /// Executors par numéro (voir [`executor_id`]) : sur chaque page, 1 à 10 avec fader,
    /// 11 à 20 en boutons.
    pub executors: BTreeMap<u32, Executor>,
    /// Noms des pages d'executors, par numéro de page.
    pub pages: BTreeMap<u32, String>,
    /// Battements par minute, pour les effets calés sur le tempo.
    pub tempo: f32,
    /// Effets enregistrés par l'utilisateur (« Mes effets »), sans cibles : on les relance
    /// sur la sélection du moment.
    pub effect_pool: BTreeMap<u32, Effect>,
    /// Place des projecteurs dans la fenêtre Scène ; les autres sont alignés sur une perche.
    pub stage: BTreeMap<FixtureId, StagePlace>,
    /// Macros par numéro : des lignes de commande jouées l'une après l'autre.
    pub macros: BTreeMap<u32, Macro>,
    /// Executors qui lancent une macro au lieu de jouer une séquence : executor -> macro.
    pub macro_executors: BTreeMap<u32, u32>,
    /// Destinations OSC nommées, pour la commande `Osc "Reaper" "/play"` : nom -> « adresse:port ».
    pub osc_destinations: BTreeMap<String, String>,
}

/// Une macro : des lignes de commande jouées dans l'ordre ; `Wait 2` attend 2 secondes.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Macro {
    pub name: String,
    pub lines: Vec<String>,
}

/// Place d'un projecteur sur scène, en mètres : `x` vers cour, `y` la hauteur, `z` vers le public.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct StagePlace {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// Posé au sol, faisceau vers le haut (sinon suspendu, faisceau vers le bas).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub floor: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            patch: Patch::default(),
            groups: BTreeMap::new(),
            presets: BTreeMap::new(),
            sequences: BTreeMap::new(),
            executors: BTreeMap::new(),
            pages: BTreeMap::new(),
            tempo: DEFAULT_TEMPO,
            effect_pool: BTreeMap::new(),
            stage: BTreeMap::new(),
            macros: BTreeMap::new(),
            macro_executors: BTreeMap::new(),
            osc_destinations: BTreeMap::new(),
        }
    }
}

/// Sélection enregistrée, dans l'ordre où elle a été faite (utile aux effets).
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub targets: Vec<Target>,
}

/// Valeurs enregistrées pour une famille d'attributs, fixture par fixture.
/// Les paramètres sont désignés par leur nom, plus stable que leur position.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Preset {
    pub name: String,
    pub values: BTreeMap<FixtureId, BTreeMap<String, u16>>,
}

/// Show lu, avec ce qui n'a pas pu être restauré.
#[derive(Debug)]
pub struct Loaded {
    pub show: Show,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct ShowFile {
    format: String,
    version: u32,
    #[serde(default)]
    saved_with: String,
    #[serde(default)]
    fixture_types: BTreeMap<String, EmbeddedType>,
    #[serde(default)]
    patch: Vec<PatchEntry>,
    #[serde(default)]
    groups: Vec<GroupEntry>,
    #[serde(default)]
    presets: Vec<PresetEntry>,
    #[serde(default)]
    sequences: Vec<SequenceEntry>,
    /// Executor (en texte, clé JSON) -> {"sequence": 1, "mode": "go"}.
    /// Un simple numéro de séquence (anciens shows) est accepté aussi.
    #[serde(default)]
    executors: BTreeMap<String, Value>,
    /// Page -> nom.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pages: BTreeMap<String, String>,
    #[serde(default = "default_tempo")]
    tempo: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    effect_pool: Vec<PoolEffectEntry>,
    /// Fixture -> place sur scène.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    stage: BTreeMap<String, StagePlace>,
    /// Numéro -> macro.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    macros: BTreeMap<String, Macro>,
    /// Executor -> macro qu'il lance.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    macro_executors: BTreeMap<String, u32>,
    /// Nom -> « adresse:port ».
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    osc_destinations: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize)]
struct PoolEffectEntry {
    number: u32,
    effect: EffectEntry,
}

fn default_tempo() -> f32 {
    DEFAULT_TEMPO
}

#[derive(Serialize, Deserialize)]
struct SequenceEntry {
    number: u32,
    name: String,
    cues: Vec<CueEntry>,
}

#[derive(Serialize, Deserialize)]
struct CueEntry {
    /// "1", "1.5"…
    number: String,
    name: String,
    fade: f32,
    #[serde(default)]
    delay: f32,
    /// Fixture -> paramètre -> valeur : un nombre, "preset 4.1" ou "release".
    values: BTreeMap<String, BTreeMap<String, Value>>,
    /// Absent : les effets précédents continuent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    effects: Option<Vec<EffectEntry>>,
    /// Famille ("position"…) -> temps propres.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    family_times: BTreeMap<String, TimingEntry>,
    /// Fixture -> paramètre -> temps propres.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    param_times: BTreeMap<String, BTreeMap<String, TimingEntry>>,
    /// Effets suivis arrêtés dans cette cue : la cue qui les a lancés et leur place.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    stopped_effects: Vec<StoppedEffectEntry>,
    /// Commande jouée au départ de la cue.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    command: String,
}

#[derive(Serialize, Deserialize)]
struct StoppedEffectEntry {
    cue: String,
    index: usize,
}

/// Temps propres dans le fichier ; un temps absent est celui de la cue.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
struct TimingEntry {
    #[serde(skip_serializing_if = "Option::is_none")]
    fade: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delay: Option<f32>,
}

impl TimingEntry {
    fn from_timing(t: &ParamTiming) -> Self {
        Self { fade: t.fade, delay: t.delay }
    }

    fn into_timing(self) -> ParamTiming {
        let clamp = |t: Option<f32>| t.map(|t| t.clamp(0.0, 3600.0));
        ParamTiming { fade: clamp(self.fade), delay: clamp(self.delay) }
    }
}

/// Options d'un executor dans le fichier ; tout champ absent prend sa valeur par défaut.
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct OptionsEntry {
    wrap: bool,
    auto_start: bool,
    auto_stop: bool,
    fader: String,
    off_time: f32,
    back_time: Option<f32>,
    priority: String,
    protected: bool,
    fixed: bool,
    chaser: bool,
    chaser_bpm: f32,
    chaser_follow_tempo: bool,
    /// Fondu entre pas, en fraction du pas.
    chaser_step_fade: f32,
    chaser_direction: String,
    restart: String,
    ooo: bool,
    mib: bool,
    swop_protect: bool,
    kill_protect: bool,
}

impl Default for OptionsEntry {
    fn default() -> Self {
        Self::from_options(&ExecutorOptions::default())
    }
}

impl OptionsEntry {
    fn from_options(o: &ExecutorOptions) -> Self {
        Self {
            wrap: o.wrap,
            auto_start: o.auto_start,
            auto_stop: o.auto_stop,
            fader: o.fader.key().into(),
            off_time: o.off_time,
            back_time: o.back_time,
            priority: o.priority.key().into(),
            protected: o.protected,
            fixed: o.fixed,
            chaser: o.chaser,
            chaser_bpm: o.chaser_settings.bpm,
            chaser_follow_tempo: o.chaser_settings.follow_tempo,
            chaser_step_fade: o.chaser_settings.step_fade,
            chaser_direction: o.chaser_settings.direction.key().into(),
            restart: o.restart.key().into(),
            ooo: o.ooo,
            mib: o.mib,
            swop_protect: o.swop_protect,
            kill_protect: o.kill_protect,
        }
    }

    fn into_options(self) -> ExecutorOptions {
        ExecutorOptions {
            wrap: self.wrap,
            auto_start: self.auto_start,
            auto_stop: self.auto_stop,
            fader: FaderFunction::from_key(&self.fader).unwrap_or_default(),
            off_time: self.off_time.clamp(0.0, 3600.0),
            back_time: self.back_time.map(|t| t.clamp(0.0, 3600.0)),
            priority: Priority::from_key(&self.priority).unwrap_or_default(),
            protected: self.protected,
            fixed: self.fixed,
            chaser: self.chaser,
            chaser_settings: ChaserSettings {
                bpm: if self.chaser_bpm.is_finite() { self.chaser_bpm.clamp(1.0, 999.0) } else { 120.0 },
                follow_tempo: self.chaser_follow_tempo,
                step_fade: if self.chaser_step_fade.is_finite() { self.chaser_step_fade.clamp(0.0, 1.0) } else { 0.0 },
                direction: ChaserDirection::from_key(&self.chaser_direction).unwrap_or_default(),
            },
            restart: RestartMode::from_key(&self.restart).unwrap_or_default(),
            ooo: self.ooo,
            mib: self.mib,
            swop_protect: self.swop_protect,
            kill_protect: self.kill_protect,
        }
    }
}

fn cue_value_to_json(value: &CueValue) -> Value {
    match value {
        CueValue::Value(v) => Value::from(*v),
        CueValue::Preset(family, number) => Value::from(format!("preset {}.{number}", family.preset_number())),
        CueValue::Release => Value::from("release"),
    }
}

fn cue_value_from_json(value: &Value) -> Option<CueValue> {
    match value {
        Value::Number(n) => n.as_u64().and_then(|v| u16::try_from(v).ok()).map(CueValue::Value),
        Value::String(s) if s == "release" => Some(CueValue::Release),
        Value::String(s) => {
            let (family, number) = s.strip_prefix("preset ")?.split_once('.')?;
            let family = FeatureGroup::from_preset_number(family.parse().ok()?)?;
            Some(CueValue::Preset(family, number.parse().ok()?))
        }
        _ => None,
    }
}

#[derive(Serialize, Deserialize)]
struct GroupEntry {
    number: u32,
    name: String,
    /// "5" ou "5.3", dans l'ordre de sélection.
    targets: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct PresetEntry {
    family: String,
    number: u32,
    name: String,
    /// Numéro de fixture (en texte, clé JSON) -> paramètre -> valeur 16 bits.
    values: BTreeMap<String, BTreeMap<String, u16>>,
}

#[derive(Serialize, Deserialize)]
struct EmbeddedType {
    manufacturer: String,
    /// Fiche OFL d'origine, telle quelle.
    ofl: Value,
}

#[derive(Serialize, Deserialize)]
struct PatchEntry {
    id: u32,
    name: String,
    #[serde(rename = "type")]
    fixture_type: String,
    /// Nom du mode plutôt que son index : il reste valable si la fiche évolue.
    mode: String,
    /// Univers en base 1, comme dans l'interface.
    universe: usize,
    address: u16,
}

pub fn to_json(show: &Show) -> String {
    let mut fixture_types = BTreeMap::new();
    for f in show.patch.fixtures() {
        let t = &f.fixture_type;
        fixture_types.entry(t.key.clone()).or_insert_with(|| EmbeddedType {
            manufacturer: t.manufacturer.clone(),
            ofl: serde_json::from_str(&t.ofl_source).unwrap_or(Value::Null),
        });
    }
    let patch = show
        .patch
        .fixtures()
        .iter()
        .map(|f| PatchEntry {
            id: f.id.0,
            name: f.name.clone(),
            fixture_type: f.fixture_type.key.clone(),
            mode: f.mode().name.clone(),
            universe: f.universe + 1,
            address: f.address,
        })
        .collect();
    let groups = show
        .groups
        .iter()
        .map(|(number, g)| GroupEntry {
            number: *number,
            name: g.name.clone(),
            targets: g.targets.iter().map(Target::to_string).collect(),
        })
        .collect();
    let presets = show
        .presets
        .iter()
        .map(|((family, number), p)| PresetEntry {
            family: family.key().into(),
            number: *number,
            name: p.name.clone(),
            values: p.values.iter().map(|(id, v)| (id.to_string(), v.clone())).collect(),
        })
        .collect();
    let sequences = show
        .sequences
        .iter()
        .map(|(number, sequence)| SequenceEntry {
            number: *number,
            name: sequence.name.clone(),
            cues: sequence
                .cues
                .iter()
                .map(|(n, cue)| CueEntry {
                    number: n.to_string(),
                    name: cue.name.clone(),
                    fade: cue.fade,
                    delay: cue.delay,
                    values: cue
                        .values
                        .iter()
                        .map(|(id, params)| {
                            (id.to_string(), params.iter().map(|(p, v)| (p.clone(), cue_value_to_json(v))).collect())
                        })
                        .collect(),
                    effects: cue.effects.as_ref().map(|e| e.iter().map(EffectEntry::from_effect).collect()),
                    family_times: cue
                        .family_times
                        .iter()
                        .map(|(family, t)| (family.key().to_string(), TimingEntry::from_timing(t)))
                        .collect(),
                    param_times: cue
                        .param_times
                        .iter()
                        .map(|(id, params)| {
                            (id.to_string(), params.iter().map(|(p, t)| (p.clone(), TimingEntry::from_timing(t))).collect())
                        })
                        .collect(),
                    stopped_effects: cue
                        .stopped_effects
                        .iter()
                        .map(|(origin, index)| StoppedEffectEntry { cue: origin.to_string(), index: *index })
                        .collect(),
                    command: cue.command.clone(),
                })
                .collect(),
        })
        .collect();
    let executors = show
        .executors
        .iter()
        .map(|(e, x)| {
            let options = serde_json::to_value(OptionsEntry::from_options(&x.options)).unwrap_or_default();
            (e.to_string(), serde_json::json!({ "sequence": x.sequence, "mode": x.mode.key(), "options": options }))
        })
        .collect();
    let file = ShowFile {
        format: FORMAT.into(),
        version: VERSION,
        saved_with: concat!("Folie-DMX ", env!("CARGO_PKG_VERSION")).into(),
        fixture_types,
        patch,
        groups,
        presets,
        sequences,
        executors,
        pages: show.pages.iter().map(|(n, name)| (n.to_string(), name.clone())).collect(),
        tempo: show.tempo,
        effect_pool: show
            .effect_pool
            .iter()
            .map(|(number, effect)| PoolEffectEntry { number: *number, effect: EffectEntry::from_effect(effect) })
            .collect(),
        stage: show.stage.iter().map(|(id, place)| (id.to_string(), *place)).collect(),
        macros: show.macros.iter().map(|(n, m)| (n.to_string(), m.clone())).collect(),
        macro_executors: show.macro_executors.iter().map(|(e, m)| (e.to_string(), *m)).collect(),
        osc_destinations: show.osc_destinations.clone(),
    };
    serde_json::to_string_pretty(&file).expect("un show se sérialise toujours")
}

pub fn from_json(json: &str, library: &Library) -> Result<Loaded, ShowError> {
    // Vérifie l'en-tête avant tout le reste, pour un message d'erreur clair.
    let header: Value = serde_json::from_str(json)?;
    if header.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(ShowError::NotAShow);
    }
    let version = header.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
    if version > VERSION {
        return Err(ShowError::TooNew(version));
    }
    let file: ShowFile = serde_json::from_value(header)?;

    let mut warnings = Vec::new();
    let types = resolve_types(&file.fixture_types, library, &mut warnings);

    let mut patch = Patch::default();
    for entry in file.patch {
        let what = format!("Fixture {} ({})", entry.id, entry.name);
        let Some(fixture_type) = types.get(&entry.fixture_type) else {
            warnings.push(format!("{what} : fiche « {} » introuvable", entry.fixture_type));
            continue;
        };
        let Some(mode) = fixture_type.modes.iter().position(|m| m.name == entry.mode) else {
            warnings.push(format!("{what} : mode « {} » introuvable", entry.mode));
            continue;
        };
        let fixture = PatchedFixture {
            id: FixtureId(entry.id),
            name: entry.name,
            fixture_type: fixture_type.clone(),
            mode,
            universe: entry.universe.saturating_sub(1),
            address: entry.address,
        };
        if let Err(err) = patch.add(fixture) {
            warnings.push(format!("{what} : {err}"));
        }
    }
    let mut groups = BTreeMap::new();
    for entry in file.groups {
        let mut targets = Vec::new();
        for text in &entry.targets {
            match text.parse::<Target>() {
                Ok(t) if patch.contains(t) => targets.push(t),
                _ => warnings.push(format!("Groupe {} : « {text} » n'est pas patché, retiré", entry.number)),
            }
        }
        groups.insert(entry.number, Group { name: entry.name, targets });
    }

    let mut presets = BTreeMap::new();
    for entry in file.presets {
        let Some(family) = FeatureGroup::from_key(&entry.family) else {
            warnings.push(format!("Preset « {} » : famille inconnue « {} »", entry.name, entry.family));
            continue;
        };
        let mut values = BTreeMap::new();
        for (id, parameters) in entry.values {
            match id.parse::<u32>().map(FixtureId) {
                Ok(id) if patch.get(id).is_some() => {
                    values.insert(id, parameters);
                }
                _ => warnings.push(format!(
                    "Preset {}.{} : fixture {id} absente du patch, valeurs retirées",
                    family.preset_number(),
                    entry.number
                )),
            }
        }
        presets.insert((family, entry.number), Preset { name: entry.name, values });
    }

    let mut sequences = BTreeMap::new();
    for entry in file.sequences {
        let mut cues = BTreeMap::new();
        for cue_entry in entry.cues {
            let what = format!("Séquence {}, cue {}", entry.number, cue_entry.number);
            let Ok(number) = cue_entry.number.parse::<CueNumber>() else {
                warnings.push(format!("{what} : numéro illisible, cue retirée"));
                continue;
            };
            let effects = cue_entry.effects.map(|entries| {
                entries.into_iter().filter_map(|e| e.into_effect(|t| patch.contains(t))).collect()
            });
            let mut cue = Cue {
                name: cue_entry.name,
                fade: cue_entry.fade.max(0.0),
                delay: cue_entry.delay.max(0.0),
                values: BTreeMap::new(),
                effects,
                family_times: BTreeMap::new(),
                param_times: BTreeMap::new(),
                stopped_effects: cue_entry
                    .stopped_effects
                    .iter()
                    .filter_map(|s| Some((s.cue.parse::<CueNumber>().ok()?, s.index)))
                    .collect(),
                command: cue_entry.command,
            };
            for (family, timing) in cue_entry.family_times {
                match FeatureGroup::from_key(&family) {
                    Some(family) => {
                        cue.family_times.insert(family, timing.into_timing());
                    }
                    None => warnings.push(format!("{what} : famille « {family} » inconnue, temps ignorés")),
                }
            }
            for (id, params) in cue_entry.param_times {
                match id.parse::<u32>().ok().map(FixtureId).filter(|id| patch.get(*id).is_some()) {
                    Some(id) => {
                        let times = params.into_iter().map(|(p, t)| (p, t.into_timing())).collect();
                        cue.param_times.insert(id, times);
                    }
                    None => warnings.push(format!("{what} : fixture {id} absente du patch, temps retirés")),
                }
            }
            for (id, params) in cue_entry.values {
                let Some(id) = id.parse::<u32>().ok().map(FixtureId).filter(|id| patch.get(*id).is_some()) else {
                    warnings.push(format!("{what} : fixture {id} absente du patch, valeurs retirées"));
                    continue;
                };
                let mut values = BTreeMap::new();
                for (param, raw) in params {
                    match cue_value_from_json(&raw) {
                        Some(v) => {
                            values.insert(param, v);
                        }
                        None => warnings.push(format!("{what} : valeur illisible pour {param}")),
                    }
                }
                cue.values.insert(id, values);
            }
            cues.insert(number, cue);
        }
        sequences.insert(entry.number, Sequence { name: entry.name, cues });
    }
    let mut executors = BTreeMap::new();
    for (executor, raw) in file.executors {
        let (sequence, mode, options) = match &raw {
            Value::Number(n) => (n.as_u64(), Some(ExecutorMode::Go), ExecutorOptions::default()),
            Value::Object(o) => (
                o.get("sequence").and_then(Value::as_u64),
                o.get("mode").and_then(Value::as_str).map_or(Some(ExecutorMode::Go), ExecutorMode::from_key),
                o.get("options")
                    .and_then(|v| serde_json::from_value::<OptionsEntry>(v.clone()).ok())
                    .map(OptionsEntry::into_options)
                    .unwrap_or_default(),
            ),
            _ => (None, None, ExecutorOptions::default()),
        };
        match (executor.parse::<u32>(), sequence.map(|s| s as u32), mode) {
            (Ok(executor), Some(sequence), Some(mode)) if sequences.contains_key(&sequence) => {
                executors.insert(executor, Executor { sequence, mode, options });
            }
            _ => warnings.push(format!("Executor {executor} : illisible ou séquence introuvable, retiré")),
        }
    }

    let mut pages = BTreeMap::new();
    for (page, name) in file.pages {
        match page.parse::<u32>() {
            Ok(n) if (1..=MAX_PAGE).contains(&n) => {
                pages.insert(n, name);
            }
            _ => warnings.push(format!("Page « {page} » : numéro invalide, nom ignoré")),
        }
    }

    let mut effect_pool = BTreeMap::new();
    for entry in file.effect_pool {
        match entry.effect.into_template() {
            Some(effect) => {
                effect_pool.insert(entry.number, effect);
            }
            None => warnings.push(format!("Effet {} du pool : illisible, retiré", entry.number)),
        }
    }

    let mut stage = BTreeMap::new();
    for (id, place) in file.stage {
        match id.parse::<u32>().ok().map(FixtureId).filter(|id| patch.get(*id).is_some()) {
            Some(id) => {
                stage.insert(id, place);
            }
            None => warnings.push(format!("Scène : fixture {id} absente du patch, place retirée")),
        }
    }

    let mut macros = BTreeMap::new();
    for (number, m) in file.macros {
        match number.parse::<u32>() {
            Ok(n) => {
                macros.insert(n, m);
            }
            Err(_) => warnings.push(format!("Macro « {number} » : numéro illisible, retirée")),
        }
    }
    let mut macro_executors = BTreeMap::new();
    for (executor, number) in file.macro_executors {
        match executor.parse::<u32>() {
            Ok(e) if macros.contains_key(&number) => {
                macro_executors.insert(e, number);
            }
            _ => warnings.push(format!("Executor {executor} : macro {number} introuvable, retirée")),
        }
    }

    let tempo = file.tempo.clamp(1.0, 999.0);
    let osc_destinations = file.osc_destinations;
    let show =
        Show { patch, groups, presets, sequences, executors, pages, tempo, effect_pool, stage, macros, macro_executors, osc_destinations };
    Ok(Loaded { show, warnings })
}

/// Fiche à utiliser pour chaque type du show : la copie embarquée en priorité,
/// la bibliothèque en secours.
fn resolve_types(
    embedded: &BTreeMap<String, EmbeddedType>,
    library: &Library,
    warnings: &mut Vec<String>,
) -> BTreeMap<String, Arc<FixtureType>> {
    let mut types = BTreeMap::new();
    for (key, e) in embedded {
        let in_library = library.get(key);
        let parsed = (!e.ofl.is_null())
            .then(|| ofl::parse(&e.ofl.to_string(), key, &e.manufacturer))
            .transpose();
        let fixture_type = match (parsed, in_library) {
            (Ok(Some(t)), library_type) => {
                let same = library_type
                    .and_then(|l| serde_json::from_str::<Value>(&l.ofl_source).ok())
                    .is_none_or(|l| l == e.ofl);
                if !same {
                    warnings.push(format!(
                        "La fiche « {key} » du show diffère de la bibliothèque : la version du show est utilisée"
                    ));
                }
                Arc::new(t)
            }
            (Ok(None), Some(l)) => l.clone(),
            (Err(err), Some(l)) => {
                warnings.push(format!("Fiche « {key} » du show illisible ({err}) : version de la bibliothèque utilisée"));
                l.clone()
            }
            (Ok(None), None) => continue,
            (Err(err), None) => {
                warnings.push(format!("Fiche « {key} » du show illisible : {err}"));
                continue;
            }
        };
        types.insert(key.clone(), fixture_type);
    }
    // Un show écrit à la main peut omettre les fiches : on complète avec la bibliothèque.
    for t in &library.fixtures {
        types.entry(t.key.clone()).or_insert_with(|| t.clone());
    }
    types
}

/// Écrit le show de façon atomique : un fichier temporaire est écrit puis
/// renommé, si bien qu'une coupure ne laisse jamais un show à moitié écrit.
pub fn save(show: &Show, path: &Path) -> Result<(), ShowError> {
    let io = |source| ShowError::Io { path: path.to_path_buf(), source };
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);

    let mut file = std::fs::File::create(&tmp).map_err(io)?;
    file.write_all(to_json(show).as_bytes()).map_err(io)?;
    file.sync_all().map_err(io)?;
    drop(file);
    std::fs::rename(&tmp, path).map_err(io)
}

pub fn load(path: &Path, library: &Library) -> Result<Loaded, ShowError> {
    let json = std::fs::read_to_string(path)
        .map_err(|source| ShowError::Io { path: path.to_path_buf(), source })?;
    from_json(&json, library)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Library {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let (library, errors) = Library::load_dir(root);
        assert!(errors.is_empty());
        library
    }

    fn sample(library: &Library) -> Show {
        let mut patch = Patch::default();
        let mut add = |id, key: &str, mode, address| {
            let fixture_type = library.get(key).unwrap().clone();
            let name = format!("{} {id}", fixture_type.name);
            patch
                .add(PatchedFixture { id: FixtureId(id), name, fixture_type, mode, universe: 0, address })
                .unwrap();
        };
        add(1, "martin/rush-mh-6-wash", 0, 1);
        add(2, "martin/rush-mh-6-wash", 0, 15);
        add(10, "showtec/sunstrip-active-mkii", 3, 101);
        let groups = BTreeMap::from([(
            3,
            Group { name: "Cour".into(), targets: vec!["2".parse().unwrap(), "10.4".parse().unwrap()] },
        )]);
        let values = BTreeMap::from([(FixtureId(1), BTreeMap::from([("Red".to_string(), 65_535)]))]);
        let presets = BTreeMap::from([((FeatureGroup::Color, 1), Preset { name: "Rouge".into(), values })]);
        let mut cue = Cue::new("Entrée".into());
        cue.fade = 4.5;
        cue.values.insert(
            FixtureId(1),
            BTreeMap::from([
                ("Dimmer".to_string(), CueValue::Value(65_535)),
                ("Red".to_string(), CueValue::Preset(FeatureGroup::Color, 1)),
                ("Pan".to_string(), CueValue::Release),
            ]),
        );
        let mut wave = effects::templates()[2].clone();
        wave.targets = vec!["1".parse().unwrap(), "10.2".parse().unwrap()];
        let mut circle = effects::templates()[3].clone();
        circle.targets = vec!["1".parse().unwrap()];
        assert!(circle.lines.iter().all(|l| l.relative), "le cercle tourne autour de la position");
        let mut chaser = effects::templates()[1].clone();
        chaser.lines[0].duty = 20.0;
        chaser.fade_in = 2.0;
        chaser.fade_out = 3.5;
        chaser.targets = vec!["1".parse().unwrap()];
        let mut sparkle = effects::templates()[6].clone();
        assert_eq!(sparkle.lines[0].waveform, Waveform::Random);
        sparkle.targets = vec!["1".parse().unwrap()];
        cue.effects = Some(vec![wave, circle, chaser, sparkle]);
        cue.family_times.insert(FeatureGroup::Position, ParamTiming { fade: Some(4.0), delay: None });
        cue.param_times
            .entry(FixtureId(1))
            .or_default()
            .insert("Pan".into(), ParamTiming { fade: Some(6.0), delay: Some(1.0) });
        cue.command = r#"Osc "Reaper" "/marker/1"; Osc "Reaper" "/play""#.into();
        let mut cues = BTreeMap::from([(CueNumber::whole(1), cue)]);
        let mut black = Cue::new("Noir".into());
        black.stopped_effects.insert((CueNumber::whole(1), 1)); // le cercle s'arrête, la vague continue
        cues.insert("1.5".parse().unwrap(), black);
        let sequences = BTreeMap::from([(1, Sequence { name: "Spectacle".into(), cues })]);
        let executors = BTreeMap::from([
            (1, Executor::new(1)),
            (11, Executor { sequence: 1, mode: ExecutorMode::Flash, options: ExecutorOptions::default() }),
            (
                3,
                Executor {
                    sequence: 1,
                    mode: ExecutorMode::Swop,
                    options: ExecutorOptions {
                        chaser: true,
                        chaser_settings: ChaserSettings {
                            bpm: 90.0,
                            follow_tempo: true,
                            step_fade: 0.5,
                            direction: ChaserDirection::Bounce,
                        },
                        fader: FaderFunction::Crossfade,
                        restart: RestartMode::Current,
                        ooo: true,
                        mib: true,
                        swop_protect: true,
                        kill_protect: true,
                        ..Default::default()
                    },
                },
            ),
            (
                2,
                Executor {
                    sequence: 1,
                    mode: ExecutorMode::Go,
                    options: ExecutorOptions {
                        wrap: true,
                        fader: FaderFunction::Temp,
                        off_time: 3.0,
                        back_time: Some(1.5),
                        priority: Priority::High,
                        protected: true,
                        ..Default::default()
                    },
                },
            ),
            (
                executor_id(2, 1),
                Executor {
                    sequence: 1,
                    mode: ExecutorMode::Go,
                    options: ExecutorOptions { fixed: true, ..Default::default() },
                },
            ),
        ]);
        let pages = BTreeMap::from([(2, "Acte 2".to_string())]);
        let mut ballet = effects::templates()[3].clone();
        ballet.name = "Ballet".into();
        ballet.spread = 180.0;
        let effect_pool = BTreeMap::from([(2, ballet)]);
        let stage = BTreeMap::from([(FixtureId(1), StagePlace { x: -2.5, y: 0.0, z: 3.0, floor: true })]);
        let lines = vec!["Fixture 1 Thru 4".to_string(), "At 100".into(), "Wait 2".into(), "Go Executor 2".into()];
        let macros = BTreeMap::from([(3, Macro { name: "Ouverture".into(), lines })]);
        let macro_executors = BTreeMap::from([(7, 3)]);
        let osc_destinations = BTreeMap::from([("Reaper".to_string(), "127.0.0.1:8000".to_string())]);
        Show {
            patch,
            groups,
            presets,
            sequences,
            executors,
            pages,
            tempo: 98.0,
            effect_pool,
            stage,
            macros,
            macro_executors,
            osc_destinations,
        }
    }

    fn summary(show: &Show) -> Vec<(u32, String, String, u16)> {
        show.patch
            .fixtures()
            .iter()
            .map(|f| (f.id.0, f.fixture_type.key.clone(), f.mode().name.clone(), f.address))
            .collect()
    }

    #[test]
    fn round_trip_through_a_file() {
        let library = library();
        let show = sample(&library);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("concert.folie");

        save(&show, &path).unwrap();
        let loaded = load(&path, &library).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(summary(&loaded.show), summary(&show));
        assert_eq!(loaded.show.patch.fixtures()[2].mode().name, "Individual");
        assert_eq!(loaded.show.groups, show.groups);
        assert_eq!(loaded.show.presets, show.presets);
        assert_eq!(loaded.show.sequences, show.sequences);
        assert_eq!(loaded.show.executors, show.executors);
        assert_eq!(loaded.show.pages, show.pages);
        assert_eq!(loaded.show.effect_pool, show.effect_pool);
        assert_eq!(loaded.show.stage, show.stage);
        assert_eq!(loaded.show.macros, show.macros);
        assert_eq!(loaded.show.macro_executors, show.macro_executors);
        assert_eq!(loaded.show.tempo, 98.0);
        // L'écriture atomique ne laisse rien derrière elle.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn opens_without_the_library_thanks_to_embedded_types() {
        let library = library();
        let json = to_json(&sample(&library));
        let loaded = from_json(&json, &Library::default()).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(summary(&loaded.show), summary(&sample(&library)));
    }

    #[test]
    fn tolerates_damaged_entries() {
        let library = library();
        let mut file: Value = serde_json::from_str(&to_json(&sample(&library))).unwrap();
        file["patch"][0]["mode"] = "Mode disparu".into();
        file["patch"][1]["address"] = 600.into();
        let loaded = from_json(&file.to_string(), &library).unwrap();
        assert_eq!(loaded.show.patch.fixtures().len(), 1);
        // 2 fixtures écartées, puis le groupe, le preset, la cue (valeurs et temps propres) et
        // la place sur scène qui les utilisaient sont nettoyés.
        assert_eq!(loaded.warnings.len(), 7, "{:?}", loaded.warnings);
        assert!(loaded.warnings.iter().any(|w| w == "Scène : fixture 1 absente du patch, place retirée"));
        assert!(loaded.warnings.iter().any(|w| w.ends_with("fixture 1 absente du patch, temps retirés")));
    }

    #[test]
    fn drops_group_and_preset_entries_for_missing_fixtures() {
        let library = library();
        let mut file: Value = serde_json::from_str(&to_json(&sample(&library))).unwrap();
        file["groups"][0]["targets"] = serde_json::json!(["2", "99", "10.40"]);
        file["presets"][0]["values"]["99"] = serde_json::json!({ "Red": 1 });
        let loaded = from_json(&file.to_string(), &library).unwrap();
        assert_eq!(loaded.show.groups[&3].targets, ["2".parse().unwrap()]);
        assert_eq!(loaded.show.presets[&(FeatureGroup::Color, 1)].values.len(), 1);
        assert_eq!(loaded.warnings.len(), 3, "{:?}", loaded.warnings);
    }

    #[test]
    fn reads_executors_of_older_shows() {
        let library = library();
        let mut file: Value = serde_json::from_str(&to_json(&sample(&library))).unwrap();
        file["executors"] = serde_json::json!({ "1": 1, "2": { "sequence": 1, "mode": "toggle" }, "3": 99 });
        let loaded = from_json(&file.to_string(), &library).unwrap();
        assert_eq!(loaded.show.executors[&1], Executor::new(1));
        assert_eq!(loaded.show.executors[&2].mode, ExecutorMode::Toggle);
        assert_eq!(loaded.show.executors[&2].options, ExecutorOptions::default(), "options absentes : valeurs par défaut");
        assert!(!loaded.show.executors.contains_key(&3), "séquence 99 inexistante");
    }

    #[test]
    fn refuses_foreign_and_newer_files() {
        let library = library();
        assert!(matches!(from_json("{}", &library), Err(ShowError::NotAShow)));
        let newer = r#"{ "format": "folie-show", "version": 99 }"#;
        assert!(matches!(from_json(newer, &library), Err(ShowError::TooNew(99))));
    }

    #[test]
    fn warns_when_embedded_type_differs_from_library() {
        let library = library();
        let mut file: Value = serde_json::from_str(&to_json(&sample(&library))).unwrap();
        file["fixture_types"]["martin/rush-mh-6-wash"]["ofl"]["comment"] = "modifiée".into();
        let loaded = from_json(&file.to_string(), &library).unwrap();
        assert_eq!(loaded.show.patch.fixtures().len(), 3);
        assert!(loaded.warnings[0].contains("diffère"), "{:?}", loaded.warnings);
    }
}

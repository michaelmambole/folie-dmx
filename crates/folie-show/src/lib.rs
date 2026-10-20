//! Fichier de show Folie-DMX (`.folie`).
//!
//! Un show est un document JSON lisible et versionné. Il embarque une copie de
//! chaque fiche de fixture qu'il utilise : il s'ouvre donc à l'identique sur
//! une machine dont la bibliothèque est différente ou incomplète.
//!
//! La lecture est tolérante : une fixture qui ne peut pas être restaurée est
//! écartée avec un avertissement, le reste du show est chargé.

pub mod cues;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use folie_fixture::{FeatureGroup, FixtureId, FixtureType, Library, Patch, PatchedFixture, Target, ofl};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub use cues::{Cue, CueNumber, CueValue, DEFAULT_FADE, Sequence};

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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Show {
    pub patch: Patch,
    /// Groupes par numéro dans le pool.
    pub groups: BTreeMap<u32, Group>,
    /// Presets par (famille, numéro) : `Preset 4.1` = (Couleur, 1).
    pub presets: BTreeMap<(FeatureGroup, u32), Preset>,
    /// Séquences (listes de cues) par numéro.
    pub sequences: BTreeMap<u32, Sequence>,
    /// Executor -> séquence qu'il joue.
    pub executors: BTreeMap<u32, u32>,
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
    /// Executor (en texte, clé JSON) -> séquence.
    #[serde(default)]
    executors: BTreeMap<String, u32>,
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
                })
                .collect(),
        })
        .collect();
    let executors = show.executors.iter().map(|(e, s)| (e.to_string(), *s)).collect();
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
            let mut cue = Cue {
                name: cue_entry.name,
                fade: cue_entry.fade.max(0.0),
                delay: cue_entry.delay.max(0.0),
                values: BTreeMap::new(),
            };
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
    for (executor, sequence) in file.executors {
        match executor.parse::<u32>() {
            Ok(executor) if sequences.contains_key(&sequence) => {
                executors.insert(executor, sequence);
            }
            _ => warnings.push(format!("Executor {executor} : séquence {sequence} introuvable")),
        }
    }

    Ok(Loaded { show: Show { patch, groups, presets, sequences, executors }, warnings })
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
        let mut cues = BTreeMap::from([(CueNumber::whole(1), cue)]);
        cues.insert("1.5".parse().unwrap(), Cue::new("Noir".into()));
        let sequences = BTreeMap::from([(1, Sequence { name: "Spectacle".into(), cues })]);
        let executors = BTreeMap::from([(1, 1)]);
        Show { patch, groups, presets, sequences, executors }
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
        // 2 fixtures écartées, puis le groupe, le preset et la cue qui les utilisaient sont nettoyés.
        assert_eq!(loaded.warnings.len(), 5, "{:?}", loaded.warnings);
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

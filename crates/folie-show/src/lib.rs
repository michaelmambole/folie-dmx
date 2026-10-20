//! Fichier de show Folie-DMX (`.folie`).
//!
//! Un show est un document JSON lisible et versionné. Il embarque une copie de
//! chaque fiche de fixture qu'il utilise : il s'ouvre donc à l'identique sur
//! une machine dont la bibliothèque est différente ou incomplète.
//!
//! La lecture est tolérante : une fixture qui ne peut pas être restaurée est
//! écartée avec un avertissement, le reste du show est chargé.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use folie_fixture::{FixtureId, FixtureType, Library, Patch, PatchedFixture, ofl};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

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

/// Contenu d'un show. S'enrichira des groupes, presets et cues.
#[derive(Debug, Clone, Default)]
pub struct Show {
    pub patch: Patch,
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
    let file = ShowFile {
        format: FORMAT.into(),
        version: VERSION,
        saved_with: concat!("Folie-DMX ", env!("CARGO_PKG_VERSION")).into(),
        fixture_types,
        patch,
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
    Ok(Loaded { show: Show { patch }, warnings })
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
        Show { patch }
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
        assert_eq!(loaded.warnings.len(), 2, "{:?}", loaded.warnings);
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

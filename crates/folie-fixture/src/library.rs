use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::FixtureType;
use crate::ofl;

/// Ensemble des types de fixtures disponibles, triés par constructeur puis par nom.
#[derive(Debug, Default, Clone)]
pub struct Library {
    pub fixtures: Vec<Arc<FixtureType>>,
    /// Fiches venues de la bibliothèque de l'utilisateur (importées ou modifiées).
    user: BTreeSet<String>,
}

/// Fichier qui n'a pas pu être importé, avec la raison.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

/// Noms des constructeurs, au format OFL : `{"martin": {"name": "Martin"}, …}`.
const MANUFACTURERS_FILE: &str = "manufacturers.json";

impl Library {
    /// Charge `<dossier>/<constructeur>/<modèle>.json`. Un fichier invalide
    /// n'empêche pas de charger les autres : il est signalé dans la liste d'erreurs.
    pub fn load_dir(root: &Path) -> (Self, Vec<LoadError>) {
        Self::load(root, None)
    }

    /// Bibliothèque livrée, puis celle de l'utilisateur : une fiche de l'utilisateur
    /// remplace la fiche livrée de même clé (c'est ainsi qu'on corrige une fiche).
    pub fn load(builtin: &Path, user: Option<&Path>) -> (Self, Vec<LoadError>) {
        let mut errors = Vec::new();
        let mut by_key = BTreeMap::new();
        load_into(builtin, &mut by_key, &mut errors);
        let mut user_keys = BTreeSet::new();
        if let Some(user) = user.filter(|dir| dir.is_dir()) {
            let mut mine = BTreeMap::new();
            load_into(user, &mut mine, &mut errors);
            user_keys = mine.keys().cloned().collect();
            by_key.extend(mine);
        }
        let mut fixtures: Vec<_> = by_key.into_values().collect();
        fixtures.sort_by(|a, b| (&a.manufacturer, &a.name).cmp(&(&b.manufacturer, &b.name)));
        (Self { fixtures, user: user_keys }, errors)
    }

    /// La fiche vient-elle de la bibliothèque de l'utilisateur ?
    pub fn is_user(&self, key: &str) -> bool {
        self.user.contains(key)
    }

    /// Constructeurs connus (noms affichés), pour proposer un choix à l'import.
    pub fn manufacturers(&self) -> Vec<String> {
        let names: BTreeSet<&str> = self.fixtures.iter().map(|f| f.manufacturer.as_str()).collect();
        names.into_iter().map(str::to_string).collect()
    }

    pub fn get(&self, key: &str) -> Option<&Arc<FixtureType>> {
        self.fixtures.iter().find(|f| f.key == key)
    }
}

fn load_into(root: &Path, by_key: &mut BTreeMap<String, Arc<FixtureType>>, errors: &mut Vec<LoadError>) {
    let mut fail = |path: &Path, message: String| errors.push(LoadError { path: path.to_path_buf(), message });
    let manufacturers = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(err) => {
            fail(root, err.to_string());
            return;
        }
    };
    let names = manufacturer_names(root);
    for dir in manufacturers.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
        let manufacturer_key = file_stem(&dir);
        let manufacturer =
            names.get(&manufacturer_key).cloned().unwrap_or_else(|| manufacturer_name(&manufacturer_key));
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        for path in files.flatten().map(|e| e.path()) {
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let key = format!("{manufacturer_key}/{}", file_stem(&path));
            let result = std::fs::read_to_string(&path)
                .map_err(|e| e.to_string())
                .and_then(|json| ofl::parse(&json, &key, &manufacturer).map_err(|e| e.to_string()));
            match result {
                Ok(fixture) => {
                    by_key.insert(key, Arc::new(fixture));
                }
                Err(message) => fail(&path, message),
            }
        }
    }
}

/// `manufacturers.json` du dossier, s'il existe : clé -> nom affiché.
fn manufacturer_names(root: &Path) -> BTreeMap<String, String> {
    let Ok(text) = std::fs::read_to_string(root.join(MANUFACTURERS_FILE)) else { return BTreeMap::new() };
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&text) else { return BTreeMap::new() };
    map.into_iter()
        .filter_map(|(key, value)| Some((key, value.get("name")?.as_str()?.to_string())))
        .collect()
}

/// Clé de fichier à partir d'un nom : "Robin LEDWash 600" -> "robin-ledwash-600".
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        let plain = match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => "a",
            'é' | 'è' | 'ê' | 'ë' => "e",
            'î' | 'ï' | 'í' | 'ì' => "i",
            'ô' | 'ö' | 'ó' | 'ò' | 'õ' | 'ø' => "o",
            'ù' | 'û' | 'ü' | 'ú' => "u",
            'ç' => "c",
            'ñ' => "n",
            'ÿ' => "y",
            'æ' => "ae",
            'œ' => "oe",
            'ß' => "ss",
            _ => "",
        };
        if !plain.is_empty() {
            out.push_str(plain);
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Range une fiche OFL dans la bibliothèque de l'utilisateur, après l'avoir validée :
/// `<dossier>/<constructeur>/<modèle>.json`, et le nom du constructeur dans
/// `manufacturers.json`. Rend la clé de la fiche. `replace` : écraser une fiche existante.
pub fn install(user_root: &Path, manufacturer: &str, model: &str, json: &str, replace: bool) -> Result<String, String> {
    let manufacturer = manufacturer.trim();
    let (manufacturer_key, model_key) = (slug(manufacturer), slug(model));
    if manufacturer_key.is_empty() || model_key.is_empty() {
        return Err("constructeur et modèle doivent contenir au moins une lettre ou un chiffre".into());
    }
    let key = format!("{manufacturer_key}/{model_key}");
    ofl::parse(json, &key, manufacturer).map_err(|e| e.to_string())?;
    let dir = user_root.join(&manufacturer_key);
    let path = dir.join(format!("{model_key}.json"));
    if path.exists() && !replace {
        return Err(format!("la fiche {key} existe déjà dans votre bibliothèque"));
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} : {e}", dir.display()))?;
    write_atomic(&path, json)?;

    let mut names = std::fs::read_to_string(user_root.join(MANUFACTURERS_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&t).ok())
        .unwrap_or_default();
    names.insert(manufacturer_key, serde_json::json!({ "name": manufacturer }));
    let text = serde_json::to_string_pretty(&names).map_err(|e| e.to_string())?;
    write_atomic(&user_root.join(MANUFACTURERS_FILE), &text)?;
    Ok(key)
}

/// Écrit à côté puis renomme : un fichier de bibliothèque n'est jamais à moitié écrit.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).and_then(|_| std::fs::rename(&tmp, path)).map_err(|e| format!("{} : {e}", path.display()))
}

fn file_stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// "martin" -> "Martin", "chauvet-dj" -> "Chauvet Dj".
fn manufacturer_name(key: &str) -> String {
    key.split('-')
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map(|c| c.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_project_library() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let (library, errors) = Library::load_dir(root);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(library.get("martin/rush-mh-6-wash").is_some());
        assert_eq!(library.get("martin/rush-mh-2-wash").unwrap().manufacturer, "Martin");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("folie-library-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn user_fixtures_are_installed_and_override_builtin_ones() {
        let builtin = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let user = scratch("override");
        let json = std::fs::read_to_string(builtin.join("starway/suprakolor-hd.json")).unwrap();

        // Une fiche importée sous un nouveau constructeur, nom affiché retenu.
        let key = install(&user, "Ma Boîte Lumière", "SupraKolor perso", &json, false).unwrap();
        assert_eq!(key, "ma-boite-lumiere/suprakolor-perso");
        let (library, errors) = Library::load(builtin, Some(&user));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(library.get(&key).unwrap().manufacturer, "Ma Boîte Lumière");
        assert!(library.is_user(&key) && !library.is_user("starway/suprakolor-hd"));
        assert!(library.manufacturers().contains(&"Ma Boîte Lumière".to_string()));

        // Même clé qu'une fiche livrée : la fiche de l'utilisateur la remplace.
        let edited = json.replacen("\"name\": \"SupraKolor HD\"", "\"name\": \"SupraKolor corrigé\"", 1);
        assert_ne!(edited, json, "le nom est bien dans la fiche");
        assert!(install(&user, "Starway", "suprakolor-hd", &edited, false).is_ok());
        let (library, _) = Library::load(builtin, Some(&user));
        assert_eq!(library.get("starway/suprakolor-hd").unwrap().name, "SupraKolor corrigé");
        assert!(library.is_user("starway/suprakolor-hd"));
        assert_eq!(library.fixtures.iter().filter(|f| f.key == "starway/suprakolor-hd").count(), 1);

        // Déjà là sans « remplacer », ou fiche invalide : refusé, rien d'écrit.
        assert!(install(&user, "Starway", "suprakolor-hd", &json, false).unwrap_err().contains("existe déjà"));
        assert!(install(&user, "Starway", "cassée", "{\"name\": 3}", false).is_err());
        assert!(!user.join("starway/cassee.json").exists());
        let _ = std::fs::remove_dir_all(&user);
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Robin LEDWash 600"), "robin-ledwash-600");
        assert_eq!(slug("  Chauvet DJ  "), "chauvet-dj");
        assert_eq!(slug("Projecteur Écran Œil"), "projecteur-ecran-oeil");
        assert_eq!(slug("--"), "");
    }

    /// Vérifié sur l'appareil le 2026-10-26, en valeurs DMX : le manuel annonce « ouvert »
    /// jusqu'à 10, mais le SupraKolor clignote dès 10.
    #[test]
    fn suprakolor_strobe_starts_at_10() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let (library, _) = Library::load_dir(root);
        let tour = &library.get("starway/suprakolor-hd").unwrap().modes[0];
        let strobe = tour.parameters.iter().find(|p| p.name == "Strobe").unwrap();
        let dmx = |v: u16| v >> 8;
        assert_eq!(dmx(*strobe.capabilities[0].range.end()), 9, "ouvert de 0 à 9");
        assert_eq!(dmx(*strobe.capabilities[1].range.start()), 10, "strobe à partir de 10");
        // Blanc par défaut, comme la MH 6 : un simple « At 50 » allume.
        let white = tour.parameters.iter().find(|p| p.name == "White").unwrap();
        assert_eq!(white.default, u16::MAX);
    }

    /// Vérifié sur l'appareil le 2026-10-26 : 250 donne un rouge fixe, la roue virtuelle
    /// défile en avant de 251 à 253 (la fiche OFL plaçait un rouge en double à 251) ; elle
    /// n'agit qu'avec le Blanc à 0.
    #[test]
    fn servocolor_wheel_rotates_from_251_and_needs_white_off() {
        use crate::CapabilityKind;
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let (library, _) = Library::load_dir(root);
        for mode in &library.get("starway/servo-color-4k").unwrap().modes {
            let wheel = mode.parameters.iter().find(|p| p.name == "Virtual Color Wheel").unwrap();
            let at = |dmx: u16| &wheel.capability_at(dmx * 257).unwrap().kind;
            assert!(matches!(at(250), CapabilityKind::ColorPreset { .. }), "250 : rouge fixe");
            assert!(!matches!(at(251), CapabilityKind::ColorPreset { .. }), "251 : défilement");
            // La roue n'agit qu'avec le Blanc à 0, et ses blancs avec RGB à fond.
            let default = |name: &str| mode.parameters.iter().find(|p| p.name == name).unwrap().default;
            assert_eq!((default("White"), default("Red")), (0, u16::MAX));
        }
    }
}

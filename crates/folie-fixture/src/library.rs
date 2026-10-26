use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::FixtureType;
use crate::ofl;

/// Ensemble des types de fixtures disponibles, triés par constructeur puis par nom.
#[derive(Debug, Default, Clone)]
pub struct Library {
    pub fixtures: Vec<Arc<FixtureType>>,
}

/// Fichier qui n'a pas pu être importé, avec la raison.
#[derive(Debug, Clone)]
pub struct LoadError {
    pub path: PathBuf,
    pub message: String,
}

impl Library {
    /// Charge `<dossier>/<constructeur>/<modèle>.json`. Un fichier invalide
    /// n'empêche pas de charger les autres : il est signalé dans la liste d'erreurs.
    pub fn load_dir(root: &Path) -> (Self, Vec<LoadError>) {
        let mut fixtures = Vec::new();
        let mut errors = Vec::new();
        let mut fail = |path: &Path, message: String| {
            errors.push(LoadError { path: path.to_path_buf(), message })
        };

        let manufacturers = match std::fs::read_dir(root) {
            Ok(entries) => entries,
            Err(err) => {
                fail(root, err.to_string());
                return (Self::default(), errors);
            }
        };
        for dir in manufacturers.flatten().map(|e| e.path()).filter(|p| p.is_dir()) {
            let manufacturer_key = file_stem(&dir);
            let Ok(files) = std::fs::read_dir(&dir) else { continue };
            for path in files.flatten().map(|e| e.path()) {
                if path.extension().is_none_or(|ext| ext != "json") {
                    continue;
                }
                let key = format!("{manufacturer_key}/{}", file_stem(&path));
                let result = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|json| {
                        ofl::parse(&json, &key, &manufacturer_name(&manufacturer_key))
                            .map_err(|e| e.to_string())
                    });
                match result {
                    Ok(fixture) => fixtures.push(Arc::new(fixture)),
                    Err(message) => fail(&path, message),
                }
            }
        }
        fixtures.sort_by(|a, b| (&a.manufacturer, &a.name).cmp(&(&b.manufacturer, &b.name)));
        (Self { fixtures }, errors)
    }

    pub fn get(&self, key: &str) -> Option<&Arc<FixtureType>> {
        self.fixtures.iter().find(|f| f.key == key)
    }
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

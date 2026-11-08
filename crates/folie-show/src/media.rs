//! Sons du show : un pool de fichiers avec leurs réglages de lecture. Les fichiers sont
//! rangés à côté du show (`concert.folie` -> dossier `concert-sons`), si bien que le
//! show et ses sons se déplacent ensemble.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Un son du pool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sound {
    pub name: String,
    /// Fichier : chemin complet en mémoire, relatif au dossier du show dans le fichier.
    pub file: PathBuf,
    /// Point d'entrée, en secondes depuis le début du fichier.
    pub start: f64,
    /// Point de sortie ; `None` : la fin du fichier.
    pub end: Option<f64>,
    /// Volume, en dB.
    pub gain_db: f32,
    pub fade_in: f32,
    pub fade_out: f32,
    pub looped: bool,
    /// Groupe d'exclusion, comme sur un sampleur : un son qui part coupe ceux de son
    /// groupe. 0 : aucun.
    pub group: u32,
}

impl Default for Sound {
    fn default() -> Self {
        Self {
            name: String::new(),
            file: PathBuf::new(),
            start: 0.0,
            end: None,
            gain_db: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            looped: false,
            group: 0,
        }
    }
}

impl Sound {
    pub fn from_file(file: &Path) -> Self {
        let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        Self { name, file: file.to_path_buf(), ..Default::default() }
    }
}

/// Dossier des sons d'un show : `concert.folie` -> `concert-sons`, à côté.
pub fn media_dir(show_path: &Path) -> PathBuf {
    let stem = show_path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "show".into());
    show_path.with_file_name(format!("{stem}-sons"))
}

/// Range dans le dossier du show les sons qui n'y sont pas encore (copie), et met à jour
/// leur chemin. Rend les problèmes rencontrés (fichier introuvable…), sans s'arrêter.
pub fn gather(sounds: &mut std::collections::BTreeMap<u32, Sound>, show_path: &Path) -> Vec<String> {
    let dir = media_dir(show_path);
    let mut problems = Vec::new();
    for (number, sound) in sounds.iter_mut() {
        if sound.file.as_os_str().is_empty() || sound.file.starts_with(&dir) {
            continue;
        }
        if !sound.file.is_file() {
            problems.push(format!("son {number} : fichier introuvable ({})", sound.file.display()));
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&dir) {
            problems.push(format!("{} : {e}", dir.display()));
            break;
        }
        let file_name = sound.file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| format!("son-{number}"));
        // Même nom, contenu différent : on numérote la copie.
        let mut target = dir.join(&file_name);
        let mut n = 2;
        while target.exists() && std::fs::read(&target).ok() != std::fs::read(&sound.file).ok() {
            let path = Path::new(&file_name);
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            target = dir.join(format!("{stem} {n}{ext}"));
            n += 1;
        }
        if !target.exists()
            && let Err(e) = std::fs::copy(&sound.file, &target)
        {
            problems.push(format!("son {number} : copie impossible ({e})"));
            continue;
        }
        sound.file = target;
    }
    problems
}

/// Chemin écrit dans le fichier : relatif au dossier du show quand il y est.
pub(crate) fn relative(file: &Path, show_dir: Option<&Path>) -> PathBuf {
    match show_dir.and_then(|dir| file.strip_prefix(dir).ok()) {
        Some(rel) => rel.to_path_buf(),
        None => file.to_path_buf(),
    }
}

/// Chemin en mémoire : complet, à partir du dossier du show.
pub(crate) fn absolute(file: &Path, show_dir: Option<&Path>) -> PathBuf {
    match show_dir {
        Some(dir) if file.is_relative() && !file.as_os_str().is_empty() => dir.join(file),
        _ => file.to_path_buf(),
    }
}

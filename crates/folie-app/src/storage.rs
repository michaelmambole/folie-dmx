//! Dossier de données de l'application : réglages et sauvegarde automatique.

use std::path::{Path, PathBuf};

use folie_show::Show;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;

use crate::workspace::{Layout, Workspace};

const SETTINGS_FILE: &str = "settings.json";
const AUTOSAVE_FILE: &str = "autosave.folie";

#[derive(Default, Serialize, Deserialize)]
struct Settings {
    /// Show ouvert à la dernière fermeture, rouvert au démarrage.
    last_show: Option<PathBuf>,
    /// Show dont la sauvegarde automatique contient les modifications non enregistrées.
    autosave_of: Option<PathBuf>,
    /// Fenêtres telles qu'elles étaient à la dernière fermeture.
    #[serde(default)]
    workspace: Option<Workspace>,
    /// Dispositions enregistrées, par case (1 à 9).
    #[serde(default)]
    layouts: BTreeMap<u8, Layout>,
    /// Sortie Art-Net, en plus de la sortie choisie.
    #[serde(default)]
    artnet: ArtNetSettings,
    /// Télécommande web.
    #[serde(default)]
    remote: RemoteSettings,
    /// Pilotage OSC.
    #[serde(default)]
    osc: OscSettings,
}

/// Réglages de la sortie Art-Net.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtNetSettings {
    pub enabled: bool,
    /// Adresse d'un node ou de diffusion.
    pub destination: String,
    /// Adresse de port Art-Net (net, sous-réseau, univers sur 15 bits).
    pub universe: u16,
}

impl Default for ArtNetSettings {
    fn default() -> Self {
        Self { enabled: false, destination: folie_dmx::artnet::BROADCAST.to_string(), universe: 0 }
    }
}

/// Réglages de la télécommande web. Coupée tant qu'on ne l'active pas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteSettings {
    pub enabled: bool,
    pub port: u16,
    /// Code « régie » (tout permis), tiré au hasard à la première activation.
    pub pin: String,
    /// Code « lecture » : GO, Back et touches des executors seulement.
    pub playback_pin: String,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self { enabled: false, port: 8090, pin: String::new(), playback_pin: String::new() }
    }
}

/// Réglages de l'OSC. Coupé tant qu'on ne l'active pas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OscSettings {
    pub enabled: bool,
    /// Port d'écoute (TouchOSC envoie vers 8000 par défaut).
    pub port: u16,
    /// Port où les clients attendent le retour d'état (9000 chez TouchOSC) ; 0 : celui d'où ils écrivent.
    pub reply_port: u16,
    /// Adresses autorisées, séparées par des virgules ou des espaces ; vide : tout le réseau local.
    pub allowed: String,
    /// Les marqueurs de Reaper font Goto Cue (marqueur 3 : cue 3).
    pub reaper_markers: bool,
}

impl Default for OscSettings {
    fn default() -> Self {
        Self { enabled: false, port: 8000, reply_port: 9000, allowed: String::new(), reaper_markers: false }
    }
}

impl OscSettings {
    /// Adresses autorisées, ou la première qui est illisible.
    pub fn allowed_ips(&self) -> Result<Vec<std::net::IpAddr>, String> {
        self.allowed
            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().map_err(|_| s.to_string()))
            .collect()
    }
}

/// Sauvegarde automatique trouvée au démarrage.
pub struct Recovery {
    pub path: PathBuf,
    /// Show d'origine, `None` s'il n'avait jamais été enregistré.
    pub original: Option<PathBuf>,
}

pub struct Storage {
    /// `None` : rien n'est lu ni écrit (tests, ou dossier de données introuvable).
    dir: Option<PathBuf>,
}

impl Storage {
    /// `~/Library/Application Support/Folie-DMX` sur macOS.
    pub fn for_user() -> Self {
        Self { dir: dirs::data_dir().map(|d| d.join("Folie-DMX")) }
    }

    #[cfg(test)]
    pub fn disabled() -> Self {
        Self { dir: None }
    }

    #[cfg(test)]
    pub fn at(dir: &Path) -> Self {
        Self { dir: Some(dir.to_path_buf()) }
    }

    /// Dossier des réglages, s'il y en a un.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(name))
    }

    fn settings(&self) -> Settings {
        self.file(SETTINGS_FILE)
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    fn update_settings(&self, change: impl FnOnce(&mut Settings)) {
        let (Some(dir), Some(path)) = (&self.dir, self.file(SETTINGS_FILE)) else { return };
        let mut settings = self.settings();
        change(&mut settings);
        let result = std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(&path, serde_json::to_string_pretty(&settings).unwrap_or_default()));
        if let Err(err) = result {
            log::warn!("réglages non enregistrés ({}) : {err}", path.display());
        }
    }

    pub fn last_show(&self) -> Option<PathBuf> {
        self.settings().last_show.filter(|p| p.exists())
    }

    pub fn set_last_show(&self, path: Option<&Path>) {
        self.update_settings(|s| s.last_show = path.map(Path::to_path_buf));
    }

    pub fn workspace(&self) -> Option<Workspace> {
        self.settings().workspace
    }

    pub fn set_workspace(&self, workspace: &Workspace) {
        self.update_settings(|s| s.workspace = Some(workspace.clone()));
    }

    pub fn artnet(&self) -> ArtNetSettings {
        self.settings().artnet
    }

    pub fn set_artnet(&self, artnet: &ArtNetSettings) {
        self.update_settings(|s| s.artnet = artnet.clone());
    }

    pub fn remote(&self) -> RemoteSettings {
        self.settings().remote
    }

    pub fn set_remote(&self, remote: &RemoteSettings) {
        self.update_settings(|s| s.remote = remote.clone());
    }

    pub fn osc(&self) -> OscSettings {
        self.settings().osc
    }

    pub fn set_osc(&self, osc: &OscSettings) {
        self.update_settings(|s| s.osc = osc.clone());
    }

    pub fn layouts(&self) -> BTreeMap<u8, Layout> {
        self.settings().layouts
    }

    pub fn set_layouts(&self, layouts: &BTreeMap<u8, Layout>) {
        self.update_settings(|s| s.layouts = layouts.clone());
    }

    pub fn write_autosave(&self, show: &Show, original: Option<&Path>) -> Result<(), String> {
        let (Some(dir), Some(path)) = (&self.dir, self.file(AUTOSAVE_FILE)) else { return Ok(()) };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        folie_show::save(show, &path).map_err(|e| e.to_string())?;
        self.update_settings(|s| s.autosave_of = original.map(Path::to_path_buf));
        Ok(())
    }

    pub fn recovery(&self) -> Option<Recovery> {
        let path = self.file(AUTOSAVE_FILE).filter(|p| p.exists())?;
        Some(Recovery { path, original: self.settings().autosave_of })
    }

    /// À appeler dès que plus rien n'est à récupérer (show enregistré, ou modifications abandonnées).
    pub fn clear_autosave(&self) {
        if let Some(path) = self.file(AUTOSAVE_FILE)
            && path.exists()
            && let Err(err) = std::fs::remove_file(&path)
        {
            log::warn!("sauvegarde automatique non supprimée : {err}");
        }
        self.update_settings(|s| s.autosave_of = None);
    }
}

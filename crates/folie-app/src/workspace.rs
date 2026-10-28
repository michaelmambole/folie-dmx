//! Espace de travail : quelles fenêtres sont ouvertes, où, et avec quels réglages.
//! Enregistré avec les réglages de l'utilisateur, pas dans le show.

use eframe::egui::{self, Key, Pos2, Vec2};
use folie_fixture::FeatureGroup;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum WindowKind {
    Patch,
    Fixtures,
    Groups,
    Presets,
    Output,
    Cues,
    Executors,
    Effects,
    Tracking,
    Stage,
    Macros,
    Sound,
    FixtureEditor,
}

impl WindowKind {
    pub const ALL: [WindowKind; 13] = [
        Self::Patch,
        Self::Fixtures,
        Self::Groups,
        Self::Presets,
        Self::Output,
        Self::Cues,
        Self::Executors,
        Self::Effects,
        Self::Tracking,
        Self::Stage,
        Self::Macros,
        Self::Sound,
        Self::FixtureEditor,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Patch => "Patch",
            Self::Fixtures => "Fixtures",
            Self::Groups => "Groupes",
            Self::Presets => "Presets",
            Self::Output => "Sortie DMX",
            Self::Cues => "Cues",
            Self::Executors => "Executors",
            Self::Effects => "Effets",
            Self::Tracking => "Tracking",
            Self::Stage => "Scène",
            Self::Macros => "Macros",
            Self::Sound => "Son",
            Self::FixtureEditor => "Éditeur de fiches",
        }
    }

    /// ⌘1, ⌘2… dans l'ordre du menu Fenêtres ; les chiffres épuisés, Fixtures prend ⌘F
    /// ; Son et Éditeur de fiches n'en ont pas.
    pub fn key(self) -> Option<Key> {
        Some(match self {
            Self::Patch => Key::Num1,
            Self::Fixtures => Key::F,
            Self::Groups => Key::Num2,
            Self::Presets => Key::Num3,
            Self::Output => Key::Num4,
            Self::Cues => Key::Num5,
            Self::Executors => Key::Num6,
            Self::Effects => Key::Num7,
            Self::Tracking => Key::Num8,
            Self::Stage => Key::Num9,
            Self::Macros => Key::Num0,
            Self::Sound | Self::FixtureEditor => return None,
        })
    }

    pub fn default_size(self) -> Vec2 {
        match self {
            Self::Patch => egui::vec2(640.0, 640.0),
            Self::Fixtures => egui::vec2(780.0, 560.0),
            Self::Groups | Self::Presets => egui::vec2(340.0, 380.0),
            Self::Output => egui::vec2(1100.0, 720.0),
            Self::Cues => egui::vec2(560.0, 420.0),
            Self::Executors => egui::vec2(1000.0, 360.0),
            Self::Effects => egui::vec2(560.0, 520.0),
            Self::Tracking => egui::vec2(760.0, 560.0),
            Self::Stage => egui::vec2(900.0, 560.0),
            Self::Macros => egui::vec2(560.0, 460.0),
            Self::Sound => egui::vec2(520.0, 560.0),
            Self::FixtureEditor => egui::vec2(1040.0, 700.0),
        }
    }
}

#[cfg(test)]
#[test]
fn window_shortcuts_are_all_different() {
    let keys: Vec<Key> = WindowKind::ALL.iter().filter_map(|k| k.key()).collect();
    let unique: std::collections::BTreeSet<String> = keys.iter().map(|k| format!("{k:?}")).collect();
    assert_eq!(unique.len(), keys.len(), "deux fenêtres sur le même raccourci : {keys:?}");
    // ⌥⌘1 à ⌥⌘9 sont aux dispositions, ⌘S, ⌘O, ⌘N au fichier.
    assert!(!keys.iter().any(|k| matches!(k, Key::S | Key::O | Key::N)));
    assert_eq!(WindowKind::Fixtures.key(), Some(Key::F));
}

/// Position et taille d'une fenêtre, en points, dans l'espace des écrans.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Placement {
    pub position: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
    #[serde(default)]
    pub fullscreen: bool,
}

impl Placement {
    pub fn at_size(position: Pos2, size: Vec2) -> Self {
        Self { position: Some([position.x, position.y]), size: Some([size.x, size.y]), fullscreen: false }
    }

    pub fn position(&self) -> Option<Pos2> {
        self.position.map(|[x, y]| egui::pos2(x, y))
    }

    pub fn size(&self) -> Option<Vec2> {
        self.size.map(|[w, h]| egui::vec2(w, h))
    }

    /// Relève la position réelle de la fenêtre en cours d'affichage.
    pub fn capture(ctx: &egui::Context) -> Option<Self> {
        ctx.input(|i| {
            let info = i.viewport();
            let outer = info.outer_rect?;
            let inner = info.inner_rect?;
            Some(Self {
                position: Some([outer.min.x, outer.min.y]),
                size: Some([inner.width(), inner.height()]),
                fullscreen: info.fullscreen.unwrap_or(false),
            })
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    pub kind: WindowKind,
    #[serde(default)]
    pub placement: Placement,
    /// Fenêtre Presets : famille affichée (clé stable, ex. "color").
    #[serde(default)]
    pub preset_family: Option<String>,
}

impl WindowState {
    pub fn new(kind: WindowKind) -> Self {
        Self { kind, placement: Placement::default(), preset_family: None }
    }

    pub fn family(&self) -> FeatureGroup {
        self.preset_family.as_deref().and_then(FeatureGroup::from_key).unwrap_or(FeatureGroup::Color)
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Workspace {
    /// Fenêtre principale (Programmer).
    #[serde(default)]
    pub main: Placement,
    /// Fenêtres ouvertes, dans l'ordre d'ouverture.
    #[serde(default)]
    pub windows: Vec<WindowState>,
}

impl Workspace {
    /// Premier lancement : le patch, les fixtures et les pools sont ouverts, l'OS les place.
    pub fn first_launch() -> Self {
        let windows = [WindowKind::Patch, WindowKind::Fixtures, WindowKind::Groups, WindowKind::Presets]
            .into_iter()
            .map(WindowState::new)
            .collect();
        Self { main: Placement::default(), windows }
    }

    pub fn is_open(&self, kind: WindowKind) -> bool {
        self.windows.iter().any(|w| w.kind == kind)
    }

    pub fn open(&mut self, kind: WindowKind) {
        if !self.is_open(kind) {
            self.windows.push(WindowState::new(kind));
        }
    }

    pub fn close(&mut self, kind: WindowKind) {
        self.windows.retain(|w| w.kind != kind);
    }

    pub fn window_mut(&mut self, kind: WindowKind) -> Option<&mut WindowState> {
        self.windows.iter_mut().find(|w| w.kind == kind)
    }
}

/// Disposition enregistrée, rappelée par ⌥⌘1 à ⌥⌘9 (les « Window Configurations » de Pro Tools).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    pub workspace: Workspace,
}

/// Cases de dispositions : 1 à 9.
pub const LAYOUT_SLOTS: std::ops::RangeInclusive<u8> = 1..=9;

pub fn layout_key(slot: u8) -> Key {
    [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9]
        [(slot.clamp(1, 9) - 1) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_and_round_trip() {
        let mut workspace = Workspace::first_launch();
        assert!(workspace.is_open(WindowKind::Patch));
        workspace.open(WindowKind::Patch);
        assert_eq!(workspace.windows.len(), 4, "pas de doublon");
        workspace.open(WindowKind::Output);
        workspace.close(WindowKind::Groups);
        workspace.window_mut(WindowKind::Presets).unwrap().preset_family = Some("position".into());

        let json = serde_json::to_string(&workspace).unwrap();
        let back: Workspace = serde_json::from_str(&json).unwrap();
        assert_eq!(back, workspace);
        assert_eq!(back.windows[2].family(), FeatureGroup::Position);
        // Un fichier ancien ou incomplet reste lisible.
        assert_eq!(serde_json::from_str::<Workspace>("{}").unwrap(), Workspace::default());
    }
}

//! Écrans branchés : une fenêtre enregistrée sur un écran absent (portable seul
//! en tournée, vidéoprojecteur débranché…) est ramenée sur l'écran principal.

use eframe::egui::{Rect, pos2, vec2};

use crate::workspace::Placement;

/// Barre de titre approximative : c'est elle qui doit rester attrapable.
const TITLE_BAR: f32 = 28.0;

/// Rectangles des écrans en points, l'écran principal en premier. Vide si inconnu.
pub fn displays() -> Vec<Rect> {
    let mut list = match display_info::DisplayInfo::all() {
        Ok(list) => list,
        Err(err) => {
            log::warn!("liste des écrans indisponible : {err}");
            return Vec::new();
        }
    };
    list.sort_by_key(|d| !d.is_primary);
    list.iter()
        .map(|d| Rect::from_min_size(pos2(d.x as f32, d.y as f32), vec2(d.width as f32, d.height as f32)))
        .collect()
}

/// La barre de titre de la fenêtre est-elle suffisamment sur un écran pour être attrapée ?
fn reachable(placement: &Placement, displays: &[Rect]) -> bool {
    let (Some(position), Some(size)) = (placement.position(), placement.size()) else { return true };
    let title = Rect::from_min_size(position, vec2(size.x, TITLE_BAR));
    displays.iter().any(|d| {
        let visible = d.intersect(title);
        visible.width() >= 80.0 && visible.height() >= TITLE_BAR / 2.0
    })
}

/// Ramène la fenêtre sur l'écran principal si elle n'est sur aucun écran.
/// `index` décale les fenêtres ramenées pour qu'elles ne s'empilent pas exactement.
pub fn rescue(placement: Placement, displays: &[Rect], index: usize) -> Placement {
    let Some(primary) = displays.first() else { return placement };
    if reachable(&placement, displays) {
        return placement;
    }
    let offset = 40.0 + 30.0 * index as f32;
    let size = placement.size().map(|s| s.min(primary.size() - vec2(offset, offset + 40.0)));
    Placement {
        position: Some([primary.min.x + offset, primary.min.y + offset]),
        size: size.map(|s| [s.x, s.y]),
        fullscreen: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(x: f32, y: f32) -> Placement {
        Placement::at_size(pos2(x, y), vec2(400.0, 300.0))
    }

    #[test]
    fn windows_on_a_missing_screen_come_back() {
        let laptop = Rect::from_min_size(pos2(0.0, 0.0), vec2(1440.0, 900.0));
        let external = Rect::from_min_size(pos2(1440.0, 0.0), vec2(1920.0, 1080.0));

        let on_external = placement(2000.0, 100.0);
        assert_eq!(rescue(on_external, &[laptop, external], 0), on_external, "écran présent : inchangée");

        let rescued = rescue(on_external, &[laptop], 0);
        assert_eq!(rescued.position, Some([40.0, 40.0]));
        assert_eq!(rescued.size, Some([400.0, 300.0]));

        // À cheval sur le bord mais attrapable : on n'y touche pas.
        assert_eq!(rescue(placement(1300.0, 100.0), &[laptop], 0), placement(1300.0, 100.0));
        // Sans liste d'écrans, on ne devine rien.
        assert_eq!(rescue(on_external, &[], 0), on_external);
    }
}


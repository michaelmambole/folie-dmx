//! Fenêtre Scène : une vue simple en 3D, comme la fenêtre Stage de grandMA2. Les
//! projecteurs sont des blocs, chaque faisceau un trait de sa couleur réelle jusqu'au sol.
//! Tout part de ce qui sort vraiment : effets et executors compris.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use folie_control::{Console, SelectMode};
use folie_engine::Snapshot;
use folie_fixture::{Attribute, Parameter, PatchedFixture, Target};

use crate::theme;

/// Un point de la scène, en mètres : x vers jardin–cour, y vers le haut, z vers le public.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Vec3 {
    x: f32,
    y: f32,
    z: f32,
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3 { x, y, z }
}

impl std::ops::Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl std::ops::Mul<f32> for Vec3 {
    type Output = Vec3;
    fn mul(self, k: f32) -> Vec3 {
        v3(self.x * k, self.y * k, self.z * k)
    }
}

/// Largeur et profondeur du plateau, hauteur de la perche.
const STAGE_WIDTH: f32 = 12.0;
const STAGE_DEPTH: f32 = 8.0;
const TRUSS_HEIGHT: f32 = 5.0;
const TRUSS_Z: f32 = -1.0;
/// Longueur d'un faisceau qui ne touche pas le sol (dirigé vers le haut).
const SKY_BEAM: f32 = 8.0;

/// Point de vue : rotation autour de la scène, inclinaison, distance.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Camera {
    yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Camera {
    const FACE: Camera = Camera { yaw: 0.0, pitch: 0.08, distance: 17.0 };
    const TOP: Camera = Camera { yaw: 0.0, pitch: 1.5, distance: 17.0 };
    const THREE_QUARTER: Camera = Camera { yaw: -0.6, pitch: 0.45, distance: 17.0 };

    /// Position à l'écran (et profondeur) d'un point de la scène.
    fn project(&self, p: Vec3, rect: Rect) -> Option<(Pos2, f32)> {
        let (cy, sy, cp, sp) = (self.yaw.cos(), self.yaw.sin(), self.pitch.cos(), self.pitch.sin());
        // Le centre visé : le milieu du plateau, à mi-hauteur de la perche.
        let (x, y, z) = (p.x, p.y - TRUSS_HEIGHT / 2.5, p.z);
        let (x, z) = (x * cy - z * sy, x * sy + z * cy);
        // La caméra est côté public et en hauteur : ce qui est haut ou vers le public est plus proche.
        let (y, z) = (y * cp - z * sp, self.distance - y * sp - z * cp);
        if z <= 0.5 {
            return None;
        }
        let focal = rect.height().min(rect.width()) * 1.25;
        Some((pos2(rect.center().x + x * focal / z, rect.center().y - y * focal / z), z))
    }
}

/// Direction du faisceau d'un projecteur suspendu : tilt 0° vers le bas, pan autour de la verticale.
fn beam_direction(pan_degrees: f32, tilt_degrees: f32) -> Vec3 {
    let (pan, tilt) = (pan_degrees.to_radians(), tilt_degrees.to_radians());
    v3(tilt.sin() * pan.sin(), -tilt.cos(), tilt.sin() * pan.cos())
}

/// Où le faisceau s'arrête : au sol, ou à `SKY_BEAM` mètres s'il ne le touche pas.
fn beam_end(origin: Vec3, direction: Vec3) -> (Vec3, bool) {
    if direction.y < -0.05 {
        (origin + direction * (origin.y / -direction.y), true)
    } else {
        (origin + direction * SKY_BEAM, false)
    }
}

/// Angle en degrés d'un paramètre de position, rapporté au milieu de sa course
/// (0° = projecteur au repos, faisceau vers le bas).
fn angle_of(parameter: &Parameter, value: u16) -> f32 {
    match parameter.capability_at(value).and_then(|c| c.angle.map(|a| (c, a))) {
        Some((cap, (start, end))) => {
            let span = (*cap.range.end() - *cap.range.start()).max(1) as f32;
            let t = (value - *cap.range.start()) as f32 / span;
            start + (end - start) * t - (start + end) / 2.0
        }
        None => 0.0,
    }
}

/// Ce que la scène montre d'un projecteur.
struct Look {
    position: Vec3,
    /// 0 à 1 : le plus haut des dimmers du projecteur.
    intensity: f32,
    color: Color32,
    direction: Vec3,
}

fn look(console: &Console, snapshot: &Snapshot, fixture: &PatchedFixture, position: Vec3) -> Look {
    let parameters = &fixture.mode().parameters;
    let live = |p: &Parameter| crate::fixtures::live_value(snapshot, fixture, p);
    let dimmers: Vec<f32> =
        parameters.iter().filter(|p| p.attribute == Attribute::Dimmer).filter_map(live).map(|v| v as f32 / 65535.0).collect();
    // Sans dimmer (projecteur fixe), il est allumé.
    let intensity = dimmers.iter().copied().fold(None, |m: Option<f32>, v| Some(m.map_or(v, |m| m.max(v)))).unwrap_or(1.0);
    let angle = |attribute: Attribute| {
        parameters.iter().find(|p| p.attribute == attribute).and_then(|p| live(p).map(|v| angle_of(p, v))).unwrap_or(0.0)
    };
    let values: BTreeMap<String, u16> = parameters.iter().filter_map(|p| live(p).map(|v| (p.name.clone(), v))).collect();
    let color = console
        .target_color(Target::whole(fixture.id), &values)
        .map_or(Color32::from_rgb(255, 236, 205), |[r, g, b]| Color32::from_rgb(r, g, b));
    Look { position, intensity, color, direction: beam_direction(angle(Attribute::Pan), angle(Attribute::Tilt)) }
}

/// Placement automatique : les projecteurs du patch alignés sur une perche, dans l'ordre.
fn auto_positions(count: usize) -> Vec<Vec3> {
    let spacing = (STAGE_WIDTH / count.max(1) as f32).min(1.6);
    let first = -spacing * (count.saturating_sub(1)) as f32 / 2.0;
    (0..count).map(|i| v3(first + spacing * i as f32, TRUSS_HEIGHT, TRUSS_Z)).collect()
}

pub fn stage_window(ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot) {
    let camera_id = egui::Id::unique("stage-camera");
    let mut camera = ui.data_mut(|d| d.get_persisted::<(f32, f32, f32)>(camera_id))
        .map_or(Camera::THREE_QUARTER, |(yaw, pitch, distance)| Camera { yaw, pitch, distance });

    ui.horizontal(|ui| {
        theme::section(ui, "Scène");
        for (label, view) in [("Face", Camera::FACE), ("Dessus", Camera::TOP), ("3/4", Camera::THREE_QUARTER)] {
            let on = (camera.yaw, camera.pitch) == (view.yaw, view.pitch);
            if ui.add(egui::Button::selectable(on, label)).on_hover_text(format!("Vue {label}")).clicked() {
                camera = Camera { distance: camera.distance, ..view };
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(egui::RichText::new("Glisser : tourner · molette : zoom · clic : sélectionner").size(11.5).color(theme::MUTED));
        });
    });

    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Canvas, true, "Vue de la scène"));
    if response.dragged() {
        let delta = response.drag_delta();
        camera.yaw += delta.x * 0.008;
        camera.pitch = (camera.pitch + delta.y * 0.006).clamp(0.02, 1.55);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        camera.distance = (camera.distance - scroll * 0.02).clamp(6.0, 40.0);
    }
    ui.data_mut(|d| d.insert_persisted(camera_id, (camera.yaw, camera.pitch, camera.distance)));

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, ui.visuals().extreme_bg_color);
    let line = |a: Vec3, b: Vec3, stroke: Stroke| {
        if let (Some((p, _)), Some((q, _))) = (camera.project(a, rect), camera.project(b, rect)) {
            painter.line_segment([p, q], stroke);
        }
    };

    // Sol quadrillé, bords du plateau, perche.
    let (hw, hd) = (STAGE_WIDTH / 2.0, STAGE_DEPTH / 2.0);
    let grid = Stroke::new(1.0, theme::SURFACE_3);
    for i in 0..=12 {
        let x = -hw + STAGE_WIDTH * i as f32 / 12.0;
        line(v3(x, 0.0, -hd), v3(x, 0.0, hd), grid);
    }
    for i in 0..=8 {
        let z = -hd + STAGE_DEPTH * i as f32 / 8.0;
        line(v3(-hw, 0.0, z), v3(hw, 0.0, z), grid);
    }
    let edge = Stroke::new(2.0, theme::DEFAULT);
    line(v3(-hw, 0.0, hd), v3(hw, 0.0, hd), edge); // nez de scène
    line(v3(-hw, 0.0, -hd), v3(hw, 0.0, -hd), edge);
    line(v3(-hw - 0.5, TRUSS_HEIGHT + 0.35, TRUSS_Z), v3(hw + 0.5, TRUSS_HEIGHT + 0.35, TRUSS_Z), Stroke::new(3.0, theme::MUTED));

    let fixtures: Vec<PatchedFixture> = console.show.patch.fixtures().iter().filter(|f| f.universe == 0).cloned().collect();
    let positions = auto_positions(fixtures.len());
    let mut clicked = None;
    for (fixture, position) in fixtures.iter().zip(positions) {
        let look = look(console, snapshot, fixture, position);
        let selected = console.selection().iter().any(|t| t.fixture == fixture.id);

        // Faisceau : un trait de la couleur du projecteur, plus pâle quand le dimmer baisse.
        // Une couleur noire (une LED dont toutes les couleurs sont à 0) n'éclaire pas.
        let [r, g, b, _] = look.color.to_array();
        let light = look.intensity * r.max(g).max(b) as f32 / 255.0;
        if light > 0.02 {
            let (end, on_floor) = beam_end(look.position, look.direction);
            let alpha = 0.25 + 0.7 * look.intensity;
            line(look.position, end, Stroke::new(3.0, look.color.gamma_multiply(alpha)));
            if on_floor && let Some((spot, depth)) = camera.project(end, rect) {
                let radius = (rect.height() * 0.6 / depth).max(3.0);
                painter.circle_filled(spot, radius, look.color.gamma_multiply(0.35 * look.intensity));
            }
        }

        // Bloc du projecteur : un cube en fil de fer, rempli de sa couleur quand il éclaire.
        let s = 0.3;
        let mut corners = Vec::with_capacity(8);
        for dx in [-s, s] {
            for dy in [-s, s] {
                for dz in [-s, s] {
                    corners.push(position + v3(dx, dy, dz));
                }
            }
        }
        let stroke = Stroke::new(if selected { 2.0 } else { 1.5 }, if selected { theme::SELECTION } else { theme::TEXT });
        for (a, b) in [(0, 1), (0, 2), (0, 4), (1, 3), (1, 5), (2, 3), (2, 6), (3, 7), (4, 5), (4, 6), (5, 7), (6, 7)] {
            line(corners[a], corners[b], stroke);
        }
        let Some((center, depth)) = camera.project(position, rect) else { continue };
        let size = rect.height() * 0.9 / depth;
        painter.circle_filled(center, size * 0.5, look.color.gamma_multiply(0.15 + 0.6 * look.intensity));
        if let Some((label, _)) = camera.project(position + v3(0.0, 0.65, 0.0), rect) {
            let color = if selected { theme::SELECTION } else { theme::MUTED };
            painter.text(label, egui::Align2::CENTER_BOTTOM, fixture.id.to_string(), egui::FontId::proportional(12.0), color);
        }

        // Zone cliquable du projecteur (et son nom, pour l'accessibilité).
        let hit = Rect::from_center_size(center, vec2(size.max(14.0), size.max(14.0)));
        let id = ui.scope_id().with(("stage-fixture", fixture.id));
        let area = ui.interact(hit.intersect(rect), id, Sense::click());
        let name = format!("Projecteur {} · {}", fixture.id, fixture.name);
        area.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        if area.on_hover_text(&name).clicked() {
            clicked = Some(fixture.id);
        }
    }
    if let Some(id) = clicked {
        let mode = if ui.input(|i| i.modifiers.command) { SelectMode::Toggle } else { SelectMode::Replace };
        console.select([Target::whole(id)], mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a.x - b.x).abs() < 1e-4 && (a.y - b.y).abs() < 1e-4 && (a.z - b.z).abs() < 1e-4
    }

    #[test]
    fn beams_point_down_then_follow_pan_and_tilt() {
        assert!(close(beam_direction(0.0, 0.0), v3(0.0, -1.0, 0.0)), "au repos : vers le bas");
        assert!(close(beam_direction(0.0, 90.0), v3(0.0, 0.0, 1.0)), "tilt 90° : vers le public");
        assert!(close(beam_direction(90.0, 90.0), v3(1.0, 0.0, 0.0)), "pan 90° : vers le côté");
        let (end, floor) = beam_end(v3(0.0, 5.0, 0.0), beam_direction(0.0, 45.0));
        assert!(floor && close(end, v3(0.0, 0.0, 5.0)), "45° depuis 5 m : touche le sol à 5 m");
        assert!(!beam_end(v3(0.0, 5.0, 0.0), beam_direction(0.0, 120.0)).1, "vers le haut : ne touche pas le sol");
    }

    #[test]
    fn camera_looks_from_the_audience_and_from_above() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        let depth = |camera: Camera, p: Vec3| camera.project(p, rect).unwrap().1;
        assert!(depth(Camera::FACE, v3(0.0, 0.0, 3.0)) < depth(Camera::FACE, v3(0.0, 0.0, -3.0)), "le nez de scène est devant");
        assert!(depth(Camera::TOP, v3(0.0, 5.0, 0.0)) < depth(Camera::TOP, v3(0.0, 0.0, 0.0)), "vue de dessus : la perche est devant le sol");
        let screen_x = |p: Vec3| Camera::FACE.project(p, rect).unwrap().0.x;
        assert!(screen_x(v3(2.0, 0.0, 0.0)) > screen_x(v3(-2.0, 0.0, 0.0)), "x vers la droite de l'écran");
        // Un faisceau au repos tombe à la verticale : vu de dessus, sur le projecteur.
        let (above, floor) = (Camera::TOP.project(v3(3.0, 5.0, 0.0), rect).unwrap().0, Camera::TOP.project(v3(3.0, 0.0, 0.0), rect).unwrap().0);
        assert!(floor.x.abs() < above.x.abs() + 1.0 && (above.x - rect.center().x).abs() > (floor.x - rect.center().x).abs(), "le sol, plus loin, est plus près du centre");
    }

    #[test]
    fn fixtures_line_up_on_the_truss() {
        let positions = auto_positions(4);
        assert_eq!(positions.len(), 4);
        assert!((positions[0].x + positions[3].x).abs() < 1e-4, "centrés");
        assert!(positions.iter().all(|p| p.y == TRUSS_HEIGHT));
    }
}

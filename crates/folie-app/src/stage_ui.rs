//! Fenêtre Scène : une vue simple en 3D, comme la fenêtre Stage de grandMA2. Les
//! projecteurs sont des blocs, chaque faisceau un trait de sa couleur réelle jusqu'au sol.
//! Tout part de ce qui sort vraiment : effets et executors compris.

use std::collections::BTreeMap;

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Stroke, pos2, vec2};
use folie_control::{CommandError, Console, Outcome, SelectMode};
use folie_engine::Snapshot;
use folie_fixture::{Attribute, FixtureId, Parameter, PatchedFixture, Target};
use folie_show::StagePlace;

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

    /// Point de la scène, à la hauteur `height`, qui se trouve sous `screen` (pour placer
    /// un projecteur à la souris). `None` si le regard ne croise pas ce plan.
    fn floor_point(&self, screen: Pos2, rect: Rect, height: f32) -> Option<Vec3> {
        let (cy, sy, cp, sp) = (self.yaw.cos(), self.yaw.sin(), self.pitch.cos(), self.pitch.sin());
        let focal = rect.height().min(rect.width()) * 1.25;
        // Point de la scène vu sous `screen` à la profondeur `depth` : l'inverse de `project`.
        let at = |depth: f32| {
            let x1 = (screen.x - rect.center().x) * depth / focal;
            let ys = -(screen.y - rect.center().y) * depth / focal;
            let along = self.distance - depth;
            let (y0, z1) = (ys * cp + along * sp, -ys * sp + along * cp);
            v3(x1 * cy + z1 * sy, y0 + TRUSS_HEIGHT / 2.5, -x1 * sy + z1 * cy)
        };
        let (a, b) = (at(0.0), at(1.0));
        let slope = b.y - a.y;
        if slope.abs() < 1e-6 {
            return None;
        }
        let depth = (height - a.y) / slope;
        (depth > 0.5).then(|| at(depth))
    }

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
pub(crate) fn angle_of(parameter: &Parameter, value: u16) -> f32 {
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

fn look(console: &Console, snapshot: &Snapshot, fixture: &PatchedFixture, position: Vec3, floor: bool) -> Look {
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
    let mut direction = beam_direction(angle(Attribute::Pan), angle(Attribute::Tilt));
    if floor {
        // Posé au sol : le projecteur est retourné, son faisceau part vers le haut.
        direction.y = -direction.y;
    }
    Look { position, intensity, color, direction }
}

/// Placement automatique : les projecteurs du patch alignés sur une perche, dans l'ordre.
fn auto_positions(count: usize) -> Vec<Vec3> {
    let spacing = (STAGE_WIDTH / count.max(1) as f32).min(1.6);
    let first = -spacing * (count.saturating_sub(1)) as f32 / 2.0;
    (0..count).map(|i| v3(first + spacing * i as f32, TRUSS_HEIGHT, TRUSS_Z)).collect()
}

type Action = Option<Result<Outcome, CommandError>>;

/// Place d'un projecteur : celle du show, sinon sur la perche (dans l'ordre du patch).
fn place_of(console: &Console, fixture: FixtureId, auto: Vec3) -> StagePlace {
    console.show.stage.get(&fixture).copied().unwrap_or(StagePlace { x: auto.x, y: auto.y, z: auto.z, floor: false })
}

/// Carte « Position » : côté, profondeur, hauteur et accroche des projecteurs sélectionnés.
fn position_card(ui: &mut egui::Ui, console: &mut Console, places: &BTreeMap<FixtureId, StagePlace>) -> Action {
    let selected: Vec<FixtureId> = {
        let mut ids: Vec<FixtureId> = console.selection().iter().map(|t| t.fixture).filter(|id| places.contains_key(id)).collect();
        ids.dedup();
        ids
    };
    let mut action = None;
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::section(ui, "Position");
        let Some(&first) = selected.first() else {
            ui.label(egui::RichText::new("Sélectionnez un projecteur (clic sur son bloc) pour le placer.").color(theme::MUTED));
            return;
        };
        let title = match selected.len() {
            1 => console.show.patch.get(first).map_or_else(|| first.to_string(), |f| format!("{} · {}", f.id, f.name)),
            n => format!("{n} projecteurs"),
        };
        ui.label(egui::RichText::new(title).family(theme::medium()));
        let mut place = places[&first];
        let before = place;
        egui::Grid::new("stage-position").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            let meters = |ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>| {
                let label = ui.label(egui::RichText::new(label).color(theme::MUTED));
                ui.add(egui::DragValue::new(value).range(range).speed(0.05).max_decimals(2).suffix(" m")).labelled_by(label.id);
                ui.end_row();
            };
            meters(ui, "Côté", &mut place.x, -20.0..=20.0);
            meters(ui, "Profondeur", &mut place.z, -20.0..=20.0);
            meters(ui, "Hauteur", &mut place.y, 0.0..=20.0);
        });
        // Accroche sur sa propre ligne : la carte reste étroite.
        ui.label(egui::RichText::new("Accroche").color(theme::MUTED));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut place.floor, false, "Suspendu").on_hover_text("Faisceau vers le bas");
            ui.selectable_value(&mut place.floor, true, "Au sol").on_hover_text("Faisceau vers le haut");
        });
        // Un champ changé s'applique à tous les projecteurs sélectionnés.
        if place != before {
            for id in &selected {
                let mut p = places[id];
                if place.x != before.x {
                    p.x = place.x;
                }
                if place.z != before.z {
                    p.z = place.z;
                }
                if place.y != before.y {
                    p.y = place.y;
                }
                p.floor = place.floor;
                action = Some(console.set_stage_place(*id, p));
            }
        }
        ui.add_space(4.0);
        if ui.button("Replacer sur la perche").on_hover_text("Rend les projecteurs sélectionnés à leur place sur la perche").clicked() {
            action = Some(console.reset_stage_places(&selected));
        }
        if selected.len() > 1 {
            ui.label(egui::RichText::new("Les champs règlent tous les projecteurs sélectionnés.").size(11.5).color(theme::MUTED));
        }
    });
    action
}

/// Côté (x) de la n-ième place sur la perche, pour les tests de l'interface.
#[cfg(test)]
pub(crate) fn auto_x(count: usize, index: usize) -> f32 {
    auto_positions(count)[index].x
}

pub fn stage_window(ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot) -> Action {
    let camera_id = egui::Id::unique("stage-camera");
    let placing_id = egui::Id::unique("stage-placing");
    let mut camera = ui
        .data_mut(|d| d.get_persisted::<(f32, f32, f32)>(camera_id))
        .map_or(Camera::THREE_QUARTER, |(yaw, pitch, distance)| Camera { yaw, pitch, distance });
    let mut placing = ui.data_mut(|d| d.get_persisted::<bool>(placing_id)).unwrap_or(false);
    let mut action = None;

    ui.horizontal(|ui| {
        theme::section(ui, "Scène");
        for (label, view) in [("Face", Camera::FACE), ("Dessus", Camera::TOP), ("3/4", Camera::THREE_QUARTER)] {
            let on = (camera.yaw, camera.pitch) == (view.yaw, view.pitch);
            if ui.add(egui::Button::selectable(on, label)).on_hover_text(format!("Vue {label}")).clicked() {
                camera = Camera { distance: camera.distance, ..view };
            }
        }
        ui.separator();
        ui.toggle_value(&mut placing, "Placer").on_hover_text("Déplacer les projecteurs à la souris");
        let hint = if placing {
            "Glisser un projecteur : le déplacer · Maj-glisser : sa hauteur"
        } else {
            "Glisser : tourner · molette : zoom · clic : sélectionner"
        };
        ui.label(egui::RichText::new(hint).size(11.5).color(theme::MUTED));
    });

    let fixtures: Vec<PatchedFixture> = console.show.patch.fixtures().iter().filter(|f| f.universe == 0).cloned().collect();
    let places: BTreeMap<FixtureId, StagePlace> = fixtures
        .iter()
        .zip(auto_positions(fixtures.len()))
        .map(|(f, auto)| (f.id, place_of(console, f.id, auto)))
        .collect();

    let card_width = if placing { 250.0 } else { 0.0 };
    let size = vec2((ui.available_width() - card_width).max(200.0), ui.available_height());
    ui.horizontal_top(|ui| {
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
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
        if let Some(a) = draw_scene(ui, console, snapshot, &camera, rect, &fixtures, &places, placing) {
            action = Some(a);
        }
        if placing {
            ui.vertical(|ui| {
                ui.set_width(card_width - 8.0);
                ui.set_max_width(card_width - 8.0);
                if let Some(a) = position_card(ui, console, &places) {
                    action = Some(a);
                }
            });
        }
    });
    ui.data_mut(|d| {
        d.insert_persisted(camera_id, (camera.yaw, camera.pitch, camera.distance));
        d.insert_persisted(placing_id, placing);
    });
    action
}

/// Arêtes d'une boîte centrée sur `center`, de demi-côtés `half`.
fn box_edges(center: Vec3, half: Vec3, stroke: Stroke, line: &impl Fn(Vec3, Vec3, Stroke)) {
    let mut corners = Vec::with_capacity(8);
    for dx in [-half.x, half.x] {
        for dy in [-half.y, half.y] {
            for dz in [-half.z, half.z] {
                corners.push(center + v3(dx, dy, dz));
            }
        }
    }
    for (a, b) in [(0, 1), (0, 2), (0, 4), (1, 3), (1, 5), (2, 3), (2, 6), (3, 7), (4, 5), (4, 6), (5, 7), (6, 7)] {
        line(corners[a], corners[b], stroke);
    }
}

/// Longueur en mètres d'un appareil en barre (bien plus long que large et profond), lue dans
/// les dimensions de sa fiche ; `None` pour un projecteur ordinaire. Gardée d'une image à l'autre.
fn bar_length(ui: &egui::Ui, fixture: &PatchedFixture) -> Option<f32> {
    let cache = egui::Id::unique("stage-bar-lengths");
    let key = fixture.fixture_type.key.clone();
    if let Some(known) = ui.data(|d| d.get_temp::<BTreeMap<String, Option<f32>>>(cache).and_then(|m| m.get(&key).copied())) {
        return known;
    }
    let length = bar_length_of(&fixture.fixture_type.ofl_source);
    ui.data_mut(|d| d.get_temp_mut_or_default::<BTreeMap<String, Option<f32>>>(cache).insert(key, length));
    length
}

fn bar_length_of(ofl_source: &str) -> Option<f32> {
    let ofl: serde_json::Value = serde_json::from_str(ofl_source).ok()?;
    let size = |i: usize| ofl.pointer(&format!("/physical/dimensions/{i}")).and_then(serde_json::Value::as_f64);
    let (width, height, depth) = (size(0)?, size(1).unwrap_or(0.0), size(2).unwrap_or(0.0));
    (width > 3.0 * height.max(depth)).then_some(width as f32 / 1000.0)
}

/// Niveau (0 à 1) de chaque cellule d'une barre, dans l'ordre des cellules : son propre
/// dimmer, multiplié par le dimmer général s'il y en a un. Sans cellules, une seule case.
fn cell_levels(snapshot: &Snapshot, fixture: &PatchedFixture, whole: f32) -> Vec<(Option<usize>, f32)> {
    let mode = fixture.mode();
    let level = |head: Option<usize>| {
        mode.parameters
            .iter()
            .filter(|p| p.attribute == Attribute::Dimmer && p.head == head)
            .filter_map(|p| crate::fixtures::live_value(snapshot, fixture, p))
            .map(|v| v as f32 / 65535.0)
            .reduce(f32::max)
    };
    if mode.heads.is_empty() {
        return vec![(None, whole)];
    }
    let master = level(None).unwrap_or(1.0);
    (0..mode.heads.len()).map(|h| (Some(h), level(Some(h)).unwrap_or(1.0) * master)).collect()
}

/// Sol, plateau, perche, projecteurs et faisceaux ; clics et glissés sur les projecteurs.
#[allow(clippy::too_many_arguments)]
fn draw_scene(
    ui: &mut egui::Ui,
    console: &mut Console,
    snapshot: &Snapshot,
    camera: &Camera,
    rect: Rect,
    fixtures: &[PatchedFixture],
    places: &BTreeMap<FixtureId, StagePlace>,
    placing: bool,
) -> Action {
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

    let mut action = None;
    let mut clicked = None;
    for fixture in fixtures {
        let place = places[&fixture.id];
        let position = v3(place.x, place.y, place.z);
        let look = look(console, snapshot, fixture, position, place.floor);
        let selected = console.selection().iter().any(|t| t.fixture == fixture.id);

        let stroke = Stroke::new(if selected { 2.0 } else { 1.5 }, if selected { theme::SELECTION } else { theme::TEXT });
        // Cellules cliquables d'une barre, enregistrées après le projecteur pour passer dessus.
        let mut cell_hits: Vec<(Rect, usize)> = Vec::new();
        let (depth, hit) = if let Some(length) = bar_length(ui, fixture) {
            // Appareil en barre (Sunstrip…) : ses cellules côte à côte, chacune à son dimmer.
            let s = 0.12;
            let half = length / 2.0;
            box_edges(position, v3(half, s, s), stroke, &line);
            let cells = cell_levels(snapshot, fixture, look.intensity);
            let width = length / cells.len() as f32;
            for (i, (head, level)) in cells.iter().enumerate() {
                // Cellule 1 côté cour (à droite vu de la salle), vérifié sur un Sunstrip.
                let x = half - width * (i as f32 + 0.5);
                let quad: Option<Vec<Pos2>> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .iter()
                    .map(|(dx, dy)| camera.project(position + v3(x + dx * width * 0.42, dy * s * 0.8, s), rect).map(|(p, _)| p))
                    .collect();
                let [r, g, b, _] = look.color.to_array();
                let light = level * r.max(g).max(b) as f32 / 255.0;
                let cell_selected = head.is_some_and(|h| console.selection().contains(&Target { fixture: fixture.id, head: Some(h) }));
                if let Some(quad) = quad {
                    let outline = if cell_selected { Stroke::new(1.5, theme::SELECTION) } else { Stroke::NONE };
                    painter.add(egui::Shape::convex_polygon(quad.clone(), look.color.gamma_multiply(0.08 + 0.9 * level), outline));
                    if let Some(h) = head {
                        cell_hits.push((Rect::from_points(&quad).expand(2.0), *h));
                    }
                }
                // Faisceau court et pâle : une barre éclaire large et proche.
                if light > 0.02 {
                    let origin = position + v3(x, -s, 0.0);
                    let (end, on_floor) = beam_end(origin, look.direction);
                    line(origin, end, Stroke::new(2.0, look.color.gamma_multiply(0.12 + 0.45 * level)));
                    if on_floor && let Some((spot, depth)) = camera.project(end, rect) {
                        let radius = (rect.height() * 0.25 / depth).max(2.0);
                        painter.circle_filled(spot, radius, look.color.gamma_multiply(0.25 * level));
                    }
                }
            }
            let (Some((a, _)), Some((b, depth))) = (camera.project(position + v3(-half, 0.0, 0.0), rect), camera.project(position + v3(half, 0.0, 0.0), rect))
            else {
                continue;
            };
            let size = rect.height() * 0.9 / depth;
            (depth, Rect::from_two_pos(a, b).expand2(vec2(4.0, (size * 0.3).max(7.0))))
        } else {
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
            box_edges(position, v3(0.3, 0.3, 0.3), stroke, &line);
            let Some((center, depth)) = camera.project(position, rect) else { continue };
            let size = rect.height() * 0.9 / depth;
            painter.circle_filled(center, size * 0.5, look.color.gamma_multiply(0.15 + 0.6 * look.intensity));
            (depth, Rect::from_center_size(center, vec2(size.max(14.0), size.max(14.0))))
        };
        // En placement, un trait jusqu'au sol aide à voir où est le projecteur.
        if placing && place.y > 0.05 {
            line(position, v3(place.x, 0.0, place.z), Stroke::new(1.0, if selected { theme::SELECTION } else { theme::DEFAULT }));
        }
        if let Some((label, _)) = camera.project(position + v3(0.0, 0.65, 0.0), rect) {
            let color = if selected { theme::SELECTION } else { theme::MUTED };
            painter.text(label, egui::Align2::CENTER_BOTTOM, fixture.id.to_string(), egui::FontId::proportional(12.0), color);
        }

        // Zone du projecteur : clic pour le sélectionner, glissé pour le placer.
        let id = ui.scope_id().with(("stage-fixture", fixture.id));
        let sense = if placing { Sense::click_and_drag() } else { Sense::click() };
        let area = ui.interact(hit.intersect(rect), id, sense);
        let name = format!("Projecteur {} · {}", fixture.id, fixture.name);
        area.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
        let area = area.on_hover_text(&name);
        if !placing {
            for (cell, h) in cell_hits {
                let id = ui.scope_id().with(("stage-cell", fixture.id, h));
                let name = format!("Projecteur {}.{}", fixture.id, h + 1);
                let cell = ui.interact(cell.intersect(rect), id, Sense::click());
                cell.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &name));
                if cell.on_hover_text(&name).clicked() {
                    clicked = Some(Target { fixture: fixture.id, head: Some(h) });
                }
            }
        }

        if clicked.is_none() && (area.clicked() || (area.drag_started() && !selected)) {
            clicked = Some(Target::whole(fixture.id));
        }
        if placing && area.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            let mut moved = place;
            if ui.input(|i| i.modifiers.shift) {
                // Hauteur : vers le haut de l'écran, plus haut.
                let meters_per_point = depth / (rect.height().min(rect.width()) * 1.25);
                moved.y = (place.y - area.drag_delta().y * meters_per_point).clamp(0.0, 20.0);
            } else if let Some(pointer) = area.interact_pointer_pos()
                && let Some(point) = camera.floor_point(pointer, rect, place.y)
            {
                moved.x = point.x.clamp(-20.0, 20.0);
                moved.z = point.z.clamp(-20.0, 20.0);
            }
            if moved != place {
                action = Some(console.set_stage_place(fixture.id, moved));
            }
        }
    }
    if let Some(target) = clicked {
        let mode = if ui.input(|i| i.modifiers.command) { SelectMode::Toggle } else { SelectMode::Replace };
        console.select([target], mode);
    }
    action
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
    fn a_point_on_screen_maps_back_to_the_stage() {
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        for camera in [Camera::FACE, Camera::TOP, Camera::THREE_QUARTER] {
            for point in [v3(2.0, 0.0, 1.5), v3(-3.0, 5.0, -1.0)] {
                let (screen, _) = camera.project(point, rect).unwrap();
                let back = camera.floor_point(screen, rect, point.y).unwrap();
                let near = (back.x - point.x).abs() < 1e-2 && (back.y - point.y).abs() < 1e-2 && (back.z - point.z).abs() < 1e-2;
                assert!(near, "{camera:?} {point:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn bars_are_recognised_from_their_dimensions() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library/");
        let source = |path: &str| std::fs::read_to_string(format!("{root}{path}")).unwrap();
        let sunstrip = bar_length_of(&source("showtec/sunstrip-active-mkii.json")).expect("une barre");
        assert!((sunstrip - 1.08).abs() < 1e-3);
        assert_eq!(bar_length_of(&source("martin/rush-mh-6-wash.json")), None, "une lyre n'est pas une barre");
        assert_eq!(bar_length_of("{}"), None);
    }

    #[test]
    fn fixtures_line_up_on_the_truss() {
        let positions = auto_positions(4);
        assert_eq!(positions.len(), 4);
        assert!((positions[0].x + positions[3].x).abs() < 1e-4, "centrés");
        assert!(positions.iter().all(|p| p.y == TRUSS_HEIGHT));
    }
}

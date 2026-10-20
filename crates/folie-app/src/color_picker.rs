//! Sélecteur de couleur du Programmer : carré saturation/luminosité, bande de
//! teinte, couleurs rapides.

use eframe::egui::{self, Color32, Mesh, Pos2, Rect, Sense, Stroke, vec2};
use folie_control::color::{hsv_to_rgb, rgb_to_hsv};

use crate::theme;

const SQUARE: f32 = 150.0;
const STRIP: f32 = 18.0;

/// Couleurs rapides : (nom, rouge, vert, bleu).
pub const QUICK: [(&str, [f32; 3]); 11] = [
    ("Blanc", [1.0, 1.0, 1.0]),
    ("Blanc chaud", [1.0, 0.78, 0.5]),
    ("Rouge", [1.0, 0.0, 0.0]),
    ("Ambre", [1.0, 0.5, 0.0]),
    ("Jaune", [1.0, 0.85, 0.0]),
    ("Vert", [0.0, 1.0, 0.0]),
    ("Cyan", [0.0, 1.0, 1.0]),
    ("Bleu", [0.0, 0.0, 1.0]),
    ("Lavande", [0.55, 0.4, 1.0]),
    ("Magenta", [1.0, 0.0, 1.0]),
    ("Rose", [1.0, 0.35, 0.6]),
];

fn color(rgb: [f32; 3]) -> Color32 {
    let [r, g, b] = rgb.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

/// Carré saturation (horizontale) × luminosité (verticale) pour la teinte en cours.
fn sv_square(ui: &mut egui::Ui, hsv: &mut [f32; 3]) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(SQUARE, SQUARE), Sense::click_and_drag());
    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        hsv[1] = ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        hsv[2] = (1.0 - (pos.y - rect.top()) / rect.height()).clamp(0.0, 1.0);
        response.mark_changed();
    }
    // Blanc → teinte pure en haut, noir en bas : l'interpolation des sommets donne
    // exactement luminosité × mélange(blanc, teinte, saturation).
    let hue = color(hsv_to_rgb([hsv[0], 1.0, 1.0]));
    let mut mesh = Mesh::default();
    for (pos, c) in [
        (rect.left_top(), Color32::WHITE),
        (rect.right_top(), hue),
        (rect.right_bottom(), Color32::BLACK),
        (rect.left_bottom(), Color32::BLACK),
    ] {
        mesh.colored_vertex(pos, c);
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    let painter = ui.painter_at(rect);
    painter.add(mesh);
    let marker = Pos2::new(rect.left() + hsv[1] * rect.width(), rect.top() + (1.0 - hsv[2]) * rect.height());
    painter.circle_stroke(marker, 6.0, Stroke::new(2.0, Color32::WHITE));
    painter.circle_stroke(marker, 7.5, Stroke::new(1.0, Color32::BLACK));
    let value = hsv[1];
    response.widget_info(|| egui::WidgetInfo::slider(true, value as f64, "Saturation et luminosité"));
    response
}

/// Bande verticale des teintes.
fn hue_strip(ui: &mut egui::Ui, hsv: &mut [f32; 3]) -> egui::Response {
    let (rect, mut response) = ui.allocate_exact_size(vec2(STRIP, SQUARE), Sense::click_and_drag());
    if (response.dragged() || response.clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        hsv[0] = ((pos.y - rect.top()) / rect.height()).clamp(0.0, 0.999);
        response.mark_changed();
    }
    let mut mesh = Mesh::default();
    const STEPS: usize = 36;
    for i in 0..=STEPS {
        let t = i as f32 / STEPS as f32;
        let y = rect.top() + t * rect.height();
        let c = color(hsv_to_rgb([t, 1.0, 1.0]));
        mesh.colored_vertex(Pos2::new(rect.left(), y), c);
        mesh.colored_vertex(Pos2::new(rect.right(), y), c);
        if i > 0 {
            let base = (2 * i - 2) as u32;
            mesh.add_triangle(base, base + 1, base + 2);
            mesh.add_triangle(base + 1, base + 3, base + 2);
        }
    }
    let painter = ui.painter_at(rect.expand(2.0));
    painter.add(mesh);
    let y = rect.top() + hsv[0] * rect.height();
    let marker = Rect::from_min_max(Pos2::new(rect.left() - 2.0, y - 2.0), Pos2::new(rect.right() + 2.0, y + 2.0));
    painter.rect_stroke(marker, 2.0, Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Middle);
    let hue = hsv[0];
    response.widget_info(|| egui::WidgetInfo::slider(true, hue as f64, "Teinte"));
    response
}

fn swatch_button(ui: &mut egui::Ui, name: &str, rgb: [f32; 3]) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::click());
    let painter = ui.painter();
    painter.rect_filled(rect, 5.0, color(rgb));
    let stroke = if response.hovered() { Stroke::new(1.5, Color32::WHITE) } else { Stroke::new(1.0, theme::LINE) };
    painter.rect_stroke(rect, 5.0, stroke, egui::StrokeKind::Inside);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
    response.on_hover_text(name).clicked()
}

/// Sélecteur complet. `current` : couleur produite en ce moment par la cible de
/// référence. Renvoie la couleur choisie quand l'utilisateur en change.
pub fn color_picker(ui: &mut egui::Ui, hsv: &mut [f32; 3], current: Option<[u8; 3]>) -> Option<[f32; 3]> {
    let mut chosen = None;
    ui.horizontal_top(|ui| {
        if sv_square(ui, hsv).changed() | hue_strip(ui, hsv).changed() {
            chosen = Some(hsv_to_rgb(*hsv));
        }
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let preview = |ui: &mut egui::Ui, c: Option<Color32>, label: &str| {
                    ui.vertical(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(56.0, 30.0), Sense::hover());
                        match c {
                            Some(c) => ui.painter().rect_filled(rect, 5.0, c),
                            None => ui.painter().rect_stroke(rect, 5.0, Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside),
                        };
                        ui.label(egui::RichText::new(label).size(11.0).color(theme::MUTED));
                    });
                };
                preview(ui, Some(color(hsv_to_rgb(*hsv))), "visée");
                preview(ui, current.map(|[r, g, b]| Color32::from_rgb(r, g, b)), "actuelle");
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(6.0 * 30.0);
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for (name, rgb) in QUICK {
                    if swatch_button(ui, name, rgb) {
                        *hsv = rgb_to_hsv(rgb);
                        chosen = Some(rgb);
                    }
                }
            });
        });
    });
    chosen
}

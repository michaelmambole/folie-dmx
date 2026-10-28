//! Logo de Folie : le « l » est une grande jauge, le « i » une petite, et le point du
//! « i » est rouge (la couleur du programmer). L'icône ne garde que le « li ».

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Stroke, StrokeKind, vec2};

use crate::theme;

const TRACK: Color32 = theme::SURFACE_2;
const DOT: Color32 = theme::PROGRAMMER;
/// Remplissage des jauges : le « l » plus plein que le « i ».
const L_LEVEL: f32 = 0.66;
const I_LEVEL: f32 = 0.55;

/// Une jauge : rail sombre, remplissage ambre par le bas.
fn gauge(painter: &egui::Painter, rect: Rect, level: f32) {
    let radius = CornerRadius::same((rect.width() * 0.3) as u8);
    painter.rect(rect, radius, TRACK, Stroke::new((rect.width() * 0.08).max(1.0), theme::LINE), StrokeKind::Inside);
    let inset = rect.width() * 0.16;
    let inner = rect.shrink(inset);
    let fill = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - inner.height() * level), inner.max);
    painter.rect_filled(fill, CornerRadius::same((inner.width() * 0.3) as u8), theme::EXECUTOR);
}

/// « li » : grande jauge, petite jauge et son point, dans `rect` (proportions 1 × 1).
pub fn mark(painter: &egui::Painter, rect: Rect) {
    let unit = rect.height() / 44.0;
    let at = |x: f32, y: f32| rect.min + vec2(x * unit, y * unit);
    let width = 12.0 * unit;
    gauge(painter, Rect::from_min_size(at(6.0, 0.0), vec2(width, 44.0 * unit)), L_LEVEL);
    gauge(painter, Rect::from_min_size(at(26.0, 16.0), vec2(width, 28.0 * unit)), I_LEVEL);
    painter.circle_filled(at(32.0, 6.0), 5.5 * unit, DOT);
}

/// Marque « li » seule, à la taille `size`, comme un widget.
pub fn mark_widget(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    mark(ui.painter(), rect);
    response
}

/// Logo complet « Folie », `height` : hauteur des capitales.
pub fn wordmark(ui: &mut egui::Ui, height: f32) -> egui::Response {
    let font = egui::FontId::new(height / 0.73, theme::medium());
    let fo = ui.painter().layout_no_wrap("Fo".into(), font.clone(), theme::TEXT);
    let e = ui.painter().layout_no_wrap("e".into(), font, theme::TEXT);
    let width_gauge = height * 0.30;
    let gap = height * 0.10;
    let total = fo.size().x + gap + width_gauge + gap + width_gauge + gap * 0.8 + e.size().x;
    let (rect, response) = ui.allocate_exact_size(vec2(total, fo.size().y), egui::Sense::hover());
    let painter = ui.painter();
    // Ligne de base et hauteur des capitales d'Inter, d'après la hauteur de ligne.
    let baseline = rect.top() + fo.size().y * 0.80;
    let cap_top = baseline - height;
    let x_top = baseline - height * 0.66;
    painter.galley(rect.min, fo.clone(), theme::TEXT);
    let mut x = rect.left() + fo.size().x + gap;
    gauge(painter, Rect::from_min_max(Pos2::new(x, cap_top), Pos2::new(x + width_gauge, baseline)), L_LEVEL);
    x += width_gauge + gap;
    gauge(painter, Rect::from_min_max(Pos2::new(x, x_top), Pos2::new(x + width_gauge, baseline)), I_LEVEL);
    // Le point flotte au-dessus de la petite jauge, comme celui d'un « i ».
    painter.circle_filled(Pos2::new(x + width_gauge / 2.0, x_top - width_gauge * 0.85), width_gauge * 0.5, DOT);
    x += width_gauge + gap * 0.8;
    painter.galley(Pos2::new(x, rect.top()), e, theme::TEXT);
    response.on_hover_text("Folie-DMX")
}

/// Icône de l'application (Dock, fenêtres) : le « li » sur un carré arrondi sombre.
/// Dessinée point par point, avec 4 × 4 échantillons par pixel pour adoucir les bords.
pub fn icon(size: u32) -> egui::IconData {
    // Formes en unités de 64, peintes dans l'ordre : (gauche, haut, droite, bas, rayon, couleur).
    let rgba = |c: Color32| [c.r(), c.g(), c.b(), 255];
    let mut shapes: Vec<([f32; 5], [u8; 4])> = vec![
        ([2.0, 2.0, 62.0, 62.0, 14.0], rgba(theme::LINE)),
        ([3.0, 3.0, 61.0, 61.0, 13.0], rgba(theme::SURFACE_1)),
    ];
    let mut gauge = |l: f32, t: f32, r: f32, b: f32, level: f32| {
        shapes.push(([l, t, r, b, 4.0], rgba(theme::LINE)));
        shapes.push(([l + 1.0, t + 1.0, r - 1.0, b - 1.0, 3.2], rgba(TRACK)));
        let (il, ir, ib) = (l + 2.0, r - 2.0, b - 2.0);
        let it = ib - (ib - (t + 2.0)) * level;
        shapes.push(([il, it, ir, ib, 2.5], rgba(theme::EXECUTOR)));
    };
    gauge(17.0, 10.0, 29.0, 54.0, L_LEVEL);
    gauge(35.0, 26.0, 47.0, 54.0, I_LEVEL);
    let dot = (41.0, 16.0, 5.5);

    let scale = 64.0 / size as f32;
    let mut pixels = vec![0u8; (size * size * 4) as usize];
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = (px as f32 + (sx as f32 + 0.5) / 4.0) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / 4.0) * scale;
                    let mut color = [0.0f32; 4];
                    for ([l, t, r, b, radius], c) in &shapes {
                        if inside_rounded(x, y, *l, *t, *r, *b, *radius) {
                            color = [c[0] as f32, c[1] as f32, c[2] as f32, 255.0];
                        }
                    }
                    if (x - dot.0).powi(2) + (y - dot.1).powi(2) <= dot.2 * dot.2 {
                        color = [DOT.r() as f32, DOT.g() as f32, DOT.b() as f32, 255.0];
                    }
                    // Couleurs prémultipliées le temps de la moyenne.
                    let a = color[3] / 255.0;
                    acc[0] += color[0] * a;
                    acc[1] += color[1] * a;
                    acc[2] += color[2] * a;
                    acc[3] += color[3];
                }
            }
            let alpha = acc[3] / 16.0;
            let i = ((py * size + px) * 4) as usize;
            let unmultiply = |c: f32| if alpha > 0.0 { (c / 16.0 / (alpha / 255.0)).round().min(255.0) as u8 } else { 0 };
            pixels[i..i + 4].copy_from_slice(&[unmultiply(acc[0]), unmultiply(acc[1]), unmultiply(acc[2]), alpha.round() as u8]);
        }
    }
    egui::IconData { rgba: pixels, width: size, height: size }
}

fn inside_rounded(x: f32, y: f32, l: f32, t: f32, r: f32, b: f32, radius: f32) -> bool {
    if x < l || x > r || y < t || y > b {
        return false;
    }
    let cx = x.clamp(l + radius, r - radius);
    let cy = y.clamp(t + radius, b - radius);
    (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_transparent_corners_and_a_red_dot() {
        let icon = icon(64);
        let pixel = |x: u32, y: u32| {
            let i = ((y * 64 + x) * 4) as usize;
            [icon.rgba[i], icon.rgba[i + 1], icon.rgba[i + 2], icon.rgba[i + 3]]
        };
        assert_eq!(pixel(0, 0)[3], 0, "coin transparent");
        assert_eq!(pixel(41, 16), [DOT.r(), DOT.g(), DOT.b(), 255], "point du i");
        assert_eq!(pixel(23, 50), [theme::EXECUTOR.r(), theme::EXECUTOR.g(), theme::EXECUTOR.b(), 255], "jauge du l");
        if let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") {
            let big = super::icon(512);
            let image = image_of(&big);
            image.save(std::path::Path::new(&dir).join("icone.png")).expect("écriture de l'icône");
        }
    }

    /// FOLIE_ICONSET=<dossier>.iconset écrit les images de l'icône macOS (pour `iconutil`),
    /// avec la marge des icônes d'application : le dessin occupe 824 points sur 1024.
    #[test]
    fn macos_iconset() {
        let Some(dir) = std::env::var_os("FOLIE_ICONSET") else { return };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).expect("dossier de l'iconset");
        for points in [16u32, 32, 128, 256, 512] {
            for scale in [1u32, 2] {
                let size = points * scale;
                let inner = (size as f32 * 824.0 / 1024.0).round() as u32;
                let mut canvas = image::RgbaImage::new(size, size);
                let offset = ((size - inner) / 2) as i64;
                image::imageops::overlay(&mut canvas, &image_of(&super::icon(inner)), offset, offset);
                let suffix = if scale == 2 { "@2x" } else { "" };
                canvas.save(dir.join(format!("icon_{points}x{points}{suffix}.png"))).expect("écriture de l'icône");
            }
        }
    }

    /// Le logo en grand, pour en vérifier les proportions.
    #[test]
    fn wordmark_capture() {
        let Some(dir) = std::env::var_os("FOLIE_UI_CAPTURE") else { return };
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(520.0, 240.0)).build_ui(|ui| {
            crate::theme::apply(ui.ctx());
            egui::Frame::new().fill(theme::BG).inner_margin(20).show(ui, |ui| {
                wordmark(ui, 90.0);
                ui.horizontal(|ui| {
                    wordmark(ui, 30.0);
                    mark_widget(ui, 30.0);
                });
            });
        });
        harness.run();
        harness.render().expect("rendu").save(std::path::Path::new(&dir).join("logo.png")).expect("écriture du logo");
    }

    fn image_of(icon: &egui::IconData) -> image::RgbaImage {
        image::RgbaImage::from_raw(icon.width, icon.height, icon.rgba.clone()).unwrap()
    }
}

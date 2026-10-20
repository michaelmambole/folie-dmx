//! Sélecteur de couleur : une couleur visée, traduite pour chaque appareil selon
//! ce qu'il sait faire (RGB, RGBW, CMY ou roue de couleurs seule).

use folie_fixture::{Attribute, CapabilityKind, Parameter, Target};

use crate::Console;

/// Teinte (0 à 1), saturation et luminosité (0 à 1) vers RGB (0 à 1).
pub fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let h = h.rem_euclid(1.0) * 6.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

pub fn rgb_to_hsv([r, g, b]: [f32; 3]) -> [f32; 3] {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let h = if delta == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / delta).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / delta + 2.0) / 6.0
    } else {
        ((r - g) / delta + 4.0) / 6.0
    };
    let s = if max == 0.0 { 0.0 } else { delta / max };
    [h, s, max]
}

fn level(v: f32) -> u16 {
    (v.clamp(0.0, 1.0) * u16::MAX as f32).round() as u16
}

/// Valeurs à envoyer à une cible pour approcher `rgb`. Vide si l'appareil n'a
/// aucun moyen de faire de la couleur.
pub fn color_values(parameters: &[(usize, &Parameter)], rgb: [f32; 3]) -> Vec<(usize, u16)> {
    let mix = |name: &str| {
        parameters.iter().find(|(_, p)| p.attribute == Attribute::ColorMix(name.into())).map(|(i, _)| *i)
    };
    let [r, g, b] = rgb.map(|c| c.clamp(0.0, 1.0));
    let mut values = Vec::new();

    if let (Some(ri), Some(gi), Some(bi)) = (mix("Red"), mix("Green"), mix("Blue")) {
        // Additif. Avec un blanc, on en extrait la part commune : plus de lumière, pastels plus francs.
        let rest = match mix("White") {
            Some(wi) => {
                let w = r.min(g).min(b);
                values.push((wi, level(w)));
                [r - w, g - w, b - w]
            }
            None => [r, g, b],
        };
        values.extend([(ri, level(rest[0])), (gi, level(rest[1])), (bi, level(rest[2]))]);
        // Les autres émetteurs se taisent : la couleur ne doit venir que du mélange choisi.
        for other in ["Amber", "Lime", "UV", "Warm White", "Cold White"] {
            if let Some(i) = mix(other) {
                values.push((i, 0));
            }
        }
        // Une roue (simulée ou réelle) masquerait le mélange : on la met en position ouverte.
        for (i, p) in parameters.iter().filter(|(_, p)| p.attribute == Attribute::ColorMacro) {
            if let Some(open) = p.capabilities.iter().find(|c| c.kind == CapabilityKind::NoFunction) {
                values.push((*i, open.select_value()));
            }
        }
        return values;
    }

    if let (Some(ci), Some(mi), Some(yi)) = (mix("Cyan"), mix("Magenta"), mix("Yellow")) {
        return vec![(ci, level(1.0 - r)), (mi, level(1.0 - g)), (yi, level(1.0 - b))];
    }

    // Roue de couleurs seule : la couleur la plus proche.
    let distance = |c: [u8; 3]| {
        let c = c.map(|v| v as f32 / 255.0);
        (c[0] - r).powi(2) + (c[1] - g).powi(2) + (c[2] - b).powi(2)
    };
    for (i, p) in parameters.iter().filter(|(_, p)| p.attribute == Attribute::ColorMacro) {
        let nearest = p
            .capabilities
            .iter()
            .filter_map(|cap| match &cap.kind {
                CapabilityKind::ColorPreset { colors } => colors.first().map(|c| (distance(*c), cap.select_value())),
                _ => None,
            })
            .min_by(|a, b| a.0.total_cmp(&b.0));
        if let Some((_, value)) = nearest {
            values.push((*i, value));
        }
    }
    values
}

impl Console {
    /// Donne une couleur à toute la sélection, traduite pour chaque appareil.
    /// Renvoie le nombre de cibles qui ont pu la prendre.
    pub fn apply_color(&mut self, rgb: [f32; 3]) -> usize {
        let mut colored = 0;
        for target in self.selection.clone() {
            let values: Vec<(usize, u16)> = {
                let parameters = self.show.patch.parameters_of(target);
                color_values(&parameters, rgb)
            };
            colored += usize::from(!values.is_empty());
            for (parameter, value) in values {
                self.send_value(target, parameter, value);
            }
        }
        self.selection_used = true;
        colored
    }

    /// Couleur que produit une cible avec les valeurs données (par nom de paramètre).
    pub fn target_color(&self, target: Target, values: &std::collections::BTreeMap<String, u16>) -> Option<[u8; 3]> {
        let fixture = self.show.patch.get(target.fixture)?;
        crate::fixture_color(fixture, values).map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
    }
}

/// Clé utile aux tests : un paramètre de la première cible par nom.
#[cfg(test)]
pub(crate) fn key_of(console: &Console, target: Target, name: &str) -> folie_fixture::ParamKey {
    let (parameter, _) =
        console.show.patch.parameters_of(target).into_iter().find(|(_, p)| p.name == name).expect("paramètre");
    folie_fixture::ParamKey { fixture: target.fixture, parameter }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_round_trip() {
        for rgb in [[1.0, 0.0, 0.0], [1.0, 0.5, 0.0], [0.2, 0.4, 0.9], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]] {
            let back = hsv_to_rgb(rgb_to_hsv(rgb));
            for i in 0..3 {
                assert!((back[i] - rgb[i]).abs() < 1e-5, "{rgb:?} → {back:?}");
            }
        }
        assert_eq!(hsv_to_rgb([1.0 / 3.0, 1.0, 1.0]), [0.0, 1.0, 0.0]);
    }
}

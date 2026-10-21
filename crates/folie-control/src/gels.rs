//! Gélatines des nuanciers Lee et Rosco, pour retrouver une couleur de théâtre connue.
//!
//! Les couleurs sont des approximations à l'écran (sRGB) d'un filtre devant une source
//! blanche : une LED n'aura jamais exactement le spectre d'une gélatine, mais on part
//! de la bonne teinte.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Brand {
    Lee,
    Rosco,
}

impl Brand {
    pub const ALL: [Brand; 2] = [Brand::Lee, Brand::Rosco];

    pub fn label(self) -> &'static str {
        match self {
            Self::Lee => "Lee",
            Self::Rosco => "Rosco",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gel {
    pub brand: Brand,
    /// Référence du nuancier : « 201 » chez Lee, « R80 » chez Rosco.
    pub code: &'static str,
    pub name: &'static str,
    rgb: u32,
}

impl Gel {
    /// Rouge, vert, bleu de 0 à 1.
    pub fn rgb(&self) -> [f32; 3] {
        [(self.rgb >> 16) & 0xff, (self.rgb >> 8) & 0xff, self.rgb & 0xff].map(|c| c as f32 / 255.0)
    }

    /// « Lee 201 Full C.T. Blue ».
    pub fn label(&self) -> String {
        format!("{} {} {}", self.brand.label(), self.code, self.name)
    }

    /// La recherche porte sur la référence et le nom : « 201 », « r80 », « blue ».
    pub fn matches(&self, query: &str) -> bool {
        let query = query.trim().to_lowercase();
        query.is_empty() || self.code.to_lowercase().contains(&query) || self.name.to_lowercase().contains(&query)
    }
}

const fn lee(code: &'static str, name: &'static str, rgb: u32) -> Gel {
    Gel { brand: Brand::Lee, code, name, rgb }
}

const fn rosco(code: &'static str, name: &'static str, rgb: u32) -> Gel {
    Gel { brand: Brand::Rosco, code, name, rgb }
}

/// Les références les plus courantes en régie, dans l'ordre des nuanciers.
pub const GELS: &[Gel] = &[
    lee("002", "Rose Pink", 0xff5fb4),
    lee("003", "Lavender Tint", 0xf2e4ff),
    lee("007", "Pale Yellow", 0xfffbc8),
    lee("008", "Dark Salmon", 0xff8a6a),
    lee("010", "Medium Yellow", 0xfff04a),
    lee("013", "Straw Tint", 0xffe6a8),
    lee("017", "Surprise Peach", 0xd9947e),
    lee("020", "Medium Amber", 0xffb15a),
    lee("021", "Gold Amber", 0xff9a3c),
    lee("024", "Scarlet", 0xff2a50),
    lee("026", "Bright Red", 0xe0002a),
    lee("027", "Medium Red", 0xc20018),
    lee("035", "Light Pink", 0xffc4d6),
    lee("036", "Medium Pink", 0xff9cc0),
    lee("046", "Dark Magenta", 0xd0007a),
    lee("053", "Paler Lavender", 0xe6e6ff),
    lee("058", "Lavender", 0x9a6cff),
    lee("061", "Mist Blue", 0xdcefff),
    lee("063", "Pale Blue", 0xc4e2ff),
    lee("068", "Sky Blue", 0x3a8cff),
    lee("079", "Just Blue", 0x1a4cff),
    lee("085", "Deeper Blue", 0x0028c8),
    lee("101", "Yellow", 0xffe600),
    lee("103", "Straw", 0xfff0b4),
    lee("104", "Deep Amber", 0xffb400),
    lee("105", "Orange", 0xff8c00),
    lee("106", "Primary Red", 0xf00000),
    lee("110", "Middle Rose", 0xff8cc8),
    lee("115", "Peacock Blue", 0x00c8c8),
    lee("116", "Medium Blue-Green", 0x00a08c),
    lee("117", "Steel Blue", 0xb4e6f0),
    lee("119", "Dark Blue", 0x0a1ec8),
    lee("120", "Deep Blue", 0x0014a0),
    lee("124", "Dark Green", 0x00a03c),
    lee("126", "Mauve", 0xb4148c),
    lee("128", "Bright Pink", 0xff2896),
    lee("132", "Medium Blue", 0x0064ff),
    lee("134", "Golden Amber", 0xff8c50),
    lee("136", "Pale Lavender", 0xe0c8ff),
    lee("139", "Primary Green", 0x00b400),
    lee("141", "Bright Blue", 0x00a0ff),
    lee("147", "Apricot", 0xffa070),
    lee("151", "Gold Tint", 0xffd8c8),
    lee("152", "Pale Gold", 0xffd8b0),
    lee("154", "Pale Rose", 0xffd2d2),
    lee("158", "Deep Orange", 0xff7800),
    lee("161", "Slate Blue", 0x78b4ff),
    lee("164", "Flame Red", 0xff3200),
    lee("166", "Pale Red", 0xff7878),
    lee("170", "Deep Lavender", 0xc896ff),
    lee("179", "Chrome Orange", 0xffa000),
    lee("180", "Dark Lavender", 0x6e46ff),
    lee("181", "Congo Blue", 0x2800a0),
    lee("195", "Zenith Blue", 0x1428e6),
    lee("200", "Double C.T. Blue", 0x8cb4ff),
    lee("201", "Full C.T. Blue", 0xb4d2ff),
    lee("202", "Half C.T. Blue", 0xd6e6ff),
    lee("203", "Quarter C.T. Blue", 0xebf2ff),
    lee("204", "Full C.T. Orange", 0xffbe78),
    lee("205", "Half C.T. Orange", 0xffd8a8),
    lee("206", "Quarter C.T. Orange", 0xffead0),
    lee("711", "Cold Blue", 0x96bedc),
    lee("724", "Ocean Blue", 0x00b4dc),
    rosco("R02", "Bastard Amber", 0xffc8a0),
    rosco("R08", "Pale Gold", 0xffd8a0),
    rosco("R09", "Pale Amber Gold", 0xffc890),
    rosco("R14", "Medium Straw", 0xffd264),
    rosco("R18", "Flame", 0xff7846),
    rosco("R19", "Fire", 0xff3c14),
    rosco("R21", "Golden Amber", 0xff8c3c),
    rosco("R23", "Orange", 0xff8214),
    rosco("R26", "Light Red", 0xf01428),
    rosco("R27", "Medium Red", 0xb40014),
    rosco("R33", "No Color Pink", 0xffc8d2),
    rosco("R39", "Skelton Exotic Sangria", 0xc8005a),
    rosco("R44", "Middle Rose", 0xff78b4),
    rosco("R51", "Surprise Pink", 0xe6c8f0),
    rosco("R54", "Special Lavender", 0xe6d2ff),
    rosco("R57", "Lavender", 0xb48cff),
    rosco("R60", "No Color Blue", 0xd2e6ff),
    rosco("R62", "Booster Blue", 0xb4d2ff),
    rosco("R64", "Light Steel Blue", 0x78b4ff),
    rosco("R68", "Parry Sky Blue", 0x508cff),
    rosco("R74", "Night Blue", 0x1e1eb4),
    rosco("R80", "Primary Blue", 0x0050ff),
    rosco("R83", "Medium Blue", 0x0028c8),
    rosco("R89", "Moss Green", 0x50c850),
    rosco("R90", "Dark Yellow Green", 0x1e8c1e),
    rosco("R91", "Primary Green", 0x00a03c),
    rosco("R95", "Medium Blue Green", 0x00aaaa),
    rosco("R3202", "Full Blue (CTB)", 0xb4d2ff),
    rosco("R3204", "Half Blue (1/2 CTB)", 0xd6e6ff),
    rosco("R3407", "Roscosun CTO", 0xffbe78),
];

/// Gélatine par marque et référence, sans tenir compte de la casse : (Lee, « 201 »), (Rosco, « r80 »).
pub fn find(brand: Brand, code: &str) -> Option<&'static Gel> {
    let code = code.trim();
    GELS.iter().find(|g| g.brand == brand && g.code.eq_ignore_ascii_case(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_are_unique_and_found() {
        for (i, gel) in GELS.iter().enumerate() {
            assert!(!GELS[..i].iter().any(|g| g.brand == gel.brand && g.code == gel.code), "{} en double", gel.label());
        }
        assert_eq!(find(Brand::Lee, "201").unwrap().name, "Full C.T. Blue");
        assert_eq!(find(Brand::Rosco, "r80").unwrap().rgb(), [0.0, 80.0 / 255.0, 1.0]);
        assert!(find(Brand::Rosco, "201").is_none());
        assert!(GELS[0].matches("rose") && GELS[0].matches("002") && !GELS[0].matches("blue"));
    }
}

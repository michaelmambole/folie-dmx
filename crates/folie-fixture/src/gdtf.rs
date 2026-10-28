//! Import GDTF : la fiche (archive `.gdtf`, `description.xml` dedans) est convertie en
//! fiche Open Fixture Library, le format de la bibliothèque. Elle se lit et se modifie
//! ensuite comme n'importe quelle autre fiche.
//!
//! Gardé : modes, canaux (fins compris), plages nommées, roues (couleurs et gobos),
//! angles de pan/tilt, valeurs par défaut, cellules répétées (GeometryReference).
//! Laissé de côté : modèles 3D, images de gobos, canaux virtuels, univers secondaires.

use std::collections::BTreeMap;
use std::io::Read;

use serde_json::{Map, Value, json};

/// Une fiche GDTF convertie.
#[derive(Debug, Clone)]
pub struct Converted {
    pub manufacturer: String,
    pub name: String,
    /// Fiche OFL, prête pour [`crate::library::install`].
    pub ofl_json: String,
    /// Ce qui n'a pas pu être repris.
    pub warnings: Vec<String>,
}

/// Lit une archive `.gdtf`.
pub fn convert_archive(bytes: &[u8]) -> Result<Converted, String> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("archive GDTF illisible : {e}"))?;
    let mut file = archive.by_name("description.xml").map_err(|_| "description.xml absent de l'archive".to_string())?;
    let mut xml = String::new();
    file.read_to_string(&mut xml).map_err(|e| format!("description.xml illisible : {e}"))?;
    convert_xml(&xml)
}

type Node<'a, 'i> = roxmltree::Node<'a, 'i>;

/// Lit le `description.xml` d'une fiche GDTF.
pub fn convert_xml(xml: &str) -> Result<Converted, String> {
    let doc = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}')).map_err(|e| format!("XML invalide : {e}"))?;
    let fixture = doc
        .descendants()
        .find(|n| n.has_tag_name("FixtureType"))
        .ok_or("pas d'élément FixtureType : est-ce bien une fiche GDTF ?")?;
    let attr = |n: Node, name: &str| n.attribute(name).unwrap_or_default().trim().to_string();
    let manufacturer = attr(fixture, "Manufacturer");
    let name = Some(attr(fixture, "LongName")).filter(|s| !s.is_empty()).unwrap_or_else(|| attr(fixture, "Name"));
    let short_name = attr(fixture, "ShortName");

    let wheels: BTreeMap<String, Vec<Slot>> = child(fixture, "Wheels")
        .map(|w| {
            elements(w, "Wheel")
                .map(|wheel| {
                    let slots = elements(wheel, "Slot")
                        .map(|s| Slot { name: attr(s, "Name"), color: s.attribute("Color").and_then(cie_to_hex) })
                        .collect();
                    (attr(wheel, "Name"), slots)
                })
                .collect()
        })
        .unwrap_or_default();

    let geometries = child(fixture, "Geometries");
    let mut warnings = Vec::new();
    let mut available = Map::new();
    let mut templates = Map::new();
    let mut pixel_keys: Vec<String> = Vec::new();
    let mut modes = Vec::new();
    let mut has_pan = false;

    let dmx_modes = child(fixture, "DMXModes").ok_or("aucun mode DMX")?;
    for mode in elements(dmx_modes, "DMXMode") {
        let mode_name = attr(mode, "Name");
        // Cellules répétées : géométrie de référence -> (nom de la cellule, décalage DMX).
        let mut references: BTreeMap<String, Vec<(String, u32)>> = BTreeMap::new();
        if let Some(top) = geometries.and_then(|g| g.descendants().find(|n| n.is_element() && n.attribute("Name") == mode.attribute("Geometry"))) {
            for reference in top.descendants().filter(|n| n.has_tag_name("GeometryReference")) {
                let offset = elements(reference, "Break")
                    .find(|b| b.attribute("DMXBreak").unwrap_or("1") == "1")
                    .and_then(|b| b.attribute("DMXOffset")?.parse().ok())
                    .unwrap_or(1);
                references.entry(attr(reference, "Geometry")).or_default().push((attr(reference, "Name"), offset));
            }
        }

        let mut slots: BTreeMap<u32, Value> = BTreeMap::new();
        let Some(channels) = child(mode, "DMXChannels") else { continue };
        for channel in elements(channels, "DMXChannel") {
            let offsets: Vec<u32> = match channel.attribute("Offset") {
                Some(o) if o != "None" && !o.is_empty() => o.split(',').filter_map(|v| v.trim().parse().ok()).collect(),
                _ => continue, // canal virtuel : rien sur la ligne DMX
            };
            if offsets.is_empty() {
                continue;
            }
            if channel.attribute("DMXBreak").unwrap_or("1") != "1" {
                warnings.push(format!("{mode_name} : canaux d'un second univers ignorés"));
                continue;
            }
            let Some(logical) = child(channel, "LogicalChannel") else { continue };
            let attribute = attr(logical, "Attribute");
            has_pan |= attribute == "Pan";
            let geometry = attr(channel, "Geometry");
            let definition = channel_definition(channel, logical, offsets.len(), &wheels);
            let base = pretty(&attribute);

            match references.get(&geometry) {
                // Canal répété dans chaque cellule : canal modèle « Dimmer $pixelKey ».
                Some(cells) => {
                    let template = format!("{base} $pixelKey");
                    let fine: Vec<String> = (1..offsets.len()).map(|i| fine_name(&template, i)).collect();
                    let mut definition = definition;
                    if !fine.is_empty() {
                        definition["fineChannelAliases"] = json!(fine);
                    }
                    templates.insert(template.clone(), definition);
                    for (cell, start) in cells {
                        if !pixel_keys.contains(cell) {
                            pixel_keys.push(cell.clone());
                        }
                        let concrete = template.replace("$pixelKey", cell);
                        for (level, offset) in offsets.iter().enumerate() {
                            let name = if level == 0 { concrete.clone() } else { fine_name(&concrete, level) };
                            slots.insert(offset + start - 1, json!(name));
                        }
                    }
                }
                None => {
                    let name = unique_name(&mut available, &base, &geometry, &mode_name, definition, offsets.len());
                    for (level, offset) in offsets.iter().enumerate() {
                        let alias = if level == 0 { name.clone() } else { fine_name(&name, level) };
                        slots.insert(*offset, json!(alias));
                    }
                }
            }
        }
        let Some(&last) = slots.keys().max() else {
            warnings.push(format!("mode {mode_name} sans canal"));
            continue;
        };
        let channels: Vec<Value> = (1..=last).map(|i| slots.remove(&i).unwrap_or(Value::Null)).collect();
        modes.push(json!({ "name": mode_name, "channels": channels }));
    }
    if modes.is_empty() {
        return Err("aucun mode utilisable".into());
    }

    let mut ofl = json!({
        "$schema": "https://raw.githubusercontent.com/OpenLightingProject/open-fixture-library/master/schemas/fixture.json",
        "name": name,
        "categories": [if has_pan { "Moving Head" } else if pixel_keys.len() > 1 { "Pixel Bar" } else { "Color Changer" }],
        "meta": { "authors": ["Folie-DMX (import GDTF)"] },
        "availableChannels": available,
        "modes": modes,
    });
    if !short_name.is_empty() {
        ofl["shortName"] = json!(short_name);
    }
    if !templates.is_empty() {
        ofl["templateChannels"] = Value::Object(templates);
        ofl["matrix"] = json!({ "pixelKeys": [[pixel_keys]] });
    }
    warnings.dedup();
    let ofl_json = serde_json::to_string_pretty(&ofl).map_err(|e| e.to_string())?;
    Ok(Converted { manufacturer, name, ofl_json, warnings })
}

struct Slot {
    name: String,
    color: Option<String>,
}

fn child<'a, 'i>(node: Node<'a, 'i>, tag: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|n| n.has_tag_name(tag))
}

fn elements<'a, 'i: 'a>(node: Node<'a, 'i>, tag: &'a str) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    node.children().filter(move |n| n.has_tag_name(tag))
}

/// "Dimmer" (OFL, nom de canal) pour un attribut GDTF.
fn pretty(attribute: &str) -> String {
    let named = match attribute {
        "ColorAdd_R" | "ColorRGB_Red" => "Red",
        "ColorAdd_G" | "ColorRGB_Green" => "Green",
        "ColorAdd_B" | "ColorRGB_Blue" => "Blue",
        "ColorAdd_W" => "White",
        "ColorAdd_WW" => "Warm White",
        "ColorAdd_CW" => "Cold White",
        "ColorAdd_RY" | "ColorAdd_A" => "Amber",
        "ColorAdd_UV" => "UV",
        "ColorAdd_GY" => "Lime",
        "ColorAdd_C" | "ColorSub_C" => "Cyan",
        "ColorAdd_M" | "ColorSub_M" => "Magenta",
        "ColorAdd_Y" | "ColorSub_Y" => "Yellow",
        "Shutter1" => "Shutter",
        "Focus1" => "Focus",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_string();
    }
    // "Gobo1PosRotate" -> "Gobo 1 Pos Rotate", "Color1" -> "Color Wheel 1".
    let mut words = String::new();
    let mut previous: Option<char> = None;
    for c in attribute.chars() {
        let boundary = match previous {
            Some(p) => (p.is_ascii_lowercase() && (c.is_ascii_uppercase() || c.is_ascii_digit()))
                || (p.is_ascii_digit() && c.is_ascii_alphabetic())
                || (p.is_ascii_uppercase() && c.is_ascii_digit()),
            None => false,
        };
        if c == '_' {
            words.push(' ');
        } else {
            if boundary {
                words.push(' ');
            }
            words.push(c);
        }
        previous = Some(c);
    }
    match words.strip_prefix("Color ") {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => format!("Color Wheel {n}"),
        _ => words,
    }
}

/// Nom de fonction sans information : « Dimmer 1 », « ColorAdd_R 1 », vide…
fn plain_name(name: &str, attribute: &str) -> bool {
    let name = name.trim();
    let rest = name.strip_prefix(attribute).or_else(|| name.strip_prefix(pretty(attribute).as_str()));
    name.is_empty() || rest.is_some_and(|r| r.trim().chars().all(|c| c.is_ascii_digit()))
}

fn fine_name(name: &str, level: usize) -> String {
    if level == 1 { format!("{name} fine") } else { format!("{name} fine^{level}") }
}

/// Nom de canal unique dans la fiche : même nom pour la même définition dans plusieurs
/// modes, sinon on précise la géométrie puis le mode.
fn unique_name(available: &mut Map<String, Value>, base: &str, geometry: &str, mode: &str, mut definition: Value, bytes: usize) -> String {
    let candidates = [base.to_string(), format!("{base} ({mode})"), format!("{base} {geometry}")];
    for (i, name) in candidates.iter().enumerate() {
        if i == 2 && geometry.is_empty() {
            continue;
        }
        let fine: Vec<String> = (1..bytes).map(|l| fine_name(name, l)).collect();
        if !fine.is_empty() {
            definition["fineChannelAliases"] = json!(fine);
        } else if let Some(map) = definition.as_object_mut() {
            map.remove("fineChannelAliases");
        }
        match available.get(name) {
            None => {
                available.insert(name.clone(), definition);
                return name.clone();
            }
            Some(existing) if *existing == definition => return name.clone(),
            Some(_) => {}
        }
    }
    let mut n = 2;
    loop {
        let name = format!("{base} ({mode}) {n}");
        if !available.contains_key(&name) {
            available.insert(name.clone(), definition);
            return name;
        }
        n += 1;
    }
}

/// "128/1" ou "32768/2" ramené à la résolution du canal (`bytes` octets).
fn dmx_value(text: Option<&str>, bytes: usize) -> Option<u32> {
    let text = text?;
    let (value, size) = match text.split_once('/') {
        Some((v, n)) => (v.trim().parse::<u64>().ok()?, n.trim().parse::<usize>().ok()?),
        None => (text.trim().parse::<u64>().ok()?, 1),
    };
    let shifted = if size >= bytes { value >> (8 * (size - bytes)) } else { value << (8 * (bytes - size)) };
    Some(shifted.min((1u64 << (8 * bytes)) - 1) as u32)
}

/// Attributs continus : leurs ChannelSets (repères comme « Home ») ne coupent pas la plage.
fn continuous(attribute: &str) -> bool {
    attribute == "Dimmer"
        || attribute == "Pan"
        || attribute == "Tilt"
        || attribute.starts_with("ColorAdd_")
        || attribute.starts_with("ColorSub_")
        || attribute.starts_with("ColorRGB_")
        || attribute.starts_with("CT")
        || attribute == "Zoom"
        || attribute.starts_with("Focus")
        || attribute == "Iris"
}

/// Une plage : (début, nom de la fonction ou du repère, fonction, repère éventuel).
struct Range<'a, 'i> {
    from: u32,
    name: String,
    function: Node<'a, 'i>,
    set: Option<Node<'a, 'i>>,
    /// Le canal a plusieurs fonctions : le nom de la fonction précise le repère.
    shared: bool,
}

fn channel_definition(channel: Node, logical: Node, bytes: usize, wheels: &BTreeMap<String, Vec<Slot>>) -> Value {
    let max = (1u64 << (8 * bytes)) as u32 - 1;
    let functions: Vec<Node> = elements(logical, "ChannelFunction").collect();

    // Défaut : celui de la fonction initiale (GDTF 1.2), sinon celui du canal (1.0/1.1).
    let initial = channel.attribute("InitialFunction").and_then(|f| f.rsplit('.').next());
    let default = functions
        .iter()
        .find(|f| f.attribute("Name") == initial)
        .or(functions.first())
        .and_then(|f| dmx_value(f.attribute("Default"), bytes))
        .or_else(|| dmx_value(channel.attribute("Default"), bytes))
        .unwrap_or(0);

    let mut ranges: Vec<Range> = Vec::new();
    for function in &functions {
        let attribute = function.attribute("Attribute").unwrap_or_default();
        let from = dmx_value(function.attribute("DMXFrom"), bytes).unwrap_or(0);
        let name = function.attribute("Name").unwrap_or_default().trim().to_string();
        let sets: Vec<Node> = elements(*function, "ChannelSet").collect();
        if sets.is_empty() || continuous(attribute) {
            ranges.push(Range { from, name, function: *function, set: None, shared: functions.len() > 1 });
        } else {
            for set in sets {
                let set_from = dmx_value(set.attribute("DMXFrom"), bytes).unwrap_or(from);
                let set_name = set.attribute("Name").unwrap_or_default().trim();
                let name = if set_name.is_empty() { name.clone() } else { set_name.to_string() };
                ranges.push(Range { from: set_from, name, function: *function, set: Some(set), shared: functions.len() > 1 });
            }
        }
    }
    ranges.sort_by_key(|r| r.from);
    ranges.dedup_by_key(|r| r.from);

    let mut capabilities = Vec::new();
    for (i, range) in ranges.iter().enumerate() {
        let end = ranges.get(i + 1).map_or(max, |next| next.from.saturating_sub(1));
        if end < range.from {
            continue;
        }
        let mut capability = capability(range, wheels);
        capability["dmxRange"] = json!([range.from, end]);
        capabilities.push(capability);
    }
    if capabilities.is_empty() {
        capabilities.push(json!({ "dmxRange": [0, max], "type": "NoFunction" }));
    }

    let mut definition = json!({ "defaultValue": default, "capabilities": capabilities });
    if bytes > 1 {
        definition["dmxValueResolution"] = json!(format!("{}bit", 8 * bytes));
    }
    definition
}

fn capability(range: &Range, wheels: &BTreeMap<String, Vec<Slot>>) -> Value {
    let function = range.function;
    let attribute = function.attribute("Attribute").unwrap_or_default();
    let name = range.name.as_str();
    let physical = |key: &str| function.attribute(key).and_then(|v| v.trim().parse::<f32>().ok());
    let mut cap = Map::new();
    let kind = match attribute {
        "NoFeature" => "NoFunction",
        "Dimmer" => "Intensity",
        "Pan" | "Tilt" => {
            if let (Some(from), Some(to)) = (physical("PhysicalFrom"), physical("PhysicalTo"))
                && from != to
            {
                cap.insert("angleStart".into(), json!(format!("{from}deg")));
                cap.insert("angleEnd".into(), json!(format!("{to}deg")));
            }
            attribute
        }
        a if a.starts_with("PanTilt") => "PanTiltSpeed",
        a if a.starts_with("ColorAdd_") || a.starts_with("ColorSub_") || a.starts_with("ColorRGB_") => {
            cap.insert("color".into(), json!(pretty(a)));
            "ColorIntensity"
        }
        a if a.starts_with("CT") || a == "ColorTemperature" => "ColorTemperature",
        a if a.starts_with("Color") && a[5..].chars().all(|c| c.is_ascii_digit()) && !a[5..].is_empty() => {
            match wheel_slot(range, wheels) {
                Some(slot) => {
                    if let Some(color) = &slot.color {
                        cap.insert("colors".into(), json!([color]));
                    }
                    "ColorPreset"
                }
                None => "Effect",
            }
        }
        a if a.starts_with("Gobo") => {
            let rest = a.trim_start_matches("Gobo").trim_start_matches(|c: char| c.is_ascii_digit());
            if rest.starts_with("Pos") {
                "WheelSlotRotation"
            } else if rest.starts_with("WheelSpin") || rest.starts_with("WheelShake") || rest.starts_with("WheelAudio") {
                "WheelRotation"
            } else {
                if let Some(index) = range.set.and_then(|s| s.attribute("WheelSlotIndex")?.parse::<u32>().ok()).filter(|i| *i > 0) {
                    cap.insert("slotNumber".into(), json!(index));
                }
                "WheelSlot"
            }
        }
        a if a.starts_with("Prism") => "Prism",
        "Zoom" => "Zoom",
        a if a.starts_with("Focus") => "Focus",
        "Iris" => "Iris",
        a if a.starts_with("Shutter") || a.starts_with("Strobe") => {
            let lower = name.to_lowercase();
            let effect = if lower.contains("open") && !lower.contains("close") {
                "Open"
            } else if lower.contains("close") && !lower.contains("open") {
                "Closed"
            } else if a.contains("Pulse") || lower.contains("pulse") {
                "Pulse"
            } else {
                "Strobe"
            };
            cap.insert("shutterEffect".into(), json!(effect));
            if a.contains("Random") || lower.contains("random") {
                cap.insert("randomTiming".into(), json!(true));
            }
            "ShutterStrobe"
        }
        a if a.starts_with("Frost") || a.starts_with("Effects") || a.starts_with("Animation") || a.starts_with("Video") => "Effect",
        a if a.starts_with("Control") || a.starts_with("Reset") || a.starts_with("Lamp") || a.starts_with("Fan") || a.starts_with("Display") || a.starts_with("DimmerCurve") || a.starts_with("Function") => "Maintenance",
        _ => "Generic",
    };
    cap.insert("type".into(), json!(kind));
    let function_name = function.attribute("Name").unwrap_or_default().trim();
    // Le nom d'un repère (« Gobo 1 », « Home ») compte ; celui d'une fonction seule souvent pas.
    let plain = if range.set.is_some() { name.is_empty() } else { plain_name(name, attribute) };
    let mut comment = if plain { String::new() } else { name.to_string() };
    // « Open », « Strobe »… : déjà dit par le type de la plage.
    if kind == "ShutterStrobe" && ["open", "closed", "close", "strobe"].contains(&comment.to_lowercase().as_str()) {
        comment.clear();
    }
    if range.shared
        && range.set.is_some()
        && kind != "ShutterStrobe"
        && !plain_name(function_name, attribute)
        && function_name != name
    {
        comment = if comment.is_empty() { function_name.to_string() } else { format!("{comment} ({function_name})") };
    }
    if kind == "Generic" {
        comment = if comment.is_empty() { pretty(attribute) } else { format!("{} — {comment}", pretty(attribute)) };
    }
    if !comment.is_empty() {
        cap.insert("comment".into(), json!(comment));
    }
    Value::Object(cap)
}

fn wheel_slot<'w>(range: &Range, wheels: &'w BTreeMap<String, Vec<Slot>>) -> Option<&'w Slot> {
    let wheel = wheels.get(range.function.attribute("Wheel")?)?;
    let index: usize = range.set?.attribute("WheelSlotIndex")?.parse().ok()?;
    wheel.get(index.checked_sub(1)?).filter(|slot| !slot.name.is_empty() || slot.color.is_some())
}

/// Couleur GDTF (CIE 1931 « x,y,Y ») en "#rrggbb", à pleine luminosité.
fn cie_to_hex(text: &str) -> Option<String> {
    let mut parts = text.split(',').map(|v| v.trim().parse::<f32>().ok());
    let (x, y) = (parts.next()??, parts.next()??);
    if y <= 0.0 {
        return None;
    }
    let (cx, cz) = (x / y, (1.0 - x - y) / y);
    let linear = [
        3.2406 * cx - 1.5372 - 0.4986 * cz,
        -0.9689 * cx + 1.8758 + 0.0415 * cz,
        0.0557 * cx - 0.2040 + 1.0570 * cz,
    ];
    let peak = linear.iter().cloned().fold(f32::MIN, f32::max).max(1e-6);
    let gamma = |c: f32| {
        let c = (c / peak).clamp(0.0, 1.0);
        let v = if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
        (v * 255.0).round() as u8
    };
    let [r, g, b] = linear.map(gamma);
    Some(format!("#{r:02x}{g:02x}{b:02x}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Attribute, CapabilityKind, ofl};

    fn sample(name: &str) -> Converted {
        let path = format!("{}/testdata/gdtf/BlenderDMX@{name}.gdtf", env!("CARGO_MANIFEST_DIR"));
        convert_archive(&std::fs::read(path).unwrap()).unwrap()
    }

    fn parsed(c: &Converted) -> crate::FixtureType {
        ofl::parse(&c.ofl_json, "essai/fiche", &c.manufacturer).unwrap()
    }

    #[test]
    fn moving_head_with_gobo_wheels() {
        let converted = sample("BeskydBeam@version10");
        assert_eq!((converted.manufacturer.as_str(), converted.name.as_str()), ("BlenderDMX", "BeskydBeam"));
        assert!(converted.warnings.is_empty(), "{:?}", converted.warnings);
        let fixture = parsed(&converted);
        let mode = &fixture.modes[0];
        assert_eq!(mode.footprint, 17);
        let param = |name: &str| mode.parameters.iter().find(|p| p.name == name).unwrap();
        // Pan 16 bits, au milieu par défaut, avec son amplitude.
        let pan = param("Pan");
        assert_eq!((pan.attribute.clone(), pan.offsets.clone(), pan.default), (Attribute::Pan, vec![1, 2], 32768));
        assert!(pan.capabilities[0].angle.is_some());
        // Roue de gobos : une plage par gobo, nommée.
        let gobo = param("Gobo 1");
        assert_eq!(gobo.attribute, Attribute::Gobo);
        assert!(gobo.capabilities.iter().any(|c| c.label.contains("Gobo 3")));
        assert_eq!(param("Red").default, 255 << 8, "rouge à fond par défaut");
        let shutter: Vec<&str> = param("Shutter").capabilities.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(shutter, ["Fermé", "Ouvert", "Strobe", "Ouvert"]);
    }

    #[test]
    fn several_modes_and_resolutions() {
        let fixture = parsed(&sample("AlpineTubeRGB@version5"));
        let footprints: Vec<u16> = fixture.modes.iter().map(|m| m.footprint).collect();
        assert_eq!(footprints, [3, 6, 9]);
        let red = |mode: usize| fixture.modes[mode].parameters[0].offsets.clone();
        assert_eq!((red(0), red(1)), (vec![0], vec![0, 1]), "8 bits puis 16 bits");
        let rgba = parsed(&sample("RysyParLED@version7"));
        assert_eq!(rgba.modes[1].parameters[4].attribute, Attribute::ColorMix("Amber".into()));
    }

    /// Fiche écrite pour l'essai : 4 cellules répétées (GeometryReference) et une roue de couleurs.
    const BAR: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GDTF DataVersion="1.2"><FixtureType Name="Barre" LongName="Barre 4" ShortName="B4" Manufacturer="Essai Lumière">
  <Wheels><Wheel Name="Couleurs"><Slot Name="Blanc" Color="0.3127,0.3290,100"/><Slot Name="Rouge" Color="0.64,0.33,21"/></Wheel></Wheels>
  <Geometries>
    <Geometry Name="Corps"><GeometryReference Name="Pixel 1" Geometry="Pixel"><Break DMXBreak="1" DMXOffset="2"/></GeometryReference>
      <GeometryReference Name="Pixel 2" Geometry="Pixel"><Break DMXBreak="1" DMXOffset="3"/></GeometryReference>
      <GeometryReference Name="Pixel 3" Geometry="Pixel"><Break DMXBreak="1" DMXOffset="4"/></GeometryReference>
      <GeometryReference Name="Pixel 4" Geometry="Pixel"><Break DMXBreak="1" DMXOffset="5"/></GeometryReference></Geometry>
    <Geometry Name="Pixel"/>
  </Geometries>
  <DMXModes><DMXMode Name="Pixels" Geometry="Corps"><DMXChannels>
    <DMXChannel DMXBreak="1" Offset="1" Geometry="Corps" InitialFunction="Corps_Color1.Color1.Roue"><LogicalChannel Attribute="Color1">
      <ChannelFunction Name="Roue" Attribute="Color1" DMXFrom="0/1" Default="0/1" Wheel="Couleurs">
        <ChannelSet Name="Blanc" DMXFrom="0/1" WheelSlotIndex="1"/><ChannelSet Name="Rouge" DMXFrom="128/1" WheelSlotIndex="2"/>
      </ChannelFunction></LogicalChannel></DMXChannel>
    <DMXChannel DMXBreak="1" Offset="1" Geometry="Pixel" InitialFunction="Pixel_Dimmer.Dimmer.Dimmer"><LogicalChannel Attribute="Dimmer">
      <ChannelFunction Name="Dimmer" Attribute="Dimmer" DMXFrom="0/1" Default="0/1"/></LogicalChannel></DMXChannel>
    <DMXChannel DMXBreak="1" Offset="None" Geometry="Corps"><LogicalChannel Attribute="Dimmer"><ChannelFunction Attribute="Dimmer" DMXFrom="0/1"/></LogicalChannel></DMXChannel>
  </DMXChannels></DMXMode></DMXModes>
</FixtureType></GDTF>"#;

    #[test]
    fn repeated_cells_become_heads_and_wheel_colors_are_kept() {
        let converted = convert_xml(BAR).unwrap();
        assert_eq!((converted.manufacturer.as_str(), converted.name.as_str()), ("Essai Lumière", "Barre 4"));
        let fixture = parsed(&converted);
        let mode = &fixture.modes[0];
        assert_eq!(mode.footprint, 5, "roue + 4 cellules, le canal virtuel ne compte pas");
        assert_eq!(mode.heads.len(), 4);
        let dimmers: Vec<(&str, Vec<u16>)> = mode
            .parameters
            .iter()
            .filter(|p| p.attribute == Attribute::Dimmer)
            .map(|p| (p.name.as_str(), p.offsets.clone()))
            .collect();
        assert_eq!(dimmers, [("Dimmer Pixel 1", vec![1]), ("Dimmer Pixel 2", vec![2]), ("Dimmer Pixel 3", vec![3]), ("Dimmer Pixel 4", vec![4])]);
        let wheel = &mode.parameters[0];
        let colors: Vec<[u8; 3]> = wheel
            .capabilities
            .iter()
            .filter_map(|c| match &c.kind {
                CapabilityKind::ColorPreset { colors } => colors.first().copied(),
                _ => None,
            })
            .collect();
        assert_eq!(colors.len(), 2);
        assert!(colors[0].iter().all(|&c| c > 240), "blanc D65 : {:?}", colors[0]);
        assert!(colors[1][0] > 200 && colors[1][1] < 80 && colors[1][2] < 80, "rouge : {:?}", colors[1]);
    }

    #[test]
    fn not_a_gdtf() {
        assert!(convert_xml("<Autre/>").unwrap_err().contains("FixtureType"));
        assert!(convert_archive(b"pas une archive").is_err());
    }
}

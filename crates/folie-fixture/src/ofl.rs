//! Import du format Open Fixture Library.
//!
//! Référence : <https://github.com/OpenLightingProject/open-fixture-library/blob/master/docs/fixture-format.md>.
//! Les matrices (appareils multi-instance) sont gérées en une dimension.
//! Un mode qui utilise une fonction non gérée (matrice 2D, canaux commutés…)
//! est écarté avec un avertissement ; les autres modes restent utilisables.

use std::collections::{BTreeMap, HashMap};

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

use crate::model::{Attribute, Capability, CapabilityKind, FixtureType, Mode, Parameter};

#[derive(Debug, Error)]
pub enum OflError {
    #[error("JSON invalide : {0}")]
    Json(#[from] serde_json::Error),
    #[error("mode « {mode} » : canal inconnu « {channel} »")]
    UnknownChannel { mode: String, channel: String },
    #[error("mode « {mode} » : canal fin « {channel} » sans son canal grossier")]
    FineWithoutCoarse { mode: String, channel: String },
    #[error("mode « {mode} » : {feature} pas encore géré")]
    Unsupported { mode: String, feature: &'static str },
    #[error("canal « {channel} » : {message}")]
    InvalidChannel { channel: String, message: String },
    #[error("aucun mode utilisable : {0}")]
    NoUsableMode(String),
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OflFixture {
    name: String,
    short_name: Option<String>,
    #[serde(default)]
    categories: Vec<String>,
    #[serde(default)]
    available_channels: BTreeMap<String, OflChannel>,
    #[serde(default)]
    template_channels: BTreeMap<String, OflChannel>,
    matrix: Option<OflMatrix>,
    modes: Vec<OflMode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OflMatrix {
    pixel_count: Option<[u32; 3]>,
    /// Clés par plan Z, ligne Y, colonne X ; `null` = pas de cellule.
    pixel_keys: Option<Vec<Vec<Vec<Option<String>>>>>,
    #[serde(default)]
    pixel_groups: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OflMatrixInsert {
    insert: String,
    repeat_for: Value,
    channel_order: String,
    template_channels: Vec<String>,
}

/// Cellules d'une matrice à une dimension, dans l'ordre, et groupes de cellules.
struct Pixels {
    keys: Vec<String>,
    groups: Vec<String>,
    one_dimensional: bool,
}

impl Pixels {
    fn new(matrix: Option<&OflMatrix>) -> Self {
        let Some(matrix) = matrix else {
            return Self { keys: Vec::new(), groups: Vec::new(), one_dimensional: true };
        };
        let (keys, one_dimensional) = match (&matrix.pixel_keys, matrix.pixel_count) {
            (Some(planes), _) => {
                let rows = planes.iter().map(Vec::len).max().unwrap_or(0);
                let keys = planes.iter().flatten().flatten().flatten().cloned().collect();
                (keys, planes.len() <= 1 && rows <= 1)
            }
            (None, Some([x, y, z])) => {
                // Clés générées par OFL : "1", "2"… sur la seule dimension utilisée.
                let used = [x, y, z].into_iter().filter(|n| *n > 1).count();
                let n = x * y * z;
                ((1..=n).map(|i| i.to_string()).collect(), used <= 1)
            }
            (None, None) => (Vec::new(), true),
        };
        Self { keys, groups: matrix.pixel_groups.keys().cloned().collect(), one_dimensional }
    }
}

/// Ce que désigne un nom de canal utilisé dans un mode.
struct ChannelRef<'a> {
    channel: &'a OflChannel,
    /// Nom du canal grossier correspondant (concret, clé de cellule incluse).
    coarse: String,
    /// Nom sans la clé de cellule : "Dimmer" pour "Dimmer 3".
    base: String,
    /// Cellule ou groupe de cellules pilotés, pour un canal de matrice.
    head: Option<String>,
    /// 0 = grossier, 1 = fin, 2 = ultra-fin…
    level: usize,
}

fn template_base(template: &str) -> String {
    template.replace("$pixelKey", "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Tous les noms de canaux concrets du fichier : canaux simples, canaux fins
/// et canaux de matrice générés pour chaque cellule et chaque groupe.
fn channel_refs<'a>(fixture: &'a OflFixture, pixels: &Pixels) -> HashMap<String, ChannelRef<'a>> {
    let mut refs = HashMap::new();
    let mut add = |name: &str, channel: &'a OflChannel, base: String, head: Option<String>| {
        refs.insert(
            name.to_string(),
            ChannelRef { channel, coarse: name.into(), base: base.clone(), head: head.clone(), level: 0 },
        );
        for (i, alias) in channel.fine_channel_aliases.iter().enumerate() {
            let alias = match &head {
                Some(key) => alias.replace("$pixelKey", key),
                None => alias.clone(),
            };
            refs.insert(
                alias,
                ChannelRef { channel, coarse: name.into(), base: base.clone(), head: head.clone(), level: i + 1 },
            );
        }
    };
    for (name, channel) in &fixture.available_channels {
        add(name, channel, name.clone(), None);
    }
    // Canaux commutés : on retient la fonction active quand le canal qui les
    // commande est à sa valeur par défaut. Les autres fonctions viendront plus tard.
    for (controller, switch, target) in default_switch_targets(fixture) {
        if let Some(channel) = fixture.available_channels.get(&target) {
            add(&switch, channel, format!("{switch} (selon {controller})"), None);
        }
    }
    for (template, channel) in &fixture.template_channels {
        for key in pixels.keys.iter().chain(&pixels.groups) {
            add(&template.replace("$pixelKey", key), channel, template_base(template), Some(key.clone()));
        }
    }
    refs
}

/// Pour chaque canal commuté : (canal qui le commande, canal commuté, canal réel actif par défaut).
fn default_switch_targets(fixture: &OflFixture) -> Vec<(String, String, String)> {
    let mut targets = Vec::new();
    for (controller_name, controller) in &fixture.available_channels {
        let Some(caps) = &controller.capabilities else { continue };
        let default = match &controller.default_value {
            Some(Value::Number(n)) => n.as_u64().unwrap_or(0) as u32,
            _ => 0,
        };
        // Plage contenant la valeur par défaut, sinon la première.
        let active = caps
            .iter()
            .find(|c| c.dmx_range.is_some_and(|[a, b]| (a..=b).contains(&default)))
            .or(caps.first());
        let Some(Value::Object(switches)) = active.and_then(|c| c.extra.get("switchChannels")) else {
            continue;
        };
        for (switch, target) in switches {
            if let Some(target) = target.as_str() {
                targets.push((controller_name.clone(), switch.clone(), target.to_string()));
            }
        }
    }
    targets
}

/// Déroule une insertion de matrice en noms de canaux concrets.
fn expand_insert(insert: &OflMatrixInsert, pixels: &Pixels, mode: &str) -> Result<Vec<String>, OflError> {
    let unsupported = |feature| OflError::Unsupported { mode: mode.to_string(), feature };
    if insert.insert != "matrixChannels" {
        return Err(unsupported("ce type d'insertion"));
    }
    let keys: Vec<String> = match &insert.repeat_for {
        Value::String(order) if order.starts_with("eachPixel") && order != "eachPixelGroup" => {
            if !pixels.one_dimensional {
                return Err(unsupported("la matrice en 2D ou 3D"));
            }
            let mut keys = pixels.keys.clone();
            if order == "eachPixelABC" {
                keys.sort();
            }
            keys
        }
        Value::String(order) if order == "eachPixelGroup" => pixels.groups.clone(),
        Value::Array(list) => list.iter().filter_map(|k| k.as_str().map(String::from)).collect(),
        _ => return Err(unsupported("cet ordre de répétition")),
    };
    let concrete = |template: &String, key: &String| template.replace("$pixelKey", key);
    Ok(match insert.channel_order.as_str() {
        "perPixel" => keys
            .iter()
            .flat_map(|k| insert.template_channels.iter().map(move |t| concrete(t, k)))
            .collect(),
        "perChannel" => insert
            .template_channels
            .iter()
            .flat_map(|t| keys.iter().map(move |k| concrete(t, k)))
            .collect(),
        _ => return Err(unsupported("cet ordre de canaux")),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OflChannel {
    #[serde(default)]
    fine_channel_aliases: Vec<String>,
    dmx_value_resolution: Option<String>,
    default_value: Option<Value>,
    capability: Option<OflCapability>,
    capabilities: Option<Vec<OflCapability>>,
    /// Extension propre à Folie (ignorée par OFL) : canaux à mettre à 0 quand celui-ci sert.
    #[serde(default, rename = "folieTurnsOff")]
    turns_off: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OflCapability {
    dmx_range: Option<[u32; 2]>,
    #[serde(rename = "type")]
    kind: String,
    comment: Option<String>,
    menu_click: Option<String>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OflMode {
    name: String,
    short_name: Option<String>,
    channels: Vec<Value>,
}

/// Convertit un fichier OFL. `key` et `manufacturer` viennent de l'emplacement du fichier.
pub fn parse(json: &str, key: &str, manufacturer: &str) -> Result<FixtureType, OflError> {
    let fixture: OflFixture = serde_json::from_str(json)?;
    let pixels = Pixels::new(fixture.matrix.as_ref());
    let refs = channel_refs(&fixture, &pixels);

    let mut modes = Vec::new();
    let mut warnings = Vec::new();
    for mode in &fixture.modes {
        match parse_mode(mode, &refs, &pixels) {
            Ok(mode) => modes.push(mode),
            Err(err) => warnings.push(err.to_string()),
        }
    }
    if modes.is_empty() {
        return Err(OflError::NoUsableMode(warnings.join(" ; ")));
    }

    Ok(FixtureType {
        key: key.to_string(),
        manufacturer: manufacturer.to_string(),
        short_name: fixture.short_name.unwrap_or_else(|| fixture.name.clone()),
        name: fixture.name,
        categories: fixture.categories,
        modes,
        warnings,
        ofl_source: json.to_string(),
    })
}

fn parse_mode(
    mode: &OflMode,
    refs: &HashMap<String, ChannelRef<'_>>,
    pixels: &Pixels,
) -> Result<Mode, OflError> {
    // Liste à plat des canaux du mode, insertions de matrice déroulées.
    let mut names: Vec<Option<String>> = Vec::new();
    for entry in &mode.channels {
        match entry {
            Value::Null => names.push(None), // canal inutilisé
            Value::String(name) => names.push(Some(name.clone())),
            Value::Object(_) => {
                let insert: OflMatrixInsert = serde_json::from_value(entry.clone())?;
                names.extend(expand_insert(&insert, pixels, &mode.name)?.into_iter().map(Some));
            }
            _ => {
                return Err(OflError::Unsupported { mode: mode.name.clone(), feature: "ce type d'entrée" });
            }
        }
    }

    // Canal grossier -> décalage de chaque niveau de résolution.
    let mut offsets: BTreeMap<&str, Vec<Option<u16>>> = BTreeMap::new();
    for (offset, name) in names.iter().enumerate() {
        let Some(name) = name else { continue };
        let channel = refs
            .get(name)
            .ok_or_else(|| OflError::UnknownChannel { mode: mode.name.clone(), channel: name.clone() })?;
        let levels = offsets.entry(channel.coarse.as_str()).or_default();
        if levels.len() <= channel.level {
            levels.resize(channel.level + 1, None);
        }
        levels[channel.level] = Some(offset as u16);
    }

    let mut parameters = Vec::new();
    for (coarse, levels) in offsets {
        // Seuls les niveaux consécutifs depuis le grossier sont utilisables.
        let used: Vec<u16> = levels.iter().map_while(|o| *o).collect();
        if used.is_empty() {
            return Err(OflError::FineWithoutCoarse { mode: mode.name.clone(), channel: coarse.into() });
        }
        parameters.push((refs[coarse].head.clone(), parse_parameter(coarse, &refs[coarse], used)?));
    }
    parameters.sort_by_key(|(_, p)| p.offsets[0]);

    // Sous-fixtures dans l'ordre où elles apparaissent dans le mode.
    let mut heads: Vec<String> = Vec::new();
    let parameters = parameters
        .into_iter()
        .map(|(head, mut parameter)| {
            parameter.head = head.map(|key| match heads.iter().position(|h| *h == key) {
                Some(i) => i,
                None => {
                    heads.push(key);
                    heads.len() - 1
                }
            });
            parameter
        })
        .collect();

    Ok(Mode {
        name: mode.name.clone(),
        short_name: mode.short_name.clone().unwrap_or_else(|| mode.name.clone()),
        footprint: names.len() as u16,
        heads,
        parameters,
    })
}

fn parse_parameter(name: &str, reference: &ChannelRef<'_>, offsets: Vec<u16>) -> Result<Parameter, OflError> {
    let channel = reference.channel;
    let invalid = |message: String| OflError::InvalidChannel { channel: name.into(), message };

    let raw_caps: Vec<&OflCapability> = match (&channel.capability, &channel.capabilities) {
        (Some(cap), None) => vec![cap],
        (None, Some(caps)) if !caps.is_empty() => caps.iter().collect(),
        _ => return Err(invalid("ni « capability » ni « capabilities »".into())),
    };

    let resolution = value_resolution(channel, &raw_caps).map_err(invalid)?;
    let capabilities = raw_caps
        .iter()
        .map(|cap| {
            let range = match cap.dmx_range {
                Some([start, end]) => to_u16(start, resolution)..=to_u16_end(end, resolution),
                None => 0..=u16::MAX,
            };
            convert_capability(cap, range)
        })
        .collect::<Vec<_>>();

    let default = match &channel.default_value {
        None => 0,
        Some(Value::Number(n)) => to_u16(n.as_u64().unwrap_or(0) as u32, resolution),
        Some(Value::String(s)) => percent(s)
            .map(|p| (p * u16::MAX as f32).round() as u16)
            .ok_or_else(|| invalid(format!("valeur par défaut illisible : {s}")))?,
        Some(other) => return Err(invalid(format!("valeur par défaut illisible : {other}"))),
    };

    Ok(Parameter {
        name: name.to_string(),
        base_name: reference.base.clone(),
        head: None,
        attribute: attribute_for(&reference.base, &capabilities),
        offsets,
        default,
        capabilities,
        turns_off: channel.turns_off.clone(),
    })
}

/// Résolution (en bits) dans laquelle sont exprimées les plages et la valeur par défaut.
fn value_resolution(channel: &OflChannel, caps: &[&OflCapability]) -> Result<u32, String> {
    if let Some(res) = &channel.dmx_value_resolution {
        return match res.as_str() {
            "8bit" => Ok(8),
            "16bit" => Ok(16),
            "24bit" => Ok(24),
            other => Err(format!("résolution inconnue : {other}")),
        };
    }
    // Sans indication explicite, on déduit la résolution des valeurs elles-mêmes.
    let max_range = caps.iter().filter_map(|c| c.dmx_range).map(|[_, end]| end).max().unwrap_or(0);
    let max_default = channel.default_value.as_ref().and_then(Value::as_u64).unwrap_or(0) as u32;
    Ok(match max_range.max(max_default) {
        0..=255 => 8,
        256..=65_535 => 16,
        _ => 24,
    })
}

fn to_u16(value: u32, resolution: u32) -> u16 {
    match resolution {
        8 => (value.min(255) as u16) << 8,
        16 => value.min(65_535) as u16,
        _ => (value >> 8).min(65_535) as u16,
    }
}

/// Fin de plage : une valeur 8 bits couvre tous les pas fins qu'elle contient.
fn to_u16_end(value: u32, resolution: u32) -> u16 {
    match resolution {
        8 => to_u16(value, 8) | 0xFF,
        _ => to_u16(value, resolution),
    }
}

fn percent(s: &str) -> Option<f32> {
    let p: f32 = s.strip_suffix('%')?.trim().parse().ok()?;
    Some((p / 100.0).clamp(0.0, 1.0))
}

fn text(extra: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    extra.get(key).map(|v| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

fn angle(extra: &BTreeMap<String, Value>) -> Option<(f32, f32)> {
    let parse = |key| -> Option<f32> { text(extra, key)?.strip_suffix("deg")?.trim().parse().ok() };
    Some((parse("angleStart")?, parse("angleEnd")?))
}

fn hex_color(s: &str) -> Option<[u8; 3]> {
    let s = s.strip_prefix('#')?;
    let v = u32::from_str_radix(s, 16).ok().filter(|_| s.len() == 6)?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn speed_text(extra: &BTreeMap<String, Value>) -> Option<String> {
    let word = |s: String| match s.as_str() {
        "fast" => "rapide".to_string(),
        "slow" => "lent".to_string(),
        "stop" => "arrêt".to_string(),
        "fast reverse" => "rapide inverse".to_string(),
        "slow reverse" => "lent inverse".to_string(),
        _ => s,
    };
    match (text(extra, "speedStart"), text(extra, "speedEnd"), text(extra, "speed")) {
        (Some(a), Some(b), _) => Some(format!("{} à {}", word(a), word(b))),
        (_, _, Some(s)) => Some(word(s)),
        _ => None,
    }
}

fn color_name(color: &str) -> &str {
    match color {
        "Red" => "Rouge",
        "Green" => "Vert",
        "Blue" => "Bleu",
        "White" => "Blanc",
        "Amber" => "Ambre",
        "UV" => "UV",
        "Cyan" => "Cyan",
        "Magenta" => "Magenta",
        "Yellow" => "Jaune",
        "Lime" => "Lime",
        "Warm White" => "Blanc chaud",
        "Cold White" => "Blanc froid",
        other => other,
    }
}

fn convert_capability(cap: &OflCapability, range: std::ops::RangeInclusive<u16>) -> Capability {
    let extra = &cap.extra;
    let speed = speed_text(extra);
    let with_speed = |base: &str| match &speed {
        Some(s) => format!("{base} ({s})"),
        None => base.to_string(),
    };

    let (kind, base_label) = match cap.kind.as_str() {
        "NoFunction" => (CapabilityKind::NoFunction, "Aucune fonction".to_string()),
        "Intensity" => (CapabilityKind::Intensity, "Intensité".to_string()),
        "ShutterStrobe" => {
            let effect = text(extra, "shutterEffect").unwrap_or_default();
            let random = extra.get("randomTiming").and_then(Value::as_bool).unwrap_or(false);
            let mut label = match effect.as_str() {
                "Open" => "Ouvert".to_string(),
                "Closed" => "Fermé".to_string(),
                "Strobe" => "Strobe".to_string(),
                "Pulse" => "Pulse".to_string(),
                "RampUp" => "Pulse montant".to_string(),
                "RampDown" => "Pulse descendant".to_string(),
                "RampUpDown" => "Pulse montant/descendant".to_string(),
                "Lightning" => "Éclairs".to_string(),
                "Spikes" => "Pics".to_string(),
                "Burst" => "Rafales".to_string(),
                other => other.to_string(),
            };
            if random {
                label.push_str(" aléatoire");
            }
            (CapabilityKind::Shutter { effect, random }, with_speed(&label))
        }
        "ColorIntensity" => {
            let color = text(extra, "color").unwrap_or_default();
            let label = color_name(&color).to_string();
            (CapabilityKind::ColorIntensity(color), label)
        }
        "ColorPreset" => {
            let colors = extra
                .get("colors")
                .or_else(|| extra.get("colorsStart"))
                .and_then(Value::as_array)
                .map(|list| list.iter().filter_map(|c| c.as_str().and_then(hex_color)).collect())
                .unwrap_or_default();
            (CapabilityKind::ColorPreset { colors }, "Couleur".to_string())
        }
        "WheelSlot" => (CapabilityKind::Gobo, "Position de roue".to_string()),
        "ColorTemperature" => (CapabilityKind::ColorTemperature, "Température de couleur".into()),
        "Pan" => (CapabilityKind::Pan, "Pan".to_string()),
        "Tilt" => (CapabilityKind::Tilt, "Tilt".to_string()),
        "PanTiltSpeed" => (CapabilityKind::PanTiltSpeed, with_speed("Vitesse pan/tilt")),
        "Zoom" => (CapabilityKind::Zoom, "Zoom".to_string()),
        "Focus" => (CapabilityKind::Focus, "Focus".to_string()),
        "Iris" => (CapabilityKind::Iris, "Iris".to_string()),
        "Prism" => (CapabilityKind::Prism, with_speed("Prisme")),
        "Effect" => {
            let name = text(extra, "effectName")
                .or_else(|| text(extra, "effectPreset"))
                .unwrap_or_else(|| "Effet".into());
            (CapabilityKind::Effect, with_speed(&name))
        }
        "EffectSpeed" => (CapabilityKind::Effect, with_speed("Vitesse d'effet")),
        "EffectDuration" => {
            let duration = match (text(extra, "durationStart"), text(extra, "durationEnd")) {
                (Some(a), Some(b)) => format!("Durée d'effet ({a} à {b})"),
                _ => "Durée d'effet".to_string(),
            };
            (CapabilityKind::Effect, duration)
        }
        "Maintenance" => (CapabilityKind::Maintenance, "Maintenance".to_string()),
        other => (CapabilityKind::Other(other.to_string()), other.to_string()),
    };

    // Le commentaire du fichier est plus précis que notre libellé générique pour ces types.
    let label = match (&cap.comment, &kind) {
        (
            Some(comment),
            CapabilityKind::NoFunction
            | CapabilityKind::ColorPreset { .. }
            | CapabilityKind::Maintenance
            | CapabilityKind::PanTiltSpeed,
        ) => comment.clone(),
        (Some(comment), _) => format!("{base_label} — {comment}"),
        (None, _) => base_label,
    };

    Capability {
        range,
        kind,
        label,
        hidden: cap.menu_click.as_deref() == Some("hidden"),
        angle: angle(extra),
    }
}

/// Attribut d'un canal : le type de plage le plus représenté, hors « aucune fonction ».
fn attribute_for(name: &str, capabilities: &[Capability]) -> Attribute {
    let mut counts: Vec<(Attribute, usize)> = Vec::new();
    for cap in capabilities {
        let attribute = match &cap.kind {
            CapabilityKind::NoFunction => continue,
            CapabilityKind::Intensity => Attribute::Dimmer,
            CapabilityKind::Shutter { .. } => Attribute::Shutter,
            CapabilityKind::ColorIntensity(color) => Attribute::ColorMix(color.clone()),
            CapabilityKind::ColorPreset { .. } => Attribute::ColorMacro,
            CapabilityKind::ColorTemperature => Attribute::ColorTemperature,
            CapabilityKind::Pan => Attribute::Pan,
            CapabilityKind::Tilt => Attribute::Tilt,
            CapabilityKind::PanTiltSpeed => Attribute::PanTiltSpeed,
            CapabilityKind::Zoom => Attribute::Zoom,
            CapabilityKind::Focus => Attribute::Focus,
            CapabilityKind::Iris => Attribute::Iris,
            CapabilityKind::Gobo => Attribute::Gobo,
            CapabilityKind::Prism => Attribute::Prism,
            CapabilityKind::Effect => Attribute::Effect,
            CapabilityKind::Maintenance => Attribute::Control,
            CapabilityKind::Other(_) => Attribute::Other(name.to_string()),
        };
        match counts.iter_mut().find(|(a, _)| *a == attribute) {
            Some((_, n)) => *n += 1,
            None => counts.push((attribute, 1)),
        }
    }
    // max_by_key garde le dernier ex aequo : on inverse pour garder le premier.
    counts
        .into_iter()
        .rev()
        .max_by_key(|(_, n)| *n)
        .map(|(a, _)| a)
        .unwrap_or_else(|| Attribute::Other(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::FeatureGroup;

    fn library_file(path: &str) -> String {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../library/");
        std::fs::read_to_string(format!("{root}{path}")).unwrap()
    }

    #[test]
    fn rush_mh6_wash_14_channels() {
        let json = library_file("martin/rush-mh-6-wash.json");
        let fixture = parse(&json, "martin/rush-mh-6-wash", "Martin").unwrap();
        let mode = &fixture.modes[0];
        assert_eq!(mode.footprint, 14);
        assert_eq!(mode.parameters.len(), 11);

        let names: Vec<_> = mode.parameters.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Shutter", "Dimmer", "Red", "Green", "Blue", "White", "Color Wheel", "Zoom", "Pan",
                "Tilt", "Fixture Control Settings"
            ]
        );

        let find = |name: &str| mode.parameters.iter().find(|p| p.name == name).unwrap();
        assert_eq!(find("Dimmer").offsets, [1, 2]);
        assert_eq!(find("Pan").offsets, [9, 10]);
        assert_eq!(find("Pan").attribute, Attribute::Pan);
        assert_eq!(find("Pan").default, 32_768); // 50 %
        assert_eq!(find("Shutter").default, 12 * 256);
        // Blanc par défaut : un simple « At 50 » allume (choix validé sur l'appareil).
        assert_eq!(find("Red").default, u16::MAX);
        assert_eq!(find("White").default, u16::MAX);
        assert_eq!(find("Shutter").attribute, Attribute::Shutter);
        assert_eq!(find("Color Wheel").attribute, Attribute::ColorMacro);
        assert_eq!(find("Fixture Control Settings").attribute.group(), FeatureGroup::Control);
        assert_eq!(find("Zoom").capabilities[0].angle, Some((60.0, 10.0)));

        // Couleur 1 : DMX 11 à 15, rouge pur.
        let wheel = find("Color Wheel");
        let red = wheel.capability_at(13 * 257).unwrap();
        assert_eq!(red.kind, CapabilityKind::ColorPreset { colors: vec![[255, 0, 0]] });
        assert_eq!(red.select_value() >> 8, 11);
    }

    #[test]
    fn rush_mh2_wash_from_ofl() {
        let json = library_file("martin/rush-mh-2-wash.json");
        let fixture = parse(&json, "martin/rush-mh-2-wash", "Martin").unwrap();
        let mode = &fixture.modes[0];
        assert_eq!(mode.footprint, 12);
        assert_eq!(mode.parameters.len(), 10);
        let shutter = &mode.parameters[0];
        assert_eq!(shutter.capability_at(0).unwrap().label, "Fermé");
    }

    #[test]
    fn fine_channel_without_coarse_is_rejected() {
        let json = r#"{
            "name": "Test",
            "availableChannels": { "Pan": { "fineChannelAliases": ["Pan fine"], "capability": { "type": "Pan" } } },
            "modes": [ { "name": "Fin seul", "channels": ["Pan fine"] } ]
        }"#;
        let err = parse(json, "t", "T").unwrap_err();
        assert!(matches!(&err, OflError::NoUsableMode(msg) if msg.contains("sans son canal grossier")));
    }

    #[test]
    fn invalid_mode_is_skipped_not_fatal() {
        let json = r#"{
            "name": "Test",
            "availableChannels": { "Dimmer": { "capability": { "type": "Intensity" } } },
            "modes": [
                { "name": "Bon", "channels": ["Dimmer"] },
                { "name": "Cassé", "channels": ["Inconnu"] }
            ]
        }"#;
        let fixture = parse(json, "t", "T").unwrap();
        assert_eq!(fixture.modes.len(), 1);
        assert_eq!(fixture.warnings.len(), 1);
    }

    #[test]
    fn sunstrip_matrix_modes() {
        let json = library_file("showtec/sunstrip-active-mkii.json");
        let fixture = parse(&json, "showtec/sunstrip-active-mkii", "Showtec").unwrap();
        assert!(fixture.warnings.is_empty(), "{:?}", fixture.warnings);
        let mode = |name: &str| fixture.modes.iter().find(|m| m.name == name).unwrap();

        let individual = mode("Individual");
        assert_eq!(individual.footprint, 10);
        assert_eq!(individual.heads, ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10"]);
        let third = &individual.parameters[2];
        assert_eq!((third.name.as_str(), third.base_name.as_str()), ("Dimmer 3", "Dimmer"));
        assert_eq!((third.head, third.offsets.as_slice()), (Some(2), &[2u16][..]));
        assert_eq!(third.attribute, Attribute::Dimmer);

        assert_eq!(mode("Halves").heads, ["1/2", "2/2"]);
        assert_eq!(mode("Master").footprint, 1);
    }

    #[test]
    fn ledbeam_350_and_suprakolor_hd() {
        let json = library_file("robe/robin-ledbeam-350.json");
        let ledbeam = parse(&json, "robe/robin-ledbeam-350", "Robe").unwrap();
        let footprints: Vec<u16> = ledbeam.modes.iter().map(|m| m.footprint).collect();
        assert_eq!(footprints, [22, 16, 24]);
        let reduced = &ledbeam.modes[1];
        assert_eq!(reduced.parameters.len(), 14); // 16 canaux dont pan et tilt fins
        let red = reduced.parameters.iter().find(|p| p.name == "Red").unwrap();
        assert_eq!((red.offsets.as_slice(), red.default), (&[7u16][..], u16::MAX));

        let json = library_file("starway/suprakolor-hd.json");
        let supra = parse(&json, "starway/suprakolor-hd", "Starway").unwrap();
        assert_eq!(supra.modes[0].footprint, 12);
        let macros = &supra.modes[0].parameters[5];
        assert_eq!(macros.attribute, Attribute::ColorMacro);
        let yellow = macros.capability_at(40 * 257).unwrap();
        assert_eq!(yellow.kind, CapabilityKind::ColorPreset { colors: vec![[255, 255, 0]] });
    }

    #[test]
    fn servocolor_4k_with_switching_channel() {
        let json = library_file("starway/servo-color-4k.json");
        let fixture = parse(&json, "starway/servo-color-4k", "Starway").unwrap();
        let mode = fixture.modes.iter().find(|m| m.footprint == 20).unwrap();
        assert_eq!(mode.parameters.len(), 13); // 20 canaux dont 7 fins
        let speed = mode.parameters.iter().find(|p| p.name == "Effect Speed").unwrap();
        assert_eq!(speed.offsets, [18]);
        assert_eq!(speed.base_name, "Effect Speed (selon Virtual Color Wheel)");
        assert_eq!(speed.capabilities[0].label, "Durée d'effet (10s à 1s)");
        let dimmer = mode.parameters.iter().find(|p| p.name == "Dimmer").unwrap();
        assert_eq!(dimmer.offsets, [5, 6]);
    }
}
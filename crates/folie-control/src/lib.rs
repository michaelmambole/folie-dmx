//! Couche de contrôle de Folie-DMX.
//!
//! La [`Console`] tient le show et la sélection, et traduit les actions de
//! l'utilisateur en commandes pour le moteur. Toutes les interfaces passent par
//! elle (écran, et plus tard OSC et web), si bien qu'une fonction ajoutée ici est
//! disponible partout à la fois.

pub mod color;
pub mod command;
pub mod completion;
pub mod cues;
pub mod effects;
pub mod gels;
pub mod macros;
pub mod osc_out;
pub mod tracking;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use folie_engine::{Command as EngineCommand, EngineHandle};
use folie_fixture::{Attribute, Capability, CapabilityKind, FeatureGroup, FixtureId, ParamKey, Parameter, Patch, Target};
use folie_show::{Effect, Group, Preset, Show};
use thiserror::Error;

use command::{Command, Item, Object, PageMove, Selection, SyntaxError, Value};

#[derive(Debug, Error)]
pub enum CommandError {
    #[error(transparent)]
    Syntax(#[from] SyntaxError),
    #[error("{0}")]
    Invalid(String),
}

fn invalid<T>(message: impl Into<String>) -> Result<T, CommandError> {
    Err(CommandError::Invalid(message.into()))
}

/// Résultat d'une commande réussie.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub message: String,
    /// Le show a changé (groupes, presets…) : il est à enregistrer.
    pub show_changed: bool,
}

impl Outcome {
    fn info(message: impl Into<String>) -> Self {
        Self { message: message.into(), show_changed: false }
    }

    fn changed(message: impl Into<String>) -> Self {
        Self { message: message.into(), show_changed: true }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectMode {
    Replace,
    Add,
    Toggle,
}

fn plural(n: usize, word: &str) -> String {
    if n > 1 { format!("{n} {word}s") } else { format!("{n} {word}") }
}

/// Paramètres d'une cible équivalents à `reference` : même fonction, sinon même attribut.
pub fn matching(patch: &Patch, target: Target, reference: &Parameter) -> Vec<usize> {
    let parameters = patch.parameters_of(target);
    let by_name: Vec<usize> =
        parameters.iter().filter(|(_, p)| p.base_name == reference.base_name).map(|(i, _)| *i).collect();
    if !by_name.is_empty() {
        return by_name;
    }
    parameters.iter().filter(|(_, p)| p.attribute == reference.attribute).map(|(i, _)| *i).collect()
}

/// Couleur affichable d'une fixture d'après des valeurs nommées (celles d'un
/// preset) : couleur de roue si elle est choisie, sinon le mélange. Les
/// composantes absentes gardent leur valeur par défaut.
fn fixture_color(fixture: &folie_fixture::PatchedFixture, values: &std::collections::BTreeMap<String, u16>) -> Option<[f32; 3]> {
    let parameters = &fixture.mode().parameters;
    let level = |p: &Parameter| values.get(&p.name).copied().unwrap_or(p.default) as f32 / u16::MAX as f32;
    // La roue (réelle ou simulée) passe devant le mélange quand une couleur y est choisie.
    for p in parameters.iter().filter(|p| p.attribute == Attribute::ColorMacro && values.contains_key(&p.name)) {
        let value = values[&p.name];
        if let Some(CapabilityKind::ColorPreset { colors }) = p.capability_at(value).map(|c| &c.kind)
            && let Some([r, g, b]) = colors.first()
        {
            return Some([*r as f32 / 255.0, *g as f32 / 255.0, *b as f32 / 255.0]);
        }
    }
    let (mut light, mut filter, mut any) = ([0.0f32; 3], [1.0f32; 3], false);
    for p in parameters {
        let Attribute::ColorMix(color) = &p.attribute else { continue };
        any = true;
        let v = level(p);
        let add = |light: &mut [f32; 3], rgb: [f32; 3]| (0..3).for_each(|i| light[i] += rgb[i] * v);
        match color.as_str() {
            "Red" => add(&mut light, [1.0, 0.0, 0.0]),
            "Green" => add(&mut light, [0.0, 1.0, 0.0]),
            "Blue" => add(&mut light, [0.0, 0.0, 1.0]),
            "White" => add(&mut light, [0.9, 0.9, 0.9]),
            "Warm White" => add(&mut light, [1.0, 0.82, 0.6]),
            "Cold White" => add(&mut light, [0.82, 0.9, 1.0]),
            "Amber" => add(&mut light, [1.0, 0.6, 0.0]),
            "Lime" => add(&mut light, [0.6, 1.0, 0.0]),
            "UV" => add(&mut light, [0.25, 0.0, 0.5]),
            // Soustractif : chaque filtre retire une primaire.
            "Cyan" => filter[0] *= 1.0 - v,
            "Magenta" => filter[1] *= 1.0 - v,
            "Yellow" => filter[2] *= 1.0 - v,
            _ => {}
        }
    }
    if !any {
        return None;
    }
    // Sans source additive (machine CMY), la lumière est blanche avant les filtres.
    if light == [0.0; 3] && filter != [1.0; 3] {
        light = [1.0; 3];
    }
    let mixed = [light[0] * filter[0], light[1] * filter[1], light[2] * filter[2]];
    let peak = mixed.iter().copied().fold(0.0f32, f32::max);
    Some(if peak > 1.0 { mixed.map(|c| c / peak) } else { mixed })
}

/// Couleur que donne un preset, moyenne de ses fixtures ; `None` s'il ne touche
/// aucune couleur.
pub fn preset_color(preset: &Preset, patch: &Patch) -> Option<[u8; 3]> {
    let colors: Vec<[f32; 3]> = preset
        .values
        .iter()
        .filter_map(|(id, values)| patch.get(*id).and_then(|f| fixture_color(f, values)))
        .collect();
    if colors.is_empty() {
        return None;
    }
    let n = colors.len() as f32;
    let channel = |i: usize| (colors.iter().map(|c| c[i]).sum::<f32>() / n * 255.0).round().clamp(0.0, 255.0) as u8;
    Some([channel(0), channel(1), channel(2)])
}

pub struct Console {
    pub show: Show,
    /// Sélection dans l'ordre où elle a été faite.
    selection: Vec<Target>,
    /// Après une valeur ou un preset, la sélection suivante remplace au lieu
    /// d'ajouter, comme sur grandMA.
    selection_used: bool,
    engine: EngineHandle,
    /// Executor visé par Go, Back, Store Cue… quand aucun n'est précisé.
    selected_executor: u32,
    /// Page d'executors affichée : les numéros sans page la visent.
    page: u32,
    /// Programmer : la console en tient la référence et le moteur en reçoit une
    /// copie. Ainsi `At 100` suivi aussitôt de `Store Cue` enregistre bien 100,
    /// même si le moteur n'a pas encore traité la première commande.
    programmer: HashMap<ParamKey, u16>,
    /// Valeurs du programmer venues d'un preset : enregistrées dans une cue,
    /// elles restent des références au preset.
    origins: HashMap<ParamKey, (FeatureGroup, u32)>,
    /// Paramètres marqués « relâchés » (Release) : enregistrés dans une cue, la
    /// séquence cesse de les tenir à partir de cette cue.
    released: HashSet<ParamKey>,
    /// Effets qui tournent dans le programmer.
    effects: Vec<Effect>,
    taps: effects::TapTempo,
    /// Macros en cours de lecture.
    macro_runs: Vec<macros::MacroRun>,
    /// Pendant la lecture des macros : une macro lancée par une ligne attend l'image suivante.
    macro_ticking: bool,
    /// Cue en cours de chaque executor à l'image précédente : une cue qui part joue sa commande.
    cues_seen: HashMap<u32, Option<usize>>,
    /// Socket d'envoi OSC, ouverte au premier message.
    osc_socket: Option<std::net::UdpSocket>,
}

impl Console {
    pub fn new(engine: EngineHandle) -> Self {
        Self {
            show: Show::default(),
            selection: Vec::new(),
            selection_used: false,
            engine,
            selected_executor: 1,
            page: 1,
            programmer: HashMap::new(),
            origins: HashMap::new(),
            released: HashSet::new(),
            effects: Vec::new(),
            taps: effects::TapTempo::default(),
            macro_runs: Vec::new(),
            macro_ticking: false,
            cues_seen: HashMap::new(),
            osc_socket: None,
        }
    }

    pub fn engine(&self) -> &EngineHandle {
        &self.engine
    }

    pub fn selection(&self) -> &[Target] {
        &self.selection
    }

    /// Valeurs saisies, en attente d'être enregistrées.
    pub fn programmer(&self) -> &HashMap<ParamKey, u16> {
        &self.programmer
    }

    /// Paramètres marqués « relâchés » dans le programmer.
    pub fn released(&self) -> &HashSet<ParamKey> {
        &self.released
    }

    /// Presets appliqués dans le programmer (au moins une valeur en vient encore).
    pub fn programmer_presets(&self) -> std::collections::BTreeSet<(FeatureGroup, u32)> {
        self.origins.values().copied().collect()
    }

    pub fn select(&mut self, targets: impl IntoIterator<Item = Target>, mode: SelectMode) {
        if mode == SelectMode::Replace {
            self.selection.clear();
        }
        for target in targets.into_iter().filter(|t| self.show.patch.contains(*t)) {
            match self.selection.iter().position(|t| *t == target) {
                Some(i) if mode == SelectMode::Toggle => {
                    self.selection.remove(i);
                }
                Some(_) => {}
                None => self.selection.push(target),
            }
        }
        self.selection_used = false;
    }

    pub fn clear_selection(&mut self) {
        self.selection.clear();
    }

    /// Remplace le show : sélection et programmer repartent de zéro.
    pub fn load_show(&mut self, show: Show) {
        self.show = show;
        self.selection.clear();
        self.clear_programmer();
        self.engine.send(EngineCommand::SetTempo(self.show.tempo));
        self.patch_changed();
    }

    /// À appeler après toute modification du patch : retire des groupes, des
    /// presets et de la sélection ce qui n'est plus patché, puis prévient le moteur.
    pub fn patch_changed(&mut self) {
        let patch = &self.show.patch;
        self.selection.retain(|t| patch.contains(*t));
        for group in self.show.groups.values_mut() {
            group.targets.retain(|t| patch.contains(*t));
        }
        for preset in self.show.presets.values_mut() {
            preset.values.retain(|id, _| patch.get(*id).is_some());
        }
        for sequence in self.show.sequences.values_mut() {
            for cue in sequence.cues.values_mut() {
                cue.values.retain(|id, _| patch.get(*id).is_some());
                for effect in cue.effects.iter_mut().flatten() {
                    effect.targets.retain(|t| patch.contains(*t));
                }
            }
        }
        for effect in &mut self.effects {
            effect.targets.retain(|t| patch.contains(*t));
        }
        self.effects.retain(|e| !e.targets.is_empty());
        self.effects_changed();
        self.origins.retain(|key, _| patch.get(key.fixture).is_some());
        self.released.retain(|key| patch.get(key.fixture).is_some_and(|f| key.parameter < f.mode().parameters.len()));
        self.programmer
            .retain(|key, _| patch.get(key.fixture).is_some_and(|f| key.parameter < f.mode().parameters.len()));
        self.engine.send(EngineCommand::SetPatch(Arc::new(patch.clone())));
        self.sync_playback();
    }

    fn clear_programmer(&mut self) {
        self.programmer.clear();
        self.origins.clear();
        self.released.clear();
        self.effects.clear();
        self.effects_changed();
        self.engine.send(EngineCommand::ClearProgrammer);
    }

    /// Valeur saisie à la main : elle n'est plus liée à un preset.
    fn send_value(&mut self, target: Target, parameter: usize, value: u16) {
        let key = ParamKey { fixture: target.fixture, parameter };
        self.origins.remove(&key);
        self.released.remove(&key);
        self.programmer.insert(key, value);
        self.engine.send(EngineCommand::SetAttribute { key, value });
        self.turn_off_companions(target, parameter, value);
    }

    /// Un paramètre quitte sa position neutre : les canaux que l'appareil exige alors à 0
    /// (`Parameter::turns_off`) y sont mis, dans le programmer comme le reste.
    fn turn_off_companions(&mut self, target: Target, parameter: usize, value: u16) {
        let companions: Vec<usize> = {
            let parameters = self.show.patch.parameters_of(target);
            let Some((_, p)) = parameters.iter().find(|(i, _)| *i == parameter) else { return };
            let neutral = p.capability_at(value).is_none_or(|c| c.kind == CapabilityKind::NoFunction);
            if p.turns_off.is_empty() || neutral {
                return;
            }
            parameters.iter().filter(|(_, q)| p.turns_off.contains(&q.base_name)).map(|(i, _)| *i).collect()
        };
        for companion in companions {
            self.send_value(target, companion, 0);
        }
    }

    /// Donne à toute la sélection la valeur d'un paramètre de référence.
    pub fn set_like(&mut self, reference: &Parameter, value: u16) {
        for target in self.selection.clone() {
            for parameter in matching(&self.show.patch, target, reference) {
                self.send_value(target, parameter, value);
            }
        }
        self.selection_used = true;
    }

    /// Relâche un paramètre (et ses équivalents) sur toute la sélection : il revient
    /// à ce qui est dessous, executor ou valeur par défaut.
    pub fn release_like(&mut self, reference: &Parameter) {
        for target in self.selection.clone() {
            for parameter in matching(&self.show.patch, target, reference) {
                let key = ParamKey { fixture: target.fixture, parameter };
                self.origins.remove(&key);
                self.released.remove(&key);
                self.programmer.remove(&key);
                self.engine.send(EngineCommand::ReleaseAttribute(key));
            }
        }
    }

    /// Dimmer à 100 % et shutter ouvert sur la sélection.
    pub fn full(&mut self) {
        for target in self.selection.clone() {
            let parameters: Vec<(usize, Parameter)> =
                self.show.patch.parameters_of(target).into_iter().map(|(i, p)| (i, p.clone())).collect();
            for (index, p) in parameters {
                let value = match p.attribute {
                    Attribute::Dimmer => Some(u16::MAX),
                    Attribute::Shutter => p
                        .capabilities
                        .iter()
                        .find(|c| matches!(&c.kind, CapabilityKind::Shutter { effect, .. } if effect == "Open"))
                        .map(Capability::select_value),
                    _ => None,
                };
                if let Some(value) = value {
                    self.send_value(target, index, value);
                }
            }
        }
        self.selection_used = true;
    }

    /// Rend la sélection à ses valeurs par défaut.
    pub fn release_selection(&mut self) {
        for &target in &self.selection {
            for (parameter, _) in self.show.patch.parameters_of(target) {
                let key = ParamKey { fixture: target.fixture, parameter };
                self.origins.remove(&key);
                self.released.remove(&key);
                self.programmer.remove(&key);
                self.engine.send(EngineCommand::ReleaseAttribute(key));
            }
        }
    }

    /// `Release` : marque tous les paramètres de la sélection comme relâchés. Store
    /// les enregistre ainsi dans la cue : la séquence cesse de les tenir.
    pub fn release_in_programmer(&mut self) -> Result<Outcome, CommandError> {
        if self.selection.is_empty() {
            return invalid("aucune sélection à relâcher");
        }
        let mut count = 0;
        for &target in &self.selection {
            for (parameter, _) in self.show.patch.parameters_of(target) {
                let key = ParamKey { fixture: target.fixture, parameter };
                self.origins.remove(&key);
                self.programmer.remove(&key);
                self.engine.send(EngineCommand::ReleaseAttribute(key));
                count += usize::from(self.released.insert(key));
            }
        }
        self.selection_used = true;
        let s = if count > 1 { "s" } else { "" };
        Ok(Outcome::info(format!("{} relâché{s} : Store les enregistre dans la cue", plural(count, "paramètre"))))
    }

    pub fn execute(&mut self, line: &str) -> Result<Outcome, CommandError> {
        match command::parse(line)? {
            Command::Select { selection, attribute, value } => {
                let mut message = String::new();
                if let Some(selection) = selection {
                    self.execute_selection(&selection)?;
                    message = format!("{} sélectionnée(s)", plural(self.selection.len(), "cible"));
                }
                match (attribute, value) {
                    (None, Some(Value::Percent(percent))) => message = self.set_dimmer(percent)?,
                    (attribute, Some(value)) => message = self.set_attribute(attribute.as_deref(), value)?,
                    (_, None) => {}
                }
                Ok(Outcome::info(message))
            }
            Command::ApplyPreset(family, number) => self.apply_preset(family, number),
            Command::Store { object: Object::Group(number), name, .. } => self.store_group(number, name),
            Command::Store { object: Object::Preset(family, number), name, .. } => {
                self.store_preset(family, number, name)
            }
            Command::Store { object: Object::Cue(number), name, fade } => self.store_cue(number, fade, name),
            Command::Store { object: Object::Macro(number), name, .. } => Ok(self.store_macro(number, name)),
            Command::Label { object: Object::Macro(number), name } => self.label_macro(number, name),
            Command::Delete(Object::Macro(number)) => self.delete_macro(number),
            Command::Macro(number) => self.run_macro(number),
            Command::Wait(_) => invalid("Wait ne s'utilise que dans une macro"),
            Command::Osc { destination, address, args } => self.send_osc(&destination, &address, &args),
            Command::Label { object: Object::Cue(Some(number)), name } => self.label_cue(number, name),
            Command::Delete(Object::Cue(Some(number))) => self.delete_cue(number),
            Command::Label { object, name } => self.label(object, name),
            Command::Delete(object) => self.delete(object),
            Command::Release(selection) => {
                if let Some(selection) = selection {
                    self.execute_selection(&selection)?;
                }
                self.release_in_programmer()
            }
            Command::Assert(executor) => self.assert(executor.map(|x| self.resolve_executor(x))),
            Command::Go(executor) => self.go(executor.map(|x| self.resolve_executor(x))),
            Command::Back(executor) => self.back(executor.map(|x| self.resolve_executor(x))),
            Command::Off(executor) => self.off(executor.map(|x| self.resolve_executor(x))),
            Command::Goto(number) => self.goto(number),
            Command::SelectExecutor(executor) => Ok(self.select_executor(self.resolve_executor(executor))),
            Command::SetFader(executor, percent) => self.set_fader(self.resolve_executor(executor), percent),
            Command::ApplyEffect(number) => self.apply_effect(number),
            Command::StoreExecutor(executor) => self.store_in_executor(self.resolve_executor(executor)),
            Command::Page(PageMove::To(page)) => Ok(self.set_page(page)),
            Command::Page(PageMove::Next) => Ok(self.set_page(self.page + 1)),
            Command::Page(PageMove::Previous) => Ok(self.set_page(self.page.saturating_sub(1))),
            Command::Fix(executor) => self.toggle_fixed(self.resolve_executor(executor)),
            Command::Block(number) => self.block_cue(number),
            Command::Unblock(number) => self.unblock_cue(number),
            Command::OffAll => Ok(self.off_all()),
            Command::SetTempo(bpm) => Ok(self.set_tempo(bpm)),
            Command::Clear => {
                if self.selection.is_empty() {
                    self.clear_programmer();
                    Ok(Outcome::info("Programmer vidé"))
                } else {
                    self.selection.clear();
                    Ok(Outcome::info("Sélection vidée (Clear à nouveau pour vider le programmer)"))
                }
            }
            Command::ClearAll => {
                self.selection.clear();
                self.clear_programmer();
                Ok(Outcome::info("Sélection et programmer vidés"))
            }
        }
    }

    /// Sélection tapée en ligne de commande : elle complète la précédente, sauf après une valeur.
    fn execute_selection(&mut self, selection: &Selection) -> Result<(), CommandError> {
        let targets = self.resolve(selection)?;
        if self.selection_used {
            self.selection.clear();
        }
        for (add, targets) in targets {
            if add {
                self.select(targets, SelectMode::Add);
            } else {
                self.selection.retain(|t| !targets.contains(t));
            }
        }
        self.selection_used = false;
        Ok(())
    }

    /// Cibles désignées par chaque élément d'une sélection, dans l'ordre.
    fn resolve(&self, selection: &Selection) -> Result<Vec<(bool, Vec<Target>)>, CommandError> {
        let patch = &self.show.patch;
        let check = |t: Target| {
            if patch.contains(t) { Ok(t) } else { invalid(format!("Fixture {t} n'est pas patchée")) }
        };
        let mut resolved = Vec::new();
        for (add, item) in &selection.items {
            let targets = match *item {
                Item::Target(t) => vec![check(t)?],
                Item::Range(a, b) => match (a.head, b.head) {
                    (None, None) => {
                        let (low, high) = (a.fixture.min(b.fixture), a.fixture.max(b.fixture));
                        let mut ids: Vec<Target> = patch
                            .fixtures()
                            .iter()
                            .filter(|f| (low..=high).contains(&f.id))
                            .map(|f| Target::whole(f.id))
                            .collect();
                        // « Thru » à l'envers garde l'ordre demandé, comme sur MA.
                        if a.fixture > b.fixture {
                            ids.reverse();
                        }
                        if ids.is_empty() {
                            return invalid(format!("aucune fixture patchée de {a} à {b}"));
                        }
                        ids
                    }
                    (Some(h1), Some(h2)) if a.fixture == b.fixture => {
                        check(a)?;
                        check(b)?;
                        let heads: Vec<usize> =
                            if h1 <= h2 { (h1..=h2).collect() } else { (h2..=h1).rev().collect() };
                        heads.into_iter().map(|head| Target { fixture: a.fixture, head: Some(head) }).collect()
                    }
                    _ => return invalid("Thru entre sous-fixtures : rester dans la même fixture (ex. 5.1 Thru 5.4)"),
                },
                Item::Group(number) => match self.show.groups.get(&number) {
                    Some(group) => group.targets.iter().copied().filter(|t| patch.contains(*t)).collect(),
                    None => return invalid(format!("le groupe {number} n'existe pas")),
                },
            };
            resolved.push((*add, targets));
        }
        Ok(resolved)
    }

    /// Valeur d'un paramètre telle qu'elle sort en ce moment.
    fn live_value(&self, key: ParamKey) -> Option<u16> {
        let fixture = self.show.patch.get(key.fixture)?;
        let parameter = fixture.mode().parameters.get(key.parameter)?;
        let snapshot = self.engine.snapshot();
        let universe = snapshot.output.get(fixture.universe)?;
        let byte = |offset: u16| universe.get(fixture.address + offset).ok();
        match parameter.offsets.as_slice() {
            [coarse] => byte(*coarse).map(|v| v as u16 * 257),
            [coarse, fine, ..] => Some(((byte(*coarse)? as u16) << 8) | byte(*fine)? as u16),
            [] => None,
        }
    }

    /// `Attribute "Pan" At 50`, `At + 10` : un paramètre nommé (le dimmer sans nom) sur toute
    /// la sélection, à une valeur ou relatif à sa valeur en cours (programmer, sinon sortie).
    fn set_attribute(&mut self, name: Option<&str>, value: Value) -> Result<String, CommandError> {
        if self.selection.is_empty() {
            return invalid("aucune sélection");
        }
        let wanted = |p: &Parameter| match name {
            None => p.attribute == Attribute::Dimmer,
            Some(name) => {
                [p.base_name.as_str(), p.name.as_str(), &p.attribute.label()].iter().any(|n| n.eq_ignore_ascii_case(name))
            }
        };
        let mut touched = 0;
        for target in self.selection.clone() {
            let parameters: Vec<usize> =
                self.show.patch.parameters_of(target).into_iter().filter(|(_, p)| wanted(p)).map(|(i, _)| i).collect();
            touched += usize::from(!parameters.is_empty());
            for parameter in parameters {
                let key = ParamKey { fixture: target.fixture, parameter };
                let full = u16::MAX as f32;
                let percent = match value {
                    Value::Percent(p) => p,
                    Value::Relative(delta) => {
                        let current = self.programmer.get(&key).copied().or_else(|| self.live_value(key)).unwrap_or(0);
                        (current as f32 / full * 100.0 + delta).clamp(0.0, 100.0)
                    }
                };
                self.send_value(target, parameter, (percent / 100.0 * full).round() as u16);
            }
        }
        let label = name.unwrap_or("Dimmer");
        if touched == 0 {
            return invalid(format!("aucun paramètre « {label} » dans la sélection"));
        }
        self.selection_used = true;
        let what = match value {
            Value::Percent(p) => format!("à {p} %"),
            Value::Relative(d) if d >= 0.0 => format!("+ {d} %"),
            Value::Relative(d) => format!("- {} %", -d),
        };
        Ok(format!("{label} {what} sur {}", plural(touched, "cible")))
    }

    fn set_dimmer(&mut self, percent: f32) -> Result<String, CommandError> {
        if self.selection.is_empty() {
            return invalid("aucune sélection");
        }
        let value = (percent / 100.0 * u16::MAX as f32).round() as u16;
        let mut touched = 0;
        for target in self.selection.clone() {
            let dimmers: Vec<usize> = self
                .show
                .patch
                .parameters_of(target)
                .into_iter()
                .filter(|(_, p)| p.attribute == Attribute::Dimmer)
                .map(|(i, _)| i)
                .collect();
            touched += usize::from(!dimmers.is_empty());
            for parameter in dimmers {
                self.send_value(target, parameter, value);
            }
        }
        if touched == 0 {
            return invalid("aucun dimmer dans la sélection");
        }
        self.selection_used = true;
        Ok(format!("Dimmer à {percent} % sur {}", plural(touched, "cible")))
    }

    /// Place un projecteur sur scène (fenêtre Scène).
    pub fn set_stage_place(&mut self, fixture: FixtureId, place: folie_show::StagePlace) -> Result<Outcome, CommandError> {
        if self.show.patch.get(fixture).is_none() {
            return invalid(format!("Fixture {fixture} n'est pas patchée"));
        }
        self.show.stage.insert(fixture, place);
        Ok(Outcome::changed(format!("Fixture {fixture} placée")))
    }

    /// Rend des projecteurs à leur place automatique, sur la perche.
    pub fn reset_stage_places(&mut self, fixtures: &[FixtureId]) -> Result<Outcome, CommandError> {
        let removed = fixtures.iter().filter(|f| self.show.stage.remove(f).is_some()).count();
        Ok(Outcome::changed(format!("{} replacée(s) sur la perche", plural(removed, "fixture"))))
    }

    pub fn store_group(&mut self, number: u32, name: Option<String>) -> Result<Outcome, CommandError> {
        if self.selection.is_empty() {
            return invalid("aucune sélection à enregistrer");
        }
        let replaced = self.show.groups.get(&number);
        let name = name
            .or_else(|| replaced.map(|g| g.name.clone()))
            .unwrap_or_else(|| format!("Groupe {number}"));
        let verb = if replaced.is_some() { "remplacé" } else { "enregistré" };
        let count = self.selection.len();
        self.show.groups.insert(number, Group { name: name.clone(), targets: self.selection.clone() });
        Ok(Outcome::changed(format!("Groupe {number} « {name} » {verb} ({})", plural(count, "cible"))))
    }

    pub fn select_group(&mut self, number: u32, mode: SelectMode) -> Result<Outcome, CommandError> {
        let Some(group) = self.show.groups.get(&number) else {
            return invalid(format!("le groupe {number} n'existe pas"));
        };
        let (name, targets) = (group.name.clone(), group.targets.clone());
        self.select(targets, mode);
        Ok(Outcome::info(format!("Groupe {number} « {name} » sélectionné")))
    }

    /// Enregistre les valeurs du programmer de cette famille : pour la
    /// sélection, ou pour toutes les fixtures programmées s'il n'y a pas de sélection.
    pub fn store_preset(
        &mut self,
        family: FeatureGroup,
        number: u32,
        name: Option<String>,
    ) -> Result<Outcome, CommandError> {
        let targets: Vec<Target> = if self.selection.is_empty() {
            self.show.patch.fixtures().iter().map(|f| Target::whole(f.id)).collect()
        } else {
            self.selection.clone()
        };
        let mut preset = Preset::default();
        for target in targets {
            for (index, parameter) in self.show.patch.parameters_of(target) {
                if parameter.attribute.group() != family {
                    continue;
                }
                let key = ParamKey { fixture: target.fixture, parameter: index };
                if let Some(value) = self.programmer.get(&key) {
                    preset.values.entry(target.fixture).or_default().insert(parameter.name.clone(), *value);
                }
            }
        }
        if preset.values.is_empty() {
            return invalid(format!("aucune valeur « {} » dans le programmer", family.label()));
        }
        let key = (family, number);
        let replaced = self.show.presets.get(&key);
        preset.name = name
            .or_else(|| replaced.map(|p| p.name.clone()))
            .unwrap_or_else(|| format!("{} {number}", family.label()));
        let verb = if replaced.is_some() { "remplacé" } else { "enregistré" };
        let message = format!(
            "Preset {}.{number} « {} » {verb} ({})",
            family.preset_number(),
            preset.name,
            plural(preset.values.len(), "fixture")
        );
        self.show.presets.insert(key, preset);
        self.sync_playback(); // les cues qui l'utilisent changent avec lui
        Ok(Outcome::changed(message))
    }

    /// Applique un preset à la sélection. Sans sélection, sélectionne d'abord
    /// les fixtures qu'il contient.
    pub fn apply_preset(&mut self, family: FeatureGroup, number: u32) -> Result<Outcome, CommandError> {
        let Some(preset) = self.show.presets.get(&(family, number)) else {
            return invalid(format!("le preset {}.{number} n'existe pas", family.preset_number()));
        };
        if self.selection.is_empty() {
            self.selection = preset.values.keys().map(|id| Target::whole(*id)).collect();
        }
        let mut applied = 0;
        for &target in &self.selection {
            let Some(values) = preset.values.get(&target.fixture) else { continue };
            let mut any = false;
            for (index, parameter) in self.show.patch.parameters_of(target) {
                if let Some(value) = values.get(&parameter.name) {
                    let key = ParamKey { fixture: target.fixture, parameter: index };
                    self.engine.send(EngineCommand::SetAttribute { key, value: *value });
                    self.programmer.insert(key, *value);
                    self.origins.insert(key, (family, number));
                    any = true;
                }
            }
            applied += usize::from(any);
        }
        if applied == 0 {
            return invalid("aucune cible de la sélection n'est dans ce preset");
        }
        self.selection_used = true;
        Ok(Outcome::info(format!(
            "Preset {}.{number} « {} » appliqué à {}",
            family.preset_number(),
            preset.name,
            plural(applied, "cible")
        )))
    }

    /// Renomme un groupe ou un preset (`Label Group 3 "Cour"`).
    pub fn label(&mut self, object: Object, name: String) -> Result<Outcome, CommandError> {
        let slot = match object {
            Object::Group(n) => self.show.groups.get_mut(&n).map(|g| &mut g.name),
            Object::Preset(f, n) => self.show.presets.get_mut(&(f, n)).map(|p| &mut p.name),
            Object::Cue(_) | Object::Macro(_) => None,
        };
        match slot {
            Some(slot) => {
                *slot = name.clone();
                Ok(Outcome::changed(format!("Renommé « {name} »")))
            }
            None => invalid("rien à renommer à ce numéro"),
        }
    }

    /// Supprime un groupe ou un preset (`Delete Group 3`).
    pub fn delete(&mut self, object: Object) -> Result<Outcome, CommandError> {
        let removed = match object {
            Object::Group(n) => self.show.groups.remove(&n).map(|g| format!("Groupe {n} « {} » supprimé", g.name)),
            Object::Preset(f, n) => self
                .show
                .presets
                .remove(&(f, n))
                .map(|p| format!("Preset {}.{n} « {} » supprimé", f.preset_number(), p.name)),
            Object::Cue(_) | Object::Macro(_) => None,
        };
        if removed.is_some() {
            self.sync_playback();
        }
        match removed {
            Some(message) => Ok(Outcome::changed(message)),
            None => invalid("rien à supprimer à ce numéro"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Duration;

    use folie_dmx::NullOutput;
    use folie_engine::{EngineConfig, OutputBinding, Snapshot};
    use folie_fixture::{FixtureId, Library, PatchedFixture};
    use folie_show::{CueNumber, CueValue};

    use super::*;
    use crate::command::fixture;

    /// 4 MH 6 (1 à 4) et un Sunstrip 10 dimmers (10).
    pub(crate) fn console() -> Console {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let (library, _) = Library::load_dir(root);
        let engine = folie_engine::spawn(
            EngineConfig { universes: 1, rate_hz: 200.0 },
            vec![OutputBinding { universe: 0, output: Box::new(NullOutput) }],
        );
        let mut console = Console::new(engine);
        let mut add = |id, key: &str, mode, address| {
            let fixture_type = library.get(key).unwrap().clone();
            let fixture = PatchedFixture {
                id: FixtureId(id),
                name: format!("F{id}"),
                fixture_type,
                mode,
                universe: 0,
                address,
            };
            console.show.patch.add(fixture).unwrap();
        };
        for i in 0..4 {
            add(i + 1, "martin/rush-mh-6-wash", 0, 1 + 14 * i as u16);
        }
        add(10, "showtec/sunstrip-active-mkii", 3, 101);
        console.patch_changed();
        console
    }

    fn settled(console: &Console) -> Snapshot {
        std::thread::sleep(Duration::from_millis(30));
        console.engine().snapshot()
    }

    fn dmx(console: &Console, channel: u16) -> u8 {
        settled(console).output[0].get(channel).unwrap()
    }

    pub(crate) fn run(console: &mut Console, line: &str) -> Outcome {
        console.execute(line).unwrap_or_else(|e| panic!("{line} : {e}"))
    }

    #[test]
    fn select_and_set_dimmer() {
        let mut c = console();
        run(&mut c, "Fixture 1 Thru 4 - 2");
        assert_eq!(c.selection(), [fixture(1), fixture(3), fixture(4)]);
        let outcome = run(&mut c, "At 50");
        assert_eq!(outcome.message, "Dimmer à 50 % sur 3 cibles");
        assert_eq!(dmx(&c, 2), 128); // dimmer de la fixture 1, canal 2
        assert_eq!(dmx(&c, 16), 0); // fixture 2 non sélectionnée

        // Après une valeur, la sélection suivante remplace.
        run(&mut c, "Fixture 2 At Full");
        assert_eq!(c.selection(), [fixture(2)]);
        assert_eq!(dmx(&c, 16), 255);
    }

    #[test]
    fn thru_backwards_keeps_order_and_sub_fixtures() {
        let mut c = console();
        run(&mut c, "Fixture 4 Thru 2");
        assert_eq!(c.selection(), [fixture(4), fixture(3), fixture(2)]);
        run(&mut c, "Clear");
        run(&mut c, "Fixture 10.2 Thru 10.4 At 100");
        assert_eq!((dmx(&c, 101), dmx(&c, 102), dmx(&c, 104), dmx(&c, 105)), (0, 255, 255, 0));
    }

    #[test]
    fn groups_store_recall_and_survive_repatch() {
        let mut c = console();
        run(&mut c, "Fixture 3 + 1 + 10.5");
        assert!(run(&mut c, r#"Store Group 7 "Cour""#).show_changed);
        assert_eq!(c.show.groups[&7].targets, [fixture(3), fixture(1), "10.5".parse().unwrap()]);
        run(&mut c, "ClearAll");
        run(&mut c, "Group 7 At 100");
        assert_eq!((dmx(&c, 30), dmx(&c, 2), dmx(&c, 105), dmx(&c, 106)), (255, 255, 255, 0));

        c.show.patch.remove(FixtureId(3));
        c.patch_changed();
        assert_eq!(c.show.groups[&7].targets.len(), 2, "la fixture dépatchée quitte le groupe");
        assert!(c.execute("Group 99").is_err());
    }

    #[test]
    fn presets_store_and_apply() {
        let mut c = console();
        // Sans rouge sur la fixture 1 : on règle le canal Red puis on enregistre la couleur.
        let red = c.show.patch.get(FixtureId(1)).unwrap().mode().parameters[2].clone();
        assert_eq!(red.name, "Red");
        run(&mut c, "Fixture 1");
        c.set_like(&red, 0);
        settled(&c);
        let stored = run(&mut c, r#"Store Preset 4.1 "Sans rouge""#);
        assert_eq!(stored.message, "Preset 4.1 « Sans rouge » enregistré (1 fixture)");
        assert!(c.execute("Store Preset 2.1").is_err(), "aucune valeur de position");

        // Appliqué à la fixture 1 seulement : elle est la seule dans le preset.
        run(&mut c, "ClearAll");
        settled(&c);
        assert_eq!(dmx(&c, 4), 255, "Red par défaut à fond sur la MH 6 (blanc)");
        let applied = run(&mut c, "Preset 4.1");
        assert_eq!(c.selection(), [fixture(1)], "sans sélection : les fixtures du preset");
        assert!(applied.message.contains("appliqué à 1 cible"));
        assert_eq!(dmx(&c, 4), 0);

        run(&mut c, r#"Label Preset 4.1 "Cyan""#);
        assert_eq!(c.show.presets[&(FeatureGroup::Color, 1)].name, "Cyan");
        run(&mut c, "Delete Preset 4.1");
        assert!(c.show.presets.is_empty());
    }

    /// La roue virtuelle de la ServoColor 4K n'agit qu'avec le Blanc à 0 (vérifié sur l'appareil).
    #[test]
    fn servocolor_wheel_turns_white_off() {
        let mut c = console();
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../library"));
        let (library, _) = Library::load_dir(root);
        let fixture_type = library.get("starway/servo-color-4k").unwrap().clone();
        let mode = fixture_type.modes.iter().position(|m| m.footprint == 20).unwrap();
        let servo = PatchedFixture { id: FixtureId(20), name: "Servo".into(), fixture_type, mode, universe: 0, address: 201 };
        c.show.patch.add(servo).unwrap();
        c.patch_changed();
        let parameters = c.show.patch.get(FixtureId(20)).unwrap().mode().parameters.clone();
        let find = |name: &str| parameters.iter().position(|p| p.name == name).unwrap();
        let (wheel, white) = (find("Virtual Color Wheel"), find("White"));
        let key = |parameter| ParamKey { fixture: FixtureId(20), parameter };

        run(&mut c, "Fixture 20");
        c.set_like(&parameters[white], u16::MAX);
        c.set_like(&parameters[wheel], 0);
        assert_eq!(c.programmer().get(&key(white)), Some(&u16::MAX), "roue au neutre : le Blanc reste");
        c.set_like(&parameters[wheel], 11 * 257);
        assert_eq!(c.programmer().get(&key(white)), Some(&0), "roue utilisée : Blanc à 0");
        assert_eq!(c.programmer().get(&key(wheel)), Some(&(11 * 257)));
    }

    #[test]
    fn release_a_single_parameter() {
        let mut c = console();
        run(&mut c, "Fixture 1 Thru 2 At 100");
        let fixture = c.show.patch.get(FixtureId(1)).unwrap().clone();
        let red = fixture.mode().parameters[2].clone();
        c.set_like(&red, 0);
        settled(&c);
        c.release_like(&red);
        assert_eq!((dmx(&c, 4), dmx(&c, 18)), (255, 255), "rouge relâché sur les deux fixtures : retour au blanc");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (255, 255), "le dimmer reste programmé");
        assert_eq!(c.programmer().len(), 2);
    }

    #[test]
    fn clear_twice_like_ma() {
        let mut c = console();
        run(&mut c, "Fixture 1 At 100");
        assert_eq!(run(&mut c, "Clear").message, "Sélection vidée (Clear à nouveau pour vider le programmer)");
        assert_eq!(dmx(&c, 2), 255, "le programmer est encore là");
        assert_eq!(run(&mut c, "Clear").message, "Programmer vidé");
        assert_eq!(dmx(&c, 2), 0);
    }

    #[test]
    fn errors_are_explained() {
        let mut c = console();
        let err = |c: &mut Console, line| c.execute(line).unwrap_err().to_string();
        assert_eq!(err(&mut c, "Fixture 99"), "Fixture 99 n'est pas patchée");
        assert_eq!(err(&mut c, "At 50"), "aucune sélection");
        assert_eq!(err(&mut c, "Preset 4.9"), "le preset 4.9 n'existe pas");
        assert!(err(&mut c, "Fixture 1.2").contains("n'est pas patchée"), "la MH 6 n'a pas de sous-fixture");
        assert!(err(&mut c, "Fixture 1 Thru 10.2").contains("Thru"));
    }

    #[test]
    fn cues_track_go_and_back() {
        let mut c = console();
        run(&mut c, "Fixture 1 At 100");
        run(&mut c, r#"Store Cue 1 Fade 0 "Face""#);
        run(&mut c, "ClearAll");
        run(&mut c, "Fixture 2 At 50");
        let stored = run(&mut c, "Store Cue Fade 0");
        assert!(stored.message.starts_with("Cue 2 enregistrée"), "{}", stored.message);
        run(&mut c, "ClearAll");
        assert_eq!(dmx(&c, 2), 0, "programmer vidé : rien ne sort");

        run(&mut c, "Go");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (255, 0));
        run(&mut c, "Go");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (255, 128), "la fixture 1 est suivie (tracking)");
        run(&mut c, "Back");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (255, 0));

        // Release : la fixture 1 ne tient plus à partir de la cue 3.
        run(&mut c, "Fixture 1 At 0");
        run(&mut c, "Store Cue 3 Fade 0");
        run(&mut c, "ClearAll");
        run(&mut c, "Goto Cue 3");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (0, 128));
        run(&mut c, "Off");
        assert_eq!(dmx(&c, 16), 0);
    }

    #[test]
    fn cues_follow_their_presets() {
        let mut c = console();
        let red = c.show.patch.get(FixtureId(3)).unwrap().mode().parameters[2].clone();
        run(&mut c, "Fixture 3");
        c.set_like(&red, u16::MAX);
        settled(&c);
        run(&mut c, r#"Store Preset 4.1 "Rouge""#);
        run(&mut c, "ClearAll");

        run(&mut c, "Fixture 3");
        run(&mut c, "Preset 4.1");
        settled(&c);
        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "ClearAll");
        let cue = &c.show.sequences[&1].cues[&CueNumber::whole(1)];
        assert_eq!(cue.values[&FixtureId(3)]["Red"], CueValue::Preset(FeatureGroup::Color, 1));

        run(&mut c, "Go");
        assert_eq!(dmx(&c, 32), 255);
        // On retouche le preset : la cue suit sans être réenregistrée.
        run(&mut c, "Fixture 3");
        c.set_like(&red, 0x8000);
        settled(&c);
        run(&mut c, "Store Preset 4.1");
        run(&mut c, "ClearAll");
        assert_eq!(dmx(&c, 32), 128);
    }

    #[test]
    fn color_picker_translates_per_fixture() {
        use crate::color::key_of;
        let mut c = console();
        // MH 6 (RGBW + roue simulée) : orange pastel → blanc extrait, roue ouverte.
        run(&mut c, "Fixture 1");
        assert_eq!(c.apply_color([1.0, 0.75, 0.5]), 1);
        let t = fixture(1);
        let value = |c: &Console, name| c.programmer()[&key_of(c, t, name)];
        assert_eq!(value(&c, "White"), 32_768, "la part commune passe dans le blanc");
        assert_eq!((value(&c, "Red"), value(&c, "Green"), value(&c, "Blue")), (32_768, 16_384, 0));
        assert_eq!(value(&c, "Color Wheel") >> 8, 0, "roue en position ouverte");

        // Le Sunstrip n'a pas de couleur : rien n'est envoyé.
        run(&mut c, "Fixture 10");
        assert_eq!(c.apply_color([1.0, 0.0, 0.0]), 0);
    }

    #[test]
    fn wheel_only_picks_the_nearest_color() {
        let c = console();
        let parameters = c.show.patch.parameters_of(fixture(1));
        let wheel: Vec<(usize, &Parameter)> =
            parameters.into_iter().filter(|(_, p)| p.attribute == Attribute::ColorMacro).collect();
        // Vert pur : la couleur 12 (R0 G255 B0 W0), DMX 66 à 70.
        let values = crate::color::color_values(&wheel, [0.0, 1.0, 0.0]);
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].1 >> 8, 66);
    }

    #[test]
    fn preset_colors() {
        let c = console();
        let preset = |values: &[(&str, u16)], fixture| folie_show::Preset {
            name: String::new(),
            values: std::collections::BTreeMap::from([(
                FixtureId(fixture),
                values.iter().map(|(n, v)| (n.to_string(), *v)).collect(),
            )]),
        };
        // MH 6 (blanc par défaut) : rouge seul, puis rouge + vert = jaune ; la roue (couleur 12, vert) prime.
        let red = [("Red", 65_535), ("Green", 0), ("Blue", 0), ("White", 0)];
        assert_eq!(preset_color(&preset(&red, 1), &c.show.patch), Some([255, 0, 0]));
        let yellow = [("Red", 65_535), ("Green", 65_535), ("Blue", 0), ("White", 0)];
        assert_eq!(preset_color(&preset(&yellow, 1), &c.show.patch), Some([255, 255, 0]));
        assert_eq!(preset_color(&preset(&[("Red", 65_535)], 1), &c.show.patch), Some([255, 255, 255]), "le reste au défaut");
        let wheel = preset(&[("Red", 65_535), ("Color Wheel", 66 * 257)], 1);
        assert_eq!(preset_color(&wheel, &c.show.patch), Some([0, 255, 0]));
        // Le Sunstrip n'a pas de couleur.
        assert_eq!(preset_color(&preset(&[("Dimmer 1", 65_535)], 10), &c.show.patch), None);
    }

    #[test]
    fn presets_in_programmer_and_in_executors() {
        use std::collections::BTreeMap;
        let mut c = console();
        let red = c.show.patch.get(FixtureId(1)).unwrap().mode().parameters[2].clone();
        run(&mut c, "Fixture 1");
        c.set_like(&red, u16::MAX);
        run(&mut c, "Store Preset 4.1");
        run(&mut c, "ClearAll");
        run(&mut c, "Fixture 1");
        run(&mut c, "Preset 4.1");
        assert!(c.programmer_presets().contains(&(FeatureGroup::Color, 1)));
        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "ClearAll");
        assert!(c.programmer_presets().is_empty());
        run(&mut c, "Fixture 2 At 50");
        run(&mut c, "Store Cue 2 Fade 0");
        run(&mut c, "ClearAll");

        // La cue 2 ne touche pas la fixture 1 : le preset reste joué par tracking.
        let used = c.presets_in_use(&BTreeMap::from([(1, 1)]));
        assert_eq!(used.get(&(FeatureGroup::Color, 1)), Some(&vec![1]));
        assert!(c.presets_in_use(&BTreeMap::new()).is_empty());
    }

    #[test]
    fn executors_and_cue_errors() {
        let mut c = console();
        assert!(run(&mut c, "Executor 2").message.contains("vide"));
        assert_eq!(c.execute("Store Cue").unwrap_err().to_string(), "programmer vide : rien à enregistrer");
        run(&mut c, "Fixture 4 At 100");
        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "ClearAll");
        assert_eq!(c.show.executors[&2].sequence, 1, "première séquence créée pour l'executor 2");

        run(&mut c, "Go Executor 2");
        assert_eq!(dmx(&c, 44), 255);
        run(&mut c, "Executor 2 At 50");
        assert_eq!(dmx(&c, 44), 128, "le fader dose l'intensité");

        let err = |c: &mut Console, line| c.execute(line).unwrap_err().to_string();
        assert_eq!(err(&mut c, "Go Executor 9"), "l'executor 9 n'a pas de séquence");
        assert_eq!(err(&mut c, "Goto Cue 7"), "la cue 7 n'existe pas");
        assert!(run(&mut c, "Delete Cue 1").show_changed);
        assert_eq!(err(&mut c, "Delete Cue 1"), "la cue 1 n'existe pas");
    }

    #[test]
    fn effects_run_in_programmer_and_track_in_cues() {
        let mut c = console();
        run(&mut c, "Fixture 1 Thru 4");
        let applied = run(&mut c, "Effect 3"); // Vague : sinus avec éventail de 360°
        assert_eq!(applied.message, "Effet « Vague » sur 4 cible(s)");
        assert_eq!(settled(&c).running_effects, 1);

        // Chaque MH 6 reçoit son dimmer avec un quart de période de décalage.
        let wave = crate::effects::instance(&c.programmer_effects()[0], 1, &c.show.patch);
        let phases: Vec<f32> = wave.lines[0].params.iter().map(|(_, p)| *p).collect();
        assert_eq!(phases, [0.0, 0.25, 0.5, 0.75]);

        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "ClearAll");
        assert_eq!(settled(&c).running_effects, 0, "Clear arrête les effets du programmer");
        run(&mut c, "Fixture 1 At 50");
        run(&mut c, "Store Cue 2 Fade 0");
        run(&mut c, "ClearAll");

        let cue = |n| &c.show.sequences[&1].cues[&CueNumber::whole(n)];
        assert!(cue(1).effects.as_ref().is_some_and(|e| e.len() == 1));
        assert_eq!(cue(2).effects, None, "la cue 2 n'enregistre pas d'effet : celui de la cue 1 continue");
        let states = cues::tracked_states(1, &c.show.sequences[&1], &c.show.patch, &c.show.presets);
        assert_eq!(states[1].effects.len(), 1);
        assert_eq!(states[0].effects[0].id, states[1].effects[0].id, "même effet, même phase");

        run(&mut c, "Go");
        assert_eq!(settled(&c).running_effects, 1, "l'executor joue l'effet de la cue");
        run(&mut c, "Off");
        assert_eq!(settled(&c).running_effects, 0);
    }

    #[test]
    fn tempo_is_saved_and_reaches_the_engine() {
        let mut c = console();
        assert!(run(&mut c, "Tempo 98").show_changed);
        assert_eq!(c.show.tempo, 98.0);
        assert_eq!(settled(&c).tempo, 98.0);
        assert!(c.execute("Effect 9").unwrap_err().to_string().contains("n'existe pas"));
        assert_eq!(c.execute("Effect 1").unwrap_err().to_string(), "aucune sélection");
    }

    #[test]
    fn executor_modes_and_management() {
        use folie_show::ExecutorMode;
        let mut c = console();
        assert!(c.press(3).unwrap_err().to_string().contains("Store, puis cliquez-le"));

        run(&mut c, "Fixture 1 At 100");
        assert!(run(&mut c, "Store Executor 3").message.starts_with("Cue 1 enregistrée sur l'executor 3"));
        run(&mut c, "ClearAll");
        // Store Executor sélectionne l'executor : on peut retoucher sa cue tout de suite.
        c.set_cue_times(CueNumber::whole(1), 0.0, 0.0).unwrap();
        c.set_executor_mode(3, ExecutorMode::Toggle).unwrap();
        c.press(3).unwrap();
        assert_eq!(dmx(&c, 2), 255, "bascule : allumé");
        c.press(3).unwrap();
        assert_eq!(dmx(&c, 2), 0, "bascule : éteint");

        c.set_executor_mode(3, ExecutorMode::Flash).unwrap();
        c.flash(3, true);
        assert_eq!(dmx(&c, 2), 255);
        c.flash(3, false);
        assert_eq!(dmx(&c, 2), 0);

        // Options : protégé, l'executor 3 résiste à Off All ; l'executor 1, non.
        c.set_executor_mode(3, ExecutorMode::Go).unwrap();
        let protected = folie_show::ExecutorOptions { protected: true, ..Default::default() };
        c.set_executor_options(3, protected).unwrap();
        run(&mut c, "Fixture 2 At 100");
        run(&mut c, "Store Cue Fade 0");
        run(&mut c, "Executor 1");
        run(&mut c, "Store Cue Fade 0");
        run(&mut c, "ClearAll");
        run(&mut c, "Go Executor 1");
        run(&mut c, "Go Executor 3");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (255, 255));
        assert_eq!(run(&mut c, "Off All").message, "Executors éteints, sauf 1 protégé(s)");
        assert_eq!(dmx(&c, 2), 255, "executor 3 protégé : toujours allumé");
        run(&mut c, "Off Executor 3");
        c.clear_executor(1).unwrap();

        c.rename_executor(3, "Face".into()).unwrap();
        assert_eq!(c.executor(3).unwrap().1.name, "Face");
        c.assign_sequence(4, 1).unwrap();
        assert_eq!(c.clear_executor(3).unwrap().message, "Executor 3 vidé", "la séquence sert encore à l'executor 4");
        assert_eq!(c.clear_executor(4).unwrap().message, "Executor 4 vidé, séquence supprimée");
        assert!(c.show.sequences.is_empty());
    }

    #[test]
    fn executor_pages_and_fixed_executors() {
        let mut c = console();
        let err = |c: &mut Console, line| c.execute(line).unwrap_err().to_string();
        run(&mut c, "Fixture 4 At 100");
        run(&mut c, "Executor 1");
        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "ClearAll");

        // Page 2 : l'executor 1 y est un autre executor, encore vide.
        assert_eq!(run(&mut c, "Page 2").message, "Page 2");
        assert_eq!(c.executor_at(1), 101);
        assert_eq!(err(&mut c, "Go Executor 1"), "l'executor 2.1 n'a pas de séquence");
        run(&mut c, "Fixture 1 At 100");
        assert_eq!(run(&mut c, "Store Executor 1").message, "Cue 1 enregistrée sur l'executor 2.1 (fondu 2 s)");
        run(&mut c, "ClearAll");
        assert!(c.show.executors.contains_key(&101));

        // Un numéro page.place vise une autre page sans quitter la page courante.
        run(&mut c, "Go Executor 1.1");
        assert_eq!(dmx(&c, 44), 255, "l'executor 1 de la page 1 joue");
        run(&mut c, "Page -");
        assert_eq!(c.page(), 1);
        assert_eq!(dmx(&c, 44), 255, "changer de page n'éteint rien");
        run(&mut c, "Page -");
        assert_eq!(c.page(), 1, "pas de page 0");

        // Fix : l'executor 1 de la page 1 reste à sa place sur la page 2.
        assert_eq!(run(&mut c, "Fix Executor 1").message, "Executor 1 fixé sur toutes les pages");
        run(&mut c, "Page +");
        assert_eq!(c.executor_at(1), 1, "l'executor fixé l'emporte sur celui de la page");
        run(&mut c, "Off Executor 1");
        assert_eq!(dmx(&c, 44), 0);
        assert_eq!(
            err(&mut c, "Fix Executor 2.1"),
            "l'executor 1 est déjà fixé à la place 1",
            "une seule place fixée par numéro"
        );
        assert_eq!(run(&mut c, "Fix 1").message, "Executor 1 rendu à sa page");
        assert_eq!(c.executor_at(1), 101);

        assert!(c.rename_page(2, " Acte 2 ".into()).show_changed);
        assert_eq!(c.page_name(2), "Acte 2");
        assert_eq!(run(&mut c, "Page 2").message, "Page 2 « Acte 2 »");
    }

    #[test]
    fn release_is_stored_in_cues() {
        let mut c = console();
        run(&mut c, "Fixture 1 + 2 At 100");
        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "ClearAll");
        assert_eq!(c.execute("Release").unwrap_err().to_string(), "aucune sélection à relâcher");
        assert_eq!(run(&mut c, "Fixture 1 Release").message, "11 paramètres relâchés : Store les enregistre dans la cue");
        assert_eq!(c.released().len(), 11);
        // Une valeur saisie ensuite n'est plus relâchée.
        run(&mut c, "Fixture 1 At 50");
        assert_eq!(c.released().len(), 10);
        run(&mut c, "Fixture 1 Release");
        run(&mut c, "Store Cue 2 Fade 0");
        let cue = &c.selected_sequence().unwrap().1.cues[&CueNumber::whole(2)];
        assert_eq!(cue.values[&FixtureId(1)]["Dimmer"], CueValue::Release);
        run(&mut c, "ClearAll");
        assert!(c.released().is_empty(), "Clear vide aussi les relâchements");

        run(&mut c, "Go");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (255, 255));
        run(&mut c, "Go");
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (0, 255), "cue 2 : la fixture 1 n'est plus tenue");
    }

    #[test]
    fn swop_kill_assert_and_chaser_options() {
        use folie_show::{ExecutorMode, ExecutorOptions};
        let mut c = console();
        run(&mut c, "Fixture 1 At 100");
        run(&mut c, "Store Executor 1");
        run(&mut c, "ClearAll");
        run(&mut c, "Fixture 1 At 50");
        run(&mut c, "Fixture 2 At 100");
        run(&mut c, "Store Executor 2");
        run(&mut c, "ClearAll");
        for executor in [1, 2] {
            c.select_executor(executor);
            c.set_cue_times(CueNumber::whole(1), 0.0, 0.0).unwrap();
        }

        // Kill : la touche de l'executor 2 éteint l'executor 1.
        c.set_executor_mode(2, ExecutorMode::Kill).unwrap();
        run(&mut c, "Go Executor 1");
        c.press(2).unwrap();
        assert_eq!((dmx(&c, 2), dmx(&c, 16)), (128, 255), "dimmer de la fixture 1 donné par l'executor 2 seul");
        assert_eq!(settled(&c).executors[&1].cue, None);

        // Swop : maintenu, seul l'executor 2 éclaire.
        c.set_executor_mode(2, ExecutorMode::Swop).unwrap();
        run(&mut c, "Go Executor 1");
        c.flash(2, true);
        assert_eq!(dmx(&c, 2), 128);
        c.flash(2, false);
        assert_eq!(dmx(&c, 2), 255);

        // Assert en ligne de commande.
        assert_eq!(run(&mut c, "Assert Executor 1").message, "Assert executor 1");
        assert!(c.execute("Assert Executor 9").is_err());

        // Les options du chenillard arrivent au moteur : il avance seul.
        run(&mut c, "Fixture 3 At 100");
        run(&mut c, "Store Executor 1");
        run(&mut c, "ClearAll");
        let mut options = ExecutorOptions { chaser: true, ..Default::default() };
        options.chaser_settings.bpm = 600.0;
        c.set_executor_options(1, options).unwrap();
        run(&mut c, "Off Executor 1");
        run(&mut c, "Go Executor 1");
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..20 {
            seen.extend(settled(&c).executors[&1].cue);
        }
        assert_eq!(seen.len(), 2, "les deux pas ont défilé");
    }

    #[test]
    fn attribute_values_absolute_and_relative() {
        let mut c = console();
        let index = |c: &Console, name: &str| {
            c.show.patch.get(FixtureId(1)).unwrap().mode().parameters.iter().position(|p| p.name == name).unwrap()
        };
        let programmed = |c: &Console, fixture: u32, name: &str| {
            let key = ParamKey { fixture: FixtureId(fixture), parameter: index(c, name) };
            c.programmer().get(&key).map(|v| (*v as f32 / u16::MAX as f32 * 100.0).round())
        };
        assert_eq!(run(&mut c, r#"Fixture 1 Thru 2 Attribute "Pan" At 25"#).message, "Pan à 25 % sur 2 cibles");
        assert_eq!((programmed(&c, 1, "Pan"), programmed(&c, 2, "Pan")), (Some(25.0), Some(25.0)));
        assert_eq!(programmed(&c, 1, "Dimmer"), None, "seul le Pan est touché");

        // Relatif : depuis ce qui sort (0), puis depuis le programmer.
        assert_eq!(run(&mut c, "At + 10").message, "Dimmer + 10 % sur 2 cibles");
        assert_eq!(programmed(&c, 1, "Dimmer"), Some(10.0));
        run(&mut c, "At + 10");
        assert_eq!(programmed(&c, 2, "Dimmer"), Some(20.0));
        assert_eq!(run(&mut c, "At - 50").message, "Dimmer - 50 % sur 2 cibles");
        assert_eq!(programmed(&c, 1, "Dimmer"), Some(0.0), "jamais sous 0");
        run(&mut c, r#"attribute "pan" at + 10"#);
        assert_eq!(programmed(&c, 1, "Pan"), Some(35.0), "nom sans tenir compte des majuscules");

        let err = c.execute(r#"Attribute "Couleur magique" At 5"#).unwrap_err().to_string();
        assert_eq!(err, "aucun paramètre « Couleur magique » dans la sélection");
        assert!(c.execute("Executor 1 At + 10").is_err(), "un fader prend une valeur absolue");
    }

    #[test]
    fn relative_circle_turns_around_the_current_position() {
        let mut c = console();
        run(&mut c, r#"Fixture 1 Attribute "Pan" At 50"#);
        run(&mut c, r#"Attribute "Tilt" At 20"#);
        run(&mut c, "Tempo 600"); // un cycle de 4 temps en 0,4 s
        run(&mut c, "Effect 4"); // Cercle, relatif : ±10 % autour de la position
        // MH 6 : Pan sur les canaux 10-11, Tilt sur 12-13.
        let samples: Vec<(u8, u8)> = (0..8).map(|_| (dmx(&c, 10), dmx(&c, 12))).collect();
        let (pans, tilts): (Vec<u8>, Vec<u8>) = samples.into_iter().unzip();
        assert!(pans.iter().all(|p| (100..=155).contains(p)), "pan autour de 50 % (128) : {pans:?}");
        assert!(tilts.iter().all(|t| (25..=78).contains(t)), "tilt autour de 20 % (51) : {tilts:?}");
        assert!(pans.iter().max() != pans.iter().min(), "le pan bouge : {pans:?}");
    }

    #[test]
    fn sparkle_differs_from_one_fixture_to_the_next() {
        let mut c = console();
        run(&mut c, "Fixture 1 Thru 4");
        run(&mut c, "Effect 7"); // Scintillement : dimmer aléatoire entre 40 et 100 %
        // Dimmer des MH 6 : canal 2 de chaque fixture (adresses 1, 15, 29, 43).
        let levels: Vec<u8> = [2, 16, 30, 44].iter().map(|ch| dmx(&c, *ch)).collect();
        assert!(levels.iter().all(|v| (100..=255).contains(v)), "entre 40 et 100 % : {levels:?}");
        let mut distinct = levels.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(distinct.len() > 1, "chaque projecteur tire sa valeur : {levels:?}");
    }

    #[test]
    fn one_effect_stops_in_a_cue_the_others_go_on() {
        let mut c = console();
        let cue = |n: u32| folie_show::CueNumber::whole(n);
        run(&mut c, "Fixture 1 Thru 4 At 50");
        run(&mut c, "Effect 3"); // Vague
        run(&mut c, "Effect 4"); // Cercle
        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "ClearAll");
        for n in [2, 3] {
            run(&mut c, "Fixture 1 At 80");
            run(&mut c, &format!("Store Cue {n} Fade 0"));
            run(&mut c, "ClearAll");
        }
        let names = |c: &Console, n: u32| c.cue_effects(cue(n)).unwrap().into_iter().map(|e| (e.name, e.here, e.stopped)).collect::<Vec<_>>();
        assert_eq!(names(&c, 1), [("Vague".to_string(), true, false), ("Cercle".to_string(), true, false)]);
        assert_eq!(names(&c, 2), [("Vague".to_string(), false, false), ("Cercle".to_string(), false, false)]);

        // Le Cercle s'arrête dans la cue 2 ; la Vague continue, avec le même identifiant.
        assert_eq!(c.set_effect_stopped(cue(2), cue(1), 1, true).unwrap().message, "Cue 2 : effet « Cercle » arrêté");
        let played = crate::cues::playback_data(&c.show);
        let ids = |cue: usize| played[&1][cue].effects.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(1).len(), 1);
        assert_eq!(ids(1)[0], ids(0)[0], "la Vague garde sa phase");
        assert_eq!(ids(2), ids(1), "l'arrêt se suit dans la cue 3");
        assert_eq!(names(&c, 2)[1], ("Cercle".to_string(), false, true));
        assert!(c.set_effect_stopped(cue(1), cue(1), 0, true).is_err(), "un effet lancé ici ne se suit pas");

        c.set_effect_stopped(cue(2), cue(1), 1, false).unwrap();
        assert_eq!(crate::cues::playback_data(&c.show)[&1][1].effects.len(), 2, "repris");
    }

    #[test]
    fn a_stopped_effect_fades_out() {
        let mut c = console();
        run(&mut c, "Fixture 1 At 0");
        run(&mut c, "Effect 1"); // Sinus dimmer, figé à 100 % pour le test
        {
            let effect = &mut c.programmer_effects_mut()[0];
            effect.rate = folie_show::EffectRate::Hz(0.0);
            effect.lines[0].low = 100.0;
            effect.fade_out = 1.0;
        }
        c.effects_changed();
        assert_eq!(dmx(&c, 2), 255);
        c.remove_effect(0);
        let fading = dmx(&c, 2);
        assert!((100..255).contains(&fading), "l'effet s'efface au lieu de couper : {fading}");
        std::thread::sleep(Duration::from_millis(1100));
        assert_eq!(dmx(&c, 2), 0, "effacé : le dimmer revient au programmer (0)");
    }

    #[test]
    fn my_effects_pool() {
        let mut c = console();
        run(&mut c, "Fixture 1 Thru 2");
        run(&mut c, "Effect 4");
        c.programmer_effects_mut()[0].spread = 180.0;
        let stored = c.store_effect_in_pool(0).unwrap();
        assert_eq!(stored.message, "Effet « Cercle » enregistré dans Mes effets (1)");
        assert!(stored.show_changed);
        assert!(c.show.effect_pool[&1].targets.is_empty(), "sans cibles");
        c.rename_pool_effect(1, "Ballet".into()).unwrap();
        run(&mut c, "ClearAll");

        run(&mut c, "Fixture 3 Thru 4");
        assert_eq!(c.apply_pool_effect(1).unwrap().message, "Effet « Ballet » sur 2 cible(s)");
        let effect = &c.programmer_effects()[0];
        assert_eq!((effect.spread, effect.targets.len()), (180.0, 2), "réglages gardés, nouvelles cibles");
        c.store_effect_in_pool(0).unwrap();
        assert!(c.show.effect_pool.contains_key(&2), "case suivante");
        c.delete_pool_effect(1).unwrap();
        assert!(c.apply_pool_effect(1).is_err());
    }
}

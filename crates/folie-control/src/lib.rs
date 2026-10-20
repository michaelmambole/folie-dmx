//! Couche de contrôle de Folie-DMX.
//!
//! La [`Console`] tient le show et la sélection, et traduit les actions de
//! l'utilisateur en commandes pour le moteur. Toutes les interfaces passent par
//! elle (écran, et plus tard OSC et web), si bien qu'une fonction ajoutée ici est
//! disponible partout à la fois.

pub mod command;

use std::sync::Arc;

use folie_engine::{Command as EngineCommand, EngineHandle, Snapshot};
use folie_fixture::{Attribute, Capability, CapabilityKind, FeatureGroup, ParamKey, Parameter, Patch, Target};
use folie_show::{Group, Preset, Show};
use thiserror::Error;

use command::{Command, Item, Object, Selection, SyntaxError, Value};

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

pub struct Console {
    pub show: Show,
    /// Sélection dans l'ordre où elle a été faite.
    selection: Vec<Target>,
    /// Après une valeur ou un preset, la sélection suivante remplace au lieu
    /// d'ajouter, comme sur grandMA.
    selection_used: bool,
    engine: EngineHandle,
}

impl Console {
    pub fn new(engine: EngineHandle) -> Self {
        Self { show: Show::default(), selection: Vec::new(), selection_used: false, engine }
    }

    pub fn engine(&self) -> &EngineHandle {
        &self.engine
    }

    pub fn selection(&self) -> &[Target] {
        &self.selection
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
        self.engine.send(EngineCommand::ClearProgrammer);
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
        self.engine.send(EngineCommand::SetPatch(Arc::new(patch.clone())));
    }

    fn send_value(&self, target: Target, parameter: usize, value: u16) {
        let key = ParamKey { fixture: target.fixture, parameter };
        self.engine.send(EngineCommand::SetAttribute { key, value });
    }

    /// Donne à toute la sélection la valeur d'un paramètre de référence.
    pub fn set_like(&mut self, reference: &Parameter, value: u16) {
        for &target in &self.selection {
            for parameter in matching(&self.show.patch, target, reference) {
                self.send_value(target, parameter, value);
            }
        }
        self.selection_used = true;
    }

    /// Dimmer à 100 % et shutter ouvert sur la sélection.
    pub fn full(&mut self) {
        for &target in &self.selection {
            for (index, p) in self.show.patch.parameters_of(target) {
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
    pub fn release_selection(&self) {
        for &target in &self.selection {
            for (parameter, _) in self.show.patch.parameters_of(target) {
                let key = ParamKey { fixture: target.fixture, parameter };
                self.engine.send(EngineCommand::ReleaseAttribute(key));
            }
        }
    }

    pub fn execute(&mut self, line: &str) -> Result<Outcome, CommandError> {
        match command::parse(line)? {
            Command::Select { selection, value } => {
                let mut message = String::new();
                if let Some(selection) = selection {
                    let targets = self.resolve(&selection)?;
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
                    message = format!("{} sélectionnée(s)", plural(self.selection.len(), "cible"));
                }
                if let Some(Value::Percent(percent)) = value {
                    message = self.set_dimmer(percent)?;
                }
                Ok(Outcome::info(message))
            }
            Command::ApplyPreset(family, number) => self.apply_preset(family, number),
            Command::Store { object: Object::Group(number), name } => self.store_group(number, name),
            Command::Store { object: Object::Preset(family, number), name } => {
                self.store_preset(family, number, name)
            }
            Command::Label { object, name } => self.label(object, name),
            Command::Delete(object) => self.delete(object),
            Command::Clear => {
                if self.selection.is_empty() {
                    self.engine.send(EngineCommand::ClearProgrammer);
                    Ok(Outcome::info("Programmer vidé"))
                } else {
                    self.selection.clear();
                    Ok(Outcome::info("Sélection vidée (Clear à nouveau pour vider le programmer)"))
                }
            }
            Command::ClearAll => {
                self.selection.clear();
                self.engine.send(EngineCommand::ClearProgrammer);
                Ok(Outcome::info("Sélection et programmer vidés"))
            }
        }
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

    fn set_dimmer(&mut self, percent: f32) -> Result<String, CommandError> {
        if self.selection.is_empty() {
            return invalid("aucune sélection");
        }
        let value = (percent / 100.0 * u16::MAX as f32).round() as u16;
        let mut touched = 0;
        for &target in &self.selection {
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
        let snapshot: Snapshot = self.engine.snapshot();
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
                if let Some(value) = snapshot.attributes.get(&key) {
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

    fn label(&mut self, object: Object, name: String) -> Result<Outcome, CommandError> {
        let slot = match object {
            Object::Group(n) => self.show.groups.get_mut(&n).map(|g| &mut g.name),
            Object::Preset(f, n) => self.show.presets.get_mut(&(f, n)).map(|p| &mut p.name),
        };
        match slot {
            Some(slot) => {
                *slot = name.clone();
                Ok(Outcome::changed(format!("Renommé « {name} »")))
            }
            None => invalid("rien à renommer à ce numéro"),
        }
    }

    fn delete(&mut self, object: Object) -> Result<Outcome, CommandError> {
        let removed = match object {
            Object::Group(n) => self.show.groups.remove(&n).map(|g| format!("Groupe {n} « {} » supprimé", g.name)),
            Object::Preset(f, n) => self
                .show
                .presets
                .remove(&(f, n))
                .map(|p| format!("Preset {}.{n} « {} » supprimé", f.preset_number(), p.name)),
        };
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
    use folie_engine::{EngineConfig, OutputBinding};
    use folie_fixture::{FixtureId, Library, PatchedFixture};

    use super::*;
    use crate::command::fixture;

    /// 4 MH 6 (1 à 4) et un Sunstrip 10 dimmers (10).
    fn console() -> Console {
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

    fn run(console: &mut Console, line: &str) -> Outcome {
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
        // Rouge sur la fixture 1 : on règle le canal Red puis on enregistre la couleur.
        let red = c.show.patch.get(FixtureId(1)).unwrap().mode().parameters[2].clone();
        assert_eq!(red.name, "Red");
        run(&mut c, "Fixture 1");
        c.set_like(&red, u16::MAX);
        settled(&c);
        let stored = run(&mut c, r#"Store Preset 4.1 "Rouge""#);
        assert_eq!(stored.message, "Preset 4.1 « Rouge » enregistré (1 fixture)");
        assert!(c.execute("Store Preset 2.1").is_err(), "aucune valeur de position");

        // Appliqué à la fixture 1 seulement : elle est la seule dans le preset.
        run(&mut c, "ClearAll");
        settled(&c);
        assert_eq!(dmx(&c, 4), 0, "Red par défaut à 0 sur la MH 6");
        let applied = run(&mut c, "Preset 4.1");
        assert_eq!(c.selection(), [fixture(1)], "sans sélection : les fixtures du preset");
        assert!(applied.message.contains("appliqué à 1 cible"));
        assert_eq!(dmx(&c, 4), 255);

        run(&mut c, r#"Label Preset 4.1 "Cyan""#);
        assert_eq!(c.show.presets[&(FeatureGroup::Color, 1)].name, "Cyan");
        run(&mut c, "Delete Preset 4.1");
        assert!(c.show.presets.is_empty());
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
}

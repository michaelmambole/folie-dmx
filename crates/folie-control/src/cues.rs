//! Cues et executors : enregistrement, tracking, lecture.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use folie_engine::{Command as EngineCommand, CueState, Sequences};
use folie_fixture::{FeatureGroup, FixtureId, ParamKey, Patch};
use folie_show::{Cue, CueNumber, CueValue, Preset, Sequence, Show};

use crate::{CommandError, Console, Outcome, invalid};

/// Valeur d'une cue telle qu'elle sortira : la référence à un preset est
/// résolue maintenant, si bien que modifier le preset modifie la cue.
fn resolve(value: CueValue, fixture: FixtureId, parameter: &str, presets: &BTreeMap<(FeatureGroup, u32), Preset>) -> Option<u16> {
    match value {
        CueValue::Value(v) => Some(v),
        CueValue::Preset(family, number) => {
            presets.get(&(family, number))?.values.get(&fixture)?.get(parameter).copied()
        }
        CueValue::Release => None,
    }
}

/// État complet de chaque cue : ce qu'elle change, plus tout ce qui est hérité
/// des cues précédentes (tracking). `Release` retire un paramètre.
pub fn tracked_states(sequence: &Sequence, patch: &Patch, presets: &BTreeMap<(FeatureGroup, u32), Preset>) -> Vec<CueState> {
    let mut state: HashMap<ParamKey, u16> = HashMap::new();
    let mut states = Vec::new();
    for (number, cue) in &sequence.cues {
        for (fixture, parameters) in &cue.values {
            let Some(patched) = patch.get(*fixture) else { continue };
            for (name, value) in parameters {
                let Some(index) = patched.mode().parameters.iter().position(|p| p.name == *name) else { continue };
                let key = ParamKey { fixture: *fixture, parameter: index };
                match resolve(*value, *fixture, name, presets) {
                    Some(v) => {
                        state.insert(key, v);
                    }
                    None => {
                        state.remove(&key);
                    }
                }
            }
        }
        let label = if cue.name.is_empty() { number.to_string() } else { format!("{number} {}", cue.name) };
        states.push(CueState { label, values: state.clone(), fade: cue.fade, delay: cue.delay });
    }
    states
}

/// Séquences jouées par chaque executor, prêtes pour le moteur.
pub fn playback_data(show: &Show) -> Sequences {
    show.executors
        .iter()
        .filter_map(|(executor, sequence)| {
            let sequence = show.sequences.get(sequence)?;
            Some((*executor, Arc::new(tracked_states(sequence, &show.patch, &show.presets))))
        })
        .collect()
}

impl Console {
    /// À appeler après toute modification des cues, des presets ou du patch.
    pub fn sync_playback(&self) {
        self.engine.send(EngineCommand::SetSequences(Arc::new(playback_data(&self.show))));
    }

    pub fn selected_executor(&self) -> u32 {
        self.selected_executor
    }

    pub fn select_executor(&mut self, executor: u32) -> Outcome {
        self.selected_executor = executor;
        let what = match self.show.executors.get(&executor).and_then(|s| self.show.sequences.get(s)) {
            Some(sequence) => format!(" (séquence « {} »)", sequence.name),
            None => " (vide)".into(),
        };
        Outcome::info(format!("Executor {executor} sélectionné{what}"))
    }

    /// Séquence de l'executor sélectionné, créée au premier `Store Cue`.
    pub fn selected_sequence(&self) -> Option<(u32, &Sequence)> {
        let number = *self.show.executors.get(&self.selected_executor)?;
        Some((number, self.show.sequences.get(&number)?))
    }

    fn selected_sequence_mut(&mut self) -> Result<&mut Sequence, CommandError> {
        let executor = self.selected_executor;
        let Some(number) = self.show.executors.get(&executor).copied() else {
            return invalid(format!("l'executor {executor} n'a pas de séquence"));
        };
        self.show.sequences.get_mut(&number).ok_or_else(|| CommandError::Invalid(format!("séquence {number} introuvable")))
    }

    /// Enregistre le programmer dans la séquence de l'executor sélectionné.
    /// Une cue existante est complétée (fusion) ; les valeurs venues d'un preset
    /// restent des références au preset.
    pub fn store_cue(
        &mut self,
        number: Option<CueNumber>,
        fade: Option<f32>,
        name: Option<String>,
    ) -> Result<Outcome, CommandError> {
        if self.programmer.is_empty() {
            return invalid("programmer vide : rien à enregistrer");
        }
        let mut values: BTreeMap<FixtureId, BTreeMap<String, CueValue>> = BTreeMap::new();
        for (key, value) in &self.programmer {
            let Some(parameter) = self.show.patch.get(key.fixture).and_then(|f| f.mode().parameters.get(key.parameter))
            else {
                continue;
            };
            let value = match self.origins.get(key) {
                Some(&(family, preset)) if self.show.presets.contains_key(&(family, preset)) => {
                    CueValue::Preset(family, preset)
                }
                _ => CueValue::Value(*value),
            };
            values.entry(key.fixture).or_default().insert(parameter.name.clone(), value);
        }

        let executor = self.selected_executor;
        let sequence_number = match self.show.executors.get(&executor) {
            Some(n) => *n,
            None => {
                let n = (1..).find(|n| !self.show.sequences.contains_key(n)).unwrap_or(executor);
                self.show.sequences.insert(n, Sequence { name: format!("Séquence {n}"), cues: BTreeMap::new() });
                self.show.executors.insert(executor, n);
                n
            }
        };
        let sequence = self.show.sequences.get_mut(&sequence_number).expect("séquence créée juste avant");
        let number = number.unwrap_or_else(|| {
            sequence.cues.keys().next_back().map_or(CueNumber::whole(1), |last| last.next_whole())
        });
        let merged = sequence.cues.contains_key(&number);
        let cue = sequence.cues.entry(number).or_insert_with(|| Cue::new(String::new()));
        for (fixture, parameters) in values {
            cue.values.entry(fixture).or_default().extend(parameters);
        }
        if let Some(name) = name {
            cue.name = name;
        }
        if let Some(fade) = fade {
            cue.fade = fade;
        }
        let verb = if merged { "complétée" } else { "enregistrée" };
        let message = format!("Cue {number} {verb} sur l'executor {executor} (fondu {} s)", cue.fade);
        self.sync_playback();
        Ok(Outcome::changed(message))
    }

    pub fn delete_cue(&mut self, number: CueNumber) -> Result<Outcome, CommandError> {
        let removed = self.selected_sequence_mut()?.cues.remove(&number);
        if removed.is_none() {
            return invalid(format!("la cue {number} n'existe pas"));
        }
        self.sync_playback();
        Ok(Outcome::changed(format!("Cue {number} supprimée")))
    }

    pub fn label_cue(&mut self, number: CueNumber, name: String) -> Result<Outcome, CommandError> {
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        cue.name = name.clone();
        self.sync_playback();
        Ok(Outcome::changed(format!("Cue {number} renommée « {name} »")))
    }

    /// Temps d'une cue, depuis la fenêtre Cues.
    pub fn set_cue_times(&mut self, number: CueNumber, fade: f32, delay: f32) -> Result<Outcome, CommandError> {
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        cue.fade = fade.max(0.0);
        cue.delay = delay.max(0.0);
        let message = format!("Cue {number} : fondu {} s, délai {} s", cue.fade, cue.delay);
        self.sync_playback();
        Ok(Outcome::changed(message))
    }

    fn executor_with_sequence(&self, executor: Option<u32>) -> Result<u32, CommandError> {
        let executor = executor.unwrap_or(self.selected_executor);
        if self.show.executors.contains_key(&executor) {
            Ok(executor)
        } else {
            invalid(format!("l'executor {executor} n'a pas de séquence"))
        }
    }

    pub fn go(&self, executor: Option<u32>) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(executor)?;
        self.engine.send(EngineCommand::Go(executor));
        Ok(Outcome::info(format!("Go executor {executor}")))
    }

    pub fn back(&self, executor: Option<u32>) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(executor)?;
        self.engine.send(EngineCommand::Back(executor));
        Ok(Outcome::info(format!("Back executor {executor}")))
    }

    pub fn off(&self, executor: Option<u32>) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(executor)?;
        self.engine.send(EngineCommand::Off(executor));
        Ok(Outcome::info(format!("Executor {executor} éteint")))
    }

    pub fn goto(&self, number: CueNumber) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(None)?;
        let Some((_, sequence)) = self.selected_sequence() else { return invalid("aucune séquence") };
        let Some(index) = sequence.cues.keys().position(|n| *n == number) else {
            return invalid(format!("la cue {number} n'existe pas"));
        };
        self.engine.send(EngineCommand::Goto { executor, cue: index });
        Ok(Outcome::info(format!("Goto cue {number}")))
    }

    pub fn set_fader(&self, executor: u32, percent: f32) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(Some(executor))?;
        self.engine.send(EngineCommand::SetFader { executor, level: percent / 100.0 });
        Ok(Outcome::info(format!("Executor {executor} à {percent} %")))
    }
}

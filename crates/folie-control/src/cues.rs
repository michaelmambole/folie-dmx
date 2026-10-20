//! Cues et executors : enregistrement, tracking, lecture.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use folie_engine::{ChaserDirection as Direction, ChaserOptions, Command as EngineCommand, CueState, FaderMode, PlaybackOptions, Restart, Sequences};
use folie_fixture::{FeatureGroup, FixtureId, ParamKey, Patch};
use folie_show::{
    ChaserDirection, Cue, CueNumber, CueValue, Effect, Executor, ExecutorMode, ExecutorOptions, FaderFunction, MAX_PAGE, ParamTiming,
    Preset, RestartMode, Sequence, Show, executor_id, executor_label, executor_place,
};

use crate::command::ExecutorRef;
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

/// Identifiant d'un effet de cue : il dépend de la cue où l'effet est né, si
/// bien qu'un effet suivi dans les cues suivantes ne repart pas de zéro à chaque Go.
fn cue_effect_id(executor: u32, origin: usize, index: usize) -> u64 {
    ((executor as u64) << 40) | ((origin as u64) << 16) | index as u64
}

/// État complet de chaque cue : ce qu'elle change, plus tout ce qui est hérité
/// des cues précédentes (tracking). `Release` retire un paramètre. Les effets
/// continuent jusqu'à ce qu'une cue en enregistre d'autres.
pub fn tracked_states(
    executor: u32,
    sequence: &Sequence,
    patch: &Patch,
    presets: &BTreeMap<(FeatureGroup, u32), Preset>,
) -> Vec<CueState> {
    let mut state: HashMap<ParamKey, u16> = HashMap::new();
    let mut effects: (usize, Vec<Effect>) = (0, Vec::new());
    let mut states = Vec::new();
    for (position, (number, cue)) in sequence.cues.iter().enumerate() {
        if let Some(list) = &cue.effects {
            effects = (position, list.clone());
        }
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
        // Temps propres, pour les paramètres que tient la cue.
        let times = state
            .keys()
            .filter_map(|key| {
                let parameter = patch.get(key.fixture)?.mode().parameters.get(key.parameter)?;
                let family = parameter.attribute.group();
                cue.has_own_times(key.fixture, &parameter.name, family)
                    .then(|| (*key, cue.times_of(key.fixture, &parameter.name, family)))
            })
            .collect();
        let label = if cue.name.is_empty() { number.to_string() } else { format!("{number} {}", cue.name) };
        let (origin, list) = &effects;
        let instances = list
            .iter()
            .enumerate()
            .map(|(i, e)| crate::effects::instance(e, cue_effect_id(executor, *origin, i), patch))
            .collect();
        states.push(CueState { label, values: state.clone(), fade: cue.fade, delay: cue.delay, times, effects: instances });
    }
    states
}

/// Séquences jouées par chaque executor, prêtes pour le moteur.
pub fn playback_data(show: &Show) -> Sequences {
    show.executors
        .iter()
        .filter_map(|(executor, x)| {
            let sequence = show.sequences.get(&x.sequence)?;
            Some((*executor, Arc::new(tracked_states(*executor, sequence, &show.patch, &show.presets))))
        })
        .collect()
}

/// Options d'un executor, telles que le moteur les attend.
fn playback_options(o: &ExecutorOptions) -> PlaybackOptions {
    PlaybackOptions {
        wrap: o.wrap,
        auto_start: o.auto_start,
        auto_stop: o.auto_stop,
        fader: match o.fader {
            FaderFunction::Master => FaderMode::Master,
            FaderFunction::Temp => FaderMode::Temp,
            FaderFunction::Speed => FaderMode::Speed,
            FaderFunction::Crossfade => FaderMode::Crossfade,
        },
        off_time: o.off_time,
        back_time: o.back_time,
        priority: o.priority.level(),
        protected: o.protected,
        chaser: o.chaser.then(|| {
            let c = o.chaser_settings;
            ChaserOptions {
                bpm: (!c.follow_tempo).then_some(c.bpm),
                step_fade: c.step_fade,
                direction: match c.direction {
                    ChaserDirection::Forward => Direction::Forward,
                    ChaserDirection::Backward => Direction::Backward,
                    ChaserDirection::Bounce => Direction::Bounce,
                    ChaserDirection::Random => Direction::Random,
                },
            }
        }),
        restart: match o.restart {
            RestartMode::First => Restart::First,
            RestartMode::Current => Restart::Current,
            RestartMode::Next => Restart::Next,
        },
        ooo: o.ooo,
        mib: o.mib,
        swop_protect: o.swop_protect,
        kill_protect: o.kill_protect,
    }
}

impl Console {
    /// À appeler après toute modification des cues, des presets, du patch ou des executors.
    pub fn sync_playback(&self) {
        let options = self.show.executors.iter().map(|(e, x)| (*e, playback_options(&x.options))).collect();
        self.engine.send(EngineCommand::SetExecutorOptions(options));
        self.engine.send(EngineCommand::SetSequences(Arc::new(playback_data(&self.show))));
    }

    /// Presets tenus par la cue en cours de chaque executor, tracking compris :
    /// pour chaque preset, les executors qui le jouent. `current` : executor ->
    /// index de sa cue en cours.
    pub fn presets_in_use(&self, current: &BTreeMap<u32, usize>) -> BTreeMap<(FeatureGroup, u32), Vec<u32>> {
        let mut used: BTreeMap<(FeatureGroup, u32), Vec<u32>> = BTreeMap::new();
        for (executor, index) in current {
            let Some((_, sequence)) = self.executor(*executor) else { continue };
            // Dernière valeur de chaque paramètre jusqu'à la cue en cours.
            let mut state: BTreeMap<(FixtureId, &str), CueValue> = BTreeMap::new();
            for cue in sequence.cues.values().take(index + 1) {
                for (fixture, parameters) in &cue.values {
                    for (name, value) in parameters {
                        state.insert((*fixture, name.as_str()), *value);
                    }
                }
            }
            for value in state.values() {
                if let CueValue::Preset(family, number) = value {
                    let list = used.entry((*family, *number)).or_default();
                    if !list.contains(executor) {
                        list.push(*executor);
                    }
                }
            }
        }
        used
    }

    pub fn set_executor_options(&mut self, executor: u32, options: ExecutorOptions) -> Result<Outcome, CommandError> {
        // Une seule place fixée par numéro : sinon deux executors se disputeraient la même place.
        if options.fixed
            && let Some(other) = self.fixed_at(executor_place(executor).1).filter(|o| *o != executor)
        {
            let place = executor_place(executor).1;
            return invalid(format!("l'executor {} est déjà fixé à la place {place}", executor_label(other)));
        }
        let Some(x) = self.show.executors.get_mut(&executor) else {
            return invalid(format!("l'executor {executor} est vide", executor = executor_label(executor)));
        };
        x.options = options;
        self.sync_playback();
        Ok(Outcome::changed(format!("Options de l'executor {executor} enregistrées", executor = executor_label(executor))))
    }

    /// Éteint tous les executors, sauf ceux qui sont protégés.
    pub fn off_all(&self) -> Outcome {
        self.engine.send(EngineCommand::OffAll);
        let protected = self.show.executors.values().filter(|x| x.options.protected).count();
        match protected {
            0 => Outcome::info("Tous les executors éteints"),
            n => Outcome::info(format!("Executors éteints, sauf {n} protégé(s)")),
        }
    }

    /// Page d'executors affichée.
    pub fn page(&self) -> u32 {
        self.page
    }

    pub fn set_page(&mut self, page: u32) -> Outcome {
        self.page = page.clamp(1, MAX_PAGE);
        let name = self.show.pages.get(&self.page).map(|n| format!(" « {n} »")).unwrap_or_default();
        Outcome::info(format!("Page {}{name}", self.page))
    }

    pub fn page_name(&self, page: u32) -> &str {
        self.show.pages.get(&page).map_or("", String::as_str)
    }

    pub fn rename_page(&mut self, page: u32, name: String) -> Outcome {
        let name = name.trim().to_string();
        if name.is_empty() {
            self.show.pages.remove(&page);
            Outcome::changed(format!("Page {page} sans nom"))
        } else {
            self.show.pages.insert(page, name.clone());
            Outcome::changed(format!("Page {page} renommée « {name} »"))
        }
    }

    /// Executor fixé à cette place, présent sur toutes les pages.
    fn fixed_at(&self, slot: u32) -> Option<u32> {
        self.show.executors.iter().find(|(id, x)| x.options.fixed && executor_place(**id).1 == slot).map(|(id, _)| *id)
    }

    /// Executor affiché à une place de la page courante : celui qui y est fixé, sinon celui de la page.
    pub fn executor_at(&self, slot: u32) -> u32 {
        self.fixed_at(slot).unwrap_or_else(|| executor_id(self.page, slot))
    }

    /// Numéro d'executor désigné en ligne de commande.
    pub fn resolve_executor(&self, executor: ExecutorRef) -> u32 {
        match executor.page {
            Some(page) => executor_id(page, executor.slot),
            None => self.executor_at(executor.slot),
        }
    }

    /// `Fix` : fixe l'executor sur toutes les pages, ou le rend à sa page.
    pub fn toggle_fixed(&mut self, executor: u32) -> Result<Outcome, CommandError> {
        let Some(x) = self.show.executors.get(&executor) else {
            return invalid(format!("l'executor {} est vide", executor_label(executor)));
        };
        let options = ExecutorOptions { fixed: !x.options.fixed, ..x.options };
        self.set_executor_options(executor, options)?;
        let label = executor_label(executor);
        Ok(Outcome::changed(if options.fixed {
            format!("Executor {label} fixé sur toutes les pages")
        } else {
            format!("Executor {label} rendu à sa page")
        }))
    }

    pub fn selected_executor(&self) -> u32 {
        self.selected_executor
    }

    pub fn select_executor(&mut self, executor: u32) -> Outcome {
        self.selected_executor = executor;
        let what = match self.show.executors.get(&executor).and_then(|x| self.show.sequences.get(&x.sequence)) {
            Some(sequence) => format!(" (séquence « {} »)", sequence.name),
            None => " (vide)".into(),
        };
        Outcome::info(format!("Executor {executor} sélectionné{what}", executor = executor_label(executor)))
    }

    /// Séquence de l'executor sélectionné, créée au premier `Store Cue`.
    pub fn selected_sequence(&self) -> Option<(u32, &Sequence)> {
        let number = self.show.executors.get(&self.selected_executor)?.sequence;
        Some((number, self.show.sequences.get(&number)?))
    }

    fn selected_sequence_mut(&mut self) -> Result<&mut Sequence, CommandError> {
        let executor = self.selected_executor;
        let Some(number) = self.show.executors.get(&executor).map(|x| x.sequence) else {
            return invalid(format!("l'executor {executor} n'a pas de séquence", executor = executor_label(executor)));
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
        if self.programmer.is_empty() && self.released.is_empty() && self.effects.is_empty() {
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
        for key in &self.released {
            if let Some(parameter) = self.show.patch.get(key.fixture).and_then(|f| f.mode().parameters.get(key.parameter)) {
                values.entry(key.fixture).or_default().insert(parameter.name.clone(), CueValue::Release);
            }
        }

        let executor = self.selected_executor;
        let sequence_number = match self.show.executors.get(&executor) {
            Some(x) => x.sequence,
            None => {
                let n = (1..).find(|n| !self.show.sequences.contains_key(n)).unwrap_or(executor);
                self.show.sequences.insert(n, Sequence { name: format!("Séquence {n}"), cues: BTreeMap::new() });
                self.show.executors.insert(executor, Executor::new(n));
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
        if !self.effects.is_empty() {
            cue.effects = Some(self.effects.clone());
        }
        let verb = if merged { "complétée" } else { "enregistrée" };
        let message = format!("Cue {number} {verb} sur l'executor {executor} (fondu {} s)", cue.fade, executor = executor_label(executor));
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

    /// Écrit la valeur d'un paramètre dans une cue (valeur, preset ou relâchement),
    /// depuis la fenêtre Tracking.
    pub fn set_cue_value(
        &mut self,
        number: CueNumber,
        fixture: FixtureId,
        parameter: &str,
        value: CueValue,
    ) -> Result<Outcome, CommandError> {
        if self.show.patch.get(fixture).is_none() {
            return invalid(format!("Fixture {fixture} n'est pas patchée"));
        }
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        cue.values.entry(fixture).or_default().insert(parameter.to_string(), value);
        self.sync_playback();
        let what = match value {
            CueValue::Release => "relâché".to_string(),
            CueValue::Preset(family, n) => format!("preset {}.{n}", family.preset_number()),
            CueValue::Value(_) => "réglé".to_string(),
        };
        Ok(Outcome::changed(format!("Cue {number} : {parameter} de la fixture {fixture} {what}")))
    }

    /// Retire un paramètre d'une cue (et ses temps propres) : la cue reprend la valeur suivie.
    pub fn remove_cue_value(&mut self, number: CueNumber, fixture: FixtureId, parameter: &str) -> Result<Outcome, CommandError> {
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        let removed = cue.values.get_mut(&fixture).and_then(|p| p.remove(parameter)).is_some();
        if cue.values.get(&fixture).is_some_and(BTreeMap::is_empty) {
            cue.values.remove(&fixture);
        }
        if let Some(times) = cue.param_times.get_mut(&fixture) {
            times.remove(parameter);
            if times.is_empty() {
                cue.param_times.remove(&fixture);
            }
        }
        if !removed {
            return invalid(format!("la cue {number} ne tient pas {parameter} de la fixture {fixture}"));
        }
        self.sync_playback();
        Ok(Outcome::changed(format!("Cue {number} : {parameter} de la fixture {fixture} retiré")))
    }

    /// Temps propres d'une famille dans une cue ; vides, la famille reprend le temps de la cue.
    pub fn set_family_times(&mut self, number: CueNumber, family: FeatureGroup, timing: ParamTiming) -> Result<Outcome, CommandError> {
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        if timing.is_empty() {
            cue.family_times.remove(&family);
        } else {
            cue.family_times.insert(family, timing);
        }
        self.sync_playback();
        Ok(Outcome::changed(format!("Cue {number} : temps de {}", family.label())))
    }

    /// Temps propres d'un paramètre d'une fixture dans une cue.
    pub fn set_param_times(
        &mut self,
        number: CueNumber,
        fixture: FixtureId,
        parameter: &str,
        timing: ParamTiming,
    ) -> Result<Outcome, CommandError> {
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        if timing.is_empty() {
            if let Some(params) = cue.param_times.get_mut(&fixture) {
                params.remove(parameter);
                if params.is_empty() {
                    cue.param_times.remove(&fixture);
                }
            }
        } else {
            cue.param_times.entry(fixture).or_default().insert(parameter.to_string(), timing);
        }
        self.sync_playback();
        Ok(Outcome::changed(format!("Cue {number} : temps de {parameter}, fixture {fixture}")))
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
            invalid(format!("l'executor {executor} n'a pas de séquence", executor = executor_label(executor)))
        }
    }

    pub fn go(&self, executor: Option<u32>) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(executor)?;
        self.engine.send(EngineCommand::Go(executor));
        Ok(Outcome::info(format!("Go executor {executor}", executor = executor_label(executor))))
    }

    pub fn back(&self, executor: Option<u32>) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(executor)?;
        self.engine.send(EngineCommand::Back(executor));
        Ok(Outcome::info(format!("Back executor {executor}", executor = executor_label(executor))))
    }

    pub fn off(&self, executor: Option<u32>) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(executor)?;
        self.engine.send(EngineCommand::Off(executor));
        Ok(Outcome::info(format!("Executor {executor} éteint", executor = executor_label(executor))))
    }

    /// Assert : l'executor reprend la main sur ce qu'il tient.
    pub fn assert(&self, executor: Option<u32>) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(executor)?;
        self.engine.send(EngineCommand::Assert(executor));
        Ok(Outcome::info(format!("Assert executor {executor}", executor = executor_label(executor))))
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

    pub fn executor(&self, executor: u32) -> Option<(&Executor, &Sequence)> {
        let x = self.show.executors.get(&executor)?;
        Some((x, self.show.sequences.get(&x.sequence)?))
    }

    /// Touche principale de l'executor, selon son mode (Flash : voir [`Console::flash`]).
    pub fn press(&self, executor: u32) -> Result<Outcome, CommandError> {
        let Some((x, _)) = self.executor(executor) else {
            return invalid(format!("l'executor {executor} est vide : Store, puis cliquez-le pour y enregistrer", executor = executor_label(executor)));
        };
        match x.mode {
            ExecutorMode::Go | ExecutorMode::Flash | ExecutorMode::Swop => self.go(Some(executor)),
            ExecutorMode::Toggle => {
                self.engine.send(EngineCommand::Toggle(executor));
                Ok(Outcome::info(format!("Executor {executor} basculé", executor = executor_label(executor))))
            }
            ExecutorMode::Kill => {
                self.engine.send(EngineCommand::Kill(executor));
                Ok(Outcome::info(format!("Kill executor {executor} : les autres s'éteignent", executor = executor_label(executor))))
            }
        }
    }

    /// Touche Flash ou Swop enfoncée ou relâchée, selon le mode de l'executor.
    pub fn flash(&self, executor: u32, pressed: bool) {
        match self.show.executors.get(&executor).map(|x| x.mode) {
            Some(ExecutorMode::Swop) => self.engine.send(EngineCommand::Swop { executor, pressed }),
            Some(_) => self.engine.send(EngineCommand::Flash { executor, pressed }),
            None => {}
        }
    }

    /// Le programmer devient la cue suivante de l'executor (créé s'il est vide).
    pub fn store_in_executor(&mut self, executor: u32) -> Result<Outcome, CommandError> {
        let previous = self.selected_executor;
        self.selected_executor = executor;
        let result = self.store_cue(None, None, None);
        if result.is_err() {
            self.selected_executor = previous;
        }
        result
    }

    pub fn set_executor_mode(&mut self, executor: u32, mode: ExecutorMode) -> Result<Outcome, CommandError> {
        let Some(x) = self.show.executors.get_mut(&executor) else {
            return invalid(format!("l'executor {executor} est vide", executor = executor_label(executor)));
        };
        x.mode = mode;
        self.engine.send(EngineCommand::Off(executor));
        Ok(Outcome::changed(format!("Executor {executor} en mode {}", mode.label(), executor = executor_label(executor))))
    }

    /// Renomme la séquence de l'executor.
    pub fn rename_executor(&mut self, executor: u32, name: String) -> Result<Outcome, CommandError> {
        let Some(sequence) = self.show.executors.get(&executor).map(|x| x.sequence) else {
            return invalid(format!("l'executor {executor} est vide", executor = executor_label(executor)));
        };
        if let Some(s) = self.show.sequences.get_mut(&sequence) {
            s.name = name.clone();
        }
        Ok(Outcome::changed(format!("Séquence {sequence} renommée « {name} »")))
    }

    /// Fait jouer une autre séquence existante à l'executor.
    pub fn assign_sequence(&mut self, executor: u32, sequence: u32) -> Result<Outcome, CommandError> {
        if !self.show.sequences.contains_key(&sequence) {
            return invalid(format!("la séquence {sequence} n'existe pas"));
        }
        self.engine.send(EngineCommand::Off(executor));
        let (mode, options) =
            self.show.executors.get(&executor).map_or((ExecutorMode::Go, ExecutorOptions::default()), |x| (x.mode, x.options));
        self.show.executors.insert(executor, Executor { sequence, mode, options });
        self.sync_playback();
        Ok(Outcome::changed(format!("Executor {executor} : séquence {sequence}", executor = executor_label(executor))))
    }

    /// Libère l'executor. Sa séquence est supprimée si plus aucun executor ne la joue.
    pub fn clear_executor(&mut self, executor: u32) -> Result<Outcome, CommandError> {
        let Some(x) = self.show.executors.remove(&executor) else {
            return invalid(format!("l'executor {executor} est déjà vide", executor = executor_label(executor)));
        };
        self.engine.send(EngineCommand::Off(executor));
        let still_used = self.show.executors.values().any(|o| o.sequence == x.sequence);
        if !still_used {
            self.show.sequences.remove(&x.sequence);
        }
        self.sync_playback();
        let detail = if still_used { "" } else { ", séquence supprimée" };
        Ok(Outcome::changed(format!("Executor {executor} vidé{detail}", executor = executor_label(executor))))
    }

    pub fn set_fader(&self, executor: u32, percent: f32) -> Result<Outcome, CommandError> {
        let executor = self.executor_with_sequence(Some(executor))?;
        self.engine.send(EngineCommand::SetFader { executor, level: percent / 100.0 });
        Ok(Outcome::info(format!("Executor {executor} à {percent} %", executor = executor_label(executor))))
    }
}

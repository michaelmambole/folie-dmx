//! Moteur temps réel de Folie-DMX.
//!
//! Le moteur tourne dans son propre thread à fréquence fixe. Il ne partage
//! rien de mutable avec l'interface : il reçoit des [`Command`] par un canal
//! et publie un [`Snapshot`] en lecture seule à chaque trame. Une interface
//! lente ou bloquée ne peut donc jamais retarder la sortie DMX.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender};
use folie_dmx::{CHANNELS_PER_UNIVERSE, DmxOutput, Universe};
use folie_fixture::{ParamKey, Patch};

/// Fréquence par défaut : une trame DMX complète de 512 canaux dure environ
/// 22,7 ms, soit ~44 trames par seconde au maximum.
pub const DEFAULT_RATE_HZ: f64 = 44.0;

/// Canaux DMX forcés à la main (mode trad), par univers. `None` = non forcé.
pub type ChannelOverrides = Box<[Option<u8>; CHANNELS_PER_UNIVERSE]>;

fn no_overrides() -> ChannelOverrides {
    Box::new([None; CHANNELS_PER_UNIVERSE])
}

#[derive(Debug)]
pub enum Command {
    /// Force un canal DMX, par-dessus les fixtures.
    /// `universe` est en base 0, `channel` en base 1.
    SetChannel { universe: usize, channel: u16, value: u8 },
    /// Rend un canal forcé à son fonctionnement normal.
    ReleaseChannel { universe: usize, channel: u16 },
    /// Valeur 16 bits d'un paramètre de fixture.
    SetAttribute { key: ParamKey, value: u16 },
    /// Rend un paramètre à sa valeur par défaut.
    ReleaseAttribute(ParamKey),
    /// Remplace le patch. Les valeurs des fixtures disparues sont oubliées.
    SetPatch(Arc<Patch>),
    /// Oublie toutes les valeurs saisies : les fixtures reviennent à leurs valeurs par défaut.
    ClearProgrammer,
    Blackout(bool),
    /// Remplace toutes les sorties, sans interrompre le moteur.
    SetOutputs(Vec<OutputBinding>),
    /// Envoie une dernière trame puis arrête le moteur.
    Shutdown,
}

/// Une sortie rattachée à un univers du moteur.
pub struct OutputBinding {
    pub universe: usize,
    pub output: Box<dyn DmxOutput>,
}

impl std::fmt::Debug for OutputBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputBinding")
            .field("universe", &self.universe)
            .field("output", &self.output.name())
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub universes: usize,
    pub rate_hz: f64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { universes: 1, rate_hz: DEFAULT_RATE_HZ }
    }
}

/// État publié par le moteur après chaque trame.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub frame: u64,
    pub blackout: bool,
    pub patch: Arc<Patch>,
    /// Valeurs saisies sur les fixtures. Ce qui n'y figure pas reste à sa valeur par défaut.
    pub attributes: HashMap<ParamKey, u16>,
    pub channel_overrides: Vec<ChannelOverrides>,
    /// Valeurs réellement envoyées (blackout compris).
    pub output: Vec<Universe>,
    pub output_errors: u64,
    /// Trames émises en retard sur leur échéance.
    pub late_frames: u64,
    /// Nom de chaque sortie active.
    pub output_names: Vec<String>,
}

/// Point d'accès au moteur depuis l'interface.
pub struct EngineHandle {
    tx: Sender<Command>,
    snapshot: Arc<Mutex<Snapshot>>,
    thread: Option<JoinHandle<()>>,
}

impl EngineHandle {
    pub fn send(&self, command: Command) {
        // Le canal ne se ferme que si le moteur s'est arrêté : rien à faire de plus.
        let _ = self.tx.send(command);
    }

    pub fn set_channel(&self, universe: usize, channel: u16, value: u8) {
        self.send(Command::SetChannel { universe, channel, value });
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }

    /// Arrête le moteur et attend la fin de son thread.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.send(Command::Shutdown);
            let _ = thread.join();
        }
    }
}

impl Drop for EngineHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn spawn(config: EngineConfig, outputs: Vec<OutputBinding>) -> EngineHandle {
    let (tx, rx) = crossbeam_channel::unbounded();
    let snapshot = Arc::new(Mutex::new(Snapshot::default()));
    let engine = Engine::new(config, outputs, rx, snapshot.clone());
    let thread = std::thread::Builder::new()
        .name("folie-engine".into())
        .spawn(move || engine.run())
        .expect("impossible de démarrer le thread du moteur");
    EngineHandle { tx, snapshot, thread: Some(thread) }
}

struct Engine {
    period: Duration,
    rx: Receiver<Command>,
    published: Arc<Mutex<Snapshot>>,
    outputs: Vec<OutputBinding>,
    /// Contient aussi le programmer (`attributes`, `channel_overrides`), qui deviendra
    /// une pile de couches (executors, effets…) fusionnées en HTP/LTP.
    state: Snapshot,
    shutting_down: bool,
}

impl Engine {
    fn new(
        config: EngineConfig,
        outputs: Vec<OutputBinding>,
        rx: Receiver<Command>,
        published: Arc<Mutex<Snapshot>>,
    ) -> Self {
        let universes = config.universes.max(1);
        let outputs = valid_bindings(outputs, universes);
        let state = Snapshot {
            channel_overrides: vec![no_overrides(); universes],
            output: vec![Universe::new(); universes],
            output_names: outputs.iter().map(|b| b.output.name().to_string()).collect(),
            ..Default::default()
        };
        // L'interface peut lire l'état avant la première trame : il doit déjà être complet.
        published.lock().unwrap().clone_from(&state);
        Self {
            period: Duration::from_secs_f64(1.0 / config.rate_hz),
            rx,
            published,
            outputs,
            state,
            shutting_down: false,
        }
    }

    fn run(mut self) {
        let mut deadline = Instant::now();
        loop {
            self.drain_commands();
            self.render();
            self.emit();
            self.publish();
            if self.shutting_down {
                break;
            }

            deadline += self.period;
            let now = Instant::now();
            if now < deadline {
                std::thread::sleep(deadline - now);
            } else {
                // En retard : on repart de maintenant plutôt que d'enchaîner des trames en rafale.
                self.state.late_frames += 1;
                deadline = now;
            }
        }
    }

    fn drain_commands(&mut self) {
        while let Ok(command) = self.rx.try_recv() {
            match command {
                Command::SetChannel { universe, channel, value } => {
                    self.set_override(universe, channel, Some(value))
                }
                Command::ReleaseChannel { universe, channel } => {
                    self.set_override(universe, channel, None)
                }
                Command::SetAttribute { key, value } => {
                    let exists = self
                        .state
                        .patch
                        .get(key.fixture)
                        .is_some_and(|f| key.parameter < f.mode().parameters.len());
                    if exists {
                        self.state.attributes.insert(key, value);
                    } else {
                        log::warn!("paramètre ignoré : {key:?}");
                    }
                }
                Command::ReleaseAttribute(key) => {
                    self.state.attributes.remove(&key);
                }
                Command::SetPatch(patch) => {
                    self.state.attributes.retain(|key, _| {
                        patch.get(key.fixture).is_some_and(|f| key.parameter < f.mode().parameters.len())
                    });
                    self.state.patch = patch;
                }
                Command::ClearProgrammer => {
                    self.state.attributes.clear();
                    self.state.channel_overrides.iter_mut().for_each(|o| o.fill(None));
                }
                Command::Blackout(on) => self.state.blackout = on,
                Command::SetOutputs(outputs) => {
                    self.outputs = valid_bindings(outputs, self.state.output.len());
                    self.state.output_names =
                        self.outputs.iter().map(|b| b.output.name().to_string()).collect();
                }
                Command::Shutdown => self.shutting_down = true,
            }
        }
    }

    fn set_override(&mut self, universe: usize, channel: u16, value: Option<u8>) {
        let slot = self
            .state
            .channel_overrides
            .get_mut(universe)
            .and_then(|o| o.get_mut((channel as usize).wrapping_sub(1)));
        match slot {
            Some(slot) => *slot = value,
            None => log::warn!("adresse ignorée : univers {universe}, canal {channel}"),
        }
    }

    fn render(&mut self) {
        let state = &mut self.state;
        state.output.iter_mut().for_each(Universe::clear);
        if state.blackout {
            return;
        }
        state.patch.render(&state.attributes, &mut state.output);
        for (out, overrides) in state.output.iter_mut().zip(&state.channel_overrides) {
            for (slot, forced) in out.as_mut_slice().iter_mut().zip(overrides.iter()) {
                if let Some(value) = forced {
                    *slot = *value;
                }
            }
        }
    }

    fn emit(&mut self) {
        for binding in &mut self.outputs {
            if let Err(err) = binding.output.send(&self.state.output[binding.universe]) {
                self.state.output_errors += 1;
                // Une erreur par seconde environ suffit dans les journaux.
                if self.state.output_errors % 44 == 1 {
                    log::error!("{} : {err}", binding.output.name());
                }
            }
        }
    }

    fn publish(&mut self) {
        self.state.frame += 1;
        self.published.lock().unwrap().clone_from(&self.state);
    }
}

/// Écarte les sorties rattachées à un univers inexistant.
fn valid_bindings(outputs: Vec<OutputBinding>, universes: usize) -> Vec<OutputBinding> {
    outputs
        .into_iter()
        .filter(|b| {
            let ok = b.universe < universes;
            if !ok {
                log::error!("sortie {} ignorée : univers {} inexistant", b.output.name(), b.universe);
            }
            ok
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use folie_dmx::OutputError;
    use std::sync::Mutex as StdMutex;

    /// Sortie de test qui garde la dernière trame reçue.
    struct Capture(Arc<StdMutex<Option<Universe>>>);

    impl DmxOutput for Capture {
        fn name(&self) -> &str {
            "capture"
        }
        fn send(&mut self, universe: &Universe) -> Result<(), OutputError> {
            *self.0.lock().unwrap() = Some(universe.clone());
            Ok(())
        }
    }

    #[test]
    fn commands_reach_output_and_blackout_wins() {
        let last = Arc::new(StdMutex::new(None));
        let engine = spawn(
            EngineConfig { universes: 1, rate_hz: 200.0 },
            vec![OutputBinding { universe: 0, output: Box::new(Capture(last.clone())) }],
        );

        engine.set_channel(0, 5, 200);
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(last.lock().unwrap().as_ref().unwrap().get(5), Ok(200));

        engine.send(Command::Blackout(true));
        engine.shutdown();
        assert_eq!(last.lock().unwrap().as_ref().unwrap().get(5), Ok(0));
    }

    #[test]
    fn fixture_defaults_values_and_overrides() {
        use folie_fixture::{FixtureId, PatchedFixture, ofl};

        let json = include_str!("../../../library/martin/rush-mh-6-wash.json");
        let mh6 = Arc::new(ofl::parse(json, "martin/rush-mh-6-wash", "Martin").unwrap());
        let mut patch = Patch::default();
        patch
            .add(PatchedFixture {
                id: FixtureId(1),
                name: "MH6 1".into(),
                fixture_type: mh6,
                mode: 0,
                universe: 0,
                address: 101,
            })
            .unwrap();

        let last = Arc::new(StdMutex::new(None));
        let engine = spawn(
            EngineConfig { universes: 1, rate_hz: 200.0 },
            vec![OutputBinding { universe: 0, output: Box::new(Capture(last.clone())) }],
        );
        engine.send(Command::SetPatch(Arc::new(patch)));
        // Dimmer (paramètre 1, canaux 102-103) à 0x8040.
        engine.send(Command::SetAttribute {
            key: ParamKey { fixture: FixtureId(1), parameter: 1 },
            value: 0x8040,
        });
        engine.set_channel(0, 101, 255); // shutter forcé à la main
        std::thread::sleep(Duration::from_millis(30));

        let out = last.lock().unwrap().clone().unwrap();
        assert_eq!(out.get(101), Ok(255)); // forcé, au lieu du défaut 12
        assert_eq!((out.get(102), out.get(103)), (Ok(0x80), Ok(0x40)));
        assert_eq!(out.get(110), Ok(128)); // pan par défaut 50 %

        engine.send(Command::ReleaseChannel { universe: 0, channel: 101 });
        engine.send(Command::ClearProgrammer);
        engine.shutdown();
        let out = last.lock().unwrap().clone().unwrap();
        assert_eq!(out.get(101), Ok(12));
        assert_eq!(out.get(102), Ok(0));
    }
}
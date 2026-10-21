//! Ce que font les executors sur le son, appliqué directement par le moteur à chaque
//! paquet d'analyse : rien n'attend l'affichage, le beat part au plus tôt.

use std::collections::HashMap;

use crossbeam_channel::Sender;
use folie_engine::Command;
use folie_show::sound::{Band, Reaction, SoundSetup};

use crate::analysis::{Hop, Levels};

/// Durée d'un flash sur le beat.
const FLASH: f64 = 0.1;
/// Écart de fader en dessous duquel on n'envoie rien.
const FADER_STEP: f32 = 0.004;

fn band(levels: &Levels, band: Band) -> f32 {
    match band {
        Band::Level => levels.level,
        Band::Bass => levels.bass,
        Band::Mid => levels.mid,
        Band::High => levels.high,
    }
}

pub struct Reactor {
    tx: Sender<Command>,
    setup: SoundSetup,
    beats: u64,
    /// Flashs à relâcher : (executor, heure).
    releases: Vec<(u32, f64)>,
    faders: HashMap<u32, f32>,
    tempo: Option<f32>,
}

impl Reactor {
    pub fn new(tx: Sender<Command>, setup: SoundSetup) -> Self {
        Self { tx, setup, beats: 0, releases: Vec::new(), faders: HashMap::new(), tempo: None }
    }

    pub fn set_setup(&mut self, setup: SoundSetup) {
        self.release_all();
        self.faders.clear();
        self.setup = setup;
    }

    fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    pub fn on_hop(&mut self, hop: &Hop, bpm: Option<f32>) {
        let now = hop.time;
        let due: Vec<u32> = self.releases.iter().filter(|(_, at)| *at <= now).map(|(x, _)| *x).collect();
        self.releases.retain(|(_, at)| *at > now);
        for executor in due {
            self.send(Command::Flash { executor, pressed: false });
        }
        if hop.beat {
            self.beats += 1;
        }
        if self.setup.tempo
            && let Some(bpm) = bpm
            && self.tempo.is_none_or(|t| (t - bpm).abs() >= 0.3)
        {
            self.tempo = Some(bpm);
            self.send(Command::SetTempo(bpm));
        }
        for r in self.setup.reactions.clone() {
            match r.reaction {
                Reaction::Step { every } if hop.beat && self.beats.is_multiple_of(u64::from(every.max(1))) => {
                    self.send(Command::Go(r.executor));
                }
                Reaction::Flash { every } if hop.beat && self.beats.is_multiple_of(u64::from(every.max(1))) => {
                    self.send(Command::Flash { executor: r.executor, pressed: true });
                    self.releases.retain(|(x, _)| *x != r.executor);
                    self.releases.push((r.executor, now + FLASH));
                }
                Reaction::Fader { band: b, min, max } => {
                    let level = (min + (max - min) * band(&hop.levels, b)).clamp(0.0, 1.0);
                    if self.faders.get(&r.executor).is_none_or(|last| (last - level).abs() >= FADER_STEP) {
                        self.faders.insert(r.executor, level);
                        self.send(Command::SetFader { executor: r.executor, level });
                    }
                }
                _ => {}
            }
        }
    }

    fn release_all(&mut self) {
        for (executor, _) in std::mem::take(&mut self.releases) {
            self.send(Command::Flash { executor, pressed: false });
        }
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        self.release_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use folie_show::sound::SoundReaction;

    fn hop(time: f64, beat: bool, bass: f32) -> Hop {
        Hop { levels: Levels { level: bass, bass, mid: 0.0, high: 0.0 }, beat, time }
    }

    #[test]
    fn beats_and_levels_become_engine_commands() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let setup = SoundSetup {
            tempo: true,
            reactions: vec![
                SoundReaction { executor: 1, reaction: Reaction::Step { every: 2 } },
                SoundReaction { executor: 2, reaction: Reaction::Flash { every: 1 } },
                SoundReaction { executor: 3, reaction: Reaction::Fader { band: Band::Bass, min: 0.2, max: 1.0 } },
            ],
        };
        let mut reactor = Reactor::new(tx, setup);
        // Les commandes du moteur ne se comparent pas : on compare leur écriture.
        let take = || rx.try_iter().map(|c| format!("{c:?}")).collect::<Vec<_>>();
        let c = |command: Command| format!("{command:?}");

        reactor.on_hop(&hop(0.0, true, 1.0), Some(124.0));
        let first = take();
        assert!(first.contains(&c(Command::SetTempo(124.0))));
        assert!(first.contains(&c(Command::Flash { executor: 2, pressed: true })));
        assert!(first.contains(&c(Command::SetFader { executor: 3, level: 1.0 })));
        assert!(!first.contains(&c(Command::Go(1))), "un pas tous les 2 beats");

        reactor.on_hop(&hop(0.05, false, 1.0), Some(124.1));
        assert!(take().is_empty(), "rien de neuf : ni fader, ni tempo à 0,1 BPM près");

        reactor.on_hop(&hop(0.12, false, 0.0), Some(124.0));
        let later = take();
        assert!(later.contains(&c(Command::Flash { executor: 2, pressed: false })), "flash relâché après 100 ms");
        assert!(later.contains(&c(Command::SetFader { executor: 3, level: 0.2 })), "graves à zéro : le minimum");

        reactor.on_hop(&hop(0.5, true, 1.0), None);
        assert!(take().contains(&c(Command::Go(1))), "deuxième beat : Go");

        // Arrêt : un flash encore enfoncé est relâché.
        drop(reactor);
        assert_eq!(take(), [c(Command::Flash { executor: 2, pressed: false })]);
    }
}

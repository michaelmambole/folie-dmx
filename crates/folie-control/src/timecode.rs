//! Cues au temps : une cue qui a un temps TC part quand le temps suivi par sa séquence
//! passe ce temps, sur l'executor de la séquence. Le temps suivi est le timecode MIDI
//! reçu (MTC), ou la position de lecture d'un son du show. Quand le temps saute
//! (démarrage, retour en arrière, déplacement), la séquence se cale sur la dernière cue
//! passée.

use folie_engine::Command as EngineCommand;
use folie_show::{CueNumber, format_timecode};

use crate::{CommandError, Console, Outcome, invalid};

/// Au-delà, l'écart entre deux temps reçus est un saut, pas le défilement.
const JUMP: f64 = 1.0;

/// D'où vient le temps suivi par une séquence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeSource {
    /// Timecode MIDI reçu.
    Mtc,
    /// Position de lecture d'un son du show.
    Sound(u32),
}

impl TimeSource {
    pub fn of(sequence: &folie_show::Sequence) -> Self {
        sequence.follows_sound.map_or(Self::Mtc, Self::Sound)
    }
}

impl Console {
    /// Temps TC d'une cue de la séquence sélectionnée (`None` : plus de départ au temps).
    pub fn set_cue_timecode(&mut self, number: CueNumber, seconds: Option<f64>) -> Result<Outcome, CommandError> {
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        cue.timecode = seconds;
        Ok(Outcome::changed(match seconds {
            Some(t) => format!("Cue {number} : départ à {}", format_timecode(t)),
            None => format!("Cue {number} : plus de départ au temps"),
        }))
    }

    /// La séquence sélectionnée suit un son (`None` : le timecode MIDI).
    pub fn set_sequence_follows(&mut self, sound: Option<u32>) -> Result<Outcome, CommandError> {
        let sequence = self.selected_sequence_mut()?;
        sequence.follows_sound = sound;
        let name = sequence.name.clone();
        Ok(Outcome::changed(match sound {
            Some(n) => format!("{name} suit la lecture du son {n}"),
            None => format!("{name} suit le timecode MIDI"),
        }))
    }

    /// Dernier temps du timecode MIDI reçu.
    pub fn received_timecode(&self) -> Option<f64> {
        self.time_of(TimeSource::Mtc)
    }

    /// Dernier temps reçu d'une source.
    pub fn time_of(&self, source: TimeSource) -> Option<f64> {
        self.times.get(&source).copied()
    }

    /// Temps suivi par la séquence sélectionnée, s'il en arrive un.
    pub fn selected_sequence_time(&self) -> Option<f64> {
        let (_, sequence) = self.selected_sequence()?;
        self.time_of(TimeSource::of(sequence))
    }

    /// Timecode MIDI reçu.
    pub fn timecode_tick(&mut self, now: f64) -> Vec<Outcome> {
        self.time_tick(TimeSource::Mtc, now)
    }

    /// Temps reçu d'une source : lance les cues des séquences qui la suivent et dont le
    /// temps vient d'être passé. À appeler à chaque nouveau temps.
    pub fn time_tick(&mut self, source: TimeSource, now: f64) -> Vec<Outcome> {
        let previous = self.times.insert(source, now);
        if previous == Some(now) {
            return Vec::new();
        }
        let jumped = previous.is_none_or(|p| now < p || now - p > JUMP);
        let snapshot = self.engine.snapshot();
        let mut outcomes = Vec::new();
        for (&executor, x) in &self.show.executors {
            let Some(sequence) = self.show.sequences.get(&x.sequence) else { continue };
            if TimeSource::of(sequence) != source {
                continue;
            }
            // Dernière cue dont le temps est passé : franchie depuis le dernier temps, ou
            // (après un saut) la position où la séquence doit être.
            let target = sequence
                .cues
                .iter()
                .enumerate()
                .filter_map(|(index, (number, cue))| Some((index, *number, cue.timecode?)))
                .filter(|(_, _, t)| *t <= now && (jumped || previous.is_some_and(|p| *t > p)))
                .max_by(|a, b| a.2.total_cmp(&b.2));
            let Some((index, number, _)) = target else { continue };
            // Déjà sur cette cue (après un saut) : rien à refaire.
            if jumped && snapshot.executors.get(&executor).and_then(|s| s.cue) == Some(index) {
                continue;
            }
            self.engine.send(EngineCommand::Goto { executor, cue: index });
            let from = match source {
                TimeSource::Mtc => "Timecode".to_string(),
                TimeSource::Sound(n) => format!("Son {n}"),
            };
            outcomes.push(Outcome::info(format!(
                "{from} {} : cue {number} (executor {})",
                format_timecode(now),
                folie_show::executor_label(executor)
            )));
        }
        outcomes
    }

    /// Un son s'est arrêté : son temps n'est plus suivi (le prochain départ recalera).
    pub fn forget_time(&mut self, source: TimeSource) {
        self.times.remove(&source);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use folie_show::parse_timecode;

    use crate::tests::{console, run};

    /// Cues 1, 2, 3 à 10 s, 20 s et 30 s : défilement, saut en avant, retour en arrière.
    #[test]
    fn cues_follow_the_timecode() {
        let mut c = console();
        for (cue, level) in [(1, 20), (2, 40), (3, 60)] {
            run(&mut c, &format!("Fixture 1 At {level}"));
            run(&mut c, &format!("Store Cue {cue} Fade 0"));
        }
        run(&mut c, "ClearAll");
        for (cue, t) in [(1, 10.0), (2, 20.0), (3, 30.0)] {
            c.set_cue_timecode(format!("{cue}").parse().unwrap(), Some(t)).unwrap();
        }
        let cue_now = |c: &crate::Console| {
            std::thread::sleep(Duration::from_millis(30));
            c.engine().snapshot().executors.get(&1).and_then(|s| s.cue)
        };
        assert!(c.timecode_tick(5.0).is_empty(), "avant la première cue");
        assert!(c.timecode_tick(5.04).is_empty());
        let mut t = 5.04;
        let mut started = Vec::new();
        while t < 10.5 {
            t += 0.04;
            started.extend(c.timecode_tick(t).into_iter().map(|o| o.message));
        }
        assert_eq!(started.len(), 1, "{started:?}");
        assert!(started[0].contains("cue 1"), "{started:?}");
        assert_eq!(cue_now(&c), Some(0));
        // On avance la bande à 25 s : calé sur la cue 2, sans rejouer la 1.
        let jumped: Vec<String> = c.timecode_tick(25.0).into_iter().map(|o| o.message).collect();
        assert_eq!(jumped.len(), 1);
        assert!(jumped[0].contains("cue 2"));
        assert_eq!(cue_now(&c), Some(1));
        // Même temps reçu deux fois : rien.
        assert!(c.timecode_tick(25.0).is_empty());
        // Retour à 12 s : la cue 1.
        assert!(c.timecode_tick(12.0)[0].message.contains("cue 1"));
        assert_eq!(cue_now(&c), Some(0));
        assert_eq!(c.received_timecode(), Some(12.0));
    }

    /// Une séquence qui suit le son 4 ne réagit qu'à lui, pas au timecode MIDI.
    #[test]
    fn a_sequence_follows_a_sound() {
        use super::TimeSource;
        let mut c = console();
        for (cue, level) in [(1, 20), (2, 40)] {
            run(&mut c, &format!("Fixture 1 At {level}"));
            run(&mut c, &format!("Store Cue {cue} Fade 0"));
        }
        run(&mut c, "ClearAll");
        c.set_cue_timecode("2".parse().unwrap(), Some(3.0)).unwrap();
        c.set_sequence_follows(Some(4)).unwrap();
        assert!(c.timecode_tick(5.0).is_empty(), "le MTC ne la concerne plus");
        assert!(c.time_tick(TimeSource::Sound(3), 5.0).is_empty(), "un autre son non plus");
        assert!(c.time_tick(TimeSource::Sound(4), 2.9).is_empty());
        let started = c.time_tick(TimeSource::Sound(4), 3.02);
        assert_eq!(started.len(), 1);
        assert!(started[0].message.starts_with("Son 4 0:00:03.02 : cue 2"), "{}", started[0].message);
        assert_eq!(c.selected_sequence_time(), Some(3.02));
        c.forget_time(TimeSource::Sound(4));
        assert_eq!(c.selected_sequence_time(), None);
    }

    #[test]
    fn timecode_text() {
        assert_eq!(parse_timecode("1:02:03.5"), Some(3723.5));
        assert_eq!(parse_timecode("2:03"), Some(123.0));
        assert_eq!(parse_timecode("83.5"), Some(83.5));
        assert_eq!(parse_timecode("1:75"), None, "75 secondes dans une minute");
        assert_eq!(parse_timecode(""), None);
        assert_eq!(folie_show::format_timecode(3723.5), "1:02:03.50");
    }
}

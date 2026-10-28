//! Cues au timecode : une cue qui a un temps TC part quand le timecode reçu (MTC) passe
//! ce temps, sur l'executor de sa séquence. Quand le timecode saute (démarrage, retour en
//! arrière, déplacement dans la bande), la séquence se cale sur la dernière cue passée.

use folie_engine::Command as EngineCommand;
use folie_show::{CueNumber, format_timecode};

use crate::{CommandError, Console, Outcome, invalid};

/// Au-delà, l'écart entre deux temps reçus est un saut, pas le défilement.
const JUMP: f64 = 1.0;

impl Console {
    /// Temps TC d'une cue de la séquence sélectionnée (`None` : plus de départ au timecode).
    pub fn set_cue_timecode(&mut self, number: CueNumber, seconds: Option<f64>) -> Result<Outcome, CommandError> {
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        cue.timecode = seconds;
        Ok(Outcome::changed(match seconds {
            Some(t) => format!("Cue {number} : départ au timecode {}", format_timecode(t)),
            None => format!("Cue {number} : plus de départ au timecode"),
        }))
    }

    /// Dernier temps de timecode reçu.
    pub fn received_timecode(&self) -> Option<f64> {
        self.timecode
    }

    /// Temps reçu : lance les cues dont le temps vient d'être passé. À appeler à chaque
    /// nouveau temps.
    pub fn timecode_tick(&mut self, now: f64) -> Vec<Outcome> {
        let previous = self.timecode.replace(now);
        if previous == Some(now) {
            return Vec::new();
        }
        let jumped = previous.is_none_or(|p| now < p || now - p > JUMP);
        let snapshot = self.engine.snapshot();
        let mut outcomes = Vec::new();
        for (&executor, x) in &self.show.executors {
            let Some(sequence) = self.show.sequences.get(&x.sequence) else { continue };
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
            outcomes.push(Outcome::info(format!(
                "Timecode {} : cue {number} (executor {})",
                format_timecode(now),
                folie_show::executor_label(executor)
            )));
        }
        outcomes
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

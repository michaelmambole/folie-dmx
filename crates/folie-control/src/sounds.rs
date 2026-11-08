//! Commandes de son : `Sound 3`, `Sound 3 Off Fade 2`, `Sound Off`. La console ne joue
//! rien elle-même : elle range les demandes, que l'interface passe à la carte son.

use std::collections::{BTreeMap, HashMap};

use crate::command::Keyword;
use crate::timecode::TimeSource;
use crate::{CommandError, Console, Outcome, invalid};

/// Ce que la carte son doit faire.
#[derive(Debug, Clone, PartialEq)]
pub enum SoundRequest {
    Play(u32),
    Stop { number: u32, fade: f32 },
    StopAll { fade: f32 },
}

impl Console {
    pub(crate) fn sound_command(&mut self, number: Option<u32>, stop: Option<f32>) -> Result<Outcome, CommandError> {
        let Some(number) = number else {
            let fade = stop.unwrap_or(0.0);
            self.sound_requests.push(SoundRequest::StopAll { fade });
            return Ok(Outcome::info("Tous les sons arrêtés"));
        };
        let Some(sound) = self.show.sounds.get(&number) else {
            return invalid(format!("le son {number} n'existe pas"));
        };
        let name = if sound.name.is_empty() { String::new() } else { format!(" « {} »", sound.name) };
        Ok(match stop {
            Some(fade) => {
                self.sound_requests.push(SoundRequest::Stop { number, fade });
                Outcome::info(format!("Son {number}{name} arrêté"))
            }
            None => {
                self.sound_requests.push(SoundRequest::Play(number));
                Outcome::info(format!("Son {number}{name} lancé"))
            }
        })
    }

    /// Demandes de son en attente, à passer à la carte son (une seule fois).
    pub fn take_sound_requests(&mut self) -> Vec<SoundRequest> {
        std::mem::take(&mut self.sound_requests)
    }

    /// Déplace le son `number` à la place `index` de la liste : les sons gardent la même
    /// série de numéros, dans le nouvel ordre. Les références suivent : `Sound 3` des
    /// cues et des macros, séquences qui suivent un son. Rend ancien -> nouveau numéro
    /// (vide si rien ne bouge).
    pub fn move_sound(&mut self, number: u32, index: usize) -> HashMap<u32, u32> {
        let numbers: Vec<u32> = self.show.sounds.keys().copied().collect();
        let Some(from) = numbers.iter().position(|n| *n == number) else { return HashMap::new() };
        let mut order = numbers.clone();
        let moved = order.remove(from);
        order.insert(index.min(order.len()), moved);
        let map: HashMap<u32, u32> = order.iter().zip(&numbers).filter(|(old, new)| old != new).map(|(o, n)| (*o, *n)).collect();
        if !map.is_empty() {
            self.renumber_sounds(&map);
        }
        map
    }

    /// Change les numéros des sons (ancien -> nouveau) et tout ce qui les désigne.
    pub fn renumber_sounds(&mut self, map: &HashMap<u32, u32>) {
        let renumber = |n: u32| map.get(&n).copied().unwrap_or(n);
        let sounds = std::mem::take(&mut self.show.sounds);
        self.show.sounds = sounds.into_iter().map(|(n, s)| (renumber(n), s)).collect::<BTreeMap<_, _>>();
        for sequence in self.show.sequences.values_mut() {
            sequence.follows_sound = sequence.follows_sound.map(renumber);
            for cue in sequence.cues.values_mut() {
                cue.command = renumber_in_line(&cue.command, map);
            }
        }
        for entry in self.show.macros.values_mut() {
            for line in &mut entry.lines {
                *line = renumber_in_line(line, map);
            }
        }
        let times = std::mem::take(&mut self.times);
        self.times = times
            .into_iter()
            .map(|(source, t)| match source {
                TimeSource::Sound(n) => (TimeSource::Sound(renumber(n)), t),
                other => (other, t),
            })
            .collect();
    }
}

/// Réécrit les numéros de son d'une ligne de commande (`Sound 3`, `so 3 Off`…) sans
/// toucher au reste : espaces, textes entre guillemets, autres commandes.
fn renumber_in_line(line: &str, map: &HashMap<u32, u32>) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.char_indices().peekable();
    let mut after_sound = false;
    while let Some((i, c)) = chars.next() {
        if c == '"' {
            out.push(c);
            for (_, c) in chars.by_ref() {
                out.push(c);
                if c == '"' {
                    break;
                }
            }
            after_sound = false;
        } else if c.is_alphabetic() {
            let mut end = i + c.len_utf8();
            while let Some(&(j, c)) = chars.peek().filter(|(_, c)| c.is_alphanumeric()) {
                end = j + c.len_utf8();
                chars.next();
            }
            let word = &line[i..end];
            out.push_str(word);
            after_sound = Keyword::parse(word) == Ok(Keyword::Sound);
        } else if c.is_ascii_digit() {
            let mut end = i + 1;
            while let Some(&(j, c)) = chars.peek().filter(|(_, c)| c.is_ascii_digit() || *c == '.' || *c == ',') {
                end = j + c.len_utf8();
                chars.next();
            }
            let number = &line[i..end];
            match number.parse::<u32>().ok().and_then(|n| map.get(&n)).filter(|_| after_sound) {
                Some(new) => out.push_str(&new.to_string()),
                None => out.push_str(number),
            }
            after_sound = false;
        } else {
            out.push(c);
            if !c.is_whitespace() {
                after_sound = false;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{console, run};

    #[test]
    fn sound_commands_are_queued() {
        let mut c = console();
        assert!(c.execute("Sound 2").unwrap_err().to_string().contains("n'existe pas"));
        c.show.sounds.insert(2, folie_show::media::Sound { name: "Pluie".into(), ..Default::default() });
        assert_eq!(run(&mut c, "Sound 2").message, "Son 2 « Pluie » lancé");
        run(&mut c, "Sound 2 Off Fade 3");
        run(&mut c, "Sound Off");
        assert_eq!(
            c.take_sound_requests(),
            [SoundRequest::Play(2), SoundRequest::Stop { number: 2, fade: 3.0 }, SoundRequest::StopAll { fade: 0.0 }]
        );
        assert!(c.take_sound_requests().is_empty(), "rendues une seule fois");
    }

    #[test]
    fn renumbering_follows_references() {
        let map = HashMap::from([(3, 1), (1, 2), (2, 3)]);
        assert_eq!(renumber_in_line("Sound 3; so 1 Off Fade 2", &map), "Sound 1; so 2 Off Fade 2");
        assert_eq!(renumber_in_line("Macro 3; Wait 1; Osc \"Sound 3\" \"/go\" 3", &map), "Macro 3; Wait 1; Osc \"Sound 3\" \"/go\" 3");
        assert_eq!(renumber_in_line("Sound Off Fade 3", &map), "Sound Off Fade 3");

        let mut c = console();
        for name in ["Pluie", "Orage", "Vent"] {
            let n = c.show.sounds.len() as u32 + 1;
            c.show.sounds.insert(if n == 3 { 5 } else { n }, folie_show::media::Sound { name: name.into(), ..Default::default() });
        }
        c.show.macros.insert(1, folie_show::Macro { name: "Orage".into(), lines: vec!["Sound 2".into(), "Sound 5 Off".into()] });
        // Le vent (5) passe en tête : les numéros 1, 2, 5 restent, dans le nouvel ordre.
        let map = c.move_sound(5, 0);
        assert_eq!(map, HashMap::from([(5, 1), (1, 2), (2, 5)]));
        let names: Vec<(u32, &str)> = c.show.sounds.iter().map(|(n, s)| (*n, s.name.as_str())).collect();
        assert_eq!(names, [(1, "Vent"), (2, "Pluie"), (5, "Orage")]);
        assert_eq!(c.show.macros[&1].lines, ["Sound 5", "Sound 1 Off"]);
        assert!(c.move_sound(1, 0).is_empty(), "déjà à sa place");
    }
}

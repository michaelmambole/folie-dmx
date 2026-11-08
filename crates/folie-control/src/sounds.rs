//! Commandes de son : `Sound 3`, `Sound 3 Off Fade 2`, `Sound Off`. La console ne joue
//! rien elle-même : elle range les demandes, que l'interface passe à la carte son.

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
}

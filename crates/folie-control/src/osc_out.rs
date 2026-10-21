//! Envoi d'OSC vers d'autres logiciels (Reaper, QLab, vidéo…), et commandes jouées au
//! départ des cues.
//!
//! Les destinations sont nommées dans le show : `Osc "Reaper" "/play"` part vers
//! l'adresse enregistrée sous « Reaper ». Changer de salle, c'est changer une adresse,
//! pas toutes les cues.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};

use folie_engine::Snapshot;
use rosc::{OscMessage, OscPacket, OscType};

use crate::command::OscArg;
use crate::{CommandError, Console, Outcome, invalid};

/// Destination toute prête : Reaper sur cet ordinateur, port d'écoute OSC par défaut.
pub const REAPER: (&str, &str) = ("Reaper", "127.0.0.1:8000");

/// « adresse:port » lisible ? Accepte aussi un nom de machine (`localhost:8000`).
pub fn resolve(address: &str) -> Result<SocketAddr, String> {
    address
        .trim()
        .to_socket_addrs()
        .map_err(|_| format!("« {address} » : écrire adresse:port, ex. 127.0.0.1:8000"))?
        .find(SocketAddr::is_ipv4)
        .ok_or_else(|| format!("« {address} » : aucune adresse IPv4"))
}

/// Coupe une commande de cue en lignes, sur les « ; » hors guillemets.
pub fn split_commands(text: &str) -> Vec<String> {
    let mut lines = vec![String::new()];
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                lines.last_mut().unwrap().push(c);
            }
            ';' if !quoted => lines.push(String::new()),
            c => lines.last_mut().unwrap().push(c),
        }
    }
    lines.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

impl Console {
    /// Adresse de la destination, sans tenir compte de la casse du nom.
    fn osc_destination(&self, name: &str) -> Option<(&String, &String)> {
        self.show.osc_destinations.iter().find(|(n, _)| n.eq_ignore_ascii_case(name.trim()))
    }

    pub fn send_osc(&mut self, destination: &str, address: &str, args: &[OscArg]) -> Result<Outcome, CommandError> {
        let Some((name, target)) = self.osc_destination(destination) else {
            return invalid(format!("destination OSC « {destination} » inconnue : à créer dans le menu Télécommande"));
        };
        let name = name.clone();
        let target = resolve(target).map_err(|e| CommandError::Invalid(format!("destination « {name} » {e}")))?;
        let message = OscMessage {
            addr: address.to_string(),
            args: args
                .iter()
                .map(|a| match a {
                    OscArg::Int(i) => OscType::Int(*i),
                    OscArg::Float(f) => OscType::Float(*f),
                    OscArg::Text(t) => OscType::String(t.clone()),
                })
                .collect(),
        };
        let bytes = rosc::encoder::encode(&OscPacket::Message(message))
            .map_err(|e| CommandError::Invalid(format!("message OSC impossible : {e}")))?;
        if self.osc_socket.is_none() {
            let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| CommandError::Invalid(format!("OSC : {e}")))?;
            self.osc_socket = Some(socket);
        }
        let socket = self.osc_socket.as_ref().expect("créée juste au-dessus");
        socket.send_to(&bytes, target).map_err(|e| CommandError::Invalid(format!("OSC vers {name} ({target}) : {e}")))?;
        let values: Vec<String> = args
            .iter()
            .map(|a| match a {
                OscArg::Int(i) => i.to_string(),
                OscArg::Float(f) => f.to_string(),
                OscArg::Text(t) => format!("\"{t}\""),
            })
            .collect();
        Ok(Outcome::info(format!("OSC vers {name} {address} {}", values.join(" ")).trim_end().to_string()))
    }

    /// Crée ou change une destination ; l'adresse est vérifiée.
    pub fn set_osc_destination(&mut self, name: &str, address: &str) -> Result<Outcome, CommandError> {
        let name = name.trim();
        if name.is_empty() {
            return invalid("nom de destination vide");
        }
        resolve(address).map_err(CommandError::Invalid)?;
        // Un même nom, quelle que soit la casse, ne fait qu'une destination.
        self.show.osc_destinations.retain(|n, _| !n.eq_ignore_ascii_case(name));
        self.show.osc_destinations.insert(name.to_string(), address.trim().to_string());
        Ok(Outcome::changed(format!("Destination OSC « {name} » : {}", address.trim())))
    }

    pub fn remove_osc_destination(&mut self, name: &str) -> Outcome {
        self.show.osc_destinations.remove(name);
        Outcome::changed(format!("Destination OSC « {name} » supprimée"))
    }

    /// Commande de la cue (séquence de l'executor sélectionné) ; chaque ligne est vérifiée.
    pub fn set_cue_command(&mut self, number: folie_show::CueNumber, command: &str) -> Result<Outcome, CommandError> {
        for line in split_commands(command) {
            match crate::command::parse(&line) {
                Ok(crate::command::Command::Wait(_)) => return invalid("Wait ne s'utilise que dans une macro : lancez une macro depuis la cue"),
                Ok(_) => {}
                Err(err) => return invalid(format!("« {line} » : {err}")),
            }
        }
        let command = command.trim().to_string();
        let cue = self.selected_sequence_mut()?.cues.get_mut(&number);
        let Some(cue) = cue else { return invalid(format!("la cue {number} n'existe pas")) };
        cue.command = command.clone();
        Ok(Outcome::changed(if command.is_empty() {
            format!("Cue {number} : plus de commande")
        } else {
            format!("Cue {number} : commande « {command} »")
        }))
    }

    /// Joue la commande des cues qui viennent de partir (Go, Goto, chenillard, Auto…).
    /// À appeler à chaque image avec l'état du moteur.
    pub fn tick_cue_commands(&mut self, snapshot: &Snapshot) -> Vec<Result<Outcome, CommandError>> {
        let mut lines = Vec::new();
        for (executor, status) in &snapshot.executors {
            let previous = self.cues_seen.insert(*executor, status.cue);
            let Some(index) = status.cue.filter(|c| previous != Some(Some(*c))) else { continue };
            let command = self
                .executor(*executor)
                .and_then(|(_, sequence)| sequence.cues.values().nth(index))
                .map(|cue| cue.command.clone())
                .unwrap_or_default();
            lines.extend(split_commands(&command));
        }
        self.cues_seen.retain(|executor, _| snapshot.executors.contains_key(executor));
        lines.iter().map(|line| self.execute(line)).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::tests::{console, run};

    #[test]
    fn commands_split_outside_quotes() {
        assert_eq!(split_commands(r#"Osc "A;B" "/x"; Go Executor 2 ;"#), [r#"Osc "A;B" "/x""#, "Go Executor 2"]);
        assert!(split_commands("  ").is_empty());
    }

    /// Un faux Reaper écoute ; la commande de la cue 2 lui parvient au Go.
    #[test]
    fn cue_command_sends_osc_on_go() {
        let reaper = UdpSocket::bind("127.0.0.1:0").unwrap();
        reaper.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut c = console();
        assert!(c.execute(r#"Osc "Reaper" "/play""#).unwrap_err().to_string().contains("inconnue"));
        assert!(c.set_osc_destination("Reaper", "pas une adresse").is_err());
        c.set_osc_destination("Reaper", &reaper.local_addr().unwrap().to_string()).unwrap();

        run(&mut c, "Fixture 1 At 50");
        run(&mut c, "Store Cue 1 Fade 0");
        run(&mut c, "At 100");
        run(&mut c, "Store Cue 2 Fade 0");
        let number = "2".parse().unwrap();
        assert!(c.set_cue_command(number, "Wait 2").is_err());
        assert!(c.set_cue_command(number, r#"Go; Osc "x""#).unwrap_err().to_string().contains("Osc"));
        c.set_cue_command(number, r#"osc "reaper" "/marker" 2; Osc "Reaper" "/play""#).unwrap();

        let wait = |c: &mut Console, cue: usize| {
            for _ in 0..100 {
                let snapshot = c.engine().snapshot();
                if snapshot.executors.get(&1).and_then(|s| s.cue) == Some(cue) {
                    return c.tick_cue_commands(&snapshot);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("cue {cue} jamais atteinte");
        };
        run(&mut c, "Go");
        assert!(wait(&mut c, 0).is_empty(), "la cue 1 n'a pas de commande");
        run(&mut c, "Go");
        let results = wait(&mut c, 1);
        let messages: Vec<String> = results.into_iter().map(|r| r.unwrap().message).collect();
        assert_eq!(messages, ["OSC vers Reaper /marker 2", "OSC vers Reaper /play"]);
        let snapshot = c.engine().snapshot();
        assert!(c.tick_cue_commands(&snapshot).is_empty(), "une seule fois par départ");

        let mut buffer = [0u8; 1024];
        let mut received = Vec::new();
        for _ in 0..2 {
            let (size, _) = reaper.recv_from(&mut buffer).unwrap();
            let (_, OscPacket::Message(m)) = rosc::decoder::decode_udp(&buffer[..size]).unwrap() else { panic!() };
            received.push((m.addr, m.args));
        }
        assert_eq!(received, [("/marker".to_string(), vec![OscType::Int(2)]), ("/play".to_string(), vec![])]);
    }
}

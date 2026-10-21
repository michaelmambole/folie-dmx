use thiserror::Error;

use crate::Universe;

#[derive(Debug, Error)]
pub enum OutputError {
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),
    #[error("erreur du port série : {0}")]
    Serial(#[from] serialport::Error),
    #[error("réponse inattendue de l'interface : {0}")]
    Protocol(String),
    #[error("interface débranchée")]
    Disconnected,
}

/// Une destination physique ou réseau pour un univers DMX.
///
/// Appelé depuis le thread du moteur à chaque trame : une implémentation
/// ne doit jamais bloquer longtemps.
pub trait DmxOutput: Send {
    fn name(&self) -> &str;
    fn send(&mut self, universe: &Universe) -> Result<(), OutputError>;

    /// `false` tant qu'une interface débranchée n'est pas revenue (elle est recherchée
    /// à chaque seconde par `send`).
    fn connected(&self) -> bool {
        true
    }
}

/// Rythme des tentatives de reconnexion : une par seconde, pas plus.
#[derive(Debug, Default)]
pub struct Retry {
    next: Option<std::time::Instant>,
}

impl Retry {
    pub const EVERY: std::time::Duration = std::time::Duration::from_secs(1);

    /// La liaison vient d'être perdue : première tentative dans une seconde.
    pub fn lost(&mut self, now: std::time::Instant) {
        self.next = Some(now + Self::EVERY);
    }

    /// Est-ce le moment d'essayer ? Si oui, la tentative suivante est prévue une seconde plus tard.
    pub fn due(&mut self, now: std::time::Instant) -> bool {
        match self.next {
            Some(at) if now < at => false,
            _ => {
                self.next = Some(now + Self::EVERY);
                true
            }
        }
    }
}

/// Sortie qui n'envoie rien : pour développer sans interface branchée.
#[derive(Debug, Default)]
pub struct NullOutput;

impl DmxOutput for NullOutput {
    fn name(&self) -> &str {
        "virtuelle"
    }

    fn send(&mut self, _universe: &Universe) -> Result<(), OutputError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn retries_once_per_second() {
        let start = Instant::now();
        let mut retry = Retry::default();
        retry.lost(start);
        assert!(!retry.due(start + Duration::from_millis(500)));
        assert!(retry.due(start + Duration::from_millis(1000)));
        assert!(!retry.due(start + Duration::from_millis(1500)), "pas deux fois dans la seconde");
        assert!(retry.due(start + Duration::from_millis(2001)));
    }
}

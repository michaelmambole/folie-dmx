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
}

/// Une destination physique ou réseau pour un univers DMX.
///
/// Appelé depuis le thread du moteur à chaque trame : une implémentation
/// ne doit jamais bloquer longtemps.
pub trait DmxOutput: Send {
    fn name(&self) -> &str;
    fn send(&mut self, universe: &Universe) -> Result<(), OutputError>;
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

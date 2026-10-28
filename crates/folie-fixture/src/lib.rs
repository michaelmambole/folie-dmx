//! Fixtures : types d'appareils, import Open Fixture Library, patch et rendu DMX.
//!
//! Les valeurs d'attributs sont toujours exprimées sur 16 bits (0 à 65535),
//! quelle que soit la résolution réelle du canal : un canal 8 bits n'utilise
//! que l'octet de poids fort. On peut ainsi passer d'un modèle 8 bits à un
//! modèle 16 bits sans toucher au show.

pub mod edit;
pub mod gdtf;
pub mod library;
pub mod model;
pub mod ofl;
pub mod patch;

pub use library::Library;
pub use model::{Attribute, Capability, CapabilityKind, FeatureGroup, FixtureType, Mode, Parameter};
pub use patch::{FixtureId, ParamKey, Patch, PatchError, PatchedFixture, Target};

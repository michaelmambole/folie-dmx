use std::collections::HashMap;
use std::sync::Arc;

use folie_dmx::{CHANNELS_PER_UNIVERSE, Universe};
use thiserror::Error;

use crate::{FixtureType, Mode};

/// Numéro de fixture choisi par l'utilisateur (« Fixture 12 »).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FixtureId(pub u32);

impl std::fmt::Display for FixtureId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Désigne un paramètre d'une fixture patchée (index dans `Mode::parameters`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamKey {
    pub fixture: FixtureId,
    pub parameter: usize,
}

#[derive(Debug, Clone)]
pub struct PatchedFixture {
    pub id: FixtureId,
    pub name: String,
    pub fixture_type: Arc<FixtureType>,
    pub mode: usize,
    /// Univers en base 0.
    pub universe: usize,
    /// Adresse de départ, 1 à 512.
    pub address: u16,
}

impl PatchedFixture {
    pub fn mode(&self) -> &Mode {
        &self.fixture_type.modes[self.mode]
    }

    pub fn last_address(&self) -> u16 {
        self.address + self.mode().footprint.saturating_sub(1)
    }

    fn overlaps(&self, other: &PatchedFixture) -> bool {
        self.universe == other.universe
            && self.address <= other.last_address()
            && other.address <= self.last_address()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatchError {
    #[error("la fixture {0} existe déjà")]
    DuplicateId(FixtureId),
    #[error("mode inexistant pour ce type de fixture")]
    InvalidMode,
    #[error("adresses {start}–{end} hors de l'univers (1–512)")]
    OutOfRange { start: u16, end: u16 },
    #[error("chevauche la fixture {0}")]
    Overlap(FixtureId),
}

/// Liste des fixtures patchées, triée par numéro.
#[derive(Debug, Clone, Default)]
pub struct Patch {
    fixtures: Vec<PatchedFixture>,
}

impl Patch {
    pub fn fixtures(&self) -> &[PatchedFixture] {
        &self.fixtures
    }

    pub fn get(&self, id: FixtureId) -> Option<&PatchedFixture> {
        self.fixtures.binary_search_by_key(&id, |f| f.id).ok().map(|i| &self.fixtures[i])
    }

    pub fn add(&mut self, fixture: PatchedFixture) -> Result<(), PatchError> {
        if fixture.mode >= fixture.fixture_type.modes.len() {
            return Err(PatchError::InvalidMode);
        }
        let end = fixture.address as usize + fixture.mode().footprint as usize - 1;
        if fixture.address == 0 || end > CHANNELS_PER_UNIVERSE {
            return Err(PatchError::OutOfRange { start: fixture.address, end: end as u16 });
        }
        let index = match self.fixtures.binary_search_by_key(&fixture.id, |f| f.id) {
            Ok(_) => return Err(PatchError::DuplicateId(fixture.id)),
            Err(index) => index,
        };
        if let Some(other) = self.fixtures.iter().find(|f| f.overlaps(&fixture)) {
            return Err(PatchError::Overlap(other.id));
        }
        self.fixtures.insert(index, fixture);
        Ok(())
    }

    pub fn remove(&mut self, id: FixtureId) -> Option<PatchedFixture> {
        let index = self.fixtures.binary_search_by_key(&id, |f| f.id).ok()?;
        Some(self.fixtures.remove(index))
    }

    pub fn next_free_id(&self) -> FixtureId {
        FixtureId(self.fixtures.last().map_or(1, |f| f.id.0 + 1))
    }

    /// Première adresse libre d'un univers pouvant accueillir `footprint` canaux.
    pub fn next_free_address(&self, universe: usize, footprint: u16) -> Option<u16> {
        let mut used: Vec<(u16, u16)> = self
            .fixtures
            .iter()
            .filter(|f| f.universe == universe)
            .map(|f| (f.address, f.last_address()))
            .collect();
        used.sort();
        let mut candidate = 1u16;
        for (start, end) in used {
            if candidate + footprint <= start {
                break;
            }
            candidate = candidate.max(end + 1);
        }
        (candidate as usize + footprint as usize - 1 <= CHANNELS_PER_UNIVERSE).then_some(candidate)
    }

    /// Écrit dans les univers la valeur de chaque paramètre patché :
    /// celle de `values` si elle existe, sinon la valeur par défaut du paramètre.
    pub fn render(&self, values: &HashMap<ParamKey, u16>, universes: &mut [Universe]) {
        for fixture in &self.fixtures {
            let Some(universe) = universes.get_mut(fixture.universe) else { continue };
            let start = fixture.address as usize - 1;
            let dmx = &mut universe.as_mut_slice()[start..];
            for (index, parameter) in fixture.mode().parameters.iter().enumerate() {
                let key = ParamKey { fixture: fixture.id, parameter: index };
                let value = values.get(&key).copied().unwrap_or(parameter.default);
                parameter.write(value, dmx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attribute, Parameter};

    fn fixture_type(footprint: u16) -> Arc<FixtureType> {
        let parameters = (0..footprint)
            .map(|i| Parameter {
                name: format!("P{i}"),
                base_name: format!("P{i}"),
                head: None,
                attribute: Attribute::Dimmer,
                offsets: vec![i],
                default: 0x0A00,
                capabilities: Vec::new(),
            })
            .collect();
        Arc::new(FixtureType {
            key: "test/t".into(),
            manufacturer: "Test".into(),
            name: "T".into(),
            short_name: "T".into(),
            categories: Vec::new(),
            modes: vec![Mode {
                name: "m".into(),
                short_name: "m".into(),
                footprint,
                heads: Vec::new(),
                parameters,
            }],
            warnings: Vec::new(),
            ofl_source: String::new(),
        })
    }

    fn patched(id: u32, address: u16, t: &Arc<FixtureType>) -> PatchedFixture {
        PatchedFixture {
            id: FixtureId(id),
            name: format!("F{id}"),
            fixture_type: t.clone(),
            mode: 0,
            universe: 0,
            address,
        }
    }

    #[test]
    fn rejects_overlaps_duplicates_and_overflow() {
        let t = fixture_type(14);
        let mut patch = Patch::default();
        patch.add(patched(1, 1, &t)).unwrap();
        assert_eq!(patch.add(patched(2, 14, &t)), Err(PatchError::Overlap(FixtureId(1))));
        assert_eq!(patch.add(patched(1, 100, &t)), Err(PatchError::DuplicateId(FixtureId(1))));
        assert!(matches!(patch.add(patched(3, 500, &t)), Err(PatchError::OutOfRange { .. })));
        patch.add(patched(2, 15, &t)).unwrap();
    }

    #[test]
    fn finds_free_addresses_in_gaps() {
        let t = fixture_type(10);
        let mut patch = Patch::default();
        patch.add(patched(1, 1, &t)).unwrap();
        patch.add(patched(2, 31, &t)).unwrap();
        assert_eq!(patch.next_free_address(0, 10), Some(11));
        assert_eq!(patch.next_free_address(0, 25), Some(41));
        assert_eq!(patch.next_free_address(0, 600), None);
        assert_eq!(patch.next_free_id(), FixtureId(3));
    }

    #[test]
    fn renders_defaults_and_values() {
        let t = fixture_type(2);
        let mut patch = Patch::default();
        patch.add(patched(7, 10, &t)).unwrap();
        let values = HashMap::from([(ParamKey { fixture: FixtureId(7), parameter: 1 }, 0xFF00)]);
        let mut universes = vec![Universe::new()];
        patch.render(&values, &mut universes);
        assert_eq!(universes[0].get(10), Ok(0x0A));
        assert_eq!(universes[0].get(11), Ok(0xFF));
    }
}

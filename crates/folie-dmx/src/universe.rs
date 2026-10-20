use thiserror::Error;

/// Nombre de canaux dans un univers DMX512.
pub const CHANNELS_PER_UNIVERSE: usize = 512;

/// Adresse hors de la plage 1..=512.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("canal DMX invalide : {0} (attendu 1..=512)")]
pub struct InvalidChannel(pub u16);

/// Un univers DMX de 512 canaux.
///
/// Les adresses publiques sont en base 1, comme sur une console (canal 1 à 512).
#[derive(Clone, PartialEq, Eq)]
pub struct Universe {
    data: [u8; CHANNELS_PER_UNIVERSE],
}

impl Universe {
    pub const fn new() -> Self {
        Self { data: [0; CHANNELS_PER_UNIVERSE] }
    }

    fn index(channel: u16) -> Result<usize, InvalidChannel> {
        match channel {
            1..=512 => Ok(channel as usize - 1),
            _ => Err(InvalidChannel(channel)),
        }
    }

    pub fn get(&self, channel: u16) -> Result<u8, InvalidChannel> {
        Ok(self.data[Self::index(channel)?])
    }

    pub fn set(&mut self, channel: u16, value: u8) -> Result<(), InvalidChannel> {
        self.data[Self::index(channel)?] = value;
        Ok(())
    }

    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    pub fn as_slice(&self) -> &[u8; CHANNELS_PER_UNIVERSE] {
        &self.data
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8; CHANNELS_PER_UNIVERSE] {
        &mut self.data
    }
}

impl Default for Universe {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Universe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // N'affiche que les canaux non nuls pour rester lisible.
        let active: Vec<_> = self
            .data
            .iter()
            .enumerate()
            .filter(|(_, v)| **v != 0)
            .map(|(i, v)| (i + 1, *v))
            .collect();
        f.debug_struct("Universe").field("active", &active).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_one_based() {
        let mut u = Universe::new();
        u.set(1, 10).unwrap();
        u.set(512, 20).unwrap();
        assert_eq!(u.as_slice()[0], 10);
        assert_eq!(u.as_slice()[511], 20);
        assert_eq!(u.set(0, 1), Err(InvalidChannel(0)));
        assert_eq!(u.set(513, 1), Err(InvalidChannel(513)));
    }
}

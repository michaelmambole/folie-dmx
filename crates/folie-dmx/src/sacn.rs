//! Sortie sACN (ANSI E1.31) : chaque trame part en paquet de données (UDP, port 5568),
//! en multicast vers le groupe de l'univers (239.255.x.y) ou vers un node précis.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

use crate::{CHANNELS_PER_UNIVERSE, DmxOutput, OutputError, Universe};

/// Port UDP du sACN.
pub const PORT: u16 = 5568;
/// Univers sACN valides.
pub const MIN_UNIVERSE: u16 = 1;
pub const MAX_UNIVERSE: u16 = 63_999;
/// Priorité par défaut ; un récepteur suit la source de plus haute priorité.
pub const DEFAULT_PRIORITY: u8 = 100;

const ACN_ID: &[u8; 12] = b"ASC-E1.17\0\0\0";
const PACKET_LEN: usize = 126 + CHANNELS_PER_UNIVERSE;
const SOURCE_NAME: &str = "Folie-DMX";

/// Groupe multicast d'un univers : 239.255.(univers / 256).(univers % 256).
pub fn multicast_address(universe: u16) -> Ipv4Addr {
    Ipv4Addr::new(239, 255, (universe >> 8) as u8, (universe & 0xFF) as u8)
}

/// Champ « drapeaux et longueur » : 0x7 dans les 4 bits hauts, puis la longueur.
fn flags_and_length(length: usize) -> [u8; 2] {
    (0x7000 | length as u16).to_be_bytes()
}

/// Paquet de données E1.31 : couche racine, couche de trame, couche DMP et 512 canaux.
pub fn data_packet(universe: &Universe, number: u16, priority: u8, sequence: u8, cid: &[u8; 16]) -> Vec<u8> {
    let mut p = Vec::with_capacity(PACKET_LEN);
    // Couche racine.
    p.extend_from_slice(&0x0010u16.to_be_bytes()); // taille du préambule
    p.extend_from_slice(&0u16.to_be_bytes()); // taille du postambule
    p.extend_from_slice(ACN_ID);
    p.extend_from_slice(&flags_and_length(PACKET_LEN - 16));
    p.extend_from_slice(&4u32.to_be_bytes()); // VECTOR_ROOT_E131_DATA
    p.extend_from_slice(cid);
    // Couche de trame.
    p.extend_from_slice(&flags_and_length(PACKET_LEN - 38));
    p.extend_from_slice(&2u32.to_be_bytes()); // VECTOR_E131_DATA_PACKET
    let mut name = [0u8; 64];
    name[..SOURCE_NAME.len()].copy_from_slice(SOURCE_NAME.as_bytes());
    p.extend_from_slice(&name);
    p.push(priority.min(200));
    p.extend_from_slice(&0u16.to_be_bytes()); // pas de synchronisation
    p.push(sequence);
    p.push(0); // options
    p.extend_from_slice(&number.to_be_bytes());
    // Couche DMP.
    p.extend_from_slice(&flags_and_length(PACKET_LEN - 115));
    p.push(2); // VECTOR_DMP_SET_PROPERTY
    p.push(0xA1); // type d'adresse et de données
    p.extend_from_slice(&0u16.to_be_bytes()); // première adresse
    p.extend_from_slice(&1u16.to_be_bytes()); // incrément
    p.extend_from_slice(&(CHANNELS_PER_UNIVERSE as u16 + 1).to_be_bytes());
    p.push(0); // code de départ DMX
    p.extend_from_slice(universe.as_slice());
    p
}

/// Identifiant de la source (CID) : tiré au hasard à l'ouverture.
fn new_cid() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let mut cid = [0u8; 16];
    for half in cid.chunks_mut(8) {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        half.copy_from_slice(&hasher.finish().to_be_bytes());
    }
    // UUID version 4.
    cid[6] = (cid[6] & 0x0F) | 0x40;
    cid[8] = (cid[8] & 0x3F) | 0x80;
    cid
}

pub struct Sacn {
    socket: UdpSocket,
    target: SocketAddr,
    universe: u16,
    priority: u8,
    sequence: u8,
    cid: [u8; 16],
    name: String,
}

impl Sacn {
    /// Sortie de l'univers `universe` (1 à 63999) ; `target` : `None` pour le multicast,
    /// sinon l'adresse d'un node.
    pub fn open(universe: u16, target: Option<IpAddr>, priority: u8) -> Result<Self, OutputError> {
        let universe = universe.clamp(MIN_UNIVERSE, MAX_UNIVERSE);
        let ip = target.unwrap_or(IpAddr::V4(multicast_address(universe)));
        Self::open_to(SocketAddr::new(ip, PORT), universe, priority)
    }

    /// Comme [`Sacn::open`], vers une adresse et un port choisis (tests).
    pub fn open_to(target: SocketAddr, universe: u16, priority: u8) -> Result<Self, OutputError> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        if target.ip().is_multicast() {
            socket.set_multicast_ttl_v4(8)?;
        }
        // Le moteur ne doit jamais attendre le réseau.
        socket.set_nonblocking(true)?;
        let how = if target.ip().is_multicast() { "multicast".to_string() } else { target.ip().to_string() };
        let name = format!("sACN univers {universe} ({how})");
        Ok(Self { socket, target, universe, priority, sequence: 0, cid: new_cid(), name })
    }
}

impl DmxOutput for Sacn {
    fn name(&self) -> &str {
        &self.name
    }

    fn send(&mut self, universe: &Universe) -> Result<(), OutputError> {
        self.sequence = self.sequence.wrapping_add(1);
        let packet = data_packet(universe, self.universe, self.priority, self.sequence, &self.cid);
        match self.socket.send_to(&packet, self.target) {
            Ok(_) => Ok(()),
            // Tampon d'envoi plein : cette trame est perdue, la suivante repartira.
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn packet_layout() {
        let mut universe = Universe::new();
        universe.set(1, 255).unwrap();
        universe.set(512, 7).unwrap();
        let cid = [9u8; 16];
        let p = data_packet(&universe, 258, 100, 42, &cid);
        assert_eq!(p.len(), 638);
        assert_eq!(&p[0..4], &[0x00, 0x10, 0x00, 0x00]);
        assert_eq!(&p[4..16], b"ASC-E1.17\0\0\0");
        assert_eq!(&p[16..18], &[0x72, 0x6E], "racine : 622 octets");
        assert_eq!(&p[18..22], &[0, 0, 0, 4]);
        assert_eq!(&p[22..38], &cid);
        assert_eq!(&p[38..40], &[0x72, 0x58], "trame : 600 octets");
        assert_eq!(&p[40..44], &[0, 0, 0, 2]);
        assert_eq!(&p[44..53], b"Folie-DMX");
        assert_eq!(p[108], 100, "priorité");
        assert_eq!(p[111], 42, "séquence");
        assert_eq!(&p[113..115], &[1, 2], "univers 258");
        assert_eq!(&p[115..117], &[0x72, 0x0B], "DMP : 523 octets");
        assert_eq!((p[117], p[118]), (2, 0xA1));
        assert_eq!(&p[123..125], &[0x02, 0x01], "513 valeurs");
        assert_eq!(p[125], 0, "code de départ");
        assert_eq!((p[126], p[637]), (255, 7));
        assert_eq!(multicast_address(258), Ipv4Addr::new(239, 255, 1, 2));
    }

    #[test]
    fn frames_reach_a_listener() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        listener.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut output = Sacn::open_to(listener.local_addr().unwrap(), 3, DEFAULT_PRIORITY).unwrap();
        assert_eq!(output.name(), "sACN univers 3 (127.0.0.1)");
        let mut universe = Universe::new();
        universe.set(5, 200).unwrap();
        output.send(&universe).unwrap();
        output.send(&universe).unwrap();
        let mut buffer = [0u8; 700];
        let (size, _) = listener.recv_from(&mut buffer).unwrap();
        assert_eq!(size, 638);
        assert_eq!((buffer[111], buffer[126 + 4]), (1, 200));
        listener.recv_from(&mut buffer).unwrap();
        assert_eq!(buffer[111], 2, "la séquence avance");
        let multicast = Sacn::open(7, None, DEFAULT_PRIORITY).unwrap();
        assert_eq!(multicast.name(), "sACN univers 7 (multicast)");
    }
}

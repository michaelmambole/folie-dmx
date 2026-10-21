//! Sortie Art-Net : chaque trame part en paquet ArtDmx (UDP, port 6454) vers un node,
//! une adresse de diffusion ou un visualiseur.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};

use crate::{CHANNELS_PER_UNIVERSE, DmxOutput, OutputError, Universe};

/// Port UDP de l'Art-Net.
pub const PORT: u16 = 6454;
/// Destination par défaut : tout le réseau local.
pub const BROADCAST: IpAddr = IpAddr::V4(Ipv4Addr::BROADCAST);
/// Plus grande adresse de port Art-Net (15 bits : net, sous-réseau, univers).
pub const MAX_UNIVERSE: u16 = 0x7FFF;

const ID: &[u8; 8] = b"Art-Net\0";
const OP_DMX: u16 = 0x5000;
const PROTOCOL_VERSION: u16 = 14;

/// Net (0 à 127), sous-réseau (0 à 15) et univers (0 à 15) d'une adresse de port.
pub fn split_universe(port_address: u16) -> (u8, u8, u8) {
    let a = port_address & MAX_UNIVERSE;
    ((a >> 8) as u8, ((a >> 4) & 0x0F) as u8, (a & 0x0F) as u8)
}

/// Paquet ArtDmx : en-tête, numéro de séquence, adresse de port, puis les 512 canaux.
pub fn dmx_packet(universe: &Universe, port_address: u16, sequence: u8) -> Vec<u8> {
    let port_address = port_address & MAX_UNIVERSE;
    let mut packet = Vec::with_capacity(18 + CHANNELS_PER_UNIVERSE);
    packet.extend_from_slice(ID);
    packet.extend_from_slice(&OP_DMX.to_le_bytes());
    packet.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    packet.push(sequence);
    packet.push(0); // port physique : informatif
    packet.push((port_address & 0xFF) as u8); // SubUni : sous-réseau et univers
    packet.push((port_address >> 8) as u8); // Net
    packet.extend_from_slice(&(CHANNELS_PER_UNIVERSE as u16).to_be_bytes());
    packet.extend_from_slice(universe.as_slice());
    packet
}

pub struct ArtNet {
    socket: UdpSocket,
    target: SocketAddr,
    port_address: u16,
    /// 1 à 255 ; 0 voudrait dire « pas de séquence ».
    sequence: u8,
    name: String,
}

impl ArtNet {
    /// Sortie vers `target` (port 6454), sur l'adresse de port Art-Net `port_address`.
    pub fn open(target: IpAddr, port_address: u16) -> Result<Self, OutputError> {
        Self::open_to(SocketAddr::new(target, PORT), port_address)
    }

    /// Comme [`ArtNet::open`], vers un port choisi (tests, visualiseurs).
    pub fn open_to(target: SocketAddr, port_address: u16) -> Result<Self, OutputError> {
        let socket = UdpSocket::bind(("0.0.0.0", 0))?;
        socket.set_broadcast(true)?;
        // Le moteur ne doit jamais attendre le réseau.
        socket.set_nonblocking(true)?;
        let (net, subnet, universe) = split_universe(port_address);
        let name = format!("Art-Net {} ({net}:{subnet}:{universe})", target.ip());
        Ok(Self { socket, target, port_address: port_address & MAX_UNIVERSE, sequence: 0, name })
    }
}

impl DmxOutput for ArtNet {
    fn name(&self) -> &str {
        &self.name
    }

    fn send(&mut self, universe: &Universe) -> Result<(), OutputError> {
        self.sequence = self.sequence.checked_add(1).unwrap_or(1);
        let packet = dmx_packet(universe, self.port_address, self.sequence);
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
        let packet = dmx_packet(&universe, 0x0123, 9);
        assert_eq!(&packet[..8], b"Art-Net\0");
        assert_eq!(&packet[8..10], &[0x00, 0x50], "OpDmx, petit-boutiste");
        assert_eq!(&packet[10..12], &[0, 14], "version 14");
        assert_eq!(packet[12], 9, "séquence");
        assert_eq!((packet[14], packet[15]), (0x23, 0x01), "sous-réseau 2, univers 3, net 1");
        assert_eq!(&packet[16..18], &[0x02, 0x00], "512 canaux");
        assert_eq!(packet.len(), 18 + 512);
        assert_eq!((packet[18], packet[18 + 511]), (255, 7));
        assert_eq!(split_universe(0x0123), (1, 2, 3));
    }

    #[test]
    fn frames_reach_a_listener() {
        let listener = UdpSocket::bind("127.0.0.1:0").unwrap();
        listener.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut output = ArtNet::open_to(listener.local_addr().unwrap(), 5).unwrap();
        assert_eq!(output.name(), "Art-Net 127.0.0.1 (0:0:5)");
        let mut universe = Universe::new();
        universe.set(3, 128).unwrap();
        output.send(&universe).unwrap();
        output.send(&universe).unwrap();
        let mut buffer = [0u8; 600];
        let (first, _) = listener.recv_from(&mut buffer).unwrap();
        assert_eq!(first, 530);
        assert_eq!((buffer[12], buffer[14], buffer[18 + 2]), (1, 5, 128));
        listener.recv_from(&mut buffer).unwrap();
        assert_eq!(buffer[12], 2, "la séquence avance à chaque trame");
    }
}

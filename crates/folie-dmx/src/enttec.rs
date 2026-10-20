//! Pilote Enttec DMX USB Pro.
//!
//! L'interface est vue comme un port série (puce FTDI). Chaque message suit
//! le format :
//!
//! ```text
//! 0x7E | label | longueur LSB | longueur MSB | données… | 0xE7
//! ```
//!
//! L'interface ré-émet en boucle la dernière trame reçue : il suffit de lui
//! envoyer une nouvelle trame quand les valeurs changent.

use std::io::{Read, Write};
use std::time::Duration;

use serialport::{SerialPort, SerialPortType};

use crate::{DmxOutput, OutputError, Universe};

const START_OF_MESSAGE: u8 = 0x7E;
const END_OF_MESSAGE: u8 = 0xE7;
const DMX_START_CODE: u8 = 0x00;
const FTDI_VENDOR_ID: u16 = 0x0403;
const MAX_PAYLOAD: usize = 600;

/// Labels de messages utilisés (voir la spécification API d'Enttec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Label {
    GetWidgetParameters = 3,
    OutputOnlySendDmx = 6,
    GetWidgetSerialNumber = 10,
}

/// Ajoute un message complet à `buf`.
pub fn encode_message(label: Label, payload: &[u8], buf: &mut Vec<u8>) {
    debug_assert!(payload.len() <= MAX_PAYLOAD);
    let len = payload.len() as u16;
    buf.push(START_OF_MESSAGE);
    buf.push(label as u8);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.extend_from_slice(payload);
    buf.push(END_OF_MESSAGE);
}

/// Encode un univers complet (start code + 512 canaux) dans `buf`, vidé au préalable.
pub fn encode_dmx(universe: &Universe, buf: &mut Vec<u8>) {
    buf.clear();
    let len = (1 + universe.as_slice().len()) as u16;
    buf.push(START_OF_MESSAGE);
    buf.push(Label::OutputOnlySendDmx as u8);
    buf.extend_from_slice(&len.to_le_bytes());
    buf.push(DMX_START_CODE);
    buf.extend_from_slice(universe.as_slice());
    buf.push(END_OF_MESSAGE);
}

/// Paramètres renvoyés par l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WidgetParameters {
    pub firmware_version: u16,
    /// Durée du break, en unités de 10,67 µs.
    pub break_time: u8,
    /// Durée du « mark after break », en unités de 10,67 µs.
    pub mab_time: u8,
    /// Trames par seconde (0 = aussi vite que possible).
    pub refresh_rate: u8,
}

impl WidgetParameters {
    fn parse(payload: &[u8]) -> Result<Self, OutputError> {
        if payload.len() < 5 {
            return Err(OutputError::Protocol(format!(
                "paramètres trop courts ({} octets)",
                payload.len()
            )));
        }
        Ok(Self {
            firmware_version: u16::from_le_bytes([payload[0], payload[1]]),
            break_time: payload[2],
            mab_time: payload[3],
            refresh_rate: payload[4],
        })
    }

    pub fn break_micros(&self) -> f64 {
        self.break_time as f64 * 10.67
    }

    pub fn mab_micros(&self) -> f64 {
        self.mab_time as f64 * 10.67
    }
}

/// Le numéro de série est codé en BCD sur 4 octets, poids faible en premier.
fn decode_serial(payload: &[u8]) -> Result<u32, OutputError> {
    if payload.len() < 4 {
        return Err(OutputError::Protocol("numéro de série trop court".into()));
    }
    Ok(payload[..4]
        .iter()
        .rev()
        .fold(0, |acc, b| acc * 100 + (b >> 4) as u32 * 10 + (b & 0x0F) as u32))
}

/// Un port série susceptible d'être une interface DMX.
#[derive(Debug, Clone)]
pub struct PortCandidate {
    pub path: String,
    /// Puce FTDI détectée : très probablement une interface Enttec.
    pub is_ftdi: bool,
    pub description: Option<String>,
    pub serial_number: Option<String>,
}

/// Liste les ports série, interfaces FTDI en premier.
pub fn find_ports() -> Result<Vec<PortCandidate>, OutputError> {
    let mut ports: Vec<_> = serialport::available_ports()?
        .into_iter()
        // Sur macOS, chaque périphérique apparaît en /dev/tty.* et /dev/cu.* ;
        // seul cu.* s'ouvre sans attendre de signal de porteuse.
        .filter(|p| !(cfg!(target_os = "macos") && p.port_name.starts_with("/dev/tty.")))
        .map(|p| match p.port_type {
            SerialPortType::UsbPort(usb) => PortCandidate {
                path: p.port_name,
                is_ftdi: usb.vid == FTDI_VENDOR_ID,
                description: usb.product,
                serial_number: usb.serial_number,
            },
            _ => PortCandidate {
                path: p.port_name,
                is_ftdi: false,
                description: None,
                serial_number: None,
            },
        })
        .collect();
    ports.sort_by_key(|p| !p.is_ftdi);
    Ok(ports)
}

/// Une interface Enttec DMX USB Pro ouverte.
pub struct UsbPro {
    port: Box<dyn SerialPort>,
    name: String,
    buf: Vec<u8>,
}

impl UsbPro {
    pub fn open(path: &str) -> Result<Self, OutputError> {
        // Le débit est ignoré par l'interface (liaison USB native), mais le pilote en exige un.
        let port = serialport::new(path, 57_600)
            .timeout(Duration::from_millis(500))
            .open()?;
        Ok(Self {
            port,
            name: format!("Enttec USB Pro ({path})"),
            buf: Vec::with_capacity(MAX_PAYLOAD + 5),
        })
    }

    pub fn widget_parameters(&mut self) -> Result<WidgetParameters, OutputError> {
        // Les deux octets demandent 0 octet de configuration utilisateur.
        let reply = self.request(Label::GetWidgetParameters, &[0, 0])?;
        WidgetParameters::parse(&reply)
    }

    pub fn serial_number(&mut self) -> Result<u32, OutputError> {
        let reply = self.request(Label::GetWidgetSerialNumber, &[])?;
        decode_serial(&reply)
    }

    fn request(&mut self, label: Label, payload: &[u8]) -> Result<Vec<u8>, OutputError> {
        self.port.clear(serialport::ClearBuffer::Input)?;
        self.buf.clear();
        encode_message(label, payload, &mut self.buf);
        self.port.write_all(&self.buf)?;
        // L'interface peut envoyer d'autres messages entre-temps (entrée DMX…) : on les ignore.
        for _ in 0..8 {
            let (reply_label, reply) = self.read_message()?;
            if reply_label == label as u8 {
                return Ok(reply);
            }
            log::debug!("message ignoré, label {reply_label}");
        }
        Err(OutputError::Protocol(format!("pas de réponse au label {}", label as u8)))
    }

    fn read_message(&mut self) -> Result<(u8, Vec<u8>), OutputError> {
        let mut byte = [0u8; 1];
        loop {
            self.port.read_exact(&mut byte)?;
            if byte[0] == START_OF_MESSAGE {
                break;
            }
        }
        let mut header = [0u8; 3];
        self.port.read_exact(&mut header)?;
        let len = u16::from_le_bytes([header[1], header[2]]) as usize;
        if len > MAX_PAYLOAD {
            return Err(OutputError::Protocol(format!("longueur invalide : {len}")));
        }
        let mut payload = vec![0u8; len];
        self.port.read_exact(&mut payload)?;
        self.port.read_exact(&mut byte)?;
        if byte[0] != END_OF_MESSAGE {
            return Err(OutputError::Protocol("fin de message absente".into()));
        }
        Ok((header[0], payload))
    }
}

impl DmxOutput for UsbPro {
    fn name(&self) -> &str {
        &self.name
    }

    fn send(&mut self, universe: &Universe) -> Result<(), OutputError> {
        encode_dmx(universe, &mut self.buf);
        self.port.write_all(&self.buf)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmx_frame_layout() {
        let mut u = Universe::new();
        u.set(1, 0xFF).unwrap();
        u.set(512, 0x42).unwrap();
        let mut buf = Vec::new();
        encode_dmx(&u, &mut buf);

        assert_eq!(buf.len(), 4 + 513 + 1);
        assert_eq!(&buf[..5], &[0x7E, 6, 0x01, 0x02, 0x00]); // 513 = 0x0201
        assert_eq!(buf[5], 0xFF);
        assert_eq!(buf[516], 0x42);
        assert_eq!(buf[517], 0xE7);
    }

    #[test]
    fn serial_number_is_bcd() {
        // 12 34 56 78 -> 0x78 0x56 0x34 0x12 sur le fil
        assert_eq!(decode_serial(&[0x78, 0x56, 0x34, 0x12]).unwrap(), 12_345_678);
    }

    #[test]
    fn widget_parameters_parse() {
        let p = WidgetParameters::parse(&[0x44, 0x01, 9, 1, 40]).unwrap();
        assert_eq!(p.firmware_version, 0x0144);
        assert_eq!(p.refresh_rate, 40);
    }
}

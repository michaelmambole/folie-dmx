//! Adresses de cet ordinateur sur le réseau local, avec le nom de leur connexion
//! (« Wi-Fi », « Ethernet »…), pour dire au téléphone où trouver la console.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::time::{Duration, Instant};

/// Liste relue au plus toutes les 5 secondes (le Wi-Fi peut changer en cours de route).
const REFRESH: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalAddress {
    pub ip: Ipv4Addr,
    /// Interface système : « en0 ».
    pub interface: String,
    /// Nom lisible : « Wi-Fi », ou l'interface si on ne le connaît pas.
    pub label: String,
}

/// Noms des connexions d'après `networksetup -listallhardwareports` (macOS) : interface -> nom.
fn parse_hardware_ports(text: &str) -> HashMap<String, String> {
    let mut names = HashMap::new();
    let mut port = None;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("Hardware Port: ") {
            port = Some(name.trim().to_string());
        } else if let (Some(device), Some(name)) = (line.strip_prefix("Device: "), port.take()) {
            names.insert(device.trim().to_string(), name);
        }
    }
    names
}

fn hardware_ports() -> HashMap<String, String> {
    if !cfg!(target_os = "macos") {
        return HashMap::new();
    }
    std::process::Command::new("networksetup")
        .arg("-listallhardwareports")
        .output()
        .map(|out| parse_hardware_ports(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

/// Adresse que le système choisirait pour sortir (une socket UDP « connectée » n'envoie rien).
fn preferred() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect((Ipv4Addr::new(192, 168, 0, 1), 9)).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// Adresses IPv4 du réseau local, celle par défaut en premier.
pub fn local_addresses() -> Vec<LocalAddress> {
    let names = hardware_ports();
    let mut addresses: Vec<LocalAddress> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() && (ip.is_private() || ip.is_link_local()) => {
                let label = names.get(&i.name).cloned().unwrap_or_else(|| i.name.clone());
                Some(LocalAddress { ip, interface: i.name, label })
            }
            _ => None,
        })
        .collect();
    let first = preferred();
    addresses.sort_by_key(|a| (Some(IpAddr::V4(a.ip)) != first, a.interface.clone()));
    addresses.dedup();
    addresses
}

/// Liste gardée en mémoire, relue de temps en temps.
#[derive(Default)]
pub struct AddressCache {
    read: Option<(Instant, Vec<LocalAddress>)>,
}

impl AddressCache {
    pub fn get(&mut self) -> &[LocalAddress] {
        if self.read.as_ref().is_none_or(|(at, _)| at.elapsed() > REFRESH) {
            self.read = Some((Instant::now(), local_addresses()));
        }
        self.read.as_ref().map_or(&[], |(_, list)| list)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_port_names() {
        let text = "\nHardware Port: Wi-Fi\nDevice: en0\nEthernet Address: aa:bb\n\nHardware Port: Thunderbolt Bridge\nDevice: bridge0\n\nVLAN Configurations\n";
        let names = parse_hardware_ports(text);
        assert_eq!(names["en0"], "Wi-Fi");
        assert_eq!(names["bridge0"], "Thunderbolt Bridge");
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn this_computer_has_local_addresses_or_none() {
        for a in local_addresses() {
            assert!(a.ip.is_private() || a.ip.is_link_local(), "{a:?}");
        }
    }
}

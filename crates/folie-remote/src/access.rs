//! Qui a le droit de piloter : réseau local, code PIN, mise à l'écart après trop d'erreurs.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Erreurs de code tolérées avant la mise à l'écart.
const MAX_FAILURES: u32 = 5;
const LOCKOUT: Duration = Duration::from_secs(60);

/// Adresses du réseau local : la machine elle-même, les plages privées, les liens locaux.
pub fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            // fc00::/7 (adresses uniques locales) et fe80::/10 (lien local).
            v6.is_loopback() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

/// Accès refusé : `reason` vaut « pin » (code faux) ou « locked » (trop d'erreurs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub reason: &'static str,
    pub message: &'static str,
}

/// Ce que permet le code donné.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Code « régie » : tout.
    Full,
    /// Code « lecture » : jouer les executors (GO, Back, Flash), rien d'autre.
    Playback,
}

pub struct Access {
    full: String,
    playback: String,
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl Access {
    /// Un code vide n'ouvre rien.
    pub fn new(full: &str, playback: &str) -> Self {
        Self { full: full.trim().to_string(), playback: playback.trim().to_string(), failures: Mutex::new(HashMap::new()) }
    }

    pub fn check(&self, ip: IpAddr, pin: &str) -> Result<Role, Refusal> {
        let mut failures = self.failures.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        failures.retain(|_, (_, last)| now.duration_since(*last) < LOCKOUT);
        if failures.get(&ip).is_some_and(|(count, _)| *count >= MAX_FAILURES) {
            return Err(Refusal { reason: "locked", message: "Trop d'erreurs : réessayez dans une minute" });
        }
        let pin = pin.trim();
        let role = if !self.full.is_empty() && pin == self.full {
            Some(Role::Full)
        } else if !self.playback.is_empty() && pin == self.playback {
            Some(Role::Playback)
        } else {
            None
        };
        if let Some(role) = role {
            failures.remove(&ip);
            return Ok(role);
        }
        let entry = failures.entry(ip).or_insert((0, now));
        *entry = (entry.0 + 1, now);
        log::warn!("télécommande : code faux depuis {ip}");
        Err(Refusal { reason: "pin", message: "Code PIN incorrect" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses_only() {
        for ip in ["127.0.0.1", "192.168.1.20", "10.0.0.5", "172.16.3.4", "169.254.1.1", "::1", "fe80::1", "fd12::3"] {
            assert!(is_local(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "2a01:cb00::1"] {
            assert!(!is_local(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn wrong_codes_lock_the_address_out() {
        let access = Access::new("4821", "1357");
        let phone: IpAddr = "192.168.1.20".parse().unwrap();
        let other: IpAddr = "192.168.1.21".parse().unwrap();
        assert_eq!(access.check(phone, "4821"), Ok(Role::Full));
        assert_eq!(access.check(phone, " 1357 "), Ok(Role::Playback));
        for _ in 0..MAX_FAILURES {
            assert_eq!(access.check(phone, "0000").unwrap_err().reason, "pin");
        }
        assert_eq!(access.check(phone, "4821").unwrap_err().reason, "locked", "même le bon code");
        assert!(access.check(other, "4821").is_ok(), "les autres ne sont pas touchés");
        assert!(Access::new("", "").check(phone, "").is_err(), "sans code, personne n'entre");
    }
}

//! Télécommande web et OSC : bouton de la barre du haut, réglages, et passage des
//! demandes des téléphones et des clients OSC à la console à chaque image.

use eframe::egui::{self, Color32, RichText};
use folie_remote::RemoteServer;
use folie_remote::osc::OscServer;

use super::FolieApp;
use crate::network::LocalAddress;
use crate::theme;

/// Code à 4 chiffres, tiré de l'horloge et de l'aléa de la table de hachage.
fn new_pin() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    format!("{:04}", hasher.finish() % 10_000)
}

/// Nom lisible d'une adresse : « Wi-Fi 192.168.1.20 ».
fn address_label(a: &LocalAddress) -> String {
    format!("{}  {}", a.label, a.ip)
}

/// Adresse proposée au téléphone. Avec plusieurs connexions (Wi-Fi pour les téléphones,
/// Ethernet vers un node Art-Net…), on choisit laquelle ; le choix est retenu.
fn choose_address(ui: &mut egui::Ui, addresses: &[LocalAddress]) -> String {
    let Some(first) = addresses.first() else {
        ui.label(RichText::new("Aucun réseau local : branchez le Wi-Fi ou un câble").size(12.0).color(theme::DANGER));
        return "localhost".into();
    };
    let id = egui::Id::unique("remote-interface");
    let chosen = ui.data_mut(|d| d.get_persisted::<String>(id));
    let current = addresses.iter().find(|a| Some(&a.interface) == chosen.as_ref()).unwrap_or(first);
    if addresses.len() > 1 {
        for a in addresses {
            let text = RichText::new(address_label(a)).size(12.5);
            if ui.radio(a == current, text).clicked() {
                ui.data_mut(|d| d.insert_persisted(id, a.interface.clone()));
            }
        }
    } else {
        ui.label(RichText::new(&current.label).size(12.5));
    }
    current.ip.to_string()
}

/// Toutes les adresses, à recopier dans le client.
fn address_list(ui: &mut egui::Ui, addresses: &[LocalAddress]) {
    if addresses.is_empty() {
        ui.label(RichText::new("Aucun réseau local : branchez le Wi-Fi ou un câble").size(12.0).color(theme::DANGER));
    }
    for a in addresses {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&a.label).size(12.5).color(theme::MUTED));
            ui.add(egui::Label::new(RichText::new(a.ip.to_string()).monospace().size(13.0)).selectable(true));
        });
    }
}

/// QR code de l'adresse, à scanner avec l'appareil photo du téléphone.
fn qr_code(ui: &mut egui::Ui, text: &str, size: f32) {
    let Ok(code) = qrcodegen::QrCode::encode_text(text, qrcodegen::QrCodeEcc::Medium) else { return };
    // Marge blanche de 4 modules autour, comme le veut la norme.
    let modules = code.size() + 8;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    response.on_hover_text(text);
    let module = size / modules as f32;
    let painter = ui.painter();
    painter.rect_filled(rect, 4.0, Color32::WHITE);
    for y in 0..code.size() {
        for x in 0..code.size() {
            if code.get_module(x, y) {
                let min = rect.min + egui::vec2((x + 4) as f32 * module, (y + 4) as f32 * module);
                painter.rect_filled(egui::Rect::from_min_size(min, egui::vec2(module, module)), 0.0, Color32::BLACK);
            }
        }
    }
}

impl FolieApp {
    /// Demandes des téléphones et des clients OSC appliquées, puis état publié pour eux.
    pub(super) fn remote_tick(&mut self) {
        let mut results = Vec::new();
        if let Some(server) = &self.remote {
            results.extend(server.handle(&mut self.console));
            server.publish(&self.console, &self.snapshot, &self.file.show_name());
        }
        if let Some(osc) = &self.osc {
            results.extend(osc.handle(&mut self.console));
            osc.publish(&self.console, &self.snapshot);
        }
        for result in results {
            if self.command_line.report(result) {
                self.mark_dirty();
            }
        }
    }

    /// Démarre, arrête ou relance le serveur selon les réglages, et les enregistre.
    pub(crate) fn remote_changed(&mut self, ctx: &egui::Context) {
        // Le serveur précédent est arrêté d'abord : il libère son port.
        self.remote = None;
        if self.remote_settings.enabled {
            let s = &mut self.remote_settings;
            if s.pin.is_empty() {
                s.pin = new_pin();
            }
            // Deux codes distincts, sinon le code lecture donnerait tout.
            while s.playback_pin.is_empty() || s.playback_pin == s.pin {
                s.playback_pin = new_pin();
            }
            let ctx = ctx.clone();
            let started = RemoteServer::start(s.port, &s.pin, &s.playback_pin, move || ctx.request_repaint());
            match started {
                Ok(server) => self.remote = Some(server),
                Err(err) => {
                    self.remote_settings.enabled = false;
                    self.error = Some(format!("Télécommande : port {} indisponible ({err})", self.remote_settings.port));
                }
            }
        }
        self.file.storage.set_remote(&self.remote_settings);
    }

    /// Démarre, arrête ou relance l'OSC selon les réglages, et les enregistre.
    pub(crate) fn osc_changed(&mut self, ctx: &egui::Context) {
        self.osc = None;
        let s = &self.osc_settings;
        if s.enabled {
            let started = s
                .allowed_ips()
                .map_err(|bad| format!("OSC : adresse autorisée « {bad} » illisible"))
                .and_then(|allowed| {
                    let ctx = ctx.clone();
                    OscServer::start(s.port, s.reply_port, allowed, s.reaper_markers, move || ctx.request_repaint())
                        .map_err(|err| format!("OSC : port {} indisponible ({err}) ; Reaper ou TouchOSC l'utilise peut-être, essayez {}", s.port, s.port + 1))
                });
            match started {
                Ok(server) => self.osc = Some(server),
                Err(err) => {
                    self.osc_settings.enabled = false;
                    self.error = Some(err);
                }
            }
        }
        self.file.storage.set_osc(&self.osc_settings);
    }

    /// Bouton « Télécommande » de la barre du haut : vert quand le web ou l'OSC est actif,
    /// avec le nombre d'appareils connectés ; un clic ouvre les réglages.
    pub(super) fn remote_button(&mut self, ui: &mut egui::Ui) {
        let active = self.remote.is_some() || self.osc.is_some();
        let clients = self.remote.as_ref().map_or(0, RemoteServer::clients) + self.osc.as_ref().map_or(0, OscServer::clients);
        let label = if clients > 0 { format!("Télécommande · {clients}") } else { "Télécommande".to_string() };
        let mut button = egui::Button::new(RichText::new(label).size(12.0).color(if active { theme::TEXT } else { theme::MUTED }));
        if active {
            button = button.stroke(egui::Stroke::new(1.0, theme::GO)).fill(theme::GO.gamma_multiply(0.12));
        }
        let help = match (&self.remote, &self.osc) {
            (None, None) => "Télécommande web et OSC coupées : piloter depuis un téléphone, une tablette ou TouchOSC".to_string(),
            (web, osc) => {
                let mut lines = vec![format!("{clients} appareil(s) connecté(s)")];
                let addresses = self.addresses.get();
                if let Some(web) = web {
                    lines.extend(addresses.iter().map(|a| format!("Web : http://{}:{}  ({})", a.ip, web.port(), a.label)));
                }
                if let Some(osc) = osc {
                    lines.extend(addresses.iter().map(|a| format!("OSC : {} port {}  ({})", a.ip, osc.port(), a.label)));
                }
                lines.join("\n")
            }
        };
        let response = ui.add(button).on_hover_text(help);
        egui::Popup::menu(&response)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                self.remote_menu(ui);
                ui.add_space(8.0);
                ui.separator();
                self.osc_menu(ui);
            });
    }

    fn remote_menu(&mut self, ui: &mut egui::Ui) {
        theme::menu_fields(ui);
        ui.set_width(260.0);
        let mut settings = self.remote_settings.clone();
        let addresses = self.addresses.get().to_vec();
        theme::section(ui, "Télécommande web");
        ui.add_space(4.0);
        if let Some(server) = &self.remote {
            ui.label(RichText::new("Sur le téléphone, même réseau que :").size(12.0).color(theme::MUTED));
            let host = choose_address(ui, &addresses);
            let url = format!("http://{host}:{}", server.port());
            ui.add(egui::Label::new(RichText::new(&url).monospace().size(13.0)).selectable(true));
            ui.add_space(6.0);
            ui.vertical_centered(|ui| qr_code(ui, &url, 150.0));
            ui.add_space(6.0);
            egui::Grid::new("remote-codes").num_columns(3).spacing([10.0, 2.0]).show(ui, |ui| {
                let code = |ui: &mut egui::Ui, title: &str, pin: &str, detail: &str| {
                    ui.label(RichText::new(title).color(theme::MUTED));
                    ui.label(RichText::new(pin).monospace().size(20.0).strong().color(theme::SELECTION));
                    ui.label(RichText::new(detail).size(11.5).color(theme::MUTED));
                    ui.end_row();
                };
                code(ui, "Code régie", &settings.pin, "tout");
                code(ui, "Code lecture", &settings.playback_pin, "GO, Back, Flash");
            });
            let help = "Tirer deux autres codes : les appareils connectés sont déconnectés";
            if ui.button("Nouveaux codes").on_hover_text(help).clicked() {
                settings.pin = new_pin();
                settings.playback_pin.clear(); // tiré à nouveau au démarrage, différent du code régie
            }
            let clients = server.clients();
            let connected = match clients {
                0 => "Aucun appareil connecté".to_string(),
                1 => "1 appareil connecté".to_string(),
                n => format!("{n} appareils connectés"),
            };
            ui.label(RichText::new(connected).size(12.0).color(if clients > 0 { theme::GO } else { theme::MUTED }));
            ui.add_space(6.0);
            let off = egui::Button::new(RichText::new("Couper la télécommande").family(theme::medium()).color(Color32::WHITE)).fill(theme::DANGER);
            if ui.add_sized([ui.available_width(), 28.0], off).on_hover_text("Déconnecte les appareils et ferme l'accès").clicked() {
                settings.enabled = false;
            }
        } else {
            let text = "Piloter les executors, Go/Back et la ligne de commande depuis un téléphone ou une tablette du réseau local. Deux codes : régie (tout) et lecture (GO, Back, Flash).";
            ui.label(RichText::new(text).size(12.0).color(theme::MUTED));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = ui.label(RichText::new("Port").color(theme::MUTED));
                ui.add(egui::DragValue::new(&mut settings.port).range(1024..=65535).clamp_existing_to_range(false)).labelled_by(label.id);
            });
            ui.add_space(6.0);
            if ui.add_sized([ui.available_width(), 28.0], theme::primary_button("Activer la télécommande")).clicked() {
                settings.enabled = true;
            }
        }
        if settings != self.remote_settings {
            self.remote_settings = settings;
            self.remote_changed(&ui.ctx().clone());
        }
    }

    fn osc_menu(&mut self, ui: &mut egui::Ui) {
        let mut settings = self.osc_settings.clone();
        let addresses = self.addresses.get().to_vec();
        theme::section(ui, "OSC");
        ui.add_space(4.0);
        if let Some(osc) = &self.osc {
            ui.label(RichText::new(format!("Dans TouchOSC, QLab… envoyer vers le port {} de :", osc.port())).size(12.0).color(theme::MUTED));
            address_list(ui, &addresses);
            let reply = match osc.reply_port() {
                0 => "Retour d'état vers le port d'où le client écrit".to_string(),
                port => format!("Retour d'état vers le port {port} du client"),
            };
            ui.label(RichText::new(reply).size(12.0).color(theme::MUTED));
            let allowed = if settings.allowed.trim().is_empty() { "tout le réseau local".into() } else { settings.allowed.clone() };
            ui.label(RichText::new(format!("Autorisés : {allowed}")).size(12.0).color(theme::MUTED));
            let clients = osc.clients();
            let heard = match clients {
                0 => "Aucun client entendu".to_string(),
                1 => "1 client entendu".to_string(),
                n => format!("{n} clients entendus"),
            };
            ui.label(RichText::new(heard).size(12.0).color(if clients > 0 { theme::GO } else { theme::MUTED }));
            ui.add_space(6.0);
            let off = egui::Button::new(RichText::new("Couper l'OSC").family(theme::medium()).color(Color32::WHITE)).fill(theme::DANGER);
            if ui.add_sized([ui.available_width(), 28.0], off).clicked() {
                settings.enabled = false;
            }
        } else {
            let text = "Piloter depuis TouchOSC, QLab ou une surface de contrôle. Pas de code : limitez l'accès aux adresses connues.";
            let addresses = "/folie/exec/3/fader 0.75\n/folie/exec/3/go, back, off, flash\n/folie/go, /folie/back\n/folie/page 2, /folie/blackout 1\n/folie/cmd \"Fixture 1 At 50\"";
            ui.label(RichText::new(text).size(12.0).color(theme::MUTED)).on_hover_text(addresses);
            ui.add_space(6.0);
            egui::Grid::new("osc-settings").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
                let label = ui.label(RichText::new("Port").color(theme::MUTED));
                ui.add(egui::DragValue::new(&mut settings.port).range(1024..=65535).clamp_existing_to_range(false)).labelled_by(label.id);
                ui.end_row();
                let label = ui.label(RichText::new("Port de retour").color(theme::MUTED));
                ui.add(egui::DragValue::new(&mut settings.reply_port).range(0..=65535))
                    .labelled_by(label.id)
                    .on_hover_text("Port où le client attend l'état (9000 pour TouchOSC) ; 0 : celui d'où il écrit");
                ui.end_row();
                let label = ui.label(RichText::new("Autorisés").color(theme::MUTED));
                // Texte libre : retenu quand on quitte le champ.
                let id = egui::Id::unique("osc-allowed");
                let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| settings.allowed.clone());
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut text).hint_text("tout le réseau local").desired_width(150.0))
                    .labelled_by(label.id)
                    .on_hover_text("Adresses IP séparées par des virgules ; vide : tout le réseau local");
                if edit.lost_focus() {
                    settings.allowed = text.trim().to_string();
                    ui.data_mut(|d| d.remove::<String>(id));
                } else if edit.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, text));
                }
                ui.end_row();
            });
            ui.add_space(6.0);
            if ui.add_sized([ui.available_width(), 28.0], theme::primary_button("Activer l'OSC")).clicked() {
                settings.enabled = true;
            }
        }
        ui.add_space(4.0);
        let help = "Reaper en surface de contrôle OSC (fichier Default), envoyant vers ce port : chaque marqueur passé fait Goto Cue du même numéro, sur l'executor sélectionné";
        ui.checkbox(&mut settings.reaper_markers, "Suivre les marqueurs de Reaper").on_hover_text(help);
        if settings != self.osc_settings {
            self.osc_settings = settings;
            self.osc_changed(&ui.ctx().clone());
        }
        ui.add_space(8.0);
        self.osc_destinations_menu(ui);
    }

    /// Destinations de la commande `Osc` (Reaper, QLab, vidéo…), enregistrées dans le show.
    fn osc_destinations_menu(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Envoi OSC");
        let help = "Dans la commande d'une cue (clic droit sur la cue) : Osc \"Reaper\" \"/play\"";
        ui.label(RichText::new("Destinations du show, pour la commande Osc").size(12.0).color(theme::MUTED)).on_hover_text(help);
        let mut result = None;
        let listening = self.osc.as_ref().map(OscServer::port);
        let destinations: Vec<(String, String)> = self.console.show.osc_destinations.iter().map(|(n, a)| (n.clone(), a.clone())).collect();
        egui::Grid::new("osc-destinations").num_columns(3).spacing([8.0, 2.0]).show(ui, |ui| {
            for (name, address) in &destinations {
                ui.label(RichText::new(name).color(theme::TEXT));
                ui.label(RichText::new(address).monospace().size(12.5));
                let delete = egui::Button::new(RichText::new("×").size(14.0).color(theme::MUTED)).frame(false);
                if ui.add(delete).on_hover_text(format!("Supprimer la destination « {name} »")).clicked() {
                    result = Some(Ok(self.console.remove_osc_destination(name)));
                }
                ui.end_row();
                // Même machine et même port que l'écoute de Folie : le message reviendrait à Folie.
                let loops = folie_control::osc_out::resolve(address)
                    .is_ok_and(|a| a.ip().is_loopback() && Some(a.port()) == listening);
                if loops {
                    let text = format!("Folie écoute déjà le port {} : changez le port de l'un des deux", a_port(address));
                    ui.label(RichText::new(text).size(11.5).color(theme::DANGER));
                    ui.end_row();
                }
            }
        });
        // Ajout : nom et adresse, gardés pendant la saisie.
        let id = egui::Id::unique("osc-new-destination");
        let (mut name, mut address) = ui.data_mut(|d| d.get_temp::<(String, String)>(id)).unwrap_or_default();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add(egui::TextEdit::singleline(&mut name).hint_text("Nom").desired_width(70.0)).on_hover_text("Nom de la destination");
            ui.add(egui::TextEdit::singleline(&mut address).hint_text("127.0.0.1:8000").desired_width(110.0).font(egui::TextStyle::Monospace))
                .on_hover_text("Adresse:port de la destination");
            if ui.button("Ajouter").clicked() {
                let added = self.console.set_osc_destination(&name, &address);
                if added.is_ok() {
                    (name, address) = Default::default();
                }
                result = Some(added);
            }
        });
        ui.data_mut(|d| d.insert_temp(id, (name, address)));
        if !self.console.show.osc_destinations.keys().any(|n| n.eq_ignore_ascii_case("reaper")) {
            let (name, address) = folie_control::osc_out::REAPER;
            if ui.button(format!("Ajouter Reaper ({address})")).on_hover_text("Reaper sur cet ordinateur, port d'écoute OSC par défaut").clicked() {
                result = Some(self.console.set_osc_destination(name, address));
            }
        }
        if let Some(result) = result
            && self.command_line.report(result)
        {
            self.mark_dirty();
        }
    }
}

/// Port d'une adresse « ip:port », pour les messages.
fn a_port(address: &str) -> &str {
    address.rsplit(':').next().unwrap_or(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    /// Wi-Fi pour les téléphones, Ethernet vers un node Art-Net : on choisit, le choix reste.
    #[test]
    fn several_networks_to_choose_from() {
        let address = |ip: [u8; 4], interface: &str, label: &str| LocalAddress {
            ip: ip.into(),
            interface: interface.into(),
            label: label.into(),
        };
        let addresses = vec![address([10, 0, 0, 5], "en5", "Ethernet"), address([192, 168, 1, 20], "en0", "Wi-Fi")];
        let mut harness = Harness::new_ui_state(
            |ui, host: &mut String| {
                *host = choose_address(ui, &addresses);
            },
            String::new(),
        );
        harness.run();
        assert_eq!(harness.state(), "10.0.0.5", "celle du système d'abord");
        harness.get_by_label("Wi-Fi  192.168.1.20").click();
        harness.run();
        assert_eq!(harness.state(), "192.168.1.20");
        harness.run();
        assert_eq!(harness.state(), "192.168.1.20", "choix retenu");
    }
}

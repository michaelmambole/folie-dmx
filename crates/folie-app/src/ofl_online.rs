//! Recherche dans Open Fixture Library en ligne : on cherche un appareil par son nom,
//! la fiche choisie est téléchargée puis passe par la boîte d'import habituelle.
//! Les requêtes partent d'un fil à part : l'interface ne gèle pas pendant l'attente.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use eframe::egui::{self, Key, RichText};

use crate::theme;

const SITE: &str = "https://open-fixture-library.org";

/// Un appareil trouvé : clé OFL ("robe/robin-600e-spot") et nom complet.
#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub key: String,
    pub name: String,
}

/// D'où viennent les fiches : le site OFL, ou un faux site dans les tests.
pub trait Source: Send + Sync {
    fn search(&self, query: &str) -> Result<Vec<Hit>, String>;
    /// Texte JSON de la fiche.
    fn fixture(&self, key: &str) -> Result<String, String>;
    /// Constructeurs : clé -> nom affiché.
    fn manufacturers(&self) -> Result<BTreeMap<String, String>, String>;
}

/// Le site open-fixture-library.org (son API publique de recherche).
pub struct OflSite {
    agent: ureq::Agent,
}

impl OflSite {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build();
        Self { agent: config.into() }
    }

    fn get(&self, url: &str) -> Result<String, String> {
        self.agent
            .get(url)
            .call()
            .map_err(describe)?
            .body_mut()
            .read_to_string()
            .map_err(describe)
    }
}

fn describe(err: ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(404) => "introuvable sur le site".into(),
        ureq::Error::StatusCode(code) => format!("le site a répondu {code}"),
        other => format!("pas de réponse du site ({other})"),
    }
}

impl Source for OflSite {
    fn search(&self, query: &str) -> Result<Vec<Hit>, String> {
        let body = serde_json::json!({ "searchQuery": query, "manufacturersQuery": [], "categoriesQuery": [] });
        let text = self
            .agent
            .post(format!("{SITE}/api/v1/get-search-results"))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(describe)?
            .body_mut()
            .read_to_string()
            .map_err(describe)?;
        let hits: Vec<serde_json::Value> = serde_json::from_str(&text).map_err(|e| format!("réponse illisible : {e}"))?;
        Ok(hits
            .iter()
            .filter_map(|h| Some(Hit { key: h.get("key")?.as_str()?.into(), name: h.get("name")?.as_str()?.into() }))
            .collect())
    }

    fn fixture(&self, key: &str) -> Result<String, String> {
        self.get(&format!("{SITE}/{key}.json"))
    }

    fn manufacturers(&self) -> Result<BTreeMap<String, String>, String> {
        let text = self.get(&format!("{SITE}/api/v1/manufacturers"))?;
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&text).map_err(|e| format!("réponse illisible : {e}"))?;
        Ok(map
            .into_iter()
            .filter_map(|(key, value)| Some((key, value.get("name")?.as_str()?.to_string())))
            .collect())
    }
}

enum Reply {
    Results(Result<Vec<Hit>, String>),
    /// Fiche téléchargée, avec le nom de son constructeur.
    Fixture(Result<(String, String), String>),
}

/// Ce que la boîte a donné à cette image.
pub enum SearchOutcome {
    Open,
    Cancelled,
    /// Fiche choisie : (texte JSON, nom du constructeur).
    Chosen(String, String),
}

pub struct OnlineSearch {
    source: Arc<dyn Source>,
    query: String,
    results: Option<Vec<Hit>>,
    waiting: Option<(String, Receiver<Reply>)>,
    error: Option<String>,
    focus: bool,
}

impl OnlineSearch {
    pub fn new(source: Arc<dyn Source>) -> Self {
        Self { source, query: String::new(), results: None, waiting: None, error: None, focus: true }
    }

    fn start(&mut self, what: String, job: impl FnOnce(&dyn Source) -> Reply + Send + 'static, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let source = self.source.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job(source.as_ref()));
            ctx.request_repaint();
        });
        self.waiting = Some((what, rx));
        self.error = None;
    }

    pub fn search(&mut self, ctx: &egui::Context) {
        let query = self.query.trim().to_string();
        if query.is_empty() {
            return;
        }
        self.start(format!("Recherche de « {query} »…"), move |s| Reply::Results(s.search(&query)), ctx);
    }

    pub fn choose(&mut self, hit: &Hit, ctx: &egui::Context) {
        let key = hit.key.clone();
        self.start(
            format!("Téléchargement de {}…", hit.name),
            move |s| {
                Reply::Fixture(s.fixture(&key).map(|json| {
                    let manufacturer_key = key.split('/').next().unwrap_or_default();
                    let manufacturer = s
                        .manufacturers()
                        .ok()
                        .and_then(|m| m.get(manufacturer_key).cloned())
                        .unwrap_or_default();
                    (json, manufacturer)
                }))
            },
            ctx,
        );
    }

    pub fn show(&mut self, ctx: &egui::Context) -> SearchOutcome {
        let mut outcome = SearchOutcome::Open;
        if let Some((_, rx)) = &self.waiting {
            match rx.try_recv() {
                Ok(Reply::Results(Ok(hits))) => {
                    self.results = Some(hits);
                    self.waiting = None;
                }
                Ok(Reply::Fixture(Ok((json, manufacturer)))) => {
                    self.waiting = None;
                    return SearchOutcome::Chosen(json, manufacturer);
                }
                Ok(Reply::Results(Err(e)) | Reply::Fixture(Err(e))) => {
                    self.error = Some(e);
                    self.waiting = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.error = Some("la requête s'est interrompue".into());
                    self.waiting = None;
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }

        let mut chosen = None;
        let response = egui::Modal::new(egui::Id::unique("ofl_search")).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.heading("Rechercher une fiche en ligne");
            ui.label(RichText::new("Dans Open Fixture Library (open-fixture-library.org)").color(theme::MUTED));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let label = ui.label("Appareil");
                let edit = ui
                    .add(egui::TextEdit::singleline(&mut self.query).desired_width(280.0).hint_text("Robin 600, SupraKolor…"))
                    .labelled_by(label.id);
                if std::mem::take(&mut self.focus) {
                    edit.request_focus();
                }
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                if (ui.add_enabled(self.waiting.is_none(), egui::Button::new("Rechercher")).clicked() || enter)
                    && self.waiting.is_none()
                {
                    self.search(ui.ctx());
                }
            });
            ui.add_space(6.0);
            if let Some((what, _)) = &self.waiting {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(what);
                });
            }
            if let Some(error) = &self.error {
                ui.colored_label(theme::DANGER, format!("Échec : {error}"));
            }
            match &self.results {
                Some(hits) if hits.is_empty() => {
                    ui.label(RichText::new("Aucun appareil trouvé.").color(theme::MUTED));
                }
                Some(hits) => {
                    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                        for hit in hits {
                            let button = egui::Button::new(&hit.name).min_size(egui::vec2(ui.available_width(), 0.0));
                            let response = ui.add_enabled(self.waiting.is_none(), button).on_hover_text(&hit.key);
                            if response.clicked() {
                                chosen = Some(hit.clone());
                            }
                        }
                    });
                }
                None => {}
            }
            ui.add_space(6.0);
            if ui.button("Annuler").clicked() {
                outcome = SearchOutcome::Cancelled;
            }
        });
        if let Some(hit) = chosen {
            self.choose(&hit, ctx);
        }
        if response.should_close() && matches!(outcome, SearchOutcome::Open) {
            outcome = SearchOutcome::Cancelled;
        }
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Le vrai site, à lancer à la main : `cargo test -p folie-app ofl_site -- --ignored`.
    #[test]
    #[ignore = "réseau"]
    fn ofl_site_answers() {
        let site = OflSite::new();
        let hits = site.search("ledwash 600").unwrap();
        assert!(hits.iter().any(|h| h.key == "robe/robin-ledwash-600"), "{hits:?}");
        let json = site.fixture("robe/robin-ledwash-600").unwrap();
        assert!(folie_fixture::ofl::parse(&json, "robe/robin-ledwash-600", "Robe").is_ok());
        assert_eq!(site.manufacturers().unwrap()["robe"], "Robe");
    }
}

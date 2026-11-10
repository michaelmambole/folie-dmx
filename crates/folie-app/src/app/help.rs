//! Aide des commandes : la référence `docs/commandes.md`, affichée dans Folie avec une
//! recherche. Un clic sur un exemple le recopie dans la ligne de commande.

use std::sync::OnceLock;

use eframe::egui::{self, RichText};
use egui::text::LayoutJob;

use super::FolieApp;
use crate::theme;

/// État de la fenêtre d'aide (pas enregistré).
#[derive(Default)]
pub struct HelpView {
    pub open: bool,
    pub search: String,
}

/// Morceau de la référence.
#[derive(Clone)]
enum Block {
    Title(String),
    Section(String),
    Paragraph(String),
    Bullet(String),
    /// Tableau : en-têtes, lignes. Les exemples d'un tableau « Commande » se cliquent.
    Table { header: Vec<String>, rows: Vec<Vec<String>> },
}

/// La référence découpée en sections : (titre, morceaux). La première, sans titre, est
/// l'introduction.
fn sections() -> &'static [(String, Vec<Block>)] {
    static SECTIONS: OnceLock<Vec<(String, Vec<Block>)>> = OnceLock::new();
    SECTIONS.get_or_init(|| parse(folie_control::command::REFERENCE))
}

fn cells(line: &str) -> Vec<String> {
    let inner = line.trim().trim_start_matches('|').trim_end_matches('|');
    inner.split('|').map(|c| c.trim().to_string()).collect()
}

fn parse(text: &str) -> Vec<(String, Vec<Block>)> {
    let mut sections = vec![(String::new(), Vec::new())];
    let mut paragraph = String::new();
    let flush = |paragraph: &mut String, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(std::mem::take(paragraph)));
        }
    };
    for line in text.lines() {
        let blocks = &mut sections.last_mut().expect("une section").1;
        let line = line.trim_end();
        if let Some(title) = line.strip_prefix("## ") {
            flush(&mut paragraph, blocks);
            sections.push((title.to_string(), vec![Block::Section(title.to_string())]));
        } else if let Some(title) = line.strip_prefix("# ") {
            flush(&mut paragraph, blocks);
            blocks.push(Block::Title(title.to_string()));
        } else if line.starts_with('|') {
            flush(&mut paragraph, blocks);
            if line.starts_with("|---") {
                continue;
            }
            match blocks.last_mut() {
                Some(Block::Table { rows, .. }) => rows.push(cells(line)),
                _ => blocks.push(Block::Table { header: cells(line), rows: Vec::new() }),
            }
        } else if let Some(item) = line.strip_prefix("- ") {
            flush(&mut paragraph, blocks);
            blocks.push(Block::Bullet(item.to_string()));
        } else if line.is_empty() {
            flush(&mut paragraph, blocks);
        } else if let Some(Block::Bullet(item)) = blocks.last_mut().filter(|_| paragraph.is_empty() && line.starts_with("  ")) {
            item.push(' ');
            item.push_str(line.trim());
        } else {
            if !paragraph.is_empty() {
                paragraph.push(' ');
            }
            paragraph.push_str(line);
        }
    }
    let blocks = &mut sections.last_mut().expect("une section").1;
    flush(&mut paragraph, blocks);
    sections
}

/// Texte avec ses `exemples` en police à chasse fixe.
fn rich(text: &str, size: f32, color: egui::Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    for (i, part) in text.split('`').enumerate() {
        let format = if i % 2 == 1 {
            egui::TextFormat { font_id: egui::FontId::monospace(size - 0.5), color: theme::SELECTION, ..Default::default() }
        } else {
            egui::TextFormat { font_id: egui::FontId::proportional(size), color, ..Default::default() }
        };
        job.append(part, 0.0, format);
    }
    job.wrap.max_width = f32::INFINITY;
    job
}

fn paragraph(ui: &mut egui::Ui, text: &str) {
    let mut job = rich(text, 13.5, theme::TEXT);
    job.wrap.max_width = ui.available_width();
    ui.label(job);
}

/// Sans accents ni majuscules, pour chercher « fondu » dans « Fondu ».
fn plain(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

fn row_text(row: &[String]) -> String {
    row.join(" ")
}

fn block_text(block: &Block) -> String {
    match block {
        Block::Title(t) | Block::Section(t) | Block::Paragraph(t) | Block::Bullet(t) => t.clone(),
        Block::Table { header, rows } => header.iter().chain(rows.iter().flatten()).cloned().collect::<Vec<_>>().join(" "),
    }
}

/// Dessine des morceaux de la référence ; rend l'exemple cliqué.
fn blocks(ui: &mut egui::Ui, blocks: &[Block], id: &str) -> Option<String> {
    let mut picked = None;
    for (n, block) in blocks.iter().enumerate() {
        match block {
            Block::Title(title) => {
                ui.label(RichText::new(title).size(20.0).strong());
                ui.add_space(4.0);
            }
            Block::Section(title) => {
                ui.add_space(12.0);
                ui.label(RichText::new(title).size(16.0).strong().color(theme::TEXT));
                ui.add_space(4.0);
            }
            Block::Paragraph(text) => {
                paragraph(ui, text);
                ui.add_space(4.0);
            }
            Block::Bullet(text) => {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new("•").color(theme::MUTED));
                    paragraph(ui, text);
                });
            }
            Block::Table { header, rows } => {
                let commands = header.first().is_some_and(|h| h == "Commande");
                egui::Grid::new(format!("{id}-{n}")).num_columns(header.len()).striped(true).spacing([14.0, 5.0]).show(ui, |ui| {
                    for h in header {
                        ui.label(RichText::new(h).size(12.0).color(theme::MUTED));
                    }
                    ui.end_row();
                    for row in rows {
                        for (c, cell) in row.iter().enumerate() {
                            if c == 0 && commands {
                                // Les exemples se cliquent : recopiés dans la ligne de commande.
                                ui.horizontal(|ui| {
                                    for example in cell.split('`').skip(1).step_by(2) {
                                        let button = egui::Button::new(RichText::new(example).monospace().color(theme::SELECTION))
                                            .frame(false)
                                            .wrap_mode(egui::TextWrapMode::Extend);
                                        if ui.add(button).on_hover_text("Recopier dans la ligne de commande").clicked() {
                                            picked = Some(example.to_string());
                                        }
                                    }
                                });
                            } else {
                                let mut job = rich(cell, 13.0, theme::TEXT);
                                job.wrap.max_width = 460.0;
                                ui.label(job);
                            }
                        }
                        ui.end_row();
                    }
                });
                ui.add_space(6.0);
            }
        }
    }
    picked
}

impl FolieApp {
    pub(super) fn help_window(&mut self, ctx: &egui::Context) {
        if !self.help.open {
            return;
        }
        let mut open = true;
        let mut picked = None;
        egui::Window::new("Aide des commandes")
            .open(&mut open)
            .default_size([780.0, 680.0])
            .resizable(true)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("Chercher");
                    ui.add(egui::TextEdit::singleline(&mut self.help.search).hint_text("fondu, preset, Sound…").desired_width(220.0))
                        .labelled_by(label.id);
                    if !self.help.search.is_empty() && ui.small_button("Effacer").clicked() {
                        self.help.search.clear();
                    }
                    ui.label(RichText::new("Clic sur un exemple : il passe dans la ligne de commande").size(11.5).color(theme::MUTED));
                });
                ui.separator();
                let query = plain(self.help.search.trim());
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let mut found = false;
                    for (i, (title, section)) in sections().iter().enumerate() {
                        // Une recherche garde, dans chaque section, ce qui la contient : les
                        // paragraphes, et dans les tableaux les seules lignes qui conviennent.
                        let shown: Vec<Block> = if query.is_empty() {
                            section.clone()
                        } else if plain(title).contains(&query) {
                            section.iter().filter(|b| !matches!(b, Block::Title(_))).cloned().collect()
                        } else {
                            let matching: Vec<Block> = section
                                .iter()
                                .filter_map(|b| match b {
                                    Block::Table { header, rows } => {
                                        let rows: Vec<Vec<String>> = rows.iter().filter(|r| plain(&row_text(r)).contains(&query)).cloned().collect();
                                        (!rows.is_empty()).then(|| Block::Table { header: header.clone(), rows })
                                    }
                                    b => plain(&block_text(b)).contains(&query).then(|| b.clone()),
                                })
                                .collect();
                            if matching.is_empty() {
                                continue;
                            }
                            section.iter().filter(|b| matches!(b, Block::Section(_))).cloned().chain(matching).collect()
                        };
                        if shown.iter().any(|b| !matches!(b, Block::Section(_))) {
                            found = true;
                            if let Some(example) = blocks(ui, &shown, &format!("aide-{i}")) {
                                picked = Some(example);
                            }
                        }
                    }
                    if !found {
                        ui.label(RichText::new(format!("Rien sur « {} »", self.help.search.trim())).color(theme::MUTED));
                    }
                });
            });
        self.help.open = open;
        if let Some(example) = picked {
            self.command_line.input = example;
            self.command_line.feedback = Some((true, "Exemple recopié : complétez-le, puis Entrée".into()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_is_split_into_sections() {
        let sections = sections();
        assert!(sections.len() > 8);
        assert!(matches!(sections[0].1.first(), Some(Block::Title(t)) if t == "Commandes de Folie-DMX"));
        let sounds = sections.iter().find(|(t, _)| t == "Sons").expect("section Sons");
        let table = sounds.1.iter().find_map(|b| match b {
            Block::Table { header, rows } => Some((header, rows)),
            _ => None,
        });
        let (header, rows) = table.expect("tableau des sons");
        assert_eq!(header, &["Commande", "Effet"]);
        assert_eq!(rows[0][0], "`Sound 3`");
        assert_eq!(plain("Fondu Éteint"), "fondu eteint");
    }
}

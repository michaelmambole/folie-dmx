//! Pools de groupes et de presets : des grilles de cases numérotées, comme sur MA.
//! Un clic rappelle la case ; avec « Store » armé, un clic y enregistre.
//!
//! Chaque case montre son contenu d'un coup d'œil : bandeau de couleur (la
//! couleur obtenue pour un preset Couleur, la couleur de famille sinon), LED de
//! disponibilité pour la sélection, LED rouge s'il est dans le programmer, badge
//! des executors qui le jouent.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32, FontId, RichText, Stroke};
use folie_control::command::Object;
use folie_control::{CommandError, Console, Outcome, SelectMode};
use folie_engine::Snapshot;
use folie_fixture::{FeatureGroup, FixtureId};

use crate::theme;

const CELL: egui::Vec2 = egui::vec2(104.0, 66.0);
const SPACING: f32 = 6.0;
/// Cases affichées au minimum, comme un pool de console.
const MIN_CELLS: usize = 48;
/// Rangées libres toujours visibles après la dernière case occupée.
const FREE_ROWS: usize = 2;
const ARMED: Color32 = theme::DANGER;

pub struct PoolsView {
    /// Macro dont on modifie les lignes, et le texte en cours.
    editing: Option<(u32, String)>,
}

/// Touche Store : armée, le prochain clic dans un pool, sur un executor ou une
/// cue enregistre au lieu de rappeler. Une seule pour toute l'application.
pub fn store_button(ui: &mut egui::Ui, armed: &mut bool) {
    let text = RichText::new("Store").family(theme::medium());
    let button = if *armed { egui::Button::new(text.color(Color32::WHITE)).fill(ARMED) } else { egui::Button::new(text) };
    store_with(ui, armed, button);
}

/// Touche STORE de la barre du haut : cadre rouge, pleine quand elle est armée.
pub fn store_key(ui: &mut egui::Ui, armed: &mut bool) {
    let button = if *armed {
        theme::key_button("STORE", ARMED, ARMED, Color32::WHITE)
    } else {
        theme::key_button("STORE", ARMED, theme::SURFACE_1, Color32::from_rgb(255, 138, 132))
    };
    store_with(ui, armed, button);
}

fn store_with(ui: &mut egui::Ui, armed: &mut bool, button: egui::Button<'_>) {
    let help = "Armer (touche S), puis cliquer une case de pool, un executor ou une cue pour y enregistrer (Échap pour annuler)";
    if ui.add(button).on_hover_text(help).clicked() {
        *armed = !*armed;
    }
}

/// Couverture d'une case pour la sélection en cours.
#[derive(Clone, Copy, PartialEq)]
enum Coverage {
    /// Pas de sélection : rien à dire.
    NoSelection,
    None,
    Partial,
    Full,
}

impl Coverage {
    fn of(selected: &BTreeSet<FixtureId>, contained: &BTreeSet<FixtureId>) -> Self {
        if selected.is_empty() {
            return Self::NoSelection;
        }
        match selected.intersection(contained).count() {
            0 => Self::None,
            n if n == selected.len() => Self::Full,
            _ => Self::Partial,
        }
    }

    fn led(self) -> Option<Color32> {
        match self {
            Self::Full => Some(theme::GO),
            Self::Partial => Some(theme::EXECUTOR),
            Self::None | Self::NoSelection => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Full => "couvre toute la sélection",
            Self::Partial => "couvre une partie de la sélection",
            Self::None => "ne couvre pas la sélection",
            Self::NoSelection => "",
        }
    }
}

/// Ce qu'affiche une case occupée.
struct Item {
    number: u32,
    name: String,
    /// Bandeau à gauche de la case.
    accent: Color32,
    /// Couleur réelle (preset Couleur) : bandeau plus large et fond teinté.
    colored: bool,
    coverage: Coverage,
    /// LED jaune (groupe = sélection) ou rouge (preset dans le programmer).
    marker: Option<Color32>,
    /// Executors qui le jouent en ce moment.
    executors: Vec<u32>,
    detail: String,
}

impl PoolsView {
    pub fn new() -> Self {
        Self { editing: None }
    }

    /// Pool de groupes. Renvoie le résultat de l'action faite, s'il y en a une.
    pub fn groups(
        &mut self,
        ui: &mut egui::Ui,
        console: &mut Console,
        store_armed: &mut bool,
    ) -> Option<Result<Outcome, CommandError>> {
        toolbar(ui, store_armed);
        let selected: BTreeSet<FixtureId> = console.selection().iter().map(|t| t.fixture).collect();
        let items: Vec<Item> = console
            .show
            .groups
            .iter()
            .map(|(n, g)| {
                let contained: BTreeSet<FixtureId> = g.targets.iter().map(|t| t.fixture).collect();
                let exact = console.selection() == g.targets.as_slice();
                Item {
                    number: *n,
                    name: g.name.clone(),
                    accent: theme::SELECTION.gamma_multiply(0.6),
                    colored: false,
                    coverage: Coverage::of(&selected, &contained),
                    marker: exact.then_some(theme::SELECTION),
                    executors: Vec::new(),
                    detail: format!("{} cible(s)", g.targets.len()),
                }
            })
            .collect();
        let action = grid(ui, "groups", &items, *store_armed, false)?;
        Some(match action {
            CellAction::Click(number) => {
                let add = ui.input(|i| i.modifiers.command);
                let result = if *store_armed {
                    console.store_group(number, None)
                } else {
                    console.select_group(number, if add { SelectMode::Add } else { SelectMode::Replace })
                };
                *store_armed = false;
                result
            }
            CellAction::Rename(number, name) => console.label(Object::Group(number), name),
            CellAction::Delete(number) => console.delete(Object::Group(number)),
            CellAction::Edit(_) => return None,
        })
    }

    /// Pool de presets d'une famille, choisie dans la fenêtre.
    pub fn presets(
        &mut self,
        ui: &mut egui::Ui,
        console: &mut Console,
        snapshot: &Snapshot,
        family: &mut FeatureGroup,
        store_armed: &mut bool,
    ) -> Option<Result<Outcome, CommandError>> {
        toolbar(ui, store_armed);
        family_tabs(ui, family);
        ui.add_space(4.0);
        let current = *family;
        let selected: BTreeSet<FixtureId> = console.selection().iter().map(|t| t.fixture).collect();
        let in_programmer = console.programmer_presets();
        let playing: BTreeMap<u32, usize> =
            snapshot.executors.iter().filter_map(|(e, s)| s.cue.map(|c| (*e, c))).collect();
        let in_use = console.presets_in_use(&playing);
        let items: Vec<Item> = console
            .show
            .presets
            .iter()
            .filter(|((f, _), _)| *f == current)
            .map(|((_, n), p)| {
                let contained: BTreeSet<FixtureId> = p.values.keys().copied().collect();
                let color = (current == FeatureGroup::Color)
                    .then(|| folie_control::preset_color(p, &console.show.patch))
                    .flatten()
                    .map(|[r, g, b]| Color32::from_rgb(r, g, b));
                Item {
                    number: *n,
                    name: p.name.clone(),
                    accent: color.unwrap_or_else(|| theme::family_color(current)),
                    colored: color.is_some(),
                    coverage: Coverage::of(&selected, &contained),
                    marker: in_programmer.contains(&(current, *n)).then_some(theme::PROGRAMMER),
                    executors: in_use.get(&(current, *n)).cloned().unwrap_or_default(),
                    detail: format!("{} fixture(s)", p.values.len()),
                }
            })
            .collect();
        let action = grid(ui, ("presets", current), &items, *store_armed, false)?;
        Some(match action {
            CellAction::Click(number) => {
                let result = if *store_armed {
                    console.store_preset(current, number, None)
                } else {
                    console.apply_preset(current, number)
                };
                *store_armed = false;
                result
            }
            CellAction::Rename(number, name) => console.label(Object::Preset(current, number), name),
            CellAction::Delete(number) => console.delete(Object::Preset(current, number)),
            CellAction::Edit(_) => return None,
        })
    }

    /// Pool de macros : clic pour lancer (ou arrêter si elle joue), clic droit pour modifier
    /// ses lignes, la renommer ou la supprimer. Store, puis une case libre : nouvelle macro.
    pub fn macros(&mut self, ui: &mut egui::Ui, console: &mut Console, store_armed: &mut bool) -> Option<Result<Outcome, CommandError>> {
        ui.horizontal(|ui| {
            store_button(ui, store_armed);
            let hint = if *store_armed {
                "Choisissez la case de la nouvelle macro"
            } else {
                "Clic : lancer (ou arrêter) · clic droit : modifier, renommer, supprimer"
            };
            ui.label(RichText::new(hint).size(12.0).color(theme::MUTED));
        });
        ui.add_space(4.0);
        let mut result = self.macro_editor(ui, console);

        let progress = console.macros_in_progress();
        let items: Vec<Item> = console
            .show
            .macros
            .iter()
            .map(|(n, m)| {
                let running = progress.iter().find(|p| p.number == *n);
                let detail = match running {
                    Some(p) if p.waiting => format!("ligne {}/{} · attente", p.line, p.lines),
                    Some(p) => format!("ligne {}/{}", p.line, p.lines),
                    None => format!("{} ligne(s)", m.lines.len()),
                };
                Item {
                    number: *n,
                    name: m.name.clone(),
                    accent: if running.is_some() { theme::GO } else { theme::DEFAULT },
                    // En cours : bandeau large et fond teinté de vert.
                    colored: running.is_some(),
                    coverage: Coverage::NoSelection,
                    marker: running.map(|_| theme::GO),
                    executors: Vec::new(),
                    detail,
                }
            })
            .collect();
        if let Some(action) = grid(ui, "macros", &items, *store_armed, true) {
            result = Some(match action {
                CellAction::Click(number) if *store_armed => {
                    *store_armed = false;
                    let outcome = console.store_macro(number, None);
                    self.editing = Some((number, String::new()));
                    Ok(outcome)
                }
                CellAction::Click(number) if progress.iter().any(|p| p.number == number) => Ok(console.stop_macro(number)),
                CellAction::Click(number) => console.run_macro(number),
                CellAction::Rename(number, name) => console.label_macro(number, name),
                CellAction::Delete(number) => console.delete_macro(number),
                CellAction::Edit(number) => {
                    let lines = console.show.macros.get(&number).map(|m| m.lines.join("\n")).unwrap_or_default();
                    self.editing = Some((number, lines));
                    return result;
                }
            });
        }
        result
    }

    /// Éditeur des lignes de la macro en cours de modification, au-dessus du pool.
    /// Chaque ligne est vérifiée pendant la saisie ; les fautes sont signalées en rouge.
    fn macro_editor(&mut self, ui: &mut egui::Ui, console: &mut Console) -> Option<Result<Outcome, CommandError>> {
        let (number, text) = self.editing.as_mut()?;
        let number = *number;
        let name = console.show.macros.get(&number).map_or_else(|| format!("Macro {number}"), |m| m.name.clone());
        let mut result = None;
        let mut close = false;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Macro {number} · {name}")).family(theme::medium()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(theme::primary_button("Enregistrer")).clicked() {
                        let lines = text.lines().map(str::to_string).collect();
                        result = Some(console.set_macro_lines(number, lines));
                        close = true;
                    }
                    if ui.button("Annuler").clicked() {
                        close = true;
                    }
                });
            });
            let label = ui.label(RichText::new("Une commande par ligne · « Wait 2 » attend 2 secondes").size(11.5).color(theme::MUTED));
            ui.add(
                egui::TextEdit::multiline(text)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(6)
                    .desired_width(f32::INFINITY)
                    .hint_text("Fixture 1 Thru 4\nAt 100\nWait 2\nGo Executor 2"),
            )
            .labelled_by(label.id);
            // Lignes fautives, avec leur numéro.
            for (index, line) in text.lines().enumerate() {
                if let Err(err) = folie_control::macros::check_line(line) {
                    ui.colored_label(theme::DANGER, format!("Ligne {} : {err}", index + 1));
                }
            }
        });
        ui.add_space(6.0);
        if close {
            self.editing = None;
        }
        result
    }
}

fn toolbar(ui: &mut egui::Ui, store_armed: &mut bool) {
    ui.horizontal(|ui| {
        store_button(ui, store_armed);
        let hint = if *store_armed { "Choisissez la case où enregistrer" } else { "Clic : rappeler · clic droit : renommer, supprimer" };
        ui.label(RichText::new(hint).size(12.0).color(theme::MUTED));
    });
    ui.add_space(4.0);
}

/// Onglets de famille, chacun avec sa couleur repère soulignée.
fn family_tabs(ui: &mut egui::Ui, family: &mut FeatureGroup) {
    ui.horizontal_wrapped(|ui| {
        for f in FeatureGroup::ALL {
            let selected = *family == f;
            let text = RichText::new(format!("{} {}", f.preset_number(), f.label()));
            let text = if selected { text.color(theme::TEXT) } else { text.color(theme::MUTED) };
            let response = ui.selectable_label(selected, text);
            let rect = response.rect;
            let height = if selected { 2.0 } else { 1.0 };
            let line = egui::Rect::from_min_max(
                egui::pos2(rect.left() + 4.0, rect.bottom() - height),
                egui::pos2(rect.right() - 4.0, rect.bottom()),
            );
            let alpha = if selected { 1.0 } else { 0.5 };
            ui.painter().rect_filled(line, 1.0, theme::family_color(f).gamma_multiply(alpha));
            if response.clicked() {
                *family = f;
            }
        }
    });
}

/// Ce que l'utilisateur a fait sur une case.
enum CellAction {
    Click(u32),
    Rename(u32, String),
    Delete(u32),
    /// Modifier le contenu (les lignes d'une macro).
    Edit(u32),
}

/// Menu du clic droit d'une case occupée : renommer, supprimer.
fn cell_menu(ui: &mut egui::Ui, id: egui::Id, number: u32, name: &str, editable: bool) -> Option<CellAction> {
    theme::menu_fields(ui);
    let mut action = None;
    // Le texte en cours de saisie est gardé tant que le menu est ouvert.
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| name.to_string());
    ui.horizontal(|ui| {
        let label = ui.label("Nom");
        let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(150.0)).labelled_by(label.id);
        if !edit.has_focus() && !edit.lost_focus() && text == name {
            edit.request_focus();
        }
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Renommer").clicked() || enter) && !text.trim().is_empty() {
            action = Some(CellAction::Rename(number, text.trim().to_string()));
        }
    });
    ui.separator();
    if editable && ui.button("Modifier les lignes").clicked() {
        action = Some(CellAction::Edit(number));
    }
    if ui.button("Supprimer").on_hover_text(format!("Supprimer la case {number}")).clicked() {
        action = Some(CellAction::Delete(number));
    }
    if action.is_some() {
        ui.data_mut(|d| d.remove::<String>(id));
        ui.close();
    } else {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    action
}

/// Dessine une case et renvoie sa réponse. `item` : `None` pour une case libre.
fn cell(ui: &mut egui::Ui, number: u32, item: Option<&Item>, armed: bool) -> egui::Response {
    let enabled = item.is_some() || armed;
    let sense = if enabled { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(CELL, sense);
    let painter = ui.painter_at(rect.expand(1.0));
    let hovered = response.hovered() && enabled;

    let (fill, stroke) = match item {
        Some(_) => (if hovered { theme::SURFACE_3 } else { theme::SURFACE_2 }, theme::LINE),
        None => (if hovered { theme::SURFACE_1 } else { theme::BG }, theme::SURFACE_2),
    };
    painter.rect_filled(rect, 8.0, fill);
    // Contour : rouge si Store est armé ou si le preset est dans le programmer.
    let in_programmer = item.and_then(|i| i.marker) == Some(theme::PROGRAMMER);
    let outline = if armed {
        Stroke::new(1.5, ARMED)
    } else if in_programmer {
        Stroke::new(1.5, theme::PROGRAMMER)
    } else {
        Stroke::new(1.0, stroke)
    };
    painter.rect_stroke(rect, 8.0, outline, egui::StrokeKind::Inside);

    let number_color = if item.is_some() { theme::MUTED } else { theme::SURFACE_3 };
    let text_left = rect.left() + match item {
        Some(i) if i.colored => 18.0,
        Some(_) => 14.0,
        None => 8.0,
    };
    painter.text(
        egui::pos2(text_left, rect.top() + 6.0),
        egui::Align2::LEFT_TOP,
        number.to_string(),
        FontId::monospace(11.0),
        number_color,
    );

    let Some(item) = item else {
        if armed {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "Store", FontId::proportional(12.0), ARMED);
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, number.to_string()));
        return response;
    };

    // Bandeau de couleur à gauche ; pour une vraie couleur, plus large et fond teinté.
    if item.colored {
        painter.rect_filled(rect.shrink(1.0), 7.0, item.accent.gamma_multiply(0.14));
    }
    let band_width = if item.colored { 10.0 } else { 6.0 };
    let band = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + band_width, rect.bottom()));
    painter.rect_filled(band, egui::CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 }, item.accent);

    // LED : disponibilité pour la sélection, puis marqueur (programmer ou groupe = sélection).
    let led_y = rect.top() + 11.0;
    theme::led(&painter, egui::pos2(rect.right() - 11.0, led_y), item.coverage.led());
    if let Some(marker) = item.marker {
        theme::led(&painter, egui::pos2(rect.right() - 24.0, led_y), Some(marker));
    }

    // Nom : deux lignes au plus, coupé proprement.
    let mut job = egui::text::LayoutJob::simple(
        item.name.clone(),
        FontId::new(13.0, theme::medium()),
        theme::TEXT,
        rect.width() - 22.0,
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = false;
    job.wrap.overflow_character = Some('…');
    let galley = painter.layout_job(job);
    painter.galley(egui::pos2(text_left, rect.top() + 22.0), galley, theme::TEXT);

    // Badge des executors qui jouent ce preset.
    if let Some(first) = item.executors.first() {
        let badge = if item.executors.len() > 1 { format!("E{first}+") } else { format!("E{first}") };
        let galley = painter.layout_no_wrap(badge, FontId::monospace(10.0), Color32::BLACK);
        let size = galley.size() + egui::vec2(8.0, 2.0);
        let badge_rect = egui::Rect::from_min_size(rect.right_bottom() - size - egui::vec2(5.0, 5.0), size);
        painter.rect_filled(badge_rect, 3.0, theme::EXECUTOR);
        painter.galley(badge_rect.min + egui::vec2(4.0, 1.0), galley, Color32::BLACK);
    }

    let label = format!("{number}\n{}", item.name);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    response
}

/// Grille de cases : au moins 48, et toujours deux rangées libres.
/// Une case libre n'est cliquable que pour enregistrer.
fn grid(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    items: &[Item],
    armed: bool,
    editable: bool,
) -> Option<CellAction> {
    // Autant de colonnes que la largeur de la fenêtre le permet.
    let columns = (((ui.available_width() + SPACING) / (CELL.x + SPACING)) as usize).max(2);
    let last = items.iter().map(|i| i.number).max().unwrap_or(0) as usize;
    let rows = (last.div_ceil(columns) + FREE_ROWS).max(MIN_CELLS.div_ceil(columns));
    let cells = rows * columns;
    let mut action = None;
    egui::Grid::new(id).spacing([SPACING, SPACING]).show(ui, |ui| {
        for number in 1..=cells as u32 {
            let item = items.iter().find(|i| i.number == number);
            let mut response = cell(ui, number, item, armed);
            response = match item {
                Some(i) => {
                    let coverage = i.coverage.label();
                    let playing = if i.executors.is_empty() {
                        String::new()
                    } else {
                        let list: Vec<String> = i.executors.iter().map(|e| e.to_string()).collect();
                        format!(" · joué par l'executor {}", list.join(", "))
                    };
                    let sep = if coverage.is_empty() { "" } else { " · " };
                    response.on_hover_text(format!(
                        "{} ({}){sep}{coverage}{playing}\nClic droit : renommer, supprimer",
                        i.name, i.detail
                    ))
                }
                None => response.on_hover_text("Case libre : Store, puis clic"),
            };
            if response.clicked() {
                action = Some(CellAction::Click(number));
            }
            if let Some(i) = item {
                let menu_id = egui::Id::unique(("pool-rename", id, number));
                response.context_menu(|ui| {
                    if let Some(a) = cell_menu(ui, menu_id, number, &i.name, editable) {
                        action = Some(a);
                    }
                });
            }
            if (number as usize).is_multiple_of(columns) {
                ui.end_row();
            }
        }
    });
    action
}

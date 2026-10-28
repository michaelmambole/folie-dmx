//! Fenêtre Fixtures : une ligne par fixture (et par sous-fixture), avec ce qu'elle
//! fait en ce moment, relu dans la sortie DMX. C'est là qu'on sélectionne en jeu.

use std::collections::{BTreeMap, BTreeSet};

use eframe::egui::{self, Color32, Rect, RichText, Sense, pos2, vec2};
use folie_control::{Console, SelectMode};
use folie_engine::Snapshot;
use folie_fixture::{Attribute, CapabilityKind, FixtureId, ParamKey, Parameter, PatchedFixture, Target};

use crate::app::DisplayMode;
use crate::fixtures::{click_select, shown_value, source_of, value_text};
use crate::theme::{self, Source};

const ROW_HEIGHT: f32 = 24.0;

/// Largeur des colonnes ; le nom prend la place qui reste.
const NUMBER: f32 = 92.0;
const INTENSITY: f32 = 96.0;
const COLOR: f32 = 40.0;
const ANGLE: f32 = 52.0;
const ZOOM: f32 = 56.0;
const STATE: f32 = 110.0;
const MIN_NAME: f32 = 130.0;

#[derive(Default)]
pub struct FixtureSheet {
    expanded: BTreeSet<FixtureId>,
    last_clicked: Option<Target>,
    /// Type affiché (nom complet du type) ; `None` : tous.
    type_filter: Option<String>,
    /// Groupe affiché ; `None` : tous.
    group_filter: Option<u32>,
    /// Glisser en cours : ligne de départ, et sélection à laquelle on ajoute (⌘).
    drag: Option<(Target, Vec<Target>)>,
}

/// Une valeur de la feuille et d'où elle vient.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown<T> {
    pub value: T,
    pub source: Source,
}

/// Ce que la feuille montre d'une cible.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LiveRow {
    /// Intensité de 0 à 1 : le plus haut de ses dimmers.
    pub intensity: Option<Shown<f32>>,
    pub color: Option<Shown<[u8; 3]>>,
    pub pan: Option<Shown<String>>,
    pub tilt: Option<Shown<String>>,
    pub zoom: Option<Shown<String>>,
    /// Le shutter est sur une plage de strobe.
    pub strobe: Option<Source>,
    /// Effets qui l'animent, programmer et executors confondus.
    pub effects: Vec<String>,
}

/// La source la plus parlante d'un ensemble de paramètres : effet, puis programmer, puis executor.
fn strongest(sources: impl IntoIterator<Item = Source>) -> Source {
    let rank = |s: &Source| match s {
        Source::Effect => 3,
        Source::Programmer => 2,
        Source::Executor => 1,
        Source::Default => 0,
    };
    sources.into_iter().max_by_key(rank).unwrap_or(Source::Default)
}

/// Paramètres animés par un effet en cours, et le nom de cet effet.
pub fn animated(console: &Console, snapshot: &Snapshot) -> BTreeMap<ParamKey, String> {
    let patch = &console.show.patch;
    let mut effects: Vec<&folie_show::Effect> = console.programmer_effects().iter().collect();
    for (executor, status) in &snapshot.executors {
        let Some(cue) = status.cue else { continue };
        let Some(sequence) = console.show.executors.get(executor).and_then(|x| console.show.sequences.get(&x.sequence)) else {
            continue;
        };
        if let Some(active) = folie_control::cues::effects_by_cue(sequence).get(cue) {
            effects.extend(active.iter().map(|(_, _, effect)| *effect));
        }
    }
    let mut keys = BTreeMap::new();
    for effect in effects {
        for line in folie_control::effects::instance(effect, 0, patch).lines {
            for (key, _) in line.params {
                keys.entry(key).or_insert_with(|| effect.name.clone());
            }
        }
    }
    keys
}

/// Valeurs en direct d'une cible, avec leur source.
pub fn live_row(
    console: &Console,
    snapshot: &Snapshot,
    target: Target,
    animated: &BTreeMap<ParamKey, String>,
    display: DisplayMode,
) -> LiveRow {
    let Some(fixture) = console.show.patch.get(target.fixture) else { return LiveRow::default() };
    let parameters = console.show.patch.parameters_of(target);
    let key = |index: usize| ParamKey { fixture: target.fixture, parameter: index };
    let source = |index: usize| source_of(console, snapshot, key(index), animated.contains_key(&key(index)));
    // Ce qui sort ; en blind, le programmer blind par-dessus.
    let value = |i: usize, p: &Parameter| shown_value(console, snapshot, fixture, i, p).unwrap_or(p.default);
    let of = |attribute: Attribute| parameters.iter().find(|(_, p)| p.attribute == attribute);

    let dimmers: Vec<&(usize, &Parameter)> = parameters.iter().filter(|(_, p)| p.attribute == Attribute::Dimmer).collect();
    let intensity = (!dimmers.is_empty()).then(|| Shown {
        value: dimmers.iter().map(|(i, p)| value(*i, p) as f32 / u16::MAX as f32).fold(0.0, f32::max),
        source: strongest(dimmers.iter().map(|(i, _)| source(*i))),
    });

    let color_params: Vec<usize> = parameters
        .iter()
        .filter(|(_, p)| matches!(p.attribute, Attribute::ColorMix(_) | Attribute::ColorMacro))
        .map(|(i, _)| *i)
        .collect();
    let values: BTreeMap<String, u16> = parameters.iter().map(|(i, p)| (p.name.clone(), value(*i, p))).collect();
    let color = if color_params.is_empty() {
        None
    } else {
        console.target_color(target, &values).map(|rgb| Shown { value: rgb, source: strongest(color_params.iter().map(|i| source(*i))) })
    };

    let angle = |attribute: Attribute| {
        of(attribute).map(|(i, p)| Shown { value: format!("{:.0}°", crate::stage_ui::angle_of(p, value(*i, p))), source: source(*i) })
    };
    let zoom = of(Attribute::Zoom).map(|(i, p)| Shown { value: value_text(p, value(*i, p), display), source: source(*i) });
    let strobe = of(Attribute::Shutter).and_then(|(i, p)| {
        let strobing = p.capability_at(value(*i, p)).is_some_and(|c| match &c.kind {
            CapabilityKind::Shutter { effect, .. } => effect != "Open" && effect != "Closed",
            _ => false,
        });
        strobing.then(|| source(*i))
    });
    let mut effects: Vec<String> = parameters.iter().filter_map(|(i, _)| animated.get(&key(*i)).cloned()).collect();
    effects.sort();
    effects.dedup();

    LiveRow { intensity, color, pan: angle(Attribute::Pan), tilt: angle(Attribute::Tilt), zoom, strobe, effects }
}

impl FixtureSheet {
    pub fn show(&mut self, ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot, display: DisplayMode) {
        crate::fixtures::blind_banner(ui, console);
        self.toolbar(ui, console);
        ui.add_space(4.0);
        if console.show.patch.fixtures().is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Aucune fixture patchée").size(16.0).family(theme::medium()));
                ui.label(RichText::new("Ajoutez-en dans la fenêtre Patch.").color(theme::MUTED));
            });
            return;
        }

        let animated = animated(console, snapshot);
        let order: BTreeMap<Target, usize> = console.selection().iter().enumerate().map(|(i, t)| (*t, i + 1)).collect();
        let visible = self.visible_fixtures(console);
        let name_width = (ui.available_width() - NUMBER - INTENSITY - COLOR - 2.0 * ANGLE - ZOOM - STATE - 16.0).max(MIN_NAME);
        header(ui, name_width);

        // Lignes dans l'ordre d'affichage, avec leur place : pour le Maj-clic et le glisser.
        let mut rows: Vec<(Target, Rect)> = Vec::new();
        let mut clicked = None;
        let mut drag_started = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            if visible.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("Aucune fixture ne correspond au filtre.").color(theme::MUTED));
            }
            for fixture in &visible {
                let heads = fixture.mode().heads.len();
                let open = self.expanded.contains(&fixture.id);
                let mut targets = vec![Target::whole(fixture.id)];
                if open {
                    targets.extend((0..heads).map(|head| Target { fixture: fixture.id, head: Some(head) }));
                }
                for target in targets {
                    let live = live_row(console, snapshot, target, &animated, display);
                    let line = SheetLine {
                        target,
                        fixture,
                        live: &live,
                        order: order.get(&target).copied(),
                        expander: (target.head.is_none() && heads > 0).then_some(open),
                        name_width,
                    };
                    let (response, expander) = line.show(ui);
                    if expander {
                        if !self.expanded.remove(&fixture.id) {
                            self.expanded.insert(fixture.id);
                        }
                    } else if response.clicked() {
                        clicked = Some(target);
                    }
                    if response.drag_started() {
                        drag_started = Some(target);
                    }
                    rows.push((target, response.rect));
                }
            }
        });

        let targets: Vec<Target> = rows.iter().map(|(t, _)| *t).collect();
        if let Some(target) = clicked {
            click_select(console, &targets, &mut self.last_clicked, target, ui.input(|i| i.modifiers));
        }
        self.drag_select(ui, console, drag_started, &rows);
    }

    /// Glisser sur les lignes : sélectionne de la ligne de départ à celle sous le pointeur,
    /// dans le sens du geste ; avec ⌘, en plus de la sélection d'avant.
    fn drag_select(&mut self, ui: &egui::Ui, console: &mut Console, started: Option<Target>, rows: &[(Target, Rect)]) {
        if let Some(anchor) = started {
            let base = if ui.input(|i| i.modifiers.command) { console.selection().to_vec() } else { Vec::new() };
            self.drag = Some((anchor, base));
        }
        let Some((anchor, base)) = &self.drag else { return };
        let (down, pointer) = ui.input(|i| (i.pointer.primary_down(), i.pointer.interact_pos()));
        if !down {
            self.drag = None;
            return;
        }
        let position = |t: Target| rows.iter().position(|(r, _)| *r == t);
        let Some(a) = position(*anchor) else { return };
        let Some(b) = pointer.and_then(|p| {
            let y = p.y.clamp(rows.first()?.1.top(), rows.last()?.1.bottom() - 1.0);
            rows.iter().position(|(_, rect)| rect.y_range().contains(y))
        }) else {
            return;
        };
        let range: Vec<Target> =
            if a <= b { rows[a..=b].iter().map(|(t, _)| *t).collect() } else { rows[b..=a].iter().rev().map(|(t, _)| *t).collect() };
        let mut wanted = base.clone();
        wanted.extend(range.into_iter().filter(|t| !base.contains(t)));
        if console.selection() != wanted.as_slice() {
            console.select(wanted, SelectMode::Replace);
        }
        self.last_clicked = Some(*anchor);
    }

    /// Fixtures affichées : filtre de type, puis de groupe.
    fn visible_fixtures<'a>(&self, console: &'a Console) -> Vec<&'a PatchedFixture> {
        let group: Option<BTreeSet<FixtureId>> =
            self.group_filter.and_then(|n| console.show.groups.get(&n)).map(|g| g.targets.iter().map(|t| t.fixture).collect());
        console
            .show
            .patch
            .fixtures()
            .iter()
            .filter(|f| self.type_filter.as_ref().is_none_or(|t| *t == f.fixture_type.display_name()))
            .filter(|f| group.as_ref().is_none_or(|g| g.contains(&f.id)))
            .collect()
    }

    /// Filtres (type, groupe), ordre de sélection et touches Tout / Aucune.
    fn toolbar(&mut self, ui: &mut egui::Ui, console: &mut Console) {
        let mut types: Vec<String> = console.show.patch.fixtures().iter().map(|f| f.fixture_type.display_name()).collect();
        types.sort();
        types.dedup();
        if self.type_filter.as_ref().is_some_and(|t| !types.contains(t)) {
            self.type_filter = None;
        }
        if self.group_filter.is_some_and(|n| !console.show.groups.contains_key(&n)) {
            self.group_filter = None;
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            ui.label(RichText::new("Type").size(11.5).color(theme::MUTED));
            if filter_chip(ui, "Tous", self.type_filter.is_none()).clicked() {
                self.type_filter = None;
            }
            for name in &types {
                let on = self.type_filter.as_ref() == Some(name);
                if filter_chip(ui, name, on).on_hover_text(format!("N'afficher que les {name}")).clicked() {
                    self.type_filter = if on { None } else { Some(name.clone()) };
                }
            }
            ui.add_space(10.0);
            let label = ui.label(RichText::new("Groupe").size(11.5).color(theme::MUTED));
            let current = match self.group_filter.and_then(|n| console.show.groups.get(&n).map(|g| (n, g))) {
                Some((n, g)) => format!("{n} {}", g.name),
                None => "Tous".into(),
            };
            egui::ComboBox::from_id_salt("sheet_group")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.group_filter, None, "Tous");
                    for (n, group) in &console.show.groups {
                        ui.selectable_value(&mut self.group_filter, Some(*n), format!("{n} {}", group.name));
                    }
                })
                .response
                .labelled_by(label.id);
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Sélection").size(11.5).color(theme::MUTED));
            if console.selection().is_empty() {
                ui.label(RichText::new("aucune · clic, glisser, Maj-clic, Cmd-clic").size(11.5).color(theme::MUTED));
            } else {
                crate::fixtures::selection_chips(ui, console.selection());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Aucune").clicked() {
                    console.clear_selection();
                }
                if ui.button("Tout sélectionner").on_hover_text("Les fixtures affichées, dans l'ordre").clicked() {
                    let all: Vec<Target> = self.visible_fixtures(console).iter().map(|f| Target::whole(f.id)).collect();
                    console.select(all, SelectMode::Replace);
                }
            });
        });
    }
}

fn filter_chip(ui: &mut egui::Ui, label: &str, on: bool) -> egui::Response {
    let (stroke, color) = if on { (theme::SELECTION, theme::SELECTION) } else { (theme::LINE, theme::TEXT) };
    let text = RichText::new(label).size(11.5).color(color);
    ui.add(egui::Button::selectable(on, text).stroke(egui::Stroke::new(1.0, stroke)))
}

fn header(ui: &mut egui::Ui, name_width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let painter = ui.painter();
    let columns = [
        ("N°", NUMBER),
        ("Nom", name_width),
        ("Intensité", INTENSITY),
        ("Coul.", COLOR),
        ("Pan", ANGLE),
        ("Tilt", ANGLE),
        ("Zoom", ZOOM),
        ("Strobe · effet", STATE),
    ];
    let mut x = rect.left() + 6.0;
    for (title, width) in columns {
        let font = egui::FontId::proportional(11.0);
        painter.text(pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, title, font, theme::MUTED);
        x += width;
    }
    painter.hline(rect.x_range(), rect.bottom(), egui::Stroke::new(1.0, theme::LINE));
}

/// Une ligne de la feuille, dessinée d'un bloc : toute la ligne se clique et se glisse.
struct SheetLine<'a> {
    target: Target,
    fixture: &'a PatchedFixture,
    live: &'a LiveRow,
    /// Place dans l'ordre de sélection (1 = la première).
    order: Option<usize>,
    /// `Some(déplié)` pour une fixture à sous-fixtures.
    expander: Option<bool>,
    name_width: f32,
}

impl SheetLine<'_> {
    /// Renvoie la réponse de la ligne, et `true` si le chevron a été cliqué.
    fn show(&self, ui: &mut egui::Ui) -> (egui::Response, bool) {
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), Sense::click_and_drag());
        let selected = self.order.is_some();
        let label = self.target.to_string();
        response.widget_info(|| egui::WidgetInfo::selected(egui::accesskit::Role::Row, true, selected, &label));
        let painter = ui.painter_at(rect);
        let fill = if selected {
            theme::SELECTION.gamma_multiply(0.10)
        } else if response.hovered() {
            theme::SURFACE_1
        } else {
            Color32::TRANSPARENT
        };
        painter.rect_filled(rect.shrink2(vec2(0.0, 1.0)), 4.0, fill);
        painter.hline(rect.x_range(), rect.bottom() - 0.5, egui::Stroke::new(1.0, theme::SURFACE_2));
        let y = rect.center().y;
        let small = egui::FontId::proportional(11.0);
        let mono = egui::FontId::monospace(12.0);

        // Chevron, ordre de sélection et numéro ; une cellule est en retrait par son nom.
        let mut expander_clicked = false;
        if let Some(open) = self.expander {
            let chevron = Rect::from_center_size(pos2(rect.left() + 10.0, y), vec2(16.0, ROW_HEIGHT));
            let id = response.id.with("chevron");
            let chevron_response = ui.interact(chevron, id, Sense::click());
            let help = if open { "Masquer les sous-fixtures" } else { "Afficher les sous-fixtures" };
            chevron_response.widget_info(|| egui::WidgetInfo::labeled(egui::accesskit::Role::Button, true, help));
            expander_clicked = chevron_response.on_hover_text(help).clicked();
            let c = chevron.center();
            let points = if open {
                vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)]
            } else {
                vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)]
            };
            painter.add(egui::Shape::convex_polygon(points, theme::MUTED, egui::Stroke::NONE));
        }
        let mut x = rect.left() + 20.0;
        if let Some(order) = self.order {
            painter.circle_filled(pos2(x + 7.0, y), 7.5, theme::SELECTION);
            let font = egui::FontId::proportional(if order > 99 { 8.0 } else { 10.0 });
            painter.text(pos2(x + 7.0, y), egui::Align2::CENTER_CENTER, order.to_string(), font, theme::BG);
        }
        x += 18.0;
        let number = Rect::from_min_size(pos2(x, y - 9.0), vec2(NUMBER - 42.0, 18.0));
        let (stroke, color) = if selected { (theme::SELECTION, theme::SELECTION) } else { (theme::LINE, theme::TEXT) };
        painter.rect(number, 5.0, theme::SURFACE_2, egui::Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
        painter.text(number.center(), egui::Align2::CENTER_CENTER, &label, mono.clone(), color);

        // Nom (et, pour une fixture à sous-fixtures, leur nombre).
        let mut x = rect.left() + NUMBER;
        let name = match self.target.head {
            Some(head) => format!("cellule {}", self.fixture.mode().heads.get(head).map_or("?", |k| k.as_str())),
            None => self.fixture.name.clone(),
        };
        let name_color = if selected {
            theme::SELECTION
        } else if self.target.head.is_some() {
            theme::MUTED
        } else {
            theme::TEXT
        };
        let indent = if self.target.head.is_some() { 14.0 } else { 0.0 };
        let name_rect = Rect::from_min_size(pos2(x + 6.0 + indent, rect.top()), vec2(self.name_width - 12.0 - indent, ROW_HEIGHT));
        let galley = painter.layout(name, egui::FontId::proportional(13.0), name_color, f32::INFINITY);
        let name_end = (name_rect.left() + galley.size().x).min(name_rect.right());
        painter.with_clip_rect(name_rect).galley(pos2(name_rect.left(), y - galley.size().y / 2.0), galley, name_color);
        if self.target.head.is_none() && self.expander.is_some() {
            let count = format!("· {} cellules", self.fixture.mode().heads.len());
            if name_end + 70.0 < name_rect.right() {
                painter.text(pos2(name_end + 6.0, y), egui::Align2::LEFT_CENTER, count, small.clone(), theme::MUTED);
            }
        }
        x += self.name_width;

        // Intensité : jauge dans la couleur de sa source.
        if let Some(Shown { value, source }) = &self.live.intensity {
            let gauge = Rect::from_min_size(pos2(x + 6.0, y - 5.0), vec2(INTENSITY - 16.0, 10.0));
            painter.rect_filled(gauge, 3.0, theme::SURFACE_2);
            let filled = Rect::from_min_size(gauge.min, vec2(gauge.width() * value.clamp(0.0, 1.0), gauge.height()));
            painter.rect_filled(filled, 3.0, source.color());
        }
        x += INTENSITY;

        // Couleur obtenue.
        if let Some(Shown { value: [r, g, b], source }) = &self.live.color {
            let swatch = Rect::from_center_size(pos2(x + 14.0, y), vec2(16.0, 16.0));
            let stroke = if *source == Source::Default { theme::LINE } else { source.color() };
            painter.rect(swatch, 3.0, Color32::from_rgb(*r, *g, *b), egui::Stroke::new(1.5, stroke), egui::StrokeKind::Outside);
        }
        x += COLOR;

        // Pan, tilt, zoom : texte dans la couleur de sa source (gris par défaut).
        for (cell, width) in [(&self.live.pan, ANGLE), (&self.live.tilt, ANGLE), (&self.live.zoom, ZOOM)] {
            if let Some(Shown { value, source }) = cell {
                let color = if *source == Source::Default { theme::MUTED } else { source.color() };
                painter.text(pos2(x + 6.0, y), egui::Align2::LEFT_CENTER, value, mono.clone(), color);
            }
            x += width;
        }

        // Strobe et effets.
        let mut tx = x + 6.0;
        if let Some(source) = self.live.strobe {
            let color = if source == Source::Default { theme::MUTED } else { source.color() };
            let r = painter.text(pos2(tx, y), egui::Align2::LEFT_CENTER, "Strobe", small.clone(), color);
            tx = r.right() + 8.0;
        }
        if !self.live.effects.is_empty() {
            painter.circle_filled(pos2(tx + 3.0, y), 3.0, theme::EFFECT);
            painter.text(pos2(tx + 10.0, y), egui::Align2::LEFT_CENTER, self.live.effects.join(", "), small, theme::EFFECT);
        }

        (response, expander_clicked)
    }
}

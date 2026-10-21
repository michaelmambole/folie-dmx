//! Fenêtres Cues et Executors.

use std::collections::BTreeSet;

use eframe::egui::{self, Color32, RichText};
use folie_control::{CommandError, Console, Outcome};
use folie_engine::{ExecutorStatus, Snapshot};
use folie_fixture::FeatureGroup;
use folie_show::{
    ChaserDirection, Cue, CueNumber, CueValue, ExecutorMode, ExecutorOptions, FaderFunction, MAX_PAGE, Priority, RestartMode,
    SLOTS_PER_PAGE, executor_label, executor_place,
};

use crate::theme;

type Action = Option<Result<Outcome, CommandError>>;

/// Places des executors à fader sur une page : une aile de 10.
pub const FADER_SLOTS: std::ops::RangeInclusive<u32> = 1..=10;
/// Places des executors à boutons, sans fader.
pub const BUTTON_SLOTS: std::ops::RangeInclusive<u32> = 11..=SLOTS_PER_PAGE;
const ARMED: Color32 = crate::theme::DANGER;
const MODE: Color32 = Color32::from_rgb(120, 170, 255);
const DELAY: Color32 = crate::theme::EXECUTOR;
const FADE: Color32 = crate::theme::INFO;
const BAR_HEIGHT: f32 = 14.0;

/// Jauge de transition, toujours à la même place pour que rien ne bouge :
/// ambre pendant le délai, bleue pendant le fondu, vide au repos.
fn transition_bar(ui: &mut egui::Ui, status: Option<&ExecutorStatus>, width: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, BAR_HEIGHT), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    // Chenillard : avancement du pas ; crossfade : position du fader dans la transition.
    let steady = status.and_then(|s| match (s.step, s.manual, s.cue) {
        (Some(step), _, Some(cue)) => Some((step, format!("pas {}/{}", cue + 1, s.cue_count))),
        (_, Some(manual), _) => Some((manual, format!("crossfade {:.0} %", manual * 100.0))),
        _ => None,
    });
    let Some(s) = status.filter(|s| steady.is_some() || s.in_transition()) else { return };
    let (fraction, color, text) = if let Some((fraction, text)) = steady {
        (fraction, FADE, text)
    } else if s.elapsed >= s.delay + s.fade {
        // Temps de la cue écoulé : des paramètres finissent sur leurs temps propres.
        (s.elapsed / s.duration.max(f32::EPSILON), FADE, format!("temps propres {:.1} s", s.duration - s.elapsed))
    } else if s.elapsed < s.delay {
        (s.elapsed / s.delay, DELAY, format!("délai {:.1} s", s.delay - s.elapsed))
    } else {
        let into_fade = s.elapsed - s.delay;
        (into_fade / s.fade.max(f32::EPSILON), FADE, format!("fondu {:.1} s", (s.fade - into_fade).max(0.0)))
    };
    let filled = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * fraction.clamp(0.0, 1.0), rect.height()));
    painter.rect_filled(filled, 3.0, color);
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(11.0), Color32::WHITE);
}

/// Cadre d'un executor, de largeur fixe, contenu centré. Un executor fixé
/// (présent sur toutes les pages) a un cadre en pointillés.
fn card<R>(ui: &mut egui::Ui, width: f32, dim: bool, fixed: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let (fill, stroke) = if dim { (theme::BG, theme::SURFACE_2) } else { (theme::SURFACE_1, theme::LINE) };
    let stroke = if fixed { Color32::TRANSPARENT } else { stroke };
    let frame = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(8)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_width(width);
            ui.vertical_centered(add).inner
        });
    if fixed {
        let r = frame.response.rect.shrink(0.5);
        let outline = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
        ui.painter().extend(egui::Shape::dashed_line(&outline, egui::Stroke::new(1.0, theme::MUTED), 5.0, 3.0));
    }
    frame.inner
}

fn cue_label(number: &CueNumber, name: &str) -> String {
    if name.is_empty() { number.to_string() } else { format!("{number} {name}") }
}

/// Cue en cours et cue suivante d'un executor.
fn cue_lines(console: &Console, executor: u32, status: Option<&ExecutorStatus>) -> (String, String) {
    let Some((_, sequence)) = console.executor(executor) else {
        return ("vide".into(), "Store puis clic ici".into());
    };
    let current = status.and_then(|s| s.cue);
    let label = |i: usize| sequence.cues.iter().nth(i).map(|(n, c)| cue_label(n, &c.name));
    let now = current.and_then(label).unwrap_or_else(|| "éteint".into());
    let next = match current.map_or(Some(0), |c| c.checked_add(1)).and_then(label) {
        Some(next) => next,
        None if sequence.cues.is_empty() => "aucune cue".into(),
        None => "fin de séquence".into(),
    };
    (now, next)
}

fn store_here(ui: &mut egui::Ui, console: &mut Console, executor: u32, store_armed: &mut bool, width: f32) -> Action {
    let button = egui::Button::new(RichText::new("Enregistrer ici").family(theme::medium()).color(Color32::WHITE)).fill(ARMED);
    if ui.add_sized([width, 26.0], button).on_hover_text(format!("Store executor {executor}", executor = executor_label(executor))).clicked() {
        *store_armed = false;
        return Some(console.store_in_executor(executor));
    }
    None
}

/// Touches de l'executor selon son mode. Flash : actif tant que la touche est maintenue.
/// GO est vert quand l'executor a une séquence ; sur un executor vide, tout reste discret.
fn mode_buttons(ui: &mut egui::Ui, console: &mut Console, executor: u32, held: &mut BTreeSet<u32>, width: f32) -> Action {
    // Executor qui lance une macro : une seule touche verte (Arrêter pendant qu'elle joue).
    if let Some((number, m)) = console.executor_macro(executor) {
        let name = m.name.clone();
        let running = console.macros_in_progress().iter().any(|p| p.number == number);
        let (button, help) = if running {
            (egui::Button::new(RichText::new("Arrêter").size(12.0)), format!("Arrêter la macro {number} « {name} »"))
        } else {
            (theme::primary_button("Macro"), format!("Lancer la macro {number} « {name} »"))
        };
        if ui.add_sized([width, 26.0], button).on_hover_text(help).clicked() {
            return Some(if running { Ok(console.stop_macro(number)) } else { console.press(executor) });
        }
        return None;
    }
    let assigned = console.executor(executor).is_some();
    let mode = console.executor(executor).map_or(ExecutorMode::Go, |(x, _)| x.mode);
    let quiet = |text: &'static str| {
        let text = RichText::new(text).size(12.0);
        egui::Button::new(if assigned { text } else { text.color(theme::MUTED) })
    };
    let key = |text: &'static str| if assigned { theme::primary_button(text) } else { quiet(text) };
    let mut action = None;
    match mode {
        ExecutorMode::Go | ExecutorMode::Kill => {
            let main = if mode == ExecutorMode::Kill { "Kill" } else { "Go" };
            ui.spacing_mut().button_padding.x = 4.0;
            ui.spacing_mut().item_spacing = egui::vec2(3.0, 3.0);
            // Assez large : les trois touches côte à côte ; sinon GO en pleine largeur au-dessus.
            let wide = width >= 120.0;
            let go_width = if wide { (width - 6.0) / 3.0 } else { width };
            let small = if wide { go_width } else { (width - 3.0) / 2.0 };
            // Les clics sont notés d'abord, puis joués : la console n'est empruntée qu'une fois.
            let (mut go, mut back, mut off) = (false, false, false);
            let mut back_off = |ui: &mut egui::Ui| {
                back |= ui.add_sized([small, 24.0], quiet("Back")).on_hover_text(format!("Back executor {executor}", executor = executor_label(executor))).clicked();
                off |= ui.add_sized([small, 24.0], quiet("Off")).on_hover_text(format!("Éteindre l'executor {executor}", executor = executor_label(executor))).clicked();
            };
            ui.horizontal(|ui| {
                let help = if mode == ExecutorMode::Kill {
                    format!("Go executor {executor}, et Off sur tous les autres", executor = executor_label(executor))
                } else {
                    format!("Go executor {executor}", executor = executor_label(executor))
                };
                go = ui.add_sized([go_width, 26.0], key(main)).on_hover_text(help).clicked();
                if wide {
                    back_off(ui);
                }
            });
            if !wide {
                ui.horizontal(back_off);
            }
            if back {
                action = Some(console.back(Some(executor)));
            }
            if off {
                action = Some(console.off(Some(executor)));
            }
            if go {
                action = Some(console.press(executor));
            }
        }
        ExecutorMode::Flash | ExecutorMode::Swop => {
            let help = if mode == ExecutorMode::Swop {
                format!("Swop executor {executor} (maintenir) : seul lui éclaire", executor = executor_label(executor))
            } else {
                format!("Flash executor {executor} (maintenir)", executor = executor_label(executor))
            };
            let response = ui.add_sized([width, 26.0], key(mode.label())).on_hover_text(help);
            let down = response.is_pointer_button_down_on();
            if down && held.insert(executor) {
                console.flash(executor, true);
            } else if !down && held.remove(&executor) {
                console.flash(executor, false);
            }
        }
        ExecutorMode::Toggle => {
            let on = console.engine().snapshot().executors.get(&executor).is_some_and(|s| s.cue.is_some());
            let button = if on { theme::primary_button("Bascule") } else { quiet("Bascule") };
            if ui.add_sized([width, 26.0], button).on_hover_text(format!("Basculer l'executor {executor}", executor = executor_label(executor))).clicked() {
                action = Some(console.press(executor));
            }
        }
    }
    action
}

/// Largeur d'une colonne du menu d'un executor.
const MENU_COLUMN: f32 = 280.0;

/// « Lancer une macro » : sous-menu des macros du show.
fn macro_choice(ui: &mut egui::Ui, console: &mut Console, executor: u32) -> Action {
    let macros: Vec<(u32, String)> = console.show.macros.iter().map(|(n, m)| (*n, m.name.clone())).collect();
    if macros.is_empty() {
        ui.weak("Pas de macro : Store Macro 1, ou le pool Macros");
        return None;
    }
    let current = console.executor_macro(executor).map(|(n, _)| n);
    let mut action = None;
    ui.menu_button("Lancer une macro", |ui| {
        for (number, name) in macros {
            if ui.add(egui::Button::selectable(current == Some(number), format!("{number}. {name}"))).clicked() {
                action = Some(console.set_executor_macro(executor, Some(number)));
                ui.close();
            }
        }
    });
    action
}

/// Menu du clic droit : lecture et comportement côte à côte, appliqués dès qu'on
/// les change ; puis nom, séquence, vider.
fn executor_menu(ui: &mut egui::Ui, console: &mut Console, executor: u32) -> Action {
    theme::menu_fields(ui);
    let mut action = None;
    let Some((x, sequence)) = console.executor(executor).map(|(x, s)| (*x, s.name.clone())) else {
        // Sans séquence : vide, ou il lance une macro.
        match console.executor_macro(executor).map(|(n, m)| (n, m.name.clone())) {
            Some((n, name)) => {
                ui.label(RichText::new(format!("Lance la macro {n} « {name} »")).color(theme::GO));
                if ui.button("Ne plus lancer de macro").clicked() {
                    action = Some(console.set_executor_macro(executor, None));
                    ui.close();
                }
            }
            None => {
                ui.label("Executor vide");
                ui.weak("Store, puis cliquez l'executor");
            }
        }
        if let Some(a) = macro_choice(ui, console, executor) {
            action = Some(a);
        }
        return action;
    };
    let (mut mode, mut o) = (x.mode, x.options);
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(MENU_COLUMN);
            playing_options(ui, &mut mode, &mut o);
        });
        // Un séparateur vertical prendrait toute la hauteur et ferait grandir le menu à chaque image.
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width(MENU_COLUMN);
            behaviour_options(ui, &mut o);
        });
    });
    if mode != x.mode {
        action = Some(console.set_executor_mode(executor, mode));
    }
    if o != x.options {
        action = Some(console.set_executor_options(executor, o));
    }
    ui.separator();
    let id = egui::Id::unique(("rename-executor", executor));
    let mut name = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or(sequence);
    ui.horizontal(|ui| {
        let label = ui.label("Nom");
        let edit = ui.add(egui::TextEdit::singleline(&mut name).desired_width(140.0)).labelled_by(label.id);
        if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            action = Some(console.rename_executor(executor, name.clone()));
            ui.close();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, name));
    let sequences: Vec<(u32, String)> = console.show.sequences.iter().map(|(n, s)| (*n, s.name.clone())).collect();
    ui.menu_button("Jouer une autre séquence", |ui| {
        for (number, name) in sequences {
            if ui.add(egui::Button::selectable(x.sequence == number, format!("{number}. {name}"))).clicked() {
                action = Some(console.assign_sequence(executor, number));
                ui.close();
            }
        }
    });
    ui.separator();
    if ui.button("Vider l'executor").clicked() {
        action = Some(console.clear_executor(executor));
        ui.close();
    }
    action
}

/// Durée lisible à la française : « 0,50 ».
fn decimal(value: f32) -> String {
    format!("{value:.2}").replace('.', ",")
}

/// Colonne « lecture » : mode de la touche, chenillard, fonction du fader.
fn playing_options(ui: &mut egui::Ui, mode: &mut ExecutorMode, o: &mut ExecutorOptions) {
    theme::section(ui, "Mode");
    ui.horizontal_wrapped(|ui| {
        for m in ExecutorMode::ALL {
            let help = match m {
                ExecutorMode::Go => "Go, Back, Off : on avance dans la séquence",
                ExecutorMode::Flash => "Allumé à fond tant que la touche est maintenue",
                ExecutorMode::Toggle => "Un appui allume, le suivant éteint",
                ExecutorMode::Swop => "Comme Flash, et les autres executors s'éteignent pendant l'appui",
                ExecutorMode::Kill => "Go, et tous les autres executors s'éteignent",
            };
            ui.selectable_value(mode, m, m.label()).on_hover_text(help);
        }
    });

    ui.add_space(4.0);
    theme::section(ui, "Chenillard");
    ui.checkbox(&mut o.chaser, "Chenillard (les cues défilent seules)");
    ui.add_enabled_ui(o.chaser, |ui| {
        let c = &mut o.chaser_settings;
        ui.horizontal(|ui| {
            let label = ui.label("Vitesse");
            ui.add_enabled(!c.follow_tempo, egui::DragValue::new(&mut c.bpm).range(1.0..=999.0).speed(1.0).suffix(" BPM"))
                .labelled_by(label.id)
                .on_hover_text("Pas par minute");
            if !c.follow_tempo {
                ui.weak(format!("= {} s par pas", decimal(60.0 / c.bpm.max(1.0))));
            }
        });
        ui.checkbox(&mut c.follow_tempo, "Suivre le tempo du show");
        ui.horizontal(|ui| {
            let label = ui.label("Fondu entre pas");
            let mut percent = c.step_fade * 100.0;
            let response = ui
                .add(egui::DragValue::new(&mut percent).range(0.0..=100.0).speed(1.0).suffix(" %"))
                .labelled_by(label.id)
                .on_hover_text("En part de la durée d'un pas");
            if response.changed() {
                c.step_fade = (percent / 100.0).clamp(0.0, 1.0);
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Sens");
            for d in ChaserDirection::ALL {
                ui.selectable_value(&mut c.direction, d, d.label());
            }
        });
    });

    ui.add_space(4.0);
    theme::section(ui, "Fader");
    ui.horizontal_wrapped(|ui| {
        for f in FaderFunction::ALL {
            ui.selectable_value(&mut o.fader, f, f.label());
        }
    });
    let hint = match o.fader {
        FaderFunction::Master => "Dose l'intensité.",
        FaderFunction::Temp => "Dose l'intensité ; l'executor ne joue que fader levé.",
        FaderFunction::Speed => "Vitesse des effets et du chenillard, de x0 à x2.",
        FaderFunction::Crossfade => "Fait la transition vers la cue suivante à la main : en montant, puis en descendant.",
    };
    ui.add(egui::Label::new(RichText::new(hint).size(12.0).color(theme::MUTED)).wrap());
}

/// Colonne « comportement » : démarrage, temps, priorité, protections.
fn behaviour_options(ui: &mut egui::Ui, o: &mut ExecutorOptions) {
    theme::section(ui, "Démarrage");
    ui.horizontal_wrapped(|ui| {
        ui.label("Reprise").on_hover_text("Cue jouée quand on rallume l'executor");
        for r in RestartMode::ALL {
            ui.selectable_value(&mut o.restart, r, r.label());
        }
    });
    ui.checkbox(&mut o.wrap, "Boucler en fin de séquence");
    ui.horizontal(|ui| {
        ui.checkbox(&mut o.auto_start, "Auto start").on_hover_text("Lever le fader allume l'executor");
        ui.checkbox(&mut o.auto_stop, "Auto stop").on_hover_text("Fader à 0 : l'executor s'éteint");
    });

    ui.add_space(4.0);
    theme::section(ui, "Temps");
    ui.horizontal(|ui| {
        let label = ui.label("Off");
        ui.add(egui::DragValue::new(&mut o.off_time).range(0.0..=3600.0).speed(0.1).suffix(" s"))
            .labelled_by(label.id)
            .on_hover_text("Durée de l'extinction");
        ui.add_space(8.0);
        let mut own = o.back_time.is_some();
        ui.checkbox(&mut own, "Back");
        if own {
            let mut t = o.back_time.unwrap_or(1.0);
            ui.add(egui::DragValue::new(&mut t).range(0.0..=3600.0).speed(0.1).suffix(" s")).on_hover_text("Durée du Back");
            o.back_time = Some(t);
        } else {
            o.back_time = None;
            ui.weak("fondu de la cue");
        }
    });

    ui.add_space(4.0);
    theme::section(ui, "Priorité");
    ui.horizontal(|ui| {
        for p in Priority::ALL {
            ui.selectable_value(&mut o.priority, p, p.label());
        }
    });
    ui.checkbox(&mut o.ooo, "Ooo : s'éteint quand tout est repris")
        .on_hover_text("Off on Overwritten : l'executor s'éteint quand d'autres ont repris tout ce qu'il tient");
    ui.checkbox(&mut o.mib, "MIB : prépositionne dans le noir")
        .on_hover_text("Move in Black : les fixtures éteintes prennent d'avance leur position de la cue suivante");

    ui.add_space(4.0);
    theme::section(ui, "Protections");
    ui.checkbox(&mut o.protected, "Tout éteindre").on_hover_text("Ignore « Tout éteindre » (Off All)");
    ui.horizontal(|ui| {
        ui.checkbox(&mut o.swop_protect, "Swop").on_hover_text("Reste allumé quand un autre executor fait un Swop");
        ui.checkbox(&mut o.kill_protect, "Kill").on_hover_text("Reste allumé quand un autre executor fait un Kill");
    });
    ui.checkbox(&mut o.fixed, "Fixé (toutes les pages)").on_hover_text("Reste à sa place sur toutes les pages");
}

/// Repères des options actives, pour la carte de l'executor.
fn option_tags(options: &ExecutorOptions) -> String {
    let mut tags = Vec::new();
    if options.chaser {
        let c = options.chaser_settings;
        tags.push(if c.follow_tempo { "chenillard tempo".to_string() } else { format!("chenillard {:.0}", c.bpm) });
    }
    if options.wrap {
        tags.push("boucle".to_string());
    }
    if options.fader != FaderFunction::Master {
        tags.push(options.fader.label().to_lowercase());
    }
    if options.priority != Priority::Normal {
        tags.push(format!("prio {}", options.priority.label().to_lowercase()));
    }
    if options.ooo {
        tags.push("Ooo".into());
    }
    if options.mib {
        tags.push("MIB".into());
    }
    if options.protected {
        tags.push("protégé".into());
    }
    if options.fixed {
        tags.push("fixe".into());
    }
    tags.join(" · ")
}

/// En-tête d'un executor : numéro (clic : sélection, clic droit : réglages),
/// mode et nom sur une ligne, puis cue en cours et suivante. Hauteur fixe pour
/// que tous les faders restent alignés.
fn executor_header(
    ui: &mut egui::Ui,
    console: &mut Console,
    executor: u32,
    status: Option<&ExecutorStatus>,
    width: f32,
) -> Action {
    let mut action = None;
    let selected = console.selected_executor() == executor;
    let info = console.executor(executor).map(|(x, s)| (x.mode.label(), s.name.clone(), option_tags(&x.options)));
    let launched = console.executor_macro(executor).map(|(n, m)| (n, m.name.clone(), m.lines.len()));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        let title = RichText::new(executor_label(executor)).family(theme::medium());
        let empty = info.is_none() && launched.is_none();
        let title = if selected {
            title.color(theme::SELECTION)
        } else if empty {
            title.color(theme::MUTED)
        } else {
            title
        };
        let response = ui.selectable_label(selected, title).on_hover_text("Sélectionner (clic) · réglages (clic droit)");
        if response.clicked() {
            action = Some(Ok(console.select_executor(executor)));
        }
        // Le menu reste ouvert pendant qu'on coche les options ; un clic à côté le ferme.
        egui::Popup::context_menu(&response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            if let Some(a) = executor_menu(ui, console, executor) {
                action = Some(a);
            }
        });
        match (&info, &launched) {
            (Some((_, name, _)), _) | (None, Some((_, name, _))) => {
                ui.add(egui::Label::new(RichText::new(name).size(12.0)).truncate());
            }
            // Executor vide : rien à côté du numéro, la carte le dit déjà.
            (None, None) => {}
        }
    });
    // Lignes de hauteur fixe, coupées si trop longues : les cartes restent alignées.
    let line = |ui: &mut egui::Ui, text: &str, color: Color32| {
        let layout = egui::Layout::left_to_right(egui::Align::Center);
        ui.allocate_ui_with_layout(egui::vec2(width, 16.0), layout, |ui| {
            ui.set_min_size(egui::vec2(width, 16.0));
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(color)).truncate());
        });
    };
    // Mode et options actives sur une seule ligne : « Go · boucle · protégé ».
    let mode_line = match &info {
        Some((mode, _, tags)) if tags.is_empty() => mode.to_string(),
        Some((mode, _, tags)) => format!("{mode} · {tags}"),
        None => String::new(),
    };
    if let Some((number, _, lines)) = &launched {
        // Macro : son numéro en vert, puis sa progression quand elle joue.
        line(ui, &format!("Macro {number}"), theme::GO);
        let progress = console.macros_in_progress().into_iter().find(|p| p.number == *number);
        let state = match progress {
            Some(p) if p.waiting => format!("ligne {}/{} · attente", p.line, p.lines),
            Some(p) => format!("ligne {}/{}", p.line, p.lines),
            None => format!("{lines} ligne(s)"),
        };
        line(ui, &state, if progress.is_some() { theme::GO } else { theme::MUTED });
        line(ui, "", theme::MUTED);
        transition_bar(ui, status, width);
        return action;
    }
    line(ui, &mode_line, MODE);
    if info.is_some() {
        let (now, next) = cue_lines(console, executor, status);
        let active = status.is_some_and(|s| s.cue.is_some());
        line(ui, &now, if active { theme::EXECUTOR } else { theme::MUTED });
        line(ui, &format!("› {next}"), theme::MUTED);
    } else {
        line(ui, "", theme::MUTED);
        line(ui, "", theme::MUTED);
        // Executor vide : la place de la jauge, sans la jauge.
        ui.allocate_exact_size(egui::vec2(width, BAR_HEIGHT), egui::Sense::hover());
        return action;
    }
    transition_bar(ui, status, width);
    action
}

/// Corps d'un executor vide : la même place qu'un executor plein (les cartes restent
/// alignées), mais seulement un rail en pointillés et « vide ». Rien à cliquer.
fn empty_body(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    let body = ui
        .scope(|ui| {
            ui.set_invisible();
            add(ui);
        })
        .response
        .rect;
    let painter = ui.painter();
    let top = egui::pos2(body.center().x, body.top() + 6.0);
    let bottom = egui::pos2(body.center().x, body.bottom() - 26.0);
    if bottom.y - top.y > 30.0 {
        painter.extend(egui::Shape::dashed_line(&[top, bottom], egui::Stroke::new(1.0, theme::SURFACE_3), 4.0, 4.0));
    }
    let label = egui::pos2(body.center().x, body.bottom() - 12.0);
    painter.text(label, egui::Align2::CENTER_CENTER, "vide", egui::FontId::proportional(11.5), theme::DEFAULT);
}

/// Ce que mémorise une cue, en quelques mots : « 4 fixtures · 1 preset · 1 effet ».
fn cue_contents(cue: &Cue) -> String {
    let count = |n: usize, word: &str| format!("{n} {word}{}", if n > 1 { "s" } else { "" });
    let mut parts = Vec::new();
    if !cue.values.is_empty() {
        parts.push(count(cue.values.len(), "fixture"));
    }
    let presets: BTreeSet<_> = cue
        .values
        .values()
        .flat_map(|values| values.values())
        .filter_map(|v| match v {
            CueValue::Preset(family, number) => Some((*family, *number)),
            _ => None,
        })
        .collect();
    if !presets.is_empty() {
        parts.push(count(presets.len(), "preset"));
    }
    match &cue.effects {
        None => {}
        Some(effects) if effects.is_empty() => parts.push("effets arrêtés".into()),
        Some(effects) => parts.push(count(effects.len(), "effet")),
    }
    if !cue.stopped_effects.is_empty() {
        let n = cue.stopped_effects.len();
        parts.push(format!("{} arrêté{}", count(n, "effet"), if n > 1 { "s" } else { "" }));
    }
    if parts.is_empty() { "vide".into() } else { parts.join(" · ") }
}

/// Place d'une cue par rapport à la cue en cours de l'executor.
#[derive(Clone, Copy, PartialEq)]
enum CueState {
    Past,
    Current,
    Next,
    Later,
}

/// Petite étiquette en capitales : « EN COURS », « SUIVANTE ».
fn tag(ui: &mut egui::Ui, text: &str, fill: Color32, stroke: Color32, color: Color32) {
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(4)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(text).size(10.5).color(color).extra_letter_spacing(0.6)).selectable(false));
        });
}

/// Temps facultatif : grisé tant qu'il reprend `base`, « × » pour y revenir.
/// Renvoie `true` si le temps a changé.
pub fn optional_time(ui: &mut egui::Ui, value: &mut Option<f32>, base: f32, help: String) -> bool {
    let mut changed = false;
    ui.scope(|ui| {
        theme::menu_fields(ui);
        if value.is_none() {
            ui.visuals_mut().widgets.inactive.fg_stroke.color = theme::DEFAULT;
        }
        let mut seconds = value.unwrap_or(base);
        let field = egui::DragValue::new(&mut seconds).range(0.0..=3600.0).speed(0.1).max_decimals(2).suffix(" s");
        if ui.add_sized([62.0, 22.0], field).on_hover_text(help).changed() {
            *value = Some(seconds);
            changed = true;
        }
    });
    let reset = egui::Button::new(RichText::new("×").color(theme::MUTED)).frame(false);
    if value.is_some() && ui.add(reset).on_hover_text("Reprendre le temps de la cue").clicked() {
        *value = None;
        changed = true;
    }
    changed
}

/// Familles des paramètres enregistrés dans une cue, dans l'ordre des presets.
fn cue_families(console: &Console, cue: &Cue) -> Vec<FeatureGroup> {
    let families: BTreeSet<FeatureGroup> = cue
        .values
        .iter()
        .filter_map(|(fixture, params)| Some((console.show.patch.get(*fixture)?, params)))
        .flat_map(|(fixture, params)| {
            params.keys().filter_map(|name| {
                fixture.mode().parameters.iter().find(|p| p.name == *name).map(|p| p.attribute.group())
            })
        })
        .collect();
    families.into_iter().collect()
}

/// Résumé des temps propres d'une cue : « Position 4 s · Couleur 0 s +1 ».
fn family_times_summary(cue: &Cue) -> String {
    cue.family_times
        .iter()
        .map(|(family, t)| {
            let (fade, delay) = t.or((cue.fade, cue.delay));
            let delay = if delay > 0.0 { format!(" +{delay}") } else { String::new() };
            format!("{} {fade} s{delay}", family.label())
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Oublie la saisie en cours du menu d'une cue, à sa fermeture.
fn forget_cue_menu(ctx: &egui::Context, number: CueNumber) {
    ctx.data_mut(|d| {
        d.remove::<String>(egui::Id::unique(("rename-cue", number)));
        d.remove::<bool>(egui::Id::unique(("rename-cue-focused", number)));
        d.remove::<String>(egui::Id::unique(("cue-command", number)));
    });
}

/// Menu du clic droit d'une cue : renommer, temps par famille.
fn cue_menu(ui: &mut egui::Ui, console: &mut Console, number: CueNumber, cue: &Cue) -> Action {
    theme::menu_fields(ui);
    let id = egui::Id::unique(("rename-cue", number));
    let focus_id = egui::Id::unique(("rename-cue-focused", number));
    // Le texte en cours de saisie est gardé tant que le menu est ouvert.
    let mut text = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| cue.name.clone());
    let mut action = None;
    let mut renamed = false;
    ui.horizontal(|ui| {
        let label = ui.label("Nom");
        let edit = ui.add(egui::TextEdit::singleline(&mut text).desired_width(150.0)).labelled_by(label.id);
        // Le champ prend la main à l'ouverture du menu seulement : les temps restent éditables.
        if ui.data_mut(|d| d.get_temp::<bool>(focus_id)).is_none() {
            edit.request_focus();
            ui.data_mut(|d| d.insert_temp(focus_id, true));
        }
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if ui.button("Renommer").on_hover_text(format!("Renommer la cue {number}")).clicked() || enter {
            action = Some(console.label_cue(number, text.trim().to_string()));
            renamed = true;
        }
    });
    // Commande jouée au départ de la cue : OSC vers Reaper, macro, autre executor…
    let command_id = egui::Id::unique(("cue-command", number));
    let mut command = ui.data_mut(|d| d.get_temp::<String>(command_id)).unwrap_or_else(|| cue.command.clone());
    ui.horizontal(|ui| {
        let label = ui.label("Au départ");
        let edit = egui::TextEdit::singleline(&mut command)
            .hint_text("Osc \"Reaper\" \"/play\"")
            .font(egui::TextStyle::Monospace)
            .desired_width(220.0);
        let help = "Jouée quand la cue part : Osc \"Reaper\" \"/marker\" 3, Macro 2, Go Executor 5… Plusieurs : séparées par « ; »";
        let edit = ui.add(edit).labelled_by(label.id).on_hover_text(help);
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Valider").on_hover_text(format!("Commande de la cue {number}")).clicked() || enter) && command.trim() != cue.command {
            action = Some(console.set_cue_command(number, &command));
        }
    });
    ui.data_mut(|d| d.insert_temp(command_id, command));
    ui.horizontal(|ui| {
        let help = "Écrit en dur dans la cue les valeurs qu'elle suit : changer une cue précédente ne la modifie plus";
        if ui.button("Bloquer").on_hover_text(help).clicked() {
            action = Some(console.block_cue(number));
        }
        let help = "Retire de la cue les valeurs identiques à celles qu'elle suivrait";
        if ui.button("Débloquer").on_hover_text(help).clicked() {
            action = Some(console.unblock_cue(number));
        }
    });
    // Effets actifs dans la cue : un effet suivi peut s'arrêter ici, sans les autres.
    let effects = console.cue_effects(number).unwrap_or_default();
    if !effects.is_empty() {
        ui.separator();
        theme::section(ui, "Effets");
        egui::Grid::new(("cue-effects", number)).num_columns(3).spacing([10.0, 4.0]).show(ui, |ui| {
            for effect in &effects {
                let name = RichText::new(&effect.name).color(if effect.stopped { theme::MUTED } else { theme::EFFECT });
                ui.add(egui::Label::new(if effect.stopped { name.strikethrough() } else { name }).selectable(false));
                let status = if effect.here {
                    "lancé ici".to_string()
                } else if effect.stopped {
                    "arrêté ici".to_string()
                } else {
                    format!("suivi depuis la cue {}", effect.origin)
                };
                ui.add(egui::Label::new(RichText::new(status).size(11.5).color(theme::MUTED)).selectable(false));
                if effect.here {
                    ui.label("");
                } else {
                    let (text, help) = if effect.stopped {
                        ("Reprendre", format!("« {} » continue dans la cue {number}", effect.name))
                    } else {
                        ("Arrêter ici", format!("Arrêter « {} » dans la cue {number}, sans les autres effets", effect.name))
                    };
                    if ui.button(text).on_hover_text(help).clicked() {
                        action = Some(console.set_effect_stopped(number, effect.origin, effect.index, !effect.stopped));
                    }
                }
                ui.end_row();
            }
        });
    }
    let families = cue_families(console, cue);
    if !families.is_empty() {
        ui.separator();
        theme::section(ui, "Temps par famille");
        egui::Grid::new(("family-times", number)).num_columns(3).spacing([10.0, 4.0]).show(ui, |ui| {
            let head = |text: &str| egui::Label::new(RichText::new(text).size(11.5).color(theme::MUTED)).selectable(false);
            ui.add(head("Famille"));
            ui.add(head("Fondu"));
            ui.add(head("Délai"));
            ui.end_row();
            for family in families {
                let mut timing = cue.family_times.get(&family).copied().unwrap_or_default();
                ui.add(egui::Label::new(family.label()).selectable(false));
                let mut changed = false;
                ui.horizontal(|ui| {
                    let help = format!("Fondu de {}, cue {number}", family.label());
                    changed |= optional_time(ui, &mut timing.fade, cue.fade, help);
                });
                ui.horizontal(|ui| {
                    let help = format!("Délai de {}, cue {number}", family.label());
                    changed |= optional_time(ui, &mut timing.delay, cue.delay, help);
                });
                if changed {
                    action = Some(console.set_family_times(number, family, timing));
                }
                ui.end_row();
            }
        });
        let help = "Glissez un champ, ou cliquez-le pour taper un temps. En gris : temps de la cue.";
        ui.add(egui::Label::new(RichText::new(help).size(11.0).color(theme::MUTED)).selectable(false));
    }
    if renamed {
        forget_cue_menu(ui.ctx(), number);
        ui.close();
    } else {
        ui.data_mut(|d| d.insert_temp(id, text));
    }
    action
}

/// Une ligne de la liste : numéro (Goto, ou compléter quand Store est armé),
/// nom et contenu, puis fondu, délai et suppression. Clic droit : renommer.
fn cue_row(
    ui: &mut egui::Ui,
    console: &mut Console,
    number: CueNumber,
    cue: &Cue,
    state: CueState,
    store_armed: &mut bool,
) -> Action {
    let mut action = None;
    let (fill, stroke) = match state {
        CueState::Current => (theme::EXECUTOR.gamma_multiply(0.12), theme::EXECUTOR.gamma_multiply(0.55)),
        CueState::Next => (Color32::TRANSPARENT, theme::LINE),
        _ => (Color32::TRANSPARENT, Color32::TRANSPARENT),
    };
    let text_color = if state == CueState::Past { theme::MUTED } else { theme::TEXT };
    // Le fond de la ligne est enregistré avant son contenu : les contrôles gardent leurs clics.
    let row = ui.scope_builder(egui::UiBuilder::new().id_salt(("cue-row", number)).sense(egui::Sense::click()), |ui| {
        egui::Frame::new()
            .fill(fill)
            .stroke(egui::Stroke::new(1.0, stroke))
            .corner_radius(theme::RADIUS)
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                // Champs de temps discrets : fond de la ligne, texte atténué hors cue en cours.
                let widgets = &mut ui.visuals_mut().widgets;
                widgets.inactive.weak_bg_fill = theme::SURFACE_1;
                widgets.inactive.fg_stroke.color = if state == CueState::Current { theme::TEXT } else { theme::MUTED };
                ui.horizontal(|ui| {
                    let (number_fill, number_color) = match state {
                        CueState::Current => (theme::EXECUTOR, Color32::from_rgb(42, 26, 4)),
                        CueState::Past => (theme::SURFACE_1, theme::MUTED),
                        _ => (theme::SURFACE_2, theme::TEXT),
                    };
                    let number_stroke = if *store_armed {
                        egui::Stroke::new(2.0, theme::DANGER)
                    } else {
                        egui::Stroke::new(1.0, if state == CueState::Current { theme::EXECUTOR } else { theme::LINE })
                    };
                    let button = egui::Button::new(RichText::new(number.to_string()).monospace().size(13.0).color(number_color))
                        .fill(number_fill)
                        .stroke(number_stroke)
                        .selected(state == CueState::Current)
                        .min_size(egui::vec2(52.0, 26.0));
                    let help = if *store_armed { "Compléter cette cue avec le programmer" } else { "Aller à cette cue" };
                    if ui.add(button).on_hover_text(help).clicked() {
                        action = Some(if *store_armed {
                            *store_armed = false;
                            console.store_cue(Some(number), None, None)
                        } else {
                            console.goto(number)
                        });
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let delete = egui::Button::new(RichText::new("×").size(15.0).color(theme::MUTED)).frame(false);
                        if ui.add(delete).on_hover_text(format!("Supprimer la cue {number}")).clicked() {
                            action = Some(console.delete_cue(number));
                        }
                        let (mut fade, mut delay) = (cue.fade, cue.delay);
                        fn time(value: &mut f32) -> egui::DragValue<'_> {
                            egui::DragValue::new(value).range(0.0..=3600.0).speed(0.1).max_decimals(2).suffix(" s")
                        }
                        ui.add_sized([62.0, 22.0], time(&mut delay)).on_hover_text(format!("Délai de la cue {number}"));
                        ui.add_sized([62.0, 22.0], time(&mut fade)).on_hover_text(format!("Fondu de la cue {number}"));
                        if fade != cue.fade || delay != cue.delay {
                            action = Some(console.set_cue_times(number, fade, delay));
                        }
                        let layout = egui::Layout::top_down(egui::Align::Min);
                        ui.allocate_ui_with_layout(egui::vec2(ui.available_width(), 34.0), layout, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 6.0;
                                let name = if cue.name.is_empty() { RichText::new("sans nom").italics().color(theme::MUTED) } else {
                                    let name = RichText::new(&cue.name).color(text_color);
                                    if state == CueState::Current { name.family(theme::medium()) } else { name }
                                };
                                ui.add(egui::Label::new(name).truncate().selectable(false));
                                match state {
                                    CueState::Current => tag(ui, "EN COURS", theme::EXECUTOR, theme::EXECUTOR, Color32::from_rgb(42, 26, 4)),
                                    CueState::Next => tag(ui, "SUIVANTE", Color32::TRANSPARENT, theme::MUTED, theme::MUTED),
                                    _ => {}
                                }
                                // Commande jouée au départ : « CMD », la commande au survol.
                                if !cue.command.is_empty() {
                                    let text = RichText::new("CMD").size(10.5).color(theme::SELECTION);
                                    let chip = egui::Button::new(text).stroke(egui::Stroke::new(1.0, theme::SELECTION)).fill(Color32::TRANSPARENT).small();
                                    ui.add(chip).on_hover_text(format!("Au départ de la cue : {}", cue.command));
                                }
                                // Temps propres par famille, en bleu : « Position 4 s ».
                                if !cue.family_times.is_empty() {
                                    let summary = family_times_summary(cue);
                                    let text = RichText::new(summary).size(11.0).color(theme::INFO);
                                    ui.add(egui::Label::new(text).truncate().selectable(false));
                                }
                            });
                            ui.add(egui::Label::new(RichText::new(cue_contents(cue)).size(11.5).color(theme::MUTED)).truncate().selectable(false));
                        });
                    });
                });
            });
    });
    // Le menu reste ouvert pendant qu'on règle les temps ; un clic à côté le ferme.
    let menu = egui::Popup::context_menu(&row.response).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    let open = menu.show(|ui| {
        if let Some(a) = cue_menu(ui, console, number, cue) {
            action = Some(a);
        }
    });
    if open.is_none() {
        forget_cue_menu(ui.ctx(), number);
    }
    action
}

/// « EXECUTOR [3] » : choix de l'executor sélectionné parmi les places de la page courante.
pub fn executor_field(ui: &mut egui::Ui, console: &mut Console) -> Action {
    let mut action = None;
    let executor = console.selected_executor();
    let label = ui.label(RichText::new("EXECUTOR").size(11.0).color(theme::MUTED).extra_letter_spacing(0.8));
    // On choisit une place de la page courante ; le champ affiche le nom de l'executor (« 2.3 »).
    let slot = executor_place(executor).1;
    let labels: Vec<String> = (1..=SLOTS_PER_PAGE)
        .map(|n| executor_label(if n == slot { executor } else { console.executor_at(n) }))
        .collect();
    let mut selected = slot;
    let field = egui::DragValue::new(&mut selected)
        .range(1..=SLOTS_PER_PAGE)
        .custom_formatter(move |n, _| labels[(n as usize).clamp(1, labels.len()) - 1].clone())
        .custom_parser(|text| text.rsplit('.').next()?.trim().parse().ok());
    ui.add(field).labelled_by(label.id);
    if selected != slot {
        action = Some(Ok(console.select_executor(console.executor_at(selected))));
    }
    action
}

/// Liste des cues de la séquence de l'executor sélectionné : en-tête en carte,
/// cue en cours en ambre, suivante repérée, passées atténuées.
pub fn cues_window(ui: &mut egui::Ui, console: &mut Console, snapshot: &Snapshot, store_armed: &mut bool) -> Action {
    let mut action = None;
    let executor = console.selected_executor();
    let status = snapshot.executors.get(&executor);
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            if let Some(a) = executor_field(ui, console) {
                action = Some(a);
            }
            match console.selected_sequence() {
                Some((_, sequence)) => {
                    ui.label(RichText::new(&sequence.name).family(theme::medium()).size(15.0));
                    let count = sequence.cues.len();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let text = format!("{count} cue{}", if count > 1 { "s" } else { "" });
                        ui.label(RichText::new(text).size(12.0).color(theme::MUTED));
                    });
                }
                None => {
                    ui.label(RichText::new("aucune séquence").color(theme::MUTED));
                }
            }
        });
        ui.add_space(2.0);
        transition_bar(ui, status, ui.available_width());
    });
    if *store_armed {
        egui::Frame::new()
            .fill(ARMED.gamma_multiply(0.14))
            .stroke(egui::Stroke::new(1.0, ARMED))
            .corner_radius(theme::RADIUS)
            .inner_margin(egui::Margin::symmetric(10, 5))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.colored_label(Color32::from_rgb(255, 138, 132), "Store armé : cliquez un numéro pour compléter cette cue");
            });
    }

    let Some((_, sequence)) = console.selected_sequence() else {
        ui.add_space(4.0);
        ui.label("Aucune cue. Réglez des valeurs puis tapez « Store Cue », ou :");
        ui.monospace("Fixture 1 Thru 4 At 100   puis   Store Cue 1 \"Entrée\"");
        return action;
    };
    let current = status.and_then(|s| s.cue);
    let next = current.map_or(0, |c| c + 1);
    let cues: Vec<(CueNumber, Cue)> = sequence.cues.iter().map(|(n, c)| (*n, c.clone())).collect();

    ui.horizontal(|ui| {
        theme::section(ui, "Cues");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Au-dessus des champs de temps de chaque ligne (marges et touche × comprises).
            let head = |text: &str| egui::Label::new(RichText::new(text).size(11.0).color(theme::MUTED));
            ui.add_space(30.0);
            ui.add_sized([62.0, 14.0], head("délai"));
            ui.add_sized([62.0, 14.0], head("fondu"));
            ui.add_space(12.0);
            ui.label(RichText::new("Clic droit : nom et temps").size(11.0).color(theme::MUTED));
        });
    });
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        for (index, (number, cue)) in cues.iter().enumerate() {
            let state = match current {
                Some(c) if index < c => CueState::Past,
                Some(c) if index == c => CueState::Current,
                _ if index == next => CueState::Next,
                _ => CueState::Later,
            };
            if let Some(a) = cue_row(ui, console, *number, cue, state, store_armed) {
                action = Some(a);
            }
        }
    });
    action
}

/// Page affichée : ‹ PAGE [n] nom ›. Changer de page n'éteint rien.
fn page_selector(ui: &mut egui::Ui, console: &mut Console) -> Action {
    let mut action = None;
    let page = console.page();
    if ui.add_enabled(page > 1, egui::Button::new("‹")).on_hover_text("Page précédente (Page -)").clicked() {
        action = Some(Ok(console.set_page(page - 1)));
    }
    let label = ui.label(RichText::new("PAGE").size(11.0).color(theme::MUTED).extra_letter_spacing(0.8));
    let mut number = page;
    ui.add(egui::DragValue::new(&mut number).range(1..=MAX_PAGE)).labelled_by(label.id);
    if number != page {
        action = Some(Ok(console.set_page(number)));
    }
    // Nom de la page : enregistré quand on quitte le champ.
    let id = egui::Id::unique(("page-name", page));
    let mut name = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| console.page_name(page).to_string());
    let edit = egui::TextEdit::singleline(&mut name).hint_text("Nom de la page").desired_width(120.0);
    let response = ui.add(edit).on_hover_text("Nom de la page");
    if response.lost_focus() {
        if name.trim() != console.page_name(page) {
            action = Some(Ok(console.rename_page(page, name)));
        }
        ui.data_mut(|d| d.remove::<String>(id));
    } else if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, name));
    }
    if ui.add_enabled(page < MAX_PAGE, egui::Button::new("›")).on_hover_text("Page suivante (Page +)").clicked() {
        action = Some(Ok(console.set_page(page + 1)));
    }
    action
}

/// Aile de 10 executors à fader, puis une rangée de 10 executors à boutons.
/// Les cartes se partagent la largeur et le fader suit la hauteur de la fenêtre.
pub fn executors_window(
    ui: &mut egui::Ui,
    console: &mut Console,
    snapshot: &Snapshot,
    store_armed: &mut bool,
    held: &mut BTreeSet<u32>,
) -> Action {
    const GAP: f32 = 6.0;
    let mut action = None;
    let mut keep = |a: Action| {
        if a.is_some() {
            action = a;
        }
    };
    ui.horizontal(|ui| {
        crate::pools::store_button(ui, store_armed);
        if ui.button("Tout éteindre").on_hover_text("Off sur tous les executors, sauf les protégés (Off All)").clicked() {
            keep(Some(Ok(console.off_all())));
        }
        ui.separator();
        keep(page_selector(ui, console));
        let hint = if *store_armed { "Choisissez l'executor où enregistrer" } else { "Clic droit sur un numéro : mode, options, fixer, nom" };
        ui.label(RichText::new(hint).size(12.0).color(theme::MUTED));
    });
    ui.add_space(4.0);

    // Largeur : 10 cartes et leurs écarts, moins la barre de défilement ; chaque carte
    // ajoute 6 points de marge et 1 de bordure de chaque côté.
    const CARD_CHROME: f32 = 14.0;
    const SCROLLBAR: f32 = 14.0;
    let count = FADER_SLOTS.clone().count() as f32;
    let inner = ((ui.available_width() - SCROLLBAR - GAP * (count - 1.0)) / count - CARD_CHROME).clamp(78.0, 150.0);
    // Hauteur réservée hors fader : en-tête (~110), valeur et touches (~50), marges de carte,
    // puis la rangée d'executors à boutons (~190 avec son titre).
    const RESERVED: f32 = 110.0 + 50.0 + CARD_CHROME + 190.0 + 16.0;
    // Cartes étroites : les touches passent sur deux rangées, dans les deux rangées d'executors.
    let two_rows = if inner < 120.0 { 2.0 * 27.0 } else { 0.0 };
    let fader_height = (ui.available_height() - RESERVED - two_rows).clamp(70.0, 320.0);
    let fader_width = (inner * 0.42).clamp(30.0, 46.0);

    egui::ScrollArea::both().auto_shrink(false).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for slot in FADER_SLOTS {
                let executor = console.executor_at(slot);
                let status = snapshot.executors.get(&executor);
                let assigned = console.executor(executor).is_some();
                let used = assigned || console.executor_macro(executor).is_some();
                let fixed = console.executor(executor).is_some_and(|(x, _)| x.options.fixed);
                card(ui, inner, !used, fixed, |ui| {
                    keep(executor_header(ui, console, executor, status, inner));
                    if *store_armed {
                        keep(store_here(ui, console, executor, store_armed, inner));
                    }
                    if !used {
                        empty_body(ui, |ui| {
                            theme::fader(ui, &mut 1.0, theme::EXECUTOR, false, egui::vec2(fader_width, fader_height));
                            ui.label(RichText::new("100 %").monospace().size(12.0));
                            mode_buttons(ui, console, executor, held, inner);
                        });
                        return;
                    }
                    let speed = console.executor(executor).is_some_and(|(x, _)| x.options.fader == FaderFunction::Speed);
                    let mut level = status.map_or(1.0, |s| s.level);
                    let response = theme::fader(ui, &mut level, theme::EXECUTOR, assigned, egui::vec2(fader_width, fader_height))
                        .on_hover_text(format!("Fader de l'executor {executor}", executor = executor_label(executor)));
                    if response.changed() {
                        keep(Some(console.set_fader(executor, level * 100.0)));
                    }
                    let value = if speed { format!("× {:.2}", level * 2.0) } else { format!("{:.0} %", level * 100.0) };
                    ui.label(RichText::new(value).monospace().size(12.0).color(if assigned { theme::TEXT } else { theme::MUTED }));
                    keep(mode_buttons(ui, console, executor, held, inner));
                });
            }
        });
        ui.add_space(8.0);
        theme::section(ui, "Executors à boutons");
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for slot in BUTTON_SLOTS {
                let executor = console.executor_at(slot);
                let status = snapshot.executors.get(&executor);
                let assigned = console.executor(executor).is_some();
                let used = assigned || console.executor_macro(executor).is_some();
                let fixed = console.executor(executor).is_some_and(|(x, _)| x.options.fixed);
                card(ui, inner, !used, fixed, |ui| {
                    keep(executor_header(ui, console, executor, status, inner));
                    if *store_armed {
                        keep(store_here(ui, console, executor, store_armed, inner));
                    } else if !used {
                        empty_body(ui, |ui| {
                            mode_buttons(ui, console, executor, held, inner);
                        });
                    } else {
                        keep(mode_buttons(ui, console, executor, held, inner));
                    }
                });
            }
        });
    });
    action
}

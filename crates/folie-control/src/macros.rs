//! Macros : des lignes de commande jouées l'une après l'autre. `Wait 2` y attend
//! 2 secondes sans bloquer la console : le lecteur avance à chaque image (`tick_macros`).

use std::time::{Duration, Instant};

use folie_show::Macro;

use crate::command::{self, Command};
use crate::{CommandError, Console, Outcome, invalid};

/// Au-delà, une macro qui en lance d'autres (ou elle-même) est arrêtée.
const MAX_RUNS: usize = 16;

/// Une macro en cours de lecture.
#[derive(Debug, Clone)]
pub(crate) struct MacroRun {
    number: u32,
    lines: Vec<String>,
    /// Prochaine ligne à jouer.
    next: usize,
    /// En attente (`Wait`) jusqu'à cet instant.
    until: Option<Instant>,
}

/// Ce que le pool et les executors montrent d'une macro en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MacroProgress {
    pub number: u32,
    /// Ligne en cours (à partir de 1) et nombre de lignes.
    pub line: usize,
    pub lines: usize,
    pub waiting: bool,
}

/// Une ligne de macro est-elle lisible ? (`Wait` compris.) Pour l'éditeur.
pub fn check_line(line: &str) -> Result<(), String> {
    if line.trim().is_empty() {
        return Ok(());
    }
    command::parse(line).map(|_| ()).map_err(|e| e.0)
}

impl Console {
    pub fn macros_in_progress(&self) -> Vec<MacroProgress> {
        self.macro_runs
            .iter()
            .map(|run| MacroProgress {
                number: run.number,
                line: run.next.min(run.lines.len()),
                lines: run.lines.len(),
                waiting: run.until.is_some(),
            })
            .collect()
    }

    /// `Store Macro 3 "Nom"` : crée la macro (vide), ou la renomme si elle existe.
    pub fn store_macro(&mut self, number: u32, name: Option<String>) -> Outcome {
        let existing = self.show.macros.contains_key(&number);
        let entry = self.show.macros.entry(number).or_insert_with(|| Macro { name: format!("Macro {number}"), lines: Vec::new() });
        if let Some(name) = name {
            entry.name = name;
        }
        let verb = if existing { "renommée" } else { "créée : écrivez ses lignes (clic droit, Modifier)" };
        Outcome::changed(format!("Macro {number} « {} » {verb}", entry.name))
    }

    /// Lignes de la macro, depuis l'éditeur. Les lignes vides sont retirées.
    pub fn set_macro_lines(&mut self, number: u32, lines: Vec<String>) -> Result<Outcome, CommandError> {
        let Some(entry) = self.show.macros.get_mut(&number) else { return invalid(format!("la macro {number} n'existe pas")) };
        entry.lines = lines.into_iter().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        let count = entry.lines.len();
        Ok(Outcome::changed(format!("Macro {number} : {count} ligne(s)")))
    }

    pub fn label_macro(&mut self, number: u32, name: String) -> Result<Outcome, CommandError> {
        let Some(entry) = self.show.macros.get_mut(&number) else { return invalid(format!("la macro {number} n'existe pas")) };
        entry.name = name.clone();
        Ok(Outcome::changed(format!("Macro {number} renommée « {name} »")))
    }

    /// Supprime la macro, l'arrête si elle joue, et libère les executors qui la lançaient.
    pub fn delete_macro(&mut self, number: u32) -> Result<Outcome, CommandError> {
        let Some(removed) = self.show.macros.remove(&number) else { return invalid(format!("la macro {number} n'existe pas")) };
        self.macro_runs.retain(|r| r.number != number);
        self.show.macro_executors.retain(|_, m| *m != number);
        Ok(Outcome::changed(format!("Macro « {} » supprimée", removed.name)))
    }

    /// Lance une macro : ses premières lignes partent tout de suite, jusqu'au premier `Wait`.
    pub fn run_macro(&mut self, number: u32) -> Result<Outcome, CommandError> {
        let Some(entry) = self.show.macros.get(&number) else { return invalid(format!("la macro {number} n'existe pas")) };
        if self.macro_runs.len() >= MAX_RUNS {
            return invalid("trop de macros en cours (une macro se lance-t-elle elle-même ?)");
        }
        let name = entry.name.clone();
        self.macro_runs.push(MacroRun { number, lines: entry.lines.clone(), next: 0, until: None });
        // Lancée par une ligne d'une autre macro : elle partira à l'image suivante.
        if self.macro_ticking {
            return Ok(Outcome::info(format!("Macro {number} « {name} » lancée")));
        }
        let results = self.tick_macros(Instant::now());
        // Une erreur dès les premières lignes est rendue tout de suite.
        if let Some(Err(err)) = results.into_iter().find(Result::is_err) {
            return Err(err);
        }
        Ok(Outcome::info(format!("Macro {number} « {name} » lancée")))
    }

    /// Fait avancer les macros en cours : à appeler à chaque image. Renvoie le résultat de
    /// chaque ligne jouée ; une ligne en erreur arrête sa macro.
    pub fn tick_macros(&mut self, now: Instant) -> Vec<Result<Outcome, CommandError>> {
        let mut results = Vec::new();
        let mut runs = std::mem::take(&mut self.macro_runs);
        self.macro_ticking = true;
        for run in &mut runs {
            loop {
                if let Some(until) = run.until {
                    if now < until {
                        break;
                    }
                    run.until = None;
                }
                let Some(line) = run.lines.get(run.next).cloned() else { break };
                run.next += 1;
                match command::parse(&line) {
                    Ok(Command::Wait(seconds)) => run.until = Some(now + Duration::from_secs_f32(seconds)),
                    Ok(Command::Macro(n)) if n == run.number => {
                        results.push(invalid(format!("Macro {n}, ligne {} : une macro ne peut pas se lancer elle-même", run.next)));
                        run.next = run.lines.len();
                        break;
                    }
                    Ok(_) => match self.execute(&line) {
                        Ok(outcome) => results.push(Ok(outcome)),
                        Err(err) => {
                            results.push(invalid(format!("Macro {}, ligne {} : {err}", run.number, run.next)));
                            run.next = run.lines.len();
                            break;
                        }
                    },
                    Err(err) => {
                        results.push(invalid(format!("Macro {}, ligne {} : {err}", run.number, run.next)));
                        run.next = run.lines.len();
                        break;
                    }
                }
            }
        }
        // Les macros lancées par une ligne (`Macro 4`) sont déjà dans `self.macro_runs`.
        runs.retain(|r| r.next < r.lines.len() || r.until.is_some());
        runs.append(&mut self.macro_runs);
        self.macro_runs = runs;
        self.macro_ticking = false;
        results
    }

    /// Arrête une macro en cours (un clic sur sa case pendant qu'elle joue).
    pub fn stop_macro(&mut self, number: u32) -> Outcome {
        self.macro_runs.retain(|r| r.number != number);
        Outcome::info(format!("Macro {number} arrêtée"))
    }

    /// L'executor lance cette macro (ou plus rien avec `None`) ; il ne joue plus de séquence.
    pub fn set_executor_macro(&mut self, executor: u32, number: Option<u32>) -> Result<Outcome, CommandError> {
        let label = folie_show::executor_label(executor);
        match number {
            Some(n) => {
                let Some(name) = self.show.macros.get(&n).map(|m| m.name.clone()) else {
                    return invalid(format!("la macro {n} n'existe pas"));
                };
                if self.show.executors.contains_key(&executor) {
                    self.clear_executor(executor)?;
                }
                self.show.macro_executors.insert(executor, n);
                Ok(Outcome::changed(format!("Executor {label} : lance la macro {n} « {name} »")))
            }
            None => {
                self.show.macro_executors.remove(&executor);
                Ok(Outcome::changed(format!("Executor {label} : plus de macro")))
            }
        }
    }

    /// Macro qu'un executor lance, s'il en lance une.
    pub fn executor_macro(&self, executor: u32) -> Option<(u32, &Macro)> {
        let number = *self.show.macro_executors.get(&executor)?;
        Some((number, self.show.macros.get(&number)?))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::tests::{console, run};

    #[test]
    fn a_macro_runs_its_lines_and_waits() {
        let mut c = console();
        assert!(run(&mut c, r#"Store Macro 3 "Ballet""#).show_changed);
        let lines = ["Fixture 1 Thru 2", "At 100", "Wait 0.2", "Fixture 3", "At 50"];
        c.set_macro_lines(3, lines.iter().map(|l| l.to_string()).chain(["".to_string()]).collect()).unwrap();
        assert_eq!(c.show.macros[&3].lines.len(), 5, "ligne vide retirée");

        assert_eq!(run(&mut c, "Macro 3").message, "Macro 3 « Ballet » lancée");
        assert_eq!(c.selection().len(), 2, "les deux premières lignes sont parties");
        let progress = c.macros_in_progress();
        assert_eq!((progress[0].line, progress[0].waiting), (3, true), "en attente sur la ligne 3");

        let later = Instant::now() + Duration::from_millis(300);
        let results = c.tick_macros(later);
        assert_eq!(results.len(), 2);
        assert_eq!(c.selection().len(), 1, "la suite est partie après l'attente");
        assert!(c.macros_in_progress().is_empty(), "macro finie");
    }

    #[test]
    fn errors_stop_a_macro_and_executors_launch_macros() {
        let mut c = console();
        run(&mut c, "Store Macro 1");
        c.set_macro_lines(1, vec!["Fixture 1".into(), "Go Executor 9".into(), "Fixture 2".into()]).unwrap();
        let err = c.run_macro(1).unwrap_err().to_string();
        assert_eq!(err, "Macro 1, ligne 2 : l'executor 9 n'a pas de séquence");
        assert!(c.macros_in_progress().is_empty(), "arrêtée à l'erreur");
        assert_eq!(check_line("Go Executor 21").unwrap_err(), "executor invalide : 21 (place 1 à 20, ou page.place)");
        assert!(check_line("Wait 2").is_ok());
        assert!(c.execute("Wait 2").is_err(), "Wait ne s'utilise que dans une macro");

        // Une macro qui se lance elle-même s'arrête au lieu de tourner sans fin.
        run(&mut c, "Store Macro 2");
        c.set_macro_lines(2, vec!["Macro 2".into()]).unwrap();
        assert_eq!(c.run_macro(2).unwrap_err().to_string(), "Macro 2, ligne 1 : une macro ne peut pas se lancer elle-même");
        // Une macro qui en lance une autre : celle-ci part à l'image suivante.
        run(&mut c, "Store Macro 4");
        c.set_macro_lines(4, vec!["Fixture 3".into()]).unwrap();
        c.set_macro_lines(2, vec!["Macro 4".into(), "Wait 5".into()]).unwrap();
        c.run_macro(2).unwrap();
        c.tick_macros(Instant::now());
        assert!(c.selection().iter().any(|t| t.fixture.0 == 3), "la macro 4 a joué");
        assert_eq!(c.stop_macro(2).message, "Macro 2 arrêtée");
        assert!(c.macros_in_progress().is_empty());

        // Executor 7 : il lance la macro 1 au lieu de jouer une séquence.
        c.set_macro_lines(1, vec!["Fixture 4".into()]).unwrap();
        c.set_executor_macro(7, Some(1)).unwrap();
        assert_eq!(c.executor_macro(7).map(|(n, _)| n), Some(1));
        run(&mut c, "ClearAll");
        c.press(7).unwrap();
        assert_eq!(c.selection(), [crate::command::fixture(4)], "la touche de l'executor a lancé la macro");
        c.delete_macro(1).unwrap();
        assert!(c.executor_macro(7).is_none(), "macro supprimée : l'executor est libéré");
    }
}

//! Complétion par Tab de la ligne de commande, comme sur grandMA.
//!
//! ```text
//! fi⇥          Fixture ␣
//! c⇥           C            (Clear, ClearAll, Cue ; Tab encore pour les faire défiler)
//! Group ⇥      1 Cour, 2 Jardin…
//! Preset 4.⇥   4.1 Rouge, 4.2 Bleu…
//! ```

use folie_fixture::FeatureGroup;
use folie_show::SLOTS_PER_PAGE;

use crate::command::{ALIASES, KEYWORDS, Keyword};
use crate::Console;

/// Une possibilité proposée par Tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Texte qui remplace le mot en cours : « Fixture␣ », « 3␣ », « 4. ».
    pub text: String,
    /// Ce qu'affiche la liste : « Cue », « 3 Cour ».
    pub label: String,
}

/// Ce que Tab propose pour une ligne.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Completion {
    /// Début du mot en cours, en octets : Tab remplace `line[start..]`.
    pub start: usize,
    /// Préfixe commun à toutes les possibilités ; s'il n'y en a qu'une, elle entière.
    pub common: String,
    pub candidates: Vec<Candidate>,
}

impl Completion {
    /// La ligne dont le mot en cours est remplacé par `text`.
    pub fn apply(&self, line: &str, text: &str) -> String {
        format!("{}{text}", line.get(..self.start).unwrap_or(line))
    }
}

impl Console {
    /// Possibilités pour compléter la fin de `line`, avec les numéros du show.
    pub fn complete(&self, line: &str) -> Completion {
        let start = word_start(line);
        // Dans un texte entre guillemets, on écrit librement.
        if line.matches('"').count() % 2 == 1 {
            return Completion { start: line.len(), ..Default::default() };
        }
        let word = &line[start..];
        let candidates = if word.starts_with(char::is_alphabetic) {
            keywords(word)
        } else {
            object_before(&line[..start]).map_or_else(Vec::new, |object| self.numbers(object, &word.replace(',', ".")))
        };
        let common = match candidates.as_slice() {
            [] => String::new(),
            [only] => only.text.clone(),
            [first, rest @ ..] => {
                let mut common = first.text.trim_end().to_string();
                for other in rest {
                    let shared = common.chars().zip(other.text.chars()).take_while(|(a, b)| a == b).count();
                    common = common.chars().take(shared).collect();
                }
                common
            }
        };
        Completion { start, common, candidates }
    }

    /// Numéros existants pour un objet, avec leur nom.
    fn numbers(&self, object: Keyword, prefix: &str) -> Vec<Candidate> {
        let show = &self.show;
        let mut found: Vec<(String, String)> = match object {
            Keyword::Fixture => show.patch.fixtures().iter().map(|f| (f.id.0.to_string(), f.name.clone())).collect(),
            Keyword::Group => show.groups.iter().map(|(n, g)| (n.to_string(), g.name.clone())).collect(),
            Keyword::Macro => show.macros.iter().map(|(n, m)| (n.to_string(), m.name.clone())).collect(),
            Keyword::Effect => {
                folie_show::effects::templates().into_iter().enumerate().map(|(i, e)| ((i + 1).to_string(), e.name)).collect()
            }
            Keyword::Cue | Keyword::Goto => match self.selected_sequence() {
                Some((_, sequence)) => {
                    sequence.cues.iter().map(|(n, cue)| (n.to_string(), cue.name.clone())).collect()
                }
                None => Vec::new(),
            },
            Keyword::Executor => (1..=SLOTS_PER_PAGE)
                .filter_map(|slot| {
                    let id = self.executor_at(slot);
                    let name = match show.macro_executors.get(&id) {
                        Some(m) => show.macros.get(m)?.name.clone(),
                        None => show.sequences.get(&show.executors.get(&id)?.sequence)?.name.clone(),
                    };
                    Some((slot.to_string(), name))
                })
                .collect(),
            // D'abord la famille (`4.`), puis les presets de cette famille.
            Keyword::Preset => match prefix.split_once('.') {
                Some((family, _)) => {
                    let family = family.parse().ok().and_then(FeatureGroup::from_preset_number);
                    show.presets
                        .iter()
                        .filter(|((f, _), _)| Some(*f) == family)
                        .map(|((f, n), p)| (format!("{}.{n}", f.preset_number()), p.name.clone()))
                        .collect()
                }
                None => {
                    let mut families: Vec<FeatureGroup> = show.presets.keys().map(|(f, _)| *f).collect();
                    families.dedup();
                    families.sort_by_key(|f| f.preset_number());
                    return families
                        .into_iter()
                        .map(|f| (f.preset_number().to_string(), f.label()))
                        .filter(|(n, _)| n.starts_with(prefix))
                        .map(|(n, label)| Candidate { text: format!("{n}."), label: format!("{n} {label}") })
                        .collect();
                }
            },
            _ => Vec::new(),
        };
        found.retain(|(n, _)| n.starts_with(prefix));
        found
            .into_iter()
            .map(|(n, name)| Candidate { text: format!("{n} "), label: format!("{n} {name}").trim_end().to_string() })
            .collect()
    }
}

/// Mots-clés commençant par `word`, ou celui de l'alias tapé.
fn keywords(word: &str) -> Vec<Candidate> {
    let word = word.to_lowercase();
    let candidate = |k: Keyword| Candidate { text: format!("{} ", k.name()), label: k.name().to_string() };
    if let Some((_, k)) = ALIASES.iter().find(|(alias, _)| *alias == word) {
        return vec![candidate(*k)];
    }
    let mut found: Vec<Keyword> = KEYWORDS.iter().filter(|(name, _)| name.starts_with(&word)).map(|(_, k)| *k).collect();
    found.sort_by_key(|k| k.name());
    found.into_iter().map(candidate).collect()
}

/// Début du mot en cours, à la fin de la ligne.
fn word_start(line: &str) -> usize {
    line.char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '.' || *c == ',')
        .last()
        .map_or(line.len(), |(i, _)| i)
}

/// Objet dont on tape le numéro : `Group ⇥`, et aussi `Group 1 + ⇥`.
fn object_before(before: &str) -> Option<Keyword> {
    let mut pieces = before
        .split(|c: char| c.is_whitespace() || c == '+' || c == '-')
        .filter(|w| !w.is_empty())
        .rev();
    let after_sign = before.trim_end().ends_with(['+', '-']);
    let mut last = pieces.next()?;
    if after_sign && last.starts_with(|c: char| c.is_ascii_digit()) {
        last = pieces.next()?;
    }
    if !last.chars().all(char::is_alphabetic) {
        return None;
    }
    Keyword::parse(last).ok()
}

#[cfg(test)]
mod tests {
    use folie_show::{Cue, CueNumber, Executor, Group, Macro, Preset, Sequence};

    use super::*;
    use crate::tests::console;

    fn labels(completion: &Completion) -> Vec<&str> {
        completion.candidates.iter().map(|c| c.label.as_str()).collect()
    }

    #[test]
    fn unique_keyword_is_completed() {
        let console = console();
        let completion = console.complete("fi");
        assert_eq!(completion.apply("fi", &completion.common), "Fixture ", "« fi » est l'alias de Fixture");
        assert_eq!(console.complete("Fixture 1 th").common, "Thru ");
        assert_eq!(console.complete("Fixture 1 Thru 4 At FU").common, "Full ");
        assert_eq!(labels(&console.complete("Fixture 1 at")), ["At", "Attribute"]);
        let completion = console.complete("Store gr");
        assert_eq!((completion.start, completion.apply("Store gr", &completion.common).as_str()), (6, "Store Group "));
    }

    #[test]
    fn ambiguous_keywords_share_their_prefix() {
        let console = console();
        let completion = console.complete("c");
        assert_eq!(labels(&completion), ["Clear", "ClearAll", "Cue"]);
        assert_eq!(completion.common, "C");
        let completion = console.complete("cle");
        assert_eq!(completion.common, "Clear");
        assert_eq!(completion.apply("cle", &completion.candidates[1].text), "ClearAll ");
        assert_eq!(labels(&console.complete("go")), ["Go", "Goto"]);
        assert!(console.complete("xyz").candidates.is_empty());
    }

    #[test]
    fn object_numbers_with_their_names() {
        let mut console = console();
        console.show.groups.insert(1, Group { name: "Cour".into(), targets: Vec::new() });
        console.show.groups.insert(12, Group { name: "Jardin".into(), targets: Vec::new() });
        let completion = console.complete("Group ");
        assert_eq!(labels(&completion), ["1 Cour", "12 Jardin"]);
        assert_eq!(completion.common, "1");
        assert_eq!(console.complete("g 12").common, "12 ");
        assert_eq!(labels(&console.complete("Group 1 + ")), ["1 Cour", "12 Jardin"]);
        assert!(console.complete("Group 1 ").candidates.is_empty(), "le numéro est déjà là");
        assert!(console.complete("Fixture 1 Thru ").candidates.is_empty());

        assert_eq!(labels(&console.complete("Fixture ")), ["1 F1", "2 F2", "3 F3", "4 F4", "10 F10"]);

        console.show.macros.insert(2, Macro { name: "Ballet".into(), lines: Vec::new() });
        assert_eq!(console.complete("Macro ").common, "2 ");
        assert_eq!(console.complete("Effect ").candidates.len(), folie_show::effects::templates().len());
    }

    #[test]
    fn presets_by_family() {
        let mut console = console();
        let preset = |name: &str| Preset { name: name.into(), ..Default::default() };
        console.show.presets.insert((FeatureGroup::Color, 1), preset("Rouge"));
        console.show.presets.insert((FeatureGroup::Color, 2), preset("Bleu"));
        console.show.presets.insert((FeatureGroup::Position, 1), preset("Face"));

        assert_eq!(labels(&console.complete("Preset ")), ["2 Position", "4 Couleur"]);
        assert_eq!(console.complete("Preset 4").common, "4.", "la famille, sans espace");
        let completion = console.complete("Preset 4.");
        assert_eq!(labels(&completion), ["4.1 Rouge", "4.2 Bleu"]);
        assert_eq!(completion.apply("Preset 4.", &completion.common), "Preset 4.");
        assert_eq!(console.complete("Delete Preset 4,2").common, "4.2 ");
    }

    #[test]
    fn cues_and_executors_of_the_show() {
        let mut console = console();
        assert!(console.complete("Cue ").candidates.is_empty(), "pas encore de séquence");
        let mut sequence = Sequence { name: "Acte 1".into(), ..Default::default() };
        sequence.cues.insert(CueNumber::whole(1), Cue::new("Entrée".into()));
        sequence.cues.insert("1.5".parse().unwrap(), Cue::new("Lent".into()));
        console.show.sequences.insert(1, sequence);
        console.show.executors.insert(1, Executor::new(1));

        assert_eq!(labels(&console.complete("Store Cue ")), ["1 Entrée", "1.5 Lent"]);
        assert_eq!(labels(&console.complete("Goto ")), ["1 Entrée", "1.5 Lent"]);
        assert_eq!(labels(&console.complete("Go Executor ")), ["1 Acte 1"]);
    }

    #[test]
    fn quoted_text_is_left_alone() {
        let console = console();
        assert!(console.complete("Store Cue 3 \"Ent").candidates.is_empty());
        assert_eq!(console.complete("Label Group 3 \"Cour\" ").candidates, []);
    }
}

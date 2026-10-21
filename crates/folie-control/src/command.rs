//! Langage de commande, sous-ensemble de la syntaxe grandMA2.
//!
//! ```text
//! Fixture 1 Thru 4 At 50        Group 2 At Full        Fixture 5.3
//! Fixture 1 + 3 - 2             Preset 4.1             Store Group 3 "Cour"
//! Store Preset 4.1 "Rouge"      Label Group 3 "Jardin" Delete Preset 4.1
//! Clear                         ClearAll
//! Go   Back   Off               Go Executor 2           Goto Cue 5
//! Store Cue 3 Fade 4 "Entrée"   Delete Cue 3            Executor 2 At 50
//! Effect 2                      Tempo 128
//! Page 2   Page +   Page -       Go Executor 2.5         Fix Executor 3
//! Attribute "Pan" At 50          Fixture 1 At + 10       Block Cue 3
//! Macro 2                        Store Macro 3 "Ballet"  Wait 2 (dans une macro)
//! Fixture 2 Release             Assert Executor 3
//! ```
//!
//! Les mots-clés sont insensibles à la casse et s'abrègent tant qu'il n'y a
//! pas d'ambiguïté : `fi 1 th 4 at 50`.

use folie_fixture::{FeatureGroup, FixtureId, Target};
use folie_show::{CueNumber, MAX_PAGE, SLOTS_PER_PAGE};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct SyntaxError(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    Fixture,
    Group,
    Preset,
    Thru,
    At,
    Full,
    Clear,
    ClearAll,
    Store,
    Label,
    Delete,
    Cue,
    Executor,
    Go,
    Back,
    Goto,
    Off,
    Fade,
    Effect,
    Tempo,
    All,
    Page,
    Fix,
    Assert,
    Release,
    Block,
    Unblock,
    Attribute,
    Macro,
    Wait,
}

/// Abréviations d'usage qui l'emportent sur la règle du préfixe unique.
/// « fi », « p » et « b » restent Fixture, Preset et Back malgré Fix, Page et Block.
const ALIASES: [(&str, Keyword); 7] = [
    ("g", Keyword::Group),
    ("exec", Keyword::Executor),
    ("t", Keyword::Thru),
    ("eff", Keyword::Effect),
    ("fi", Keyword::Fixture),
    ("p", Keyword::Preset),
    ("b", Keyword::Back),
];

const KEYWORDS: [(&str, Keyword); 30] = [
    ("fixture", Keyword::Fixture),
    ("group", Keyword::Group),
    ("preset", Keyword::Preset),
    ("thru", Keyword::Thru),
    ("at", Keyword::At),
    ("full", Keyword::Full),
    ("clear", Keyword::Clear),
    ("clearall", Keyword::ClearAll),
    ("store", Keyword::Store),
    ("label", Keyword::Label),
    ("delete", Keyword::Delete),
    ("cue", Keyword::Cue),
    ("executor", Keyword::Executor),
    ("go", Keyword::Go),
    ("back", Keyword::Back),
    ("goto", Keyword::Goto),
    ("off", Keyword::Off),
    ("fade", Keyword::Fade),
    ("effect", Keyword::Effect),
    ("tempo", Keyword::Tempo),
    ("all", Keyword::All),
    ("page", Keyword::Page),
    ("fix", Keyword::Fix),
    ("assert", Keyword::Assert),
    ("release", Keyword::Release),
    ("block", Keyword::Block),
    ("unblock", Keyword::Unblock),
    ("attribute", Keyword::Attribute),
    ("macro", Keyword::Macro),
    ("wait", Keyword::Wait),
];

impl Keyword {
    pub fn name(self) -> &'static str {
        match self {
            Self::Fixture => "Fixture",
            Self::Group => "Group",
            Self::Preset => "Preset",
            Self::Thru => "Thru",
            Self::At => "At",
            Self::Full => "Full",
            Self::Clear => "Clear",
            Self::ClearAll => "ClearAll",
            Self::Store => "Store",
            Self::Label => "Label",
            Self::Delete => "Delete",
            Self::Cue => "Cue",
            Self::Executor => "Executor",
            Self::Go => "Go",
            Self::Back => "Back",
            Self::Goto => "Goto",
            Self::Off => "Off",
            Self::Fade => "Fade",
            Self::Effect => "Effect",
            Self::Tempo => "Tempo",
            Self::All => "All",
            Self::Page => "Page",
            Self::Fix => "Fix",
            Self::Assert => "Assert",
            Self::Release => "Release",
            Self::Block => "Block",
            Self::Unblock => "Unblock",
            Self::Attribute => "Attribute",
            Self::Macro => "Macro",
            Self::Wait => "Wait",
        }
    }

    /// Mot exact, sinon préfixe sans ambiguïté.
    fn parse(word: &str) -> Result<Self, SyntaxError> {
        let word = word.to_lowercase();
        if let Some((_, k)) = KEYWORDS.iter().chain(&ALIASES).find(|(name, _)| *name == word) {
            return Ok(*k);
        }
        let candidates: Vec<Keyword> =
            KEYWORDS.iter().filter(|(name, _)| name.starts_with(&word)).map(|(_, k)| *k).collect();
        match candidates.as_slice() {
            [k] => Ok(*k),
            [] => Err(SyntaxError(format!("mot inconnu : « {word} »"))),
            many => {
                let names: Vec<&str> = many.iter().map(|k| k.name()).collect();
                Err(SyntaxError(format!("« {word} » est ambigu : {}", names.join(", "))))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Keyword(Keyword),
    /// Texte du nombre tel quel : "5", "5.3", "4.1" ou "50.5" selon le contexte.
    Number(String),
    Plus,
    Minus,
    Text(String),
}

fn tokenize(line: &str) -> Result<Vec<Token>, SyntaxError> {
    let mut tokens = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(&c) = chars.peek() {
        match c {
            c if c.is_whitespace() => {
                chars.next();
            }
            '+' => {
                chars.next();
                tokens.push(Token::Plus);
            }
            '-' => {
                chars.next();
                tokens.push(Token::Minus);
            }
            '"' => {
                chars.next();
                let text: String = chars.by_ref().take_while(|c| *c != '"').collect();
                tokens.push(Token::Text(text));
            }
            c if c.is_ascii_digit() => {
                let mut number = String::new();
                while let Some(&c) = chars.peek().filter(|c| c.is_ascii_digit() || **c == '.' || **c == ',') {
                    number.push(if c == ',' { '.' } else { c });
                    chars.next();
                }
                tokens.push(Token::Number(number));
            }
            c if c.is_alphabetic() => {
                let mut word = String::new();
                while let Some(&c) = chars.peek().filter(|c| c.is_alphanumeric()) {
                    word.push(c);
                    chars.next();
                }
                tokens.push(Token::Keyword(Keyword::parse(&word)?));
            }
            other => return Err(SyntaxError(format!("caractère inattendu : « {other} »"))),
        }
    }
    Ok(tokens)
}

/// Élément d'une sélection : une cible, une plage, ou le contenu d'un groupe.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Target(Target),
    Range(Target, Target),
    Group(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// (ajout ?, élément) : `false` pour un élément précédé de `-`.
    pub items: Vec<(bool, Item)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// Pourcentage 0 à 100.
    Percent(f32),
    /// `At + 10`, `At - 10` : écart en pourcentage, ajouté à la valeur en cours.
    Relative(f32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Group(u32),
    Preset(FeatureGroup, u32),
    /// Cue de la séquence de l'executor sélectionné ; sans numéro pour `Store Cue`.
    Cue(Option<CueNumber>),
    Macro(u32),
}

/// Executor désigné en ligne de commande : `3` (page courante) ou `2.3` (page 2, place 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutorRef {
    /// `None` : la page courante, où un executor fixé l'emporte.
    pub page: Option<u32>,
    pub slot: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageMove {
    To(u32),
    Next,
    Previous,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Sélection éventuellement suivie d'une valeur de dimmer : `Fixture 1 Thru 4 At 50`.
    /// Avec `Attribute "Pan"`, la valeur vise ce paramètre au lieu du dimmer.
    Select { selection: Option<Selection>, attribute: Option<String>, value: Option<Value> },
    ApplyPreset(FeatureGroup, u32),
    Store { object: Object, name: Option<String>, fade: Option<f32> },
    Label { object: Object, name: String },
    Delete(Object),
    Clear,
    ClearAll,
    /// `None` : l'executor sélectionné.
    Go(Option<ExecutorRef>),
    Back(Option<ExecutorRef>),
    Off(Option<ExecutorRef>),
    Goto(CueNumber),
    SelectExecutor(ExecutorRef),
    /// Fader d'un executor, en pourcentage.
    SetFader(ExecutorRef, f32),
    /// Lance un effet modèle sur la sélection.
    ApplyEffect(u32),
    SetTempo(f32),
    /// `Store Executor 3` : le programmer devient la cue suivante de l'executor 3.
    StoreExecutor(ExecutorRef),
    /// `Off All` : tout éteindre, sauf les executors protégés.
    OffAll,
    /// `Page 2`, `Page +`, `Page -` : page d'executors affichée.
    Page(PageMove),
    /// `Fix Executor 3` : fixe l'executor sur toutes les pages, ou le défixe.
    Fix(ExecutorRef),
    /// `Fixture 2 Release` ou `Release` : marque la sélection comme relâchée dans le programmer.
    Release(Option<Selection>),
    /// `Assert Executor 3` : l'executor reprend la main sur ce qu'il tient.
    Assert(Option<ExecutorRef>),
    /// `Block Cue 3` : écrit en dur dans la cue les valeurs qu'elle suit.
    Block(CueNumber),
    /// `Unblock Cue 3` : retire de la cue les valeurs identiques à celles qu'elle suivrait.
    Unblock(CueNumber),
    /// `Macro 2` : lance la macro.
    Macro(u32),
    /// `Wait 2` : pause de 2 secondes, dans une macro.
    Wait(f32),
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn eat(&mut self, keyword: Keyword) -> bool {
        if self.peek() == Some(&Token::Keyword(keyword)) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn number(&mut self, what: &str) -> Result<String, SyntaxError> {
        match self.next() {
            Some(Token::Number(n)) => Ok(n),
            _ => Err(SyntaxError(format!("{what} attendu"))),
        }
    }

    fn integer(&mut self, what: &str) -> Result<u32, SyntaxError> {
        let text = self.number(what)?;
        text.parse().map_err(|_| SyntaxError(format!("{what} invalide : {text}")))
    }

    fn target(&mut self) -> Result<Target, SyntaxError> {
        let text = self.number("numéro de fixture")?;
        text.parse().map_err(SyntaxError)
    }

    fn preset_ref(&mut self) -> Result<(FeatureGroup, u32), SyntaxError> {
        let text = self.number("numéro de preset (ex. 4.1)")?;
        let (family, number) = text
            .split_once('.')
            .and_then(|(f, n)| Some((f.parse().ok()?, n.parse().ok()?)))
            .ok_or_else(|| SyntaxError(format!("preset « {text} » : écrire famille.numéro, ex. 4.1")))?;
        let family = FeatureGroup::from_preset_number(family)
            .ok_or_else(|| SyntaxError(format!("famille de preset inconnue : {family} (1 à 7)")))?;
        Ok((family, number))
    }

    fn cue_number(&mut self) -> Result<CueNumber, SyntaxError> {
        self.number("numéro de cue")?.parse().map_err(SyntaxError)
    }

    fn optional_cue_number(&mut self) -> Result<Option<CueNumber>, SyntaxError> {
        match self.peek() {
            Some(Token::Number(_)) => Ok(Some(self.cue_number()?)),
            _ => Ok(None),
        }
    }

    /// `3` ou `2.3` (page 2, place 3).
    fn executor_ref(&mut self) -> Result<ExecutorRef, SyntaxError> {
        let text = self.number("numéro d'executor")?;
        let invalid = || SyntaxError(format!("executor invalide : {text} (place 1 à {SLOTS_PER_PAGE}, ou page.place)"));
        let (page, slot) = match text.split_once('.') {
            Some((page, slot)) => (Some(page.parse::<u32>().map_err(|_| invalid())?), slot),
            None => (None, text.as_str()),
        };
        let slot: u32 = slot.parse().map_err(|_| invalid())?;
        if !(1..=SLOTS_PER_PAGE).contains(&slot) {
            return Err(invalid());
        }
        if let Some(page) = page.filter(|p| !(1..=MAX_PAGE).contains(p)) {
            return Err(SyntaxError(format!("page invalide : {page} (1 à {MAX_PAGE})")));
        }
        Ok(ExecutorRef { page, slot })
    }

    /// `Executor 2` facultatif après Go, Back, Off.
    fn optional_executor(&mut self) -> Result<Option<ExecutorRef>, SyntaxError> {
        if self.eat(Keyword::Executor) { Ok(Some(self.executor_ref()?)) } else { Ok(None) }
    }

    fn seconds(&mut self) -> Result<f32, SyntaxError> {
        let text = self.number("durée en secondes")?;
        text.parse::<f32>()
            .ok()
            .filter(|s| (0.0..=3600.0).contains(s))
            .ok_or_else(|| SyntaxError(format!("durée invalide : {text} (0 à 3600 s)")))
    }

    fn object(&mut self) -> Result<Object, SyntaxError> {
        if self.eat(Keyword::Cue) {
            return Ok(Object::Cue(self.optional_cue_number()?));
        }
        if self.eat(Keyword::Macro) {
            return Ok(Object::Macro(self.integer("numéro de macro")?));
        }
        if self.eat(Keyword::Group) {
            Ok(Object::Group(self.integer("numéro de groupe")?))
        } else if self.eat(Keyword::Preset) {
            let (family, number) = self.preset_ref()?;
            Ok(Object::Preset(family, number))
        } else {
            Err(SyntaxError("Group, Preset, Cue ou Macro attendu".into()))
        }
    }

    fn text(&mut self) -> Option<String> {
        match self.peek() {
            Some(Token::Text(_)) => match self.next() {
                Some(Token::Text(t)) => Some(t),
                _ => None,
            },
            _ => None,
        }
    }

    /// `Fixture 1 Thru 4 + 6 - 2 + Group 3`
    fn selection(&mut self) -> Result<Option<Selection>, SyntaxError> {
        let mut items = Vec::new();
        // Type d'objet en cours : après `+`, un nombre seul garde le même type.
        let mut kind: Option<Keyword> = None;
        let mut add = true;
        loop {
            if let Some(Token::Keyword(k @ (Keyword::Fixture | Keyword::Group))) = self.peek() {
                kind = Some(*k);
                self.position += 1;
            }
            let item = match kind {
                Some(Keyword::Fixture) => {
                    let first = self.target()?;
                    if self.eat(Keyword::Thru) {
                        Item::Range(first, self.target()?)
                    } else {
                        Item::Target(first)
                    }
                }
                Some(Keyword::Group) => Item::Group(self.integer("numéro de groupe")?),
                _ => break,
            };
            items.push((add, item));
            match self.peek() {
                Some(Token::Plus) => add = true,
                Some(Token::Minus) => add = false,
                Some(Token::Number(_)) => {
                    return Err(SyntaxError("utiliser + ou - entre deux numéros".into()));
                }
                _ => break,
            }
            self.position += 1;
        }
        Ok((!items.is_empty()).then_some(Selection { items }))
    }

    fn value(&mut self) -> Result<Value, SyntaxError> {
        if self.eat(Keyword::Full) {
            return Ok(Value::Percent(100.0));
        }
        // `At + 10` / `At - 10` : relatif à la valeur en cours.
        if let Some(sign) = match self.peek() {
            Some(Token::Plus) => Some(1.0),
            Some(Token::Minus) => Some(-1.0),
            _ => None,
        } {
            self.position += 1;
            let text = self.number("écart (0 à 100)")?;
            let delta: f32 = text.parse().map_err(|_| SyntaxError(format!("écart invalide : {text}")))?;
            if !(0.0..=100.0).contains(&delta) {
                return Err(SyntaxError(format!("écart hors limites : {delta} (0 à 100)")));
            }
            return Ok(Value::Relative(sign * delta));
        }
        let text = self.number("valeur (0 à 100 ou Full)")?;
        let percent: f32 = text.parse().map_err(|_| SyntaxError(format!("valeur invalide : {text}")))?;
        if !(0.0..=100.0).contains(&percent) {
            return Err(SyntaxError(format!("valeur hors limites : {percent} (0 à 100)")));
        }
        Ok(Value::Percent(percent))
    }

    fn command(&mut self) -> Result<Command, SyntaxError> {
        let command = match self.peek() {
            Some(Token::Keyword(Keyword::Clear)) => {
                self.position += 1;
                Command::Clear
            }
            Some(Token::Keyword(Keyword::ClearAll)) => {
                self.position += 1;
                Command::ClearAll
            }
            Some(Token::Keyword(Keyword::Store)) if self.tokens.get(self.position + 1) == Some(&Token::Keyword(Keyword::Executor)) => {
                self.position += 2;
                Command::StoreExecutor(self.executor_ref()?)
            }
            Some(Token::Keyword(Keyword::Store)) => {
                self.position += 1;
                let object = self.object()?;
                // Nom et temps de fondu facultatifs, dans n'importe quel ordre.
                let (mut name, mut fade) = (None, None);
                loop {
                    if self.eat(Keyword::Fade) {
                        fade = Some(self.seconds()?);
                    } else if let Some(text) = self.text() {
                        name = Some(text);
                    } else {
                        break;
                    }
                }
                if fade.is_some() && !matches!(object, Object::Cue(_)) {
                    return Err(SyntaxError("Fade ne s'applique qu'aux cues".into()));
                }
                Command::Store { object, name, fade }
            }
            Some(Token::Keyword(Keyword::Off)) if self.tokens.get(self.position + 1) == Some(&Token::Keyword(Keyword::All)) => {
                self.position += 2;
                Command::OffAll
            }
            Some(Token::Keyword(k @ (Keyword::Go | Keyword::Back | Keyword::Off))) => {
                let k = *k;
                self.position += 1;
                let executor = self.optional_executor()?;
                match k {
                    Keyword::Go => Command::Go(executor),
                    Keyword::Back => Command::Back(executor),
                    _ => Command::Off(executor),
                }
            }
            Some(Token::Keyword(Keyword::Assert)) => {
                self.position += 1;
                Command::Assert(self.optional_executor()?)
            }
            Some(Token::Keyword(Keyword::Effect)) => {
                self.position += 1;
                Command::ApplyEffect(self.integer("numéro d'effet")?)
            }
            Some(Token::Keyword(Keyword::Tempo)) => {
                self.position += 1;
                let text = self.number("tempo en BPM")?;
                let bpm = text
                    .parse::<f32>()
                    .ok()
                    .filter(|b| (1.0..=999.0).contains(b))
                    .ok_or_else(|| SyntaxError(format!("tempo invalide : {text} (1 à 999 BPM)")))?;
                Command::SetTempo(bpm)
            }
            Some(Token::Keyword(Keyword::Goto)) => {
                self.position += 1;
                if !self.eat(Keyword::Cue) {
                    return Err(SyntaxError("Goto Cue <numéro> attendu".into()));
                }
                Command::Goto(self.cue_number()?)
            }
            Some(Token::Keyword(Keyword::Page)) => {
                self.position += 1;
                match self.next() {
                    Some(Token::Plus) => Command::Page(PageMove::Next),
                    Some(Token::Minus) => Command::Page(PageMove::Previous),
                    Some(Token::Number(n)) => match n.parse::<u32>() {
                        Ok(page) if (1..=MAX_PAGE).contains(&page) => Command::Page(PageMove::To(page)),
                        _ => return Err(SyntaxError(format!("page invalide : {n} (1 à {MAX_PAGE})"))),
                    },
                    _ => return Err(SyntaxError("Page <numéro>, Page + ou Page - attendu".into())),
                }
            }
            Some(Token::Keyword(Keyword::Macro)) => {
                self.position += 1;
                Command::Macro(self.integer("numéro de macro")?)
            }
            Some(Token::Keyword(Keyword::Wait)) => {
                self.position += 1;
                Command::Wait(self.seconds()?)
            }
            Some(Token::Keyword(k @ (Keyword::Block | Keyword::Unblock))) => {
                let k = *k;
                self.position += 1;
                if !self.eat(Keyword::Cue) {
                    return Err(SyntaxError(format!("{} Cue <numéro> attendu", k.name())));
                }
                let number = self.cue_number()?;
                if k == Keyword::Block { Command::Block(number) } else { Command::Unblock(number) }
            }
            Some(Token::Keyword(Keyword::Fix)) => {
                self.position += 1;
                self.eat(Keyword::Executor);
                Command::Fix(self.executor_ref()?)
            }
            Some(Token::Keyword(Keyword::Executor)) => {
                self.position += 1;
                let executor = self.executor_ref()?;
                if self.eat(Keyword::At) {
                    let Value::Percent(p) = self.value()? else {
                        return Err(SyntaxError("niveau de fader absolu attendu : Executor 3 At 50".into()));
                    };
                    Command::SetFader(executor, p)
                } else {
                    Command::SelectExecutor(executor)
                }
            }
            Some(Token::Keyword(Keyword::Label)) => {
                self.position += 1;
                let object = self.object()?;
                if object == Object::Cue(None) {
                    return Err(SyntaxError("numéro de cue attendu".into()));
                }
                let name = self.text().ok_or_else(|| SyntaxError("nom entre guillemets attendu".into()))?;
                Command::Label { object, name }
            }
            Some(Token::Keyword(Keyword::Delete)) => {
                self.position += 1;
                let object = self.object()?;
                if object == Object::Cue(None) {
                    return Err(SyntaxError("numéro de cue attendu".into()));
                }
                Command::Delete(object)
            }
            Some(Token::Keyword(Keyword::Preset)) => {
                self.position += 1;
                let (family, number) = self.preset_ref()?;
                Command::ApplyPreset(family, number)
            }
            _ => {
                let selection = self.selection()?;
                if self.eat(Keyword::Release) {
                    return self.end(Command::Release(selection));
                }
                let attribute = if self.eat(Keyword::Attribute) {
                    let name = self.text().ok_or_else(|| SyntaxError("nom d'attribut entre guillemets attendu : Attribute \"Pan\"".into()))?;
                    if !matches!(self.peek(), Some(Token::Keyword(Keyword::At))) {
                        return Err(SyntaxError(format!("At attendu après Attribute \"{name}\"")));
                    }
                    Some(name)
                } else {
                    None
                };
                let value = if self.eat(Keyword::At) { Some(self.value()?) } else { None };
                if selection.is_none() && value.is_none() {
                    return Err(SyntaxError(match self.peek() {
                        Some(Token::Number(_)) => "un numéro seul ne suffit pas : Fixture 1, Group 2…".into(),
                        Some(other) => format!("commande inattendue : {other:?}"),
                        None => "commande vide".into(),
                    }));
                }
                Command::Select { selection, attribute, value }
            }
        };
        self.end(command)
    }

    /// La commande doit s'arrêter là.
    fn end(&self, command: Command) -> Result<Command, SyntaxError> {
        if let Some(token) = self.peek() {
            return Err(SyntaxError(format!("en trop en fin de commande : {token:?}")));
        }
        Ok(command)
    }
}

pub fn parse(line: &str) -> Result<Command, SyntaxError> {
    let tokens = tokenize(line)?;
    if tokens.is_empty() {
        return Err(SyntaxError("commande vide".into()));
    }
    Parser { tokens, position: 0 }.command()
}

/// Raccourci pour les tests et l'aide : `Target` d'une fixture entière.
pub fn fixture(id: u32) -> Target {
    Target::whole(FixtureId(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select(line: &str) -> (Vec<(bool, Item)>, Option<Value>) {
        match parse(line).unwrap() {
            Command::Select { selection, value, .. } => (selection.map(|s| s.items).unwrap_or_default(), value),
            other => panic!("{line} : {other:?}"),
        }
    }

    #[test]
    fn selections_and_values() {
        let (items, value) = select("Fixture 1 Thru 4 At 50");
        assert_eq!(items, [(true, Item::Range(fixture(1), fixture(4)))]);
        assert_eq!(value, Some(Value::Percent(50.0)));

        let (items, _) = select("fi 1 + 3 - 2 + group 5");
        assert_eq!(
            items,
            [
                (true, Item::Target(fixture(1))),
                (true, Item::Target(fixture(3))),
                (false, Item::Target(fixture(2))),
                (true, Item::Group(5)),
            ]
        );

        let (items, _) = select("Fixture 10.1 Thru 10.4");
        let sub = |h| Target { fixture: FixtureId(10), head: Some(h) };
        assert_eq!(items, [(true, Item::Range(sub(0), sub(3)))]);

        assert_eq!(select("G 2 AT FULL").1, Some(Value::Percent(100.0)));
        assert_eq!(select("at 12,5").1, Some(Value::Percent(12.5)));
    }

    #[test]
    fn objects() {
        assert_eq!(parse("Preset 4.1"), Ok(Command::ApplyPreset(FeatureGroup::Color, 1)));
        assert_eq!(
            parse(r#"st g 3 "Cour""#),
            Ok(Command::Store { object: Object::Group(3), name: Some("Cour".into()), fade: None })
        );
        assert_eq!(
            parse(r#"Label Preset 2.5 "Face""#),
            Ok(Command::Label { object: Object::Preset(FeatureGroup::Position, 5), name: "Face".into() })
        );
        assert_eq!(parse("del p 4.1"), Ok(Command::Delete(Object::Preset(FeatureGroup::Color, 1))));
        assert_eq!(parse("clear"), Ok(Command::Clear));
        assert_eq!(parse("ClearAll"), Ok(Command::ClearAll));
    }

    #[test]
    fn playback_commands() {
        let cue = |s: &str| s.parse::<CueNumber>().unwrap();
        let here = |slot| ExecutorRef { page: None, slot };
        assert_eq!(parse("go"), Ok(Command::Go(None)));
        assert_eq!(parse("Go Executor 3"), Ok(Command::Go(Some(here(3)))));
        assert_eq!(parse("b exec 2"), Ok(Command::Back(Some(here(2)))));
        assert_eq!(parse("Off"), Ok(Command::Off(None)));
        assert_eq!(parse("Goto Cue 2.5"), Ok(Command::Goto(cue("2.5"))));
        assert_eq!(parse("Executor 4"), Ok(Command::SelectExecutor(here(4))));
        assert_eq!(parse("Exec 4 At 75"), Ok(Command::SetFader(here(4), 75.0)));
        assert_eq!(
            parse(r#"Store Cue 3 Fade 4 "Entrée""#),
            Ok(Command::Store { object: Object::Cue(Some(cue("3"))), name: Some("Entrée".into()), fade: Some(4.0) })
        );
        assert_eq!(parse("st cu"), Ok(Command::Store { object: Object::Cue(None), name: None, fade: None }));
        assert_eq!(parse("Delete Cue 1.5"), Ok(Command::Delete(Object::Cue(Some(cue("1.5"))))));
        assert!(parse("Delete Cue").is_err());
        assert!(parse("Store Group 1 Fade 3").is_err());
        assert!(parse("Goto 5").unwrap_err().0.contains("Goto Cue"));
        assert_eq!(parse("eff 3"), Ok(Command::ApplyEffect(3)));
        assert_eq!(parse("Store Exec 4"), Ok(Command::StoreExecutor(here(4))));
        assert_eq!(parse("off all"), Ok(Command::OffAll));
        assert_eq!(parse("Tempo 128"), Ok(Command::SetTempo(128.0)));
        assert!(parse("Tempo 0").is_err());
        assert!(parse("e 1").unwrap_err().0.contains("ambigu"), "Effect ou Executor");
    }

    #[test]
    fn executor_pages() {
        let on = |page, slot| ExecutorRef { page: Some(page), slot };
        assert_eq!(parse("Page 2"), Ok(Command::Page(PageMove::To(2))));
        assert_eq!(parse("page +"), Ok(Command::Page(PageMove::Next)));
        assert_eq!(parse("pa -"), Ok(Command::Page(PageMove::Previous)));
        assert!(parse("Page 0").is_err());
        assert!(parse("Page").is_err());
        assert_eq!(parse("Go Executor 2.5"), Ok(Command::Go(Some(on(2, 5)))));
        assert_eq!(parse("Exec 3.12 At 80"), Ok(Command::SetFader(on(3, 12), 80.0)));
        assert_eq!(parse("Fix Executor 3"), Ok(Command::Fix(ExecutorRef { page: None, slot: 3 })));
        assert_eq!(parse("fix 2.1"), Ok(Command::Fix(on(2, 1))));
        assert!(parse("Executor 21").is_err(), "20 places par page");
        assert!(parse("Executor 2.0").is_err());
        // Les abréviations courantes ne changent pas.
        assert!(matches!(parse("fi 1 at 50"), Ok(Command::Select { .. })));
        assert_eq!(parse("p 4.1"), Ok(Command::ApplyPreset(FeatureGroup::Color, 1)));
    }

    #[test]
    fn attributes_and_relative_values() {
        let attribute = |line: &str| match parse(line) {
            Ok(Command::Select { attribute, value, .. }) => (attribute, value),
            other => panic!("{line} : {other:?}"),
        };
        assert_eq!(attribute(r#"Attribute "Pan" At 50"#), (Some("Pan".into()), Some(Value::Percent(50.0))));
        assert_eq!(attribute(r#"Fixture 1 Thru 4 att "tilt" at - 5"#), (Some("tilt".into()), Some(Value::Relative(-5.0))));
        assert_eq!(attribute("At + 10"), (None, Some(Value::Relative(10.0))));
        assert!(parse("At + 120").is_err());
        assert!(parse(r#"Attribute "Pan""#).unwrap_err().0.contains("At attendu"));
        assert!(parse("Attribute At 50").unwrap_err().0.contains("guillemets"));
    }

    #[test]
    fn macros() {
        assert_eq!(parse("Macro 2"), Ok(Command::Macro(2)));
        assert_eq!(parse("ma 12"), Ok(Command::Macro(12)));
        assert_eq!(parse("Wait 2.5"), Ok(Command::Wait(2.5)));
        assert_eq!(
            parse(r#"Store Macro 3 "Ballet""#),
            Ok(Command::Store { object: Object::Macro(3), name: Some("Ballet".into()), fade: None })
        );
        assert_eq!(parse(r#"Label Macro 3 "Noir""#), Ok(Command::Label { object: Object::Macro(3), name: "Noir".into() }));
        assert_eq!(parse("Delete Macro 3"), Ok(Command::Delete(Object::Macro(3))));
        assert!(parse("Wait").is_err());
    }

    #[test]
    fn block_and_unblock() {
        let cue = |s: &str| s.parse::<CueNumber>().unwrap();
        assert_eq!(parse("Block Cue 3"), Ok(Command::Block(cue("3"))));
        assert_eq!(parse("unbl cu 2.5"), Ok(Command::Unblock(cue("2.5"))));
        assert!(parse("Block 3").unwrap_err().0.contains("Block Cue"));
        assert_eq!(parse("b"), Ok(Command::Back(None)), "« b » reste Back");
    }

    #[test]
    fn assert_and_release() {
        assert_eq!(parse("Assert"), Ok(Command::Assert(None)));
        assert_eq!(parse("as exec 2.3"), Ok(Command::Assert(Some(ExecutorRef { page: Some(2), slot: 3 }))));
        assert_eq!(parse("Release"), Ok(Command::Release(None)));
        let Ok(Command::Release(Some(selection))) = parse("Fixture 1 Thru 2 Release") else { panic!() };
        assert_eq!(selection.items, [(true, Item::Range(fixture(1), fixture(2)))]);
        assert!(parse("Release 3").unwrap_err().0.contains("en trop"));
    }

    #[test]
    fn helpful_errors() {
        let err = |line| parse(line).unwrap_err().0;
        assert!(err("f 1").contains("ambigu"), "{}", err("f 1"));
        assert!(err("c").contains("ambigu"));
        assert!(err("Fixture 1 At 150").contains("hors limites"));
        assert!(err("Preset 9.1").contains("famille"));
        assert!(err("Preset 4").contains("famille.numéro"));
        assert!(err("12").contains("numéro seul"));
        assert!(err("Fixture 1 2").contains("+ ou -"));
        assert!(err("Fixture 1 At 50 60").contains("en trop"));
        assert!(err("Bonjour").contains("inconnu"));
    }
}

//! Langage de commande, sous-ensemble de la syntaxe grandMA2.
//!
//! ```text
//! Fixture 1 Thru 4 At 50        Group 2 At Full        Fixture 5.3
//! Fixture 1 + 3 - 2             Preset 4.1             Store Group 3 "Cour"
//! Store Preset 4.1 "Rouge"      Label Group 3 "Jardin" Delete Preset 4.1
//! Clear                         ClearAll
//! Go   Back   Off               Go Executor 2           Goto Cue 5
//! Store Cue 3 Fade 4 "Entrée"   Delete Cue 3            Executor 2 At 50
//! ```
//!
//! Les mots-clés sont insensibles à la casse et s'abrègent tant qu'il n'y a
//! pas d'ambiguïté : `fi 1 th 4 at 50`.

use folie_fixture::{FeatureGroup, FixtureId, Target};
use folie_show::CueNumber;
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
}

/// Abréviations d'usage qui l'emportent sur la règle du préfixe unique.
const ALIASES: [(&str, Keyword); 2] = [("g", Keyword::Group), ("exec", Keyword::Executor)];

const KEYWORDS: [(&str, Keyword); 18] = [
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Group(u32),
    Preset(FeatureGroup, u32),
    /// Cue de la séquence de l'executor sélectionné ; sans numéro pour `Store Cue`.
    Cue(Option<CueNumber>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Sélection éventuellement suivie d'une valeur de dimmer : `Fixture 1 Thru 4 At 50`.
    Select { selection: Option<Selection>, value: Option<Value> },
    ApplyPreset(FeatureGroup, u32),
    Store { object: Object, name: Option<String>, fade: Option<f32> },
    Label { object: Object, name: String },
    Delete(Object),
    Clear,
    ClearAll,
    /// `None` : l'executor sélectionné.
    Go(Option<u32>),
    Back(Option<u32>),
    Off(Option<u32>),
    Goto(CueNumber),
    SelectExecutor(u32),
    /// Fader d'un executor, en pourcentage.
    SetFader(u32, f32),
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

    /// `Executor 2` facultatif après Go, Back, Off.
    fn optional_executor(&mut self) -> Result<Option<u32>, SyntaxError> {
        if self.eat(Keyword::Executor) { Ok(Some(self.integer("numéro d'executor")?)) } else { Ok(None) }
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
        if self.eat(Keyword::Group) {
            Ok(Object::Group(self.integer("numéro de groupe")?))
        } else if self.eat(Keyword::Preset) {
            let (family, number) = self.preset_ref()?;
            Ok(Object::Preset(family, number))
        } else {
            Err(SyntaxError("Group, Preset ou Cue attendu".into()))
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
            Some(Token::Keyword(Keyword::Goto)) => {
                self.position += 1;
                if !self.eat(Keyword::Cue) {
                    return Err(SyntaxError("Goto Cue <numéro> attendu".into()));
                }
                Command::Goto(self.cue_number()?)
            }
            Some(Token::Keyword(Keyword::Executor)) => {
                self.position += 1;
                let executor = self.integer("numéro d'executor")?;
                if self.eat(Keyword::At) {
                    let Value::Percent(p) = self.value()?;
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
                let value = if self.eat(Keyword::At) { Some(self.value()?) } else { None };
                if selection.is_none() && value.is_none() {
                    return Err(SyntaxError(match self.peek() {
                        Some(Token::Number(_)) => "un numéro seul ne suffit pas : Fixture 1, Group 2…".into(),
                        Some(other) => format!("commande inattendue : {other:?}"),
                        None => "commande vide".into(),
                    }));
                }
                Command::Select { selection, value }
            }
        };
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
            Command::Select { selection, value } => (selection.map(|s| s.items).unwrap_or_default(), value),
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
        assert_eq!(parse("go"), Ok(Command::Go(None)));
        assert_eq!(parse("Go Executor 3"), Ok(Command::Go(Some(3))));
        assert_eq!(parse("b exec 2"), Ok(Command::Back(Some(2))));
        assert_eq!(parse("Off"), Ok(Command::Off(None)));
        assert_eq!(parse("Goto Cue 2.5"), Ok(Command::Goto(cue("2.5"))));
        assert_eq!(parse("Executor 4"), Ok(Command::SelectExecutor(4)));
        assert_eq!(parse("Exec 4 At 75"), Ok(Command::SetFader(4, 75.0)));
        assert_eq!(
            parse(r#"Store Cue 3 Fade 4 "Entrée""#),
            Ok(Command::Store { object: Object::Cue(Some(cue("3"))), name: Some("Entrée".into()), fade: Some(4.0) })
        );
        assert_eq!(parse("st cu"), Ok(Command::Store { object: Object::Cue(None), name: None, fade: None }));
        assert_eq!(parse("Delete Cue 1.5"), Ok(Command::Delete(Object::Cue(Some(cue("1.5"))))));
        assert!(parse("Delete Cue").is_err());
        assert!(parse("Store Group 1 Fade 3").is_err());
        assert!(parse("Goto 5").unwrap_err().0.contains("Goto Cue"));
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

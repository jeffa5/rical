//! Key bindings: key sequences mapped to command lines, in aerc's
//! `binds.conf` format with the key syntax of `keyboard-centric` (chatters):
//!
//! ```text
//! [global]
//! t = :today<Enter>
//! <C-n> = :new<Enter>
//! gd = :goto<space>          # no <Enter>: opens the command line prefilled
//!
//! [month]
//! <Down> = :day +7<Enter>
//! ```
//!
//! Keys are characters (`a`, `A`, `[`) or `<...>` with optional `C-`, `A-`,
//! `S-` modifiers and a key name (`<C-z>`, `<S-Down>`, `<Enter>`). Sections
//! name the view the bindings apply to; `[global]` applies everywhere and
//! view sections take precedence. A binding with an empty value removes an
//! inherited one.

use std::{collections::BTreeMap, fmt, str::FromStr};

pub const DEFAULTS: &str = include_str!("binds.conf");

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Key {
    /// A single character, or a lower-case key name such as "enter".
    pub name: String,
    pub ctrl: bool,
    pub alt: bool,
    /// Only for named keys; for characters it is part of the character.
    pub shift: bool,
}

impl Key {
    pub fn char(c: char) -> Self {
        Self {
            name: c.to_string(),
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    fn is_char(&self) -> bool {
        self.name.chars().count() == 1
    }
}

const NAMES: &[&str] = &[
    "enter",
    "esc",
    "space",
    "tab",
    "backspace",
    "delete",
    "insert",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
];

impl FromStr for Key {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let Some(inner) = s.strip_prefix('<').and_then(|s| s.strip_suffix('>')) else {
            let mut chars = s.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(Key::char(c)),
                _ => Err(format!("invalid key {s:?}")),
            };
        };
        let mut key = Key::char(' ');
        let mut rest = inner;
        loop {
            match rest.get(..2) {
                Some("C-" | "c-") => key.ctrl = true,
                Some("A-" | "a-") => key.alt = true,
                Some("S-" | "s-") => key.shift = true,
                _ => break,
            }
            rest = &rest[2..];
        }
        let mut chars = rest.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => {
                // <S-a> is A; <C-A> is Ctrl+Shift+a
                key.name = if key.shift {
                    key.shift = false;
                    c.to_uppercase().to_string()
                } else {
                    c.to_string()
                };
            }
            _ => {
                let name = rest.to_lowercase();
                let name = match name.as_str() {
                    "return" | "cr" => "enter".to_owned(),
                    "escape" => "esc".to_owned(),
                    "del" => "delete".to_owned(),
                    "pgup" => "pageup".to_owned(),
                    "pgdown" | "pgdn" => "pagedown".to_owned(),
                    "lt" => {
                        return Ok(Key {
                            name: "<".into(),
                            ..key
                        });
                    }
                    "gt" => {
                        return Ok(Key {
                            name: ">".into(),
                            ..key
                        });
                    }
                    _ => name,
                };
                if !NAMES.contains(&name.as_str()) {
                    return Err(format!("unknown key name {rest:?}"));
                }
                key.name = name;
            }
        }
        Ok(key)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let plain = !self.ctrl && !self.alt && !self.shift;
        if plain && self.is_char() && !matches!(self.name.as_str(), "<" | ">" | " ") {
            return write!(f, "{}", self.name);
        }
        write!(f, "<")?;
        if self.ctrl {
            write!(f, "C-")?;
        }
        if self.alt {
            write!(f, "A-")?;
        }
        if self.shift {
            write!(f, "S-")?;
        }
        match self.name.as_str() {
            "<" => write!(f, "lt>"),
            ">" => write!(f, "gt>"),
            " " => write!(f, "space>"),
            n if self.is_char() => write!(f, "{n}>"),
            n => {
                let mut c = n.chars();
                let first = c.next().unwrap().to_uppercase();
                write!(f, "{first}{}>", c.as_str())
            }
        }
    }
}

/// Parse a key sequence such as `gg`, `<C-x>k` or `<S-Down>`.
pub fn parse_keys(s: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let len = if rest.starts_with('<')
            && let Some(end) = rest.find('>')
            && end > 1
        {
            end + 1
        } else {
            rest.chars().next().unwrap().len_utf8()
        };
        keys.push(rest[..len].parse()?);
        rest = &rest[len..];
    }
    Ok(keys)
}

/// What a binding does: run some command lines, then perhaps open the
/// command line with some text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub run: Vec<String>,
    pub prefill: Option<String>,
}

impl FromStr for Action {
    type Err = String;

    /// `:a<Enter>:b x<Enter>:c<space>` runs `a` and `b x`, then opens the
    /// command line with `c `.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut run = Vec::new();
        let mut rest = s.trim();
        loop {
            let Some(body) = rest.strip_prefix(':') else {
                return Err(format!("bindings must start with ':' ({s:?})"));
            };
            match find_ci(body, "<enter>") {
                Some(i) => {
                    run.push(unescape(&body[..i]));
                    rest = &body[i + "<enter>".len()..];
                    if rest.is_empty() {
                        return Ok(Action { run, prefill: None });
                    }
                }
                None => {
                    return Ok(Action {
                        run,
                        prefill: Some(unescape(body)),
                    });
                }
            }
        }
    }
}

fn find_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack.to_ascii_lowercase().find(needle)
}

fn unescape(s: &str) -> String {
    let mut s = s.to_owned();
    for (from, to) in [
        ("<space>", " "),
        ("<Space>", " "),
        ("<lt>", "<"),
        ("<gt>", ">"),
    ] {
        s = s.replace(from, to);
    }
    s
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lookup {
    Found(Action),
    /// The keys so far start a longer binding.
    Prefix,
    None,
}

/// Bindings per section ("global", "grid", "month", "agenda").
#[derive(Debug, Clone, Default)]
pub struct Binds {
    pub sections: BTreeMap<String, BTreeMap<Vec<Key>, Option<Action>>>,
}

impl Binds {
    /// Parse a binds.conf; errors name the line.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut binds = Self::default();
        let mut section = "global".to_owned();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                section = match name.trim() {
                    "default" => "global".to_owned(),
                    s => s.to_owned(),
                };
                continue;
            }
            let err = |e: String| format!("line {}: {e}", n + 1);
            let (keys, value) = line
                .split_once(" = ")
                .or_else(|| line.split_once('='))
                .ok_or_else(|| err("expected `keys = :command<Enter>`".into()))?;
            let keys = parse_keys(keys.trim()).map_err(err)?;
            // strip a trailing comment
            let value = value.split(" #").next().unwrap().trim();
            let action = if value.is_empty() {
                None
            } else {
                Some(value.parse().map_err(err)?)
            };
            binds
                .sections
                .entry(section.clone())
                .or_default()
                .insert(keys, action);
        }
        Ok(binds)
    }

    /// Layer `other` on top: its bindings replace these, empty ones remove.
    pub fn merge(&mut self, other: Binds) {
        for (section, binds) in other.sections {
            self.sections.entry(section).or_default().extend(binds);
        }
    }

    /// The action for `keys` in `section`, falling back to global.
    pub fn lookup(&self, section: &str, keys: &[Key]) -> Lookup {
        let mut prefix = false;
        for name in [section, "global"] {
            let Some(binds) = self.sections.get(name) else {
                continue;
            };
            match binds.get(keys) {
                Some(Some(action)) => return Lookup::Found(action.clone()),
                // removed here: do not fall back to global
                Some(None) => return Lookup::None,
                None => {}
            }
            prefix |= binds
                .iter()
                .any(|(k, a)| a.is_some() && k.len() > keys.len() && k.starts_with(keys));
        }
        if prefix { Lookup::Prefix } else { Lookup::None }
    }
}

pub fn keys_to_string(keys: &[Key]) -> String {
    keys.iter().map(|k| k.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(s: &str) -> Vec<Key> {
        parse_keys(s).unwrap()
    }

    #[test]
    fn keys_round_trip() {
        for s in [
            "a", "A", "[", "gg", "<C-z>", "<C-Z>", "<A-1>", "<S-Down>", "<Enter>", "<F9>",
            "<C-x>k", "<lt>", "<Space>",
        ] {
            assert_eq!(keys_to_string(&k(s)), s, "{s}");
        }
        assert_eq!(k("<S-a>"), k("A"));
        assert_eq!(k("<c-z>"), k("<C-z>"));
        assert_eq!(k("<Return>"), k("<Enter>"));
        assert!(parse_keys("<Nope>").is_err());
    }

    #[test]
    fn actions() {
        assert_eq!(
            ":today<Enter>".parse::<Action>().unwrap(),
            Action {
                run: vec!["today".into()],
                prefill: None
            }
        );
        assert_eq!(
            ":view day<Enter>:today<Enter>"
                .parse::<Action>()
                .unwrap()
                .run,
            ["view day", "today"]
        );
        assert_eq!(
            ":goto<space>".parse::<Action>().unwrap(),
            Action {
                run: vec![],
                prefill: Some("goto ".into())
            }
        );
        assert!("today".parse::<Action>().is_err());
    }

    #[test]
    fn lookup_and_override() {
        let mut binds = Binds::parse(
            "
            # comment
            t = :today<Enter>
            gg = :top<Enter>
            <Down> = :select next<Enter>

            [month]
            <Down> = :day +7<Enter>
            ",
        )
        .unwrap();
        let run = |l: Lookup| match l {
            Lookup::Found(a) => a.run.join(";"),
            other => format!("{other:?}"),
        };
        assert_eq!(run(binds.lookup("grid", &k("t"))), "today");
        assert_eq!(run(binds.lookup("grid", &k("<Down>"))), "select next");
        assert_eq!(run(binds.lookup("month", &k("<Down>"))), "day +7");
        assert_eq!(binds.lookup("grid", &k("g")), Lookup::Prefix);
        assert_eq!(binds.lookup("grid", &k("x")), Lookup::None);

        binds.merge(Binds::parse("[agenda]\n<Down> =\n[global]\nt = :view day<Enter>").unwrap());
        assert_eq!(
            binds.lookup("agenda", &k("<Down>")),
            Lookup::None,
            "removed"
        );
        assert_eq!(run(binds.lookup("grid", &k("t"))), "view day");
    }

    #[test]
    fn defaults_parse() {
        let binds = Binds::parse(DEFAULTS).unwrap();
        assert!(binds.sections.contains_key("global"));
    }

    #[test]
    fn errors_name_the_line() {
        let e = Binds::parse("t = :today<Enter>\nbroken line").unwrap_err();
        assert!(e.starts_with("line 2"), "{e}");
    }
}

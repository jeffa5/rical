//! A small generic command engine, modelled on aerc's: commands have names,
//! aliases, a description, the contexts they apply in and a declarative
//! argument spec. Lines are lexed shell-style, names may be abbreviated to
//! a unique prefix, and completion is driven by the spec.
//!
//! Nothing here knows about GTK or calendars; dynamic values (calendar
//! names, say) come from a [`Values`] implementation.

use std::fmt;

/// Where a command applies, as a bit set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context(pub u32);

impl Context {
    pub const fn contains(self, other: Context) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn or(self, other: Context) -> Context {
        Context(self.0 | other.0)
    }
}

/// What kind of value an argument takes, which decides its completions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One of a fixed set of words.
    Choice(&'static [&'static str]),
    /// A value from [`Values::values`] with this key, e.g. "calendar".
    Dynamic(&'static str),
    /// Free text; suggestions come from [`Values::suggestions`].
    Text(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct Arg {
    pub name: &'static str,
    pub kind: Kind,
    pub optional: bool,
    /// Takes the rest of the line.
    pub rest: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Flag {
    pub name: &'static str,
    pub kind: Kind,
    pub help: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub struct Spec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub help: &'static str,
    pub contexts: Context,
    pub args: &'static [Arg],
    pub flags: &'static [Flag],
}

impl Spec {
    pub fn names(&self) -> impl Iterator<Item = &'static str> + '_ {
        std::iter::once(self.name).chain(self.aliases.iter().copied())
    }

    /// e.g. "move <by>" or "new [--calendar C] [title...]".
    pub fn usage(&self) -> String {
        let mut s = self.name.to_owned();
        for f in self.flags {
            s += &format!(" [--{} {}]", f.name, f.name.to_uppercase());
        }
        for a in self.args {
            let dots = if a.rest { "..." } else { "" };
            if a.optional {
                s += &format!(" [{}{dots}]", a.name);
            } else {
                s += &format!(" <{}{dots}>", a.name);
            }
        }
        s
    }
}

/// Dynamic values for completion.
pub trait Values {
    /// All values of a dynamic kind, e.g. the calendar names.
    fn values(&self, key: &str) -> Vec<String>;
    /// Example values for free text, e.g. "today" for dates.
    fn suggestions(&self, key: &str) -> Vec<String> {
        let _ = key;
        Vec::new()
    }
    /// Values of a dynamic kind with a description each, which completion
    /// also matches (by the start of any word): an address and its name.
    fn described(&self, key: &str) -> Vec<(String, String)> {
        self.values(key)
            .into_iter()
            .map(|v| (v, String::new()))
            .collect()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Unknown(String),
    Ambiguous(String, Vec<&'static str>),
    NotHere(&'static str),
    Missing(&'static str, &'static str),
    TooMany(&'static str),
    BadValue(String, Vec<String>),
    UnknownFlag(String),
    Unclosed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unknown(n) => write!(f, "unknown command “{n}”"),
            Error::Ambiguous(n, c) => write!(f, "“{n}” could be {}", c.join(", ")),
            Error::NotHere(n) => write!(f, "“{n}” is not available here"),
            Error::Missing(cmd, arg) => write!(f, "{cmd}: missing <{arg}>"),
            Error::TooMany(cmd) => write!(f, "{cmd}: too many arguments"),
            Error::BadValue(v, ok) => write!(f, "“{v}” is not one of {}", ok.join(", ")),
            Error::UnknownFlag(fl) => write!(f, "unknown option “{fl}”"),
            Error::Unclosed => write!(f, "unclosed quote"),
        }
    }
}

impl std::error::Error for Error {}

/// A command line resolved against its spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub spec: &'static str,
    /// Positional values in spec order; a `rest` argument is joined.
    pub args: Vec<Option<String>>,
    pub flags: Vec<(&'static str, String)>,
}

impl Parsed {
    pub fn arg(&self, i: usize) -> Option<&str> {
        self.args.get(i).and_then(|a| a.as_deref())
    }

    pub fn flag(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| v.as_str())
    }
}

/// Split a line into words: whitespace separated, with '' and "" quoting
/// and backslash escapes. Also says whether the line ends inside a word
/// (so completion knows whether a new word has started).
pub fn lex(line: &str) -> Result<(Vec<String>, bool), Error> {
    let (words, in_word) = lex_spans(line, false)?;
    Ok((words.into_iter().map(|(w, _)| w).collect(), in_word))
}

/// Like [`lex`], with the byte offset where each word starts. With
/// `allow_open` an unclosed quote is accepted, as while typing.
fn lex_spans(line: &str, allow_open: bool) -> Result<(Vec<(String, usize)>, bool), Error> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut start = None;
    let mut quote: Option<char> = None;
    let mut chars = line.char_indices();
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                start.get_or_insert(i);
            }
            (None, '\\') => {
                if let Some((_, n)) = chars.next() {
                    cur.push(n);
                }
                start.get_or_insert(i);
            }
            (None, c) if c.is_whitespace() => {
                if let Some(s) = start.take() {
                    words.push((std::mem::take(&mut cur), s));
                }
            }
            (None, c) => {
                cur.push(c);
                start.get_or_insert(i);
            }
        }
    }
    if quote.is_some() && !allow_open {
        return Err(Error::Unclosed);
    }
    let in_word = start.is_some();
    if let Some(s) = start {
        words.push((cur, s));
    }
    Ok((words, in_word))
}

/// Quote a word if needed so [`lex`] reads it back as one word.
pub fn quote(word: &str) -> String {
    if !word.is_empty() && !word.contains(|c: char| c.is_whitespace() || "'\"\\".contains(c)) {
        word.to_owned()
    } else {
        format!("\"{}\"", word.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

pub struct Registry {
    pub specs: &'static [Spec],
}

impl Registry {
    pub fn active(&self, ctx: Context) -> impl Iterator<Item = &'static Spec> + '_ {
        self.specs.iter().filter(move |s| s.contexts.contains(ctx))
    }

    /// Find a command by name, alias or unique prefix of one.
    pub fn resolve(&self, name: &str, ctx: Context) -> Result<&'static Spec, Error> {
        if let Some(s) = self.specs.iter().find(|s| s.names().any(|n| n == name)) {
            return if s.contexts.contains(ctx) {
                Ok(s)
            } else {
                Err(Error::NotHere(s.name))
            };
        }
        let mut matches = self
            .active(ctx)
            .filter(|s| s.names().any(|n| n.starts_with(name)))
            .collect::<Vec<_>>();
        matches.dedup_by_key(|s| s.name);
        match matches.as_slice() {
            [one] => Ok(one),
            [] => Err(Error::Unknown(name.to_owned())),
            many => Err(Error::Ambiguous(
                name.to_owned(),
                many.iter().map(|s| s.name).collect(),
            )),
        }
    }

    pub fn parse(&self, line: &str, ctx: Context) -> Result<Parsed, Error> {
        let (words, _) = lex(line.trim_start_matches(':'))?;
        let Some((name, rest)) = words.split_first() else {
            return Err(Error::Unknown(String::new()));
        };
        let spec = self.resolve(name, ctx)?;

        let mut flags = Vec::new();
        let mut positional = Vec::new();
        let mut it = rest.iter();
        while let Some(w) = it.next() {
            if let Some(f) = w.strip_prefix("--") {
                let (fname, inline) = match f.split_once('=') {
                    Some((n, v)) => (n, Some(v.to_owned())),
                    None => (f, None),
                };
                let flag = spec
                    .flags
                    .iter()
                    .find(|fl| fl.name == fname)
                    .ok_or_else(|| Error::UnknownFlag(w.clone()))?;
                let value = match inline.or_else(|| it.next().cloned()) {
                    Some(v) => v,
                    None => return Err(Error::Missing(spec.name, flag.name)),
                };
                check(&flag.kind, &value)?;
                flags.push((flag.name, value));
            } else {
                positional.push(w.clone());
            }
        }

        let mut args = Vec::new();
        let mut words = positional.into_iter();
        for arg in spec.args {
            let value = if arg.rest {
                let rest = words.by_ref().collect::<Vec<_>>();
                (!rest.is_empty()).then(|| rest.join(" "))
            } else {
                words.next()
            };
            match &value {
                None if !arg.optional => return Err(Error::Missing(spec.name, arg.name)),
                Some(v) => check(&arg.kind, v)?,
                None => {}
            }
            args.push(value);
        }
        if words.next().is_some() {
            return Err(Error::TooMany(spec.name));
        }
        Ok(Parsed {
            spec: spec.name,
            args,
            flags,
        })
    }

    /// Completions for the word being typed at the end of `line`.
    pub fn complete(&self, line: &str, ctx: Context, values: &dyn Values) -> Completion {
        let body = line.strip_prefix(':').unwrap_or(line);
        let offset = line.len() - body.len();
        let Ok((mut words, in_word)) = lex_spans(body, true) else {
            return Completion::default();
        };
        let (current, start) = if in_word {
            let (w, s) = words.pop().unwrap();
            (w, offset + s)
        } else {
            (String::new(), line.len())
        };
        let before = words.into_iter().map(|(w, _)| w).collect::<Vec<_>>();

        let candidates = if before.is_empty() {
            let mut c = self
                .active(ctx)
                .flat_map(|s| s.names().map(move |n| (n, s)))
                .filter(|(n, _)| n.starts_with(current.as_str()))
                .map(|(n, s)| Candidate {
                    value: n.to_owned(),
                    help: s.help.to_owned(),
                    usage: s.usage(),
                })
                .collect::<Vec<_>>();
            c.sort_by(|a, b| a.value.cmp(&b.value));
            c
        } else {
            match self.resolve(&before[0], ctx) {
                Ok(spec) => complete_args(spec, &before[1..], &current, values),
                Err(_) => Vec::new(),
            }
        };
        Completion {
            start,
            prefix: current,
            candidates,
        }
    }
}

fn check(kind: &Kind, value: &str) -> Result<(), Error> {
    match kind {
        Kind::Choice(choices) if !choices.contains(&value) => Err(Error::BadValue(
            value.to_owned(),
            choices.iter().map(|c| c.to_string()).collect(),
        )),
        _ => Ok(()),
    }
}

fn complete_args(
    spec: &Spec,
    done: &[String],
    current: &str,
    values: &dyn Values,
) -> Vec<Candidate> {
    let candidate = |v: String, help: &str| Candidate {
        value: v,
        help: help.to_owned(),
        usage: String::new(),
    };
    let of_kind = |kind: &Kind, help: &str| -> Vec<Candidate> {
        if let Kind::Dynamic(key) = kind {
            let lower = current.to_lowercase();
            return values
                .described(key)
                .into_iter()
                .filter(|(v, d)| {
                    starts_with_smart(v, current)
                        || (!lower.is_empty()
                            && d.to_lowercase()
                                .split_whitespace()
                                .any(|w| w.starts_with(&lower)))
                })
                .map(|(v, d)| candidate(v, if d.is_empty() { help } else { &d }))
                .collect();
        }
        let all = match kind {
            Kind::Choice(c) => c.iter().map(|s| s.to_string()).collect(),
            Kind::Dynamic(key) => values.values(key),
            Kind::Text(key) => values.suggestions(key),
        };
        all.into_iter()
            .filter(|v| starts_with_smart(v, current))
            .map(|v| candidate(v, help))
            .collect()
    };

    // value for a flag?
    if let Some(prev) = done.last().and_then(|w| w.strip_prefix("--"))
        && !prev.contains('=')
        && let Some(flag) = spec.flags.iter().find(|f| f.name == prev)
    {
        return of_kind(&flag.kind, flag.help);
    }
    if current.starts_with('-') {
        return spec
            .flags
            .iter()
            .map(|f| format!("--{}", f.name))
            .filter(|f| f.starts_with(current))
            .map(|f| candidate(f, ""))
            .collect();
    }

    // which positional argument is this?
    let mut n = 0;
    let mut skip = false;
    for w in done {
        if skip {
            skip = false;
        } else if w.starts_with("--") {
            skip = !w.contains('=');
        } else {
            n += 1;
        }
    }
    let arg = spec
        .args
        .get(n)
        .or_else(|| spec.args.last().filter(|a| a.rest));
    let mut out = arg.map_or_else(Vec::new, |a| of_kind(&a.kind, a.name));
    if current.is_empty() {
        out.extend(
            spec.flags
                .iter()
                .map(|f| candidate(format!("--{}", f.name), f.help)),
        );
    }
    out
}

/// Prefix match, case-insensitive unless the prefix has capitals (as aerc).
fn starts_with_smart(value: &str, prefix: &str) -> bool {
    if prefix.chars().any(char::is_uppercase) {
        value.starts_with(prefix)
    } else {
        value.to_lowercase().starts_with(&prefix.to_lowercase())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub value: String,
    pub help: String,
    /// For command names, how to call it.
    pub usage: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Completion {
    /// Byte offset in the line where the completed word starts.
    pub start: usize,
    pub prefix: String,
    pub candidates: Vec<Candidate>,
}

impl Completion {
    /// The line with the word replaced by `candidate`, plus a space.
    pub fn apply(&self, line: &str, candidate: &Candidate) -> String {
        format!("{}{} ", &line[..self.start], quote(&candidate.value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: Context = Context(0b11);
    const GRID: Context = Context(0b01);
    const MONTH: Context = Context(0b10);

    static SPECS: &[Spec] = &[
        Spec {
            name: "today",
            aliases: &[],
            help: "Go to today",
            contexts: ALL,
            args: &[],
            flags: &[],
        },
        Spec {
            name: "toggle",
            aliases: &[],
            help: "Toggle a calendar",
            contexts: ALL,
            args: &[Arg {
                name: "calendar",
                kind: Kind::Dynamic("calendar"),
                optional: false,
                rest: true,
            }],
            flags: &[],
        },
        Spec {
            name: "view",
            aliases: &["v"],
            help: "Switch view",
            contexts: ALL,
            args: &[Arg {
                name: "view",
                kind: Kind::Choice(&["day", "week", "month"]),
                optional: false,
                rest: false,
            }],
            flags: &[],
        },
        Spec {
            name: "new",
            aliases: &[],
            help: "New event",
            contexts: ALL,
            args: &[Arg {
                name: "title",
                kind: Kind::Text("title"),
                optional: true,
                rest: true,
            }],
            flags: &[
                Flag {
                    name: "calendar",
                    kind: Kind::Dynamic("calendar"),
                    help: "calendar to add to",
                },
                Flag {
                    name: "at",
                    kind: Kind::Text("when"),
                    help: "start",
                },
            ],
        },
        Spec {
            name: "resize",
            aliases: &[],
            help: "Change the end",
            contexts: GRID,
            args: &[],
            flags: &[],
        },
    ];

    struct V;
    impl Values for V {
        fn values(&self, key: &str) -> Vec<String> {
            match key {
                "calendar" => vec!["Personal".into(), "work".into(), "Work stuff".into()],
                _ => vec![],
            }
        }
        fn suggestions(&self, key: &str) -> Vec<String> {
            match key {
                "when" => vec!["today".into(), "tomorrow".into()],
                _ => vec![],
            }
        }
    }

    fn reg() -> Registry {
        Registry { specs: SPECS }
    }

    fn names(c: &Completion) -> Vec<&str> {
        c.candidates.iter().map(|c| c.value.as_str()).collect()
    }

    #[test]
    fn lexing() {
        assert_eq!(
            lex(r#"new --at "tomorrow 9:00" it\'s here "#).unwrap(),
            (
                vec![
                    "new".into(),
                    "--at".into(),
                    "tomorrow 9:00".into(),
                    "it's".into(),
                    "here".into()
                ],
                false
            )
        );
        assert!(lex("view we").unwrap().1);
        assert_eq!(lex("say 'oops"), Err(Error::Unclosed));
        assert_eq!(quote("Work stuff"), "\"Work stuff\"");
        assert_eq!(quote("work"), "work");
    }

    #[test]
    fn resolving() {
        let r = reg();
        assert_eq!(r.resolve("today", ALL).unwrap().name, "today");
        assert_eq!(r.resolve("v", ALL).unwrap().name, "view", "alias");
        assert_eq!(r.resolve("vi", ALL).unwrap().name, "view", "prefix");
        assert_eq!(r.resolve("tod", ALL).unwrap().name, "today");
        assert!(matches!(r.resolve("to", ALL), Err(Error::Ambiguous(_, _))));
        assert_eq!(
            r.resolve("resize", MONTH).err(),
            Some(Error::NotHere("resize"))
        );
        // a prefix only considers commands available here
        assert!(matches!(r.resolve("r", MONTH), Err(Error::Unknown(_))));
        assert!(matches!(r.resolve("zzz", ALL), Err(Error::Unknown(_))));
    }

    #[test]
    fn parsing() {
        let r = reg();
        let p = r.parse(":view week", ALL).unwrap();
        assert_eq!((p.spec, p.arg(0)), ("view", Some("week")));
        assert!(matches!(
            r.parse("view year", ALL),
            Err(Error::BadValue(_, _))
        ));
        assert_eq!(r.parse("view", ALL), Err(Error::Missing("view", "view")));
        assert_eq!(r.parse("today now", ALL), Err(Error::TooMany("today")));

        let p = r
            .parse("new --calendar work --at=tomorrow Lunch with Sam", ALL)
            .unwrap();
        assert_eq!(p.flag("calendar"), Some("work"));
        assert_eq!(p.flag("at"), Some("tomorrow"));
        assert_eq!(p.arg(0), Some("Lunch with Sam"));
        assert_eq!(r.parse("new", ALL).unwrap().arg(0), None);
        assert!(matches!(
            r.parse("new --colour red", ALL),
            Err(Error::UnknownFlag(_))
        ));
        assert_eq!(r.parse("new --at", ALL), Err(Error::Missing("new", "at")));
    }

    #[test]
    fn completing_names() {
        let r = reg();
        assert_eq!(names(&r.complete(":to", ALL, &V)), ["today", "toggle"]);
        assert_eq!(
            names(&r.complete("", MONTH, &V)),
            ["new", "today", "toggle", "v", "view"]
        );
        let c = r.complete(":vi", ALL, &V);
        assert_eq!(c.apply(":vi", &c.candidates[0]), ":view ");
    }

    #[test]
    fn completing_args() {
        let r = reg();
        assert_eq!(
            names(&r.complete("view ", ALL, &V)),
            ["day", "week", "month"]
        );
        assert_eq!(names(&r.complete("view m", ALL, &V)), ["month"]);
        assert_eq!(names(&r.complete("v w", ALL, &V)), ["week"], "via alias");
        assert_eq!(
            names(&r.complete("toggle w", ALL, &V)),
            ["work", "Work stuff"]
        );
        assert_eq!(
            names(&r.complete("toggle W", ALL, &V)),
            ["Work stuff"],
            "smart case"
        );
        let c = r.complete("toggle Wo", ALL, &V);
        assert_eq!(
            c.apply("toggle Wo", &c.candidates[0]),
            "toggle \"Work stuff\" "
        );

        // flags and their values
        assert_eq!(
            names(&r.complete("new --", ALL, &V)),
            ["--calendar", "--at"]
        );
        assert_eq!(
            names(&r.complete("new --calendar ", ALL, &V)),
            ["Personal", "work", "Work stuff"]
        );
        assert_eq!(
            names(&r.complete("new --at t", ALL, &V)),
            ["today", "tomorrow"]
        );
        assert_eq!(
            names(&r.complete("new ", ALL, &V)),
            ["--calendar", "--at"],
            "free text offers flags"
        );
        // quoted word in progress
        let c = r.complete("toggle \"Work s", ALL, &V);
        assert_eq!(c.start, "toggle ".len());
    }
}

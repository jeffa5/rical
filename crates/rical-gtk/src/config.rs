//! Settings: from the config file, then the environment.
//!
//! ```text
//! # ~/.config/rical/config
//! vdir = ~/.calendars
//! week = workweek
//! remind = 10m
//! me = me@example.com, me@work.example
//! no-notifications = true
//! include = local.conf            # relative to this file
//! include-if-exists = ~/.config/rical/secret.conf
//! ```
//!
//! Lines apply in order, so a later line (including one after an
//! `include`) overrides an earlier one. Each setting can also be given in
//! the environment (`RICAL_WEEK`), which wins over the file. The command
//! line is only for files to open.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

/// One `name = value` setting and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Setting {
    pub name: String,
    pub value: String,
    pub origin: String,
    /// The folder of the file it is in, for relative paths.
    pub base: PathBuf,
}

/// `~/x` and `$HOME/x` → the home folder; relative paths are taken from
/// `base` (the folder of the file that names them).
pub fn expand(path: &str, base: &Path) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    let p = if let Some(rest) = path.strip_prefix("~/") {
        home().map(|h| h.join(rest))
    } else if let Some(rest) = path.strip_prefix("$HOME/") {
        home().map(|h| h.join(rest))
    } else {
        None
    }
    .unwrap_or_else(|| PathBuf::from(path));
    if p.is_relative() { base.join(p) } else { p }
}

/// Read the settings in `path`, following includes.
pub fn load(path: &Path) -> Result<Vec<Setting>, String> {
    let mut out = Vec::new();
    let mut open = BTreeSet::new();
    read(path, &mut out, &mut open)?;
    Ok(out)
}

fn read(path: &Path, out: &mut Vec<Setting>, open: &mut BTreeSet<PathBuf>) -> Result<(), String> {
    let real = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    if !open.insert(real.clone()) {
        return Err(format!("{}: included from itself", path.display()));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    for (i, line) in text.lines().enumerate() {
        let origin = format!("{}:{}", path.display(), i + 1);
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            return Err(format!("{origin}: expected name = value"));
        };
        let name = name.trim().to_owned();
        let value = unquote(value.trim()).to_owned();
        match name.as_str() {
            "include" | "include-if-exists" => {
                let target = expand(&value, base);
                if name == "include-if-exists" && !target.exists() {
                    continue;
                }
                read(&target, out, open).map_err(|e| format!("{e}\n  included from {origin}"))?;
            }
            _ => out.push(Setting {
                name,
                value,
                origin,
                base: base.to_owned(),
            }),
        }
    }
    open.remove(&real);
    Ok(())
}

/// Drop a `#` comment: a `#` at the start, or after a space and before a
/// space or the end, outside quotes. So `#ff0000` (a colour) is a value.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    let mut prev = ' ';
    let mut chars = line.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted
                && (line[..i].trim().is_empty()
                    || (prev.is_whitespace()
                        && chars.peek().is_none_or(|(_, n)| n.is_whitespace()))) =>
            {
                return &line[..i];
            }
            _ => {}
        }
        prev = c;
    }
    line
}

fn unquote(v: &str) -> &str {
    v.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(v)
}

/// Every setting, with what it is for, shown by `--help`.
pub const SETTINGS: &[(&str, &str)] = &[
    (
        "vdir",
        "Root of the vdir (a folder of calendar folders). Required.",
    ),
    ("date", "Day to open on, YYYY-MM-DD. Default today."),
    (
        "view",
        "View to open: day, week, workweek, fullweek, Ndays (e.g. 3days), month, agenda \
         or tasks. Default week.",
    ),
    (
        "week",
        "What the week view shows: fullweek, workweek or Ndays. Default fullweek.",
    ),
    (
        "calendar",
        "Calendar (folder name) for new events. Default the one with the most items.",
    ),
    (
        "binds",
        "Key bindings file. Default ~/.config/rical/binds.conf if it exists.",
    ),
    (
        "remind",
        "Remind this long before timed events without alarms of their own, e.g. 10m.",
    ),
    (
        "no-notifications",
        "true to leave reminders to `rical remind` (e.g. run as a service).",
    ),
    (
        "me",
        "My email addresses, comma-separated, to tell invitations to me apart. Default \
         guessed.",
    ),
    (
        "contacts",
        "A folder of vCards (e.g. a vdirsyncer address book) to suggest guests from.",
    ),
];

/// The environment variable for a setting: `RICAL_NO_NOTIFICATIONS`.
pub fn env_name(name: &str) -> String {
    format!("RICAL_{}", name.to_ascii_uppercase().replace('-', "_"))
}

/// The settings, from the config file and then the environment.
#[derive(Debug, Default)]
pub struct Options {
    pub vdir: Option<PathBuf>,
    pub date: Option<chrono::NaiveDate>,
    pub view: Option<String>,
    pub week: Option<String>,
    pub calendar: Option<String>,
    pub binds: Option<PathBuf>,
    pub remind: Option<String>,
    pub no_notifications: bool,
    pub me: Vec<String>,
    pub contacts: Option<PathBuf>,
}

impl Options {
    /// Read the config file (RICAL_CONFIG, else the default if it exists),
    /// then let the environment override it.
    pub fn load() -> Result<Self, String> {
        let mut o = Self::default();
        let path = match std::env::var_os("RICAL_CONFIG").filter(|v| !v.is_empty()) {
            Some(p) => Some(PathBuf::from(p)),
            None => default_path().filter(|p| p.exists()),
        };
        if let Some(path) = path {
            for s in load(&path)? {
                o.set(&s.name, &s.value, &s.base, &s.origin)?;
            }
        }
        let here = std::env::current_dir().unwrap_or_default();
        for (name, _) in SETTINGS {
            let var = env_name(name);
            if let Some(value) = std::env::var_os(&var).filter(|v| !v.is_empty()) {
                o.set(name, &value.to_string_lossy(), &here, &var)?;
            }
        }
        Ok(o)
    }

    fn set(&mut self, name: &str, value: &str, base: &Path, origin: &str) -> Result<(), String> {
        let path = || Some(expand(value, base));
        let text = || Some(value.to_owned());
        match name {
            "vdir" => self.vdir = path(),
            "binds" => self.binds = path(),
            "contacts" => self.contacts = path(),
            "view" => self.view = text(),
            "week" => self.week = text(),
            "calendar" => self.calendar = text(),
            "remind" => self.remind = text(),
            "me" => {
                self.me = value
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "date" => {
                self.date = Some(
                    chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                        .map_err(|_| format!("{origin}: date is YYYY-MM-DD, not “{value}”"))?,
                );
            }
            "no-notifications" => {
                self.no_notifications = match value.to_ascii_lowercase().as_str() {
                    "true" | "yes" | "on" | "1" => true,
                    "false" | "no" | "off" | "0" => false,
                    _ => return Err(format!("{origin}: {name} is true or false, not “{value}”")),
                };
            }
            _ => {
                let known = SETTINGS
                    .iter()
                    .map(|(n, _)| *n)
                    .chain(["include", "include-if-exists"])
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "{origin}: unknown setting “{name}” (known: {known})"
                ));
            }
        }
        Ok(())
    }
}

/// What `--help` says after the options: the settings and where they go.
pub fn help() -> String {
    let mut s = String::from(
        "Settings go in ~/.config/rical/config (or the file RICAL_CONFIG names) as\n\
         `name = value` lines, `include = FILE` reads another file, and each can be\n\
         set in the environment instead, which wins:\n\n",
    );
    for (name, what) in SETTINGS {
        s += &format!("  {name}  ({})\n      {what}\n", env_name(name));
    }
    s
}

/// The default config file: $XDG_CONFIG_HOME/rical/config.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|c| c.join("rical/config"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rical-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn includes_apply_in_place() {
        let d = dir("include");
        std::fs::write(
            d.join("config"),
            "# main\nweek = workweek\ninclude = more.conf\nremind = 5m  # after the include\n\
             include-if-exists = missing.conf\n",
        )
        .unwrap();
        std::fs::write(
            d.join("more.conf"),
            "remind = 10m\nme = \"a@example.com, b@example.com\"\n",
        )
        .unwrap();
        let got = load(&d.join("config")).unwrap();
        let pairs = got
            .iter()
            .map(|s| (s.name.as_str(), s.value.as_str()))
            .collect::<Vec<_>>();
        assert_eq!(
            pairs,
            [
                ("week", "workweek"),
                ("remind", "10m"),
                ("me", "a@example.com, b@example.com"),
                ("remind", "5m"),
            ]
        );
        assert!(got[1].origin.ends_with("more.conf:1"));
    }

    #[test]
    fn include_errors() {
        let d = dir("errors");
        std::fs::write(d.join("a"), "include = b\n").unwrap();
        std::fs::write(d.join("b"), "include = a\n").unwrap();
        assert!(
            load(&d.join("a"))
                .unwrap_err()
                .contains("included from itself")
        );
        std::fs::write(d.join("c"), "include = nope\n").unwrap();
        assert!(load(&d.join("c")).unwrap_err().contains("included from"));
        std::fs::write(d.join("d"), "just words\n").unwrap();
        assert!(
            load(&d.join("d"))
                .unwrap_err()
                .contains("d:1: expected name = value")
        );
    }

    #[test]
    fn comments_keep_colours() {
        assert_eq!(strip_comment("color = #ff0000 # red"), "color = #ff0000 ");
        assert_eq!(strip_comment("color = #ff0000"), "color = #ff0000");
        assert_eq!(strip_comment("# all"), "");
        assert_eq!(strip_comment("  #indented"), "  ");
        assert_eq!(strip_comment("name = \"a # b\""), "name = \"a # b\"");
    }
}

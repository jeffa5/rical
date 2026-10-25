//! The config file: every command-line flag can be set there instead, as
//! `name = value` lines using the flag's long name.
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
//! `include`) overrides an earlier one. Flags given on the command line
//! win over environment variables, which win over the file.

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

/// Arguments from the file, and the positional value with its origin.
pub type FileArgs = (Vec<String>, Option<(String, String)>);

/// The settings as command-line arguments for `cmd`, to go before the real
/// ones. Unknown names are an error; settings whose flag has an
/// environment variable that is set are skipped, as the environment wins;
/// `positional` names (the vdir) are returned apart.
pub fn to_args(
    settings: &[Setting],
    cmd: &clap::Command,
    positional: &str,
) -> Result<FileArgs, String> {
    let mut args = Vec::new();
    let mut pos = None;
    for s in settings {
        if s.name == positional {
            pos = Some((s.value.clone(), s.origin.clone()));
            continue;
        }
        let Some(arg) = cmd
            .get_arguments()
            .find(|a| a.get_long() == Some(s.name.as_str()))
        else {
            let known = cmd
                .get_arguments()
                .filter_map(|a| a.get_long())
                .filter(|l| !matches!(*l, "help" | "config"))
                .chain([positional, "include", "include-if-exists"])
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "{}: unknown setting “{}” (known: {known})",
                s.origin, s.name
            ));
        };
        if arg
            .get_env()
            .is_some_and(|e| std::env::var_os(e).is_some_and(|v| !v.is_empty()))
        {
            continue;
        }
        let takes_value = arg.get_action().takes_values();
        if takes_value {
            args.push(format!("--{}={}", s.name, s.value));
        } else {
            match s.value.to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => args.push(format!("--{}", s.name)),
                "false" | "no" | "off" | "0" => {}
                _ => {
                    return Err(format!(
                        "{}: {} is true or false, not “{}”",
                        s.origin, s.name, s.value
                    ));
                }
            }
        }
    }
    Ok((args, pos))
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

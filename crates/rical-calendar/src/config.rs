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
    (
        "zones",
        "Extra time zones beside the hours, comma-separated: America/New_York, or \
         NYC=America/New_York with a label.",
    ),
    (
        "renderer",
        "How GTK draws: cairo (on the CPU, the least memory), gl, vulkan or auto (GTK's \
         choice). Default cairo. GSK_RENDERER, if set, wins.",
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
    pub zones: Vec<crate::zones::Zone>,
    /// GSK_RENDERER to start GTK with; None leaves it to GTK.
    pub renderer: Option<String>,
}

impl Options {
    /// Read the config file (RICAL_CONFIG, else the default if it exists),
    /// then let the environment override it.
    pub fn load() -> Result<Self, String> {
        let mut o = Self {
            renderer: Some("cairo".to_owned()),
            ..Self::default()
        };
        // a config named in RICAL_CONFIG must be there; the default may not be
        let named = std::env::var_os("RICAL_CONFIG")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        let required = named.is_some();
        if let Some(path) = named.or_else(default_path) {
            for s in load_all(&path, required)? {
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
            "zones" => {
                self.zones =
                    crate::zones::Zone::parse_list(value).map_err(|e| format!("{origin}: {e}"))?;
            }
            "renderer" => {
                self.renderer = match value.to_ascii_lowercase().as_str() {
                    "auto" => None,
                    r @ ("cairo" | "gl" | "ngl" | "vulkan") => Some(r.to_owned()),
                    _ => {
                        return Err(format!(
                            "{origin}: renderer is cairo, gl, vulkan or auto, not “{value}”"
                        ));
                    }
                };
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

/// Where a setting changed from the GUI was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Saved {
    /// In the config file itself.
    InPlace(PathBuf),
    /// The config file can't be written (e.g. Nix made it), so in
    /// `local.conf` beside it, which is read after it.
    Local(PathBuf),
}

/// Settings that override the config file: where the GUI saves when the
/// config can't be written, read after it whenever it exists.
pub fn local_path(config: &Path) -> PathBuf {
    config.with_file_name("local.conf")
}

/// The settings in `config` (which must exist if `required`), then those
/// in [`local_path`] unless the config already includes it.
pub fn load_all(config: &Path, required: bool) -> Result<Vec<Setting>, String> {
    let mut out = if required || config.exists() {
        load(config)?
    } else {
        Vec::new()
    };
    let local = local_path(config);
    if local.exists() && !includes(config).iter().any(|p| same_file(p, &local)) {
        out.extend(load(&local)?);
    }
    Ok(out)
}

/// Set `name` to `value` (None removes it) in `config`, keeping the rest
/// of the file as it is: comments, includes and other settings. If
/// `config` is read-only, the change goes to [`local_path`] instead.
pub fn save(config: &Path, name: &str, value: Option<&str>) -> Result<Saved, String> {
    if writable(config) {
        edit_file(config, name, value)?;
        // local.conf, read after, would undo it
        let local = local_path(config);
        if local.exists() {
            edit_file(&local, name, None)?;
        }
        return Ok(Saved::InPlace(config.to_owned()));
    }
    let file = local_path(config);
    edit_file(&file, name, value)?;
    Ok(Saved::Local(file))
}

/// Whether `path` can be changed: written if it exists (not a read-only
/// file or a link into the Nix store), else created.
pub fn writable(path: &Path) -> bool {
    match std::fs::symlink_metadata(path) {
        Ok(_) => std::fs::OpenOptions::new().append(true).open(path).is_ok(),
        Err(_) => path.parent().is_some_and(|dir| {
            std::fs::create_dir_all(dir).is_ok()
                && std::fs::metadata(dir).is_ok_and(|m| !m.permissions().readonly())
        }),
    }
}

/// The files `config` includes directly (whether or not they exist).
fn includes(config: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(config) else {
        return Vec::new();
    };
    let base = config.parent().unwrap_or(Path::new("."));
    text.lines()
        .filter_map(|line| {
            let (name, value) = strip_comment(line).trim().split_once('=')?;
            matches!(name.trim(), "include" | "include-if-exists")
                .then(|| expand(unquote(value.trim()), base))
        })
        .collect()
}

fn same_file(a: &Path, b: &Path) -> bool {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_owned());
    real(a) == real(b)
}

/// `name = value` as a line, quoted if it would otherwise read differently.
fn line_for(name: &str, value: &str) -> String {
    let plain = !value.is_empty()
        && value.trim() == value
        && strip_comment(&format!("x = {value}")) == format!("x = {value}")
        && !(value.starts_with('"') && value.ends_with('"'));
    if plain {
        format!("{name} = {value}")
    } else {
        format!("{name} = \"{value}\"")
    }
}

/// Change `name`'s line in `path` (creating the file if need be): the last
/// one is replaced, keeping a trailing comment, and earlier ones (which it
/// overrode) left alone; with no line, one is added at the end. Removing
/// drops every line for it.
pub(crate) fn edit_file(path: &Path, name: &str, value: Option<&str>) -> Result<(), String> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let key_of = |line: &str| {
        let (k, _) = strip_comment(line).split_once('=')?;
        Some(k.trim().to_owned())
    };
    let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
    let mine: Vec<usize> = (0..lines.len())
        .filter(|&i| key_of(&lines[i]).as_deref() == Some(name))
        .collect();
    match (value, mine.last()) {
        (Some(v), Some(&i)) => {
            let comment = lines[i][strip_comment(&lines[i]).len()..].to_owned();
            let comment = if comment.is_empty() {
                comment
            } else {
                format!("  {}", comment.trim_start())
            };
            lines[i] = format!("{}{comment}", line_for(name, v));
        }
        (Some(v), None) => lines.push(line_for(name, v)),
        (None, _) => {
            for i in mine.into_iter().rev() {
                lines.remove(i);
            }
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // whole or not at all: a temporary file renamed over it
    let tmp = path.with_extension("rical-tmp");
    std::fs::write(&tmp, out).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The config file in use: RICAL_CONFIG, else the default.
pub fn config_path() -> Option<PathBuf> {
    std::env::var_os("RICAL_CONFIG")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(default_path)
}

/// Each setting's value now and where it comes from: the last line for it
/// in the config (and its includes), or the environment, which wins.
pub fn current(config: Option<&Path>) -> std::collections::HashMap<String, (String, Source)> {
    let mut out = std::collections::HashMap::new();
    if let Some(path) = config
        && let Ok(settings) = load_all(path, false)
    {
        for s in settings {
            let file = s
                .origin
                .rsplit_once(':')
                .map_or(s.origin.clone(), |(f, _)| f.to_owned());
            out.insert(s.name, (s.value, Source::File(PathBuf::from(file))));
        }
    }
    for (name, _) in SETTINGS {
        let var = env_name(name);
        if let Some(v) = std::env::var_os(&var).filter(|v| !v.is_empty()) {
            out.insert(
                (*name).to_owned(),
                (v.to_string_lossy().into_owned(), Source::Env(var)),
            );
        }
    }
    out
}

/// Where a setting's value comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    /// The environment variable, which no file can override.
    Env(String),
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

    #[test]
    fn saving_keeps_the_rest_of_the_file() {
        let d = dir("save");
        let config = d.join("config");
        std::fs::write(
            &config,
            "# my settings\nweek = workweek  # weekdays only\ninclude-if-exists = more.conf\nremind = 5m\n",
        )
        .unwrap();
        assert_eq!(
            save(&config, "week", Some("fullweek")),
            Ok(Saved::InPlace(config.clone()))
        );
        save(&config, "zones", Some("UK=Europe/London")).unwrap();
        save(&config, "remind", None).unwrap();
        save(&config, "me", Some("a # b")).unwrap();
        assert_eq!(
            std::fs::read_to_string(&config).unwrap(),
            "# my settings\nweek = fullweek  # weekdays only\ninclude-if-exists = more.conf\n\
             zones = UK=Europe/London\nme = \"a # b\"\n"
        );
        // and it reads back as written
        let got = load(&config).unwrap();
        assert!(got.iter().any(|s| s.name == "me" && s.value == "a # b"));
        assert!(
            got.iter()
                .any(|s| s.name == "week" && s.value == "fullweek")
        );
    }

    #[test]
    fn a_read_only_config_saves_to_local_conf() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("readonly");
        // like a Home Manager file: a link to a read-only file elsewhere
        let store = d.join("store-config");
        std::fs::write(&store, "week = workweek\nremind = 5m\n").unwrap();
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o444)).unwrap();
        let config = d.join("config");
        std::os::unix::fs::symlink(&store, &config).unwrap();

        let local = d.join("local.conf");
        assert_eq!(
            save(&config, "week", Some("3days")),
            Ok(Saved::Local(local.clone()))
        );
        assert_eq!(
            std::fs::read_to_string(&store).unwrap(),
            "week = workweek\nremind = 5m\n"
        );
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "week = 3days\n");

        // read after the config, so it wins, and the rest still applies
        let all = load_all(&config, true).unwrap();
        let last = |name: &str| all.iter().rfind(|s| s.name == name).unwrap().value.clone();
        assert_eq!(last("week"), "3days");
        assert_eq!(last("remind"), "5m");

        // a config that includes it itself doesn't read it twice
        std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::write(&store, "include-if-exists = local.conf\nweek = workweek\n").unwrap();
        let all = load_all(&config, true).unwrap();
        assert_eq!(all.iter().filter(|s| s.name == "week").count(), 2);
        assert_eq!(all.last().unwrap().value, "workweek");
    }
}

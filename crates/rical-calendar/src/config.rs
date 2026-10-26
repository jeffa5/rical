//! The calendar's settings: from the config file, then the environment.
//! The files themselves (format, includes, saving) are `rical_ui::config`.
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

use std::path::{Path, PathBuf};

pub use rical_ui::config::*;

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
        let named = std::env::var_os(env_name("config"))
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

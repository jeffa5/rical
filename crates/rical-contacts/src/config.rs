//! Contacts' settings: from `~/.config/rical/contacts/config` (then its
//! `local.conf`), then the environment (`RICAL_CONTACTS_…`). The files
//! themselves are `rical_ui::config`.

use std::path::PathBuf;

pub use rical_ui::config::*;

/// Every setting, with what it is for, shown by `--help`.
pub const SETTINGS: &[(&str, &str)] = &[
    (
        "contacts",
        "The folder of address books (folders of .vcf files, e.g. vdirsyncer's). \
         Default ~/contacts if it exists.",
    ),
    (
        "book",
        "The address book new contacts go in (folder or display name). Default the one \
         with the most contacts.",
    ),
    (
        "sort",
        "How the list is sorted: name (as shown, the default) or family (by family name, then given).",
    ),
    (
        "binds",
        "Key bindings file. Default ~/.config/rical/contacts/binds.conf if it exists.",
    ),
    (
        "renderer",
        "How GTK draws: cairo (on the CPU, the least memory), gl, vulkan or auto. \
         Default cairo. GSK_RENDERER, if set, wins.",
    ),
];

/// The settings, from the config file and then the environment.
#[derive(Debug, Default)]
pub struct Options {
    pub contacts: Option<PathBuf>,
    pub book: Option<String>,
    pub sort: Option<String>,
    pub binds: Option<PathBuf>,
    pub renderer: Option<String>,
}

impl Options {
    pub fn load() -> Result<Self, String> {
        let mut o = Self {
            renderer: Some("cairo".to_owned()),
            ..Self::default()
        };
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
        if o.contacts.is_none() {
            o.contacts = std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("contacts"))
                .filter(|p| p.is_dir());
        }
        Ok(o)
    }

    fn set(
        &mut self,
        name: &str,
        value: &str,
        base: &std::path::Path,
        origin: &str,
    ) -> Result<(), String> {
        match name {
            "contacts" => self.contacts = Some(expand(value, base)),
            "binds" => self.binds = Some(expand(value, base)),
            "book" => self.book = Some(value.to_owned()),
            "sort" => match value.to_ascii_lowercase().as_str() {
                "name" | "family" => self.sort = Some(value.to_ascii_lowercase()),
                _ => return Err(format!("{origin}: sort is name or family, not “{value}”")),
            },
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

/// What `--help` says after the options.
pub fn help() -> String {
    let mut s = String::from(
        "Settings go in ~/.config/rical/contacts/config (or the file \
         RICAL_CONTACTS_CONFIG names) as\n`name = value` lines, and each can be set \
         in the environment instead, which wins:\n\n",
    );
    for (name, what) in SETTINGS {
        s += &format!("  {name}  ({})\n      {what}\n", env_name(name));
    }
    s
}

/// The `sort` setting as an order.
pub fn sort_by(value: Option<&str>) -> rical_cards::search::SortBy {
    match value {
        Some("family") => rical_cards::search::SortBy::Family,
        _ => rical_cards::search::SortBy::Shown,
    }
}

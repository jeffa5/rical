//! Mail's settings: from `~/.config/rical-mailbox/config` (then its
//! `local.conf`), then the environment (`RICAL_MAILBOX_…`). The files
//! themselves are `rical_ui::config`.

use std::path::PathBuf;

use rical_maildir::addr::{self, Mailbox};
pub use rical_ui::config::*;

/// Every setting, with what it is for, shown by `--help`.
pub const SETTINGS: &[(&str, &str)] = &[
    (
        "mail",
        "The folder of Maildir folders (what mbsync or offlineimap keeps). \
         Default ~/mail or ~/Maildir, whichever exists.",
    ),
    (
        "me",
        "Your addresses, as `Name <address>` separated by commas. The first is what \
         new mail is from; replies are from the one they were sent to.",
    ),
    (
        "contacts",
        "The folder of address books (vCards) that addresses are completed from, and \
         people are opened in. Default ~/contacts if it exists.",
    ),
    (
        "queue",
        "The folder rical-mail sends from. Sending puts a message there. \
         Default ~/.local/share/rical-mail/outbox.",
    ),
    (
        "send",
        "A command run after a message is queued, so it goes at once: for example \
         `rical-mail run --once`. Default none: your rical-mail service sends it.",
    ),
    (
        "sync",
        "The command that fetches mail, run by :sync (for example `mbsync -a`). Default none.",
    ),
    (
        "drafts",
        "The folder drafts are kept in (its name). Default the one called Drafts.",
    ),
    (
        "archive",
        "The folder :archive moves to. Default the one called Archive.",
    ),
    (
        "wrap",
        "The column plain text is wrapped at when writing. Default 72.",
    ),
    (
        "auto-wrap",
        "Break lines at the wrap column as you type. Default true.",
    ),
    (
        "reply",
        "Where a reply starts: bottom (below the quote, the default) or top.",
    ),
    (
        "markdown",
        "Write new messages in Markdown: sent as written, with an HTML alternative. \
         Toggle per message with :markdown. Default false.",
    ),
    (
        "signature",
        "A file with your signature, or the text itself. Added after `-- `.",
    ),
    (
        "editor",
        "The command that edits a message in your own editor (:edit-external), given \
         the file; for example `foot nvim`. Default: $VISUAL or $EDITOR in $TERMINAL.",
    ),
    (
        "calendar",
        "The command that opens an invitation (it gets the .ics file). Default rical-calendar.",
    ),
    (
        "contacts-app",
        "The command that opens a person (it gets a .vcf file). Default rical-contacts.",
    ),
    ("threads", "Show messages in conversations. Default true."),
    (
        "read-after",
        "How long a message is on screen before it counts as read: 0s, 2s… or never. Default 1s.",
    ),
    (
        "font",
        "How messages are drawn: monospace (the default) or proportional.",
    ),
    (
        "binds",
        "Key bindings file. Default ~/.config/rical-mailbox/binds.conf if it exists.",
    ),
    (
        "renderer",
        "How GTK draws: cairo (on the CPU, the least memory), gl, vulkan or auto. \
         Default cairo. GSK_RENDERER, if set, wins.",
    ),
];

/// The settings, from the config file and then the environment.
#[derive(Debug, Clone)]
pub struct Options {
    pub mail: Option<PathBuf>,
    pub me: Vec<Mailbox>,
    pub contacts: Option<PathBuf>,
    pub queue: PathBuf,
    pub send: Option<String>,
    pub sync: Option<String>,
    pub drafts: Option<String>,
    pub archive: Option<String>,
    pub wrap: usize,
    pub auto_wrap: bool,
    pub markdown: bool,
    pub reply_top: bool,
    pub signature: Option<String>,
    pub editor: Option<String>,
    pub calendar: String,
    pub contacts_app: String,
    pub threads: bool,
    /// Seconds before a message counts as read; None never.
    pub read_after: Option<u32>,
    pub proportional: bool,
    pub binds: Option<PathBuf>,
    pub renderer: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(".local/share")))
            .unwrap_or_default();
        Self {
            mail: None,
            me: Vec::new(),
            contacts: None,
            queue: data.join("rical-mail/outbox"),
            send: None,
            sync: None,
            drafts: None,
            archive: None,
            wrap: 72,
            auto_wrap: true,
            markdown: false,
            reply_top: false,
            signature: None,
            editor: None,
            calendar: "rical-calendar".to_owned(),
            contacts_app: "rical-contacts".to_owned(),
            threads: true,
            read_after: Some(1),
            proportional: false,
            binds: None,
            renderer: Some("cairo".to_owned()),
        }
    }
}

fn flag(value: &str, origin: &str) -> Result<bool, String> {
    match value.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(format!("{origin}: true or false, not “{value}”")),
    }
}

impl Options {
    pub fn load() -> Result<Self, String> {
        let mut o = Self::default();
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
        let home = std::env::var_os("HOME").map(PathBuf::from);
        if o.mail.is_none() {
            o.mail = home
                .as_ref()
                .into_iter()
                .flat_map(|h| [h.join("mail"), h.join("Maildir")])
                .find(|p| p.is_dir());
        }
        if o.contacts.is_none() {
            o.contacts = home.map(|h| h.join("contacts")).filter(|p| p.is_dir());
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
            "mail" => self.mail = Some(expand(value, base)),
            "contacts" => self.contacts = Some(expand(value, base)),
            "queue" => self.queue = expand(value, base),
            "binds" => self.binds = Some(expand(value, base)),
            "me" => {
                self.me = addr::parse_list(value);
                if self.me.is_empty() {
                    return Err(format!(
                        "{origin}: me is addresses like Name <you@example.com>"
                    ));
                }
            }
            "send" => self.send = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "sync" => self.sync = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "drafts" => self.drafts = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "archive" => self.archive = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "signature" => self.signature = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "editor" => self.editor = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "calendar" => self.calendar = value.to_owned(),
            "contacts-app" => self.contacts_app = value.to_owned(),
            "wrap" => {
                self.wrap = value
                    .parse::<usize>()
                    .ok()
                    .filter(|w| (30..=200).contains(w))
                    .ok_or(format!(
                        "{origin}: wrap is a column from 30 to 200, not “{value}”"
                    ))?
            }
            "auto-wrap" => self.auto_wrap = flag(value, origin)?,
            "markdown" => self.markdown = flag(value, origin)?,
            "threads" => self.threads = flag(value, origin)?,
            "reply" => match value.to_ascii_lowercase().as_str() {
                "top" => self.reply_top = true,
                "bottom" => self.reply_top = false,
                _ => return Err(format!("{origin}: reply is top or bottom, not “{value}”")),
            },
            "read-after" => {
                self.read_after = if value.eq_ignore_ascii_case("never") {
                    None
                } else {
                    Some(value.trim_end_matches('s').parse().map_err(|_| {
                        format!("{origin}: read-after is 0s, 2s… or never, not “{value}”")
                    })?)
                }
            }
            "font" => match value.to_ascii_lowercase().as_str() {
                "monospace" => self.proportional = false,
                "proportional" => self.proportional = true,
                _ => {
                    return Err(format!(
                        "{origin}: font is monospace or proportional, not “{value}”"
                    ));
                }
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
        "Settings go in ~/.config/rical-mailbox/config (or the file \
         RICAL_MAILBOX_CONFIG names) as\n`name = value` lines, and each can be set \
         in the environment instead, which wins:\n\n",
    );
    for (name, what) in SETTINGS {
        s += &format!("  {name}  ({})\n      {what}\n", env_name(name));
    }
    s
}

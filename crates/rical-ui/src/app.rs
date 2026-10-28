//! Which app is running, for the files it keeps: settings side by side
//! under `~/.config/rical`, and its log, state and history in folders of
//! its own (`~/.local/state/rical-calendar`, `~/.local/share/rical-calendar`).

use std::{path::PathBuf, sync::OnceLock};

/// An app's identity and where its files go.
#[derive(Debug, Clone, Copy)]
pub struct App {
    /// The application ID, also its icon's name.
    pub id: &'static str,
    /// What it is called everyday ("Calendar").
    pub name: &'static str,
    /// What it is called beside others ("Calendar (rical)").
    pub full_name: &'static str,
    /// The prefix of its environment variables (`RICAL_`).
    pub env_prefix: &'static str,
    /// Its folder under `~/.config/rical`, or none for the calendar, which
    /// came first and keeps its settings there.
    pub folder: Option<&'static str>,
    /// Its own folder in `~/.local/state` and `~/.local/share`, as the
    /// program is called (`rical-calendar`).
    pub dir: &'static str,
}

static APP: OnceLock<App> = OnceLock::new();

/// Say which app this is; first thing in `main`.
pub fn init(app: App) {
    let _ = APP.set(app);
    move_old_state();
}

/// The app running (the calendar if none was set, as in tests).
pub fn get() -> App {
    *APP.get().unwrap_or(&App {
        id: "net.jeffas.Rical",
        name: "Calendar",
        full_name: "Calendar (rical)",
        env_prefix: "RICAL_",
        folder: None,
        dir: "rical-calendar",
    })
}

fn under(base: Option<PathBuf>) -> Option<PathBuf> {
    let dir = base?.join("rical");
    Some(match get().folder {
        Some(f) => dir.join(f),
        None => dir,
    })
}

/// The app's settings folder: `$XDG_CONFIG_HOME/rical[/folder]`.
pub fn config_dir() -> Option<PathBuf> {
    under(
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))),
    )
}

fn xdg(var: &str, home_default: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(home_default)))
}

/// The app's state folder (log, remembered state): `$XDG_STATE_HOME/<dir>`,
/// e.g. `~/.local/state/rical-calendar`.
pub fn state_dir() -> Option<PathBuf> {
    Some(xdg("XDG_STATE_HOME", ".local/state")?.join(get().dir))
}

/// The app's data folder (its history): `$XDG_DATA_HOME/<dir>`, e.g.
/// `~/.local/share/rical-calendar`.
pub fn data_dir() -> Option<PathBuf> {
    Some(xdg("XDG_DATA_HOME", ".local/share")?.join(get().dir))
}

/// Move the files the app kept in its old state folder (`rical`, and
/// `rical/contacts` for contacts) to its own, once: when the new one
/// doesn't exist yet. Folders are left (the calendar's old one holds
/// contacts'), and so is anything that can't be moved.
fn move_old_state() {
    let (Some(new), Some(old)) = (state_dir(), under(xdg("XDG_STATE_HOME", ".local/state"))) else {
        return;
    };
    if new.exists() || !old.is_dir() {
        return;
    }
    if std::fs::create_dir_all(&new).is_err() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(&old) else {
        return;
    };
    for e in entries.flatten() {
        if e.file_type().is_ok_and(|t| t.is_file()) {
            let _ = std::fs::rename(e.path(), new.join(e.file_name()));
        }
    }
    // the old folder, and the `rical` one above contacts', if now empty
    if std::fs::remove_dir(&old).is_ok()
        && get().folder.is_some()
        && let Some(parent) = old.parent()
    {
        let _ = std::fs::remove_dir(parent);
    }
}

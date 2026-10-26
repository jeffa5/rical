//! Which app is running, for the files it keeps: each rical app has its own
//! settings, log and remembered state, side by side under `rical`.

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
    /// Its folder under `~/.config/rical` and `~/.local/state/rical`, or
    /// none for the calendar, which came first and keeps its files there.
    pub folder: Option<&'static str>,
}

static APP: OnceLock<App> = OnceLock::new();

/// Say which app this is; first thing in `main`.
pub fn init(app: App) {
    let _ = APP.set(app);
}

/// The app running (the calendar if none was set, as in tests).
pub fn get() -> App {
    *APP.get().unwrap_or(&App {
        id: "net.jeffas.Rical",
        name: "Calendar",
        full_name: "Calendar (rical)",
        env_prefix: "RICAL_",
        folder: None,
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

/// The app's state folder (log, remembered state):
/// `$XDG_STATE_HOME/rical[/folder]`.
pub fn state_dir() -> Option<PathBuf> {
    under(
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state"))),
    )
}

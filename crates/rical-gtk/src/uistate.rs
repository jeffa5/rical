//! What rical remembers between runs that isn't a setting: how the window
//! was left (time zones folded away, say). Kept in the XDG state folder as
//! `name = value` lines, next to the log.

use std::path::PathBuf;

/// `$XDG_STATE_HOME/rical`, by default `~/.local/state/rical`.
pub fn dir() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))
        .map(|d| d.join("rical"))
}

fn file() -> Option<PathBuf> {
    dir().map(|d| d.join("state"))
}

/// The value remembered for `name`, if any.
pub fn get(name: &str) -> Option<String> {
    let path = file().filter(|p| p.exists())?;
    crate::config::load(&path)
        .ok()?
        .into_iter()
        .rfind(|s| s.name == name)
        .map(|s| s.value)
}

/// Remember `value` for `name`. Failing to is not worth bothering anyone
/// about: it is logged.
pub fn set(name: &str, value: &str) {
    let Some(path) = file() else { return };
    if let Err(e) = crate::config::edit_file(&path, name, Some(value)) {
        log::warn!("could not remember {name}: {e}");
    }
}

/// Whether the extra time zones were left showing (they are by default).
pub fn zones_shown() -> bool {
    get("zones-shown").is_none_or(|v| v != "false")
}

pub fn set_zones_shown(shown: bool) {
    set("zones-shown", if shown { "true" } else { "false" });
}

//! What rical remembers between runs that isn't a setting: how the window
//! was left (time zones folded away, say). Kept in the XDG state folder as
//! `name = value` lines, next to the log.

use std::path::PathBuf;

/// The app's state folder: `$XDG_STATE_HOME/rical[/app]`, by default
/// `~/.local/state/rical-calendar` for the calendar.
pub fn dir() -> Option<PathBuf> {
    crate::app::state_dir()
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

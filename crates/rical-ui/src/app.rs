//! Which app is running, for the files it keeps, each in folders of its
//! own named as the program is: settings in `~/.config/rical-calendar`,
//! the log and state in `~/.local/state/rical-calendar`, the history in
//! `~/.local/share/rical-calendar`.

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
    /// Where its settings and state were before it had folders of its own:
    /// `contacts` under `~/.config/rical`, or none for the calendar,
    /// whose were in `rical` itself. Read to move them.
    pub folder: Option<&'static str>,
    /// Its own folder in `~/.config`, `~/.local/state` and
    /// `~/.local/share`, named as the program is (`rical-calendar`).
    pub dir: &'static str,
}

static APP: OnceLock<App> = OnceLock::new();

/// Say which app this is; first thing in `main`.
pub fn init(app: App) {
    let _ = APP.set(app);
    move_old_files();
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

/// The app's settings folder: `$XDG_CONFIG_HOME/<dir>`, e.g.
/// `~/.config/rical-calendar`.
pub fn config_dir() -> Option<PathBuf> {
    Some(xdg("XDG_CONFIG_HOME", ".config")?.join(get().dir))
}

/// Where the app's settings were before they had a folder of its own:
/// `~/.config/rical` (`rical/contacts` for contacts).
fn old_config_dir() -> Option<PathBuf> {
    under(xdg("XDG_CONFIG_HOME", ".config"))
}

/// A settings file (`config`, `binds.conf`): in the app's folder, unless
/// only the old folder has it, as when Home Manager linked it there and
/// hasn't been switched since.
pub fn config_file(name: &str) -> Option<PathBuf> {
    let new = config_dir()?.join(name);
    if !new.exists()
        && let Some(old) = old_config_dir()
            .map(|d| d.join(name))
            .filter(|p| p.exists())
    {
        return Some(old);
    }
    Some(new)
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

/// Move the app's files from where they were before it had folders of its
/// own (`~/.config/rical`, `~/.local/state/rical`, and their `contacts`
/// folders for contacts), each unless the new folder already has one by
/// that name. While an old folder holds links (Home Manager's), it is left
/// whole, so a `local.conf` stays beside the config it adds to: once a
/// switch with the new paths has removed them, what is left moves on the
/// next start. Folders are left (the calendar's old ones hold contacts'),
/// and old folders left empty are removed.
fn move_old_files() {
    let pairs = [
        (state_dir(), under(xdg("XDG_STATE_HOME", ".local/state"))),
        (config_dir(), old_config_dir()),
    ];
    for (new, old) in pairs {
        let (Some(new), Some(old)) = (new, old) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&old) else {
            continue;
        };
        let entries: Vec<_> = entries.flatten().collect();
        if entries
            .iter()
            .any(|e| e.file_type().is_ok_and(|t| t.is_symlink()))
        {
            log::info!("{old:?} has links (Home Manager's?): read there until they go");
            continue;
        }
        for e in entries {
            let to = new.join(e.file_name());
            if e.file_type().is_ok_and(|t| t.is_file())
                && !to.exists()
                && std::fs::create_dir_all(&new).is_ok()
            {
                let _ = std::fs::rename(e.path(), to);
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
}

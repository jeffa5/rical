//! What rical's GTK apps (the calendar and contacts) share: the `:command`
//! engine and its key bindings, settings files and Preferences, logging,
//! remembered window state, and widgets and drawing helpers, so the apps
//! look and work alike.

pub mod app;
pub mod binds;
pub mod card;
pub mod command;
pub mod config;
pub mod datetime;
pub mod draw;
pub mod help;
pub mod history;
pub mod keys;
pub mod listhead;
pub mod logfile;
pub mod palette;
pub mod prefs;
pub mod section;
pub mod uistate;
pub mod util;

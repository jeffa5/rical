use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adw::prelude::*;
use chrono::{NaiveDate, Utc};
use clap::Parser as _;
use gtk::glib;
use rical_core::{Vdir, when::local_timezone};

mod agenda;
mod binds;
mod cmds;
mod command;
mod details;
mod draw;
mod editor;
mod format;
mod grid;
mod help;
mod layout;
mod listhead;
mod month;

mod state;
mod tasks;
mod window;

const APP_ID: &str = "net.jeffas.Rical";

#[derive(Debug, clap::Parser)]
#[clap(about = "Calendar for a vdir of .ics files")]
struct Args {
    /// Root of the vdir (a folder of calendar folders).
    #[clap(env = "RICAL_VDIR")]
    vdir: PathBuf,

    /// Day to open on (YYYY-MM-DD), default today.
    #[clap(long)]
    date: Option<NaiveDate>,

    /// View to open: day, week, workweek, fullweek, Ndays (e.g. 3days),
    /// month, agenda or tasks.
    #[clap(long, default_value = "week")]
    view: String,

    /// What the week view (w, the Week button) shows: fullweek, workweek or
    /// Ndays, e.g. 3days.
    #[clap(long, env = "RICAL_WEEK", default_value = "fullweek")]
    week: String,

    /// Calendar (folder name) for new events; default is the one with the
    /// most items.
    #[clap(long, env = "RICAL_CALENDAR")]
    calendar: Option<String>,

    /// Key bindings file; default ~/.config/rical/binds.conf if it exists.
    #[clap(long)]
    binds: Option<PathBuf>,

    /// Remind this long before timed events that have no alarm of their own,
    /// e.g. 10m. Alarms in the events themselves always remind.
    #[clap(long, env = "RICAL_REMIND")]
    remind: Option<String>,

    /// Do not show reminders (e.g. when `rical remind` runs as a service).
    #[clap(long, env = "RICAL_NO_NOTIFICATIONS")]
    no_notifications: bool,
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Warn)
        .parse_default_env()
        .init();
    let args = Args::parse();

    let local = local_timezone();
    let vdir = match Vdir::load(&args.vdir) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("rical-gtk: {e:#}");
            return glib::ExitCode::FAILURE;
        }
    };
    let date = args
        .date
        .unwrap_or_else(|| Utc::now().with_timezone(&local).date_naive());
    let Some(week_view) = layout::ViewKind::from_name(&args.week).filter(|v| {
        matches!(
            v,
            layout::ViewKind::Week | layout::ViewKind::WorkWeek | layout::ViewKind::Days(_)
        )
    }) else {
        eprintln!(
            "rical-gtk: --week: {:?} is not fullweek, workweek or Ndays",
            args.week
        );
        return glib::ExitCode::FAILURE;
    };
    let view = if args.view == "week" {
        Some(week_view)
    } else {
        layout::ViewKind::from_name(&args.view)
    };
    let Some(view) = view else {
        eprintln!("rical-gtk: unknown view {:?}", args.view);
        return glib::ExitCode::FAILURE;
    };

    let mut state = state::State::new(vdir, local, date);
    state.view = view;
    state.week_view = week_view;
    state.last_calendar = args.calendar;
    state.notifications = !args.no_notifications;
    if let Some(text) = &args.remind {
        match rical_core::input::parse_shift(text) {
            Some(d) if d > chrono::TimeDelta::zero() => state.default_reminder = Some(d),
            _ => {
                eprintln!("rical-gtk: --remind: not a duration: {text:?}");
                return glib::ExitCode::FAILURE;
            }
        }
    }
    let state = Rc::new(RefCell::new(state));

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let quit = gtk::gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| a.quit());
    app.add_action(&quit);

    let binds = match load_binds(args.binds.as_deref()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("rical-gtk: {e}");
            return glib::ExitCode::FAILURE;
        }
    };

    let windows = Rc::new(RefCell::new(Vec::new()));
    app.connect_activate(move |app| {
        let window = window::Window::new(app, state.clone(), binds.clone());
        window.present();
        windows.borrow_mut().push(window);
    });

    // arguments are ours, not GTK's
    app.run_with_args(&[std::env::args().next().unwrap_or_default()])
}

/// The default bindings with the user's on top.
fn load_binds(path: Option<&std::path::Path>) -> Result<binds::Binds, String> {
    let mut binds =
        binds::Binds::parse(binds::DEFAULTS).map_err(|e| format!("built-in binds: {e}"))?;
    let path = match path {
        Some(p) => Some(p.to_owned()),
        None => std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|c| c.join("rical/binds.conf"))
            .filter(|p| p.exists()),
    };
    if let Some(path) = path {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
        binds.merge(binds::Binds::parse(&text).map_err(|e| format!("{path:?}: {e}"))?);
    }
    Ok(binds)
}

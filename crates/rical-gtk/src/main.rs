use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adw::prelude::*;
use chrono::{NaiveDate, Utc};
use gtk::glib;
use rical_core::{Vdir, when::local_timezone};

mod agenda;
mod binds;
mod cmds;
mod colors;
mod command;
mod config;
mod details;
mod draw;
mod editor;
mod format;
mod grid;
mod guests;
mod help;
mod layout;
mod listhead;
mod minimonth;
mod month;
mod raw;
mod repeat;

mod section;
mod sidetasks;
mod state;
mod tasks;
mod window;

const APP_ID: &str = "net.jeffas.Rical";

#[derive(Debug, clap::Parser)]
#[clap(
    about = "Calendar for a vdir of .ics files",
    after_help = "Every flag can also be set in the config file as `name = value` (e.g. \
                  `week = workweek`, `vdir = ~/.calendars`), with `include = FILE` to read \
                  another. The command line wins over the environment, which wins over the file.",
    args_override_self = true
)]
struct Args {
    /// Root of the vdir (a folder of calendar folders).
    #[clap(env = "RICAL_VDIR")]
    vdir: Option<PathBuf>,

    /// Config file; default ~/.config/rical/config if it exists.
    #[clap(long, env = "RICAL_CONFIG")]
    config: Option<PathBuf>,

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

    /// My email addresses, to tell invitations to me (and my replies)
    /// apart; default is a guess from who is on most invitations.
    #[clap(long, env = "RICAL_ME", value_delimiter = ',')]
    me: Vec<String>,

    /// A folder of vCards (e.g. a vdirsyncer address book) to suggest
    /// guests from, as well as people on past events.
    #[clap(long, env = "RICAL_CONTACTS")]
    contacts: Option<PathBuf>,
}

fn main() -> glib::ExitCode {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Warn)
        .parse_default_env()
        .init();
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("rical-gtk: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    let Some(vdir_path) = &args.vdir else {
        eprintln!(
            "rical-gtk: which vdir? Give it as an argument, RICAL_VDIR, or `vdir = PATH` in the config"
        );
        return glib::ExitCode::FAILURE;
    };

    let local = local_timezone();
    let vdir = match Vdir::load(vdir_path) {
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
    state.me = args.me;
    state.contacts = args.contacts;
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

/// The command line, with the config file's settings under it.
fn parse_args() -> Result<Args, String> {
    use clap::{CommandFactory as _, FromArgMatches as _};
    let argv = std::env::args().collect::<Vec<_>>();
    // the config file to read: --config, RICAL_CONFIG or the default
    let named = argv
        .iter()
        .enumerate()
        .find_map(|(i, a)| match a.strip_prefix("--config") {
            Some(rest) if rest.starts_with('=') => Some(PathBuf::from(&rest[1..])),
            Some("") => argv.get(i + 1).map(PathBuf::from),
            _ => None,
        })
        .or_else(|| std::env::var_os("RICAL_CONFIG").map(PathBuf::from));
    let path = match named {
        Some(p) => Some(p),
        None => config::default_path().filter(|p| p.exists()),
    };
    let mut settings = match &path {
        Some(p) => config::load(p)?,
        None => Vec::new(),
    };
    // paths in the file are relative to it, and may start with ~
    for s in &mut settings {
        if matches!(s.name.as_str(), "vdir" | "binds" | "contacts") {
            s.value = config::expand(&s.value, &s.base).display().to_string();
        }
    }
    let cmd = Args::command();
    let (from_file, vdir) = config::to_args(&settings, &cmd, "vdir")?;
    let full = argv
        .first()
        .cloned()
        .into_iter()
        .chain(from_file)
        .chain(argv.into_iter().skip(1));
    let matches = cmd.try_get_matches_from(full).unwrap_or_else(|e| e.exit());
    let mut args = Args::from_arg_matches(&matches).map_err(|e| e.to_string())?;
    if args.vdir.is_none() {
        args.vdir = vdir.map(|(v, _)| PathBuf::from(v));
    }
    Ok(args)
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

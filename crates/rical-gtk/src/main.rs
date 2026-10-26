use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adw::prelude::*;
use chrono::Utc;
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
mod invitations;
mod journals;
mod layout;
mod listhead;
mod logfile;
mod minimonth;
mod month;
mod person;
mod prefs;
mod raw;
mod repeat;

mod section;
mod sidetasks;
mod state;
mod tasks;
mod window;
mod year;
mod zonepick;
mod zones;

pub const APP_ID: &str = "net.jeffas.Rical";

fn main() -> glib::ExitCode {
    logfile::init();
    log::info!("rical {} starting", env!("CARGO_PKG_VERSION"));
    let options = match config::Options::load() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rical-gtk: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    // GTK's GPU renderers keep a driver and its buffers loaded, 15-40 MB
    // more than cairo, which draws this app just as quickly
    if let Some(r) = &options.renderer
        && std::env::var_os("GSK_RENDERER").is_none()
    {
        // SAFETY: before GTK starts, while this is the only thread
        unsafe { std::env::set_var("GSK_RENDERER", r) };
    }

    // one instance: launching again (say, to open an invitation from a
    // mail client) hands the files to the running one over D-Bus
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.set_option_context_parameter_string(Some("[FILE.ics…]"));
    app.set_option_context_summary(Some(
        "Calendar for a vdir of .ics files. Files given are imported (invitations, \
         replies, cancellations, exports).",
    ));
    app.set_option_context_description(Some(&config::help()));
    app.connect_startup(|_| {
        // run from the source tree, the icon isn't installed yet: find it
        // in data/ (an installed one, from the package, comes first)
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::IconTheme::for_display(&display)
                .add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons"));
        }
        gtk::Window::set_default_icon_name(APP_ID);
    });

    let quit = gtk::gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| a.quit());
    app.add_action(&quit);

    // the vdir is loaded once, in the instance that stays running
    let ui: Rc<RefCell<Option<(state::SharedState, binds::Binds)>>> = Default::default();
    let failed = Rc::new(std::cell::Cell::new(false));
    let options = RefCell::new(Some(options));
    {
        let (ui, failed) = (ui.clone(), failed.clone());
        app.connect_startup(move |_| {
            let Some(options) = options.borrow_mut().take() else {
                return;
            };
            match setup(options) {
                Ok(v) => *ui.borrow_mut() = Some(v),
                Err(e) => {
                    eprintln!("rical-gtk: {e}");
                    failed.set(true);
                }
            }
        });
    }
    let window: Rc<RefCell<Option<Rc<window::Window>>>> = Default::default();
    let show = {
        let (ui, window) = (ui.clone(), window.clone());
        move |app: &adw::Application| -> Option<Rc<window::Window>> {
            if let Some(w) = window.borrow().as_ref() {
                w.present();
                return Some(w.clone());
            }
            let (state, binds) = ui.borrow().clone()?;
            let w = window::Window::new(app, state, binds);
            w.present();
            *window.borrow_mut() = Some(w.clone());
            Some(w)
        }
    };
    let show = Rc::new(show);
    {
        let show = show.clone();
        app.connect_activate(move |app| {
            show(app);
        });
    }
    app.connect_open(move |app, files, _| {
        if let Some(w) = show(app) {
            w.import_files(files);
        }
    });

    let code = app.run();
    if failed.get() {
        glib::ExitCode::FAILURE
    } else {
        code
    }
}

/// The state and bindings, from the settings.
fn setup(o: config::Options) -> Result<(state::SharedState, binds::Binds), String> {
    let vdir_path = o.vdir.ok_or(
        "which vdir? Set `vdir = PATH` in ~/.config/rical/config, or RICAL_VDIR (see --help)",
    )?;
    let local = local_timezone();
    let vdir = Vdir::load(&vdir_path).map_err(|e| format!("{e:#}"))?;
    let date = o
        .date
        .unwrap_or_else(|| Utc::now().with_timezone(&local).date_naive());
    let week = o.week.as_deref().unwrap_or("fullweek");
    let week_view = layout::ViewKind::from_name(week)
        .filter(|v| {
            matches!(
                v,
                layout::ViewKind::Week | layout::ViewKind::WorkWeek | layout::ViewKind::Days(_)
            )
        })
        .ok_or(format!("week: “{week}” is not fullweek, workweek or Ndays"))?;
    let view = match o.view.as_deref().unwrap_or("week") {
        "week" => week_view,
        name => layout::ViewKind::from_name(name).ok_or(format!("view: unknown view “{name}”"))?,
    };

    let mut state = state::State::new(vdir, local, date);
    state.view = view;
    state.week_view = week_view;
    state.last_calendar = o.calendar;
    state.notifications = !o.no_notifications;
    state.me = o.me;
    state.contacts = o.contacts;
    state.zones = o.zones;
    if let Some(text) = &o.remind {
        match rical_core::input::parse_shift(text) {
            Some(d) if d > chrono::TimeDelta::zero() => state.default_reminder = Some(d),
            _ => return Err(format!("remind: not a duration: “{text}”")),
        }
    }
    let binds = load_binds(o.binds.as_deref())?;
    Ok((Rc::new(RefCell::new(state)), binds))
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

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adw::prelude::*;
use chrono::{NaiveDate, Utc};
use clap::Parser as _;
use gtk::glib;
use rical_core::{Vdir, when::local_timezone};

mod agenda;
mod details;
mod draw;
mod format;
mod grid;
mod layout;
mod month;
mod shortcuts;
mod state;
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

    /// View to open: day, week, month or agenda.
    #[clap(long, default_value = "week")]
    view: String,
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
    let Some(view) = layout::ViewKind::from_name(&args.view) else {
        eprintln!("rical-gtk: unknown view {:?}", args.view);
        return glib::ExitCode::FAILURE;
    };

    let mut state = state::State::new(vdir, local, date);
    state.view = view;
    let state = Rc::new(RefCell::new(state));

    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();

    let quit = gtk::gio::SimpleAction::new("quit", None);
    let a = app.clone();
    quit.connect_activate(move |_, _| a.quit());
    app.add_action(&quit);
    app.set_accels_for_action("app.quit", &["<Control>q"]);

    let windows = Rc::new(RefCell::new(Vec::new()));
    app.connect_activate(move |app| {
        let window = window::Window::new(app, state.clone());
        window.present();
        windows.borrow_mut().push(window);
    });

    // arguments are ours, not GTK's
    app.run_with_args(&[std::env::args().next().unwrap_or_default()])
}

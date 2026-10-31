//! Contacts (rical): a keyboard-first address book over folders of vCards.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;

mod cmds;
mod config;
mod editor;
mod importer;
mod prefs;
mod run;
mod state;
mod tidy;
mod window;

pub const APP_ID: &str = "net.jeffas.Rical.Contacts";
/// What the app is called: everyday, and where it sits beside others.
pub const APP_NAME: &str = "Contacts";
pub const APP_FULL_NAME: &str = "Contacts (rical)";

/// The window's look on top of the shared one.
const CSS: &str = "";

fn main() -> glib::ExitCode {
    rical_ui::app::init(rical_ui::app::App {
        id: APP_ID,
        name: APP_NAME,
        full_name: APP_FULL_NAME,
        env_prefix: "RICAL_CONTACTS_",
        folder: Some("contacts"),
        dir: "rical-contacts",
    });
    rical_ui::logfile::init();
    glib::set_application_name(APP_NAME);
    log::info!("rical contacts {} starting", env!("CARGO_PKG_VERSION"));
    let options = match config::Options::load() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rical-contacts: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    if let Some(r) = &options.renderer
        && std::env::var_os("GSK_RENDERER").is_none()
    {
        // SAFETY: before GTK starts, while this is the only thread
        unsafe { std::env::set_var("GSK_RENDERER", r) };
    }

    // one instance: opening a card from another app hands it to the
    // running one over D-Bus
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.set_option_context_parameter_string(Some("[FILE.vcf…]"));
    app.set_option_context_summary(Some(
        "Contacts for folders of vCards. Files given are opened: someone you have is \
         shown, someone new is added.",
    ));
    app.set_option_context_description(Some(&config::help()));
    app.connect_startup(|_| {
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::IconTheme::for_display(&display)
                .add_search_path(concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/icons"));
        }
        gtk::Window::set_default_icon_name(APP_ID);
        rical_ui::util::install_css(CSS);
    });

    let ui: Rc<RefCell<Option<(state::SharedState, rical_ui::binds::Binds)>>> = Default::default();
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
                    eprintln!("rical-contacts: {e}");
                    failed.set(true);
                }
            }
        });
    }
    let window: Rc<RefCell<Option<Rc<window::Window>>>> = Default::default();
    let show = Rc::new({
        let (ui, window) = (ui.clone(), window.clone());
        move |app: &adw::Application| -> Option<Rc<window::Window>> {
            if let Some(w) = window.borrow().as_ref() {
                w.present();
                return Some(w.clone());
            }
            let Some((state, binds)) = ui.borrow().clone() else {
                app.quit();
                return None;
            };
            let w = window::Window::new(app, state, binds);
            rical_ui::app::log_shown(&w.window);
            w.present();
            *window.borrow_mut() = Some(w.clone());
            Some(w)
        }
    });
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
fn setup(o: config::Options) -> Result<(state::SharedState, rical_ui::binds::Binds), String> {
    let root = o.contacts.ok_or(
        "which contacts? Set `contacts = PATH` in ~/.config/rical-contacts/config, \
         or RICAL_CONTACTS_CONTACTS (see --help)",
    )?;
    let started = std::time::Instant::now();
    // the address books only: the window reads their cards in the background
    let books = rical_cards::Books::scan(&root).map_err(|e| format!("{e:#}"))?;
    let loaded = started.elapsed();
    let mut state = state::State::new(books);
    state.loaded = false;
    // start reading the cards now, while GTK builds the window
    // (watching the folders first, to know if they change under the read)
    let early = rical_ui::util::EarlyWatch::new(
        std::iter::once(root.as_path()).chain(state.books.books.iter().map(|b| b.path.as_path())),
    );
    let read = root.clone();
    let read = std::thread::spawn(move || window::read_books(&read, false));
    state.preload = Some((read, early));
    state.default_book = o.book;
    state.set_sort(config::sort_by(o.sort.as_deref()));
    state.history = Some(rical_ui::history::open(
        state::APP,
        o.history.clone(),
        o.history_days,
    ));
    log::info!(
        "{} cards in {} address books: read in {loaded:?}, ready in {:?}",
        state.books.len(),
        state.books.books.len(),
        started.elapsed()
    );
    let binds = load_binds(o.binds.as_deref())?;
    Ok((Rc::new(RefCell::new(state)), binds))
}

/// The default bindings with the user's on top.
fn load_binds(path: Option<&std::path::Path>) -> Result<rical_ui::binds::Binds, String> {
    use rical_ui::binds::Binds;
    let mut binds =
        Binds::parse(include_str!("binds.conf")).map_err(|e| format!("built-in binds: {e}"))?;
    let path = path
        .map(ToOwned::to_owned)
        .or_else(|| rical_ui::app::config_file("binds.conf").filter(|p| p.exists()));
    if let Some(path) = path {
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{path:?}: {e}"))?;
        binds.merge(Binds::parse(&text).map_err(|e| format!("{path:?}: {e}"))?);
    }
    Ok(binds)
}

//! Mail (rical): a keyboard-first mail client for Maildir folders.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;

mod address;
mod cmds;
mod compose;
mod config;
mod editor;
mod filters;
mod load;
mod onboarding;
mod prefs;
mod reader;
mod run;
mod state;
mod window;

pub const APP_ID: &str = "net.jeffas.Rical.Mail";
/// What the app is called: everyday, and where it sits beside others.
pub const APP_NAME: &str = "Mail";
pub const APP_FULL_NAME: &str = "Mail (rical)";

/// The window's look on top of the shared one.
const CSS: &str = "
.rical-body { background: transparent; }
";

fn main() -> glib::ExitCode {
    rical_ui::app::init(rical_ui::app::App {
        id: APP_ID,
        name: APP_NAME,
        full_name: APP_FULL_NAME,
        env_prefix: "RICAL_MAIL_",
        folder: None,
        dir: "rical-mail",
    });
    rical_ui::logfile::init();
    glib::set_application_name(APP_NAME);
    log::info!("rical mail {} starting", env!("CARGO_PKG_VERSION"));
    let options = match config::Options::load() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("rical-mail: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    if let Some(r) = &options.renderer
        && std::env::var_os("GSK_RENDERER").is_none()
    {
        // SAFETY: before GTK starts, while this is the only thread
        unsafe { std::env::set_var("GSK_RENDERER", r) };
    }

    // one instance: a mailto: link from another app opens a new message
    // in the running one
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gtk::gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.set_option_context_parameter_string(Some("[mailto:ADDRESS…]"));
    app.set_option_context_summary(Some(
        "Mail for Maildir folders. A mailto: link given is opened as a new message.",
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
            // no mail folder yet: the first-run window asks for one
            if options.mail.as_ref().is_none_or(|m| !m.is_dir()) {
                return;
            }
            match setup(options) {
                Ok(v) => *ui.borrow_mut() = Some(v),
                Err(e) => {
                    eprintln!("rical-mail: {e}");
                    failed.set(true);
                }
            }
        });
    }
    let window: Rc<RefCell<Option<Rc<window::Window>>>> = Default::default();
    let show = Rc::new({
        let (ui, window, failed) = (ui.clone(), window.clone(), failed.clone());
        move |app: &adw::Application| -> Option<Rc<window::Window>> {
            if let Some(w) = window.borrow().as_ref() {
                w.present();
                return Some(w.clone());
            }
            let Some((state, binds)) = ui.borrow().clone() else {
                if failed.get() {
                    app.quit();
                    return None;
                }
                // the first run: ask, then start as usual
                let (ui, window, app2) = (ui.clone(), window.clone(), app.clone());
                onboarding::open(
                    app,
                    Rc::new(move || {
                        let ready = config::Options::load().and_then(setup);
                        match ready {
                            Ok(v) => {
                                let (state, binds) = v.clone();
                                *ui.borrow_mut() = Some(v);
                                let w = window::Window::new(&app2, state, binds);
                                w.present();
                                *window.borrow_mut() = Some(w);
                            }
                            Err(e) => eprintln!("rical-mail: {e}"),
                        }
                    }),
                );
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
            for f in files {
                let uri = f.uri().to_string();
                if uri.starts_with("mailto:") {
                    compose::open_mailto(&w, &uri);
                } else {
                    w.toast(&format!("Can't open {uri}: only mailto: links"));
                }
            }
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
    let root = o.mail.clone().ok_or(
        "which mail folder? Set `mail = PATH` in ~/.config/rical-mail/config, \
         or RICAL_MAIL_MAIL (see --help)",
    )?;
    let started = std::time::Instant::now();
    // the contacts and calendar are read in the background (Window::new)
    let st = state::State::new(o.clone(), root, None);
    log::info!(
        "{} folders, {} messages in {}: ready in {:?}",
        st.folders.len(),
        st.messages.len(),
        st.place_name(),
        started.elapsed()
    );
    let binds = load_binds(o.binds.as_deref())?;
    Ok((Rc::new(RefCell::new(st)), binds))
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

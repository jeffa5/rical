//! Small things every rical app does: open links, share a look, say what
//! it is, and keep its memory in check.

use std::{cell::Cell, rc::Rc, time::Duration};

use adw::prelude::*;
use gtk::glib;

/// Open a link (a web page, `mailto:`, `tel:`, a map) in the app the
/// desktop uses for it.
pub fn open_uri(parent: Option<&gtk::Window>, uri: &str) {
    log::info!("opening {uri}");
    gtk::UriLauncher::new(uri).launch(parent, None::<&gtk::gio::Cancellable>, |r| {
        if let Err(e) = r {
            log::warn!("could not open link: {e}");
        }
    });
}

/// A link that shows `place` on a map: a `geo:` link when a maps app is
/// installed to handle it, else a web map.
pub fn map_url(place: &str) -> String {
    if gtk::gio::AppInfo::default_for_uri_scheme("geo").is_some() {
        geo_url(place)
    } else {
        web_map_url(place)
    }
}

/// Fold newlines and runs of spaces into single spaces.
fn one_line(place: &str) -> String {
    place.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Percent-encode `s` for a URL query, spaces as `+`.
fn query_escape(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".to_owned(),
            b => format!("%{b:02X}"),
        })
        .collect()
}

/// A `geo:` URI (RFC 5870) asking a maps app to search for `place`.
pub fn geo_url(place: &str) -> String {
    format!("geo:0,0?q={}", query_escape(&one_line(place)))
}

/// A web map searching for `place`.
pub fn web_map_url(place: &str) -> String {
    format!(
        "https://www.openstreetmap.org/search?query={}",
        query_escape(&one_line(place))
    )
}

/// The look the apps share: a tinted selection in lists (so it stands out
/// from a hovered row), and people's names as quiet links. Each app adds
/// its own on top.
pub const SHARED_CSS: &str = "\
    .rical-list row:selected { background-color: alpha(@accent_bg_color, 0.16); }\n\
    .rical-list row:selected:hover { background-color: alpha(@accent_bg_color, 0.22); }\n\
    button.person-link { padding: 1px 4px; margin: -1px -4px; min-height: 0; font-weight: normal; }\n";

/// Load `css` (with [`SHARED_CSS`] before it) for the whole app.
pub fn install_css(css: &str) -> gtk::CssProvider {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(&format!("{SHARED_CSS}{css}"));
    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
    provider
}

/// The About dialog: the app's name, icon and version, links and licence,
/// and for a bug report, `details` (what the app is working on) with the
/// versions, settings file and the end of the log.
pub fn about(parent: &impl IsA<gtk::Widget>, comments: &str, details: &str) {
    let app = crate::app::get();
    let log = crate::logfile::path();
    let debug = format!(
        "{} {}\nGTK {}.{}.{}, libadwaita {}.{}.{}\nrenderer: {}\nconfig: {}\n{details}log: {}\n\n\
         The end of the log:\n{}\n",
        app.full_name,
        env!("CARGO_PKG_VERSION"),
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version(),
        adw::major_version(),
        adw::minor_version(),
        adw::micro_version(),
        std::env::var("GSK_RENDERER").unwrap_or_else(|_| "auto".into()),
        crate::config::config_path().map_or("none".into(), |p| p.display().to_string()),
        log.as_deref()
            .map_or("none".into(), |p| p.display().to_string()),
        crate::logfile::tail(40),
    );
    let dialog = adw::AboutDialog::builder()
        .application_name(app.full_name)
        .application_icon(app.id)
        .version(env!("CARGO_PKG_VERSION"))
        .comments(comments)
        .developer_name("Andrew Jeffery")
        .developers(["Andrew Jeffery"])
        .copyright("© 2025–2026 Andrew Jeffery")
        .license_type(gtk::License::MitX11)
        .website("https://github.com/jeffa5/rical")
        .issue_url("https://github.com/jeffa5/rical/issues")
        .debug_info(debug)
        .debug_info_filename("rical-debug.txt")
        .build();
    // the log itself, opened in the text editor
    if let Some(p) = log.filter(|p| p.exists()) {
        dialog.add_link("Open log file", &gtk::gio::File::for_path(p).uri());
    }
    dialog.present(Some(parent.as_ref()));
}

/// Hands freed memory back to the system once things settle. Big views
/// free a lot at once, and glibc keeps it unless asked, so an app grows
/// to its largest view and stays there.
#[derive(Default, Clone)]
pub struct Trimmer {
    pending: Rc<Cell<bool>>,
}

impl Trimmer {
    /// Trim in a couple of seconds, unless a trim is already due.
    pub fn soon(&self) {
        if self.pending.replace(true) {
            return;
        }
        let pending = self.pending.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            pending.set(false);
            // SAFETY: malloc_trim only releases free memory; no Rust
            // object is touched
            unsafe { libc::malloc_trim(0) };
        });
    }
}

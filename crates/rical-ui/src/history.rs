//! The history an app keeps (`rical_core::history`): opening it from the
//! settings, and the History dialog, where anything done can be undone or
//! redone, however long ago, by keyboard or click.

use std::{path::PathBuf, rc::Rc};

use adw::prelude::*;
use chrono::{Local, TimeDelta};
use rical_core::history::{Entry, History};

/// Days kept unless a setting says otherwise.
pub const DEFAULT_DAYS: u32 = 30;

/// What the settings help says about the two settings.
pub const SETTINGS: [(&str, &str); 2] = [
    (
        "history",
        "Folder keeping what was changed and deleted, to undo it later (after a \
         restart too): a log and each file as it was. Default \
         ~/.local/share/rical/history, shared by the apps.",
    ),
    (
        "history-days",
        "How many days of history (and deleted files) to keep. Default 30.",
    ),
];

/// A `history-days` value.
pub fn parse_days(value: &str) -> Result<u32, String> {
    value
        .trim()
        .trim_end_matches('d')
        .parse()
        .map_err(|_| format!("history-days is a number of days, not “{value}”"))
}

/// The history in `dir` (else the default place), keeping `days`. If it
/// can't be opened there, one in the runtime folder stands in so undo
/// still works this session, and the problem is logged.
pub fn open(dir: Option<PathBuf>, days: Option<u32>) -> History {
    let days = days.unwrap_or(DEFAULT_DAYS);
    let dir = dir.or_else(rical_core::history::default_dir);
    if let Some(dir) = &dir {
        match History::open(dir, days) {
            Ok(h) => return h,
            Err(e) => log::warn!("cannot keep history in {dir:?}: {e:#}"),
        }
    }
    let fallback =
        gtk::glib::user_runtime_dir().join(format!("rical-history-{}", std::process::id()));
    History::open(&fallback, days).expect("a history in the runtime folder")
}

/// When, in a few words: "14:05", "yesterday 09:12", "Mon 3 Nov 14:05".
fn when(entry: &Entry) -> String {
    let Some(t) = entry.time() else {
        return String::new();
    };
    let t = t.with_timezone(&Local);
    let today = Local::now().date_naive();
    let day = t.date_naive();
    if day == today {
        t.format("%H:%M").to_string()
    } else if day == today - TimeDelta::days(1) {
        t.format("yesterday %H:%M").to_string()
    } else {
        t.format("%a %-d %b %H:%M").to_string()
    }
}

/// Undo (`true`) or redo entry `id`, saying why not if it can't be.
pub type Toggle = Rc<dyn Fn(&str, bool) -> Result<(), String>>;
/// The entries as they now are.
pub type Entries = Rc<dyn Fn() -> (Vec<Entry>, PathBuf)>;

/// The History dialog for `app`'s entries, newest first.
pub fn dialog(app: &'static str, entries: Entries, toggle: Toggle) -> adw::Dialog {
    let list = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::Single)
        .build();
    let status = adw::StatusPage::builder()
        .icon_name("document-open-recent-symbolic")
        .title("Nothing done yet")
        .description("What you change and delete is listed here, to undo later")
        .vexpand(true)
        .build();
    let problem = gtk::Label::builder()
        .css_classes(["error"])
        .wrap(true)
        .visible(false)
        .build();
    let folder = gtk::Button::builder()
        .label("Open the History Folder")
        .css_classes(["flat"])
        .halign(gtk::Align::Center)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();
    content.append(&problem);
    content.append(&list);
    content.append(&status);
    content.append(&folder);
    let scroller = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&scroller));
    let dialog = adw::Dialog::builder()
        .title("History")
        .content_width(560)
        .content_height(620)
        .child(&view)
        .build();

    // the rows, filled again after each undo or redo
    let ids: Rc<std::cell::RefCell<Vec<(String, bool)>>> = Default::default();
    let fill: Rc<dyn Fn(Option<usize>)> = {
        let (list, status, ids, entries) =
            (list.clone(), status.clone(), ids.clone(), entries.clone());
        Rc::new(move |select: Option<usize>| {
            list.remove_all();
            let (all, _) = entries();
            let mine: Vec<&Entry> = all.iter().rev().filter(|e| e.app == app).collect();
            *ids.borrow_mut() = mine.iter().map(|e| (e.id.clone(), e.undone)).collect();
            for e in &mine {
                let files = match e.files.len() {
                    1 => "1 file".to_owned(),
                    n => format!("{n} files"),
                };
                let row = adw::ActionRow::builder()
                    .title(gtk::glib::markup_escape_text(&e.what))
                    .subtitle(format!(
                        "{} · {files}{}",
                        when(e),
                        if e.undone { " · undone" } else { "" }
                    ))
                    .activatable(true)
                    .build();
                let button = gtk::Button::builder()
                    .label(if e.undone { "Redo" } else { "Undo" })
                    .valign(gtk::Align::Center)
                    .can_focus(false)
                    .build();
                let r = row.clone();
                button.connect_clicked(move |_| {
                    r.emit_by_name::<()>("activate", &[]);
                });
                row.add_suffix(&button);
                list.append(&row);
            }
            list.set_visible(!mine.is_empty());
            status.set_visible(mine.is_empty());
            let i = select.unwrap_or(0).min(mine.len().saturating_sub(1));
            if let Some(row) = list.row_at_index(i as i32) {
                list.select_row(Some(&row));
                row.grab_focus();
            }
        })
    };
    {
        let (fill, ids, problem) = (fill.clone(), ids.clone(), problem.clone());
        list.connect_row_activated(move |_, row| {
            let i = row.index() as usize;
            let Some((id, undone)) = ids.borrow().get(i).cloned() else {
                return;
            };
            match toggle(&id, !undone) {
                Ok(()) => problem.set_visible(false),
                Err(e) => {
                    problem.set_label(&e);
                    problem.set_visible(true);
                }
            }
            fill(Some(i));
        });
    }
    folder.connect_clicked(move |b| {
        let (_, dir) = entries();
        crate::util::open_uri(
            b.root().and_downcast_ref::<gtk::Window>(),
            &gtk::gio::File::for_path(dir).uri(),
        );
    });
    fill(None);
    dialog
}

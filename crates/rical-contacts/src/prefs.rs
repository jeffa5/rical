//! Contacts' Preferences: its settings on the shared dialog
//! (`rical_ui::prefs`), which saves each to the config file.

use std::rc::Rc;

use adw::prelude::*;
use rical_ui::prefs::{Prefs, owned};

use crate::{config::SETTINGS, window::Window};

/// Settings that only take effect when the app starts.
const AT_START: &[&str] = &["contacts", "binds", "renderer", "history", "history-days"];

pub fn show(win: &Rc<Window>) {
    let books: Vec<(String, String)> = win
        .state
        .borrow()
        .books
        .books
        .iter()
        .map(|b| (b.name.clone(), b.name.clone()))
        .collect();
    let weak = Rc::downgrade(win);
    let on_saved = Rc::new(move || -> Result<(), String> {
        let o = crate::config::Options::load()?;
        if let Some(w) = weak.upgrade() {
            w.state.borrow_mut().default_book = o.book;
            w.state
                .borrow_mut()
                .set_sort(crate::config::sort_by(o.sort.as_deref()));
            w.fill_list();
            w.show_detail();
        }
        Ok(())
    });
    let Some(ctx) = Prefs::open(SETTINGS, AT_START, on_saved) else {
        return;
    };
    let value = |name: &str| ctx.value(name);
    let env = |name: &str| ctx.env(name);

    let books_group = ctx.group("Address Books");
    books_group.add(&ctx.path_row(
        "contacts",
        "Contacts folder",
        "A folder of address books, e.g. vdirsyncer's",
        value("contacts"),
        env("contacts"),
        true,
    ));
    let mut names = vec![("".to_owned(), "The one with the most people".to_owned())];
    names.extend(books);
    books_group.add(&ctx.combo_row(
        "book",
        "New contacts go in",
        &names,
        value("book").unwrap_or_default(),
        env("book"),
    ));

    let orders = [
        ("", "By name, as shown (the default)"),
        ("family", "By family name"),
    ];
    books_group.add(&ctx.combo_row(
        "sort",
        "Sort people",
        &owned(&orders),
        value("sort").filter(|v| v != "name").unwrap_or_default(),
        env("sort"),
    ));

    let advanced = ctx.group("Advanced");
    advanced.add(&ctx.path_row(
        "binds",
        "Key bindings file",
        "Default ~/.config/rical/contacts/binds.conf",
        value("binds"),
        env("binds"),
        false,
    ));
    advanced.add(&ctx.path_row(
        "history",
        "History folder",
        "What was changed and deleted, to undo later. Default ~/.local/share/rical/history",
        value("history"),
        env("history"),
        true,
    ));
    advanced.add(&ctx.entry_row(
        "history-days",
        "Days of history to keep (default 30)",
        value("history-days"),
        env("history-days"),
        |v| rical_ui::history::parse_days(v).map(|_| ()),
    ));
    let renderers = [
        ("", "Cairo (the default: least memory)"),
        ("gl", "OpenGL"),
        ("vulkan", "Vulkan"),
        ("auto", "GTK's choice"),
    ];
    advanced.add(
        &ctx.combo_row(
            "renderer",
            "Drawing",
            &owned(&renderers),
            value("renderer")
                .filter(|v| v != "cairo")
                .unwrap_or_default(),
            env("renderer"),
        ),
    );
    ctx.present(&win.window);
}

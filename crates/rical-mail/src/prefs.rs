//! Mail's Preferences: its settings on the shared dialog
//! (`rical_ui::prefs`), which saves each to the config file.

use std::rc::Rc;

use adw::prelude::*;
use rical_ui::prefs::{Prefs, is_true, owned};

use crate::{config::SETTINGS, window::Window};

/// Settings that only take effect when the app starts.
const AT_START: &[&str] = &["mail", "contacts", "vdir", "binds", "renderer", "font"];

pub fn show(win: &Rc<Window>) {
    let weak = Rc::downgrade(win);
    let on_saved = Rc::new(move || -> Result<(), String> {
        let o = crate::config::Options::load()?;
        if let Some(w) = weak.upgrade() {
            w.state.borrow_mut().opts = o;
            w.fill_list();
        }
        Ok(())
    });
    let Some(ctx) = Prefs::open(SETTINGS, AT_START, on_saved) else {
        return;
    };
    let value = |name: &str| ctx.value(name);
    let env = |name: &str| ctx.env(name);
    let text = |name: &'static str, title: &str, check: fn(&str) -> Result<(), String>| {
        ctx.entry_row(name, title, value(name), env(name), check)
    };
    let any = |_: &str| -> Result<(), String> { Ok(()) };

    let mail = ctx.group("Mail");
    mail.add(&ctx.path_row(
        "mail",
        "Mail folder",
        "The folder of Maildir folders, e.g. what mbsync keeps",
        value("mail"),
        env("mail"),
        true,
    ));
    mail.add(&text(
        "me",
        "Your addresses (Name &lt;you@example.com&gt;, …)",
        |v| {
            if rical_maildir::addr::parse_list(v).is_empty() {
                Err("addresses like Name <you@example.com>".into())
            } else {
                Ok(())
            }
        },
    ));
    mail.add(&ctx.path_row(
        "contacts",
        "Contacts folder",
        "The address books addresses are completed from",
        value("contacts"),
        env("contacts"),
        true,
    ));
    mail.add(&ctx.path_row(
        "vdir",
        "Calendar folder",
        "vdirsyncer's folder: invitations are compared with the event in it",
        value("vdir"),
        env("vdir"),
        true,
    ));

    mail.add(&ctx.combo_row(
        "search",
        "Search with",
        &owned(&[
            ("builtin", "This app (every folder's headers)"),
            ("mu", "mu (indexed; results as links)"),
            ("notmuch", "notmuch (indexed)"),
        ]),
        value("search").unwrap_or_else(|| "builtin".to_owned()),
        env("search"),
    ));

    let sending = ctx.group("Sending");
    sending.add(&text(
        "index",
        "Update the search index with (mu index)",
        any,
    ));
    sending.add(&ctx.path_row(
        "queue",
        "Send queue",
        "Where rical-mail-queue takes messages from. Default ~/.local/share/rical-mail-queue/outbox",
        value("queue"),
        env("queue"),
        true,
    ));
    sending.add(&text(
        "send",
        "Send with (only without a rical-mail-queue run service)",
        any,
    ));

    let writing = ctx.group("Writing");
    writing.add(&text("wrap", "Wrap at column (72)", |v| {
        v.parse::<usize>()
            .ok()
            .filter(|w| (30..=200).contains(w))
            .map(|_| ())
            .ok_or("a column from 30 to 200".into())
    }));
    writing.add(&ctx.switch_row(
        "auto-wrap",
        "Wrap as you type",
        "Break lines at the wrap column",
        value("auto-wrap").is_none_or(|v| is_true(&v)),
        env("auto-wrap"),
        |on| Some(on.to_string()),
    ));
    writing.add(&ctx.combo_row(
        "reply",
        "Replies start",
        &owned(&[("bottom", "Below the quote"), ("top", "Above the quote")]),
        value("reply").unwrap_or_else(|| "bottom".to_owned()),
        env("reply"),
    ));
    writing.add(&text("signature", "Signature (a file, or the text)", any));
    writing.add(&text("editor", "Your editor (foot nvim)", any));

    let reading = ctx.group("Reading");
    reading.add(&ctx.switch_row(
        "threads",
        "Conversations",
        "Show replies under what they answer",
        value("threads").is_none_or(|v| is_true(&v)),
        env("threads"),
        |on| Some(on.to_string()),
    ));
    reading.add(&text(
        "read-after",
        "Counts as read after (1s, 0s, never)",
        |v| {
            if v.eq_ignore_ascii_case("never") || v.trim_end_matches('s').parse::<u32>().is_ok() {
                Ok(())
            } else {
                Err("0s, 2s… or never".into())
            }
        },
    ));
    reading.add(&ctx.combo_row(
        "font",
        "Messages are drawn in",
        &owned(&[
            ("monospace", "Monospace (the default)"),
            ("proportional", "The interface font"),
        ]),
        value("font").unwrap_or_else(|| "monospace".to_owned()),
        env("font"),
    ));

    reading.add(&ctx.switch_row(
        "notify",
        "Notify of new mail",
        "A notice with the sender and subject, when the window is not in front",
        value("notify").is_none_or(|v| is_true(&v)),
        env("notify"),
        |on| Some(on.to_string()),
    ));

    let apps = ctx.group("Other apps");
    apps.add(&text("calendar", "Opens invitations (rical-calendar)", any));
    apps.add(&text("contacts-app", "Opens people (rical-contacts)", any));

    let advanced = ctx.group("Advanced");
    advanced.add(&ctx.path_row(
        "binds",
        "Key bindings file",
        "Default ~/.config/rical-mail/binds.conf",
        value("binds"),
        env("binds"),
        false,
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

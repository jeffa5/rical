//! The calendar's Preferences: its settings, in groups, on the shared
//! dialog (`rical_ui::prefs`), which saves each to the config file.

use adw::prelude::*;
use rical_ui::prefs::{OnSaved, Prefs, is_true, owned};

use crate::config::SETTINGS;

/// Settings that only take effect when the calendar starts.
const AT_START: &[&str] = &[
    "vdir",
    "view",
    "binds",
    "renderer",
    "history",
    "history-days",
];

pub fn show(parent: &impl IsA<gtk::Widget>, calendars: Vec<(String, String)>, on_saved: OnSaved) {
    let Some(ctx) = Prefs::open(SETTINGS, AT_START, on_saved) else {
        return;
    };
    let value = |name: &str| ctx.value(name);
    let env = |name: &str| ctx.env(name);
    let group = |title: &str| ctx.group(title);

    let calendars_group = group("Calendars");
    calendars_group.add(&ctx.path_row(
        "vdir",
        "Calendar folder",
        "A folder of calendar folders, e.g. vdirsyncer's",
        value("vdir"),
        env("vdir"),
        true,
    ));
    let mut names = vec![("".to_owned(), "Automatic".to_owned())];
    names.extend(calendars);
    calendars_group.add(&ctx.combo_row(
        "calendar",
        "New events go in",
        &names,
        value("calendar").unwrap_or_default(),
        env("calendar"),
    ));
    calendars_group.add(&ctx.path_row(
        "contacts",
        "Contacts folder",
        "vCards to suggest guests from and name people by",
        value("contacts"),
        env("contacts"),
        true,
    ));
    calendars_group.add(&ctx.entry_row(
        "me",
        "My email addresses (comma-separated; guessed if empty)",
        value("me"),
        env("me"),
        |_| Ok(()),
    ));

    let views = group("Views");
    let view_names = [
        ("", "Week (the default)"),
        ("day", "Day"),
        ("3days", "3 days"),
        ("month", "Month"),
        ("year", "Year"),
        ("agenda", "Agenda"),
        ("tasks", "Tasks"),
        ("journal", "Journal"),
    ];
    views.add(&ctx.combo_row(
        "view",
        "Open on",
        &owned(&view_names),
        value("view").unwrap_or_default(),
        env("view"),
    ));
    let week_names = [
        ("", "Full week (the default)"),
        ("workweek", "Work week"),
        ("3days", "3 days"),
        ("5days", "5 days"),
    ];
    views.add(&ctx.combo_row(
        "week",
        "The week view shows",
        &owned(&week_names),
        value("week").unwrap_or_default(),
        env("week"),
    ));
    views.add(&ctx.entry_row(
        "zones",
        "Extra time zones (e.g. UK=Europe/London, America/New_York)",
        value("zones"),
        env("zones"),
        |v| crate::zones::Zone::parse_list(v).map(|_| ()),
    ));

    let reminders = group("Reminders");
    reminders.add(&ctx.switch_row(
        "no-notifications",
        "Show reminders",
        "Off to leave them to `rical remind` running as a service",
        // the setting says the opposite
        !value("no-notifications").is_some_and(|v| is_true(&v)),
        env("no-notifications"),
        |on| (!on).then(|| "true".to_owned()),
    ));
    reminders.add(&ctx.entry_row(
        "remind",
        "Remind before events without their own (e.g. 10m)",
        value("remind"),
        env("remind"),
        |v| {
            rical_core::input::parse_shift(v)
                .filter(|d| *d > chrono::TimeDelta::zero())
                .map(|_| ())
                .ok_or_else(|| format!("not a length of time: “{v}”"))
        },
    ));

    let advanced = group("Advanced");
    advanced.add(&ctx.path_row(
        "binds",
        "Key bindings file",
        "Default ~/.config/rical-calendar/binds.conf",
        value("binds"),
        env("binds"),
        false,
    ));
    advanced.add(&ctx.path_row(
        "history",
        "History folder",
        "What was changed and deleted, to undo later. Default ~/.local/share/rical-calendar/history",
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

    ctx.present(parent);
}

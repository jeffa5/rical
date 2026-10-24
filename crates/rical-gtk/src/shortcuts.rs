//! The keyboard shortcuts dialog. Keep in sync with `Window::handle_key`
//! and the accelerators in `Window::setup_actions`.

pub const SECTIONS: &[(&str, &[(&str, &str)])] = &[
    (
        "Navigation",
        &[
            ("Go to today", "t"),
            ("Next day / week / month", "n Page_Down"),
            ("Previous day / week / month", "p Page_Up"),
            ("Go to date", "g <Control>g"),
            ("Previous / next day", "h l Left Right"),
            ("Previous / next week (month view)", "Up Down"),
        ],
    ),
    (
        "Views",
        &[
            ("Day", "d <Control>1"),
            ("Week", "w <Control>2"),
            ("Month", "m <Control>3"),
            ("Agenda", "a <Control>4"),
        ],
    ),
    (
        "Events",
        &[
            ("Next event", "j Down"),
            ("Previous event", "k Up"),
            ("Show details", "Return space"),
            ("Clear selection / close", "Escape"),
        ],
    ),
    (
        "General",
        &[
            ("Show / hide calendar 1–9", "<Alt>1"),
            ("Toggle sidebar", "F9"),
            ("Reload", "<Control>r F5"),
            ("Keyboard shortcuts", "question <Control>question"),
            ("Quit", "<Control>q"),
        ],
    ),
];

pub fn dialog() -> adw::ShortcutsDialog {
    let dialog = adw::ShortcutsDialog::new();
    for (title, items) in SECTIONS {
        let section = adw::ShortcutsSection::new(Some(title));
        for (name, accel) in *items {
            section.add(adw::ShortcutsItem::new(name, accel));
        }
        dialog.add(section);
    }
    dialog
}

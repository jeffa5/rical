//! Right-click menus: what can be done to the item clicked (once it is
//! selected), or on an empty day or hour, adding one there.

use chrono::NaiveDate;
use gtk::{gio, prelude::*};
use rical_core::{Kind, Occurrence};
use rical_ui::menu::section;

use crate::state::State;

/// The menu for an event, task or journal entry. Delete goes with the
/// section before it: GTK sizes a menu ending in a section of one item too
/// short to show it.
pub fn item(state: &State, o: &Occurrence) -> gio::Menu {
    let menu = gio::Menu::new();
    match o.kind {
        Kind::Todo => {
            let done = o.status.as_deref() == Some("COMPLETED");
            section(
                &menu,
                &[
                    (if done { "Not _Done" } else { "_Done" }, "done"),
                    ("_Details", "details"),
                    ("_Edit", "edit"),
                ],
            );
            section(
                &menu,
                &[
                    ("Du_plicate", "duplicate"),
                    ("Make an E_vent", "convert"),
                    ("De_lete", "delete"),
                ],
            );
        }
        Kind::Journal => {
            section(&menu, &[("_Details", "details"), ("_Edit", "edit")]);
            section(&menu, &[("Du_plicate", "duplicate"), ("De_lete", "delete")]);
        }
        _ => {
            if state.invitation(o).is_some() {
                section(
                    &menu,
                    &[
                        ("_Accept", "accept"),
                        ("_Maybe", "maybe"),
                        ("De_cline", "decline"),
                    ],
                );
            }
            section(&menu, &[("_Details", "details"), ("_Edit", "edit")]);
            let mut more = vec![("Du_plicate", "duplicate"), ("Make a _Task", "convert")];
            if o.location.as_deref().is_some_and(|l| !l.trim().is_empty()) {
                more.push(("Show on a _Map", "map"));
            }
            more.push(("De_lete", "delete"));
            section(&menu, &more);
        }
    }
    menu
}

/// The menu on empty space: a new event then (an hour from `minutes`
/// into `day`), or all `day` without a time.
pub fn empty(day: NaiveDate, minutes: Option<f64>) -> gio::Menu {
    let menu = gio::Menu::new();
    let date = day.format("%Y-%m-%d");
    let new = match minutes {
        Some(m) => {
            let m = (m / 30.0).floor() as u32 * 30;
            format!("new {date} {:02}:{:02}", m / 60, m % 60)
        }
        None => format!("new {date}"),
    };
    section(
        &menu,
        &[
            ("_New Event Here", &new),
            ("New _Task That Day", &format!("task --due {date}")),
        ],
    );
    menu
}

/// Right clicks on a list of items: the row there is selected, as a click
/// selects it, then its menu shows.
pub fn on_list(list: &gtk::ListBox, state: crate::state::SharedState) {
    let weak = list.downgrade();
    rical_ui::menu::on_right_click(list, move |_, y| {
        let list = weak.upgrade()?;
        let row = list.row_at_y(y as i32).filter(|r| r.is_selectable())?;
        list.select_row(Some(&row));
        let s = state.borrow();
        let o = s.current_item()?;
        Some(item(&s, &o))
    });
}

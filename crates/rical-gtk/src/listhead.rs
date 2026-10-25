//! Group headings for a list: shown above the first row of each group but
//! not rows themselves, so keyboard navigation goes straight past them.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;

/// The heading (if any) above each row, by row index.
pub type Headings = Rc<RefCell<Vec<Option<gtk::Widget>>>>;

pub fn install(list: &gtk::ListBox) -> Headings {
    let headings: Headings = Default::default();
    let h = headings.clone();
    list.set_header_func(move |row, _| {
        let heading = h.borrow().get(row.index() as usize).cloned().flatten();
        if row.header() != heading {
            row.set_header(heading.as_ref());
        }
    });
    headings
}

/// A heading label.
pub fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .css_classes(["heading"])
        .xalign(0.0)
        .margin_top(12)
        .margin_bottom(4)
        .margin_start(6)
        .build()
}

/// Select the next (or previous) selectable row after the selected one,
/// or the first, and give it focus.
pub fn step(list: &gtk::ListBox, forward: bool) {
    let mut i = match list.selected_row() {
        Some(r) if forward => r.index() + 1,
        Some(r) => r.index() - 1,
        None => 0,
    };
    while i >= 0 {
        let Some(row) = list.row_at_index(i) else {
            return;
        };
        if row.is_selectable() {
            list.select_row(Some(&row));
            row.grab_focus();
            return;
        }
        i += if forward { 1 } else { -1 };
    }
}

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

/// Scroll `row`, with its heading, into view once it is laid out: the
/// selection moved from the keyboard without the list having focus.
pub fn reveal(row: &gtk::ListBoxRow) {
    let row = row.clone();
    gtk::glib::idle_add_local_once(move || {
        let Some(scroller) = row
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
        else {
            return;
        };
        let Some(b) = row.compute_bounds(&scroller) else {
            return;
        };
        let heading = row.header().map_or(0.0, |h| h.height() as f64);
        let top = b.y() as f64 - heading - 6.0;
        let bottom = (b.y() + b.height()) as f64 + 6.0;
        let adj = scroller.vadjustment();
        if top < 0.0 {
            adj.set_value(adj.value() + top);
        } else if bottom > adj.page_size() {
            adj.set_value(adj.value() + bottom - adj.page_size());
        }
    });
}

//! The event details popover.

use adw::prelude::*;
use gtk::{gdk, glib};
use rical_core::{Kind, Occurrence};

use crate::{format, state::State};

/// Show the details of `occ` in a popover pointing at `rect` in `parent`.
pub fn show(parent: &impl IsA<gtk::Widget>, rect: gdk::Rectangle, occ: &Occurrence, state: &State) {
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .width_request(260)
        .build();

    let calendar = state
        .calendar(&occ.calendar)
        .map_or(occ.calendar.clone(), |c| c.name.clone());
    let cal_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    cal_row.append(&format::color_dot(state.color(&occ.calendar), 10));
    cal_row.append(
        &gtk::Label::builder()
            .label(&calendar)
            .css_classes(["dim-label", "caption"])
            .xalign(0.0)
            .build(),
    );
    content.append(&cal_row);

    let title = match occ.kind {
        Kind::Todo => format!("{} {}", format::todo_mark(occ), occ.summary),
        _ => occ.summary.clone(),
    };
    content.append(
        &gtk::Label::builder()
            .label(if title.is_empty() {
                "(untitled)"
            } else {
                &title
            })
            .css_classes(["title-3"])
            .wrap(true)
            .max_width_chars(36)
            .selectable(false)
            .xalign(0.0)
            .build(),
    );

    let mut when = format::time_range(occ, state.local);
    if occ.recurring {
        when.push_str(" · repeats");
    }
    content.append(&row("x-office-calendar-symbolic", &when));

    if let Some(status) = &occ.status {
        content.append(&row("dialog-information-symbolic", &status.to_lowercase()));
    }
    if let Some(location) = occ.location.as_deref().filter(|l| !l.is_empty()) {
        content.append(&row("mark-location-symbolic", location));
    }
    if let Some(description) = occ.description.as_deref().filter(|d| !d.is_empty()) {
        let label = gtk::Label::builder()
            .label(description.trim())
            .wrap(true)
            .max_width_chars(48)
            .selectable(true)
            // focusing a selectable label selects all of it
            .can_focus(false)
            .xalign(0.0)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&label)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height(240)
            .build();
        content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        content.append(&scroller);
    }

    let popover = gtk::Popover::builder()
        .child(&content)
        .pointing_to(&rect)
        .position(gtk::PositionType::Right)
        .build();
    popover.set_parent(parent.as_ref());
    popover.connect_closed(|p| {
        let p = p.clone();
        // unparent after the close animation has started
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
}

fn row(icon: &str, text: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    b.append(&gtk::Image::from_icon_name(icon));
    b.append(
        &gtk::Label::builder()
            .label(text)
            .wrap(true)
            .max_width_chars(40)
            .selectable(false)
            .xalign(0.0)
            .build(),
    );
    b
}

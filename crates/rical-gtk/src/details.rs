//! The event details popover.

use std::cell::RefCell;

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

    let title = format::label(occ, state.local);
    let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if occ.kind == Kind::Todo {
        title_row.append(&format::task_check(format::is_done(occ), None));
    }
    title_row.append(
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
    content.append(&title_row);

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
    let join = format::join_url(occ);
    let place = format::place(occ).map(str::to_owned);
    if join.is_some() || place.is_some() {
        let buttons = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(6)
            .build();
        let window = parent.as_ref().root().and_downcast::<gtk::Window>();
        if let Some(url) = join {
            let b = gtk::Button::builder()
                .label("Join")
                .tooltip_text(format!("Open {url} (O)"))
                .css_classes(["suggested-action", "pill"])
                .build();
            let w = window.clone();
            b.connect_clicked(move |_| open_uri(w.as_ref(), &url));
            buttons.append(&b);
        }
        if let Some(place) = place {
            let b = gtk::Button::builder()
                .label("Map")
                .tooltip_text("Show on a map (M)")
                .css_classes(["pill"])
                .build();
            let w = window.clone();
            b.connect_clicked(move |_| open_uri(w.as_ref(), &format::map_url(&place)));
            buttons.append(&b);
        }
        content.append(&buttons);
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

    close();
    // Not modal: keys keep going to the window, so j/k/e/x still work while
    // it is open. It closes on the next change or with Escape.
    let popover = gtk::Popover::builder()
        .child(&content)
        .pointing_to(&rect)
        .position(gtk::PositionType::Right)
        .autohide(false)
        .can_focus(false)
        .build();
    popover.set_parent(parent.as_ref());
    popover.connect_closed(|p| {
        let p = p.clone();
        // unparent after the close animation has started
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
    OPEN.with(|open| *open.borrow_mut() = Some(popover));
}

thread_local! {
    static OPEN: RefCell<Option<gtk::Popover>> = const { RefCell::new(None) };
}

/// Close the open details popover. Returns whether one was open.
pub fn close() -> bool {
    OPEN.with(|open| match open.borrow_mut().take() {
        Some(p) => {
            p.popdown();
            true
        }
        None => false,
    })
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

/// Open a link in the default browser.
pub fn open_uri(parent: Option<&gtk::Window>, uri: &str) {
    log::info!("opening {uri}");
    gtk::UriLauncher::new(uri).launch(parent, None::<&gtk::gio::Cancellable>, |r| {
        if let Err(e) = r {
            log::warn!("could not open link: {e}");
        }
    });
}

//! The event details popover.

use std::cell::RefCell;

use adw::prelude::*;
use gtk::{gdk, glib};
use rical_core::{Kind, Occurrence};

use crate::{format, layout::ViewKind, state::State};

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
    // set in another time zone (a flight, a call abroad): its time there
    if let rical_core::When::Zoned(start) = occ.start
        && start.timezone() != state.local
    {
        let tz = start.timezone();
        let end = occ.end.to_utc(state.local).with_timezone(&tz);
        let place = crate::zones::Zone {
            label: String::new(),
            tz,
        };
        let name = tz.name().rsplit('/').next().unwrap_or("").replace('_', " ");
        content.append(&row(
            "preferences-system-time-symbolic",
            &format!(
                "{}–{} {name} time ({})",
                start.format("%H:%M"),
                end.format("%H:%M"),
                place.abbreviation(start.to_utc())
            ),
        ));
    }

    // only a status that says something: not the usual confirmed, final
    // or still to do
    if let Some(status) = occ.status.as_ref().filter(|s| {
        !matches!(
            s.to_ascii_uppercase().as_str(),
            "CONFIRMED" | "FINAL" | "NEEDS-ACTION"
        )
    }) {
        content.append(&row("dialog-information-symbolic", &status.to_lowercase()));
    }
    if let Some(location) = occ.location.as_deref().filter(|l| !l.is_empty()) {
        content.append(&row("mark-location-symbolic", location));
    }
    // ways to join (RFC 7986 CONFERENCE), each opening its link
    for c in &occ.conferences {
        let (icon, what) = if c.has("VIDEO") {
            ("camera-web-symbolic", "Join video call")
        } else if c.has("PHONE") || c.uri.to_ascii_lowercase().starts_with("tel:") {
            ("call-start-symbolic", "Dial in")
        } else if c.has("CHAT") {
            ("chat-message-new-symbolic", "Open chat")
        } else if c.has("AUDIO") {
            ("audio-headphones-symbolic", "Join call")
        } else {
            ("web-browser-symbolic", "Join online")
        };
        let shown = c
            .uri
            .split_once(':')
            .map_or(c.uri.as_str(), |(scheme, rest)| {
                if scheme.eq_ignore_ascii_case("tel") {
                    rest
                } else {
                    rest.trim_start_matches('/')
                }
            });
        let text = match &c.label {
            Some(label) => format!("{label}: {shown}"),
            None => format!("{what}: {shown}"),
        };
        let button = gtk::Button::builder()
            .child(&row(icon, &text))
            .css_classes(["flat"])
            .halign(gtk::Align::Start)
            .tooltip_text(&c.uri)
            .build();
        let uri = c.uri.clone();
        button.connect_clicked(move |b| {
            close();
            open_uri(b.root().and_downcast_ref::<gtk::Window>(), &uri);
        });
        content.append(&button);
    }
    // where it sits among tasks: its parent, and how its subtasks are going
    if occ.kind == Kind::Todo {
        let tasks = state.vdir.tasks();
        if let Some(parent) = occ.parent.as_ref().and_then(|p| {
            tasks
                .iter()
                .find(|t| t.uid == *p && t.calendar == occ.calendar)
        }) {
            // a link up to it
            let up = gtk::Button::builder()
                .child(&row(
                    "go-up-symbolic",
                    &format!("Subtask of “{}”", parent.summary),
                ))
                .css_classes(["flat"])
                .halign(gtk::Align::Start)
                .tooltip_text("Go to the parent task (^)")
                .action_name("win.run")
                .action_target(&"up".to_variant())
                .build();
            content.append(&up);
        }
        let kids = tasks
            .iter()
            .filter(|t| t.calendar == occ.calendar && t.parent.as_deref() == Some(occ.uid.as_str()))
            .collect::<Vec<_>>();
        if !kids.is_empty() {
            let done = kids.iter().filter(|k| k.is_done()).count();
            let names = kids
                .iter()
                .map(|k| format!("{} {}", if k.is_done() { "✓" } else { "○" }, k.summary))
                .collect::<Vec<_>>()
                .join("\n");
            let r = row(
                "view-list-symbolic",
                &format!("{done} of {} subtasks done", kids.len()),
            );
            r.set_tooltip_text(Some(&names));
            content.append(&r);
        }
    }
    if let Some(invite) = state.invitation(occ) {
        let mut text = format!("Invitation from {}", state.name_of(invite.from));
        if let Some(reply) = invite.reply {
            text += &format!(" · you: {}", format::reply(reply));
        }
        let r = row("mail-unread-symbolic", &text);
        if invite.needs_reply() {
            r.add_css_class("warning");
        }
        content.append(&person_link(&r, invite.from, &state.name_of(invite.from)));
        // answer it here too (the invitations view has these on each row)
        if state.view != ViewKind::Invitations {
            let answers = gtk::Box::builder()
                .css_classes(["linked"])
                .halign(gtk::Align::Start)
                .margin_start(24)
                .build();
            for (label, status, cmd, key) in [
                ("Accept", "ACCEPTED", "accept", "y"),
                ("Maybe", "TENTATIVE", "maybe", "Y"),
                ("Decline", "DECLINED", "decline", "N"),
            ] {
                // the answer given shows pressed
                let b = gtk::ToggleButton::builder()
                    .label(label)
                    .active(invite.reply == Some(status))
                    .tooltip_text(format!("{label} ({key})"))
                    .action_name("win.run")
                    .action_target(&cmd.to_variant())
                    .build();
                answers.append(&b);
            }
            content.append(&answers);
        }
    } else if let Some(organizer) = &occ.organizer
        && !occ.attendees.is_empty()
    {
        content.append(&person_link(
            &row(
                "avatar-default-symbolic",
                &format!("Organised by {}", state.name_of(organizer)),
            ),
            organizer,
            &state.name_of(organizer),
        ));
    }
    if !occ.attendees.is_empty() {
        content.append(&people(occ, state));
    }
    // joining and the map go with the other actions at the bottom
    let window = parent.as_ref().root().and_downcast::<gtk::Window>();
    let join = format::join_url(occ).map(|url| {
        let b = gtk::Button::builder()
            .label("Join")
            .tooltip_text(format!("Open {url} (O)"))
            .css_classes(["suggested-action"])
            .build();
        let w = window.clone();
        b.connect_clicked(move |_| open_uri(w.as_ref(), &url));
        b
    });
    let map = format::place(occ).map(str::to_owned).map(|place| {
        let b = gtk::Button::builder()
            .label("Map")
            .tooltip_text("Show on a map (M)")
            .build();
        let w = window.clone();
        b.connect_clicked(move |_| open_uri(w.as_ref(), &format::map_url(&place)));
        b
    });
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

    // what you can do with it, as the commands its keys run
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(6)
        .build();
    let action = |label: &str, icon: Option<&str>, tip: &str, cmd: &str| {
        let b = gtk::Button::builder()
            .tooltip_text(tip)
            .action_name("win.run")
            .action_target(&cmd.to_variant())
            .build();
        match icon {
            Some(icon) => b.set_icon_name(icon),
            None => b.set_label(label),
        }
        b
    };
    if let Some(join) = &join {
        actions.append(join);
    }
    actions.append(&action("Edit", None, "Edit (e, or double-click)", "edit"));
    if let Some(map) = &map {
        actions.append(map);
    }
    if occ.kind == Kind::Todo {
        let done = format::is_done(occ);
        actions.append(&action(
            if done { "Not done" } else { "Done" },
            None,
            "Mark done or not (D)",
            "done",
        ));
        let sub = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add a subtask (:subtask)")
            .action_name("win.prompt")
            .action_target(&"subtask ".to_variant())
            .build();
        actions.append(&sub);
    }
    // from a list, its place among the days around it
    let in_days = matches!(
        state.view,
        ViewKind::Day | ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_) | ViewKind::Month
    );
    if !in_days && has_day(occ, state) {
        actions.append(&action(
            "Show in calendar",
            Some("find-location-symbolic"),
            "Show in the calendar (v)",
            "reveal",
        ));
    }
    let spacer = gtk::Box::builder().hexpand(true).build();
    actions.append(&spacer);
    // not for events with guests: removing the original would tell them
    if occ.kind != Kind::Journal && occ.attendees.is_empty() {
        let (icon, tip) = if occ.kind == Kind::Todo {
            (
                "x-office-calendar-symbolic",
                "Make it an event (:convert, Ctrl+T)",
            )
        } else {
            (
                "checkbox-checked-symbolic",
                "Make it a task (:convert, Ctrl+T)",
            )
        };
        let convert = action(tip, Some(icon), tip, "convert");
        convert.add_css_class("flat");
        actions.append(&convert);
    }
    let raw = action(
        "Raw",
        Some("text-x-generic-symbolic"),
        "Show the .ics file (R)",
        "raw",
    );
    raw.add_css_class("flat");
    actions.append(&raw);
    let delete = action(
        "Delete",
        Some("user-trash-symbolic"),
        "Delete (x)",
        "delete",
    );
    delete.add_css_class("flat");
    actions.append(&delete);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&actions);

    close();
    // beside a spot on a calendar; under a row of a list, which spans the
    // width and leaves no room beside it
    let position = if parent.as_ref().is::<gtk::ListBoxRow>() {
        gtk::PositionType::Bottom
    } else {
        gtk::PositionType::Right
    };
    // Not modal: keys keep going to the window, so j/k/e/x still work while
    // it is open. It closes on the next change or with Escape.
    let popover = gtk::Popover::builder()
        .child(&content)
        .pointing_to(&rect)
        .position(position)
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

/// Open the editor on a double-click where `on_item` says there is an
/// occurrence (the first click has selected it by then).
pub fn edit_on_double_click(
    widget: &impl IsA<gtk::Widget>,
    on_item: impl Fn(f64, f64) -> bool + 'static,
) {
    let click = gtk::GestureClick::new();
    let w = widget.as_ref().downgrade();
    click.connect_released(move |_, n_press, x, y| {
        if n_press != 2 || !on_item(x, y) {
            return;
        }
        let w = w.clone();
        // after the release has finished opening the details
        glib::idle_add_local_once(move || {
            close();
            if let Some(w) = w.upgrade() {
                let _ = w.activate_action("win.run", Some(&"edit".to_variant()));
            }
        });
    });
    widget.as_ref().add_controller(click);
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

/// Who is invited and how they replied: a count by reply, then a line per
/// person (the first few, unless expanded).
fn people(occ: &Occurrence, state: &State) -> gtk::Box {
    let me = &state.my_addresses();
    const SHOWN: usize = 6;
    let attendees = &occ.attendees;
    let count = |s: &str| {
        attendees
            .iter()
            .filter(|a| a.status.as_deref().unwrap_or("NEEDS-ACTION") == s)
            .count()
    };
    let mut tally = vec![format!(
        "{} {}",
        attendees.len(),
        if attendees.len() == 1 {
            "guest"
        } else {
            "guests"
        }
    )];
    for (status, word) in [
        ("ACCEPTED", "going"),
        ("TENTATIVE", "maybe"),
        ("DECLINED", "not going"),
        ("NEEDS-ACTION", "not answered"),
    ] {
        let n = count(status);
        if n > 0 {
            tally.push(format!("{n} {word}"));
        }
    }
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.append(&row("system-users-symbolic", &tally.join(" · ")));
    let list = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(2)
        .margin_start(24)
        .build();
    let line = |a: &rical_core::Person| {
        let l = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let status = a.status.as_deref().unwrap_or("NEEDS-ACTION");
        let icon = gtk::Image::from_icon_name(format::reply_icon(status));
        icon.set_tooltip_text(Some(format::reply(status)));
        l.append(&icon);
        let mut name = state.name_of(a);
        if a.is_any_of(me) {
            name += " (you)";
        }
        if occ.organizer.as_ref().is_some_and(|o| o.email == a.email) {
            name += " · organiser";
        }
        if a.role.as_deref() == Some("OPT-PARTICIPANT") {
            name += " · optional";
        }
        if matches!(a.kind.as_deref(), Some("ROOM" | "RESOURCE")) {
            name += " · room";
        }
        let label = gtk::Label::builder()
            .label(&name)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .max_width_chars(40)
            .css_classes(["caption"])
            .build();
        if let Some(email) = a
            .email
            .as_deref()
            // the address, when a name is shown in its place
            .filter(|e| !name.to_lowercase().starts_with(&e.to_lowercase()))
        {
            label.set_tooltip_text(Some(email));
        }
        l.append(&label);
        person_link(&l, a, &state.name_of(a))
    };
    // me first, then by reply: going, maybe, unanswered, not going
    let rank = |a: &rical_core::Person| {
        let s = a.status.as_deref().unwrap_or("NEEDS-ACTION");
        (
            !a.is_any_of(me),
            ["ACCEPTED", "TENTATIVE", "NEEDS-ACTION", "DECLINED"]
                .iter()
                .position(|x| *x == s)
                .unwrap_or(4),
        )
    };
    let mut sorted = attendees.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|a| rank(a));
    for a in sorted.iter().take(SHOWN) {
        list.append(&line(a));
    }
    b.append(&list);
    if sorted.len() > SHOWN {
        let rest = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .build();
        for a in &sorted[SHOWN..] {
            rest.append(&line(a));
        }
        let more = gtk::Expander::builder()
            .label(format!("{} more", sorted.len() - SHOWN))
            .child(&rest)
            .margin_start(24)
            .css_classes(["caption"])
            .build();
        b.append(&more);
    }
    b
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

/// Whether `occ` has a day to show it on: undated tasks and notes do not.
fn has_day(occ: &Occurrence, state: &State) -> bool {
    match occ.kind {
        Kind::Todo => state
            .vdir
            .task(&occ.path, &occ.uid)
            .is_some_and(|t| t.day(state.local).is_some()),
        Kind::Journal => state
            .vdir
            .journals()
            .iter()
            .any(|(o, dated)| *dated && o.path == occ.path && o.uid == occ.uid),
        Kind::Event => true,
    }
}

/// `child` as a link to `p`'s card (`:person`), when they have an address.
fn person_link(child: &impl IsA<gtk::Widget>, p: &rical_core::Person, name: &str) -> gtk::Widget {
    let Some(email) = &p.email else {
        return child.clone().upcast();
    };
    gtk::Button::builder()
        .child(child)
        .css_classes(["flat", "person-link"])
        .tooltip_text(format!("About {name}"))
        .action_name("win.run")
        .action_target(&format!("person {email}").to_variant())
        .build()
        .upcast()
}

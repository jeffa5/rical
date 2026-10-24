//! Search: a popover with an entry and live results over events (a year
//! either side of today) and tasks. Enter jumps to the result.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{Days, NaiveDate};
use gtk::{gdk, glib};
use rical_core::{Kind, Occurrence};

use super::Window;
use crate::{
    format,
    layout::{self, ViewKind},
    state::OccurrenceKey,
};

const MAX_RESULTS: usize = 100;

#[derive(Clone)]
enum Hit {
    Occurrence(Box<Occurrence>),
    /// A task without a date: (file, UID).
    Task(std::path::PathBuf, String),
}

pub fn open(window: &Rc<Window>, text: &str) {
    crate::details::close();
    let entry = gtk::SearchEntry::builder()
        .hexpand(true)
        .placeholder_text("Search titles, places and notes")
        .build();
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .can_focus(false)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(440)
        .build();
    let status = gtk::Label::builder()
        .css_classes(["dim-label", "caption"])
        .xalign(0.0)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .width_request(600)
        .build();
    content.append(&entry);
    content.append(&status);
    content.append(&scroller);
    let popover = gtk::Popover::builder()
        .child(&content)
        .position(gtk::PositionType::Bottom)
        .has_arrow(false)
        .build();
    popover.set_parent(&window.title);
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });

    let hits: Rc<RefCell<Vec<Hit>>> = Default::default();

    let update = {
        let (w, list, status, hits) = (
            Rc::downgrade(window),
            list.clone(),
            status.clone(),
            hits.clone(),
        );
        move |query: &str| {
            let Some(w) = w.upgrade() else { return };
            while let Some(row) = list.row_at_index(0) {
                list.remove(&row);
            }
            let found = find(&w, query);
            status.set_label(&match (query.trim().is_empty(), found.len()) {
                (true, _) => "Type to search".to_owned(),
                (false, 0) => "Nothing found".to_owned(),
                (false, n) if n >= MAX_RESULTS => format!("First {n} matches"),
                (false, n) => format!("{n} matches"),
            });
            let state = w.state.borrow();
            for hit in &found {
                list.append(&row(hit, &state));
            }
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
            *hits.borrow_mut() = found;
        }
    };
    let update = Rc::new(update);

    let u = update.clone();
    entry.connect_search_changed(move |e| u(&e.text()));

    let jump = {
        let (w, popover, hits) = (Rc::downgrade(window), popover.clone(), hits.clone());
        Rc::new(move |i: usize| {
            let Some(w) = w.upgrade() else { return };
            let Some(hit) = hits.borrow().get(i).cloned() else {
                return;
            };
            popover.popdown();
            w.jump_to(hit);
        })
    };

    let j = jump.clone();
    list.connect_row_activated(move |_, row| j(row.index() as usize));

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (l, j, p) = (list.clone(), jump.clone(), popover.clone());
    keys.connect_key_pressed(move |_, key, _, _| {
        let selected = l.selected_row().map_or(-1, |r| r.index());
        let move_to = |i: i32| {
            if let Some(row) = l.row_at_index(i) {
                l.select_row(Some(&row));
                row.grab_focus();
            }
        };
        match key {
            gdk::Key::Down | gdk::Key::Tab => move_to(selected + 1),
            gdk::Key::Up | gdk::Key::ISO_Left_Tab => move_to((selected - 1).max(0)),
            gdk::Key::Return | gdk::Key::KP_Enter => j(selected.max(0) as usize),
            gdk::Key::Escape => p.popdown(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    entry.add_controller(keys);

    entry.set_text(text);
    update(text);
    popover.popup();
    entry.grab_focus();
    entry.set_position(-1);
}

fn matches(o: &Occurrence, q: &str) -> bool {
    [
        Some(&o.summary),
        o.location.as_ref(),
        o.description.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|t| t.to_lowercase().contains(q))
}

/// Matches for `query`: upcoming first (soonest first), then past (most
/// recent first), then undated tasks.
fn find(w: &Window, query: &str) -> Vec<Hit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let state = w.state.borrow();
    let today = state.today();
    let local = state.local;
    let days = |d: NaiveDate| [d - Days::new(366), d + Days::new(366)];
    let range = layout::days_range(&days(today), local);
    let mut occ = state
        .vdir
        .query(&range, &[Kind::Event, Kind::Todo], local)
        .into_iter()
        .filter(|o| !state.hidden.contains(&o.calendar) && matches(o, &q))
        .collect::<Vec<_>>();
    // one entry per repeating series: its next occurrence, else its last
    occ.sort_by_key(|o| {
        let d = o.start.date(local);
        (
            d < today,
            if d >= today {
                d
            } else {
                NaiveDate::MAX - (d - NaiveDate::MIN)
            },
        )
    });
    let mut seen = std::collections::HashSet::new();
    let mut hits = occ
        .into_iter()
        .filter(|o| seen.insert((o.path.clone(), o.uid.clone())))
        .map(|o| Hit::Occurrence(Box::new(o)))
        .collect::<Vec<_>>();
    for t in state.vdir.tasks() {
        if t.due.is_none()
            && t.start.is_none()
            && !state.hidden.contains(&t.calendar)
            && t.summary.to_lowercase().contains(&q)
        {
            hits.push(Hit::Task(t.path, t.uid));
        }
    }
    hits.truncate(MAX_RESULTS);
    hits
}

fn row(hit: &Hit, state: &crate::state::State) -> gtk::ListBoxRow {
    let local = state.local;
    let (when, title, calendar, place) = match hit {
        Hit::Occurrence(o) => (
            format::time_range(o, local),
            format::title(o, local),
            o.calendar.clone(),
            o.location.clone().unwrap_or_default(),
        ),
        Hit::Task(path, uid) => {
            let t = state.vdir.task(path, uid);
            (
                "no date".to_owned(),
                t.as_ref()
                    .map_or_else(String::new, |t| format!("☐ {}", t.summary)),
                t.map(|t| t.calendar).unwrap_or_default(),
                String::new(),
            )
        }
    };
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    b.append(&format::color_dot(state.color(&calendar), 10));
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.append(
        &gtk::Label::builder()
            .label(&title)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build(),
    );
    let sub = if place.is_empty() {
        when
    } else {
        format!("{when} · {place}")
    };
    text.append(
        &gtk::Label::builder()
            .label(&sub)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .build(),
    );
    b.append(&text);
    gtk::ListBoxRow::builder()
        .child(&b)
        .focusable(false)
        .build()
}

impl Window {
    /// Show a search result: its day (in a dated view) with it selected,
    /// or undated tasks in the tasks view.
    fn jump_to(self: &Rc<Self>, hit: Hit) {
        {
            let mut s = self.state.borrow_mut();
            match hit {
                Hit::Occurrence(o) => {
                    if matches!(s.view, ViewKind::Tasks) {
                        s.view = ViewKind::Week;
                    }
                    s.date = o.start.date(s.local);
                    s.selected = Some(OccurrenceKey::of(&o));
                }
                Hit::Task(path, uid) => {
                    s.view = ViewKind::Tasks;
                    s.selected_task = Some((path, uid));
                }
            }
        }
        self.refresh();
        self.grid.reveal_selected();
        self.focus_view();
    }
}

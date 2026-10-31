//! The agenda view: a list of upcoming occurrences grouped by day. Rows
//! are made as they are wanted: a screenful first, more as the list is
//! scrolled or the selection moves near the last made, as thirty busy days
//! can be thousands.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{Days, NaiveDate, Utc};
use rical_core::{Kind, Occurrence};

use crate::{
    details, format, layout, listhead,
    state::{OccurrenceKey, SharedState},
};

/// Rows made at first: more than a tall screen shows.
const FIRST: usize = 60;
/// Rows made at a time after.
const CHUNK: usize = 100;
/// How close to the last row made the selection may come before more are.
const AHEAD: usize = 30;

pub struct AgendaView {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    headings: listhead::Headings,
    state: SharedState,
    /// Occurrence key of each row (made or not), None for day headers.
    rows: Rc<RefCell<Vec<Option<OccurrenceKey>>>>,
    /// What each row shows: its occurrence, and the heading above it.
    items: RefCell<Vec<(usize, Option<String>)>>,
    occurrences: RefCell<Rc<Vec<Occurrence>>>,
    /// How many of the rows are made.
    made: std::cell::Cell<usize>,
    syncing: Rc<std::cell::Cell<bool>>,
    on_toggle: Rc<dyn Fn((std::path::PathBuf, String), bool)>,
    /// What the rows were built from: (generation, first day, today).
    built: std::cell::Cell<Option<(u64, NaiveDate, NaiveDate)>>,
}

impl AgendaView {
    pub fn new(
        state: SharedState,
        on_change: Rc<dyn Fn()>,
        on_toggle: Rc<dyn Fn((std::path::PathBuf, String), bool)>,
    ) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar", "rical-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Agenda")]);
        let clamp = adw::Clamp::builder()
            .maximum_size(760)
            .child(&list)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("x-office-calendar-symbolic")
            .title("Nothing coming up")
            .description(format!(
                "No events in the next {} days",
                layout::AGENDA_DAYS
            ))
            .build();

        let widget = gtk::Stack::new();
        widget.add_named(&scroller, Some("list"));
        widget.add_named(&empty, Some("empty"));

        let this = Rc::new(Self {
            widget,
            headings: listhead::install(&list),
            list,
            state,
            rows: Default::default(),
            items: Default::default(),
            occurrences: Default::default(),
            made: Default::default(),
            syncing: Default::default(),
            on_toggle,
            built: Default::default(),
        });

        // more rows when scrolled near the end of those made
        let weak = Rc::downgrade(&this);
        scroller.vadjustment().connect_value_changed(move |adj| {
            let Some(this) = weak.upgrade() else { return };
            if adj.value() + adj.page_size() * 2.0 >= adj.upper() {
                this.make(this.made.get() + CHUNK);
            }
        });

        let weak = Rc::downgrade(&this);
        let changed = on_change.clone();
        this.list.connect_row_selected(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            // and when the selection comes near the last made
            if let Some(r) = row {
                let i = r.index() as usize;
                if i + AHEAD >= this.made.get() {
                    let this = this.clone();
                    gtk::glib::idle_add_local_once(move || this.make(i + AHEAD + CHUNK));
                }
            }
            if this.syncing.get() {
                return;
            }
            let key = row.and_then(|r| {
                this.rows
                    .borrow()
                    .get(r.index() as usize)
                    .cloned()
                    .flatten()
            });
            if let Some(key) = key {
                let mut s = this.state.borrow_mut();
                // something begun earlier (a weekend away) focuses the first
                // day shown rather than moving the list back to its start
                let first = s.visible_days()[0];
                s.date = key.start.date(s.local).max(first);
                s.selected = Some(key);
                drop(s);
                changed();
            }
        });

        let weak = Rc::downgrade(&this);
        this.list.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            let state = this.state.borrow();
            if let Some(occ) = state.selected_occurrence() {
                let rect = gtk::gdk::Rectangle::new(0, 0, row.width(), row.height());
                details::show(row, rect, &occ, &state);
            }
        });
        let list = this.list.downgrade();
        details::edit_on_double_click(&this.list, move |_, y| {
            list.upgrade()
                .and_then(|l| l.row_at_y(y as i32))
                .is_some_and(|r| r.is_activatable())
        });
        this
    }

    /// Rebuild the rows if the occurrences may have changed, otherwise just
    /// show the selection (rebuilding would lose keyboard focus).
    pub fn refresh(&self) {
        let key = {
            let s = self.state.borrow();
            (s.generation(), s.visible_days()[0], s.today())
        };
        if self.built.get() != Some(key) {
            let had_focus = self.list.focus_child().is_some();
            self.rebuild();
            self.built.set(Some(key));
            if had_focus {
                self.focus();
            }
            return;
        }
        let index = {
            let s = self.state.borrow();
            self.rows
                .borrow()
                .iter()
                .position(|r| r.as_ref().is_some_and(|k| s.selected.as_ref() == Some(k)))
        };
        self.syncing.set(true);
        if let Some(i) = index {
            self.make(i + AHEAD);
        }
        match index.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => {
                self.list.select_row(Some(&row));
                listhead::reveal(&row);
            }
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
    }

    /// Work out the rows (without making them), then make those the first
    /// screen, and the selection, need.
    fn rebuild(&self) {
        self.syncing.set(true);
        self.headings.borrow_mut().clear();
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        self.made.set(0);
        let (occ, items, rows, selected) = {
            let state = self.state.borrow();
            let days = state.visible_days();
            let occ = state.occurrences(&days);
            let today = Utc::now().with_timezone(&state.local).date_naive();
            // each occurrence on its first visible day (one begun earlier
            // shows on the first)
            let mut by_day: Vec<Vec<usize>> = vec![Vec::new(); days.len()];
            for (i, o) in occ.iter().enumerate() {
                let first = o.start.date(state.local).max(days[0]);
                if let Ok(d) = days.binary_search(&first) {
                    by_day[d].push(i);
                }
            }
            let mut items = Vec::new();
            let mut rows = Vec::new();
            let mut selected = None;
            for (day, mut on_day) in days.iter().zip(by_day) {
                if on_day.is_empty() {
                    continue;
                }
                // all-day items (and tasks carried over) first, then by time
                on_day.sort_by_key(|&i| {
                    let o = &occ[i];
                    (!layout::is_all_day_row(o), o.start.to_utc(state.local))
                });
                let title = if *day == today {
                    format!("Today · {}", format::day(*day))
                } else if *day == today + Days::new(1) {
                    format!("Tomorrow · {}", format::day(*day))
                } else {
                    day.format("%A %-d %B").to_string()
                };
                // the heading sits on the day's first row, not a row of its
                // own, so keyboard navigation goes straight through
                let mut heading = Some(title);
                for i in on_day {
                    let o = &occ[i];
                    if state.selected.as_ref().is_some_and(|k| k.matches(o)) {
                        selected = Some(rows.len());
                    }
                    items.push((i, heading.take()));
                    rows.push(Some(OccurrenceKey::of(o)));
                }
            }
            (occ, items, rows, selected)
        };
        self.widget
            .set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        *self.rows.borrow_mut() = rows;
        *self.items.borrow_mut() = items;
        *self.occurrences.borrow_mut() = occ;
        self.make(selected.map_or(0, |i| i + AHEAD).max(FIRST));
        let row = selected.and_then(|i| self.list.row_at_index(i as i32));
        self.list.select_row(row.as_ref());
        if let Some(row) = &row {
            listhead::reveal(row);
        }
        self.syncing.set(false);
    }

    /// Make the rows up to `n` (of those there are).
    fn make(&self, n: usize) {
        let items = self.items.borrow();
        let n = n.min(items.len());
        let from = self.made.get();
        if n <= from {
            return;
        }
        let occ = self.occurrences.borrow().clone();
        let state = self.state.borrow();
        let syncing = self.syncing.replace(true);
        for (i, heading) in &items[from..n] {
            let row = self.row(&occ[*i], &state);
            self.headings
                .borrow_mut()
                .push(heading.as_deref().map(|t| listhead::label(t).upcast()));
            self.list.append(&row);
        }
        self.syncing.set(syncing);
        self.made.set(n);
    }

    pub fn focus(&self) {
        let row = self.list.selected_row().or_else(|| {
            (0..)
                .map_while(|i| self.list.row_at_index(i))
                .find(|r| r.is_selectable())
        });
        match row {
            Some(row) => {
                // what looks chosen is what commands act on
                if self.list.selected_row().is_none() {
                    self.list.select_row(Some(&row));
                }
                row.grab_focus();
            }
            None => {
                self.list.grab_focus();
            }
        }
    }

    pub fn show_selected_details(&self) {
        if let Some(row) = self.list.selected_row() {
            row.activate();
        }
    }

    fn row(&self, o: &Occurrence, state: &crate::state::State) -> gtk::ListBoxRow {
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        // the times on the left, start over end; the day is the heading's
        let local = state.local;
        let day = o.start.date(local);
        let all_day = layout::is_all_day_row(o);
        let times = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .build();
        times.append(
            &gtk::Label::builder()
                .label(if all_day {
                    "all day".to_owned()
                } else {
                    format::time(&o.start, local)
                })
                .width_chars(7)
                .xalign(0.0)
                .css_classes(["numeric", "dim-label"])
                .build(),
        );
        if !all_day && o.end != o.start {
            let end = if o.end.date(local) == day {
                format::time(&o.end, local)
            } else {
                format!("→ {}", o.end.to_tz(local).format("%a"))
            };
            times.append(
                &gtk::Label::builder()
                    .label(&end)
                    .xalign(0.0)
                    .css_classes(["numeric", "dim-label", "caption"])
                    .build(),
            );
        }
        b.append(&times);
        // tasks' checks in a column of their own, so titles line up
        if o.kind == Kind::Todo {
            let id = (o.path.clone(), o.uid.clone());
            let toggle = self.on_toggle.clone();
            b.append(&format::task_check(
                format::is_done(o),
                Some(Box::new(move |done| toggle(id.clone(), done))),
            ));
        } else {
            b.append(&gtk::Box::builder().width_request(18).build());
        }
        b.append(&format::color_dot(state.display_color(o), 10));

        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .valign(gtk::Align::Center)
            .build();
        let title = match o.kind {
            Kind::Todo | Kind::Journal => format::label(o, local),
            _ => o.summary.clone(),
        };
        let title = gtk::Label::builder()
            .label(&title)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        if format::is_inactive(o) {
            title.add_css_class("dim-label");
        }
        text.append(&title);
        // what the heading and times do not say: how long it runs on for,
        // where, or that it is overdue
        let mut sub = Vec::new();
        if all_day {
            let last = o.end.date(local).pred_opt().unwrap_or(day);
            if last > day {
                sub.push(format!("until {}", format::day(last)));
            }
        }
        if let Some(loc) = o.location.as_deref().filter(|l| !l.is_empty()) {
            sub.push(loc.to_owned());
        }
        let overdue = format::is_overdue(o, local);
        if overdue && let Some(due) = o.due {
            sub = vec![format!("overdue, was due {}", format::day(due.date(local)))];
        }
        let sub = gtk::Label::builder()
            .label(sub.join(" · "))
            .visible(!sub.is_empty())
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", if overdue { "error" } else { "dim-label" }])
            .build();
        text.append(&sub);
        b.append(&text);
        gtk::ListBoxRow::builder().child(&b).build()
    }
}

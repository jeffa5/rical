//! The agenda view: a list of upcoming occurrences grouped by day.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{Days, NaiveDate, Utc};
use rical_core::{Kind, Occurrence};

use crate::{
    details, format, layout, listhead,
    state::{OccurrenceKey, SharedState},
};

pub struct AgendaView {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    headings: listhead::Headings,
    state: SharedState,
    /// Occurrence key of each row, None for day headers.
    rows: Rc<RefCell<Vec<Option<OccurrenceKey>>>>,
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
            .css_classes(["navigation-sidebar"])
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
            syncing: Default::default(),
            on_toggle,
            built: Default::default(),
        });

        let weak = Rc::downgrade(&this);
        let changed = on_change.clone();
        this.list.connect_row_selected(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
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
                s.date = key.start.date(s.local);
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
        match index.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
    }

    fn rebuild(&self) {
        self.syncing.set(true);
        self.headings.borrow_mut().clear();
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let state = self.state.borrow();
        let days = state.visible_days();
        let occ = state.occurrences(&days);
        let today = Utc::now().with_timezone(&state.local).date_naive();

        let mut rows = Vec::new();
        let mut selected_row = None;
        for day in &days {
            let mut on_day = occ
                .iter()
                .filter(|o| {
                    // multi-day occurrences show on their first visible day
                    let first = o.start.date(state.local).max(days[0]);
                    first == *day
                })
                .collect::<Vec<_>>();
            // all-day items (and tasks carried over) first, then by time
            on_day.sort_by_key(|o| (!layout::is_all_day_row(o), o.start.to_utc(state.local)));
            if on_day.is_empty() {
                continue;
            }
            let title = if *day == today {
                format!("Today · {}", format::day(*day))
            } else if *day == today + Days::new(1) {
                format!("Tomorrow · {}", format::day(*day))
            } else {
                day.format("%A %-d %B").to_string()
            };
            // the heading sits on the day's first row, not a row of its own,
            // so keyboard navigation goes straight through
            let mut heading = Some(listhead::label(&title));
            for o in on_day {
                let row = self.row(o, &state);
                self.headings
                    .borrow_mut()
                    .push(heading.take().map(|l| l.upcast()));
                if state.selected.as_ref().is_some_and(|k| k.matches(o)) {
                    selected_row = Some(row.clone());
                }
                self.list.append(&row);
                rows.push(Some(OccurrenceKey::of(o)));
            }
        }
        self.widget
            .set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        *self.rows.borrow_mut() = rows;
        self.list.select_row(selected_row.as_ref());
        self.syncing.set(false);
    }

    pub fn focus(&self) {
        let row = self.list.selected_row().or_else(|| {
            (0..)
                .map_while(|i| self.list.row_at_index(i))
                .find(|r| r.is_selectable())
        });
        match row {
            Some(row) => {
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
        let when = if layout::is_all_day_row(o) {
            "all day".to_owned()
        } else {
            format::time(&o.start, state.local)
        };
        b.append(
            &gtk::Label::builder()
                .label(&when)
                .width_chars(7)
                .xalign(0.0)
                .css_classes(["numeric", "dim-label"])
                .build(),
        );
        if o.kind == Kind::Todo {
            let id = (o.path.clone(), o.uid.clone());
            let toggle = self.on_toggle.clone();
            let check = format::task_check(
                format::is_done(o),
                Some(Box::new(move |done| toggle(id.clone(), done))),
            );
            b.append(&check);
            b.append(&format::color_dot(state.display_color(o), 10));
        } else {
            b.append(&format::color_dot(state.display_color(o), 10));
        }

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = match o.kind {
            Kind::Todo => format::label(o, state.local),
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
        let mut sub = format::time_range(o, state.local);
        if let Some(loc) = o.location.as_deref().filter(|l| !l.is_empty()) {
            sub = format!("{sub} · {loc}");
        }
        let overdue = format::is_overdue(o, state.local);
        if overdue && let Some(due) = o.due {
            sub = format!("overdue, was due {}", format::day(due.date(state.local)));
        }
        let sub = gtk::Label::builder()
            .label(&sub)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", if overdue { "error" } else { "dim-label" }])
            .build();
        text.append(&sub);
        b.append(&text);
        gtk::ListBoxRow::builder().child(&b).build()
    }
}

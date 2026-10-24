//! The agenda view: a list of upcoming occurrences grouped by day.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{Days, Utc};
use rical_core::{Kind, Occurrence};

use crate::{
    details, format, layout,
    state::{OccurrenceKey, SharedState},
};

pub struct AgendaView {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    state: SharedState,
    /// Occurrence key of each row, None for day headers.
    rows: Rc<RefCell<Vec<Option<OccurrenceKey>>>>,
    syncing: Rc<std::cell::Cell<bool>>,
}

impl AgendaView {
    pub fn new(state: SharedState, on_change: Rc<dyn Fn()>) -> Rc<Self> {
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
            list,
            state,
            rows: Default::default(),
            syncing: Default::default(),
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

    pub fn refresh(&self) {
        self.syncing.set(true);
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
            let on_day = occ
                .iter()
                .filter(|o| {
                    // multi-day occurrences show on their first visible day
                    let first = o.start.date(state.local).max(days[0]);
                    first == *day
                })
                .collect::<Vec<_>>();
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
            let header = gtk::ListBoxRow::builder()
                .activatable(false)
                .selectable(false)
                .child(
                    &gtk::Label::builder()
                        .label(&title)
                        .css_classes(["heading"])
                        .xalign(0.0)
                        .margin_top(12)
                        .margin_bottom(4)
                        .build(),
                )
                .build();
            self.list.append(&header);
            rows.push(None);

            for o in on_day {
                let row = self.row(o, &state);
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
        b.append(&format::color_dot(state.color(&o.calendar), 10));

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = match o.kind {
            Kind::Todo => format!("{} {}", format::todo_mark(o), o.summary),
            _ => o.summary.clone(),
        };
        let title = gtk::Label::builder()
            .label(&title)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        if format::is_cancelled(o) {
            title.add_css_class("dim-label");
        }
        text.append(&title);
        let mut sub = format::time_range(o, state.local);
        if let Some(loc) = o.location.as_deref().filter(|l| !l.is_empty()) {
            sub = format!("{sub} · {loc}");
        }
        text.append(
            &gtk::Label::builder()
                .label(&sub)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        b.append(&text);
        gtk::ListBoxRow::builder().child(&b).build()
    }
}

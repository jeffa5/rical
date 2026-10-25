//! Open tasks in the sidebar, next to the calendar: overdue, due today and
//! undated. Drag one onto the time grid to schedule it, or onto a day's
//! all-day row to make it due that day.

use std::{cell::Cell, rc::Rc};

use adw::prelude::*;
use chrono::NaiveDate;
use gtk::gdk;
use rical_core::Task;

use crate::{format, state::SharedState};

/// How many tasks the pane lists at most.
const SHOWN: usize = 12;

/// A dragged task is carried as "file\nUID".
pub fn encode(t: &Task) -> String {
    format!("{}\n{}", t.path.display(), t.uid)
}

pub fn decode(s: &str) -> Option<(std::path::PathBuf, String)> {
    let (path, uid) = s.split_once('\n')?;
    Some((path.into(), uid.to_owned()))
}

pub struct SideTasks {
    pub widget: gtk::Box,
    list: gtk::ListBox,
    state: SharedState,
    built: Cell<Option<(u64, NaiveDate)>>,
    on_toggle: Rc<dyn Fn((std::path::PathBuf, String), bool)>,
}

impl SideTasks {
    pub fn new(
        state: SharedState,
        on_toggle: Rc<dyn Fn((std::path::PathBuf, String), bool)>,
    ) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["navigation-sidebar"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Tasks to schedule")]);
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&list);
        Rc::new(Self {
            widget,
            list,
            state,
            built: Cell::new(None),
            on_toggle,
        })
    }

    /// Rebuild if the tasks may have changed.
    pub fn refresh(&self) {
        let (key, tasks, today, local) = {
            let s = self.state.borrow();
            let today = s.today();
            let mut tasks = s
                .vdir
                .tasks()
                .into_iter()
                .filter(|t| {
                    !t.is_done()
                        && !s.hidden.contains(&t.calendar)
                        && t.day(s.local).is_none_or(|d| d <= today)
                })
                .collect::<Vec<_>>();
            // overdue first, then today, then undated
            tasks.sort_by_key(|t| {
                (
                    t.day(s.local).is_none(),
                    t.day(s.local),
                    t.priority.unwrap_or(10),
                )
            });
            ((s.generation(), today), tasks, today, s.local)
        };
        if self.built.get() == Some(key) {
            return;
        }
        self.built.set(Some(key));
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        if tasks.is_empty() {
            self.list.append(
                &gtk::ListBoxRow::builder()
                    .activatable(false)
                    .child(
                        &gtk::Label::builder()
                            .label("Nothing open; add a task with A")
                            .css_classes(["dim-label", "caption"])
                            .xalign(0.0)
                            .build(),
                    )
                    .build(),
            );
        }
        let state = self.state.borrow();
        for t in tasks.iter().take(SHOWN) {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let id = (t.path.clone(), t.uid.clone());
            let toggle = self.on_toggle.clone();
            let check =
                format::task_check(false, Some(Box::new(move |done| toggle(id.clone(), done))));
            b.append(&check);
            b.append(&format::color_dot(state.color(&t.calendar), 8));
            let label = gtk::Label::builder()
                .label(&t.summary)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            if t.is_overdue(today, local) {
                label.add_css_class("error");
                label.set_tooltip_text(
                    t.day(local)
                        .map(|d| format!("overdue, was due {}", format::day(d)))
                        .as_deref(),
                );
            }
            b.append(&label);
            let row = gtk::ListBoxRow::builder().child(&b).build();
            row.set_tooltip_text(Some("Drag onto the calendar to schedule"));

            let source = gtk::DragSource::new();
            source.set_actions(gdk::DragAction::COPY);
            let data = encode(t);
            source.connect_prepare(move |_, _, _| {
                Some(gdk::ContentProvider::for_value(&data.to_value()))
            });
            let r = row.clone();
            source.connect_drag_begin(move |src, _| {
                src.set_icon(Some(&gtk::WidgetPaintable::new(Some(&r))), 0, 0);
            });
            row.add_controller(source);
            self.list.append(&row);
        }
        if tasks.len() > SHOWN {
            self.list.append(
                &gtk::ListBoxRow::builder()
                    .activatable(false)
                    .child(
                        &gtk::Label::builder()
                            .label(format!(
                                "{} more in the tasks view (T)",
                                tasks.len() - SHOWN
                            ))
                            .css_classes(["dim-label", "caption"])
                            .xalign(0.0)
                            .build(),
                    )
                    .build(),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        assert_eq!(
            decode("/a/b c.ics\nuid-1"),
            Some(("/a/b c.ics".into(), "uid-1".to_owned()))
        );
        assert_eq!(decode("nope"), None);
    }
}

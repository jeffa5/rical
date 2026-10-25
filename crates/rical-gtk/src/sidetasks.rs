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
                    // what is on now: due by today, started, or undated;
                    // not what starts later
                    let by_today =
                        |w: Option<rical_core::When>| w.is_some_and(|w| w.date(s.local) <= today);
                    !t.is_done()
                        && !s.hidden.contains(&t.calendar)
                        && ((t.start.is_none() && t.due.is_none())
                            || by_today(t.due)
                            || by_today(t.start))
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
        // subtasks under their parent when it is listed too; otherwise
        // marked as a subtask, with the parent named on hover
        let titles = state
            .vdir
            .tasks()
            .into_iter()
            .map(|t| (t.uid, t.summary))
            .collect::<std::collections::HashMap<_, _>>();
        let listed = tasks
            .iter()
            .map(|t| t.uid.as_str())
            .collect::<std::collections::HashSet<_>>();
        let nested = |t: &Task| {
            t.parent
                .as_deref()
                .is_some_and(|p| listed.contains(p) && p != t.uid)
        };
        let mut order = Vec::new();
        let mut stack = tasks
            .iter()
            .filter(|t| !nested(t))
            .rev()
            .map(|t| (t, 0))
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        while let Some((t, depth)) = stack.pop() {
            if !seen.insert(t.uid.as_str()) {
                continue;
            }
            order.push((t, depth));
            let kids = tasks
                .iter()
                .filter(|k| k.parent.as_deref() == Some(t.uid.as_str()) && nested(k));
            stack.extend(kids.rev().map(|k| (k, depth + 1)));
        }
        for (t, depth) in order.into_iter().take(SHOWN) {
            let b = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_start(20 * depth)
                .build();
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
            let mut tip = "Drag onto the calendar to schedule".to_owned();
            if depth == 0
                && let Some(parent) = t.parent.as_ref().and_then(|p| titles.get(p))
            {
                // a subtask without its parent here
                b.insert_child_after(
                    &gtk::Label::builder()
                        .label("↳")
                        .css_classes(["dim-label"])
                        .build(),
                    Some(&check),
                );
                tip = format!("Subtask of “{parent}”\n{tip}");
            }
            row.set_tooltip_text(Some(&tip));

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

//! The tasks view: every task, grouped into overdue, today, upcoming,
//! undated and recently done, with checkboxes to complete them.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
};

use adw::prelude::*;
use chrono::{Days, NaiveDate};
use rical_core::Task;

use crate::{format, listhead, state::SharedState};

/// How many completed tasks to list, most recent first.
const DONE_SHOWN: usize = 15;

type TaskId = (PathBuf, String);

pub struct TasksView {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    headings: listhead::Headings,
    state: SharedState,
    rows: RefCell<Vec<Option<TaskId>>>,
    syncing: Cell<bool>,
    /// What the rows were built from: (generation, today).
    built: Cell<Option<(u64, NaiveDate)>>,
    /// Run when a checkbox is clicked: (task, done).
    on_toggle: Rc<dyn Fn(TaskId, bool)>,
}

impl TasksView {
    pub fn new(
        state: SharedState,
        on_change: Rc<dyn Fn()>,
        on_toggle: Rc<dyn Fn(TaskId, bool)>,
    ) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Tasks")]);
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
            .icon_name("checkbox-checked-symbolic")
            .title("No tasks")
            .description("Add one with c, or :task Buy milk --due fri")
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
            syncing: Cell::new(false),
            built: Cell::new(None),
            on_toggle,
        });

        let weak = Rc::downgrade(&this);
        this.list.connect_row_selected(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            if this.syncing.get() {
                return;
            }
            let id = row.and_then(|r| {
                this.rows
                    .borrow()
                    .get(r.index() as usize)
                    .cloned()
                    .flatten()
            });
            if id.is_some() {
                this.state.borrow_mut().selected_task = id;
                on_change();
            }
        });
        this
    }

    /// Rebuild the rows if the tasks may have changed, otherwise just show
    /// the selection (rebuilding would lose keyboard focus).
    pub fn refresh(&self) {
        let key = {
            let s = self.state.borrow();
            (s.generation(), s.today())
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
        let selected = self.state.borrow().selected_task.clone();
        let index = self
            .rows
            .borrow()
            .iter()
            .position(|r| *r == selected && r.is_some());
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
        let local = state.local;
        let today = state.today();
        let tasks = state
            .vdir
            .tasks()
            .into_iter()
            .filter(|t| !state.hidden.contains(&t.calendar))
            .collect::<Vec<_>>();

        // subtasks are listed under their parent, unless the parent is done
        // and they are not (they would be lost in Done)
        // tasks are known by (calendar, UID): the same UID can be in two
        // calendars (two accounts syncing the same items), and a subtask's
        // parent is in its own calendar
        let key = |t: &Task| (t.calendar.clone(), t.uid.clone());
        let open = tasks
            .iter()
            .filter(|t| !t.is_done())
            .map(key)
            .collect::<HashSet<_>>();
        let all = tasks.iter().map(key).collect::<HashSet<_>>();
        let mut children: HashMap<(String, String), Vec<Task>> = HashMap::new();
        let mut top = Vec::new();
        for t in tasks {
            let parent = t.parent.clone().map(|p| (t.calendar.clone(), p));
            match parent {
                Some(p)
                    if p.1 != t.uid && all.contains(&p) && (open.contains(&p) || t.is_done()) =>
                {
                    children.entry(p).or_default().push(t);
                }
                _ => top.push(t),
            }
        }

        let (done, open): (Vec<_>, Vec<_>) = top.into_iter().partition(Task::is_done);
        let mut groups: [(&str, Vec<Task>); 7] = [
            ("Overdue", Vec::new()),
            ("Today", Vec::new()),
            ("In progress", Vec::new()),
            ("Upcoming", Vec::new()),
            ("Not started yet", Vec::new()),
            ("No date", Vec::new()),
            ("Done", Vec::new()),
        ];
        for t in open {
            groups[group(&t, today, local)].1.push(t);
        }
        let sort_key = |t: &Task| {
            (
                t.due.or(t.start).map(|w| w.to_utc(local)),
                t.priority.unwrap_or(10),
                t.summary.to_lowercase(),
            )
        };
        for (_, g) in &mut groups[..6] {
            g.sort_by_key(sort_key);
        }
        let mut done = done;
        done.sort_by_key(|t| std::cmp::Reverse(t.completed.map(|w| w.to_utc(local))));
        groups[6].1 = done.into_iter().take(DONE_SHOWN).collect();

        let mut rows = Vec::new();
        let mut selected = None;
        for (heading, tasks) in &groups {
            if tasks.is_empty() {
                continue;
            }
            let label = listhead::label(&format!("{heading} · {}", tasks.len()));
            if *heading == "Overdue" {
                label.add_css_class("error");
            }
            // the heading sits on the group's first row, not a row of its
            // own, so keyboard navigation goes straight through
            let mut heading = Some(label);
            // each task, then its subtasks (open first) indented below it
            let mut stack = tasks.iter().rev().map(|t| (t, 0)).collect::<Vec<_>>();
            let mut seen = HashSet::new();
            while let Some((t, depth)) = stack.pop() {
                if !seen.insert(key(t)) {
                    continue; // a loop of parents
                }
                let kids = children.get(&key(t)).map(Vec::as_slice).unwrap_or_default();
                let progress = (!kids.is_empty())
                    .then(|| (kids.iter().filter(|k| k.is_done()).count(), kids.len()));
                let row = self.row(t, today, &state, depth, progress);
                self.headings
                    .borrow_mut()
                    .push(heading.take().map(|l| l.upcast()));
                let id = (t.path.clone(), t.uid.clone());
                if state.selected_task.as_ref() == Some(&id) {
                    selected = Some(row.clone());
                }
                self.list.append(&row);
                rows.push(Some(id));
                let mut kids = kids.iter().collect::<Vec<_>>();
                kids.sort_by_key(|k| (k.is_done(), sort_key(k)));
                stack.extend(kids.into_iter().rev().map(|k| (k, depth + 1)));
            }
        }
        self.widget
            .set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        *self.rows.borrow_mut() = rows;
        self.list.select_row(selected.as_ref());
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
                self.list.select_row(Some(&row));
                row.grab_focus();
            }
            None => {
                self.list.grab_focus();
            }
        }
    }

    /// Select the next or previous task (j and k).
    pub fn step(&self, forward: bool) {
        listhead::step(&self.list, forward);
    }

    pub fn selected_row(&self) -> Option<gtk::ListBoxRow> {
        self.list.selected_row()
    }

    /// A task's row, `depth` levels in as a subtask; `progress` is how many
    /// of its own subtasks are done, of how many.
    fn row(
        &self,
        t: &Task,
        today: NaiveDate,
        state: &crate::state::State,
        depth: usize,
        progress: Option<(usize, usize)>,
    ) -> gtk::ListBoxRow {
        let local = state.local;
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(6 + 28 * depth as i32)
            .margin_end(6)
            .build();
        let id = (t.path.clone(), t.uid.clone());
        let toggle = self.on_toggle.clone();
        let check = format::task_check(
            t.is_done(),
            Some(Box::new(move |done| toggle(id.clone(), done))),
        );
        b.append(&check);
        b.append(&format::color_dot(state.color(&t.calendar), 10));

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let title = gtk::Label::builder()
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        let summary = glib_escape(if t.summary.is_empty() {
            "(untitled)"
        } else {
            &t.summary
        });
        if t.is_done() {
            title.set_markup(&format!("<s>{summary}</s>"));
            title.add_css_class("dim-label");
        } else {
            title.set_markup(&summary);
        }
        text.append(&title);

        let mut sub = Vec::new();
        // "today", "tomorrow" or the day, with the time for a timed date
        let when = |w: rical_core::When| {
            let day = w.date(local);
            let mut s = if day == today {
                "today".to_owned()
            } else if Some(day) == today.checked_add_days(Days::new(1)) {
                "tomorrow".to_owned()
            } else {
                format::day(day)
            };
            if !w.is_date() {
                s += &format!(" {}", format::time(&w, local));
            }
            s
        };
        match t.start {
            Some(s) if s.date(local) > today => sub.push(format!("starts {}", when(s))),
            Some(s) if t.due.is_none() && !t.is_done() => {
                sub.push(format!("started {}", when(s)));
            }
            _ => {}
        }
        if let Some(due) = t.due {
            sub.push(if due.date(local) < today && !t.is_done() {
                format!("overdue, was due {}", when(due))
            } else {
                format!("due {}", when(due))
            });
        }
        if let Some((done, all)) = progress {
            sub.push(format!(
                "{done} of {all} {} done",
                if all == 1 { "subtask" } else { "subtasks" }
            ));
        }
        if t.recurring && !t.is_done() {
            sub.push("repeats".to_owned());
        }
        if let Some(p) = t.priority {
            sub.push(match p {
                1..=4 => "high priority".to_owned(),
                5 => "medium priority".to_owned(),
                _ => "low priority".to_owned(),
            });
        }
        if let Some(done) = t.completed.filter(|_| t.is_done()) {
            sub.push(format!("done {}", format::day(done.date(local))));
        }
        sub.push(
            state
                .calendar(&t.calendar)
                .map_or(t.calendar.clone(), |c| c.name.clone()),
        );
        let sub_label = gtk::Label::builder()
            .label(sub.join(" · "))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .css_classes(["caption", "dim-label"])
            .build();
        if t.is_overdue(today, local) {
            sub_label.remove_css_class("dim-label");
            sub_label.add_css_class("error");
        }
        text.append(&sub_label);
        b.append(&text);
        gtk::ListBoxRow::builder().child(&b).build()
    }
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

/// Which group an open task is in: overdue, today, in progress (started,
/// due later), upcoming (due later), not started yet, or no date.
fn group(t: &Task, today: NaiveDate, local: chrono_tz::Tz) -> usize {
    let start = t.start.map(|w| w.date(local));
    let due = t.due.map(|w| w.date(local));
    match (start, due) {
        (_, Some(d)) if d < today => 0,
        (_, Some(d)) if d == today => 1,
        (Some(s), _) if s > today => 4,
        (Some(s), None) if s == today => 1,
        (Some(_), _) => 2,
        (None, Some(_)) => 3,
        (None, None) => 5,
    }
}

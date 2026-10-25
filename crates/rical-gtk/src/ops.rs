//! Editing from the window: create, edit, move, delete, undo and redo.
//! Each edit asks whether a repeating event changes as a whole or just this
//! once, writes through the vdir, and records the change for undo.

use std::{path::PathBuf, rc::Rc};

use adw::prelude::*;
use chrono::{NaiveTime, TimeDelta, Timelike, Utc};
use gtk::gio;
use log::warn;
use rical_core::{
    Kind, Occurrence, Task, Vdir, When,
    when::resolve_local,
    write::{Change, Draft, Guest, Repeat, Scope},
};

use super::Window;
use crate::{
    editor,
    grid::DragAction,
    layout::{ViewKind, is_all_day_row},
    state::OccurrenceKey,
};

impl Window {
    pub(super) fn on_drag(self: &Rc<Self>, action: DragAction) {
        match action {
            DragAction::Move { occ, start, end } => self.set_times(*occ, start, end),
            DragAction::Create { start, end } => self.new_item(Some((start, end))),
            DragAction::DropTask { task, at } => self.drop_task(&task, at),
        }
    }

    /// For a repeating event ask whether to change this occurrence or all
    /// of them, then run `f`. Non-repeating events go straight to `f`.
    fn with_scope(
        self: &Rc<Self>,
        occ: &Occurrence,
        verb: &str,
        f: impl FnOnce(&Rc<Self>, Scope) + 'static,
    ) {
        let forced = self.forced_scope.take();
        if !occ.recurring {
            f(self, Scope::All);
            return;
        }
        if let Some(scope) = forced {
            f(self, scope);
            return;
        }
        let dialog = adw::AlertDialog::new(
            Some(&format!("{verb} repeating event")),
            Some(&format!(
                "“{}” repeats. Change just this one, or every occurrence?",
                occ.summary
            )),
        );
        dialog.add_responses(&[
            ("cancel", "_Cancel"),
            ("all", "_All events"),
            ("this", "_This event"),
        ]);
        dialog.set_response_appearance("this", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("this"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.choose(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |response| {
                let Some(w) = weak.upgrade() else { return };
                match response.as_str() {
                    "this" => f(&w, Scope::This),
                    "all" => f(&w, Scope::All),
                    _ => w.refresh(),
                }
            },
        );
    }

    /// Run an edit, record it for undo and report failure. Returns whether
    /// it worked.
    fn perform(
        self: &Rc<Self>,
        what: &str,
        f: impl FnOnce(&mut Vdir) -> anyhow::Result<Vec<Change>>,
    ) -> bool {
        let result = f(&mut self.state.borrow_mut().vdir);
        match result {
            Ok(changes) => {
                self.state.borrow_mut().record(changes);
                self.refresh();
                true
            }
            Err(e) => {
                warn!("{what} failed: {e:#}");
                self.toast(&format!("Could not {what}: {e}"));
                self.refresh();
                false
            }
        }
    }

    pub(super) fn toast(&self, text: &str) {
        self.toasts.add_toast(adw::Toast::new(text));
    }

    /// Select the occurrence of `uid` starting at `start`, if visible.
    fn reselect(&self, uid: &str, start: &When) {
        {
            let mut s = self.state.borrow_mut();
            s.selected = s.find(uid, start).map(|o| OccurrenceKey::of(&o));
        }
        self.refresh();
        self.grid.reveal_selected();
    }

    fn set_times(self: &Rc<Self>, occ: Occurrence, start: When, end: When) {
        let o = occ.clone();
        self.with_scope(&o, "Move", move |w, scope| {
            let mut draft = w.state.borrow().vdir.draft(&occ);
            draft.start = start;
            draft.end = end;
            if w.perform("move the event", |v| v.update(&occ, scope, &draft, None)) {
                w.reselect(&occ.uid, &start);
            }
        });
    }

    /// Open the editor for a new event, at `times` or the next free-ish slot
    /// on the focused day.
    pub(super) fn new_item(self: &Rc<Self>, times: Option<(When, When)>) {
        let (start, end) = times.unwrap_or_else(|| self.default_slot());
        let Some(calendar) = self.state.borrow().default_calendar() else {
            self.toast("There are no calendars to add to");
            return;
        };
        self.open_new(Draft::new(start, end), calendar);
    }

    /// Open the editor for a new item starting from `draft`.
    pub(super) fn open_new(self: &Rc<Self>, draft: Draft, calendar: String) {
        let weak = Rc::downgrade(self);
        editor::open(
            &self.window,
            &self.state,
            editor::Options {
                title: "New Event",
                draft,
                calendar,
                is_new: true,
                invitation: false,
            },
            move |draft, calendar| {
                if let Some(w) = weak.upgrade() {
                    w.create(draft, calendar);
                }
            },
            None,
        );
    }

    /// Create an item and select it.
    pub(super) fn create(self: &Rc<Self>, draft: Draft, calendar: String) {
        self.state.borrow_mut().last_calendar = Some(calendar.clone());
        let mut created = None;
        let ok = self.perform("create the event", |v| {
            let change = v.create(&calendar, &draft)?;
            created = Some(change.path.clone());
            Ok(vec![change])
        });
        if ok
            && let Some(uid) = created
                .as_ref()
                .and_then(|p| p.file_stem())
                .map(|s| s.to_string_lossy().into_owned())
        {
            let local = self.state.borrow().local;
            self.state.borrow_mut().date = draft.start.date(local);
            self.reselect(&uid, &draft.start);
        }
    }

    /// An hour from the next full hour today, or 09:00 on other days.
    pub(super) fn default_slot(&self) -> (When, When) {
        let state = self.state.borrow();
        let local = state.local;
        let now = Utc::now().with_timezone(&local);
        let time = if state.date == now.date_naive() {
            NaiveTime::from_hms_opt((now.hour() + 1).min(23), 0, 0).unwrap()
        } else {
            NaiveTime::from_hms_opt(9, 0, 0).unwrap()
        };
        let start = When::Zoned(resolve_local(local, state.date.and_time(time)));
        (start, start.add(TimeDelta::hours(1)))
    }

    /// Add and remove guests of the selected event (all of a series).
    pub(super) fn change_guests(
        self: &Rc<Self>,
        add: Vec<Guest>,
        remove: &[String],
    ) -> Result<(), String> {
        let occ = self.current_item().ok_or("select an event first")?;
        if occ.kind != rical_core::Kind::Event {
            return Err("only events have guests".into());
        }
        let mut draft = {
            let state = self.state.borrow();
            if state.invitation(&occ).is_some() {
                return Err("it is someone else's event".into());
            }
            let mut draft = state.vdir.draft(&occ);
            draft.organizer = state.organizer_for(&occ.calendar);
            draft
        };
        let guests = draft.guests.get_or_insert_with(Vec::new);
        let had = !guests.is_empty();
        guests.retain(|g| !remove.iter().any(|r| r.eq_ignore_ascii_case(&g.email)));
        for g in add {
            if !guests
                .iter()
                .any(|h| h.email.eq_ignore_ascii_case(&g.email))
            {
                guests.push(g);
            }
        }
        if !had && !guests.is_empty() && draft.organizer.is_none() {
            return Err("your address is not known: start rical with --me you@example.com".into());
        }
        let start = draft.start;
        if self.perform("change the guests", |v| {
            v.update(&occ, Scope::All, &draft, None)
        }) {
            self.reselect(&occ.uid, &start);
        }
        Ok(())
    }

    pub(super) fn edit_selected(self: &Rc<Self>) {
        let Some(occ) = self.current_item() else {
            self.toast("Select an event to edit");
            return;
        };
        let original = self.state.borrow().vdir.draft(&occ);
        let weak = Rc::downgrade(self);
        let on_save = {
            let occ = occ.clone();
            let original = original.clone();
            move |draft: Draft, calendar: String| {
                let Some(w) = weak.upgrade() else { return };
                // changing the rule or calendar only makes sense for the series
                let whole = draft.repeat != original.repeat || calendar != occ.calendar;
                let save = {
                    let occ = occ.clone();
                    move |w: &Rc<Self>, scope| {
                        if w.perform("save the event", |v| {
                            v.update(&occ, scope, &draft, Some(&calendar))
                        }) {
                            w.reselect(&occ.uid, &draft.start);
                        }
                    }
                };
                if whole {
                    save(&w, Scope::All);
                } else {
                    w.with_scope(&occ, "Edit", save);
                }
            }
        };
        let weak = Rc::downgrade(self);
        let to_delete = occ.clone();
        editor::open(
            &self.window,
            &self.state,
            editor::Options {
                title: if occ.recurring {
                    "Edit Repeating Event"
                } else {
                    "Edit Event"
                },
                draft: original,
                calendar: occ.calendar.clone(),
                is_new: false,
                invitation: self.state.borrow().invitation(&occ).is_some(),
            },
            on_save,
            Some(Box::new(move || {
                if let Some(w) = weak.upgrade() {
                    w.delete_occurrence(to_delete.clone());
                }
            })),
        );
    }

    pub(super) fn delete_selected(self: &Rc<Self>) {
        match self.current_item() {
            Some(occ) => self.delete_occurrence(occ),
            None => self.toast("Select an event to delete"),
        }
    }

    fn delete_occurrence(self: &Rc<Self>, occ: Occurrence) {
        let o = occ.clone();
        self.with_scope(&o, "Delete", move |w, scope| {
            if w.perform("delete the event", |v| {
                v.delete(&occ, scope).map(|c| vec![c])
            }) {
                w.state.borrow_mut().selected = None;
                w.refresh();
                let toast = adw::Toast::builder()
                    .title(format!("Deleted “{}”", occ.summary))
                    .button_label("Undo")
                    .action_name("win.run")
                    .action_target(&"undo".to_variant())
                    .build();
                w.toasts.add_toast(toast);
            }
        });
    }

    /// Move the selected event by `shift`, or with `resize` change its end.
    pub(super) fn nudge_selected(self: &Rc<Self>, shift: TimeDelta, resize: bool) {
        let Some(occ) = self.state.borrow().selected_occurrence() else {
            return;
        };
        if is_all_day_row(&occ) && shift.num_days() == 0 {
            // minutes mean nothing to all-day events
            return;
        }
        let (start, end) = if resize {
            let end = occ.end.add(shift);
            let min = if occ.all_day {
                TimeDelta::days(1)
            } else {
                TimeDelta::minutes(15)
            };
            let local = self.state.borrow().local;
            if end.to_utc(local) < occ.start.add(min).to_utc(local) {
                return;
            }
            (occ.start, end)
        } else {
            (occ.start.add(shift), occ.end.add(shift))
        };
        self.set_times(occ, start, end);
    }

    pub(super) fn undo(self: &Rc<Self>) {
        let Some(changes) = self.state.borrow_mut().undo.pop() else {
            self.toast("Nothing to undo");
            return;
        };
        let result = {
            let mut s = self.state.borrow_mut();
            changes
                .iter()
                .rev()
                .try_for_each(|c| s.vdir.undo(c).map(|_| ()))
        };
        match result {
            Ok(()) => {
                let mut s = self.state.borrow_mut();
                s.redo.push(changes);
                s.invalidate();
            }
            Err(e) => self.toast(&format!("Could not undo: {e}")),
        }
        self.refresh();
    }

    pub(super) fn redo(self: &Rc<Self>) {
        let Some(changes) = self.state.borrow_mut().redo.pop() else {
            self.toast("Nothing to redo");
            return;
        };
        let result = {
            let mut s = self.state.borrow_mut();
            changes.iter().try_for_each(|c| s.vdir.apply(c))
        };
        match result {
            Ok(()) => {
                let mut s = self.state.borrow_mut();
                s.undo.push(changes);
                s.invalidate();
            }
            Err(e) => self.toast(&format!("Could not redo: {e}")),
        }
        self.refresh();
    }

    /// What edit and delete act on: the selected task in the tasks view,
    /// otherwise the selected occurrence.
    pub(super) fn current_item(&self) -> Option<Occurrence> {
        let state = self.state.borrow();
        if state.view != ViewKind::Tasks {
            return state.selected_occurrence();
        }
        let (path, uid) = state.selected_task.as_ref()?;
        let t = state.vdir.task(path, uid)?;
        Some(task_occurrence(&t, state.today()))
    }

    /// The task the user means: selected in the tasks view, or a selected
    /// task occurrence elsewhere.
    pub(super) fn selected_task(&self) -> Option<Task> {
        let state = self.state.borrow();
        let (path, uid) = if state.view == ViewKind::Tasks {
            state.selected_task.clone()?
        } else {
            let o = state
                .selected_occurrence()
                .filter(|o| o.kind == Kind::Todo)?;
            (o.path, o.uid)
        };
        state.vdir.task(&path, &uid)
    }

    pub(super) fn set_task_done(self: &Rc<Self>, id: &(PathBuf, String), done: bool) {
        let task = self.state.borrow().vdir.task(&id.0, &id.1);
        let Some(task) = task else { return };
        if task.is_done() == done {
            return;
        }
        let summary = task.summary.clone();
        if self.perform("update the task", |v| {
            v.set_done(&task, done).map(|c| vec![c])
        }) && done
        {
            // a repeating task moved on rather than finished
            let moved = self
                .state
                .borrow()
                .vdir
                .task(&task.path, &task.uid)
                .filter(|t| !t.is_done())
                .and_then(|t| t.due.or(t.start));
            let title = match moved {
                Some(next) => {
                    let local = self.state.borrow().local;
                    format!(
                        "“{summary}” next due {}",
                        crate::format::day(next.date(local))
                    )
                }
                None => format!("Done: “{summary}”"),
            };
            let toast = adw::Toast::builder()
                .title(title)
                .button_label("Undo")
                .action_name("win.run")
                .action_target(&"undo".to_variant())
                .build();
            self.toasts.add_toast(toast);
        }
    }

    /// Toggle the selected task between done and not done.
    pub(super) fn toggle_done(self: &Rc<Self>) -> Result<(), String> {
        let task = self.selected_task().ok_or("select a task first")?;
        self.set_task_done(&(task.path.clone(), task.uid.clone()), !task.is_done());
        Ok(())
    }

    pub(super) fn add_task(self: &Rc<Self>, calendar: String, title: &str, due: Option<When>) {
        self.state.borrow_mut().last_calendar = Some(calendar.clone());
        let mut created = None;
        if self.perform("create the task", |v| {
            let change = v.create_task(&calendar, title, due)?;
            created = Some(change.path.clone());
            Ok(vec![change])
        }) && let Some(path) = created
        {
            let uid = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.state.borrow_mut().selected_task = Some((path, uid));
            self.refresh();
        }
    }

    pub(super) fn set_selected_due(self: &Rc<Self>, due: Option<When>) -> Result<(), String> {
        let task = self.selected_task().ok_or("select a task first")?;
        self.perform("set the due date", |v| {
            v.set_due(&task, due).map(|c| vec![c])
        });
        Ok(())
    }

    /// Time-block the selected task: give it a start and due time so it
    /// shows on the grid.
    pub(super) fn schedule_selected(
        self: &Rc<Self>,
        start: When,
        length: TimeDelta,
    ) -> Result<(), String> {
        let occ = self
            .current_item()
            .filter(|o| o.kind == Kind::Todo)
            .ok_or("select a task to schedule")?;
        self.schedule(occ, start, length);
        Ok(())
    }

    fn schedule(self: &Rc<Self>, occ: Occurrence, start: When, length: TimeDelta) {
        let mut draft = self.state.borrow().vdir.draft(&occ);
        draft.start = start;
        draft.end = start.add(length);
        if self.perform("schedule the task", |v| {
            v.update(&occ, Scope::All, &draft, None)
        }) {
            let local = self.state.borrow().local;
            self.state.borrow_mut().date = start.date(local);
            self.reselect(&occ.uid, &start);
            self.toast(&format!("Scheduled “{}”", occ.summary));
        }
    }

    /// Change how the selected event repeats (the whole series).
    pub(super) fn set_repeat(self: &Rc<Self>, repeat: Repeat) -> Result<(), String> {
        let occ = self.current_item().ok_or("select an event first")?;
        let mut draft = self.state.borrow().vdir.draft(&occ);
        let summary = match &repeat {
            Repeat::None => "no longer repeats".to_owned(),
            Repeat::Rule(r) => format!("repeats {r}"),
            Repeat::Custom(raw) => format!("repeats ({raw})"),
        };
        draft.repeat = repeat;
        if self.perform("change how it repeats", |v| {
            v.update(&occ, Scope::All, &draft, None)
        }) {
            self.toast(&format!("“{}” {summary}", occ.summary));
        }
        Ok(())
    }

    /// A task dragged onto the calendar: a time schedules it for an hour,
    /// a day (the all-day row) makes it due then.
    fn drop_task(self: &Rc<Self>, id: &(PathBuf, String), at: When) {
        let (task, today) = {
            let s = self.state.borrow();
            (s.vdir.task(&id.0, &id.1), s.today())
        };
        let Some(task) = task else { return };
        if at.is_date() {
            let summary = task.summary.clone();
            if self.perform("set the due date", |v| {
                v.set_due(&task, Some(at)).map(|c| vec![c])
            }) {
                let local = self.state.borrow().local;
                self.toast(&format!(
                    "“{summary}” due {}",
                    crate::format::day(at.date(local))
                ));
            }
        } else {
            self.schedule(task_occurrence(&task, today), at, TimeDelta::hours(1));
        }
    }
}

/// A task as an occurrence for editing: on its dates, or today if undated.
fn task_occurrence(t: &Task, today: chrono::NaiveDate) -> Occurrence {
    let start = t.start.or(t.due).unwrap_or(When::Date(today));
    let end = t.due.unwrap_or(start);
    let mut occ = t.on_day(today);
    occ.start = start;
    occ.end = end;
    occ.all_day = start.is_date();
    occ
}

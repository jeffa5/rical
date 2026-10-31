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
                "“{}” repeats. Change just this one, this and the ones after it, or every occurrence?",
                occ.summary
            )),
        );
        dialog.add_responses(&[
            ("cancel", "_Cancel"),
            ("all", "_All events"),
            ("following", "This and _following events"),
            ("this", "_This event"),
        ]);
        if verb == "Delete" {
            dialog.set_response_appearance("this", adw::ResponseAppearance::Destructive);
            dialog.set_response_appearance("following", adw::ResponseAppearance::Destructive);
            dialog.set_response_appearance("all", adw::ResponseAppearance::Destructive);
        } else {
            dialog.set_response_appearance("this", adw::ResponseAppearance::Suggested);
        }
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
                    "following" => f(&w, Scope::Following),
                    "all" => f(&w, Scope::All),
                    _ => w.refresh(),
                }
            },
        );
    }

    /// Run an edit, record it for undo and report failure. Returns whether
    /// it worked.
    pub(super) fn perform(
        self: &Rc<Self>,
        what: &str,
        f: impl FnOnce(&mut Vdir) -> anyhow::Result<Vec<Change>>,
    ) -> bool {
        let result = f(&mut self.state.borrow_mut().vdir);
        match result {
            Ok(changes) => {
                self.state.borrow_mut().record(what, changes);
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

    /// A toast that replaces any showing, for the result of what was just
    /// done.
    pub(super) fn toast_now(&self, text: &str) {
        self.toasts.add_toast(
            adw::Toast::builder()
                .title(text)
                .priority(adw::ToastPriority::High)
                .build(),
        );
    }

    /// Select the occurrence of `uid` starting at `start`, if visible.
    pub(super) fn reselect(&self, uid: &str, start: &When) {
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
            // stretching a task that only had a due time gives it a start
            if occ.kind == Kind::Todo && start.until(&end) != occ.start.until(&occ.end) {
                draft.has_start = true;
            }
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
                uid: None,
            },
            move |draft, calendar| {
                if let Some(w) = weak.upgrade() {
                    w.create(draft, calendar);
                }
            },
            None,
        );
    }

    /// Open the editor for a new journal entry.
    pub(super) fn open_new_note(self: &Rc<Self>, draft: Draft, calendar: String) {
        let weak = Rc::downgrade(self);
        editor::open(
            &self.window,
            &self.state,
            editor::Options {
                title: "New Journal Entry",
                draft,
                calendar,
                is_new: true,
                invitation: false,
                uid: None,
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
        let what = match draft.kind {
            Kind::Journal => "add the journal entry",
            _ => "create the event",
        };
        let ok = self.perform(what, |v| {
            let change = v.create(&calendar, &draft)?;
            created = Some(change.path.clone());
            Ok(vec![change])
        });
        if ok && draft.kind == Kind::Journal {
            // selected in the journal view, and shown on its day elsewhere
            let path = created.unwrap();
            let uid = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let local = self.state.borrow().local;
            {
                let mut s = self.state.borrow_mut();
                s.selected_journal = Some((path, uid.clone()));
                if draft.has_start {
                    s.date = draft.start.date(local);
                }
            }
            if draft.has_start && self.state.borrow().view.is_calendar() {
                self.reselect(&uid, &draft.start);
            } else {
                self.refresh();
            }
            let day = if draft.has_start {
                crate::format::day(draft.start.date(local))
            } else {
                "an undated note".into()
            };
            self.toast_now(&format!("Added “{}” · {day}", draft.summary));
            return;
        }
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
    /// Answer the selected invitation: ACCEPTED, TENTATIVE or DECLINED.
    pub(super) fn reply(self: &Rc<Self>, status: &str) -> Result<(), String> {
        let occ = self.current_item().ok_or("select an invitation first")?;
        let me = {
            let state = self.state.borrow();
            match state.invitation(&occ) {
                Some(i) if i.reply.is_some() => state.my_addresses(),
                Some(_) => return Err("you are not on its guest list".into()),
                None => return Err("that is not an invitation".into()),
            }
        };
        let start = occ.start;
        // in the invitations view, answered ones leave the list: go on to
        // whatever takes their place, so y, y, N works through it
        let place = {
            let state = self.state.borrow();
            (state.view == ViewKind::Invitations).then(|| {
                state
                    .navigation_order()
                    .iter()
                    .position(|o| o.uid == occ.uid && o.calendar == occ.calendar)
            })
        };
        if self.perform("reply", |v| Ok(vec![v.reply(&occ, &me, status)?])) {
            let what = match status {
                "ACCEPTED" => "Accepted",
                "TENTATIVE" => "Maybe to",
                _ => "Declined",
            };
            // at once, not queued behind the last answer's
            self.toasts.add_toast(
                adw::Toast::builder()
                    .title(format!("{what} “{}”", occ.summary))
                    .priority(adw::ToastPriority::High)
                    .build(),
            );
            match place {
                Some(index) => {
                    {
                        let mut s = self.state.borrow_mut();
                        let order = s.navigation_order();
                        let next = index
                            .and_then(|i| order.get(i).or(order.last()))
                            .map(OccurrenceKey::of);
                        s.selected = next;
                    }
                    self.refresh();
                }
                None => self.reselect(&occ.uid, &start),
            }
        }
        Ok(())
    }

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
            return Err(
                "your address is not known: set me = you@example.com in ~/.config/rical-calendar/config"
                    .into(),
            );
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
        crate::details::close();
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
                // changing the rule or calendar only makes sense for the
                // series, and a task is edited as a whole
                let whole = draft.repeat != original.repeat
                    || calendar != occ.calendar
                    || occ.kind == Kind::Todo;
                let save = {
                    let occ = occ.clone();
                    let was_done = original.done;
                    let today = w.state.borrow().today();
                    move |w: &Rc<Self>, scope| {
                        if w.perform("save the event", |v| {
                            let mut changes = v.update(&occ, scope, &draft, Some(&calendar))?;
                            // a task moved to another calendar takes its
                            // subtasks along: they never cross calendars
                            if occ.kind == Kind::Todo && calendar != occ.calendar {
                                for t in descendants(v, &occ.calendar, &occ.uid) {
                                    let o = task_occurrence(&t, today);
                                    let d = v.draft(&o);
                                    changes.extend(v.update(
                                        &o,
                                        Scope::All,
                                        &d,
                                        Some(&calendar),
                                    )?);
                                }
                            }
                            // done or not through set_done, so a repeating
                            // task moves on to its next date instead
                            if let Some(done) = draft.done
                                && Some(done) != was_done
                                && let Some(task) = v.tasks().into_iter().find(|t| t.uid == occ.uid)
                            {
                                changes.push(v.set_done(&task, done)?);
                            }
                            Ok(changes)
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
                title: match (occ.kind, occ.recurring) {
                    (Kind::Todo, true) => "Edit Repeating Task",
                    (Kind::Todo, false) => "Edit Task",
                    (Kind::Journal, _) => "Edit Journal Entry",
                    (_, true) => "Edit Repeating Event",
                    _ => "Edit Event",
                },
                draft: original,
                calendar: occ.calendar.clone(),
                is_new: false,
                invitation: self.state.borrow().invitation(&occ).is_some(),
                uid: Some(occ.uid.clone()),
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

    /// Delete, after asking: a repeating event's "this one or all" is the
    /// asking; anything else gets "Delete …?", Delete the default. Undo (and
    /// the history) bring it back.
    fn delete_occurrence(self: &Rc<Self>, occ: Occurrence) {
        let what = match occ.kind {
            rical_core::Kind::Todo => "task",
            rical_core::Kind::Journal => "journal entry",
            _ => "event",
        };
        let o = occ.clone();
        let delete = move |w: &Rc<Self>, scope: Scope| {
            if w.perform(&format!("delete the {what}"), |v| {
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
        };

        let asks = o.recurring && self.forced_scope.get().is_none();
        if asks {
            return self.with_scope(&o, "Delete", delete);
        }
        let alert = adw::AlertDialog::builder()
            .heading(format!("Delete “{}”?", o.summary))
            .body(format!(
                "The {what} goes from {}. Undo (u), or History (Ctrl+H) later, brings it back.",
                self.state
                    .borrow()
                    .vdir
                    .calendar(&o.calendar)
                    .map_or(o.calendar.clone(), |c| c.name.clone())
            ))
            .default_response("delete")
            .close_response("cancel")
            .build();
        alert.add_response("cancel", "Cancel");
        alert.add_response("delete", "Delete");
        alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        let weak = Rc::downgrade(self);
        alert.connect_response(None, move |_, response| {
            let Some(w) = weak.upgrade() else { return };
            if response == "delete" {
                w.with_scope(&o, "Delete", delete.clone());
            } else {
                w.forced_scope.set(None);
            }
        });
        alert.present(Some(&self.window));
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

    /// Undo the latest change (from the history, so after a restart too).
    pub(super) fn undo(self: &Rc<Self>) {
        self.undo_redo(true);
    }

    /// Redo what was last undone.
    pub(super) fn redo(self: &Rc<Self>) {
        self.undo_redo(false);
    }

    fn undo_redo(self: &Rc<Self>, undo: bool) {
        let entry = {
            let mut s = self.state.borrow_mut();
            s.history.as_mut().and_then(|h| {
                // the other app may have added to it
                if let Err(e) = h.reload() {
                    warn!("reading the history: {e:#}");
                }
                if undo {
                    h.last_done(crate::state::APP)
                } else {
                    h.next_undone(crate::state::APP)
                }
                .cloned()
            })
        };
        let Some(entry) = entry else {
            self.toast_now(if undo {
                "Nothing to undo"
            } else {
                "Nothing to redo"
            });
            return;
        };
        match self.undo_entry(&entry.id, undo) {
            Ok(()) => self.toast_now(&format!(
                "{}: {}",
                if undo { "Undone" } else { "Redone" },
                entry.what
            )),
            Err(e) => self.toast_now(&e),
        }
    }

    /// Undo (or redo) one entry of the history, and show the result.
    pub(super) fn undo_entry(self: &Rc<Self>, id: &str, undo: bool) -> Result<(), String> {
        let result = self.state.borrow_mut().undo_entry(id, undo);
        self.refresh();
        result.map_err(|e| format!("Could not {}: {e:#}", if undo { "undo" } else { "redo" }))
    }

    /// Everything done here, to undo or redo any of it.
    pub(super) fn show_history(self: &Rc<Self>) {
        let state = self.state.clone();
        let entries: rical_ui::history::Entries = Rc::new(move || {
            let mut s = state.borrow_mut();
            match s.history.as_mut() {
                Some(h) => {
                    let _ = h.reload();
                    (h.entries().to_vec(), h.dir().to_owned())
                }
                None => (Vec::new(), std::path::PathBuf::new()),
            }
        });
        let weak = Rc::downgrade(self);
        let toggle: rical_ui::history::Toggle = Rc::new(move |id, undo| match weak.upgrade() {
            Some(w) => w.undo_entry(id, undo),
            None => Ok(()),
        });
        rical_ui::history::dialog(crate::state::APP, entries, toggle).present(Some(&self.window));
    }

    /// What edit and delete act on: the selected task in the tasks view,
    /// otherwise the selected occurrence.
    pub(super) fn current_item(&self) -> Option<Occurrence> {
        let state = self.state.borrow();
        if state.view == ViewKind::Journal {
            let (path, uid) = state.selected_journal.as_ref()?;
            return state
                .vdir
                .journals()
                .into_iter()
                .find(|(o, _)| o.path == *path && o.uid == *uid)
                .map(|(o, _)| o);
        }
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

    pub(super) fn add_task(
        self: &Rc<Self>,
        calendar: String,
        title: &str,
        due: Option<When>,
        parent: Option<String>,
    ) {
        // a subtask goes with its parent, not where new things usually go
        if parent.is_none() {
            self.state.borrow_mut().last_calendar = Some(calendar.clone());
        }
        let mut created = None;
        if self.perform("create the task", |v| {
            let change = v.create_task(&calendar, title, due, parent.as_deref())?;
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

    /// Create a task from a draft (quick add), and select it.
    pub(super) fn add_task_draft(self: &Rc<Self>, draft: Draft, calendar: String) {
        self.state.borrow_mut().last_calendar = Some(calendar.clone());
        let mut created = None;
        if self.perform("create the task", |v| {
            let change = v.create(&calendar, &draft)?;
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

    /// Show and select the task `uid`: on the calendar at its date if it
    /// has one and can be seen there, otherwise in the tasks view.
    pub(super) fn go_to_task(self: &Rc<Self>, calendar: &str, uid: &str) -> Result<(), String> {
        let task = self
            .state
            .borrow()
            .vdir
            .tasks()
            .into_iter()
            .find(|t| t.uid == uid && t.calendar == calendar)
            .ok_or("that task is not in your calendars")?;
        let view = self.state.borrow().view;
        if view != ViewKind::Tasks
            && let Some(when) = task.start.or(task.due)
        {
            let local = self.state.borrow().local;
            self.state.borrow_mut().date = when.date(local);
            self.refresh();
            // this file's occurrence, not a copy of it in another calendar
            let occ = {
                let s = self.state.borrow();
                s.visible_occurrences()
                    .iter()
                    .find(|o| o.path == task.path && o.uid == task.uid)
                    .map(OccurrenceKey::of)
            };
            if let Some(key) = occ {
                self.state.borrow_mut().selected = Some(key);
                self.refresh();
                return Ok(());
            }
        }
        // undated, or not on the calendar (a hidden calendar, say)
        self.state.borrow_mut().selected_task = Some((task.path, task.uid));
        self.set_view(ViewKind::Tasks);
        Ok(())
    }

    /// From a subtask to its parent task.
    pub(super) fn go_to_parent(self: &Rc<Self>) -> Result<(), String> {
        let task = self.selected_task().ok_or("select a subtask first")?;
        let parent = task.parent.ok_or("it is not a subtask")?;
        crate::details::close();
        // its parent is in the same calendar
        self.go_to_task(&task.calendar, &parent)
    }

    /// Make the selected task a subtask of `parent`, or a task of its own.
    pub(super) fn set_parent(self: &Rc<Self>, parent: Option<String>) -> Result<(), String> {
        let task = self.selected_task().ok_or("select a task first")?;
        if let Some(p) = &parent {
            let state = self.state.borrow();
            if state.vdir.is_within(&task.calendar, p, &task.uid) {
                return Err("that is this task or one of its own subtasks".into());
            }
            // subtasks stay in their parent's calendar
            if !state
                .vdir
                .tasks()
                .iter()
                .any(|t| t.uid == *p && t.calendar == task.calendar)
            {
                return Err("that task is not in this calendar".into());
            }
        }
        let occ = task_occurrence(&task, self.state.borrow().today());
        let mut draft = self.state.borrow().vdir.draft(&occ);
        draft.parent = parent;
        let summary = task.summary.clone();
        if self.perform("change the parent task", |v| {
            v.update(&occ, Scope::All, &draft, None)
        }) {
            self.toast(&match &draft.parent {
                Some(_) => format!("“{summary}” is now a subtask"),
                None => format!("“{summary}” is a task of its own"),
            });
        }
        Ok(())
    }

    pub(super) fn set_selected_start(self: &Rc<Self>, start: Option<When>) -> Result<(), String> {
        let task = self.selected_task().ok_or("select a task first")?;
        self.perform("set the start", |v| {
            v.set_start(&task, start).map(|c| vec![c])
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
        // a time block: it starts then and is due at its end
        draft.has_start = true;
        draft.has_due = true;
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

/// The subtasks of `uid`, and theirs, at any depth (each once).
fn descendants(v: &Vdir, calendar: &str, uid: &str) -> Vec<Task> {
    // subtasks share their parent's calendar
    let tasks = v
        .tasks()
        .into_iter()
        .filter(|t| t.calendar == calendar)
        .collect::<Vec<_>>();
    let mut out: Vec<Task> = Vec::new();
    let mut todo = vec![uid.to_owned()];
    while let Some(parent) = todo.pop() {
        for t in tasks
            .iter()
            .filter(|t| t.parent.as_deref() == Some(parent.as_str()))
        {
            if t.uid != uid && !out.iter().any(|o| o.uid == t.uid) {
                todo.push(t.uid.clone());
                out.push(t.clone());
            }
        }
    }
    out
}

/// A task as an occurrence for editing: on its dates, or today if undated.
/// As for events the end is exclusive, so a date due is the day after.
fn task_occurrence(t: &Task, today: chrono::NaiveDate) -> Occurrence {
    let day = TimeDelta::days(1);
    let start = t.start.or(t.due).unwrap_or(When::Date(today));
    let end = match t.due {
        Some(When::Date(d)) => When::Date(d).add(day),
        Some(due) => due,
        None if start.is_date() => start.add(day),
        None => start,
    };
    let mut occ = t.on_day(today);
    occ.start = start;
    occ.end = end;
    occ.all_day = start.is_date();
    occ
}

impl Window {
    /// Show the selected item in the calendar, on its day: a task on its
    /// due (or start) day, a journal entry on its date, an agenda item in
    /// the week. From the agenda, tasks and journal views.
    pub(super) fn reveal(self: &Rc<Self>) -> Result<(), String> {
        let occ = self.current_item().ok_or("select something first")?;
        let (day, view) = {
            let s = self.state.borrow();
            let day = match s.view {
                ViewKind::Tasks => s
                    .vdir
                    .task(&occ.path, &occ.uid)
                    .and_then(|t| t.day(s.local))
                    .ok_or("it has no date to show")?,
                ViewKind::Journal => {
                    let dated = s
                        .vdir
                        .journals()
                        .into_iter()
                        .any(|(o, dated)| dated && o.path == occ.path && o.uid == occ.uid);
                    if !dated {
                        return Err("it is an undated note".into());
                    }
                    occ.start.date(s.local)
                }
                _ => occ.start.date(s.local),
            };
            // a view of days around it: the last one used, or the week
            let view = match s.calendar_view {
                ViewKind::Agenda | ViewKind::Year => s.week_view,
                v if v.is_calendar() => v,
                _ => s.week_view,
            };
            (day, view)
        };
        {
            let mut s = self.state.borrow_mut();
            s.view = view;
            s.date = day;
            let found = s
                .visible_occurrences()
                .iter()
                .filter(|o| o.path == occ.path && o.uid == occ.uid)
                .min_by_key(|o| (o.start.date(s.local) - day).num_days().abs())
                .map(OccurrenceKey::of);
            s.selected = found;
        }
        self.refresh();
        self.focus_view();
        self.grid.reveal_selected();
        Ok(())
    }

    /// `:duplicate`: copy the selected item (a whole series) beside it,
    /// and select the copy.
    pub(super) fn duplicate(self: &Rc<Self>) -> Result<(), String> {
        let occ = self.current_item().ok_or("select an event or task first")?;
        crate::details::close();
        let mut made = None;
        if !self.perform("duplicate it", |v| {
            let (change, uid) = v.duplicate(&occ)?;
            made = Some((change.path.clone(), uid));
            Ok(vec![change])
        }) {
            return Ok(());
        }
        let (path, uid) = made.unwrap();
        let view = self.state.borrow().view;
        match view {
            ViewKind::Tasks => self.state.borrow_mut().selected_task = Some((path, uid)),
            ViewKind::Journal => {
                self.state.borrow_mut().selected_journal = Some((path, uid));
            }
            _ => self.reselect(&uid, &occ.start),
        }
        self.refresh();
        let guests = if occ.attendees.is_empty() {
            ""
        } else {
            ", without its guests"
        };
        self.toast_now(&format!("Copied “{}”{guests} · u undoes", occ.summary));
        Ok(())
    }

    /// `:convert`: turn the selected event into a task, or task into an
    /// event, and select what it became.
    pub(super) fn convert(self: &Rc<Self>) -> Result<(), String> {
        let occ = self.current_item().ok_or("select an event or task first")?;
        crate::details::close();
        let today = self.today_date();
        let local = self.state.borrow().local;
        let what = match occ.kind {
            Kind::Todo => "make it an event",
            _ => "make it a task",
        };
        let mut made = None;
        if !self.perform(what, |v| {
            let (changes, uid) = v.convert(&occ, today, local)?;
            made = Some((changes[0].path.clone(), uid));
            Ok(changes)
        }) {
            return Ok(());
        }
        let (path, uid) = made.unwrap();
        if occ.kind == Kind::Todo {
            // the event, selected on its day
            let found = {
                let mut s = self.state.borrow_mut();
                let around = occ.start.to_utc(local);
                let range = around - TimeDelta::days(400)..around + TimeDelta::days(400);
                let first = s
                    .vdir
                    .query(&range, &[Kind::Event], local)
                    .into_iter()
                    .filter(|o| o.path == path && o.uid == uid)
                    .min_by_key(|o| (o.start.date(local) - today).num_days().abs());
                if let Some(o) = &first
                    && s.view.is_calendar()
                {
                    s.date = o.start.date(local);
                }
                first
            };
            if let Some(o) = found {
                self.reselect(&uid, &o.start);
            }
            self.toast_now(&format!("“{}” is now an event · u undoes", occ.summary));
        } else {
            self.state.borrow_mut().selected_task = Some((path, uid));
            self.refresh();
            self.toast_now(&format!("“{}” is now a task · u undoes", occ.summary));
        }
        Ok(())
    }

    /// `:person [who]`: someone on the selected event, by address or a
    /// name you know them by; with nobody named, the one other person.
    pub(super) fn show_person(self: &Rc<Self>, who: &str) -> Result<(), String> {
        let occ = self.current_item().ok_or("select an event first")?;
        let state = self.state.borrow();
        let me = state.my_addresses();
        let mut people: Vec<&rical_core::Person> = Vec::new();
        for p in occ.organizer.iter().chain(&occ.attendees) {
            if !people.iter().any(|q| q.email == p.email) {
                people.push(p);
            }
        }
        let who = who.trim().to_lowercase();
        let found = if who.is_empty() {
            let others = people
                .iter()
                .filter(|p| !p.is_any_of(&me))
                .collect::<Vec<_>>();
            match others[..] {
                [one] => Some(*one),
                [] => return Err("nobody else is on it".into()),
                _ => return Err("who? e.g. :person Sam".into()),
            }
        } else {
            people
                .iter()
                .find(|p| {
                    p.email
                        .as_deref()
                        .is_some_and(|e| e.eq_ignore_ascii_case(&who))
                })
                .or_else(|| {
                    people
                        .iter()
                        .find(|p| state.name_of(p).to_lowercase().contains(&who))
                })
                .copied()
        };
        let person = found.ok_or(format!("nobody called “{who}” is on it"))?;
        crate::details::close();
        crate::person::show(&self.window, &state, person);
        Ok(())
    }

    /// `:settings`: the preferences dialog over the config file.
    pub(super) fn show_settings(self: &Rc<Self>) {
        crate::details::close();
        let calendars = self
            .state
            .borrow()
            .vdir
            .calendars
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        let weak = Rc::downgrade(self);
        crate::prefs::show(
            &self.window,
            calendars,
            Rc::new(move || match weak.upgrade() {
                Some(w) => w.apply_settings(),
                None => Ok(()),
            }),
        );
    }

    /// Read the settings again and apply those that can change while
    /// running; the rest (the vdir, the view to open on, bindings, the
    /// renderer) wait for the next start.
    pub(super) fn apply_settings(self: &Rc<Self>) -> Result<(), String> {
        let o = crate::config::Options::load()?;
        let week = o.week.as_deref().unwrap_or("fullweek");
        let week_view = ViewKind::from_name(week)
            .filter(|v| matches!(v, ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_)))
            .ok_or(format!("week: “{week}” is not fullweek, workweek or Ndays"))?;
        let reminder = match &o.remind {
            Some(text) => Some(
                rical_core::input::parse_shift(text)
                    .filter(|d| *d > TimeDelta::zero())
                    .ok_or(format!("remind: not a duration: “{text}”"))?,
            ),
            None => None,
        };
        {
            let mut s = self.state.borrow_mut();
            // showing the week as it was: show it as it is now
            if s.view == s.week_view {
                s.view = week_view;
            }
            if s.calendar_view == s.week_view {
                s.calendar_view = week_view;
            }
            s.week_view = week_view;
            if o.calendar.is_some() {
                s.last_calendar = o.calendar;
            }
            s.notifications = !o.no_notifications;
            s.me = o.me;
            s.contacts = o.contacts;
            s.zones = o.zones;
            s.default_reminder = reminder;
            s.invalidate();
        }
        self.refresh();
        Ok(())
    }

    /// `:about`: who made rical, its version and licence, and details for
    /// a bug report.
    pub(super) fn show_about(self: &Rc<Self>) {
        crate::details::close();
        let details = {
            let s = self.state.borrow();
            format!(
                "vdir: {}\ncalendars: {}\ntime zone: {}\n",
                s.vdir.root.display(),
                s.vdir.calendars.len(),
                s.local.name(),
            )
        };
        rical_ui::util::about(
            &self.window,
            "A keyboard-first calendar, task list and journal for a folder of \
             iCalendar files, such as one vdirsyncer keeps in sync.",
            &details,
        );
    }
}

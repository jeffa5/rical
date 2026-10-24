//! Editing from the window: create, edit, move, delete, undo and redo.
//! Each edit asks whether a repeating event changes as a whole or just this
//! once, writes through the vdir, and records the change for undo.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{NaiveTime, TimeDelta, Timelike, Utc};
use gtk::gio;
use log::warn;
use rical_core::{
    Occurrence, Vdir, When,
    when::resolve_local,
    write::{Change, Draft, Scope},
};

use super::Window;
use crate::{editor, grid::DragAction, layout::is_all_day_row, state::OccurrenceKey};

impl Window {
    pub(super) fn on_drag(self: &Rc<Self>, action: DragAction) {
        match action {
            DragAction::Move { occ, start, end } => self.set_times(*occ, start, end),
            DragAction::Create { start, end } => self.new_item(Some((start, end))),
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
        if !occ.recurring {
            f(self, Scope::All);
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
        let weak = Rc::downgrade(self);
        editor::open(
            &self.window,
            &self.state,
            editor::Options {
                title: "New Event",
                draft: Draft::new(start, end),
                calendar,
                is_new: true,
            },
            move |draft, calendar| {
                let Some(w) = weak.upgrade() else { return };
                w.state.borrow_mut().last_calendar = Some(calendar.clone());
                let mut created = None;
                let ok = w.perform("create the event", |v| {
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
                    let local = w.state.borrow().local;
                    w.state.borrow_mut().date = draft.start.date(local);
                    w.reselect(&uid, &draft.start);
                }
            },
            None,
        );
    }

    /// An hour from the next full hour today, or 09:00 on other days.
    fn default_slot(&self) -> (When, When) {
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

    pub(super) fn edit_selected(self: &Rc<Self>) {
        let Some(occ) = self.state.borrow().selected_occurrence() else {
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
        match self.state.borrow().selected_occurrence() {
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
                    .action_name("win.undo")
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
}

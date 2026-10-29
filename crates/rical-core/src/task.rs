//! Tasks (VTODO) as a list, independent of dates: listing, completing and
//! quick creation.

use std::path::PathBuf;

use anyhow::Context as _;
use chrono::{NaiveDate, Utc};
use chrono_tz::Tz;

use crate::{
    doc::{Component, Document, Line},
    occurrence::{Kind, Occurrence, unescape_text},
    vdir::Vdir,
    when::When,
    write::Change,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    pub calendar: String,
    pub path: PathBuf,
    pub uid: String,
    pub summary: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub due: Option<When>,
    pub start: Option<When>,
    pub completed: Option<When>,
    /// 1 (highest) to 9; 0 or absent means undefined.
    pub priority: Option<u8>,
    /// Has an RRULE: completing it moves it to the next occurrence.
    pub recurring: bool,
    /// For a subtask, its parent task's UID (RELATED-TO;RELTYPE=PARENT).
    pub parent: Option<String>,
}

impl Task {
    pub fn is_done(&self) -> bool {
        self.completed.is_some()
            || matches!(self.status.as_deref(), Some("COMPLETED" | "CANCELLED"))
    }

    /// The day it is due (or starts, without a due date) for `local`.
    pub fn day(&self, local: Tz) -> Option<NaiveDate> {
        self.due.or(self.start).map(|w| w.date(local))
    }

    /// Still to do and due before `today`.
    pub fn is_overdue(&self, today: NaiveDate, local: Tz) -> bool {
        !self.is_done() && self.day(local).is_some_and(|d| d < today)
    }

    /// This task shown on `day` as an all-day item, e.g. to carry an
    /// overdue task forward to today.
    pub fn on_day(&self, day: NaiveDate) -> Occurrence {
        Occurrence {
            calendar: self.calendar.clone(),
            path: self.path.clone(),
            uid: self.uid.clone(),
            recurrence_id: None,
            kind: Kind::Todo,
            summary: self.summary.clone(),
            location: None,
            description: self.description.clone(),
            status: self.status.clone(),
            start: When::Date(day),
            end: When::Date(day.succ_opt().unwrap_or(day)),
            all_day: true,
            recurring: false,
            due: self.due,
            alarms: Vec::new(),
            organizer: None,
            attendees: Vec::new(),
            conferences: Vec::new(),
            attachments: Vec::new(),
            parent: self.parent.clone(),
        }
    }
}

impl Vdir {
    /// Every task (the master of each VTODO), dated or not.
    pub fn tasks(&self) -> Vec<Task> {
        self.components()
            .filter(|(_, c)| {
                c.component_kind() == "VTODO" && c.property_value("RECURRENCE-ID").is_none()
            })
            .map(|(cal, c)| {
                let when = |name| c.prop(name).and_then(When::from_property);
                Task {
                    calendar: cal.id.clone(),
                    path: c.path().to_owned(),
                    uid: c.get_uid().unwrap_or_default().to_owned(),
                    summary: c
                        .property_value("SUMMARY")
                        .map(unescape_text)
                        .unwrap_or_default(),
                    description: c.property_value("DESCRIPTION").map(unescape_text),
                    status: c.property_value("STATUS").map(str::to_owned),
                    due: when("DUE"),
                    start: when("DTSTART"),
                    completed: when("COMPLETED"),
                    priority: c
                        .property_value("PRIORITY")
                        .and_then(|p| p.trim().parse().ok())
                        .filter(|p| *p > 0),
                    recurring: c.property_value("RRULE").is_some(),
                    parent: crate::occurrence::parent_of(c),
                }
            })
            .collect()
    }

    pub fn task(&self, path: &std::path::Path, uid: &str) -> Option<Task> {
        self.tasks()
            .into_iter()
            .find(|t| t.path == path && t.uid == uid)
    }

    /// Mark a task done (STATUS, COMPLETED, PERCENT-COMPLETE) or not done.
    /// For a repeating task, done moves it to its next occurrence instead
    /// (counting down any COUNT), and only the last one is marked done.
    pub fn set_done(&mut self, task: &Task, done: bool) -> anyhow::Result<Change> {
        if done
            && task.recurring
            && let Some(shift) = self.next_shift(task)
        {
            return self.edit_task(task, |c| {
                for name in ["DTSTART", "DUE"] {
                    if let Some(w) = c.property(name).and_then(Line::when) {
                        c.set_when(name, &w.add(shift));
                    }
                }
                if let Some(rule) = c.property("RRULE").map(|l| l.value().to_owned()) {
                    c.set(Line::property("RRULE", &[], &count_down(&rule)));
                }
            });
        }
        self.edit_task(task, |c| {
            if done {
                c.set(Line::new("STATUS:COMPLETED"));
                c.set(Line::property(
                    "COMPLETED",
                    &[],
                    &Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
                ));
                c.set(Line::new("PERCENT-COMPLETE:100"));
            } else {
                c.set(Line::new("STATUS:NEEDS-ACTION"));
                c.remove("COMPLETED");
                c.remove("PERCENT-COMPLETE");
            }
        })
    }

    /// Set or clear a task's due date.
    pub fn set_due(&mut self, task: &Task, due: Option<When>) -> anyhow::Result<Change> {
        self.edit_task(task, |c| match due {
            Some(due) => c.set_when("DUE", &due),
            None => c.remove("DUE"),
        })
    }

    /// Whether `uid` is `ancestor` or one of its subtasks (at any depth):
    /// such a task cannot become `ancestor`'s parent without a loop.
    /// Within `calendar`: subtasks and their parents share one, and a UID
    /// may be in another calendar too.
    pub fn is_within(&self, calendar: &str, uid: &str, ancestor: &str) -> bool {
        let parents = self
            .tasks()
            .into_iter()
            .filter(|t| t.calendar == calendar)
            .map(|t| (t.uid, t.parent))
            .collect::<std::collections::HashMap<_, _>>();
        let mut at = Some(uid.to_owned());
        let mut steps = 0;
        while let Some(u) = at {
            if u == ancestor {
                return true;
            }
            steps += 1;
            if steps > parents.len() {
                return true; // already a loop
            }
            at = parents.get(&u).cloned().flatten();
        }
        false
    }

    /// Set or clear when a task starts.
    pub fn set_start(&mut self, task: &Task, start: Option<When>) -> anyhow::Result<Change> {
        self.edit_task(task, |c| match start {
            Some(start) => c.set_when("DTSTART", &start),
            None => c.remove("DTSTART"),
        })
    }

    fn edit_task(&mut self, task: &Task, f: impl FnOnce(&mut Component)) -> anyhow::Result<Change> {
        let before = std::fs::read_to_string(&task.path)
            .with_context(|| format!("reading {:?}", task.path))?;
        let mut doc = Document::parse(&before)?;
        let c = doc
            .find_mut(&task.uid, None)
            .filter(|c| c.name == "VTODO")
            .context("task not found in file")?;
        f(c);
        crate::write::touch(c);
        let change = Change {
            path: task.path.clone(),
            before: Some(before),
            after: Some(doc.write()),
        };
        self.apply_checked(&change)?;
        Ok(change)
    }

    /// Create a task with a title, an optional due date and, for a subtask,
    /// the UID of its parent task.
    pub fn create_task(
        &mut self,
        calendar: &str,
        summary: &str,
        due: Option<When>,
        parent: Option<&str>,
    ) -> anyhow::Result<Change> {
        let dir = self
            .calendar(calendar)
            .with_context(|| format!("no calendar {calendar:?}"))?
            .path
            .clone();
        let uid = uuid::Uuid::new_v4().to_string();
        let mut doc = Document::new_calendar();
        let mut c = Component::new("VTODO");
        c.insert(Line::property("UID", &[], &uid));
        let now = crate::write::stamp();
        for name in ["DTSTAMP", "CREATED", "LAST-MODIFIED"] {
            c.insert(Line::property(name, &[], &now));
        }
        c.insert(Line::new("SEQUENCE:0"));
        c.insert(Line::new("STATUS:NEEDS-ACTION"));
        c.set_text("SUMMARY", summary);
        if let Some(parent) = parent {
            c.insert(Line::property(
                "RELATED-TO",
                &[("RELTYPE", "PARENT")],
                parent,
            ));
        }
        if let Some(due) = due {
            c.set_when("DUE", &due);
        }
        doc.push(c);
        self.add_timezones(&mut doc);
        let change = Change {
            path: dir.join(format!("{uid}.ics")),
            before: None,
            after: Some(doc.write()),
        };
        self.apply_checked(&change)?;
        Ok(change)
    }
}

impl Vdir {
    /// How far a repeating task moves to reach its next occurrence, if it
    /// has one.
    fn next_shift(&self, task: &Task) -> Option<chrono::TimeDelta> {
        let anchor = task.start.or(task.due)?;
        let from = anchor.to_utc(Tz::UTC);
        let components = self
            .get(&task.path)?
            .iter()
            .filter(|c| c.get_uid() == Some(task.uid.as_str()));
        let range = from..from + chrono::TimeDelta::days(366 * 5);
        let next = crate::occurrence::expand(&task.calendar, components, &range, Tz::UTC)
            .into_iter()
            .filter(|o| o.recurrence_id.is_some() && o.start.to_utc(Tz::UTC) > from)
            .min_by_key(|o| o.start.to_utc(Tz::UTC))?;
        Some(next.start.to_utc(Tz::UTC) - from)
    }
}

/// One fewer remaining occurrence: COUNT counts from DTSTART, which moves.
fn count_down(rule: &str) -> String {
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("COUNT") => match v.parse::<u32>() {
                Ok(n) if n > 1 => format!("COUNT={}", n - 1),
                _ => part.to_owned(),
            },
            _ => part.to_owned(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

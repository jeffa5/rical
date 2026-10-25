//! Turning an event into a task, or a task into an event.
//!
//! The item is written as a new file with a new UID and the old one
//! removed, rather than changing the component in place: CalDAV servers
//! may refuse an object whose type changes, or a UID that appears twice
//! while vdirsyncer uploads one and deletes the other.

use anyhow::{Context, bail};
use chrono::{NaiveDate, TimeDelta};
use chrono_tz::Tz;

use crate::{
    Kind, Occurrence, Vdir, When,
    doc::{Component, Document, Line},
    when::parse_duration,
    write::{Change, read, stamp},
};

impl Vdir {
    /// Make `occ`'s item (the whole series) the other kind: an event a
    /// task due when it ends, a task an event at its dates (or all day
    /// `today` if it has none). Returns the changes made (to undo them
    /// together) and the new item's UID.
    pub fn convert(
        &mut self,
        occ: &Occurrence,
        today: NaiveDate,
        local: Tz,
    ) -> anyhow::Result<(Vec<Change>, String)> {
        let to = match occ.kind {
            Kind::Event => Kind::Todo,
            Kind::Todo => Kind::Event,
            Kind::Journal => bail!("only events and tasks can be converted"),
        };
        let before = read(&occ.path)?;
        let mut doc = Document::parse(&before)?;
        let parts = doc
            .items()
            .filter(|c| c.uid() == Some(occ.uid.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if parts.is_empty() {
            bail!("{:?} no longer has it; reload and try again", occ.path);
        }
        // deleting an event with guests cancels it for them, or declines it
        if parts.iter().any(|c| c.property("ATTENDEE").is_some()) {
            bail!("it has guests, who would be told it was cancelled or declined");
        }
        if to == Kind::Event
            && self
                .tasks()
                .iter()
                .any(|t| t.parent.as_deref() == Some(occ.uid.as_str()))
        {
            bail!("it has subtasks; move them to another task first");
        }

        let uid = uuid::Uuid::new_v4().to_string();
        let mut new = Document::new_calendar();
        let now = stamp();
        for mut c in parts {
            match to {
                Kind::Todo => event_to_task(&mut c),
                _ => task_to_event(&mut c, today, local),
            }
            c.set(Line::property("UID", &[], &uid));
            c.set(Line::property("DTSTAMP", &[], &now));
            c.set(Line::property("LAST-MODIFIED", &[], &now));
            c.set(Line::new("SEQUENCE:0"));
            // a subtask link means nothing on an event, nor is wanted on
            // a task made from one
            c.remove("RELATED-TO");
            new.push(c);
        }
        self.add_timezones(&mut new);
        let dir = occ.path.parent().context("item has no folder")?;
        let created = Change {
            path: dir.join(format!("{uid}.ics")),
            before: None,
            after: Some(new.write()),
        };

        doc.remove_where(|c| c.uid() == Some(occ.uid.as_str()));
        let removed = Change {
            path: occ.path.clone(),
            before: Some(before),
            after: doc.items().next().is_some().then(|| doc.write()),
        };

        self.apply_checked(&created)?;
        if let Err(e) = self.apply_checked(&removed) {
            // never leave both behind
            let _ = self.apply(&created.inverse());
            return Err(e);
        }
        Ok((vec![created, removed], uid))
    }
}

/// When a component starts and (exclusively) ends, if it says.
fn start_of(c: &Component) -> Option<When> {
    c.property("DTSTART").and_then(Line::when)
}

fn event_to_task(c: &mut Component) {
    c.rename("VTODO");
    let start = start_of(c);
    let end = c.property("DTEND").and_then(Line::when).or_else(|| {
        let d = parse_duration(c.property("DURATION")?.value().trim())?;
        Some(start?.add(d))
    });
    c.remove("DTEND");
    c.remove("DURATION");
    c.remove("TRANSP");
    let day = TimeDelta::days(1);
    if let Some(start) = start {
        let end = end.filter(|e| start.until(e) > TimeDelta::zero());
        // a date DUE is the last day itself, not the day after
        let due = match end {
            Some(e) if e.is_date() => e.add(-day),
            Some(e) => e,
            None => start,
        };
        c.set(crate::doc::when_line("DUE", &due));
        // a one-day all-day event is a task due that day; a start is kept
        // for anything longer, and for reminders set from the start
        let one_day = start.is_date() && due == start;
        let has_alarms = c.components().any(|a| a.name == "VALARM");
        if one_day
            && !has_alarms
            && c.property("RECURRENCE-ID").is_none()
            && c.property("RRULE").is_none()
        {
            c.remove("DTSTART");
        }
    }
    let status = c
        .property("STATUS")
        .map(|l| l.value().trim().to_ascii_uppercase());
    if status.as_deref() != Some("CANCELLED") {
        c.set(Line::new("STATUS:NEEDS-ACTION"));
    }
}

fn task_to_event(c: &mut Component, today: NaiveDate, local: Tz) {
    c.rename("VEVENT");
    let day = TimeDelta::days(1);
    let hour = TimeDelta::hours(1);
    let start = start_of(c);
    let due = c.property("DUE").and_then(Line::when);
    let has_duration = c.property("DURATION").is_some();
    c.remove("DUE");
    for name in ["COMPLETED", "PERCENT-COMPLETE"] {
        c.remove(name);
    }
    let status = c
        .property("STATUS")
        .map(|l| l.value().trim().to_ascii_uppercase());
    if status.as_deref() != Some("CANCELLED") {
        c.remove("STATUS");
    }
    if has_duration && start.is_some() {
        // DTSTART and DURATION mean the same on an event
        return;
    }
    let (start, end) = match (start, due) {
        // dates or a date and a time: all day, over the days it covers
        (Some(s), Some(d)) if s.is_date() || d.is_date() => {
            let (first, last) = (
                date_of(&s, local),
                date_of(&d, local).max(date_of(&s, local)),
            );
            (When::Date(first), When::Date(last).add(day))
        }
        (Some(s), Some(d)) if s.until(&d) > TimeDelta::zero() => (s, d),
        (Some(s), _) if s.is_date() => (s, s.add(day)),
        (Some(s), _) => (s, s.add(hour)),
        // at the time it is due, so a repeating one keeps its instances
        (None, Some(d)) if d.is_date() => (d, d.add(day)),
        (None, Some(d)) => (d, d.add(hour)),
        (None, None) => {
            let s = When::Date(today);
            (s, s.add(day))
        }
    };
    c.set(crate::doc::when_line("DTSTART", &start));
    c.set(crate::doc::when_line("DTEND", &end));
}

/// A date as written, in its own zone; here for UTC times.
fn date_of(w: &When, local: Tz) -> NaiveDate {
    match w {
        When::Date(d) => *d,
        When::Zoned(z) if z.timezone() == Tz::UTC => w.date(local),
        When::Zoned(z) => z.date_naive(),
        When::Floating(f) => f.date(),
    }
}

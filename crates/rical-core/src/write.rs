//! Creating, editing and deleting items in a vdir.
//!
//! Every edit re-reads the file from disk, changes only the affected lines
//! (see [`crate::doc`]) and writes it back atomically. Each write is
//! returned as a [`Change`] so it can be undone.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use chrono::{TimeDelta, Utc};
use chrono_tz::Tz;
use icalendar::Component as _;

use crate::{
    doc::{self, Component, Document, Item, Line},
    occurrence::{Kind, Occurrence, unescape_text},
    rule::{Freq, Rule},
    vdir::Vdir,
    when::When,
};

/// Which instances of a recurring series an edit applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only the selected occurrence.
    This,
    /// The whole series.
    All,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Repeat {
    None,
    /// A rule rical can edit.
    Rule(Rule),
    /// A rule using more than rical edits; kept as it is.
    Custom(String),
}

impl Repeat {
    pub fn from_rrule(rule: Option<&str>) -> Self {
        match rule {
            None => Self::None,
            Some(r) => Rule::parse(r).map_or_else(|| Self::Custom(r.to_owned()), Self::Rule),
        }
    }

    /// The RRULE value for a series starting at `start`.
    pub fn rrule(&self, start: &When) -> Option<String> {
        match self {
            Self::None => None,
            Self::Rule(r) => Some(r.to_rrule(start)),
            Self::Custom(r) => Some(r.clone()),
        }
    }

    /// Every day/week/month/year, no end: the presets.
    pub fn every(freq: Freq) -> Self {
        Self::Rule(Rule::new(freq))
    }
}

/// The editable fields of an event or todo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    pub kind: Kind,
    pub summary: String,
    pub location: String,
    pub description: String,
    pub start: When,
    pub end: When,
    pub repeat: Repeat,
    /// The guests it should have; None leaves them as they are.
    pub guests: Option<Vec<Guest>>,
    /// Me, written as the ORGANIZER when guests are added to an event
    /// that has none yet.
    pub organizer: Option<Guest>,
}

/// Someone invited, or to invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guest {
    pub email: String,
    pub name: Option<String>,
}

impl Draft {
    pub fn new(start: When, end: When) -> Self {
        Self {
            kind: Kind::Event,
            summary: String::new(),
            location: String::new(),
            description: String::new(),
            start,
            end,
            repeat: Repeat::None,
            guests: None,
            organizer: None,
        }
    }
}

/// One file write: contents before and after (None if absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Change {
    pub fn inverse(&self) -> Self {
        Self {
            path: self.path.clone(),
            before: self.after.clone(),
            after: self.before.clone(),
        }
    }
}

impl Vdir {
    /// The draft to start editing `occ` from.
    pub fn draft(&self, occ: &Occurrence) -> Draft {
        let rrule = self.get(&occ.path).and_then(|components| {
            components
                .iter()
                .find(|c| {
                    c.get_uid() == Some(occ.uid.as_str())
                        && c.property_value("RECURRENCE-ID").is_none()
                })
                .and_then(|c| c.property_value("RRULE").map(str::to_owned))
        });
        Draft {
            kind: occ.kind,
            summary: occ.summary.clone(),
            location: occ.location.clone().unwrap_or_default(),
            description: occ.description.clone().unwrap_or_default(),
            start: occ.start,
            end: occ.end,
            repeat: Repeat::from_rrule(rrule.as_deref()),
            // the guests, not the organiser's own attendee line
            guests: Some(
                occ.attendees
                    .iter()
                    .filter(|a| occ.organizer.as_ref().is_none_or(|o| o.email != a.email))
                    .filter_map(|a| {
                        Some(Guest {
                            email: a.email.clone()?,
                            name: a.name.clone(),
                        })
                    })
                    .collect(),
            ),
            organizer: None,
        }
    }

    /// Create a new item in `calendar` from `draft`.
    pub fn create(&mut self, calendar: &str, draft: &Draft) -> anyhow::Result<Change> {
        let dir = self
            .calendar(calendar)
            .with_context(|| format!("no calendar {calendar:?}"))?
            .path
            .clone();
        let uid = uuid::Uuid::new_v4().to_string();
        let path = dir.join(format!("{uid}.ics"));

        let mut doc = Document::new_calendar();
        let mut c = Component::new(match draft.kind {
            Kind::Event => "VEVENT",
            Kind::Todo => "VTODO",
            Kind::Journal => "VJOURNAL",
        });
        c.insert(Line::property("UID", &[], &uid));
        let now = stamp();
        c.insert(Line::property("DTSTAMP", &[], &now));
        c.insert(Line::property("CREATED", &[], &now));
        c.insert(Line::property("LAST-MODIFIED", &[], &now));
        c.insert(Line::new("SEQUENCE:0"));
        set_fields(&mut c, draft, true);
        set_times(&mut c, draft.kind, &draft.start, &draft.end, None);
        doc.push(c);
        self.add_timezones(&mut doc);

        let change = Change {
            path: path.clone(),
            before: None,
            after: Some(doc.write()),
        };
        self.apply_checked(&change)?;
        Ok(change)
    }

    /// Apply `draft` to `occ`, optionally moving it to another calendar.
    pub fn update(
        &mut self,
        occ: &Occurrence,
        scope: Scope,
        draft: &Draft,
        calendar: Option<&str>,
    ) -> anyhow::Result<Vec<Change>> {
        let before = read(&occ.path)?;
        let mut doc = Document::parse(&before)?;

        if occ.recurring && scope == Scope::This {
            let rid = occ
                .recurrence_id
                .context("recurring occurrence without a recurrence id")?;
            if doc.find(&occ.uid, Some(&rid)).is_none() {
                let master = doc
                    .find(&occ.uid, None)
                    .context("series has no master component")?;
                let o = make_override(master, &rid);
                doc.push(o);
            }
            let c = doc.find_mut(&occ.uid, Some(&rid)).unwrap();
            set_fields(c, draft, false);
            set_times(c, draft.kind, &draft.start, &draft.end, None);
            touch(c);
        } else if occ.recurring {
            let master = doc
                .find_mut(&occ.uid, None)
                .context("series has no master component")?;
            let master_start = master
                .property("DTSTART")
                .and_then(Line::when)
                .context("series has no DTSTART")?;
            // move the series by however far this occurrence moved
            let shift = if draft.start.is_date() == occ.start.is_date() {
                Some(
                    draft.start.to_utc(Tz::UTC)
                        - occ.start.coerce_like(&draft.start).to_utc(Tz::UTC),
                )
            } else {
                None
            };
            let start = match shift {
                Some(shift) => master_start.add(shift),
                None => draft
                    .start
                    .coerce_like(&master_start_kind(&master_start, &draft.start)),
            };
            let end = start.add(draft.start.until(&draft.end));
            set_fields(master, draft, true);
            set_times(master, draft.kind, &start, &end, Some(&master_start));
            touch(master);
            if draft.repeat == Repeat::None {
                // no longer a series: drop its exceptions
                master.remove("RDATE");
                master.remove("EXDATE");
                let uid = occ.uid.clone();
                doc.remove_where(|c| c.uid() == Some(uid.as_str()) && c.recurrence_id().is_some());
            }
        } else {
            let c = doc
                .find_mut(&occ.uid, occ.recurrence_id.as_ref())
                .context("item not found in file")?;
            let original = c.property("DTSTART").and_then(Line::when);
            set_fields(c, draft, true);
            set_times(c, draft.kind, &draft.start, &draft.end, original.as_ref());
            touch(c);
        }
        self.add_timezones(&mut doc);
        let after = doc.write();

        let target_dir = match calendar {
            Some(id) if Some(id) != self.calendar_for_path(&occ.path).map(|c| c.id.as_str()) => {
                if occ.recurring && scope == Scope::This {
                    anyhow::bail!("move the whole series to change its calendar");
                }
                Some(
                    self.calendar(id)
                        .with_context(|| format!("no calendar {id:?}"))?
                        .path
                        .clone(),
                )
            }
            _ => None,
        };

        let changes = match target_dir {
            Some(dir) => {
                let name = occ.path.file_name().context("item has no file name")?;
                let new_path = dir.join(name);
                if new_path.exists() {
                    anyhow::bail!("{new_path:?} already exists");
                }
                vec![
                    Change {
                        path: new_path,
                        before: None,
                        after: Some(after),
                    },
                    Change {
                        path: occ.path.clone(),
                        before: Some(before),
                        after: None,
                    },
                ]
            }
            None => vec![Change {
                path: occ.path.clone(),
                before: Some(before),
                after: Some(after),
            }],
        };
        for change in &changes {
            self.apply_checked(change)?;
        }
        Ok(changes)
    }

    /// Delete `occ`: one instance of a series becomes an EXDATE, anything
    /// else removes the item (and the file once it is empty).
    pub fn delete(&mut self, occ: &Occurrence, scope: Scope) -> anyhow::Result<Change> {
        let before = read(&occ.path)?;
        let mut doc = Document::parse(&before)?;
        let uid = occ.uid.clone();

        let has_master = doc.find(&uid, None).is_some();
        if occ.recurring && scope == Scope::This && has_master {
            let rid = occ
                .recurrence_id
                .context("recurring occurrence without a recurrence id")?;
            doc.remove_where(|c| {
                c.uid() == Some(uid.as_str())
                    && c.recurrence_id()
                        .is_some_and(|r| r.coerce_like(&rid) == rid)
            });
            let master = doc.find_mut(&uid, None).unwrap();
            let start = master
                .property("DTSTART")
                .and_then(Line::when)
                .context("series has no DTSTART")?;
            master.insert(doc::when_line("EXDATE", &rid.coerce_like(&start)));
            touch(master);
        } else if occ.recurring && scope == Scope::This {
            // an orphaned override
            doc.remove_where(|c| c.uid() == Some(uid.as_str()) && c.recurrence_id().is_some());
        } else {
            doc.remove_where(|c| c.uid() == Some(uid.as_str()));
        }

        let after = (doc.items().next().is_some()).then(|| doc.write());
        let change = Change {
            path: occ.path.clone(),
            before: Some(before),
            after,
        };
        self.apply_checked(&change)?;
        Ok(change)
    }

    /// Apply a change made by an edit, first checking the new text parses
    /// so rical never writes a file it (or others) cannot read back.
    pub(crate) fn apply_checked(&mut self, change: &Change) -> anyhow::Result<()> {
        if let Some(after) = &change.after {
            let invalid = || format!("refusing to write invalid {:?}", change.path);
            crate::ics::parse_calendar(after).with_context(invalid)?;
            Document::parse(after).with_context(invalid)?;
        }
        self.apply(change)
    }

    /// Write a change to disk and reload the item. Refuses if the file is
    /// not in the state the change expects, e.g. vdirsyncer changed it.
    pub fn apply(&mut self, change: &Change) -> anyhow::Result<()> {
        let current = match fs::read_to_string(&change.path) {
            Ok(s) => Some(s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).with_context(|| format!("reading {:?}", change.path)),
        };
        if current != change.before {
            anyhow::bail!("{:?} changed on disk; reload and try again", change.path);
        }
        write_atomic(&change.path, change.after.as_deref())?;
        self.reload(&change.path);
        Ok(())
    }

    /// Undo a change made by this module, returning the change that redoes
    /// it.
    pub fn undo(&mut self, change: &Change) -> anyhow::Result<Change> {
        let inverse = change.inverse();
        self.apply(&inverse)?;
        Ok(inverse)
    }

    /// Copy any VTIMEZONE definitions `doc` refers to but lacks from other
    /// items in the vdir. TZIDs without one elsewhere are left as they are;
    /// servers resolve IANA names themselves.
    pub(crate) fn add_timezones(&self, doc: &mut Document) {
        let mut tzids = Vec::new();
        for item in doc.items() {
            for line in item.lines() {
                if let Some(tzid) = line.param("TZID")
                    && !tzids.contains(&tzid)
                {
                    tzids.push(tzid);
                }
            }
        }
        for tzid in tzids {
            if doc.has_timezone(&tzid) {
                continue;
            }
            if let Some(tz) = self.find_timezone(&tzid) {
                doc.add_timezone(tz);
            }
        }
    }

    fn find_timezone(&self, tzid: &str) -> Option<Component> {
        let path = self.components().find_map(|(_, c)| {
            (c.component_kind() == "VTIMEZONE" && c.property_value("TZID") == Some(tzid))
                .then(|| c.path().to_owned())
        })?;
        let doc = Document::parse(&fs::read_to_string(path).ok()?).ok()?;
        doc.timezone(tzid).cloned()
    }
}

fn master_start_kind(master_start: &When, new: &When) -> When {
    // switching between all-day and timed: use the new value's form
    if master_start.is_date() == new.is_date() {
        *master_start
    } else {
        *new
    }
}

/// Write summary, location, description and (for masters) the repeat rule.
fn set_fields(c: &mut Component, draft: &Draft, repeat: bool) {
    fn current(c: &Component, name: &str) -> Option<String> {
        c.property(name).map(|l| unescape_text(l.value()))
    }
    if current(c, "SUMMARY").as_deref() != Some(draft.summary.as_str()) {
        c.set_text("SUMMARY", &draft.summary);
    }
    for (name, value) in [
        ("LOCATION", &draft.location),
        ("DESCRIPTION", &draft.description),
    ] {
        if value.is_empty() {
            c.remove(name);
        } else if current(c, name).as_deref() != Some(value.as_str()) {
            c.set_text(name, value);
        }
    }
    if let Some(guests) = &draft.guests {
        set_guests(c, guests, draft.organizer.as_ref());
    }
    if repeat {
        // UNTIL is written relative to the series' own start form
        let start = c
            .property("DTSTART")
            .and_then(Line::when)
            .unwrap_or(draft.start);
        let current = c.property("RRULE").map(|l| l.value().to_owned());
        match (&draft.repeat, current) {
            (Repeat::None, _) => c.remove("RRULE"),
            // the same rule written differently: leave the file alone
            (Repeat::Rule(rule), Some(cur)) if Rule::parse(&cur).as_ref() == Some(rule) => {}
            (repeat, cur) => {
                let rule = repeat.rrule(&start).unwrap();
                if cur.as_deref() != Some(rule.as_str()) {
                    c.set(Line::property("RRULE", &[], &rule));
                }
            }
        }
    }
}

/// Make the ATTENDEEs `guests`, touching only what changes: those kept
/// keep their lines (and replies), new ones are added as not yet answered,
/// and `organizer` becomes the ORGANIZER if there is none. With no guests
/// left it is no longer a meeting, so the organiser goes too.
fn set_guests(c: &mut Component, guests: &[Guest], organizer: Option<&Guest>) {
    fn email_of(l: &Line) -> String {
        let v = l.value().trim();
        v.get(..7)
            .filter(|p| p.eq_ignore_ascii_case("mailto:"))
            .map_or(v, |_| &v[7..])
            .to_lowercase()
    }
    let org = c.property("ORGANIZER").map(email_of);
    let wanted = guests
        .iter()
        .map(|g| g.email.to_lowercase())
        .collect::<Vec<_>>();
    if wanted.is_empty() {
        if c.properties("ATTENDEE").any(|l| Some(email_of(l)) != org) {
            c.remove("ATTENDEE");
            c.remove("ORGANIZER");
        }
        return;
    }
    // guests no longer wanted (the organiser's own line stays)
    c.items.retain(|i| match i {
        Item::Line(l) if l.name() == "ATTENDEE" => {
            let e = email_of(l);
            Some(&e) == org.as_ref() || wanted.contains(&e)
        }
        _ => true,
    });
    if org.is_none()
        && let Some(me) = organizer
    {
        let mut params = Vec::new();
        if let Some(n) = &me.name {
            params.push(("CN", n.as_str()));
        }
        c.insert(Line::property(
            "ORGANIZER",
            &params,
            &format!("mailto:{}", me.email),
        ));
    }
    let have = c.properties("ATTENDEE").map(email_of).collect::<Vec<_>>();
    for g in guests {
        if have.contains(&g.email.to_lowercase()) {
            continue;
        }
        let mut params = Vec::new();
        if let Some(n) = &g.name {
            params.push(("CN", n.as_str()));
        }
        params.extend([
            ("CUTYPE", "INDIVIDUAL"),
            ("ROLE", "REQ-PARTICIPANT"),
            ("PARTSTAT", "NEEDS-ACTION"),
            ("RSVP", "TRUE"),
        ]);
        c.insert(Line::property(
            "ATTENDEE",
            &params,
            &format!("mailto:{}", g.email),
        ));
    }
}

/// Write start and end, keeping the original value form (TZID, UTC or
/// floating) unless switching between all-day and timed.
fn set_times(c: &mut Component, kind: Kind, start: &When, end: &When, original: Option<&When>) {
    let original = original
        .copied()
        .or_else(|| c.property("DTSTART").and_then(Line::when))
        .or_else(|| c.property("DUE").and_then(Line::when));
    let form = |w: &When| match original {
        Some(o) if o.is_date() == w.is_date() => w.coerce_like(&o),
        _ => *w,
    };
    let start = form(start);
    let end = form(end);

    let set_if_changed = |c: &mut Component, name: &str, w: &When| {
        if c.property(name).and_then(Line::when) != Some(*w) {
            c.set_when(name, w);
        }
    };
    match kind {
        Kind::Todo => {
            // a task with only a due date keeps only a due date
            if c.property("DTSTART").is_some() || start != end {
                set_if_changed(c, "DTSTART", &start);
            }
            set_if_changed(c, "DUE", &end);
        }
        Kind::Event | Kind::Journal => {
            set_if_changed(c, "DTSTART", &start);
            if kind == Kind::Journal {
                return;
            }
            if c.property("DURATION").is_some() {
                let d = start.until(&end);
                c.set(Line::property("DURATION", &[], &format_duration(d)));
            } else {
                set_if_changed(c, "DTEND", &end);
            }
        }
    }
}

/// Bump SEQUENCE and the modification stamps.
pub(crate) fn touch(c: &mut Component) {
    let seq = c
        .property("SEQUENCE")
        .and_then(|l| l.value().trim().parse::<u32>().ok())
        .unwrap_or(0);
    c.set(Line::property("SEQUENCE", &[], &(seq + 1).to_string()));
    let now = stamp();
    c.set(Line::property("DTSTAMP", &[], &now));
    c.set(Line::property("LAST-MODIFIED", &[], &now));
}

/// A copy of `master` standing in for the instance at `rid`.
pub(crate) fn make_override(master: &Component, rid: &When) -> Component {
    let mut o = master.clone();
    for name in ["RRULE", "RDATE", "EXDATE", "EXRULE"] {
        o.remove(name);
    }
    let start = master.property("DTSTART").and_then(Line::when);
    let rid = start.map_or(*rid, |s| rid.coerce_like(&s));
    o.set_when("RECURRENCE-ID", &rid);
    o
}

pub(crate) fn stamp() -> String {
    Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

pub fn format_duration(d: TimeDelta) -> String {
    let secs = d.num_seconds().max(0);
    if secs > 0 && secs % 86400 == 0 {
        return format!("P{}D", secs / 86400);
    }
    let (h, m, s) = (secs / 3600, secs % 3600 / 60, secs % 60);
    let mut out = String::from("PT");
    if h > 0 {
        out += &format!("{h}H");
    }
    if m > 0 {
        out += &format!("{m}M");
    }
    if s > 0 || out == "PT" {
        out += &format!("{s}S");
    }
    out
}

pub(crate) fn read(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path).with_context(|| format!("reading {path:?}"))
}

/// Replace `path` with `contents` (or delete it) so readers never see a
/// partial file: write a temporary file in the same folder, sync, rename.
pub fn write_atomic(path: &Path, contents: Option<&str>) -> anyhow::Result<()> {
    let Some(contents) = contents else {
        return match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(e).with_context(|| format!("deleting {path:?}"))
            }
            _ => Ok(()),
        };
    };
    if fs::read_to_string(path).is_ok_and(|current| current == contents) {
        return Ok(());
    }
    let dir = path.parent().context("path has no parent")?;
    let name = path
        .file_name()
        .context("path has no file name")?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.rical-{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {path:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_rules() {
        assert_eq!(Repeat::from_rrule(None), Repeat::None);
        assert_eq!(
            Repeat::from_rrule(Some("FREQ=WEEKLY")),
            Repeat::every(Freq::Weekly)
        );
        assert_eq!(
            Repeat::from_rrule(Some("FREQ=DAILY;INTERVAL=1")),
            Repeat::every(Freq::Daily)
        );
        assert!(matches!(
            Repeat::from_rrule(Some("FREQ=WEEKLY;BYDAY=MO,WE")),
            Repeat::Rule(_)
        ));
        assert_eq!(
            Repeat::from_rrule(Some("FREQ=MONTHLY;BYDAY=1TU")),
            Repeat::Custom("FREQ=MONTHLY;BYDAY=1TU".into())
        );
    }

    #[test]
    fn guests_change_only_what_they_must() {
        let g = |e: &str| Guest {
            email: e.into(),
            name: None,
        };
        let me = Guest {
            email: "me@example.com".into(),
            name: Some("Me".into()),
        };
        let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nSUMMARY:Lunch\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let mut doc = Document::parse(text).unwrap();
        let c = doc.find_mut("x", None).unwrap();

        // first guest: I become the organiser
        set_guests(c, &[g("sam@example.com")], Some(&me));
        let out = doc.write().replace("\r\n ", "");
        assert!(out.contains("ORGANIZER;CN=Me:mailto:me@example.com"));
        assert!(out.contains("PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:sam@example.com"));

        // Sam replies; adding Jo keeps Sam's reply
        let replied = out.replace(
            "PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:sam",
            "PARTSTAT=ACCEPTED:mailto:sam",
        );
        let mut doc = Document::parse(&replied).unwrap();
        let c = doc.find_mut("x", None).unwrap();
        set_guests(c, &[g("SAM@example.com"), g("jo@example.com")], Some(&me));
        let out = doc.write().replace("\r\n ", "");
        assert!(out.contains("PARTSTAT=ACCEPTED:mailto:sam@example.com"));
        assert!(out.contains("mailto:jo@example.com"));
        assert_eq!(out.matches("ORGANIZER").count(), 1);

        // drop Sam, then everyone: no longer a meeting
        let mut doc = Document::parse(&out).unwrap();
        let c = doc.find_mut("x", None).unwrap();
        set_guests(c, &[g("jo@example.com")], Some(&me));
        assert!(!doc.write().replace("\r\n ", "").contains("sam@"));
        let c = doc.find_mut("x", None).unwrap();
        set_guests(c, &[], Some(&me));
        let out = doc.write().replace("\r\n ", "");
        assert!(!out.contains("ATTENDEE") && !out.contains("ORGANIZER"));
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(TimeDelta::minutes(90)), "PT1H30M");
        assert_eq!(format_duration(TimeDelta::days(2)), "P2D");
        assert_eq!(format_duration(TimeDelta::zero()), "PT0S");
    }
}

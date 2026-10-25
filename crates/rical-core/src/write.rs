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

use crate::{
    doc::{self, Component, Document, Item, Line},
    occurrence::{Kind, Occurrence, unescape_text},
    rule::{Freq, Rule},
    vdir::Vdir,
    when::{When, resolve_local},
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
    /// Start and end are in the zones they should be written in (chosen
    /// in the editor), each its own; otherwise they are converted to the
    /// item's existing zone, as a drag does.
    pub exact_zones: bool,
    /// For a task: whether it has a start (DTSTART) and a due date (DUE);
    /// either or both may be missing. `start`/`end` hold its dates as for
    /// an event, the end exclusive (a task due on a day ends after it).
    pub has_start: bool,
    pub has_due: bool,
    /// For a task: PRIORITY, 1 (highest) to 9; None for none.
    pub priority: Option<u8>,
    /// For a task: whether it is done. Not written by `update`: completing
    /// goes through [`Vdir::set_done`], as a repeating task moves on instead.
    pub done: Option<bool>,
    /// For a task: the UID of the task it is a subtask of
    /// (RELATED-TO;RELTYPE=PARENT), or None for a task of its own.
    pub parent: Option<String>,
    /// The video call link (RFC 7986 CONFERENCE); "" for none, None to
    /// leave conferences as they are.
    pub video: Option<String>,
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
            exact_zones: false,
            has_start: true,
            has_due: true,
            priority: None,
            done: None,
            parent: None,
            video: None,
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
        let master = self.get(&occ.path).and_then(|components| {
            components.iter().find(|c| {
                c.get_uid() == Some(occ.uid.as_str()) && c.property_value("RECURRENCE-ID").is_none()
            })
        });
        let rrule = master.and_then(|c| c.property_value("RRULE").map(str::to_owned));
        let has = |name| master.is_none_or(|c| c.property_value(name).is_some());
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
            exact_zones: false,
            // which dates a task has (an event always has a start)
            has_start: occ.kind != Kind::Todo || has("DTSTART"),
            has_due: occ.kind != Kind::Todo || has("DUE"),
            priority: master
                .and_then(|c| c.property_value("PRIORITY"))
                .and_then(|p| p.trim().parse().ok())
                .filter(|p| *p > 0),
            done: (occ.kind == Kind::Todo).then(|| {
                master.is_some_and(|c| {
                    c.property_value("COMPLETED").is_some()
                        || matches!(c.property_value("STATUS"), Some("COMPLETED" | "CANCELLED"))
                })
            }),
            parent: occ.parent.clone(),
            video: (occ.kind == Kind::Event)
                .then(|| video_of(&occ.conferences).unwrap_or_default()),
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
        if draft.kind == Kind::Todo {
            c.insert(Line::new("STATUS:NEEDS-ACTION"));
        }
        set_fields(&mut c, draft, true);
        set_times(
            &mut c,
            draft.kind,
            &draft.start,
            &draft.end,
            None,
            draft.exact_zones,
            (draft.has_start, draft.has_due),
        );
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
            set_times(
                c,
                draft.kind,
                &draft.start,
                &draft.end,
                None,
                draft.exact_zones,
                (draft.has_start, draft.has_due),
            );
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
                // moved to another zone: the series keeps its wall-clock
                // times there (09:00 Singapore becomes 09:00 London), plus
                // however far this occurrence's wall time was changed
                Some(_) if draft.exact_zones && zone_of(&master_start) != zone_of(&draft.start) => {
                    let wall = naive(&draft.start) - naive(&occ.start.coerce_like(&master_start));
                    let n = naive(&master_start) + wall;
                    match draft.start {
                        When::Zoned(dt) => When::Zoned(resolve_local(dt.timezone(), n)),
                        _ => When::Floating(n),
                    }
                }
                Some(shift) => master_start.add(shift),
                None => draft
                    .start
                    .coerce_like(&master_start_kind(&master_start, &draft.start)),
            };
            let mut end = start.add(draft.start.until(&draft.end));
            if draft.exact_zones && !end.is_date() {
                // the end in its own zone (a flight lands somewhere else)
                end = end.coerce_like(&draft.end);
            }
            set_fields(master, draft, true);
            set_times(
                master,
                draft.kind,
                &start,
                &end,
                Some(&master_start),
                draft.exact_zones,
                (draft.has_start, draft.has_due),
            );
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
            set_times(
                c,
                draft.kind,
                &draft.start,
                &draft.end,
                original.as_ref(),
                draft.exact_zones,
                (draft.has_start, draft.has_due),
            );
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

    /// Answer an invitation: set my PARTSTAT (ACCEPTED, TENTATIVE or
    /// DECLINED) on every instance of it, where I am an attendee under any
    /// of `me`. SEQUENCE is the organiser's, so only the stamps change;
    /// a server doing scheduling (RFC 6638) tells the organiser.
    pub fn reply(
        &mut self,
        occ: &Occurrence,
        me: &[String],
        status: &str,
    ) -> anyhow::Result<Change> {
        let before = read(&occ.path)?;
        let mut doc = Document::parse(&before)?;
        let is_me = |l: &Line| {
            let v = l.value().trim();
            let email = v
                .get(..7)
                .filter(|p| p.eq_ignore_ascii_case("mailto:"))
                .map_or(v, |_| &v[7..]);
            me.iter().any(|m| m.eq_ignore_ascii_case(email))
        };
        let mut found = false;
        for c in doc
            .items_mut()
            .filter(|c| c.uid() == Some(occ.uid.as_str()))
        {
            let mut mine = false;
            for i in c.items.iter_mut() {
                if let Item::Line(l) = i
                    && l.name() == "ATTENDEE"
                    && is_me(l)
                {
                    *l = l.with_param("PARTSTAT", status);
                    mine = true;
                }
            }
            if mine {
                let now = stamp();
                c.set(Line::property("DTSTAMP", &[], &now));
                c.set(Line::property("LAST-MODIFIED", &[], &now));
                found = true;
            }
        }
        anyhow::ensure!(found, "not invited as {}", me.join(" or "));
        let change = Change {
            path: occ.path.clone(),
            before: Some(before),
            after: Some(doc.write()),
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
        // VTIMEZONEs are not kept in memory: read one from a file with it
        let path = self.timezone_file(tzid)?;
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

/// The video call link among `conferences`: one with FEATURE=VIDEO, else
/// the first that opens in a browser.
pub fn video_of(conferences: &[crate::Conference]) -> Option<String> {
    conferences
        .iter()
        .find(|c| c.has("VIDEO") && c.is_web())
        .or_else(|| conferences.iter().find(|c| c.is_web()))
        .map(|c| c.uri.clone())
}

/// Set the video call link: change the CONFERENCE that has it (keeping
/// its FEATURE and LABEL), add one, or with "" remove it. Other ways in
/// (dial-in numbers, chat) are left as they are.
fn set_video(c: &mut Component, video: &str) {
    let is_web = |v: &str| {
        let v = v.trim().to_ascii_lowercase();
        v.starts_with("https://") || v.starts_with("http://")
    };
    let lines = c.properties("CONFERENCE").cloned().collect::<Vec<_>>();
    let target = lines
        .iter()
        .find(|l| {
            is_web(l.value())
                && l.param("FEATURE")
                    .is_some_and(|f| f.split(',').any(|f| f.trim().eq_ignore_ascii_case("VIDEO")))
        })
        .or_else(|| lines.iter().find(|l| is_web(l.value())))
        .cloned();
    let google = c.property("X-GOOGLE-CONFERENCE").cloned();
    let current = target
        .as_ref()
        .or(if lines.is_empty() {
            google.as_ref()
        } else {
            None
        })
        .map(|l| l.value().trim().to_owned());
    if current.as_deref().unwrap_or("") == video {
        return;
    }
    // Google's own copy would otherwise still be read as the link
    c.remove("X-GOOGLE-CONFERENCE");
    match target {
        Some(old) => {
            for i in c.items.iter_mut() {
                if let Item::Line(l) = i
                    && *l == old
                {
                    if video.is_empty() {
                        *l = Line::new("");
                    } else {
                        // the parameters exactly as written (FEATURE is a
                        // list, which quoting would make one value)
                        let head = &l.content()[..l.content().len() - l.value().len()];
                        *l = Line::new(format!("{head}{video}"));
                    }
                }
            }
            c.items
                .retain(|i| !matches!(i, Item::Line(l) if l.content().is_empty()));
        }
        None if !video.is_empty() => c.insert(Line::new(format!(
            "CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO:{video}"
        ))),
        None => {}
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
    if let Some(video) = &draft.video {
        set_video(c, video.trim());
    }
    if draft.kind == Kind::Todo {
        set_parent(c, draft.parent.as_deref());
    }
    if draft.kind == Kind::Todo {
        let current = c
            .property("PRIORITY")
            .and_then(|l| l.value().trim().parse::<u8>().ok())
            .filter(|p| *p > 0);
        if current != draft.priority {
            match draft.priority {
                Some(p) => c.set(Line::property("PRIORITY", &[], &p.to_string())),
                None => c.remove("PRIORITY"),
            }
        }
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

/// Make `parent` the task this one is a subtask of: its RELATED-TO with
/// RELTYPE=PARENT (or none, the default), leaving other relations alone.
fn set_parent(c: &mut Component, parent: Option<&str>) {
    let is_parent = |l: &Line| {
        l.name() == "RELATED-TO"
            && l.param("RELTYPE")
                .is_none_or(|r| r.eq_ignore_ascii_case("PARENT"))
    };
    let current = c
        .lines()
        .find(|l| is_parent(l))
        .map(|l| l.value().trim().to_owned());
    if current.as_deref() == parent {
        return;
    }
    c.items
        .retain(|i| !matches!(i, Item::Line(l) if is_parent(l)));
    if let Some(p) = parent {
        c.insert(Line::property("RELATED-TO", &[("RELTYPE", "PARENT")], p));
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
/// The zone of a timed value (None for floating or a date).
fn zone_of(w: &When) -> Option<Tz> {
    match w {
        When::Zoned(dt) => Some(dt.timezone()),
        _ => None,
    }
}

/// The wall-clock time of a value in its own zone.
fn naive(w: &When) -> chrono::NaiveDateTime {
    match w {
        When::Date(d) => d.and_time(chrono::NaiveTime::MIN),
        When::Floating(n) => *n,
        When::Zoned(dt) => dt.naive_local(),
    }
}

/// With `exact`, timed values are written as given, each in its own zone
/// (the editor chose them); otherwise they take the original's form, so a
/// drag keeps an event's TZID.
fn set_times(
    c: &mut Component,
    kind: Kind,
    start: &When,
    end: &When,
    original: Option<&When>,
    exact: bool,
    (has_start, has_due): (bool, bool),
) {
    let original = original
        .copied()
        .or_else(|| c.property("DTSTART").and_then(Line::when))
        .or_else(|| c.property("DUE").and_then(Line::when));
    let form = |w: &When| match original {
        _ if exact && !w.is_date() => *w,
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
            // only the dates it has: a task due on a day keeps just DUE
            if has_start {
                set_if_changed(c, "DTSTART", &start);
            } else {
                c.remove("DTSTART");
            }
            if has_due {
                // the end is exclusive; a date DUE is the day itself
                let due = match end {
                    When::Date(d) => When::Date(d.pred_opt().unwrap_or(d)),
                    w => w,
                };
                set_if_changed(c, "DUE", &due);
            } else {
                c.remove("DUE");
            }
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

//! Expanding components into the concrete occurrences shown on a calendar.

use std::{collections::BTreeMap, ops::Range, path::PathBuf, str::FromStr};

use chrono::{DateTime, NaiveTime, TimeDelta, TimeZone as _, Utc};
use chrono_tz::Tz;
use icalendar::Component as _;
use log::warn;
use serde::Serialize;

use crate::{
    component::AnyComponent,
    when::{When, parse_duration, resolve_local},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Event,
    Todo,
    Journal,
}

impl Kind {
    pub fn from_component_kind(kind: &str) -> Option<Self> {
        match kind {
            "VEVENT" => Some(Self::Event),
            "VTODO" => Some(Self::Todo),
            "VJOURNAL" => Some(Self::Journal),
            _ => None,
        }
    }
}

impl FromStr for Kind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "event" => Ok(Self::Event),
            "todo" => Ok(Self::Todo),
            "journal" => Ok(Self::Journal),
            _ => Err(format!("Unknown kind {s:?}")),
        }
    }
}

/// One concrete instance of a component, ready to display.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub calendar: String,
    /// Backing file.
    pub path: PathBuf,
    pub uid: String,
    /// Original start of this instance, for recurring components.
    pub recurrence_id: Option<When>,
    pub kind: Kind,
    pub summary: String,
    pub location: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    pub start: When,
    pub end: When,
    pub all_day: bool,
    pub recurring: bool,
    /// For tasks, when they are due (they may be shown on another day).
    pub due: Option<When>,
    /// When to remind (from VALARMs that display or make a sound).
    pub alarms: Vec<Trigger>,
    /// Who sent it, for invitations and meetings.
    pub organizer: Option<Person>,
    pub attendees: Vec<Person>,
    /// For a subtask, its parent task's UID.
    pub parent: Option<String>,
}

/// An ORGANIZER or ATTENDEE.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    /// The address, without "mailto:".
    pub email: Option<String>,
    /// CN.
    pub name: Option<String>,
    /// PARTSTAT: NEEDS-ACTION, ACCEPTED, DECLINED, TENTATIVE, DELEGATED.
    pub status: Option<String>,
    /// ROLE: REQ-PARTICIPANT, OPT-PARTICIPANT, CHAIR, NON-PARTICIPANT.
    pub role: Option<String>,
    /// CUTYPE: INDIVIDUAL, GROUP, RESOURCE, ROOM.
    pub kind: Option<String>,
}

impl Person {
    fn from_property(prop: &icalendar::Property) -> Self {
        let param = |k: &str| {
            prop.params()
                .get(k)
                .map(|p| p.value().trim_matches('"').to_owned())
                .filter(|v| !v.is_empty())
        };
        let upper = |k: &str| param(k).map(|v| v.to_ascii_uppercase());
        let value = prop.value().trim();
        let email = value
            .get(..7)
            .filter(|p| p.eq_ignore_ascii_case("mailto:"))
            .map_or(value, |_| &value[7..]);
        Self {
            email: (!email.is_empty()).then(|| email.to_owned()),
            name: param("CN"),
            status: upper("PARTSTAT"),
            role: upper("ROLE"),
            kind: upper("CUTYPE"),
        }
    }

    /// A name to show: the CN, else the address.
    pub fn display(&self) -> &str {
        self.name
            .as_deref()
            .or(self.email.as_deref())
            .unwrap_or("someone")
    }

    /// Whether this is one of `addresses` (compared ignoring case).
    pub fn is_any_of(&self, addresses: &[String]) -> bool {
        self.email
            .as_deref()
            .is_some_and(|e| addresses.iter().any(|a| a.eq_ignore_ascii_case(e)))
    }
}

/// The UID of the component's parent (RELATED-TO with RELTYPE=PARENT, the
/// default): a subtask's task.
pub fn parent_of(component: &AnyComponent) -> Option<String> {
    let single = component.properties().get("RELATED-TO").into_iter();
    let multi = component
        .multi_properties()
        .get("RELATED-TO")
        .into_iter()
        .flatten();
    let mut any = false;
    let parent = single
        .chain(multi)
        .inspect(|_| any = true)
        .find(|p| {
            p.params()
                .get("RELTYPE")
                .is_none_or(|r| r.value().eq_ignore_ascii_case("PARENT"))
        })
        .map(|p| p.value().trim().to_owned());
    match parent {
        Some(uid) => Some(uid).filter(|u| !u.is_empty()),
        // the icalendar crate keeps only the last RELATED-TO (it does not
        // list it as repeatable), so a parent link before a sibling or
        // child one is lost: read this item's own lines instead
        None if any => parent_from_file(component),
        None => None,
    }
}

/// The parent link of `component`, read from its file's lines.
fn parent_from_file(component: &AnyComponent) -> Option<String> {
    let text = std::fs::read_to_string(component.path()).ok()?;
    let doc = crate::doc::Document::parse(&text).ok()?;
    let uid = component.get_uid()?;
    let item = doc.items().find(|c| c.uid() == Some(uid))?;
    item.properties("RELATED-TO")
        .find(|l| {
            l.param("RELTYPE")
                .is_none_or(|r| r.eq_ignore_ascii_case("PARENT"))
        })
        .map(|l| l.value().trim().to_owned())
        .filter(|u| !u.is_empty())
}

/// Whether the server sent this component's invitations: CalDAV servers
/// that schedule (Nextcloud, for one) mark each guest with SCHEDULE-STATUS.
pub fn server_scheduled(component: &AnyComponent) -> bool {
    let single = component.properties().get("ATTENDEE").into_iter();
    let multi = component
        .multi_properties()
        .get("ATTENDEE")
        .into_iter()
        .flatten();
    single
        .chain(multi)
        .any(|p| p.params().contains_key("SCHEDULE-STATUS"))
}

/// The people of `name` (ATTENDEE, ORGANIZER) on a component.
pub fn people(component: &AnyComponent, name: &str) -> Vec<Person> {
    let single = component.properties().get(name).into_iter();
    let multi = component.multi_properties().get(name).into_iter().flatten();
    single.chain(multi).map(Person::from_property).collect()
}

/// When an alarm goes off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// Relative to the start, or with RELATED=END to the end.
    Relative {
        offset: TimeDelta,
        from_end: bool,
    },
    Absolute(DateTime<Utc>),
}

/// The display and audio alarms of a component (not email ones).
fn alarms(component: &AnyComponent) -> Vec<Trigger> {
    component
        .components()
        .iter()
        .filter(|c| c.component_kind() == "VALARM")
        .filter(|c| {
            !matches!(
                c.property_value("ACTION")
                    .map(str::to_ascii_uppercase)
                    .as_deref(),
                Some("EMAIL" | "NONE")
            )
        })
        .filter_map(|c| {
            let prop = c.properties().get("TRIGGER")?;
            let param = |k: &str| prop.params().get(k).map(|p| p.value().to_ascii_uppercase());
            if param("VALUE").as_deref() == Some("DATE-TIME") {
                return When::parse(prop.value(), None)
                    .map(|w| Trigger::Absolute(w.to_utc(Tz::UTC)));
            }
            Some(Trigger::Relative {
                offset: parse_duration(prop.value())?,
                from_end: param("RELATED").as_deref() == Some("END"),
            })
        })
        .collect()
}

impl Occurrence {
    pub fn duration(&self) -> TimeDelta {
        self.start.until(&self.end)
    }

    /// Whether this occurrence overlaps `range` for a viewer in `local`.
    /// Zero-length occurrences overlap if they start inside the range.
    pub fn overlaps(&self, range: &Range<DateTime<Utc>>, local: Tz) -> bool {
        overlaps(&self.start, &self.end, range, local)
    }
}

fn overlaps(start: &When, end: &When, range: &Range<DateTime<Utc>>, local: Tz) -> bool {
    let start = start.to_utc(local);
    let end = end.to_utc(local);
    if start == end {
        range.contains(&start)
    } else {
        start < range.end && end > range.start
    }
}

/// The parts of a component that occurrences are built from.
struct Fields<'a> {
    component: &'a AnyComponent,
    kind: Kind,
    uid: String,
    start: When,
    duration: TimeDelta,
}

impl<'a> Fields<'a> {
    fn new(component: &'a AnyComponent) -> Option<Self> {
        let kind = Kind::from_component_kind(&component.component_kind())?;
        let prop = |name| {
            component
                .properties()
                .get(name)
                .and_then(When::from_property)
        };

        let dtstart = prop("DTSTART");
        let (start, end) = match kind {
            Kind::Event => (dtstart?, prop("DTEND")),
            Kind::Todo => {
                let due = prop("DUE");
                match (dtstart, due) {
                    (Some(s), due) => (s, due),
                    (None, Some(due)) => (due, None),
                    (None, None) => return None,
                }
            }
            Kind::Journal => (dtstart?, None),
        };

        let duration = match end {
            // a task is due on its DUE date, so it covers that day too: as
            // for events, an occurrence's end is exclusive
            Some(end) if kind == Kind::Todo && end.is_date() => {
                start.until(&end) + TimeDelta::days(1)
            }
            Some(end) => start.until(&end),
            // a dated task with only a start or a due date is on that day
            None if start.is_date() && kind == Kind::Todo => TimeDelta::days(1),
            None => match component
                .property_value("DURATION")
                .and_then(parse_duration)
            {
                Some(d) => d.max(TimeDelta::zero()),
                // RFC 5545: an event with a DATE start and no end lasts one day
                None if start.is_date() && kind == Kind::Event => TimeDelta::days(1),
                None => TimeDelta::zero(),
            },
        };

        Some(Self {
            component,
            kind,
            uid: component.get_uid().unwrap_or_default().to_owned(),
            start,
            duration,
        })
    }

    fn recurrence_id(&self) -> Option<(When, bool)> {
        let prop = self.component.properties().get("RECURRENCE-ID")?;
        let when = When::from_property(prop)?;
        let this_and_future = prop
            .params()
            .get("RANGE")
            .is_some_and(|r| r.value().eq_ignore_ascii_case("THISANDFUTURE"));
        Some((when, this_and_future))
    }

    fn is_recurring(&self) -> bool {
        let props = self.component.properties();
        let multi = self.component.multi_properties();
        props.contains_key("RRULE") || props.contains_key("RDATE") || multi.contains_key("RDATE")
    }

    fn whens(&self, name: &str) -> Vec<When> {
        let single = self.component.properties().get(name).into_iter();
        let multi = self
            .component
            .multi_properties()
            .get(name)
            .into_iter()
            .flatten();
        single
            .chain(multi)
            .flat_map(When::list_from_property)
            .map(|w| w.coerce_like(&self.start))
            .collect()
    }

    fn occurrence(
        &self,
        calendar: &str,
        start: When,
        recurrence_id: Option<When>,
        recurring: bool,
    ) -> Occurrence {
        let text = |name| self.component.property_value(name).map(unescape_text);
        Occurrence {
            calendar: calendar.to_owned(),
            path: self.component.path().to_owned(),
            uid: self.uid.clone(),
            recurrence_id,
            kind: self.kind,
            summary: text("SUMMARY").unwrap_or_default(),
            location: text("LOCATION"),
            description: text("DESCRIPTION"),
            status: self.component.property_value("STATUS").map(str::to_owned),
            start,
            end: start.add(self.duration),
            all_day: start.is_date(),
            recurring,
            alarms: alarms(self.component),
            organizer: people(self.component, "ORGANIZER").into_iter().next(),
            attendees: people(self.component, "ATTENDEE"),
            parent: parent_of(self.component),
            due: self
                .component
                .properties()
                .get("DUE")
                .and_then(When::from_property),
        }
    }
}

/// Expand the components of one calendar into occurrences overlapping
/// `range`, as seen by a viewer in `local`.
///
/// Components sharing a UID are treated as one recurring series: the one
/// without `RECURRENCE-ID` is the master, the others override instances.
pub fn expand<'a>(
    calendar: &str,
    components: impl IntoIterator<Item = &'a AnyComponent>,
    range: &Range<DateTime<Utc>>,
    local: Tz,
) -> Vec<Occurrence> {
    let mut series: BTreeMap<String, (Option<Fields>, Vec<Fields>)> = BTreeMap::new();
    for component in components {
        let Some(fields) = Fields::new(component) else {
            continue;
        };
        // components without a UID cannot be linked, so keep them apart
        let key = if fields.uid.is_empty() {
            format!("{:?}#{}", component.path(), series.len())
        } else {
            format!("{:?}#{}", component.path(), fields.uid)
        };
        let entry = series.entry(key).or_default();
        if fields.recurrence_id().is_some() {
            entry.1.push(fields);
        } else if entry.0.is_none() {
            entry.0 = Some(fields);
        } else {
            warn!("duplicate UID {:?} in {:?}", fields.uid, component.path());
        }
    }

    let mut out = Vec::new();
    for (master, overrides) in series.values() {
        expand_series(calendar, master.as_ref(), overrides, range, local, &mut out);
    }
    out
}

fn expand_series(
    calendar: &str,
    master: Option<&Fields>,
    overrides: &[Fields],
    range: &Range<DateTime<Utc>>,
    local: Tz,
    out: &mut Vec<Occurrence>,
) {
    let Some(master) = master else {
        // orphaned overrides are shown as they are
        for o in overrides {
            let rid = o.recurrence_id().map(|(w, _)| w);
            let occ = o.occurrence(calendar, o.start, rid, true);
            if occ.overlaps(range, local) {
                out.push(occ);
            }
        }
        return;
    };

    if !master.is_recurring() {
        let occ = master.occurrence(calendar, master.start, None, false);
        if occ.overlaps(range, local) {
            out.push(occ);
        }
        return;
    }

    let mut replaced = Vec::new();
    let mut this_and_future = Vec::new();
    for o in overrides {
        let Some((rid, future)) = o.recurrence_id() else {
            continue;
        };
        let rid = rid.coerce_like(&master.start);
        replaced.push(rid);
        if future {
            this_and_future.push((rid, o));
        }
        let occ = o.occurrence(calendar, o.start, Some(rid), true);
        if occ.overlaps(range, local) {
            out.push(occ);
        }
    }
    this_and_future.sort_by_key(|(rid, _)| rid.to_utc(Tz::UTC));

    let exdates = master.whens("EXDATE");

    for start in instance_starts(master, range) {
        if exdates.contains(&start) || replaced.contains(&start) {
            continue;
        }

        // A THISANDFUTURE override reshapes every later instance.
        let latest_future = this_and_future
            .iter()
            .rev()
            .find(|(rid, _)| rid.to_utc(Tz::UTC) < start.to_utc(Tz::UTC));
        let occ = match latest_future {
            Some((rid, o)) => {
                let shift = o.start.to_utc(Tz::UTC) - rid.to_utc(Tz::UTC);
                o.occurrence(calendar, start.add(shift), Some(start), true)
            }
            None => master.occurrence(calendar, start, Some(start), true),
        };
        if occ.overlaps(range, local) {
            out.push(occ);
        }
    }
}

/// Start times of a recurring master, in order, from DTSTART until past the
/// end of `range`. Includes DTSTART and RDATEs; EXDATEs are not removed.
fn instance_starts(master: &Fields, range: &Range<DateTime<Utc>>) -> Vec<When> {
    // Stop once instances are clearly past the range. Floating and date
    // values are expanded in UTC, so allow for any viewer offset.
    let horizon = range.end + TimeDelta::days(2);
    let earliest = range.start - master.duration - TimeDelta::days(2);

    let mut starts = master.whens("RDATE");
    starts.push(master.start);

    if let Some(rule) = master.component.property_value("RRULE") {
        match rrule_set(rule, &master.start) {
            Ok(set) => starts.extend(
                set.into_iter()
                    .map(|dt| from_rrule(dt, &master.start))
                    .skip_while(|w| w.to_utc(Tz::UTC) < earliest)
                    .take_while(|w| w.to_utc(Tz::UTC) < horizon),
            ),
            Err(e) => warn!(
                "{:?}: ignoring RRULE {rule:?}: {e}",
                master.component.path()
            ),
        }
    }

    starts.sort_by_key(|w| w.to_utc(Tz::UTC));
    starts.dedup();
    starts
}

/// The timezone recurrences are expanded in: the zone of a zoned DTSTART,
/// otherwise UTC standing in for wall-clock time.
fn expansion_tz(start: &When) -> rrule::Tz {
    match start {
        When::Zoned(dt) => rrule::Tz::Tz(dt.timezone()),
        _ => rrule::Tz::UTC,
    }
}

fn to_rrule(w: &When, tz: rrule::Tz) -> DateTime<rrule::Tz> {
    match w {
        When::Date(d) => tz.from_utc_datetime(&d.and_time(NaiveTime::MIN)),
        When::Floating(ndt) => tz.from_utc_datetime(ndt),
        When::Zoned(dt) => dt.with_timezone(&tz),
    }
}

fn from_rrule(dt: DateTime<rrule::Tz>, like: &When) -> When {
    match like {
        When::Date(_) => When::Date(dt.naive_utc().date()),
        When::Floating(_) => When::Floating(dt.naive_utc()),
        When::Zoned(z) => When::Zoned(dt.with_timezone(&z.timezone())),
    }
}

fn rrule_set(rule: &str, start: &When) -> anyhow::Result<rrule::RRuleSet> {
    let tz = expansion_tz(start);
    let dtstart = to_rrule(start, tz);

    // UNTIL is handled here: the rrule crate reads a bare UNTIL in the
    // system timezone and rejects one before DTSTART.
    let mut until = None;
    let parts = rule
        .split(';')
        .filter(|part| match part.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("UNTIL") => {
                until = Some(v.to_owned());
                false
            }
            _ => true,
        })
        .collect::<Vec<_>>()
        .join(";");

    let mut rrule = rrule::RRule::from_str(&parts)?;
    if let Some(until) = until {
        let until = until_instant(&until, start)
            .ok_or_else(|| anyhow::anyhow!("invalid UNTIL {until:?}"))?;
        if until < dtstart.with_timezone(&Utc) {
            // nothing after DTSTART, which is added separately
            return Ok(rrule::RRuleSet::new(dtstart));
        }
        rrule = rrule.until(until.with_timezone(&rrule::Tz::UTC));
    }
    Ok(rrule.build(dtstart)?)
}

/// UNTIL as an instant in the expansion frame of `start`. UNTIL is
/// inclusive, so a date means the end of that day.
fn until_instant(value: &str, start: &When) -> Option<DateTime<Utc>> {
    let until = When::parse(value, None)?;
    let end_of_day = |d: chrono::NaiveDate| d.and_hms_opt(23, 59, 59).unwrap();
    Some(match (start, until) {
        (When::Zoned(s), When::Date(d)) => {
            resolve_local(s.timezone(), end_of_day(d)).with_timezone(&Utc)
        }
        (When::Zoned(s), When::Floating(ndt)) => {
            resolve_local(s.timezone(), ndt).with_timezone(&Utc)
        }
        (When::Zoned(_), When::Zoned(u)) => u.with_timezone(&Utc),
        (_, When::Date(d)) => end_of_day(d).and_utc(),
        (_, When::Floating(ndt)) => ndt.and_utc(),
        (_, When::Zoned(u)) => u.naive_utc().and_utc(),
    })
}

/// Undo iCalendar TEXT escaping.
pub fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(c) => out.push(c),
            None => out.push('\\'),
        }
    }
    out
}

impl Occurrence {
    /// When this occurrence's alarms go off, for a viewer in `local`.
    pub fn alarm_times(&self, local: Tz) -> Vec<DateTime<Utc>> {
        self.alarms
            .iter()
            .map(|t| match t {
                Trigger::Absolute(at) => *at,
                Trigger::Relative { offset, from_end } => {
                    let base = if *from_end { &self.end } else { &self.start };
                    base.to_utc(local) + *offset
                }
            })
            .collect()
    }
}

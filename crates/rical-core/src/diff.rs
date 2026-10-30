//! What an invitation or update changes in an event already in a calendar:
//! the title, the time (and whether the length changed), the place, the
//! words, who is invited, and how they answered. Worked out from the file
//! of an iTIP message (METHOD REQUEST, CANCEL, REPLY) and the calendar's
//! own copy, found by UID.

use chrono::TimeDelta;
use chrono_tz::Tz;

use crate::{
    Vdir,
    doc::{Component, Document, Line},
    occurrence::unescape_text,
    rule::{End, Rule},
    when::{When, local_timezone},
};

/// How this message stands against what the calendar has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standing {
    /// Not in the calendar.
    New,
    /// A later version than the calendar's.
    Newer,
    /// The calendar has this version (or a later one).
    Older,
    /// The same version again.
    Duplicate,
    /// The organiser cancelled it.
    Cancel,
    /// A guest's answer.
    Reply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Title,
    When,
    Place,
    Description,
    Organiser,
    Repeat,
    Status,
}

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Title => "Title",
            Field::When => "When",
            Field::Place => "Where",
            Field::Description => "Details",
            Field::Organiser => "Organiser",
            Field::Repeat => "Repeats",
            Field::Status => "Status",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineOp {
    Same,
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub field: Field,
    pub old: String,
    pub new: String,
    /// What the change amounts to: "30 minutes later, same length".
    pub note: Option<String>,
    /// For the details: the lines that changed, with a line either side.
    pub lines: Vec<(LineOp, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Guest {
    Added,
    Removed,
    /// Their answer changed: from, to.
    Answered(String, String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestChange {
    pub who: String,
    pub what: Guest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub uid: String,
    pub summary: String,
    pub method: String,
    pub standing: Standing,
    /// The calendar that has it.
    pub calendar: Option<String>,
    /// What this is about, when it is one occurrence of a repeating event.
    pub occurrence: Option<String>,
    pub changes: Vec<Change>,
    pub guests: Vec<GuestChange>,
}

impl Diff {
    /// Nothing differs from the calendar's copy.
    pub fn is_unchanged(&self) -> bool {
        self.changes.is_empty() && self.guests.is_empty()
    }
}

// ---- reading an event ----

fn text(c: &Component, name: &str) -> String {
    c.property(name)
        .map(|l| unescape_text(l.value()).trim().to_owned())
        .unwrap_or_default()
}

fn sequence(c: &Component) -> u32 {
    c.property("SEQUENCE")
        .and_then(|l| l.value().trim().parse().ok())
        .unwrap_or(0)
}

fn email_of(l: &Line) -> String {
    let v = l.value().trim();
    v.get(..7)
        .filter(|p| p.eq_ignore_ascii_case("mailto:"))
        .map_or(v, |_| &v[7..])
        .to_lowercase()
}

fn who(l: &Line) -> String {
    l.param("CN")
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| email_of(l))
}

fn start_end(c: &Component) -> Option<(When, Option<When>)> {
    let start = c.property("DTSTART").and_then(Line::when)?;
    let end = c
        .property("DTEND")
        .or_else(|| c.property("DUE"))
        .and_then(Line::when)
        .or_else(|| {
            c.property("DURATION")
                .and_then(|l| crate::when::parse_duration(l.value()))
                .map(|d| start.add(d))
        })
        .or_else(|| start.is_date().then(|| start.add(TimeDelta::days(1))));
    Some((start, end))
}

fn show_instant(w: &When, tz: Tz) -> String {
    match w {
        When::Date(d) => d.format("%a %-d %b %Y").to_string(),
        _ => {
            let t = w.to_tz(tz);
            format!("{}", t.format("%a %-d %b %Y, %H:%M"))
        }
    }
}

/// "Thu 5 Nov 2026, 14:00–15:00" in the viewer's zone.
pub fn show_when(start: &When, end: Option<&When>, tz: Tz) -> String {
    let a = show_instant(start, tz);
    let Some(end) = end else { return a };
    if start.is_date() {
        // the end of an all-day event is the day after
        let last = end.date(tz) - TimeDelta::days(1);
        return if last <= start.date(tz) {
            a
        } else {
            format!("{a} – {}", last.format("%a %-d %b %Y"))
        };
    }
    let (s, e) = (start.to_tz(tz), end.to_tz(tz));
    if s.date_naive() == e.date_naive() {
        format!("{a}–{}", e.format("%H:%M"))
    } else {
        format!("{a} – {}", show_instant(end, tz))
    }
}

fn span(d: TimeDelta) -> String {
    let secs = d.num_seconds().abs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (h, m) = (rest / 3600, rest % 3600 / 60);
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{days} day{}", if days == 1 { "" } else { "s" }));
    }
    if h > 0 {
        parts.push(format!("{h} hour{}", if h == 1 { "" } else { "s" }));
    }
    if m > 0 {
        parts.push(format!("{m} minute{}", if m == 1 { "" } else { "s" }));
    }
    if parts.is_empty() {
        "no time".to_owned()
    } else {
        parts.join(" ")
    }
}

fn describe_repeat(c: &Component) -> String {
    let Some(l) = c.property("RRULE") else {
        return "Does not repeat".to_owned();
    };
    let raw = l.value().trim();
    let Some(r) = Rule::parse(raw) else {
        return raw.to_owned();
    };
    let mut s = if r.interval == 1 {
        let mut f = r.freq.name().to_owned();
        f[..1].make_ascii_uppercase();
        f
    } else {
        let unit = match r.freq {
            crate::rule::Freq::Daily => "days",
            crate::rule::Freq::Weekly => "weeks",
            crate::rule::Freq::Monthly => "months",
            crate::rule::Freq::Yearly => "years",
        };
        format!("Every {} {unit}", r.interval)
    };
    if !r.weekdays.is_empty() {
        let days: Vec<String> = r.weekdays.iter().map(|d| d.to_string()).collect();
        s += &format!(" on {}", days.join(", "));
    }
    match r.end {
        End::Never => {}
        End::Count(n) => s += &format!(", {n} times"),
        End::Until(d) => s += &format!(", until {}", d.format("%-d %b %Y")),
    }
    s
}

// ---- comparing ----

/// A line diff of two texts, keeping only what changed and a line of
/// context either side.
pub fn line_diff(old: &str, new: &str) -> Vec<(LineOp, String)> {
    let a: Vec<&str> = old.lines().map(str::trim_end).collect();
    let b: Vec<&str> = new.lines().map(str::trim_end).collect();
    // longest common subsequence
    let mut table = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            table[i][j] = if a[i] == b[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let mut all: Vec<(LineOp, String)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i] == b[j] {
            all.push((LineOp::Same, a[i].to_owned()));
            i += 1;
            j += 1;
        } else if table[i + 1][j] >= table[i][j + 1] {
            all.push((LineOp::Removed, a[i].to_owned()));
            i += 1;
        } else {
            all.push((LineOp::Added, b[j].to_owned()));
            j += 1;
        }
    }
    all.extend(a[i..].iter().map(|l| (LineOp::Removed, (*l).to_owned())));
    all.extend(b[j..].iter().map(|l| (LineOp::Added, (*l).to_owned())));
    // trim unchanged lines more than one away from a change
    let keep: Vec<bool> = (0..all.len())
        .map(|k| {
            all[k].0 != LineOp::Same
                || (k > 0 && all[k - 1].0 != LineOp::Same)
                || all.get(k + 1).is_some_and(|n| n.0 != LineOp::Same)
        })
        .collect();
    all.into_iter()
        .zip(keep)
        .filter(|(_, k)| *k)
        .map(|(l, _)| l)
        .collect()
}

fn when_change(old: &Component, new: &Component, tz: Tz) -> Option<Change> {
    let (os, oe) = start_end(old)?;
    let (ns, ne) = start_end(new)?;
    let same_start = os.to_utc(tz) == ns.to_utc(tz) && os.is_date() == ns.is_date();
    let same_end = match (&oe, &ne) {
        (Some(a), Some(b)) => a.to_utc(tz) == b.to_utc(tz),
        (None, None) => true,
        _ => false,
    };
    if same_start && same_end {
        return None;
    }
    let len = |s: &When, e: &Option<When>| e.as_ref().map(|e| e.to_utc(tz) - s.to_utc(tz));
    let (ol, nl) = (len(&os, &oe), len(&ns, &ne));
    let shift = ns.to_utc(tz) - os.to_utc(tz);
    let mut notes = Vec::new();
    if !same_start {
        let how = if os.is_date() || ns.is_date() {
            let days = (ns.date(tz) - os.date(tz)).num_days();
            format!(
                "{} day{} {}",
                days.abs(),
                if days.abs() == 1 { "" } else { "s" },
                if days > 0 { "later" } else { "earlier" }
            )
        } else {
            format!(
                "{} {}",
                span(shift),
                if shift > TimeDelta::zero() {
                    "later"
                } else {
                    "earlier"
                }
            )
        };
        if !(os.is_date() && ns.is_date() && shift.is_zero()) {
            notes.push(how);
        }
    }
    match (ol, nl) {
        (Some(a), Some(b)) if a == b => {
            if !same_start {
                notes.push("same length".to_owned());
            }
        }
        (Some(a), Some(b)) => notes.push(format!("{} instead of {}", span(b), span(a))),
        _ => {}
    }
    Some(Change {
        field: Field::When,
        old: show_when(&os, oe.as_ref(), tz),
        new: show_when(&ns, ne.as_ref(), tz),
        note: (!notes.is_empty()).then(|| notes.join(", ")),
        lines: Vec::new(),
    })
}

fn simple(field: Field, old: String, new: String) -> Option<Change> {
    (old != new).then_some(Change {
        field,
        old,
        new,
        note: None,
        lines: Vec::new(),
    })
}

fn guests(c: &Component) -> Vec<(String, String, String)> {
    c.properties("ATTENDEE")
        .map(|l| {
            (
                email_of(l),
                who(l),
                l.param("PARTSTAT")
                    .unwrap_or_else(|| "NEEDS-ACTION".to_owned())
                    .to_ascii_uppercase(),
            )
        })
        .collect()
}

/// The words for an answer.
pub fn answer(partstat: &str) -> &'static str {
    match partstat {
        "ACCEPTED" => "accepted",
        "DECLINED" => "declined",
        "TENTATIVE" => "said maybe",
        "DELEGATED" => "delegated",
        _ => "has not answered",
    }
}

fn guest_changes(old: &Component, new: &Component) -> Vec<GuestChange> {
    let (o, n) = (guests(old), guests(new));
    let mut out = Vec::new();
    for (email, name, stat) in &n {
        match o.iter().find(|(e, _, _)| e == email) {
            None => out.push(GuestChange {
                who: name.clone(),
                what: Guest::Added,
            }),
            Some((_, _, before)) if before != stat => out.push(GuestChange {
                who: name.clone(),
                what: Guest::Answered(before.clone(), stat.clone()),
            }),
            Some(_) => {}
        }
    }
    for (email, name, _) in &o {
        if !n.iter().any(|(e, _, _)| e == email) {
            out.push(GuestChange {
                who: name.clone(),
                what: Guest::Removed,
            });
        }
    }
    out
}

/// How `new` differs from `old`, two versions of the same event (or of one
/// occurrence of it).
pub fn compare(old: &Component, new: &Component, tz: Tz) -> (Vec<Change>, Vec<GuestChange>) {
    let mut changes: Vec<Change> = Vec::new();
    changes.extend(simple(
        Field::Title,
        text(old, "SUMMARY"),
        text(new, "SUMMARY"),
    ));
    changes.extend(when_change(old, new, tz));
    changes.extend(simple(
        Field::Place,
        text(old, "LOCATION"),
        text(new, "LOCATION"),
    ));
    changes.extend(simple(
        Field::Repeat,
        describe_repeat(old),
        describe_repeat(new),
    ));
    let (od, nd) = (text(old, "DESCRIPTION"), text(new, "DESCRIPTION"));
    if od != nd {
        changes.push(Change {
            field: Field::Description,
            old: od.clone(),
            new: nd.clone(),
            note: None,
            lines: line_diff(&od, &nd),
        });
    }
    let org = |c: &Component| c.property("ORGANIZER").map(who).unwrap_or_default();
    changes.extend(simple(Field::Organiser, org(old), org(new)));
    let status = |c: &Component| text(c, "STATUS").to_lowercase();
    changes.extend(simple(Field::Status, status(old), status(new)));
    (changes, guest_changes(old, new))
}

impl Vdir {
    /// What each event in the iTIP file `ics` changes in the calendars:
    /// one entry per event (and per changed occurrence of a repeating one).
    pub fn diff_invite(&self, ics: &str) -> anyhow::Result<Vec<Diff>> {
        let tz = local_timezone();
        let incoming = Document::parse(ics)?;
        let method = incoming
            .root
            .property("METHOD")
            .map(|l| l.value().trim().to_ascii_uppercase())
            .unwrap_or_default();
        let mut uids: Vec<String> = Vec::new();
        for c in incoming.items() {
            if let Some(u) = c.uid()
                && !uids.iter().any(|x| x == u)
            {
                uids.push(u.to_owned());
            }
        }
        let mut out = Vec::new();
        for uid in uids {
            let comps: Vec<&Component> =
                incoming.items().filter(|c| c.uid() == Some(&uid)).collect();
            let existing = self
                .path_of(&uid)
                .and_then(|p| crate::write::read(&p).ok().map(|t| (p, t)))
                .and_then(|(p, t)| Document::parse(&t).ok().map(|d| (p, d)));
            for c in comps {
                let rid = c.recurrence_id();
                let old_doc = existing.as_ref().map(|(_, d)| d);
                // an occurrence is compared with its own override, else the series
                let old = old_doc.and_then(|d| {
                    rid.as_ref()
                        .and_then(|r| d.find(&uid, Some(r)))
                        .or_else(|| d.find(&uid, None))
                });
                let calendar = existing
                    .as_ref()
                    .and_then(|(p, _)| self.calendar_for_path(p).map(|cal| cal.name.clone()));
                let mut d = Diff {
                    uid: uid.clone(),
                    summary: text(c, "SUMMARY"),
                    method: method.clone(),
                    standing: Standing::New,
                    calendar,
                    occurrence: rid.as_ref().map(|r| show_instant(r, tz)),
                    changes: Vec::new(),
                    guests: Vec::new(),
                };
                if let Some(old) = old {
                    if d.summary.is_empty() {
                        d.summary = text(old, "SUMMARY");
                    }
                    let newer = sequence(c) > sequence(old)
                        || (sequence(c) == sequence(old)
                            && c.property("DTSTAMP").map(|l| l.value().to_owned())
                                > old.property("DTSTAMP").map(|l| l.value().to_owned()));
                    d.standing = match method.as_str() {
                        "CANCEL" => Standing::Cancel,
                        "REPLY" => Standing::Reply,
                        _ if newer => Standing::Newer,
                        _ if sequence(c) == sequence(old)
                            && c.property("DTSTAMP").map(Line::value)
                                == old.property("DTSTAMP").map(Line::value) =>
                        {
                            Standing::Duplicate
                        }
                        _ => Standing::Older,
                    };
                    match d.standing {
                        Standing::Reply => d.guests = guest_changes(old, c),
                        Standing::Cancel => {}
                        // only an update changes anything: against a version the
                        // calendar has already, differences are the calendar copy's own
                        Standing::Newer => {
                            let (ch, g) = compare(old, c, tz);
                            d.changes = ch;
                            d.guests = g;
                        }
                        Standing::New | Standing::Duplicate | Standing::Older => {}
                    }
                } else if method == "CANCEL" {
                    d.standing = Standing::Cancel;
                }
                out.push(d);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comp(extra: &str) -> Component {
        let summary = if extra.contains("SUMMARY") {
            ""
        } else {
            "SUMMARY:Planning\n"
        };
        let ics = format!(
            "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:u1\n{summary}DTSTAMP:20261001T100000Z\n{extra}END:VEVENT\nEND:VCALENDAR\n"
        );
        Document::parse(&ics)
            .unwrap()
            .items()
            .next()
            .unwrap()
            .clone()
    }

    #[test]
    fn a_moved_event_says_how_far_and_whether_the_length_changed() {
        let old = comp(
            "DTSTART;TZID=Europe/London:20261105T140000\nDTEND;TZID=Europe/London:20261105T150000\n",
        );
        // the same hour, written in UTC, is no change
        let same = comp("DTSTART:20261105T140000Z\nDTEND:20261105T150000Z\n");
        assert!(compare(&old, &same, chrono_tz::Europe::London).0.is_empty());
        let later = comp(
            "DTSTART;TZID=Europe/London:20261105T153000\nDTEND;TZID=Europe/London:20261105T163000\n",
        );
        let (c, _) = compare(&old, &later, chrono_tz::Europe::London);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].old, "Thu 5 Nov 2026, 14:00–15:00");
        assert_eq!(c[0].new, "Thu 5 Nov 2026, 15:30–16:30");
        assert_eq!(
            c[0].note.as_deref(),
            Some("1 hour 30 minutes later, same length")
        );
        let longer = comp(
            "DTSTART;TZID=Europe/London:20261105T140000\nDTEND;TZID=Europe/London:20261105T163000\n",
        );
        let (c, _) = compare(&old, &longer, chrono_tz::Europe::London);
        assert_eq!(
            c[0].note.as_deref(),
            Some("2 hours 30 minutes instead of 1 hour")
        );
    }

    #[test]
    fn places_titles_repeats_and_details() {
        let old = comp(
            "DTSTART:20261105T140000Z\nLOCATION:Room 4\nDESCRIPTION:Agenda\\nitem one\\nitem two\nRRULE:FREQ=WEEKLY;COUNT=4\n",
        );
        let new = comp(
            "DTSTART:20261105T140000Z\nSUMMARY:Planning, Q4\nLOCATION:Room 9\nDESCRIPTION:Agenda\\nitem one\\nitem three\nRRULE:FREQ=WEEKLY;INTERVAL=2\n",
        );
        let (c, _) = compare(&old, &new, chrono_tz::UTC);
        let fields: Vec<Field> = c.iter().map(|c| c.field).collect();
        assert_eq!(
            fields,
            [
                Field::Title,
                Field::Place,
                Field::Repeat,
                Field::Description
            ]
        );
        assert_eq!(c[2].old, "Weekly, 4 times");
        assert_eq!(c[2].new, "Every 2 weeks");
        assert_eq!(
            c[3].lines,
            [
                (LineOp::Same, "item one".into()),
                (LineOp::Removed, "item two".into()),
                (LineOp::Added, "item three".into())
            ]
        );
    }

    #[test]
    fn guests_added_removed_and_answers() {
        let old = comp(
            "ATTENDEE;CN=Sam;PARTSTAT=NEEDS-ACTION:mailto:sam@x.example\nATTENDEE;CN=Bo;PARTSTAT=ACCEPTED:mailto:bo@x.example\n",
        );
        let new = comp(
            "ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:SAM@x.example\nATTENDEE;CN=Cy:mailto:cy@x.example\n",
        );
        let (_, g) = compare(&old, &new, chrono_tz::UTC);
        assert_eq!(
            g,
            [
                GuestChange {
                    who: "Sam".into(),
                    what: Guest::Answered("NEEDS-ACTION".into(), "ACCEPTED".into())
                },
                GuestChange {
                    who: "Cy".into(),
                    what: Guest::Added
                },
                GuestChange {
                    who: "Bo".into(),
                    what: Guest::Removed
                },
            ]
        );
    }

    #[test]
    fn all_day_events_and_the_end_day() {
        let a = When::parse("20261105", None).unwrap();
        let end = When::parse("20261108", None).unwrap();
        assert_eq!(
            show_when(&a, Some(&end), chrono_tz::UTC),
            "Thu 5 Nov 2026 – Sat 7 Nov 2026"
        );
        let one = When::parse("20261106", None).unwrap();
        assert_eq!(show_when(&a, Some(&one), chrono_tz::UTC), "Thu 5 Nov 2026");
    }
}

#[cfg(test)]
mod vdir_tests {
    use super::*;

    const OLD: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:climb-1\r\nSEQUENCE:0\r\nDTSTAMP:20261001T100000Z\r\nSUMMARY:Climbing\r\nDTSTART:20261101T093000Z\r\nDTEND:20261101T150000Z\r\nLOCATION:Almscliff\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    fn invite(method: &str, seq: u32, extra: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:{method}\r\nBEGIN:VEVENT\r\nUID:climb-1\r\nSEQUENCE:{seq}\r\nDTSTAMP:20261020T100000Z\r\nSUMMARY:Climbing\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    }

    #[test]
    fn against_the_calendar() {
        let dir = tempfile::tempdir().unwrap();
        let cal = dir.path().join("personal");
        std::fs::create_dir_all(&cal).unwrap();
        std::fs::write(cal.join("climb-1.ics"), OLD).unwrap();
        let vdir = Vdir::load(dir.path()).unwrap();

        // a new version: later, somewhere else
        let d = &vdir
            .diff_invite(&invite(
                "REQUEST",
                1,
                "DTSTART:20261101T103000Z\r\nDTEND:20261101T160000Z\r\nLOCATION:Caley\r\n",
            ))
            .unwrap()[0];
        assert_eq!(d.standing, Standing::Newer);
        assert_eq!(d.calendar.as_deref(), Some("personal"));
        let fields: Vec<Field> = d.changes.iter().map(|c| c.field).collect();
        assert_eq!(fields, [Field::When, Field::Place]);

        // the same version again, and an older one
        let same = &vdir
            .diff_invite(&OLD.replace(
                "BEGIN:VCALENDAR\r\n",
                "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\n",
            ))
            .unwrap()[0];
        assert_eq!(same.standing, Standing::Duplicate);
        assert!(same.is_unchanged());
        let older = &vdir
            .diff_invite(
                &invite("REQUEST", 0, "DTSTART:20261101T093000Z\r\n")
                    .replace("20261020", "20260901"),
            )
            .unwrap()[0];
        assert_eq!(older.standing, Standing::Older);

        // unknown, cancelled, answered
        let new = &vdir
            .diff_invite(
                &invite("REQUEST", 0, "DTSTART:20261101T093000Z\r\n").replace("climb-1", "other"),
            )
            .unwrap()[0];
        assert_eq!(new.standing, Standing::New);
        let cancel = &vdir.diff_invite(&invite("CANCEL", 2, "")).unwrap()[0];
        assert_eq!(cancel.standing, Standing::Cancel);
        let reply = &vdir
            .diff_invite(&invite(
                "REPLY",
                0,
                "ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@x.example\r\n",
            ))
            .unwrap()[0];
        assert_eq!(reply.standing, Standing::Reply);
        assert_eq!(reply.guests[0].what, Guest::Added);
    }
}

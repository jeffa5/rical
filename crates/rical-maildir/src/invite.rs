//! A calendar invitation as mail carries it (a `text/calendar` part): just
//! enough of the iCalendar to say what it is. The calendar app does the
//! rest.

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone as _, Utc};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Invite {
    /// REQUEST, REPLY, CANCEL, PUBLISH…
    pub method: String,
    pub summary: String,
    pub start: Option<When>,
    pub end: Option<When>,
    pub location: String,
    pub organizer: String,
    pub description: String,
    /// More than one event in the file.
    pub events: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum When {
    Date(NaiveDate),
    /// In UTC.
    Utc(NaiveDateTime),
    /// On a wall clock in a named zone.
    Zoned(NaiveDateTime, String),
    Floating(NaiveDateTime),
}

impl When {
    /// "Thu 5 Nov 2026, 14:00" in the viewer's zone when it is known.
    pub fn shown(&self) -> String {
        match self {
            When::Date(d) => d.format("%a %-d %b %Y").to_string(),
            When::Utc(t) => {
                let local: DateTime<Local> = Utc.from_utc_datetime(t).with_timezone(&Local);
                local.format("%a %-d %b %Y, %H:%M").to_string()
            }
            When::Zoned(t, tz) => format!("{} ({tz})", t.format("%a %-d %b %Y, %H:%M")),
            When::Floating(t) => t.format("%a %-d %b %Y, %H:%M").to_string(),
        }
    }

    /// The date and the time of day as shown, to tell a same-day range.
    fn local_parts(&self) -> Option<(NaiveDate, Option<String>)> {
        match self {
            When::Date(d) => Some((*d, None)),
            When::Zoned(t, _) | When::Floating(t) => {
                Some((t.date(), Some(t.format("%H:%M").to_string())))
            }
            When::Utc(t) => {
                let l: DateTime<Local> = Utc.from_utc_datetime(t).with_timezone(&Local);
                Some((l.date_naive(), Some(l.format("%H:%M").to_string())))
            }
        }
    }
}

fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
        } else {
            out.push(line.to_owned());
        }
    }
    out
}

fn unescape(v: &str) -> String {
    let mut out = String::new();
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn when(params: &str, value: &str) -> Option<When> {
    let value = value.trim();
    if params.to_uppercase().contains("VALUE=DATE") && !params.to_uppercase().contains("DATE-TIME")
        || value.len() == 8
    {
        return NaiveDate::parse_from_str(value, "%Y%m%d")
            .ok()
            .map(When::Date);
    }
    let utc = value.ends_with('Z');
    let t = NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
    if utc {
        return Some(When::Utc(t));
    }
    let tzid = params
        .split(';')
        .find_map(|p| p.strip_prefix("TZID=").or_else(|| p.strip_prefix("tzid=")));
    Some(match tzid {
        Some(tz) => When::Zoned(t, tz.trim_matches('"').to_owned()),
        None => When::Floating(t),
    })
}

/// Read the first event of an iCalendar.
pub fn parse(ics: &str) -> Option<Invite> {
    let mut inv = Invite::default();
    let mut in_event = false;
    let mut seen_event = false;
    for line in unfold(ics) {
        let upper = line.to_uppercase();
        if upper == "BEGIN:VEVENT" {
            inv.events += 1;
            in_event = !seen_event;
            seen_event = true;
            continue;
        }
        if upper == "END:VEVENT" {
            in_event = false;
            continue;
        }
        let Some((name_params, value)) = line.split_once(':') else {
            continue;
        };
        let (name, params) = name_params.split_once(';').unwrap_or((name_params, ""));
        match (name.to_uppercase().as_str(), in_event) {
            ("METHOD", _) => inv.method = value.trim().to_uppercase(),
            ("SUMMARY", true) => inv.summary = unescape(value),
            ("LOCATION", true) => inv.location = unescape(value),
            ("DESCRIPTION", true) => inv.description = unescape(value),
            ("DTSTART", true) => inv.start = when(params, value),
            ("DTEND", true) => inv.end = when(params, value),
            ("ORGANIZER", true) => {
                let cn = params
                    .split(';')
                    .find_map(|p| p.strip_prefix("CN="))
                    .map(|c| c.trim_matches('"').to_owned());
                let addr = value
                    .trim()
                    .trim_start_matches("mailto:")
                    .trim_start_matches("MAILTO:");
                inv.organizer = match cn {
                    Some(c) => format!("{c} <{addr}>"),
                    None => addr.to_owned(),
                };
            }
            _ => {}
        }
    }
    (inv.events > 0).then_some(inv)
}

impl Invite {
    /// "Thu 5 Nov 2026, 14:00 – 15:00" or one date.
    pub fn when_shown(&self) -> String {
        let Some(s) = &self.start else {
            return String::new();
        };
        let a = s.shown();
        match &self.end {
            Some(e) if e == s => a,
            Some(e) => match (s.local_parts(), e.local_parts()) {
                // the same day: only the end's time
                (Some((d1, _)), Some((d2, t2))) if d1 == d2 && t2.is_some() => {
                    format!("{a} – {}", t2.unwrap_or_default())
                }
                _ => format!("{a} – {}", e.shown()),
            },
            None => a,
        }
    }

    /// What it is, for a heading: "Invitation", "Cancelled", "Reply"…
    pub fn kind(&self) -> &'static str {
        match self.method.as_str() {
            "REQUEST" => "Invitation",
            "CANCEL" => "Cancelled event",
            "REPLY" => "Reply to an invitation",
            "COUNTER" => "Proposed new time",
            "REFRESH" => "Request for the latest",
            "PUBLISH" | "" => "Event",
            _ => "Calendar message",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_invitation() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nSUMMARY:Planning\\, Q4\r\nDTSTART;TZID=Europe/London:20261105T140000\r\nDTEND;TZID=Europe/London:20261105T150000\r\nLOCATION:Room 4\r\nORGANIZER;CN=\"Ann Lee\":mailto:ann@x.example\r\nDESCRIPTION:Agenda\\nitem one\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let i = parse(ics).unwrap();
        assert_eq!(i.method, "REQUEST");
        assert_eq!(i.summary, "Planning, Q4");
        assert_eq!(i.organizer, "Ann Lee <ann@x.example>");
        assert_eq!(i.kind(), "Invitation");
        assert_eq!(
            i.when_shown(),
            "Thu 5 Nov 2026, 14:00 (Europe/London) – 15:00"
        );
        assert_eq!(i.description, "Agenda\nitem one");
    }

    #[test]
    fn all_day_and_nothing() {
        let i = parse("BEGIN:VEVENT\nSUMMARY:Holiday\nDTSTART;VALUE=DATE:20261225\nEND:VEVENT\n")
            .unwrap();
        assert_eq!(i.when_shown(), "Fri 25 Dec 2026");
        assert!(parse("BEGIN:VCALENDAR\nEND:VCALENDAR\n").is_none());
    }
}

/// An answer to an invitation, to send back to the organiser as mail
/// (iMIP: `text/calendar; method=REPLY`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    /// The iCalendar to send.
    pub ics: String,
    /// The organiser's address.
    pub to: String,
    pub subject: String,
    /// A line saying it in words, for the plain-text part.
    pub words: String,
}

fn mailto_of(value: &str) -> String {
    let v = value.trim();
    v.get(..7)
        .filter(|p| p.eq_ignore_ascii_case("mailto:"))
        .map_or(v, |_| &v[7..])
        .to_owned()
}

/// The answer `partstat` (ACCEPTED, TENTATIVE or DECLINED) from `me` (an
/// address) to the first event of the invitation `ics`.
pub fn reply_to(
    ics: &str,
    me: &str,
    name: &str,
    partstat: &str,
    now: DateTime<Utc>,
) -> Option<Reply> {
    let mut in_event = false;
    let mut kept: Vec<String> = Vec::new();
    let mut organiser = String::new();
    let mut summary = String::new();
    let mut mine: Option<String> = None;
    let mut seen = false;
    for line in unfold(ics) {
        let upper = line.to_uppercase();
        if upper == "BEGIN:VEVENT" {
            in_event = !seen;
            seen = true;
            continue;
        }
        if upper == "END:VEVENT" {
            in_event = false;
            continue;
        }
        if !in_event {
            continue;
        }
        let Some((np, value)) = line.split_once(':') else {
            continue;
        };
        let key = np.split(';').next().unwrap_or("").to_uppercase();
        match key.as_str() {
            "UID" | "SEQUENCE" | "DTSTART" | "DTEND" | "RECURRENCE-ID" | "SUMMARY" | "LOCATION" => {
                if key == "SUMMARY" {
                    summary = unescape(value);
                }
                kept.push(line.clone());
            }
            "ORGANIZER" => {
                organiser = mailto_of(value);
                kept.push(line.clone());
            }
            "ATTENDEE" if mailto_of(value).eq_ignore_ascii_case(me) => {
                let cn = np
                    .split(';')
                    .find_map(|p| p.strip_prefix("CN="))
                    .map(str::to_owned)
                    .or_else(|| (!name.is_empty()).then(|| format!("\"{name}\"")));
                mine = Some(format!(
                    "ATTENDEE;{}PARTSTAT={partstat}:mailto:{me}",
                    cn.map(|c| format!("CN={c};")).unwrap_or_default()
                ));
            }
            _ => {}
        }
    }
    if !seen || organiser.is_empty() {
        return None;
    }
    let attendee = mine.unwrap_or_else(|| format!("ATTENDEE;PARTSTAT={partstat}:mailto:{me}"));
    let mut out = String::from(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//rical//mail//EN\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\n",
    );
    for l in &kept {
        out.push_str(l);
        out.push_str("\r\n");
    }
    out.push_str(&format!("DTSTAMP:{}\r\n", now.format("%Y%m%dT%H%M%SZ")));
    out.push_str(&attendee);
    out.push_str("\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n");
    let (verb, noun) = match partstat {
        "ACCEPTED" => ("Accepted", "has accepted"),
        "DECLINED" => ("Declined", "has declined"),
        _ => ("Tentatively accepted", "might come to"),
    };
    let who = if name.is_empty() { me } else { name };
    Some(Reply {
        ics: out,
        to: organiser,
        subject: format!("{verb}: {summary}"),
        words: format!("{who} {noun}: {summary}"),
    })
}

#[cfg(test)]
mod reply_tests {
    use super::*;

    #[test]
    fn answers_keep_what_the_organiser_matches_on() {
        let ics = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:abc@x\r\nSEQUENCE:2\r\nDTSTART;TZID=Europe/London:20261105T140000\r\nSUMMARY:Planning\\, Q4\r\nORGANIZER;CN=Ann:mailto:ann@x.example\r\nATTENDEE;CN=Me;RSVP=TRUE:mailto:me@y.example\r\nATTENDEE:mailto:bo@z.example\r\nDESCRIPTION:secret agenda\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let now = DateTime::parse_from_rfc3339("2026-10-30T10:00:00Z")
            .unwrap()
            .to_utc();
        let r = reply_to(ics, "me@y.example", "Me", "ACCEPTED", now).unwrap();
        assert_eq!(r.to, "ann@x.example");
        assert_eq!(r.subject, "Accepted: Planning, Q4");
        for want in [
            "METHOD:REPLY",
            "UID:abc@x",
            "SEQUENCE:2",
            "DTSTAMP:20261030T100000Z",
            "ATTENDEE;CN=Me;PARTSTAT=ACCEPTED:mailto:me@y.example",
            "ORGANIZER;CN=Ann:mailto:ann@x.example",
        ] {
            assert!(r.ics.contains(want), "{want}\n{}", r.ics);
        }
        // not the other guests, not the agenda
        assert!(!r.ics.contains("bo@z") && !r.ics.contains("secret"));
        assert!(parse(&r.ics).is_some());
        assert!(
            reply_to(
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
                "me@y.example",
                "",
                "DECLINED",
                now
            )
            .is_none()
        );
    }
}

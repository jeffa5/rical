use std::{ops::Range, path::Path, sync::Arc};

use chrono::{DateTime, NaiveDateTime, Utc};
use chrono_tz::Tz;
use rical_core::{AnyComponent, Kind, Occurrence, When, occurrence::expand};

const LONDON: Tz = Tz::Europe__London;

fn components(body: &str) -> Vec<AnyComponent> {
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//rical//tests//EN\r\n{}END:VCALENDAR\r\n",
        body.trim_start()
            .lines()
            .map(|l| format!("{}\r\n", l.trim_start()))
            .collect::<String>()
    );
    rical_core::compact::parse(&ics)
        .unwrap()
        .0
        .into_iter()
        .map(|c| AnyComponent::new(Arc::<Path>::from(Path::new("test.ics")), c))
        .collect()
}

fn utc(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
        .unwrap()
        .and_utc()
}

fn range(from: &str, to: &str) -> Range<DateTime<Utc>> {
    utc(from)..utc(to)
}

fn occurrences(body: &str, range: Range<DateTime<Utc>>, local: Tz) -> Vec<Occurrence> {
    let mut occ = expand("cal", &components(body), &range, local);
    occ.sort_by_key(|o| o.start.to_utc(local));
    occ
}

/// Starts rendered in the viewer's timezone.
fn starts(occ: &[Occurrence], local: Tz) -> Vec<String> {
    occ.iter()
        .map(|o| match o.start {
            When::Date(d) => d.to_string(),
            _ => o.start.to_tz(local).format("%Y-%m-%d %H:%M").to_string(),
        })
        .collect()
}

#[test]
fn single_event() {
    let body = "
        BEGIN:VEVENT
        UID:one
        DTSTART:20250304T140000Z
        DTEND:20250304T153000Z
        SUMMARY:Review\\, with notes
        LOCATION:Room 1
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-03-01 00:00", "2025-03-08 00:00"), Tz::UTC);
    assert_eq!(occ.len(), 1);
    let o = &occ[0];
    assert_eq!(o.summary, "Review, with notes", "text is unescaped");
    assert_eq!(o.location.as_deref(), Some("Room 1"));
    assert_eq!(o.kind, Kind::Event);
    assert_eq!(o.duration(), chrono::TimeDelta::minutes(90));
    assert!(!o.recurring);
    assert!(!o.all_day);

    // outside the range, or touching only its edge
    assert!(occurrences(body, range("2025-03-04 15:30", "2025-03-05 00:00"), Tz::UTC).is_empty());
    assert!(occurrences(body, range("2025-03-01 00:00", "2025-03-04 14:00"), Tz::UTC).is_empty());
    // range starting mid-event still includes it
    assert_eq!(
        occurrences(body, range("2025-03-04 15:00", "2025-03-05 00:00"), Tz::UTC).len(),
        1
    );
}

#[test]
fn weekly_with_tzid_keeps_wall_clock_across_dst() {
    let body = "
        BEGIN:VEVENT
        UID:weekly
        DTSTART;TZID=Europe/London:20250317T090000
        DTEND;TZID=Europe/London:20250317T093000
        RRULE:FREQ=WEEKLY;BYDAY=MO
        SUMMARY:Standup
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-03-15 00:00", "2025-04-08 00:00"), LONDON);
    assert_eq!(
        starts(&occ, LONDON),
        [
            "2025-03-17 09:00",
            "2025-03-24 09:00",
            "2025-03-31 09:00",
            "2025-04-07 09:00"
        ]
    );
    // 09:00 GMT then 09:00 BST
    assert_eq!(occ[1].start.to_utc(LONDON), utc("2025-03-24 09:00"));
    assert_eq!(occ[2].start.to_utc(LONDON), utc("2025-03-31 08:00"));
    assert!(occ.iter().all(|o| o.recurring));
    assert_eq!(occ[2].recurrence_id, Some(occ[2].start));
    assert!(
        occ.iter()
            .all(|o| o.duration() == chrono::TimeDelta::minutes(30))
    );
}

#[test]
fn monthly_by_day() {
    let body = "
        BEGIN:VEVENT
        UID:monthly
        DTSTART:20250107T180000Z
        DTEND:20250107T190000Z
        RRULE:FREQ=MONTHLY;BYDAY=1TU
        SUMMARY:Book club
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-01-01 00:00", "2025-06-01 00:00"), Tz::UTC);
    assert_eq!(
        starts(&occ, Tz::UTC),
        [
            "2025-01-07 18:00",
            "2025-02-04 18:00",
            "2025-03-04 18:00",
            "2025-04-01 18:00",
            "2025-05-06 18:00"
        ]
    );
}

#[test]
fn count_and_until() {
    let count = "
        BEGIN:VEVENT
        UID:count
        DTSTART:20250101T100000Z
        RRULE:FREQ=DAILY;COUNT=3
        END:VEVENT
    ";
    let r = range("2024-12-01 00:00", "2025-02-01 00:00");
    assert_eq!(
        starts(&occurrences(count, r.clone(), Tz::UTC), Tz::UTC),
        ["2025-01-01 10:00", "2025-01-02 10:00", "2025-01-03 10:00"]
    );

    // UNTIL is inclusive; a floating UNTIL with a TZID start is read in that zone
    let until = "
        BEGIN:VEVENT
        UID:until
        DTSTART;TZID=Europe/London:20250101T100000
        RRULE:FREQ=DAILY;UNTIL=20250103T100000
        END:VEVENT
    ";
    assert_eq!(occurrences(until, r.clone(), LONDON).len(), 3);

    let until_utc = "
        BEGIN:VEVENT
        UID:until-utc
        DTSTART;TZID=Europe/London:20250101T100000
        RRULE:FREQ=DAILY;UNTIL=20250102T235959Z
        END:VEVENT
    ";
    assert_eq!(occurrences(until_utc, r.clone(), LONDON).len(), 2);

    // UNTIL before DTSTART still shows DTSTART
    let backwards = "
        BEGIN:VEVENT
        UID:backwards
        DTSTART:20250101T100000Z
        RRULE:FREQ=DAILY;UNTIL=20240101T000000Z
        END:VEVENT
    ";
    assert_eq!(occurrences(backwards, r, Tz::UTC).len(), 1);
}

#[test]
fn exdate_and_rdate() {
    let body = "
        BEGIN:VEVENT
        UID:ex
        DTSTART;TZID=Europe/London:20250707T090000
        DTEND;TZID=Europe/London:20250707T100000
        RRULE:FREQ=DAILY;COUNT=5
        EXDATE;TZID=Europe/London:20250708T090000,20250709T090000
        EXDATE:20250710T080000Z
        RDATE;TZID=Europe/London:20250720T150000
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-07-01 00:00", "2025-08-01 00:00"), LONDON);
    assert_eq!(
        starts(&occ, LONDON),
        ["2025-07-07 09:00", "2025-07-11 09:00", "2025-07-20 15:00"]
    );
    assert!(
        occ.iter()
            .all(|o| o.duration() == chrono::TimeDelta::hours(1))
    );
}

#[test]
fn rdate_only() {
    let body = "
        BEGIN:VEVENT
        UID:rdate
        DTSTART:20250101T100000Z
        RDATE:20250105T100000Z
        RDATE:20250110T100000Z/PT2H
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-01-01 00:00", "2025-02-01 00:00"), Tz::UTC);
    assert_eq!(
        starts(&occ, Tz::UTC),
        ["2025-01-01 10:00", "2025-01-05 10:00", "2025-01-10 10:00"]
    );
}

#[test]
fn overrides_move_and_change_instances() {
    let body = "
        BEGIN:VEVENT
        UID:series
        DTSTART:20250303T090000Z
        DTEND:20250303T100000Z
        RRULE:FREQ=WEEKLY;COUNT=4
        SUMMARY:Weekly
        END:VEVENT
        BEGIN:VEVENT
        UID:series
        RECURRENCE-ID:20250310T090000Z
        DTSTART:20250311T140000Z
        DTEND:20250311T150000Z
        SUMMARY:Moved
        END:VEVENT
        BEGIN:VEVENT
        UID:series
        RECURRENCE-ID:20250324T090000Z
        DTSTART:20250401T090000Z
        DTEND:20250401T100000Z
        SUMMARY:Moved out of range
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-03-01 00:00", "2025-03-31 00:00"), Tz::UTC);
    let got = occ
        .iter()
        .map(|o| (o.start.to_string(), o.summary.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        got,
        [
            ("2025-03-03 09:00 UTC".to_owned(), "Weekly"),
            ("2025-03-11 14:00 UTC".to_owned(), "Moved"),
            ("2025-03-17 09:00 UTC".to_owned(), "Weekly"),
        ]
    );
    assert_eq!(
        occ[1].recurrence_id,
        When::parse("20250310T090000Z", None),
        "override keeps the instance it replaces"
    );

    // an instance moved into the range from outside it
    let occ = occurrences(body, range("2025-04-01 00:00", "2025-04-02 00:00"), Tz::UTC);
    assert_eq!(occ.len(), 1);
    assert_eq!(occ[0].summary, "Moved out of range");
}

#[test]
fn this_and_future() {
    let body = "
        BEGIN:VEVENT
        UID:split
        DTSTART:20250303T090000Z
        DTEND:20250303T100000Z
        RRULE:FREQ=WEEKLY;COUNT=4
        SUMMARY:Old
        END:VEVENT
        BEGIN:VEVENT
        UID:split
        RECURRENCE-ID;RANGE=THISANDFUTURE:20250310T090000Z
        DTSTART:20250310T110000Z
        DTEND:20250310T113000Z
        SUMMARY:New
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-03-01 00:00", "2025-04-01 00:00"), Tz::UTC);
    let got = occ
        .iter()
        .map(|o| {
            (
                o.start.to_string(),
                o.summary.as_str(),
                o.duration().num_minutes(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        got,
        [
            ("2025-03-03 09:00 UTC".to_owned(), "Old", 60),
            ("2025-03-10 11:00 UTC".to_owned(), "New", 30),
            ("2025-03-17 11:00 UTC".to_owned(), "New", 30),
            ("2025-03-24 11:00 UTC".to_owned(), "New", 30),
        ]
    );
}

#[test]
fn all_day_yearly_birthday() {
    let body = "
        BEGIN:VEVENT
        UID:bday
        DTSTART;VALUE=DATE:19700123
        RRULE:FREQ=YEARLY
        SUMMARY:Birthday
        END:VEVENT
    ";
    // all-day values are dates, whatever the viewer's timezone
    for tz in [Tz::UTC, Tz::Pacific__Auckland, Tz::America__Los_Angeles] {
        let occ = occurrences(body, range("2025-01-20 00:00", "2025-01-27 00:00"), tz);
        assert_eq!(starts(&occ, tz), ["2025-01-23"], "{tz}");
        assert!(occ[0].all_day);
        assert_eq!(occ[0].end, When::parse("20250124", None).unwrap());
    }
}

#[test]
fn multi_day_events() {
    let body = "
        BEGIN:VEVENT
        UID:trip
        DTSTART;VALUE=DATE:20250310
        DTEND;VALUE=DATE:20250314
        SUMMARY:Trip
        END:VEVENT
        BEGIN:VEVENT
        UID:overnight
        DTSTART:20250312T220000Z
        DURATION:PT10H
        SUMMARY:Overnight
        END:VEVENT
    ";
    // a range in the middle of both still includes them
    let occ = occurrences(body, range("2025-03-13 00:00", "2025-03-13 06:00"), Tz::UTC);
    assert_eq!(
        occ.iter().map(|o| o.summary.as_str()).collect::<Vec<_>>(),
        ["Trip", "Overnight"]
    );
    assert_eq!(occ[0].duration(), chrono::TimeDelta::days(4));
    assert_eq!(occ[1].end, When::parse("20250313T080000Z", None).unwrap());
}

#[test]
fn floating_follows_viewer() {
    let body = "
        BEGIN:VEVENT
        UID:float
        DTSTART:20250304T090000
        DTEND:20250304T100000
        RRULE:FREQ=DAILY;COUNT=2
        END:VEVENT
    ";
    let r = range("2025-03-04 00:00", "2025-03-04 12:00");
    assert_eq!(occurrences(body, r.clone(), Tz::UTC).len(), 1);
    // 09:00 in Los Angeles is 17:00 UTC, past the range
    assert!(occurrences(body, r, Tz::America__Los_Angeles).is_empty());
}

#[test]
fn dst_gap_in_series() {
    // 01:30 does not exist on 2025-03-30 in London; that instance moves to 02:00
    let body = "
        BEGIN:VEVENT
        UID:gap
        DTSTART;TZID=Europe/London:20250329T013000
        DURATION:PT30M
        RRULE:FREQ=DAILY;COUNT=3
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-03-28 00:00", "2025-04-02 00:00"), LONDON);
    assert_eq!(occ.len(), 3);
    assert_eq!(occ[0].start.to_utc(LONDON), utc("2025-03-29 01:30"));
    assert_eq!(occ[2].start.to_utc(LONDON), utc("2025-03-31 00:30"));
}

#[test]
fn todos() {
    let body = "
        BEGIN:VTODO
        UID:due-only
        DUE:20250310T170000Z
        SUMMARY:Report
        END:VTODO
        BEGIN:VTODO
        UID:span
        DTSTART:20250310T090000Z
        DUE:20250312T090000Z
        SUMMARY:Project
        END:VTODO
        BEGIN:VTODO
        UID:undated
        SUMMARY:Someday
        END:VTODO
    ";
    let occ = occurrences(body, range("2025-03-10 00:00", "2025-03-11 00:00"), Tz::UTC);
    assert_eq!(
        occ.iter()
            .map(|o| (o.summary.as_str(), o.kind))
            .collect::<Vec<_>>(),
        [("Project", Kind::Todo), ("Report", Kind::Todo)]
    );
}

#[test]
fn bad_rrule_falls_back_to_dtstart() {
    let body = "
        BEGIN:VEVENT
        UID:bad
        DTSTART:20250101T100000Z
        RRULE:FREQ=SOMETIMES
        END:VEVENT
    ";
    let occ = occurrences(body, range("2025-01-01 00:00", "2026-01-01 00:00"), Tz::UTC);
    assert_eq!(starts(&occ, Tz::UTC), ["2025-01-01 10:00"]);
}

use std::{fs, ops::Range};

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use chrono_tz::Tz;
use rical_core::{
    Kind, Occurrence, Vdir, When,
    write::{Draft, Repeat, Scope},
};

const SINGAPORE: Tz = Tz::Asia__Singapore;

fn ics(body: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\n{}END:VCALENDAR\r\n",
        body.trim_start()
            .lines()
            .map(|l| format!("{}\r\n", l.trim_start()))
            .collect::<String>()
    )
}

const LONDON_TZ: &str = "
    BEGIN:VTIMEZONE
    TZID:Europe/London
    BEGIN:STANDARD
    DTSTART:19701025T020000
    TZOFFSETFROM:+0100
    TZOFFSETTO:+0000
    RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU
    END:STANDARD
    BEGIN:DAYLIGHT
    DTSTART:19700329T010000
    TZOFFSETFROM:+0000
    TZOFFSETTO:+0100
    RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU
    END:DAYLIGHT
    END:VTIMEZONE
";

/// A temporary vdir with calendars "home" and "work".
fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Vdir) {
    let dir = tempfile::tempdir().unwrap();
    for cal in ["home", "work"] {
        fs::create_dir(dir.path().join(cal)).unwrap();
    }
    for (path, body) in files {
        fs::write(dir.path().join(path), ics(body)).unwrap();
    }
    let vdir = Vdir::load(dir.path()).unwrap();
    (dir, vdir)
}

fn utc(s: &str) -> DateTime<Utc> {
    NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M")
        .unwrap()
        .and_utc()
}

fn range(from: &str, to: &str) -> Range<DateTime<Utc>> {
    utc(from)..utc(to)
}

fn query(vdir: &Vdir, r: Range<DateTime<Utc>>) -> Vec<Occurrence> {
    vdir.query(&r, &[Kind::Event, Kind::Todo], SINGAPORE)
}

fn when(s: &str) -> When {
    When::parse(s, None).unwrap()
}

/// Lines in `after` that are not in `before`, ignoring stamps.
fn changed_lines(before: &str, after: &str) -> Vec<String> {
    let old = before.lines().collect::<Vec<_>>();
    after
        .lines()
        .filter(|l| !old.contains(l))
        .filter(|l| !l.starts_with("DTSTAMP") && !l.starts_with("LAST-MODIFIED"))
        .map(str::to_owned)
        .collect()
}

const MEETING: &str = "
    BEGIN:VEVENT
    UID:meeting
    DTSTAMP:20250101T000000Z
    DTSTART;TZID=Europe/London:20250304T090000
    DTEND;TZID=Europe/London:20250304T100000
    SUMMARY:Meeting
    X-UNKNOWN;X-P=1:keep me
    ATTENDEE;CN=\"Doe, J\":mailto:j@example.com
    BEGIN:VALARM
    ACTION:DISPLAY
    TRIGGER:-PT15M
    END:VALARM
    END:VEVENT
";

const STANDUP: &str = "
    BEGIN:VEVENT
    UID:standup
    DTSTAMP:20250101T000000Z
    DTSTART:20250303T090000Z
    DURATION:PT15M
    RRULE:FREQ=DAILY;COUNT=5
    SUMMARY:Standup
    END:VEVENT
";

#[test]
fn move_keeps_form_and_unknown_properties() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", &format!("{LONDON_TZ}{MEETING}"))]);
    let path = dir.path().join("work/meeting.ics");
    let before = fs::read_to_string(&path).unwrap();

    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    let mut draft = vdir.draft(occ);
    // the viewer in Singapore drags it an hour later
    draft.start = When::Zoned(occ.start.to_tz(SINGAPORE) + TimeDelta::hours(1));
    draft.end = When::Zoned(occ.end.to_tz(SINGAPORE) + TimeDelta::hours(1));
    vdir.update(occ, Scope::All, &draft, None).unwrap();

    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        changed_lines(&before, &after),
        [
            "DTSTART;TZID=Europe/London:20250304T100000",
            "DTEND;TZID=Europe/London:20250304T110000",
            "SEQUENCE:1",
        ]
    );
    // nothing else was lost
    for line in before.lines().filter(|l| {
        !l.starts_with("DTSTART") && !l.starts_with("DTEND") && !l.starts_with("DTSTAMP")
    }) {
        assert!(after.contains(line), "lost {line:?}");
    }

    let moved = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    assert_eq!(moved.start.to_utc(SINGAPORE), utc("2025-03-04 10:00"));
}

#[test]
fn edit_text_fields() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", MEETING)]);
    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    let mut draft = vdir.draft(occ);
    draft.summary = "Planning; with notes, and more".into();
    draft.location = "Room 2".into();
    draft.description = "Line one\nLine two".into();
    vdir.update(occ, Scope::All, &draft, None).unwrap();

    let text = fs::read_to_string(dir.path().join("work/meeting.ics")).unwrap();
    assert!(text.contains("SUMMARY:Planning\\; with notes\\, and more\r\n"));
    assert!(text.contains("DESCRIPTION:Line one\\nLine two\r\n"));
    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    assert_eq!(occ.summary, "Planning; with notes, and more");
    assert_eq!(occ.location.as_deref(), Some("Room 2"));
    assert_eq!(occ.description.as_deref(), Some("Line one\nLine two"));

    // clearing a field removes it
    let mut draft = vdir.draft(occ);
    draft.location.clear();
    vdir.update(occ, Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("work/meeting.ics")).unwrap();
    assert!(!text.contains("LOCATION"));
}

#[test]
fn create_copies_timezone_and_round_trips() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", &format!("{LONDON_TZ}{MEETING}"))]);
    let start = When::parse("20250310T140000", Some("Europe/London")).unwrap();
    let mut draft = Draft::new(start, start.add(TimeDelta::minutes(45)));
    draft.summary = "New thing".into();
    draft.repeat = Repeat::Weekly;
    let change = vdir.create("home", &draft).unwrap();

    assert_eq!(change.path.parent().unwrap(), dir.path().join("home"));
    let name = change
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let text = fs::read_to_string(&change.path).unwrap();
    let uid = text
        .lines()
        .find_map(|l| l.strip_prefix("UID:"))
        .unwrap()
        .trim_end();
    assert_eq!(name, format!("{uid}.ics"), "vdirsyncer-style file name");
    assert!(text.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/London\r\n"));
    assert!(text.contains("DTSTART;TZID=Europe/London:20250310T140000\r\n"));
    assert!(text.contains("RRULE:FREQ=WEEKLY\r\n"));

    let occ = query(&vdir, range("2025-03-10 00:00", "2025-03-20 00:00"));
    assert_eq!(occ.len(), 2);
    assert!(occ.iter().all(|o| o.summary == "New thing" && o.recurring));
    assert_eq!(occ[0].calendar, "home");
    assert_eq!(occ[0].duration(), TimeDelta::minutes(45));
}

#[test]
fn create_all_day_todo() {
    let (_dir, mut vdir) = setup(&[]);
    let mut draft = Draft::new(when("20250310"), when("20250311"));
    draft.kind = Kind::Todo;
    draft.summary = "Pay rent".into();
    let change = vdir.create("home", &draft).unwrap();
    let text = change.after.unwrap();
    assert!(text.contains("BEGIN:VTODO"));
    assert!(text.contains("DUE;VALUE=DATE:20250311"));
    let occ = query(&vdir, range("2025-03-10 00:00", "2025-03-11 00:00"));
    assert_eq!(occ[0].kind, Kind::Todo);
}

#[test]
fn this_occurrence_only_adds_an_override() {
    let (dir, mut vdir) = setup(&[("work/standup.ics", STANDUP)]);
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    assert_eq!(all.len(), 5);

    let wed = &all[2];
    let mut draft = vdir.draft(wed);
    draft.summary = "Standup (late)".into();
    draft.start = wed.start.add(TimeDelta::hours(2));
    draft.end = wed.end.add(TimeDelta::hours(2));
    vdir.update(wed, Scope::This, &draft, None).unwrap();

    let text = fs::read_to_string(dir.path().join("work/standup.ics")).unwrap();
    assert_eq!(text.matches("BEGIN:VEVENT").count(), 2);
    assert!(text.contains("RECURRENCE-ID:20250305T090000Z\r\n"));

    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    let got = all
        .iter()
        .map(|o| (o.start.to_string(), o.summary.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        got,
        [
            ("2025-03-03 09:00 UTC".into(), "Standup"),
            ("2025-03-04 09:00 UTC".into(), "Standup"),
            ("2025-03-05 11:00 UTC".into(), "Standup (late)"),
            ("2025-03-06 09:00 UTC".into(), "Standup"),
            ("2025-03-07 09:00 UTC".into(), "Standup"),
        ]
    );

    // editing the override again changes it in place
    let late = &all[2];
    let mut draft = vdir.draft(late);
    draft.summary = "Standup (later)".into();
    vdir.update(late, Scope::This, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("work/standup.ics")).unwrap();
    assert_eq!(text.matches("BEGIN:VEVENT").count(), 2);
    assert!(text.contains("Standup (later)"));
}

#[test]
fn whole_series_moves_by_the_same_amount() {
    let (dir, mut vdir) = setup(&[("work/standup.ics", STANDUP)]);
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    // drag Wednesday's instance 30 minutes later and make it longer
    let wed = &all[2];
    let mut draft = vdir.draft(wed);
    draft.start = wed.start.add(TimeDelta::minutes(30));
    draft.end = wed.end.add(TimeDelta::minutes(45));
    vdir.update(wed, Scope::All, &draft, None).unwrap();

    let text = fs::read_to_string(dir.path().join("work/standup.ics")).unwrap();
    assert!(text.contains("DTSTART:20250303T093000Z\r\n"), "{text}");
    assert!(
        text.contains("DURATION:PT30M\r\n"),
        "DURATION kept as DURATION"
    );
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    assert!(all.iter().all(|o| o.duration() == TimeDelta::minutes(30)));
    assert_eq!(all[0].start.to_string(), "2025-03-03 09:30 UTC");
}

#[test]
fn stop_repeating() {
    let (dir, mut vdir) = setup(&[("work/standup.ics", STANDUP)]);
    let first = &query(&vdir, range("2025-03-03 00:00", "2025-03-04 00:00"))[0];
    let mut draft = vdir.draft(first);
    assert_eq!(draft.repeat, Repeat::Custom("FREQ=DAILY;COUNT=5".into()));
    draft.repeat = Repeat::None;
    vdir.update(first, Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("work/standup.ics")).unwrap();
    assert!(!text.contains("RRULE"));
    assert_eq!(
        query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00")).len(),
        1
    );
}

#[test]
fn delete_one_instance_then_series() {
    let (dir, mut vdir) = setup(&[("work/standup.ics", STANDUP)]);
    let path = dir.path().join("work/standup.ics");
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    vdir.delete(&all[1], Scope::This).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("EXDATE:20250304T090000Z\r\n"));
    assert_eq!(
        query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00")).len(),
        4
    );

    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    vdir.delete(&all[0], Scope::All).unwrap();
    assert!(!path.exists());
    assert!(query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00")).is_empty());
}

#[test]
fn delete_moved_instance_removes_override() {
    let (dir, mut vdir) = setup(&[("work/standup.ics", STANDUP)]);
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    let mut draft = vdir.draft(&all[2]);
    draft.summary = "Moved".into();
    vdir.update(&all[2], Scope::This, &draft, None).unwrap();

    let moved = query(&vdir, range("2025-03-05 00:00", "2025-03-06 00:00"));
    assert_eq!(moved[0].summary, "Moved");
    vdir.delete(&moved[0], Scope::This).unwrap();
    let text = fs::read_to_string(dir.path().join("work/standup.ics")).unwrap();
    assert_eq!(text.matches("BEGIN:VEVENT").count(), 1);
    assert!(text.contains("EXDATE:20250305T090000Z"));
    assert_eq!(
        query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00")).len(),
        4
    );
}

#[test]
fn undo_restores_exact_bytes() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", MEETING)]);
    let path = dir.path().join("work/meeting.ics");
    let original = fs::read_to_string(&path).unwrap();

    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    let change = vdir.delete(occ, Scope::All).unwrap();
    assert!(!path.exists());
    let redo = vdir.undo(&change).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(
        query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00")).len(),
        1
    );

    vdir.undo(&redo).unwrap();
    assert!(!path.exists());
}

#[test]
fn refuses_to_overwrite_external_changes() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", MEETING)]);
    let path = dir.path().join("work/meeting.ics");
    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    let change = vdir.delete(occ, Scope::All).unwrap();

    // vdirsyncer brings the file back with other content
    fs::write(&path, ics(MEETING).replace("Meeting", "Synced")).unwrap();
    assert!(vdir.undo(&change).is_err());
    assert!(fs::read_to_string(&path).unwrap().contains("Synced"));
}

#[test]
fn move_to_other_calendar() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", MEETING)]);
    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    let draft = vdir.draft(occ);
    let changes = vdir.update(occ, Scope::All, &draft, Some("home")).unwrap();
    assert_eq!(changes.len(), 2);
    assert!(!dir.path().join("work/meeting.ics").exists());
    assert!(dir.path().join("home/meeting.ics").exists());
    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    assert_eq!(occ.calendar, "home");
}

#[test]
fn toggle_all_day() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", MEETING)]);
    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    let mut draft = vdir.draft(occ);
    draft.start = when("20250304");
    draft.end = when("20250305");
    vdir.update(occ, Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("work/meeting.ics")).unwrap();
    assert!(text.contains("DTSTART;VALUE=DATE:20250304\r\n"));
    assert!(text.contains("DTEND;VALUE=DATE:20250305\r\n"));
    let occ = &query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00"))[0];
    assert!(occ.all_day);
}

#[test]
fn untouched_files_are_not_rewritten() {
    let (dir, mut vdir) = setup(&[("work/meeting.ics", MEETING), ("work/standup.ics", STANDUP)]);
    let other = dir.path().join("work/meeting.ics");
    let mtime = fs::metadata(&other).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));

    let first = &query(&vdir, range("2025-03-03 00:00", "2025-03-04 00:00"))[0];
    vdir.delete(first, Scope::This).unwrap();
    assert_eq!(fs::metadata(&other).unwrap().modified().unwrap(), mtime);
    // no temporary files left behind
    let leftovers = fs::read_dir(dir.path().join("work"))
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn timezone_from_lf_file_is_rewritten_with_crlf() {
    let (dir, mut vdir) = setup(&[]);
    // another client wrote this file with bare LF line endings
    fs::write(
        dir.path().join("work/lf.ics"),
        ics(&format!("{LONDON_TZ}{MEETING}")).replace("\r\n", "\n"),
    )
    .unwrap();
    vdir.reload_all().unwrap();

    let start = When::parse("20250310T140000", Some("Europe/London")).unwrap();
    let change = vdir
        .create("home", &Draft::new(start, start.add(TimeDelta::hours(1))))
        .unwrap();
    let text = change.after.unwrap();
    assert!(text.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/London\r\n"));
    assert!(
        !text.replace("\r\n", "").contains('\n'),
        "mixed line endings"
    );
    // and the result parses
    assert_eq!(
        query(&vdir, range("2025-03-10 00:00", "2025-03-11 00:00")).len(),
        1
    );
}

use std::{fs, ops::Range};

use chrono::{DateTime, NaiveDateTime, TimeDelta, Utc};
use chrono_tz::Tz;
use rical_core::{
    Kind, Occurrence, Vdir, When,
    rule::{End, Freq},
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
    draft.repeat = Repeat::every(Freq::Weekly);
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
    // like an event, the end is exclusive: 10th to 11th is due on the 10th
    let mut draft = Draft::new(when("20250310"), when("20250311"));
    draft.kind = Kind::Todo;
    draft.summary = "Pay rent".into();
    draft.has_start = false;
    let change = vdir.create("home", &draft).unwrap();
    let text = change.after.unwrap();
    assert!(text.contains("BEGIN:VTODO"));
    assert!(text.contains("DUE;VALUE=DATE:20250310"), "{text}");
    assert!(!text.contains("DTSTART"), "only a due date: {text}");
    let occ = query(&vdir, range("2025-03-10 00:00", "2025-03-11 00:00"));
    assert_eq!(occ[0].kind, Kind::Todo);
}

const TRIP_TASK: &str = "
    BEGIN:VTODO
    UID:pack
    DTSTAMP:20261001T000000Z
    DTSTART;VALUE=DATE:20261215
    DUE;VALUE=DATE:20261218
    SUMMARY:Pack for the UK
    PRIORITY:5
    END:VTODO
";

#[test]
fn a_task_covers_its_due_day() {
    let (_dir, vdir) = setup(&[("home/pack.ics", TRIP_TASK)]);
    // started on the 15th, due on the 18th: there on both, and between
    for day in ["2026-12-15", "2026-12-18"] {
        let from = format!("{day} 00:00");
        let to = format!("{day} 23:59");
        let occ = query(&vdir, range(&from, &to));
        assert_eq!(occ.len(), 1, "on {day}");
    }
    let occ = query(&vdir, range("2026-12-19 00:00", "2026-12-20 00:00"));
    assert!(occ.is_empty(), "not after it is due");
}

#[test]
fn task_dates_are_kept_as_they_are() {
    let (dir, mut vdir) = setup(&[("home/pack.ics", TRIP_TASK)]);
    let path = dir.path().join("home/pack.ics");
    let occ = query(&vdir, range("2026-12-15 00:00", "2026-12-16 00:00"));
    let draft = vdir.draft(&occ[0]);
    assert!(draft.has_start && draft.has_due);
    assert_eq!(draft.priority, Some(5));
    assert_eq!(draft.done, Some(false));

    // moved two days: both dates move, the due date stays a due date
    let mut moved = draft.clone();
    moved.start = draft.start.add(TimeDelta::days(2));
    moved.end = draft.end.add(TimeDelta::days(2));
    vdir.update(&occ[0], Scope::All, &moved, None).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("DTSTART;VALUE=DATE:20261217\r\n"), "{text}");
    assert!(text.contains("DUE;VALUE=DATE:20261220\r\n"), "{text}");

    // no start any more: only DUE; then no due either: undated
    let occ = query(&vdir, range("2026-12-17 00:00", "2026-12-18 00:00"));
    let mut d = vdir.draft(&occ[0]);
    d.has_start = false;
    d.priority = None;
    vdir.update(&occ[0], Scope::All, &d, None).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("DTSTART"), "{text}");
    assert!(text.contains("DUE;VALUE=DATE:20261220\r\n"));
    assert!(!text.contains("PRIORITY"));
    let occ = query(&vdir, range("2026-12-20 00:00", "2026-12-21 00:00"));
    let mut d = vdir.draft(&occ[0]);
    d.has_due = false;
    vdir.update(&occ[0], Scope::All, &d, None).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("DUE") && !text.contains("DTSTART"), "{text}");
    assert_eq!(vdir.tasks()[0].summary, "Pack for the UK");
}

#[test]
fn a_due_time_task_keeps_just_its_due_time() {
    let task = "
        BEGIN:VTODO
        UID:call
        DTSTAMP:20261001T000000Z
        DUE;TZID=Europe/London:20261221T170000
        SUMMARY:Call the bank before it shuts
        END:VTODO
    ";
    let (dir, mut vdir) = setup(&[("home/call.ics", &format!("{LONDON_TZ}{task}"))]);
    let occ = query(&vdir, range("2026-12-21 00:00", "2026-12-22 00:00"));
    let mut d = vdir.draft(&occ[0]);
    assert!(!d.has_start && d.has_due);
    d.start = d.start.add(TimeDelta::hours(1));
    d.end = d.end.add(TimeDelta::hours(1));
    vdir.update(&occ[0], Scope::All, &d, None).unwrap();
    let text = fs::read_to_string(dir.path().join("home/call.ics")).unwrap();
    assert!(
        text.contains("DUE;TZID=Europe/London:20261221T180000\r\n"),
        "{text}"
    );
    assert!(!text.contains("DTSTART;TZID"), "{text}");
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
    let Repeat::Rule(rule) = &draft.repeat else {
        panic!("{:?}", draft.repeat)
    };
    assert_eq!(rule.end, End::Count(5));
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

const TASKS: &str = "
    BEGIN:VTODO
    UID:report
    DTSTAMP:20250101T000000Z
    DUE;VALUE=DATE:20250303
    SUMMARY:Write report
    STATUS:NEEDS-ACTION
    PRIORITY:1
    END:VTODO
";

#[test]
fn tasks_list_and_complete() {
    let (dir, mut vdir) = setup(&[("work/report.ics", TASKS)]);
    let path = dir.path().join("work/report.ics");
    let tasks = vdir.tasks();
    assert_eq!(tasks.len(), 1);
    let t = &tasks[0];
    assert_eq!((t.summary.as_str(), t.priority), ("Write report", Some(1)));
    assert!(!t.is_done());
    let today = chrono::NaiveDate::from_ymd_opt(2025, 3, 5).unwrap();
    assert!(t.is_overdue(today, SINGAPORE));
    assert!(!t.is_overdue(
        chrono::NaiveDate::from_ymd_opt(2025, 3, 3).unwrap(),
        SINGAPORE
    ));

    // carried to today as an all-day item that remembers when it was due
    let carried = t.on_day(today);
    assert!(carried.all_day);
    assert_eq!(carried.due, t.due);

    let before = fs::read_to_string(&path).unwrap();
    let change = vdir.set_done(t, true).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("STATUS:COMPLETED\r\n"));
    assert!(text.contains("PERCENT-COMPLETE:100\r\n"));
    assert!(text.contains("\r\nCOMPLETED:"));
    let t = &vdir.tasks()[0];
    assert!(t.is_done() && !t.is_overdue(today, SINGAPORE));

    vdir.set_done(t, false).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("STATUS:NEEDS-ACTION\r\n"));
    assert!(!text.contains("COMPLETED:") && !text.contains("PERCENT-COMPLETE"));

    // undo of the first change is refused now the file moved on
    assert!(vdir.undo(&change).is_err());
    let _ = before;
}

#[test]
fn create_and_date_tasks() {
    let (_dir, mut vdir) = setup(&[]);
    let change = vdir.create_task("home", "Buy milk", None, None).unwrap();
    let text = change.after.unwrap();
    assert!(text.contains("BEGIN:VTODO") && text.contains("SUMMARY:Buy milk"));
    assert!(!text.contains("DUE") && !text.contains("DTSTART"));
    let t = vdir.tasks().pop().unwrap();
    assert_eq!(t.due, None);

    vdir.set_due(&t, Some(when("20250310"))).unwrap();
    let t = vdir.tasks().pop().unwrap();
    assert_eq!(t.due, Some(when("20250310")));
    // it now shows on that day
    let occ = query(&vdir, range("2025-03-09 16:00", "2025-03-10 16:00"));
    assert_eq!(occ.len(), 1);
    assert_eq!(occ[0].due, Some(when("20250310")));

    // editing its title through a draft keeps it due-only
    let mut draft = vdir.draft(&occ[0]);
    draft.summary = "Buy oat milk".into();
    vdir.update(&occ[0], Scope::All, &draft, None).unwrap();
    let t = vdir.tasks().pop().unwrap();
    assert_eq!((t.summary.as_str(), t.start), ("Buy oat milk", None));
}

#[test]
fn alarms() {
    let (_dir, vdir) = setup(&[
        (
            "work/meeting.ics",
            "
            BEGIN:VEVENT
            UID:meeting
            DTSTAMP:20250101T000000Z
            DTSTART:20250304T090000Z
            DTEND:20250304T100000Z
            SUMMARY:Meeting
            BEGIN:VALARM
            ACTION:DISPLAY
            TRIGGER:-PT30M
            END:VALARM
            BEGIN:VALARM
            ACTION:EMAIL
            TRIGGER:-PT1H
            END:VALARM
            BEGIN:VALARM
            ACTION:AUDIO
            TRIGGER;RELATED=END:PT0S
            END:VALARM
            END:VEVENT
            ",
        ),
        (
            "work/weekly.ics",
            "
            BEGIN:VEVENT
            UID:weekly
            DTSTAMP:20250101T000000Z
            DTSTART:20250303T120000Z
            DURATION:PT1H
            RRULE:FREQ=WEEKLY
            SUMMARY:Weekly
            BEGIN:VALARM
            ACTION:DISPLAY
            TRIGGER;VALUE=DATE-TIME:20250303T110000Z
            END:VALARM
            BEGIN:VALARM
            ACTION:DISPLAY
            TRIGGER:-PT10M
            END:VALARM
            END:VEVENT
            ",
        ),
        (
            "home/bday.ics",
            "
            BEGIN:VEVENT
            UID:bday
            DTSTAMP:20250101T000000Z
            DTSTART;VALUE=DATE:20250305
            SUMMARY:Birthday
            BEGIN:VALARM
            ACTION:DISPLAY
            TRIGGER:PT9H
            END:VALARM
            END:VEVENT
            ",
        ),
        (
            "home/plain.ics",
            "
            BEGIN:VEVENT
            UID:plain
            DTSTAMP:20250101T000000Z
            DTSTART:20250304T150000Z
            DTEND:20250304T160000Z
            SUMMARY:No alarm
            END:VEVENT
            BEGIN:VEVENT
            UID:off
            DTSTAMP:20250101T000000Z
            DTSTART:20250304T170000Z
            SUMMARY:Cancelled
            STATUS:CANCELLED
            BEGIN:VALARM
            ACTION:DISPLAY
            TRIGGER:-PT5M
            END:VALARM
            END:VEVENT
            ",
        ),
    ]);
    let show = |alarms: Vec<rical_core::alarm::Alarm>| {
        alarms
            .iter()
            .map(|a| format!("{} {}", a.at.format("%d %H:%M"), a.occurrence.summary))
            .collect::<Vec<_>>()
    };
    let london = Tz::Europe__London;

    // the 4th: 30 minutes before the meeting, and at its end (not email);
    // the absolute alarm only belongs to the first weekly occurrence
    assert_eq!(
        show(vdir.alarms(&range("2025-03-03 00:00", "2025-03-05 00:00"), london, None)),
        [
            "03 11:00 Weekly",
            "03 11:50 Weekly",
            "04 08:30 Meeting",
            "04 10:00 Meeting",
        ]
    );
    // later weeks repeat the relative alarm; the all-day birthday reminds at
    // 09:00 local time on the day
    assert_eq!(
        show(vdir.alarms(&range("2025-03-05 00:00", "2025-03-11 00:00"), london, None)),
        ["05 09:00 Birthday", "10 11:50 Weekly"]
    );
    assert_eq!(
        show(vdir.alarms(
            &range("2025-03-05 00:00", "2025-03-06 00:00"),
            SINGAPORE,
            None
        )),
        ["05 01:00 Birthday"],
        "09:00 in Singapore is 01:00 UTC"
    );
    // a default reminder for timed events without alarms; cancelled skipped
    assert_eq!(
        show(vdir.alarms(
            &range("2025-03-04 12:00", "2025-03-04 23:00"),
            london,
            Some(TimeDelta::minutes(15))
        )),
        ["04 14:45 No alarm"]
    );
}

#[test]
fn completing_a_repeating_task_moves_it_on() {
    let (dir, mut vdir) = setup(&[
        (
            "home/bins.ics",
            "
            BEGIN:VTODO
            UID:bins
            DTSTAMP:20250101T000000Z
            DTSTART;TZID=Europe/London:20250303T190000
            DUE;TZID=Europe/London:20250303T200000
            RRULE:FREQ=WEEKLY;COUNT=3
            SUMMARY:Put the bins out
            STATUS:NEEDS-ACTION
            END:VTODO
            ",
        ),
        (
            "home/plants.ics",
            "
            BEGIN:VTODO
            UID:plants
            DTSTAMP:20250101T000000Z
            DUE;VALUE=DATE:20250310
            RRULE:FREQ=DAILY
            SUMMARY:Water plants
            END:VTODO
            ",
        ),
    ]);
    let find = |vdir: &Vdir, uid: &str| vdir.tasks().into_iter().find(|t| t.uid == uid).unwrap();

    let bins = find(&vdir, "bins");
    assert!(bins.recurring);
    vdir.set_done(&bins, true).unwrap();
    let text = fs::read_to_string(dir.path().join("home/bins.ics")).unwrap();
    assert!(
        text.contains("DTSTART;TZID=Europe/London:20250310T190000\r\n"),
        "{text}"
    );
    assert!(text.contains("DUE;TZID=Europe/London:20250310T200000\r\n"));
    assert!(text.contains("RRULE:FREQ=WEEKLY;COUNT=2\r\n"));
    assert!(!find(&vdir, "bins").is_done());

    // and again, keeping its local time
    vdir.set_done(&find(&vdir, "bins"), true).unwrap();
    let text = fs::read_to_string(dir.path().join("home/bins.ics")).unwrap();
    assert!(text.contains("DTSTART;TZID=Europe/London:20250317T190000\r\n"));
    // the last one is really done
    vdir.set_done(&find(&vdir, "bins"), true).unwrap();
    let bins = find(&vdir, "bins");
    assert!(bins.is_done());
    assert_eq!(
        bins.due,
        When::parse("20250317T200000", Some("Europe/London"))
    );

    // a due-only daily task moves a day
    vdir.set_done(&find(&vdir, "plants"), true).unwrap();
    assert_eq!(find(&vdir, "plants").due, Some(when("20250311")));
}

#[test]
fn editing_repeat_rules() {
    let (dir, mut vdir) = setup(&[(
        "work/class.ics",
        "
        BEGIN:VEVENT
        UID:class
        DTSTAMP:20250101T000000Z
        DTSTART;TZID=Europe/London:20250303T180000
        DURATION:PT1H
        RRULE:FREQ=WEEKLY;BYDAY=WE,MO
        SUMMARY:Class
        END:VEVENT
        ",
    )]);
    let path = dir.path().join("work/class.ics");
    let occ = &query(&vdir, range("2025-03-03 00:00", "2025-03-04 00:00"))[0];

    // editing something else leaves the rule exactly as written
    let mut draft = vdir.draft(occ);
    draft.summary = "Pottery class".into();
    vdir.update(occ, Scope::All, &draft, None).unwrap();
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("RRULE:FREQ=WEEKLY;BYDAY=WE,MO\r\n")
    );

    // every two weeks on Mondays until the end of March
    let occ = &query(&vdir, range("2025-03-03 00:00", "2025-03-04 00:00"))[0];
    let mut draft = vdir.draft(occ);
    let Repeat::Rule(mut rule) = draft.repeat.clone() else {
        panic!()
    };
    rule.interval = 2;
    rule.weekdays = vec![chrono::Weekday::Mon];
    rule.end = End::Until(chrono::NaiveDate::from_ymd_opt(2025, 3, 31).unwrap());
    draft.repeat = Repeat::Rule(rule);
    vdir.update(occ, Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    // 23:59:59 on 31 March in London is BST, 22:59:59 UTC
    assert!(
        text.contains("RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO;UNTIL=20250331T225959Z\r\n"),
        "{text}"
    );
    let starts = query(&vdir, range("2025-03-01 00:00", "2025-05-01 00:00"))
        .iter()
        .map(|o| o.start.date(SINGAPORE).to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        starts,
        ["2025-03-04", "2025-03-18", "2025-04-01"],
        "Singapore dates of 18:00 London"
    );
}

/// An event at 09:00–10:00 Singapore time on 2026-12-14, weekly or not.
fn singapore_event(repeat: bool) -> String {
    format!(
        "
        BEGIN:VEVENT
        UID:catch-up
        DTSTAMP:20261001T000000Z
        DTSTART;TZID=Asia/Singapore:20261214T090000
        DTEND;TZID=Asia/Singapore:20261214T100000
        {}SUMMARY:Catch-up
        END:VEVENT
        ",
        if repeat { "RRULE:FREQ=WEEKLY\n" } else { "" }
    )
}

fn zoned(s: &str, tz: &str) -> When {
    When::parse(s, Some(tz)).unwrap()
}

#[test]
fn editor_zones_are_written_as_chosen() {
    // planning a UK trip from Singapore: 09:00 London time
    let (_dir, mut vdir) = setup(&[("work/meeting.ics", &format!("{LONDON_TZ}{MEETING}"))]);
    let mut draft = Draft::new(
        zoned("20261221T090000", "Europe/London"),
        zoned("20261221T100000", "Europe/London"),
    );
    draft.summary = "Coffee in London".into();
    draft.exact_zones = true;
    let change = vdir.create("home", &draft).unwrap();
    let text = fs::read_to_string(&change.path).unwrap();
    assert!(text.contains("DTSTART;TZID=Europe/London:20261221T090000\r\n"));
    assert!(text.contains("DTEND;TZID=Europe/London:20261221T100000\r\n"));
    // its VTIMEZONE comes along from the other file
    assert!(text.contains("TZID:Europe/London\r\n"));
    // seen from Singapore it is 17:00 (London is on GMT in December)
    let occ = query(&vdir, range("2026-12-21 00:00", "2026-12-22 00:00"));
    let local = occ[0].start.to_tz(SINGAPORE);
    assert_eq!(local.format("%H:%M").to_string(), "17:00");
}

#[test]
fn changing_an_events_zone_keeps_its_wall_time() {
    let (dir, mut vdir) = setup(&[("home/catch-up.ics", &singapore_event(false))]);
    let occ = query(&vdir, range("2026-12-14 00:00", "2026-12-15 00:00"));
    let mut draft = vdir.draft(&occ[0]);
    draft.start = zoned("20261214T090000", "Europe/London");
    draft.end = zoned("20261214T100000", "Europe/London");
    draft.exact_zones = true;
    vdir.update(&occ[0], Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("home/catch-up.ics")).unwrap();
    assert!(
        text.contains("DTSTART;TZID=Europe/London:20261214T090000\r\n"),
        "{text}"
    );
    assert!(text.contains("DTEND;TZID=Europe/London:20261214T100000\r\n"));
}

#[test]
fn a_flight_ends_in_another_zone() {
    // leaves Singapore 23:00, lands in London 05:30 the next morning
    let (dir, mut vdir) = setup(&[("home/catch-up.ics", &singapore_event(false))]);
    let occ = query(&vdir, range("2026-12-14 00:00", "2026-12-15 00:00"));
    let mut draft = vdir.draft(&occ[0]);
    draft.start = zoned("20261219T230000", "Asia/Singapore");
    draft.end = zoned("20261220T053000", "Europe/London");
    draft.exact_zones = true;
    vdir.update(&occ[0], Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("home/catch-up.ics")).unwrap();
    assert!(text.contains("DTSTART;TZID=Asia/Singapore:20261219T230000\r\n"));
    assert!(
        text.contains("DTEND;TZID=Europe/London:20261220T053000\r\n"),
        "{text}"
    );
    let occ = query(&vdir, range("2026-12-19 00:00", "2026-12-21 00:00"));
    assert_eq!(occ[0].duration(), TimeDelta::minutes(14 * 60 + 30));
}

#[test]
fn a_series_moved_to_another_zone_keeps_its_wall_times() {
    let (dir, mut vdir) = setup(&[("home/catch-up.ics", &singapore_event(true))]);
    // edit the third week's occurrence, moving the series to London
    let occ = query(&vdir, range("2026-12-28 00:00", "2026-12-29 00:00"));
    assert_eq!(occ.len(), 1);
    let mut draft = vdir.draft(&occ[0]);
    draft.start = zoned("20261228T090000", "Europe/London");
    draft.end = zoned("20261228T100000", "Europe/London");
    draft.exact_zones = true;
    vdir.update(&occ[0], Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("home/catch-up.ics")).unwrap();
    // the series starts where it did, at 09:00 London time
    assert!(
        text.contains("DTSTART;TZID=Europe/London:20261214T090000\r\n"),
        "{text}"
    );
    let occ = query(&vdir, range("2026-12-28 00:00", "2026-12-29 00:00"));
    assert_eq!(
        occ[0].start.to_tz(SINGAPORE).format("%H:%M").to_string(),
        "17:00"
    );
}

#[test]
fn a_drag_keeps_the_events_zone() {
    // moves (no exact zones) still convert into the event's own zone
    let (dir, mut vdir) = setup(&[("home/catch-up.ics", &singapore_event(false))]);
    let occ = query(&vdir, range("2026-12-14 00:00", "2026-12-15 00:00"));
    let mut draft = vdir.draft(&occ[0]);
    draft.start = zoned("20261214T020000", "UTC");
    draft.end = zoned("20261214T030000", "UTC");
    vdir.update(&occ[0], Scope::All, &draft, None).unwrap();
    let text = fs::read_to_string(dir.path().join("home/catch-up.ics")).unwrap();
    assert!(
        text.contains("DTSTART;TZID=Asia/Singapore:20261214T100000\r\n"),
        "{text}"
    );
}

#[test]
fn subtasks_link_to_their_parent() {
    let (_dir, mut vdir) = setup(&[("home/pack.ics", TRIP_TASK)]);
    let change = vdir
        .create_task("home", "Book the taxi", None, Some("pack"))
        .unwrap();
    let text = change.after.unwrap();
    assert!(
        text.contains("RELATED-TO;RELTYPE=PARENT:pack\r\n"),
        "{text}"
    );
    let tasks = vdir.tasks();
    let taxi = tasks.iter().find(|t| t.summary == "Book the taxi").unwrap();
    assert_eq!(taxi.parent.as_deref(), Some("pack"));
    let pack = tasks.iter().find(|t| t.uid == "pack").unwrap();
    assert_eq!(pack.parent, None);
}

#[test]
fn related_to_without_reltype_is_a_parent() {
    // RFC 5545: RELTYPE defaults to PARENT; siblings and children are not
    let child = "
        BEGIN:VTODO
        UID:child
        DTSTAMP:20261001T000000Z
        SUMMARY:Child
        RELATED-TO:pack
        RELATED-TO;RELTYPE=SIBLING:other
        END:VTODO
    ";
    let sibling = "
        BEGIN:VTODO
        UID:sib
        DTSTAMP:20261001T000000Z
        SUMMARY:Sibling only
        RELATED-TO;RELTYPE=SIBLING:pack
        END:VTODO
    ";
    let (_dir, vdir) = setup(&[
        ("home/pack.ics", TRIP_TASK),
        ("home/child.ics", child),
        ("home/sib.ics", sibling),
    ]);
    let tasks = vdir.tasks();
    let parent_of = |uid: &str| tasks.iter().find(|t| t.uid == uid).unwrap().parent.clone();
    assert_eq!(parent_of("child").as_deref(), Some("pack"));
    assert_eq!(parent_of("sib"), None);
}

#[test]
fn changing_a_tasks_parent() {
    let (dir, mut vdir) = setup(&[("home/pack.ics", TRIP_TASK)]);
    let taxi = vdir
        .create_task("home", "Book the taxi", None, Some("pack"))
        .unwrap()
        .path;
    let other = vdir
        .create_task("home", "Trip admin", None, None)
        .unwrap()
        .path;
    let admin_uid = vdir
        .tasks()
        .into_iter()
        .find(|t| t.path == other)
        .unwrap()
        .uid;
    let taxi_task = vdir.tasks().into_iter().find(|t| t.path == taxi).unwrap();
    // a loop is refused before it is written: pack is not within taxi
    assert!(vdir.is_within("home", &taxi_task.uid, "pack"));
    assert!(!vdir.is_within("home", "pack", &taxi_task.uid));

    // move the taxi under "Trip admin" through an edit draft
    let occ = rical_core::Occurrence {
        ..taxi_task.on_day(chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap())
    };
    let mut d = vdir.draft(&occ);
    assert_eq!(d.parent.as_deref(), Some("pack"));
    d.parent = Some(admin_uid.clone());
    d.has_start = false;
    d.has_due = false;
    vdir.update(&occ, Scope::All, &d, None).unwrap();
    let text = fs::read_to_string(&taxi).unwrap();
    assert!(
        text.contains(&format!("RELATED-TO;RELTYPE=PARENT:{admin_uid}\r\n")),
        "{text}"
    );
    assert_eq!(text.matches("RELATED-TO").count(), 1);

    // and make it a task of its own again
    let taxi_task = vdir.tasks().into_iter().find(|t| t.path == taxi).unwrap();
    let occ = taxi_task.on_day(chrono::NaiveDate::from_ymd_opt(2026, 12, 1).unwrap());
    let mut d = vdir.draft(&occ);
    d.parent = None;
    d.has_start = false;
    d.has_due = false;
    vdir.update(&occ, Scope::All, &d, None).unwrap();
    assert!(!fs::read_to_string(&taxi).unwrap().contains("RELATED-TO"));
    let _ = dir;
}

#[test]
fn replying_sets_my_answer_on_every_instance() {
    let invite = "
        BEGIN:VEVENT
        UID:sync
        DTSTAMP:20250101T000000Z
        SEQUENCE:2
        DTSTART:20250303T090000Z
        DURATION:PT30M
        RRULE:FREQ=DAILY;COUNT=3
        SUMMARY:Sync
        ORGANIZER;CN=Priya:mailto:priya@example.com
        ATTENDEE;CN=Me;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:Me@Example.com
        ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@example.com
        END:VEVENT
        BEGIN:VEVENT
        UID:sync
        DTSTAMP:20250101T000000Z
        RECURRENCE-ID:20250304T090000Z
        DTSTART:20250304T100000Z
        DURATION:PT30M
        SUMMARY:Sync (moved)
        ORGANIZER;CN=Priya:mailto:priya@example.com
        ATTENDEE;CN=Me:mailto:me@example.com
        END:VEVENT
    ";
    let (dir, mut vdir) = setup(&[("work/sync.ics", invite)]);
    let path = dir.path().join("work/sync.ics");
    let before = fs::read_to_string(&path).unwrap();
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    let me = ["me@example.com".to_owned()];
    let change = vdir.reply(&all[0], &me, "ACCEPTED").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert_eq!(
        changed_lines(&before, &after),
        [
            "ATTENDEE;CN=Me;PARTSTAT=ACCEPTED;RSVP=TRUE:mailto:Me@Example.com",
            "ATTENDEE;CN=Me;PARTSTAT=ACCEPTED:mailto:me@example.com",
        ]
    );
    // the organiser's SEQUENCE is theirs to change
    assert!(after.contains("SEQUENCE:2\r\n"));
    vdir.undo(&change).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), before);

    // not invited under that address
    assert!(
        vdir.reply(&all[0], &["other@example.com".to_owned()], "DECLINED")
            .is_err()
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn conference_links_are_read_and_written_per_rfc_7986() {
    let call = "
        BEGIN:VEVENT
        UID:call
        DTSTAMP:20250101T000000Z
        DTSTART:20250303T090000Z
        DURATION:PT30M
        SUMMARY:Call
        CONFERENCE;VALUE=URI;FEATURE=PHONE;LABEL=Dial-in:tel:+44-20-7946-0000
        CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Meet:https://meet.example.com/abc
        END:VEVENT
    ";
    let google = "
        BEGIN:VEVENT
        UID:google
        DTSTAMP:20250101T000000Z
        DTSTART:20250304T090000Z
        DURATION:PT30M
        SUMMARY:Sync
        X-GOOGLE-CONFERENCE:https://meet.google.com/xyz
        END:VEVENT
    ";
    let plain = "
        BEGIN:VEVENT
        UID:plain
        DTSTAMP:20250101T000000Z
        DTSTART:20250305T090000Z
        DURATION:PT30M
        SUMMARY:Plain
        END:VEVENT
    ";
    let (dir, mut vdir) = setup(&[
        ("work/call.ics", call),
        ("work/google.ics", google),
        ("work/plain.ics", plain),
    ]);
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    let find = |all: &[Occurrence], uid: &str| all.iter().find(|o| o.uid == uid).unwrap().clone();

    let c = find(&all, "call");
    assert_eq!(c.conferences.len(), 2);
    assert_eq!(c.conferences[0].features, ["PHONE"]);
    assert_eq!(c.conferences[1].features, ["AUDIO", "VIDEO"]);
    assert_eq!(c.conferences[1].label.as_deref(), Some("Meet"));
    assert_eq!(
        vdir.draft(&c).video.as_deref(),
        Some("https://meet.example.com/abc")
    );
    let g = find(&all, "google");
    assert!(g.conferences[0].has("VIDEO"));
    assert_eq!(g.conferences[0].uri, "https://meet.google.com/xyz");

    // a new link replaces the video one, keeping its parameters as written
    let path = dir.path().join("work/call.ics");
    let before = fs::read_to_string(&path).unwrap();
    let mut d = vdir.draft(&c);
    d.video = Some("https://meet.example.com/new".into());
    vdir.update(&c, Scope::All, &d, None).unwrap();
    // compared unfolded: long lines are folded at 75 octets
    let after = fs::read_to_string(&path).unwrap().replace("\r\n ", "");
    assert_eq!(
        changed_lines(&before.replace("\r\n ", ""), &after),
        [
            "CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Meet:https://meet.example.com/new",
            "SEQUENCE:1",
        ]
    );
    assert!(after.contains("tel:+44-20-7946-0000"));

    // added where there was none; untouched drafts leave files alone
    let p = find(&all, "plain");
    let mut d = vdir.draft(&p);
    assert_eq!(d.video.as_deref(), Some(""));
    d.video = Some("https://meet.example.com/p".into());
    vdir.update(&p, Scope::All, &d, None).unwrap();
    let text = fs::read_to_string(dir.path().join("work/plain.ics")).unwrap();
    assert!(
        text.contains("CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO:https://meet.example.com/p\r\n")
    );

    // cleared: Google's copy goes too, so it is not read back
    let mut d = vdir.draft(&g);
    d.video = Some(String::new());
    vdir.update(&g, Scope::All, &d, None).unwrap();
    let all = query(&vdir, range("2025-03-01 00:00", "2025-03-10 00:00"));
    assert!(find(&all, "google").conferences.is_empty());
}

#[test]
fn quick_add_writes_what_it_says() {
    use chrono::NaiveDate;
    use rical_core::quick;
    // an existing item with London's VTIMEZONE, for new ones to copy
    let meeting = format!("{LONDON_TZ}{MEETING}");
    let (dir, mut vdir) = setup(&[("home/meeting.ics", meeting.as_str())]);
    let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
    let slot = When::Date(today);
    let make = |vdir: &mut Vdir, text: &str, task: bool| {
        let q = quick::parse(text, today, today, SINGAPORE, task);
        let change = vdir.create("work", &q.draft(task, slot)).unwrap();
        fs::read_to_string(&change.path)
            .unwrap()
            .replace("\r\n ", "")
    };

    // a meeting in London time, from Singapore: written in London's zone
    let text = make(
        &mut vdir,
        "Meet Priya 3 jun 2027 10am London time @ King's Cross for 1h30",
        false,
    );
    assert!(text.contains("SUMMARY:Meet Priya\r\n"), "{text}");
    assert!(
        text.contains("DTSTART;TZID=Europe/London:20270603T100000\r\n"),
        "{text}"
    );
    assert!(
        text.contains("DTEND;TZID=Europe/London:20270603T113000\r\n"),
        "{text}"
    );
    assert!(text.contains("LOCATION:King's Cross\r\n"), "{text}");
    // its zone definition, copied from the item that has one
    assert!(
        text.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/London\r\n"),
        "{text}"
    );

    // a repeating event, all in the local zone
    let text = make(&mut vdir, "Standup every weekday 9:30 for 15m", false);
    assert!(
        text.contains("RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR\r\n"),
        "{text}"
    );

    // all day, over three days
    let text = make(&mut vdir, "Offsite 3-5 dec", false);
    assert!(text.contains("DTSTART;VALUE=DATE:20261203\r\n"), "{text}");
    assert!(text.contains("DTEND;VALUE=DATE:20261206\r\n"), "{text}");

    // a task: due date and priority, no start
    let text = make(&mut vdir, "Submit expenses by fri !", true);
    assert!(text.contains("BEGIN:VTODO\r\n"), "{text}");
    assert!(text.contains("STATUS:NEEDS-ACTION\r\n"), "{text}");
    assert!(text.contains("DUE;VALUE=DATE:20261023\r\n"), "{text}");
    assert!(text.contains("PRIORITY:1\r\n"), "{text}");
    assert!(!text.contains("DTSTART"), "{text}");

    // a task with neither date
    let text = make(&mut vdir, "Someday learn piano", true);
    assert!(!text.contains("DUE") && !text.contains("DTSTART"), "{text}");

    // and they all read back as written
    let vdir = Vdir::load(dir.path()).unwrap();
    let all = query(&vdir, range("2026-10-01 00:00", "2027-07-01 00:00"));
    assert!(all.iter().any(|o| o.summary == "Meet Priya"));
    assert!(all.iter().any(|o| o.summary == "Offsite" && o.all_day));
}

#[test]
fn a_zone_no_file_defines_gets_a_generated_vtimezone() {
    use chrono::NaiveDate;
    use rical_core::quick;
    let (dir, mut vdir) = setup(&[("home/meeting.ics", MEETING)]);
    let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
    let q = quick::parse(
        "Call Kenji 4 nov 9am Tokyo time",
        today,
        today,
        SINGAPORE,
        false,
    );
    let change = vdir
        .create("work", &q.draft(false, When::Date(today)))
        .unwrap();
    let text = fs::read_to_string(&change.path).unwrap();
    assert!(
        text.contains("DTSTART;TZID=Asia/Tokyo:20261104T090000\r\n"),
        "{text}"
    );
    assert!(
        text.contains("BEGIN:VTIMEZONE\r\nTZID:Asia/Tokyo\r\nBEGIN:STANDARD\r\n"),
        "{text}"
    );
    assert!(text.contains("TZOFFSETTO:+0900\r\n"), "{text}");
    // the definition comes before the event, and the file still reads
    assert!(text.find("BEGIN:VTIMEZONE") < text.find("BEGIN:VEVENT"));
    let vdir = Vdir::load(dir.path()).unwrap();
    let all = query(&vdir, range("2026-11-04 00:00", "2026-11-05 00:00"));
    let call = all.iter().find(|o| o.summary == "Call Kenji").unwrap();
    assert_eq!(
        call.start.to_utc(SINGAPORE),
        utc("2026-11-04 00:00"),
        "{call:?}"
    );
}

#[test]
fn events_and_tasks_convert_both_ways() {
    let lunch = "
        BEGIN:VEVENT
        UID:lunch
        DTSTAMP:20250101T000000Z
        DTSTART;TZID=Europe/London:20250304T120000
        DTEND;TZID=Europe/London:20250304T130000
        SUMMARY:Lunch
        LOCATION:Bistro 21
        STATUS:CONFIRMED
        TRANSP:OPAQUE
        X-UNKNOWN:keep me
        END:VEVENT
    ";
    let holiday = "
        BEGIN:VEVENT
        UID:holiday
        DTSTAMP:20250101T000000Z
        DTSTART;VALUE=DATE:20250310
        DTEND;VALUE=DATE:20250311
        SUMMARY:Pay rent
        END:VEVENT
    ";
    let file = format!("{LONDON_TZ}{lunch}");
    let (dir, mut vdir) = setup(&[
        ("home/lunch.ics", file.as_str()),
        ("home/holiday.ics", holiday),
        ("home/standup.ics", STANDUP),
    ]);
    let today = chrono::NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let find = |vdir: &Vdir, summary: &str| {
        query(vdir, range("2025-03-01 00:00", "2025-04-01 00:00"))
            .into_iter()
            .find(|o| o.summary == summary)
            .unwrap()
    };
    let read = |p: &std::path::Path| fs::read_to_string(p).unwrap();

    // an event: a task from its start, due when it ended, in its zone
    let occ = find(&vdir, "Lunch");
    let (changes, uid) = vdir.convert(&occ, today, SINGAPORE).unwrap();
    assert!(!occ.path.exists(), "the old file is gone");
    let text = read(&changes[0].path);
    assert!(text.contains("BEGIN:VTODO\r\n"), "{text}");
    assert!(text.contains(&format!("UID:{uid}\r\n")), "{text}");
    assert!(
        text.contains("DTSTART;TZID=Europe/London:20250304T120000\r\n"),
        "{text}"
    );
    assert!(
        text.contains("DUE;TZID=Europe/London:20250304T130000\r\n"),
        "{text}"
    );
    assert!(text.contains("STATUS:NEEDS-ACTION\r\n"), "{text}");
    assert!(text.contains("LOCATION:Bistro 21\r\n") && text.contains("X-UNKNOWN:keep me\r\n"));
    assert!(
        !text.contains("DTEND") && !text.contains("TRANSP"),
        "{text}"
    );
    assert!(
        text.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/London"),
        "{text}"
    );
    let task = vdir
        .tasks()
        .into_iter()
        .find(|t| t.summary == "Lunch")
        .unwrap();

    // and back: the same times again
    let occ = Occurrence {
        kind: Kind::Todo,
        ..find(&vdir, "Lunch")
    };
    assert_eq!(occ.path, task.path);
    let (changes, _) = vdir.convert(&occ, today, SINGAPORE).unwrap();
    let text = read(&changes[0].path);
    assert!(text.contains("BEGIN:VEVENT\r\n"), "{text}");
    assert!(
        text.contains("DTSTART;TZID=Europe/London:20250304T120000\r\n"),
        "{text}"
    );
    assert!(
        text.contains("DTEND;TZID=Europe/London:20250304T130000\r\n"),
        "{text}"
    );
    assert!(!text.contains("DUE") && !text.contains("STATUS"), "{text}");

    // a one-day all-day event: a task due that day
    let occ = find(&vdir, "Pay rent");
    let (changes, _) = vdir.convert(&occ, today, SINGAPORE).unwrap();
    let text = read(&changes[0].path);
    assert!(text.contains("DUE;VALUE=DATE:20250310\r\n"), "{text}");
    assert!(!text.contains("DTSTART"), "{text}");
    let task = vdir
        .tasks()
        .into_iter()
        .find(|t| t.summary == "Pay rent")
        .unwrap();
    assert_eq!(
        task.due,
        Some(When::Date(
            chrono::NaiveDate::from_ymd_opt(2025, 3, 10).unwrap()
        ))
    );

    // a repeating event keeps its rule, and a DURATION becomes a DUE
    let occ = find(&vdir, "Standup");
    let (changes, _) = vdir.convert(&occ, today, SINGAPORE).unwrap();
    let text = read(&changes[0].path);
    assert!(text.contains("RRULE:FREQ=DAILY;COUNT=5\r\n"), "{text}");
    assert!(text.contains("DUE:20250303T091500Z\r\n"), "{text}");

    // undone, the event is back as it was and the task gone
    for change in changes.iter().rev() {
        vdir.undo(change).unwrap();
    }
    assert_eq!(read(&dir.path().join("home/standup.ics")), ics(STANDUP));
    assert!(!changes[0].path.exists());
}

#[test]
fn tasks_without_times_become_all_day_events() {
    let (_dir, mut vdir) = setup(&[(
        "home/t.ics",
        "
        BEGIN:VTODO
        UID:someday
        DTSTAMP:20250101T000000Z
        SUMMARY:Someday
        STATUS:NEEDS-ACTION
        PERCENT-COMPLETE:10
        END:VTODO
        BEGIN:VTODO
        UID:by-five
        DTSTAMP:20250101T000000Z
        DUE:20250305T170000Z
        SUMMARY:By five
        END:VTODO
    ",
    )]);
    let today = chrono::NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let tasks = vdir.tasks();
    let occ = |summary: &str| {
        tasks
            .iter()
            .find(|t| t.summary == summary)
            .unwrap()
            .on_day(today)
    };
    let (changes, _) = vdir.convert(&occ("Someday"), today, SINGAPORE).unwrap();
    let text = fs::read_to_string(&changes[0].path).unwrap();
    assert!(
        text.contains("DTSTART;VALUE=DATE:20250301\r\nDTEND;VALUE=DATE:20250302\r\n"),
        "{text}"
    );
    assert!(
        !text.contains("PERCENT") && !text.contains("STATUS"),
        "{text}"
    );
    // the other task is still in the old file
    let rest = changes[1].after.as_deref().unwrap();
    assert!(
        rest.contains("UID:by-five") && !rest.contains("UID:someday"),
        "{rest}"
    );

    let (changes, _) = vdir.convert(&occ("By five"), today, SINGAPORE).unwrap();
    let text = fs::read_to_string(&changes[0].path).unwrap();
    assert!(
        text.contains("DTSTART:20250305T170000Z\r\nDTEND:20250305T180000Z\r\n"),
        "{text}"
    );
    assert_eq!(changes[1].after, None, "the emptied file is deleted");
}

#[test]
fn converting_refuses_what_would_hurt() {
    let parent = "
        BEGIN:VTODO
        UID:parent
        DTSTAMP:20250101T000000Z
        SUMMARY:Move house
        END:VTODO
        BEGIN:VTODO
        UID:child
        DTSTAMP:20250101T000000Z
        SUMMARY:Book van
        RELATED-TO;RELTYPE=PARENT:parent
        END:VTODO
    ";
    let (_dir, mut vdir) = setup(&[("home/meeting.ics", MEETING), ("home/p.ics", parent)]);
    let today = chrono::NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
    let meeting = query(&vdir, range("2025-03-04 00:00", "2025-03-05 00:00")).remove(0);
    let err = vdir.convert(&meeting, today, SINGAPORE).unwrap_err();
    assert!(err.to_string().contains("guests"), "{err}");
    let task = vdir
        .tasks()
        .into_iter()
        .find(|t| t.uid == "parent")
        .unwrap()
        .on_day(today);
    let err = vdir.convert(&task, today, SINGAPORE).unwrap_err();
    assert!(err.to_string().contains("subtasks"), "{err}");
}

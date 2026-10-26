//! Performance at scale: make a large synthetic vdir shaped like a real one,
//! then time what the app does with it and how much memory it takes.
//!
//! ```sh
//! cargo run --release --example perf -- gen /tmp/big 20000
//! cargo run --release --example perf -- run /tmp/big
//! ```
//!
//! The mix follows a real 1.6k-item calendar: about two thirds of items
//! repeat (birthdays yearly, meetings weekly, some daily), most are in a
//! named zone with its VTIMEZONE, a quarter have alarms, some have guests,
//! a sixth are tasks and a few are journal entries.

use std::{
    fmt::Write as _,
    path::Path,
    time::{Duration, Instant},
};

use chrono::{Datelike, Duration as TD, NaiveDate, TimeZone, Utc};
use chrono_tz::Tz;
use rical_core::{Kind, Vdir};

const LONDON: &str = "BEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nBEGIN:STANDARD\r\n\
DTSTART:19701025T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0000\r\n\
RRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\nBEGIN:DAYLIGHT\r\n\
DTSTART:19700329T010000\r\nTZOFFSETFROM:+0000\r\nTZOFFSETTO:+0100\r\n\
RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nEND:DAYLIGHT\r\nEND:VTIMEZONE\r\n";

/// A small deterministic generator, so runs are comparable.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

fn generate(dir: &Path, n: usize) {
    let calendars = ["personal", "work", "family", "birthdays", "shared"];
    for c in calendars {
        std::fs::create_dir_all(dir.join(c)).unwrap();
        std::fs::write(dir.join(c).join("displayname"), c).unwrap();
    }
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let base = NaiveDate::from_ymd_opt(2016, 1, 1).unwrap();
    let words = [
        "Standup",
        "Review",
        "Lunch",
        "Gym",
        "Call",
        "Dentist",
        "Planning",
        "Retro",
        "Interview",
        "Dinner",
        "Football",
        "Piano",
        "Sync",
        "Offsite",
        "Workshop",
    ];
    for i in 0..n {
        let uid = format!("perf-{i:06}");
        let cal = calendars[rng.below(calendars.len() as u64) as usize];
        let day = base + TD::days(rng.below(365 * 11) as i64);
        let title = format!("{} {}", words[rng.below(words.len() as u64) as usize], i);
        let mut ics = String::from("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//perf//EN\r\n");
        let roll = rng.below(100);
        let zoned = rng.chance(70);
        if roll < 17 {
            // a task
            ics += "BEGIN:VTODO\r\n";
            write!(
                ics,
                "UID:{uid}\r\nDTSTAMP:20250101T000000Z\r\nSUMMARY:{title}\r\n"
            )
            .unwrap();
            if rng.chance(70) {
                write!(ics, "DUE;VALUE=DATE:{}\r\n", day.format("%Y%m%d")).unwrap();
            }
            ics += if rng.chance(60) {
                "STATUS:COMPLETED\r\n"
            } else {
                "STATUS:NEEDS-ACTION\r\n"
            };
            ics += "END:VTODO\r\n";
        } else if roll < 19 {
            ics += "BEGIN:VJOURNAL\r\n";
            write!(
                ics,
                "UID:{uid}\r\nDTSTAMP:20250101T000000Z\r\nDTSTART;VALUE=DATE:{}\r\n\
                 SUMMARY:{title}\r\nDESCRIPTION:Notes from the day\\, at some length.\r\n\
                 END:VJOURNAL\r\n",
                day.format("%Y%m%d")
            )
            .unwrap();
        } else {
            if zoned {
                ics += LONDON;
            }
            ics += "BEGIN:VEVENT\r\n";
            write!(
                ics,
                "UID:{uid}\r\nDTSTAMP:20250101T000000Z\r\nSUMMARY:{title}\r\n"
            )
            .unwrap();
            let all_day = cal == "birthdays" || rng.chance(15);
            if all_day {
                write!(
                    ics,
                    "DTSTART;VALUE=DATE:{}\r\nDTEND;VALUE=DATE:{}\r\n",
                    day.format("%Y%m%d"),
                    (day + TD::days(1)).format("%Y%m%d")
                )
                .unwrap();
            } else {
                let h = 7 + rng.below(12);
                let m = [0, 15, 30, 45][rng.below(4) as usize];
                let (tzp, z) = if zoned {
                    (";TZID=Europe/London", "")
                } else {
                    ("", "Z")
                };
                write!(
                    ics,
                    "DTSTART{tzp}:{}T{h:02}{m:02}00{z}\r\nDTEND{tzp}:{}T{:02}{m:02}00{z}\r\n",
                    day.format("%Y%m%d"),
                    day.format("%Y%m%d"),
                    h + 1
                )
                .unwrap();
            }
            // two thirds repeat, as in the real calendar
            if cal == "birthdays" || rng.chance(63) {
                let rule = match (cal, rng.below(10)) {
                    ("birthdays", _) => "FREQ=YEARLY".to_owned(),
                    (_, 0) => "FREQ=DAILY;COUNT=30".to_owned(),
                    (_, 1) => format!(
                        "FREQ=DAILY;UNTIL={}T000000Z",
                        (day + TD::days(90)).format("%Y%m%d")
                    ),
                    (_, 2..=3) => "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_owned(),
                    (_, 4..=6) => "FREQ=WEEKLY".to_owned(),
                    (_, 7) => "FREQ=MONTHLY".to_owned(),
                    (_, 8) => format!(
                        "FREQ=WEEKLY;UNTIL={}T000000Z",
                        (day + TD::days(400)).format("%Y%m%d")
                    ),
                    _ => "FREQ=YEARLY".to_owned(),
                };
                write!(ics, "RRULE:{rule}\r\n").unwrap();
            }
            if rng.chance(48) {
                ics += "DESCRIPTION:Agenda: go over the plan\\, the numbers and what is next.\r\n";
            }
            if rng.chance(15) {
                ics += "ORGANIZER;CN=Priya Shah:mailto:priya@example.com\r\n\
                        ATTENDEE;CN=Me;PARTSTAT=ACCEPTED:mailto:me@example.com\r\n\
                        ATTENDEE;CN=Sam Lee;PARTSTAT=NEEDS-ACTION:mailto:sam@example.com\r\n";
            }
            if rng.chance(26) {
                ics += "BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n";
            }
            ics += "END:VEVENT\r\n";
        }
        ics += "END:VCALENDAR\r\n";
        std::fs::write(dir.join(cal).join(format!("{uid}.ics")), ics).unwrap();
    }
}

/// This process's memory: (now, peak) in MB, from /proc.
fn memory() -> (f64, f64) {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status
            .lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok())
            .unwrap_or(0.0)
            / 1024.0
    };
    (field("VmRSS:"), field("VmHWM:"))
}

/// The mean time of `f` over `n` runs.
fn time<T>(n: u32, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut last = None;
    let start = Instant::now();
    for _ in 0..n {
        last = Some(f());
    }
    (start.elapsed() / n, last.unwrap())
}

fn run(dir: &Path) {
    let local: Tz = chrono_tz::Europe::London;
    let (before, _) = memory();
    let start = Instant::now();
    let vdir = Vdir::load(dir).unwrap();
    let load = start.elapsed();
    let (after, peak) = memory();
    let items = vdir.components().count();
    println!("items          {items}");
    println!("load           {:>8.1} ms", load.as_secs_f64() * 1e3);
    println!(
        "memory         {:>8.1} MB held after load ({:.1} MB peak)",
        after - before,
        peak
    );

    let day = |y, m, d| local.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap().to_utc();
    let ranges = [
        ("week query", day(2026, 10, 19)..day(2026, 10, 26)),
        ("month query", day(2026, 9, 28)..day(2026, 11, 9)),
        ("year query", day(2026, 1, 1)..day(2027, 1, 1)),
        ("agenda 30d", day(2026, 10, 21)..day(2026, 11, 20)),
    ];
    for (name, r) in &ranges {
        let (t, n) = time(10, || {
            vdir.query(r, &[Kind::Event, Kind::Todo, Kind::Journal], local)
                .len()
        });
        println!(
            "{name:<14} {:>8.2} ms ({n} occurrences)",
            t.as_secs_f64() * 1e3
        );
    }
    let now = Utc::now();
    let (t, n) = time(10, || {
        vdir.alarms(&(now - TD::seconds(30)..now), local, Some(TD::minutes(10)))
            .len()
    });
    println!("alarm check    {:>8.2} ms ({n} due)", t.as_secs_f64() * 1e3);
    let (t, n) = time(10, || vdir.tasks().len());
    println!("tasks          {:>8.2} ms ({n})", t.as_secs_f64() * 1e3);
    let (t, n) = time(10, || vdir.journals().len());
    println!("journals       {:>8.2} ms ({n})", t.as_secs_f64() * 1e3);
    let (t, n) = time(3, || vdir.people(&[]).len());
    println!("people         {:>8.2} ms ({n})", t.as_secs_f64() * 1e3);
    let _ = now.year();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("gen") => {
            let n = args.get(3).and_then(|n| n.parse().ok()).unwrap_or(20_000);
            generate(Path::new(&args[2]), n);
        }
        Some("run") => run(Path::new(&args[2])),
        // for a profiler: the week query, over and over
        Some("week") => {
            let vdir = Vdir::load(&args[2]).unwrap();
            let local: Tz = chrono_tz::Europe::London;
            let from = local
                .with_ymd_and_hms(2026, 10, 19, 0, 0, 0)
                .unwrap()
                .to_utc();
            let r = from..from + TD::days(7);
            for _ in 0..300 {
                std::hint::black_box(vdir.query(
                    &r,
                    &[Kind::Event, Kind::Todo, Kind::Journal],
                    local,
                ));
            }
        }
        _ => eprintln!("perf gen DIR [N] | perf run DIR"),
    }
}

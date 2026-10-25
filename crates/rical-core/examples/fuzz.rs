//! A mutation fuzzer for rical's iCalendar handling that runs on stable
//! Rust: it takes real files as seeds, mutates them, and checks that
//!
//! - the compact parser never panics,
//! - the editing document (doc.rs) never panics and writes back exactly
//!   what it read,
//! - expanding occurrences (rules, dates, durations, people, alarms,
//!   conferences, journals) never panics or hangs.
//!
//! ```text
//! cargo run --release -p rical-core --example fuzz -- SECONDS OUT_DIR SEED_DIR...
//! ```
//!
//! Inputs that fail are saved to OUT_DIR. A hang (an input taking longer
//! than a few seconds) is saved and stops the run, as it cannot be undone.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use chrono::{TimeZone as _, Utc};
use chrono_tz::Tz;
use rical_core::{AnyComponent, compact, doc::Document, occurrence};

/// xorshift64*: small and good enough to pick mutations.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

/// Pieces that tend to matter to a parser of this format.
const TOKENS: &[&str] = &[
    "\r\n",
    "\n",
    "\r\n ",
    "\r\n\t",
    ":",
    ";",
    "=",
    ",",
    "\"",
    "\\",
    "\\n",
    "\\,",
    "BEGIN:VEVENT\r\n",
    "END:VEVENT\r\n",
    "BEGIN:VTODO\r\n",
    "END:VTODO\r\n",
    "BEGIN:VJOURNAL\r\n",
    "END:VJOURNAL\r\n",
    "BEGIN:VALARM\r\n",
    "END:VALARM\r\n",
    "BEGIN:VCALENDAR\r\n",
    "END:VCALENDAR\r\n",
    "BEGIN:VTIMEZONE\r\n",
    "END:VTIMEZONE\r\n",
    "BEGIN:",
    "END:",
    "DTSTART:",
    "DTEND:",
    "DUE:",
    "DURATION:",
    "RRULE:",
    "EXDATE:",
    "RDATE:",
    "RECURRENCE-ID:",
    "UID:x\r\n",
    "TRIGGER:",
    "ATTENDEE;",
    "ORGANIZER:",
    "CONFERENCE;FEATURE=VIDEO,AUDIO:",
    "RELATED-TO;RELTYPE=PARENT:",
    ";TZID=",
    ";TZID=Europe/London:",
    ";TZID=Nowhere/City:",
    ";VALUE=DATE:",
    ";VALUE=PERIOD:",
    ";RANGE=THISANDFUTURE",
    ";RELATED=END",
    "20250101",
    "20250101T090000",
    "20250101T090000Z",
    "99991231T235959Z",
    "00000000T000000",
    "19700101",
    "20251032",
    "20250229",
    "T250000",
    "PT15M",
    "-PT15M",
    "P1D",
    "P9999999W",
    "PT0S",
    "P",
    "PT",
    "-P1W2DT3H4M5S",
    "FREQ=DAILY",
    "FREQ=SECONDLY",
    "FREQ=YEARLY",
    "FREQ=MINUTELY",
    ";COUNT=3",
    ";COUNT=0",
    ";COUNT=4294967296",
    ";INTERVAL=0",
    ";INTERVAL=1000000",
    ";UNTIL=20250105T000000Z",
    ";UNTIL=20250105",
    ";BYDAY=MO,TU,-1SU,53MO,+0FR",
    ";BYMONTHDAY=31,-31,0",
    ";BYSETPOS=-366,0",
    ";BYMONTH=2,13",
    ";BYWEEKNO=53,-53",
    ";BYYEARDAY=366,-366",
    ";BYHOUR=25",
    ";WKST=XX",
    "mailto:me@example.com",
    "PARTSTAT=NEEDS-ACTION",
    "CN=\"a;b:c\"",
    "é",
    "€",
    "𝄞",
    "\u{0}",
    "\u{feff}",
    "-",
    "+",
    "999999999999999999999",
];

fn mutate(input: &[u8], seeds: &[Vec<u8>], rng: &mut Rng) -> Vec<u8> {
    let mut out = input.to_vec();
    for _ in 0..1 + rng.below(6) {
        let len = out.len();
        let at = rng.below(len + 1);
        match rng.below(9) {
            // flip a byte
            0 if len > 0 => {
                let i = rng.below(len);
                out[i] ^= 1 << rng.below(8);
            }
            // a token, inserted or overwriting
            1 | 2 => {
                let t = TOKENS[rng.below(TOKENS.len())].as_bytes();
                out.splice(at..at, t.iter().copied());
            }
            3 => {
                let t = TOKENS[rng.below(TOKENS.len())].as_bytes();
                let end = (at + t.len()).min(len);
                out.splice(at..end, t.iter().copied());
            }
            // delete a span
            4 if len > 0 => {
                let end = (at + 1 + rng.below(64)).min(len);
                out.drain(at.min(end)..end);
            }
            // repeat a span
            5 if len > 0 => {
                let start = rng.below(len);
                let end = (start + 1 + rng.below(256)).min(len);
                let span = out[start..end].to_vec();
                out.splice(at..at, span);
            }
            // a span of another seed
            6 => {
                let other = &seeds[rng.below(seeds.len())];
                if !other.is_empty() {
                    let start = rng.below(other.len());
                    let end = (start + 1 + rng.below(512)).min(other.len());
                    out.splice(at..at, other[start..end].iter().copied());
                }
            }
            // cut it short
            7 => out.truncate(at),
            // a random byte
            _ => out.insert(at, rng.next() as u8),
        }
    }
    out
}

/// Every check, on one input. Panics on a failure.
fn check(text: &str) {
    // the compact parser, and everything built from what it reads
    if let Ok((items, _tzids)) = compact::parse(text) {
        let path: Arc<Path> = Arc::from(Path::new("fuzz.ics"));
        let components = items
            .into_iter()
            .map(|c| AnyComponent::new(path.clone(), c))
            .collect::<Vec<_>>();
        let range = Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap()
            ..Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap();
        for local in [
            Tz::Europe__London,
            Tz::Asia__Singapore,
            Tz::America__New_York,
        ] {
            let occ = occurrence::expand("fuzz", &components, &range, local);
            for o in &occ {
                // what the views do with every occurrence
                let _ = (o.start.to_utc(local), o.end.to_utc(local), o.duration());
                let _ = (o.start.date(local), o.end.to_tz(local));
            }
        }
        for c in &components {
            let _ = occurrence::journal_entry("fuzz", c);
            let _ = occurrence::conferences(c);
            let _ = occurrence::people(c, "ATTENDEE");
            let _ = occurrence::parent_of(c);
        }
    }
    // the editing document: what it reads, it writes back unchanged
    if let Ok(doc) = Document::parse(text) {
        let written = doc.write();
        assert!(
            written == text,
            "document did not write back what it read ({} bytes became {})",
            text.len(),
            written.len()
        );
        for item in doc.items() {
            let _ = (item.uid(), item.recurrence_id());
            for line in item.lines() {
                let _ = (line.name(), line.params(), line.value(), line.when());
            }
        }
    }
}

fn seeds(dirs: &[PathBuf]) -> Vec<Vec<u8>> {
    fn walk(dir: &Path, out: &mut Vec<Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().is_some_and(|e| e == "ics")
                && let Ok(b) = std::fs::read(&p)
            {
                out.push(b);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        walk(d, &mut out);
    }
    out
}

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    // `fuzz replay FILE...`: run the checks on saved inputs, panics shown
    if args.first().map(String::as_str) == Some("replay") {
        for f in &args[1..] {
            eprintln!("{f}");
            check(&String::from_utf8_lossy(&std::fs::read(f).unwrap()));
        }
        return;
    }
    let [secs, out_dir, seed_dirs @ ..] = &args[..] else {
        eprintln!("usage: fuzz SECONDS OUT_DIR SEED_DIR...");
        std::process::exit(2);
    };
    let secs: u64 = secs.parse().expect("SECONDS");
    let out_dir = PathBuf::from(out_dir);
    std::fs::create_dir_all(&out_dir).unwrap();
    let seeds = seeds(&seed_dirs.iter().map(PathBuf::from).collect::<Vec<_>>());
    assert!(!seeds.is_empty(), "no .ics seeds found");
    // the seeds themselves must pass
    for s in &seeds {
        check(&String::from_utf8_lossy(s));
    }
    eprintln!("{} seeds pass; fuzzing for {secs}s", seeds.len());

    std::panic::set_hook(Box::new(|_| {}));
    let seed = std::env::var("FUZZ_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| Instant::now().elapsed().as_nanos() as u64 | 1);
    let mut rng = Rng(seed ^ 0x9e37_79b9_7f4a_7c15);
    let deadline = Instant::now() + Duration::from_secs(secs);
    let (mut runs, mut failures) = (0u64, 0u64);
    let mut seen = std::collections::HashSet::new();

    // one worker thread, watched: an input that takes too long is a hang
    let (to_worker, from_main) = mpsc::channel::<String>();
    let (to_main, from_worker) = mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        for text in from_main {
            let r = std::panic::catch_unwind(|| check(&text)).map_err(|e| {
                e.downcast_ref::<String>()
                    .cloned()
                    .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "panic".into())
            });
            if to_main.send(r).is_err() {
                return;
            }
        }
    });

    while Instant::now() < deadline {
        let base = &seeds[rng.below(seeds.len())];
        let bytes = mutate(base, &seeds, &mut rng);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        to_worker.send(text.clone()).unwrap();
        runs += 1;
        match from_worker.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => {}
            Ok(Err(message)) => {
                // one example of each failure
                let key = message.lines().next().unwrap_or("").to_owned();
                if seen.insert(key.clone()) {
                    failures += 1;
                    let file = out_dir.join(format!("fail-{failures}.ics"));
                    std::fs::write(&file, &text).unwrap();
                    eprintln!("FAIL {}: {key}", file.display());
                }
            }
            Err(_) => {
                let file = out_dir.join("hang.ics");
                std::fs::write(&file, &text).unwrap();
                eprintln!("HANG {} (over 5s)", file.display());
                std::process::exit(1);
            }
        }
    }
    eprintln!("{runs} inputs, {failures} distinct failures, seed {seed}");
    if failures > 0 {
        std::process::exit(1);
    }
}

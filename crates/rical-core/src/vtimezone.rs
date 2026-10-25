//! VTIMEZONE definitions made from the IANA database, for a file that
//! names a zone no other file in the vdir has a definition of.
//!
//! The transitions from `from` onwards are found by probing chrono-tz.
//! A transition that recurs on the same weekday of the same month every
//! year becomes one rule (`FREQ=YEARLY;BYMONTH=..;BYDAY=..`), open ended
//! if it is still in use at the end of the probed years; anything
//! irregular is listed by date.

use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use chrono_tz::{OffsetComponents, OffsetName, Tz};
use std::fmt::Write;

/// How far past `from` (or now) transitions are looked for.
const YEARS_AHEAD: i32 = 12;

/// The VTIMEZONE text (BEGIN..END, CRLF lines) for an IANA `tzid`,
/// covering times from the start of year `from`; `None` if chrono-tz
/// does not know the name.
pub fn generate(tzid: &str, from: i32) -> Option<String> {
    let tz: Tz = tzid.parse().ok()?;
    let start = Utc.with_ymd_and_hms(from, 1, 1, 0, 0, 0).single()?;
    let end_year = from.max(Utc::now().year()) + YEARS_AHEAD;
    let end = Utc.with_ymd_and_hms(end_year, 1, 1, 0, 0, 0).single()?;
    let found = transitions(&tz, start, end);

    let mut out = format!("BEGIN:VTIMEZONE\r\nTZID:{tzid}\r\n");
    // the offset in use before the first transition (or always, for a
    // zone without any), so earlier times in the year are covered too
    let first = Shift::at(&tz, start, start);
    let before = Shift {
        from: first.to,
        local: start.date_naive().and_time(chrono::NaiveTime::MIN),
        ..first
    };
    if found.first().is_none_or(|t| t.local > before.local) {
        before.write(&mut out, &[]);
    }
    for run in runs(&found, end_year - 1) {
        match run {
            Run::Single(t) => t.write(&mut out, &[]),
            Run::Yearly {
                first,
                byday,
                until,
            } => {
                let mut rule = format!(
                    "RRULE:FREQ=YEARLY;BYMONTH={};BYDAY={byday}",
                    first.local.month()
                );
                if let Some(until) = until {
                    write!(rule, ";UNTIL={}", until.format("%Y%m%dT%H%M%SZ")).unwrap();
                }
                first.write(&mut out, &[rule]);
            }
        }
    }
    out.push_str("END:VTIMEZONE\r\n");
    Some(out)
}

/// One change of offset: the local wall time it happens at (in the
/// offset before it, as DTSTART is written), and the offsets around it.
#[derive(Clone, Debug, PartialEq)]
struct Shift {
    utc: DateTime<Utc>,
    local: NaiveDateTime,
    from: i32,
    to: i32,
    dst: bool,
    name: Option<String>,
}

impl Shift {
    /// The shift that ends at `utc`, given the offset just before `prev`.
    fn at(tz: &Tz, prev: DateTime<Utc>, utc: DateTime<Utc>) -> Shift {
        let before = tz.offset_from_utc_datetime(&prev.naive_utc());
        let after = tz.offset_from_utc_datetime(&utc.naive_utc());
        let from = offset_secs(&before);
        Shift {
            utc,
            local: utc.naive_utc() + Duration::seconds(from as i64),
            from,
            to: offset_secs(&after),
            dst: !after.dst_offset().is_zero(),
            name: after.abbreviation().map(str::to_owned),
        }
    }

    /// What makes two shifts the same rule in different years.
    fn key(&self) -> (u32, chrono::Weekday, chrono::NaiveTime, i32, i32, bool) {
        (
            self.local.month(),
            self.local.weekday(),
            self.local.time(),
            self.from,
            self.to,
            self.dst,
        )
    }

    fn write(&self, out: &mut String, extra: &[String]) {
        let kind = if self.dst { "DAYLIGHT" } else { "STANDARD" };
        write!(out, "BEGIN:{kind}\r\n").unwrap();
        write!(out, "DTSTART:{}\r\n", self.local.format("%Y%m%dT%H%M%S")).unwrap();
        write!(out, "TZOFFSETFROM:{}\r\n", format_offset(self.from)).unwrap();
        write!(out, "TZOFFSETTO:{}\r\n", format_offset(self.to)).unwrap();
        // abbreviations like "+08" say nothing a client can't work out
        if let Some(name) = self
            .name
            .as_ref()
            .filter(|n| n.starts_with(char::is_alphabetic))
        {
            write!(out, "TZNAME:{name}\r\n").unwrap();
        }
        for line in extra {
            write!(out, "{line}\r\n").unwrap();
        }
        write!(out, "END:{kind}\r\n").unwrap();
    }
}

fn offset_secs(o: &chrono_tz::TzOffset) -> i32 {
    (o.base_utc_offset() + o.dst_offset()).num_seconds() as i32
}

fn format_offset(secs: i32) -> String {
    let sign = if secs < 0 { '-' } else { '+' };
    let s = secs.unsigned_abs();
    let (h, m, sec) = (s / 3600, s / 60 % 60, s % 60);
    if sec == 0 {
        format!("{sign}{h:02}{m:02}")
    } else {
        format!("{sign}{h:02}{m:02}{sec:02}")
    }
}

/// Every change of offset or DST-ness between `start` and `end`. Days are
/// stepped through and each change narrowed down to the second; no zone
/// changes twice in a day.
fn transitions(tz: &Tz, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<Shift> {
    let state = |t: DateTime<Utc>| {
        let o = tz.offset_from_utc_datetime(&t.naive_utc());
        (offset_secs(&o), o.dst_offset().is_zero())
    };
    let mut out = Vec::new();
    let mut t = start;
    let mut s = state(t);
    while t < end {
        let next = t + Duration::days(1);
        let ns = state(next);
        if ns != s {
            // the first second with the new state
            let (mut lo, mut hi) = (t.timestamp(), next.timestamp());
            while hi - lo > 1 {
                let mid = lo + (hi - lo) / 2;
                if state(DateTime::from_timestamp(mid, 0).unwrap()) == s {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let at = DateTime::from_timestamp(hi, 0).unwrap();
            let shift = Shift::at(tz, DateTime::from_timestamp(lo, 0).unwrap(), at);
            out.push(shift);
        }
        t = next;
        s = ns;
    }
    out
}

enum Run {
    Single(Shift),
    /// The same shift in consecutive years from `first`, `until` the last
    /// of them unless it carries on.
    Yearly {
        first: Shift,
        byday: String,
        until: Option<DateTime<Utc>>,
    },
}

/// Group shifts into yearly rules where they repeat.
fn runs(shifts: &[Shift], last_year: i32) -> Vec<Run> {
    let mut used = vec![false; shifts.len()];
    let mut out = Vec::new();
    for i in 0..shifts.len() {
        if used[i] {
            continue;
        }
        let first = shifts[i].clone();
        // the same rule in each following year, one shift per year
        let mut members = vec![i];
        for (j, s) in shifts.iter().enumerate().skip(i + 1) {
            let year = shifts[*members.last().unwrap()].local.year();
            if s.local.year() > year + 1 {
                break;
            }
            if !used[j] && s.local.year() == year + 1 && s.key() == first.key() {
                members.push(j);
            }
        }
        // the longest stretch from the first that one BYDAY describes
        let nth = |s: &Shift| (s.local.day() as i32 - 1) / 7 + 1;
        let is_last = |s: &Shift| s.local.day() + 7 > days_in_month(s.local.date());
        let by_nth = members
            .iter()
            .take_while(|&&m| nth(&shifts[m]) == nth(&first))
            .count();
        let by_last = members.iter().take_while(|&&m| is_last(&shifts[m])).count();
        let (len, n) = if by_last >= by_nth {
            (by_last, -1)
        } else {
            (by_nth, nth(&first))
        };
        if len < 2 {
            used[i] = true;
            out.push(Run::Single(first));
            continue;
        }
        let members = &members[..len];
        for &m in members {
            used[m] = true;
        }
        let last = &shifts[*members.last().unwrap()];
        let carries_on = last.local.year() >= last_year;
        let byday = format!("{n}{}", weekday_code(first.local.weekday()));
        out.push(Run::Yearly {
            first,
            byday,
            until: (!carries_on).then_some(last.utc),
        });
    }
    out
}

fn days_in_month(d: NaiveDate) -> u32 {
    let (y, m) = (d.year(), d.month());
    let next = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1)
    };
    next.unwrap().pred_opt().unwrap().day()
}

fn weekday_code(w: chrono::Weekday) -> &'static str {
    use chrono::Weekday::*;
    match w {
        Mon => "MO",
        Tue => "TU",
        Wed => "WE",
        Thu => "TH",
        Fri => "FR",
        Sat => "SA",
        Sun => "SU",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zone_without_daylight_saving_is_one_standard_offset() {
        let tz = generate("Asia/Tokyo", 2026).unwrap();
        assert_eq!(
            tz,
            "BEGIN:VTIMEZONE\r\nTZID:Asia/Tokyo\r\nBEGIN:STANDARD\r\nDTSTART:20260101T000000\r\n\
             TZOFFSETFROM:+0900\r\nTZOFFSETTO:+0900\r\nTZNAME:JST\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n"
        );
    }

    #[test]
    fn european_summer_time_is_two_open_rules() {
        let tz = generate("Europe/London", 2026).unwrap();
        assert!(tz.contains(
            "BEGIN:DAYLIGHT\r\nDTSTART:20260329T010000\r\nTZOFFSETFROM:+0000\r\nTZOFFSETTO:+0100\r\n\
             TZNAME:BST\r\nRRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=-1SU\r\nEND:DAYLIGHT\r\n"
        ));
        assert!(tz.contains(
            "BEGIN:STANDARD\r\nDTSTART:20261025T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0000\r\n\
             TZNAME:GMT\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\n"
        ));
        assert!(!tz.contains("UNTIL"), "{tz}");
    }

    #[test]
    fn rules_that_changed_end_with_until() {
        // the US moved its dates in 2007
        let tz = generate("America/New_York", 2005).unwrap();
        assert!(
            tz.contains("RRULE:FREQ=YEARLY;BYMONTH=4;BYDAY=1SU;UNTIL=20060402T070000Z"),
            "{tz}"
        );
        assert!(
            tz.contains("RRULE:FREQ=YEARLY;BYMONTH=3;BYDAY=2SU\r\n"),
            "{tz}"
        );
        assert!(
            tz.contains("RRULE:FREQ=YEARLY;BYMONTH=11;BYDAY=1SU\r\n"),
            "{tz}"
        );
    }

    #[test]
    fn southern_and_odd_offsets() {
        let tz = generate("Australia/Adelaide", 2026).unwrap();
        assert!(tz.contains("TZOFFSETTO:+1030"), "{tz}");
        assert!(tz.contains("BYMONTH=10;BYDAY=1SU"), "{tz}");
        assert!(tz.contains("BYMONTH=4;BYDAY=1SU"), "{tz}");
        assert!(generate("Asia/Kathmandu", 2026).unwrap().contains("+0545"));
        assert_eq!(generate("Not/AZone", 2026), None);
    }

    #[test]
    fn every_zone_reads_back_with_the_same_offsets() {
        // read each definition back the way a client would and compare it
        // with chrono-tz at a few times through the years
        for tz in chrono_tz::TZ_VARIANTS {
            let text = generate(tz.name(), 2020).unwrap();
            let defined = |t| offset_at(&text, t);
            for month in [1, 4, 7, 10] {
                for year in [2020, 2026, 2035] {
                    let t = Utc.with_ymd_and_hms(year, month, 15, 12, 0, 0).unwrap();
                    let want = offset_secs(&tz.offset_from_utc_datetime(&t.naive_utc()));
                    assert_eq!(defined(t), want, "{} at {t}\n{text}", tz.name());
                }
            }
        }
    }

    /// The offset `text` gives at `t`: the TZOFFSETTO of the latest onset
    /// at or before it, expanding the yearly rules this module writes.
    fn offset_at(text: &str, t: DateTime<Utc>) -> i32 {
        let parse_off = |v: &str| {
            let sign = if v.starts_with('-') { -1 } else { 1 };
            let d = &v[1..];
            let h: i32 = d[0..2].parse().unwrap();
            let m: i32 = d[2..4].parse().unwrap();
            let s: i32 = d.get(4..6).map_or(0, |s| s.parse().unwrap());
            sign * (h * 3600 + m * 60 + s)
        };
        let mut best: Option<(DateTime<Utc>, i32)> = None;
        for block in text.split("BEGIN:").skip(2) {
            let get = |name: &str| {
                block
                    .split("\r\n")
                    .find_map(|l| l.strip_prefix(name).and_then(|l| l.strip_prefix(':')))
            };
            let start =
                NaiveDateTime::parse_from_str(get("DTSTART").unwrap(), "%Y%m%dT%H%M%S").unwrap();
            let from = parse_off(get("TZOFFSETFROM").unwrap());
            let to = parse_off(get("TZOFFSETTO").unwrap());
            let mut onsets = vec![start];
            if let Some(rule) = get("RRULE") {
                let part = |k: &str| {
                    rule.split(';')
                        .find_map(|p| p.strip_prefix(k).and_then(|p| p.strip_prefix('=')))
                };
                let month: u32 = part("BYMONTH").unwrap().parse().unwrap();
                let byday = part("BYDAY").unwrap();
                let (n, wd) = byday.split_at(byday.len() - 2);
                let n: i32 = n.parse().unwrap();
                let wd = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]
                    .iter()
                    .position(|w| *w == wd)
                    .unwrap() as u32;
                let until = part("UNTIL").map(|u| {
                    NaiveDateTime::parse_from_str(u, "%Y%m%dT%H%M%SZ")
                        .unwrap()
                        .and_utc()
                });
                for year in start.year() + 1..=t.year() {
                    let days: Vec<NaiveDate> = (1..=31)
                        .filter_map(|d| NaiveDate::from_ymd_opt(year, month, d))
                        .filter(|d| d.weekday().num_days_from_monday() == wd)
                        .collect();
                    let day = if n < 0 {
                        *days.last().unwrap()
                    } else {
                        days[n as usize - 1]
                    };
                    let onset = day.and_time(start.time());
                    let utc = onset.and_utc() - Duration::seconds(from as i64);
                    if until.is_some_and(|u| utc > u) {
                        break;
                    }
                    onsets.push(onset);
                }
            }
            for onset in onsets {
                let utc = onset.and_utc() - Duration::seconds(from as i64);
                if utc <= t && best.is_none_or(|(b, _)| utc > b) {
                    best = Some((utc, to));
                }
            }
        }
        best.expect("no onset before the time").1
    }
}

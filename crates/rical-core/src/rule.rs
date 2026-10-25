//! The part of RRULE that rical edits: frequency, interval, weekdays and
//! how the series ends. Rules using more than this are kept as they are.

use std::fmt;

use chrono::{NaiveDate, NaiveTime, Weekday};
use chrono_tz::Tz;

use crate::when::{When, resolve_local};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    pub const ALL: [Freq; 4] = [Self::Daily, Self::Weekly, Self::Monthly, Self::Yearly];

    pub fn name(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Monthly => "monthly",
            Self::Yearly => "yearly",
        }
    }

    fn unit(self, n: u32) -> &'static str {
        match (self, n == 1) {
            (Self::Daily, true) => "day",
            (Self::Daily, false) => "days",
            (Self::Weekly, true) => "week",
            (Self::Weekly, false) => "weeks",
            (Self::Monthly, true) => "month",
            (Self::Monthly, false) => "months",
            (Self::Yearly, true) => "year",
            (Self::Yearly, false) => "years",
        }
    }

    fn rrule(self) -> &'static str {
        match self {
            Self::Daily => "DAILY",
            Self::Weekly => "WEEKLY",
            Self::Monthly => "MONTHLY",
            Self::Yearly => "YEARLY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    Never,
    /// After this many occurrences.
    Count(u32),
    /// On this day (inclusive).
    Until(NaiveDate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub freq: Freq,
    /// Every this many days/weeks/...; at least 1.
    pub interval: u32,
    /// For weekly rules: which days (empty means the start's day).
    pub weekdays: Vec<Weekday>,
    pub end: End,
}

impl Rule {
    pub fn new(freq: Freq) -> Self {
        Self {
            freq,
            interval: 1,
            weekdays: Vec::new(),
            end: End::Never,
        }
    }

    /// Read an RRULE value, if it only uses what [`Rule`] models.
    pub fn parse(rule: &str) -> Option<Self> {
        let mut freq = None;
        let mut out = Rule::new(Freq::Daily);
        for part in rule.split(';').filter(|p| !p.is_empty()) {
            let (key, value) = part.split_once('=')?;
            match key.to_ascii_uppercase().as_str() {
                "FREQ" => {
                    freq = Some(match value.to_ascii_uppercase().as_str() {
                        "DAILY" => Freq::Daily,
                        "WEEKLY" => Freq::Weekly,
                        "MONTHLY" => Freq::Monthly,
                        "YEARLY" => Freq::Yearly,
                        _ => return None,
                    })
                }
                "INTERVAL" => out.interval = value.parse().ok().filter(|n| *n >= 1)?,
                "COUNT" => out.end = End::Count(value.parse().ok().filter(|n| *n >= 1)?),
                "UNTIL" => out.end = End::Until(When::parse(value, None)?.date(Tz::UTC)),
                "BYDAY" => {
                    out.weekdays = value
                        .split(',')
                        .map(parse_weekday)
                        .collect::<Option<Vec<_>>>()?;
                }
                "WKST" => {}
                _ => return None,
            }
        }
        out.freq = freq?;
        if !out.weekdays.is_empty() && out.freq != Freq::Weekly {
            return None;
        }
        Some(out)
    }

    /// The RRULE value for a series starting at `start` (needed to write
    /// UNTIL in the form RFC 5545 asks for).
    pub fn to_rrule(&self, start: &When) -> String {
        let mut parts = vec![format!("FREQ={}", self.freq.rrule())];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if self.freq == Freq::Weekly && !self.weekdays.is_empty() {
            let mut days = self.weekdays.clone();
            days.sort_by_key(|d| d.num_days_from_monday());
            days.dedup();
            parts.push(format!(
                "BYDAY={}",
                days.iter()
                    .map(|d| weekday_code(*d))
                    .collect::<Vec<_>>()
                    .join(",")
            ));
        }
        match self.end {
            End::Never => {}
            End::Count(n) => parts.push(format!("COUNT={n}")),
            End::Until(day) => parts.push(format!(
                "UNTIL={}",
                match start {
                    When::Date(_) => day.format("%Y%m%d").to_string(),
                    When::Floating(_) => day
                        .and_hms_opt(23, 59, 59)
                        .unwrap()
                        .format("%Y%m%dT%H%M%S")
                        .to_string(),
                    // the end of that day where the series lives, in UTC
                    When::Zoned(dt) => resolve_local(
                        dt.timezone(),
                        day.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap())
                    )
                    .to_utc()
                    .format("%Y%m%dT%H%M%SZ")
                    .to_string(),
                }
            )),
        }
        parts.join(";")
    }
}

impl fmt::Display for Rule {
    /// e.g. "every 2 weeks on Mon, Wed, 10 times".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.interval == 1 {
            write!(f, "every {}", self.freq.unit(1))?;
        } else {
            write!(
                f,
                "every {} {}",
                self.interval,
                self.freq.unit(self.interval)
            )?;
        }
        if !self.weekdays.is_empty() {
            let mut days = self.weekdays.clone();
            days.sort_by_key(|d| d.num_days_from_monday());
            let names = days.iter().map(|d| format!("{d}")).collect::<Vec<_>>();
            write!(f, " on {}", names.join(", "))?;
        }
        match self.end {
            End::Never => Ok(()),
            End::Count(1) => write!(f, ", once"),
            End::Count(n) => write!(f, ", {n} times"),
            End::Until(d) => write!(f, ", until {}", d.format("%-d %b %Y")),
        }
    }
}

/// "mo", "Monday", "tue"... and RRULE's "MO".
pub fn parse_weekday(s: &str) -> Option<Weekday> {
    let s = s.trim().to_ascii_lowercase();
    if !s.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(match s.get(..2)? {
        "mo" => Weekday::Mon,
        "tu" => Weekday::Tue,
        "we" => Weekday::Wed,
        "th" => Weekday::Thu,
        "fr" => Weekday::Fri,
        "sa" => Weekday::Sat,
        "su" => Weekday::Sun,
        _ => return None,
    })
}

fn weekday_code(d: Weekday) -> &'static str {
    match d {
        Weekday::Mon => "MO",
        Weekday::Tue => "TU",
        Weekday::Wed => "WE",
        Weekday::Thu => "TH",
        Weekday::Fri => "FR",
        Weekday::Sat => "SA",
        Weekday::Sun => "SU",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn parse_and_write() {
        let r = Rule::parse("FREQ=WEEKLY;INTERVAL=2;BYDAY=WE,MO;COUNT=10").unwrap();
        assert_eq!(r.freq, Freq::Weekly);
        assert_eq!(r.interval, 2);
        assert_eq!(r.weekdays, [Weekday::Wed, Weekday::Mon]);
        assert_eq!(r.end, End::Count(10));
        assert_eq!(r.to_string(), "every 2 weeks on Mon, Wed, 10 times");
        let start = When::parse("20250303", None).unwrap();
        assert_eq!(
            r.to_rrule(&start),
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=10"
        );

        // things rical does not model are left alone
        assert_eq!(Rule::parse("FREQ=MONTHLY;BYDAY=1TU"), None);
        assert_eq!(Rule::parse("FREQ=YEARLY;BYMONTH=3"), None);
        assert_eq!(Rule::parse("FREQ=HOURLY"), None);
        assert_eq!(Rule::parse("INTERVAL=2"), None);
    }

    #[test]
    fn until_forms() {
        let mut r = Rule::new(Freq::Daily);
        r.end = End::Until(d("2025-03-31"));
        let date = When::parse("20250301", None).unwrap();
        assert_eq!(r.to_rrule(&date), "FREQ=DAILY;UNTIL=20250331");
        let london = When::parse("20250701T090000", Some("Europe/London")).unwrap();
        // 23:59:59 BST is 22:59:59 UTC
        r.end = End::Until(d("2025-07-31"));
        assert_eq!(r.to_rrule(&london), "FREQ=DAILY;UNTIL=20250731T225959Z");
        let floating = When::parse("20250701T090000", None).unwrap();
        assert_eq!(r.to_rrule(&floating), "FREQ=DAILY;UNTIL=20250731T235959");
        assert_eq!(
            Rule::parse("FREQ=DAILY;UNTIL=20250731T225959Z")
                .unwrap()
                .end,
            End::Until(d("2025-07-31"))
        );
        assert_eq!(r.to_string(), "every day, until 31 Jul 2025");
    }

    #[test]
    fn weekdays() {
        assert_eq!(parse_weekday("Monday"), Some(Weekday::Mon));
        assert_eq!(parse_weekday("th"), Some(Weekday::Thu));
        assert_eq!(parse_weekday("1MO"), None);
        assert_eq!(parse_weekday("x"), None);
    }
}

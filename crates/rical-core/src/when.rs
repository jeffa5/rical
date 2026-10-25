//! Date and time values as iCalendar models them.
//!
//! Values keep their original form (date, floating or zoned) and are only
//! converted to a concrete instant at the display edge, given the viewer's
//! timezone.

use std::{fmt, str::FromStr};

use crate::compact::Prop as Property;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone as _, Utc};
use chrono_tz::Tz;
use log::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum When {
    /// An all-day value, `VALUE=DATE`.
    Date(NaiveDate),
    /// Wall-clock time in whatever timezone the viewer is in.
    Floating(NaiveDateTime),
    /// A fixed instant, either UTC (`Z`) or with a `TZID`.
    Zoned(DateTime<Tz>),
}

impl When {
    /// Parse a single iCalendar DATE or DATE-TIME value.
    ///
    /// Unknown `TZID`s fall back to floating time with a warning.
    pub fn parse(value: &str, tzid: Option<&str>) -> Option<Self> {
        let value = value.trim();
        if value.len() == 8 {
            return NaiveDate::parse_from_str(value, "%Y%m%d")
                .ok()
                .map(Self::Date);
        }

        if let Some(utc) = value.strip_suffix('Z') {
            let ndt = NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok()?;
            return Some(Self::Zoned(Tz::UTC.from_utc_datetime(&ndt)));
        }

        let ndt = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
        match tzid {
            None => Some(Self::Floating(ndt)),
            Some(tzid) => match parse_tzid(tzid) {
                Some(tz) => Some(Self::Zoned(resolve_local(tz, ndt))),
                None => {
                    warn!("unknown TZID {tzid:?}, treating {value:?} as floating");
                    Some(Self::Floating(ndt))
                }
            },
        }
    }

    /// Parse the (first) value of a property, honouring its `TZID`.
    pub fn from_property(prop: &Property) -> Option<Self> {
        let first = prop.value().split(',').next()?;
        let start = first.split('/').next().unwrap_or(first);
        let when = Self::parse(start, prop.param("TZID"));
        if when.is_none() {
            warn!("invalid {} value {first:?}", prop.name);
        }
        when
    }

    /// Parse every comma-separated value of a property such as `EXDATE` or
    /// `RDATE`. For `PERIOD` values only the start is kept.
    pub fn list_from_property(prop: &Property) -> Vec<Self> {
        let tzid = prop.param("TZID");
        prop.value()
            .split(',')
            .filter_map(|v| {
                let start = v.split('/').next().unwrap_or(v);
                let when = Self::parse(start, tzid);
                if when.is_none() {
                    warn!("invalid {} value {v:?}", prop.name);
                }
                when
            })
            .collect()
    }

    pub fn is_date(&self) -> bool {
        matches!(self, Self::Date(_))
    }

    /// The instant this value refers to for a viewer in `local`. Dates are
    /// midnight at the start of the day.
    pub fn to_tz(&self, local: Tz) -> DateTime<Tz> {
        match self {
            Self::Date(d) => resolve_local(local, d.and_time(NaiveTime::MIN)),
            Self::Floating(ndt) => resolve_local(local, *ndt),
            Self::Zoned(dt) => dt.with_timezone(&local),
        }
    }

    pub fn to_utc(&self, local: Tz) -> DateTime<Utc> {
        self.to_tz(local).with_timezone(&Utc)
    }

    /// The calendar date this value falls on for a viewer in `local`.
    pub fn date(&self, local: Tz) -> NaiveDate {
        match self {
            Self::Date(d) => *d,
            _ => self.to_tz(local).date_naive(),
        }
    }

    /// Convert `self` into the same form as `like` so the two can be
    /// compared, e.g. an `EXDATE` against the `DTSTART` it excludes.
    pub fn coerce_like(&self, like: &When) -> When {
        match (like, self) {
            (Self::Date(_), Self::Date(_)) => *self,
            (Self::Date(_), Self::Floating(ndt)) => Self::Date(ndt.date()),
            (Self::Date(_), Self::Zoned(dt)) => Self::Date(dt.date_naive()),
            (Self::Floating(_), Self::Date(d)) => Self::Floating(d.and_time(NaiveTime::MIN)),
            (Self::Floating(_), Self::Floating(_)) => *self,
            (Self::Floating(_), Self::Zoned(dt)) => Self::Floating(dt.naive_local()),
            (Self::Zoned(like), _) => Self::Zoned(self.to_tz(like.timezone())),
        }
    }

    /// Shift by `delta`. Dates move by whole days; zoned values by exact
    /// elapsed time.
    pub fn add(&self, delta: TimeDelta) -> When {
        match self {
            Self::Date(d) => Self::Date(
                d.checked_add_signed(TimeDelta::days(delta.num_days()))
                    .unwrap_or(*d),
            ),
            Self::Floating(ndt) => Self::Floating(*ndt + delta),
            Self::Zoned(dt) => Self::Zoned(*dt + delta),
        }
    }

    /// Elapsed time from `self` to `other`, comparing like with like where
    /// possible. Never negative.
    pub fn until(&self, other: &When) -> TimeDelta {
        let other = other.coerce_like(self);
        let d = other.to_utc(Tz::UTC) - self.to_utc(Tz::UTC);
        d.max(TimeDelta::zero())
    }
}

impl fmt::Display for When {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date(d) => write!(f, "{}", d.format("%Y-%m-%d")),
            Self::Floating(ndt) => write!(f, "{}", ndt.format("%Y-%m-%d %H:%M")),
            Self::Zoned(dt) => write!(f, "{} {}", dt.format("%Y-%m-%d %H:%M"), dt.timezone()),
        }
    }
}

/// Map a local wall-clock time to an instant, picking the earlier instant in
/// a DST overlap and skipping forward over a DST gap.
pub fn resolve_local(tz: Tz, ndt: NaiveDateTime) -> DateTime<Tz> {
    if let Some(dt) = tz.from_local_datetime(&ndt).earliest() {
        return dt;
    }
    // In a gap: find the first valid minute after it.
    (1..=24 * 60)
        .find_map(|m| {
            tz.from_local_datetime(&(ndt + TimeDelta::minutes(m)))
                .earliest()
        })
        .unwrap_or_else(|| tz.from_utc_datetime(&ndt))
}

/// Parse a `TZID`, allowing vendor prefixes such as
/// `/mozilla.org/20050126_1/Europe/London`.
pub fn parse_tzid(tzid: &str) -> Option<Tz> {
    let tzid = tzid.trim_matches('"');
    if let Ok(tz) = Tz::from_str(tzid) {
        return Some(tz);
    }
    tzid.match_indices('/')
        .find_map(|(i, _)| Tz::from_str(&tzid[i + 1..]).ok())
}

/// The system timezone, falling back to UTC.
pub fn local_timezone() -> Tz {
    std::env::var("TZ")
        .ok()
        .and_then(|tz| parse_tzid(tz.trim_start_matches(':')))
        .or_else(|| {
            iana_time_zone::get_timezone()
                .ok()
                .and_then(|tz| parse_tzid(&tz))
        })
        .unwrap_or(Tz::UTC)
}

/// Parse an iCalendar `DURATION` such as `PT1H30M`, `P1D` or `-P2W`.
pub fn parse_duration(s: &str) -> Option<TimeDelta> {
    let s = s.trim();
    let (negative, s) = match s.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix('P')?;

    let mut total = TimeDelta::zero();
    let mut in_time = false;
    let mut num = String::new();
    for c in s.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            unit => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match (unit, in_time) {
                    ('W', false) => TimeDelta::weeks(n),
                    ('D', false) => TimeDelta::days(n),
                    ('H', true) => TimeDelta::hours(n),
                    ('M', true) => TimeDelta::minutes(n),
                    ('S', true) => TimeDelta::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    if !num.is_empty() {
        return None;
    }
    Some(if negative { -total } else { total })
}

#[cfg(test)]
mod tests {
    use chrono::{NaiveDate, Timelike};
    use chrono_tz::Tz;

    use super::*;

    fn ndt(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y%m%dT%H%M%S").unwrap()
    }

    #[test]
    fn parse_forms() {
        assert_eq!(
            When::parse("20250304", None),
            Some(When::Date(NaiveDate::from_ymd_opt(2025, 3, 4).unwrap()))
        );
        assert_eq!(
            When::parse("20250304T140000", None),
            Some(When::Floating(ndt("20250304T140000")))
        );
        let utc = When::parse("20250304T140000Z", None).unwrap();
        assert_eq!(utc.to_utc(Tz::UTC).hour(), 14);

        let london = When::parse("20250704T140000", Some("Europe/London")).unwrap();
        assert_eq!(london.to_utc(Tz::UTC).hour(), 13, "BST is UTC+1");

        let prefixed = When::parse(
            "20250704T140000",
            Some("/mozilla.org/20050126_1/Europe/London"),
        );
        assert_eq!(prefixed, Some(london));

        assert_eq!(
            When::parse("20250704T140000", Some("Mars/Olympus")),
            Some(When::Floating(ndt("20250704T140000")))
        );
        assert_eq!(When::parse("nope", None), None);
    }

    #[test]
    fn floating_follows_viewer() {
        let w = When::Floating(ndt("20250704T090000"));
        assert_eq!(w.to_utc(Tz::Europe__London).hour(), 8);
        assert_eq!(w.to_utc(Tz::Asia__Tokyo).hour(), 0);
    }

    #[test]
    fn dst_gap_and_overlap() {
        // 01:30 does not exist in London on 2025-03-30
        let gap = resolve_local(Tz::Europe__London, ndt("20250330T013000"));
        assert_eq!(gap.naive_local(), ndt("20250330T020000"));

        // 01:30 happens twice on 2025-10-26; take the first (BST)
        let overlap = resolve_local(Tz::Europe__London, ndt("20251026T013000"));
        assert_eq!(overlap.with_timezone(&Utc).hour(), 0);
    }

    #[test]
    fn coerce() {
        let start = When::parse("20250704T140000", Some("Europe/London")).unwrap();
        let exdate_utc = When::parse("20250704T130000Z", None).unwrap();
        assert_eq!(exdate_utc.coerce_like(&start), start);

        let day = When::parse("20250704", None).unwrap();
        assert_eq!(start.coerce_like(&day), day);
    }

    #[test]
    fn add_and_until() {
        let d = When::parse("20250228", None).unwrap();
        assert_eq!(
            d.add(TimeDelta::days(1)),
            When::parse("20250301", None).unwrap()
        );

        // Across the spring-forward change exact elapsed time is kept.
        let start = When::parse("20250329T120000", Some("Europe/London")).unwrap();
        let next = start.add(TimeDelta::hours(24));
        assert_eq!(
            next,
            When::parse("20250330T130000", Some("Europe/London")).unwrap()
        );
        assert_eq!(start.until(&next), TimeDelta::hours(24));
        assert_eq!(next.until(&start), TimeDelta::zero());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT15M"), Some(TimeDelta::minutes(15)));
        assert_eq!(
            parse_duration("P1DT2H30M"),
            Some(TimeDelta::days(1) + TimeDelta::hours(2) + TimeDelta::minutes(30))
        );
        assert_eq!(parse_duration("-P2W"), Some(-TimeDelta::weeks(2)));
        assert_eq!(parse_duration("P1H"), None);
        assert_eq!(parse_duration("PT5"), None);
    }
}

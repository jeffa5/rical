//! Parsing what people type for dates and durations, shared by the CLI and
//! the GUI command line.

use chrono::{Datelike, Days, Months, NaiveDate, NaiveTime, TimeDelta, Weekday};
use chrono_tz::Tz;

use crate::when::{When, resolve_local};

/// Parse a date relative to `today`: "2025-03-04", "03-04", "4" (day of
/// this month), "today", "tomorrow", "yesterday", a weekday ("fri": the
/// next one, today included) or an offset ("+3d", "-1w", "+2m", "+1y").
pub fn parse_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let t = text.trim().to_lowercase();
    match t.as_str() {
        "today" | "now" => return Some(today),
        "tomorrow" => return today.succ_opt(),
        "yesterday" => return today.pred_opt(),
        _ => {}
    }
    if let Ok(wd) = t.parse::<Weekday>() {
        let ahead = (7 + wd.num_days_from_monday() - today.weekday().num_days_from_monday()) % 7;
        return Some(today + Days::new(ahead as u64));
    }
    if let Some(sign) = t.chars().next().filter(|c| matches!(c, '+' | '-')) {
        let body = &t[1..];
        let (n, unit) = body.split_at(
            body.find(|c: char| !c.is_ascii_digit())
                .unwrap_or(body.len()),
        );
        let n: u32 = n.parse().ok()?;
        let forward = sign == '+';
        return match unit {
            "" | "d" => shift_days(today, n as u64, forward),
            "w" => shift_days(today, n as u64 * 7, forward),
            "m" => shift_months(today, n, forward),
            "y" => shift_months(today, n * 12, forward),
            _ => None,
        };
    }
    NaiveDate::parse_from_str(&t, "%Y-%m-%d")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(&format!("{}-{t}", today.year()), "%Y-%m-%d").ok())
        .or_else(|| t.parse::<u32>().ok().and_then(|d| today.with_day(d)))
}

fn shift_days(d: NaiveDate, n: u64, forward: bool) -> Option<NaiveDate> {
    if forward {
        d.checked_add_days(Days::new(n))
    } else {
        d.checked_sub_days(Days::new(n))
    }
}

fn shift_months(d: NaiveDate, n: u32, forward: bool) -> Option<NaiveDate> {
    if forward {
        d.checked_add_months(Months::new(n))
    } else {
        d.checked_sub_months(Months::new(n))
    }
}

/// Parse a signed duration such as "+15m", "-1h30m", "2d" or "1w".
pub fn parse_shift(text: &str) -> Option<TimeDelta> {
    let t = text.trim();
    let (negative, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if body.is_empty() {
        return None;
    }
    let mut total = TimeDelta::zero();
    let mut num = String::new();
    for c in body.chars() {
        match c {
            '0'..='9' => num.push(c),
            'm' | 'h' | 'd' | 'w' => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match c {
                    'm' => TimeDelta::minutes(n),
                    'h' => TimeDelta::hours(n),
                    'd' => TimeDelta::days(n),
                    _ => TimeDelta::weeks(n),
                };
            }
            _ => return None,
        }
    }
    if !num.is_empty() {
        return None;
    }
    Some(if negative { -total } else { total })
}

/// "[date] [HH:MM]": a date alone means all day; a time alone is on the
/// focused day.
pub fn parse_at(text: &str, focused: NaiveDate, today: NaiveDate, local: Tz) -> Option<When> {
    let mut date = None;
    let mut time = None;
    for word in text.split_whitespace() {
        if let Ok(t) = NaiveTime::parse_from_str(word, "%H:%M") {
            time = Some(t);
        } else {
            date = Some(parse_date(word, today)?);
        }
    }
    match (date, time) {
        (Some(d), None) => Some(When::Date(d)),
        (d, Some(t)) => Some(When::Zoned(resolve_local(
            local,
            d.unwrap_or(focused).and_time(t),
        ))),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn dates() {
        let today = d("2025-10-23"); // Thursday
        for (text, want) in [
            ("today", "2025-10-23"),
            ("tomorrow", "2025-10-24"),
            ("thu", "2025-10-23"),
            ("mon", "2025-10-27"),
            ("Friday", "2025-10-24"),
            ("+3", "2025-10-26"),
            ("+1w", "2025-10-30"),
            ("-2d", "2025-10-21"),
            ("+1m", "2025-11-23"),
            ("2026-01-02", "2026-01-02"),
            ("12-25", "2025-12-25"),
            ("5", "2025-10-05"),
        ] {
            assert_eq!(parse_date(text, today), Some(d(want)), "{text}");
        }
        assert_eq!(parse_date("soon", today), None);
        assert_eq!(parse_date("32", today), None);
    }

    #[test]
    fn shifts() {
        assert_eq!(parse_shift("+15m"), Some(TimeDelta::minutes(15)));
        assert_eq!(parse_shift("-1h30m"), Some(-TimeDelta::minutes(90)));
        assert_eq!(parse_shift("2d"), Some(TimeDelta::days(2)));
        assert_eq!(parse_shift("+"), None);
        assert_eq!(parse_shift("5"), None);
    }

    #[test]
    fn at() {
        let focused = d("2025-03-04");
        let today = d("2025-03-01");
        let tz = chrono_tz::Tz::UTC;
        let p = |s| parse_at(s, focused, today, tz).map(|w| w.to_string());
        assert_eq!(p("14:30").as_deref(), Some("2025-03-04 14:30 UTC"));
        assert_eq!(p("tomorrow 09:00").as_deref(), Some("2025-03-02 09:00 UTC"));
        assert_eq!(p("fri").as_deref(), Some("2025-03-07"));
        assert_eq!(p("whenever"), None);
    }
}

//! Quick add: an event or task from a line of plain language, such as
//! "Lunch with Sam at Bistro 21 tomorrow 1pm for 90m" or
//! "Submit expenses by fri 5pm !". What is not understood is the title.
//!
//! - Dates: today, tomorrow, weekdays ("fri", "next fri": the one in next
//!   week), "14 nov", "nov 14th", "14/11" (day first), "2026-11-14",
//!   "in 3 days", "next week"; ranges "3-5 dec", "mon to wed". A date in
//!   the past without a year is next year's.
//! - Times: "9am", "9:30pm", "21:30", noon, midnight; "at 5" (1 to 6 o'clock
//!   without am or pm are afternoon); ranges "9-10", "2-4pm",
//!   "from 9 to 11:30", "until 5pm"; a length "for 45m", "for 1h30",
//!   "for 2 hours", "for 3 days".
//! - Zones: "London time", "UK time", "Europe/London", SGT, BST, CET, EST…
//! - Place: "@ The Mill" or "at The Mill" (at something that is not a time).
//! - Calendar: "#work". Priority: "!" or "urgent" (high), "!5", "low
//!   priority". Repeats: "daily", "every weekday", "every mon and thu",
//!   "every 2 weeks", "monthly", "every year".
//! - Tasks: "due fri", "by fri 5pm" (any date is the due date), "starts mon".
//! - "Quoted text" is kept as it is, in the title.

use chrono::{Datelike, Days, Months, NaiveDate, NaiveTime, TimeDelta, Weekday};
use chrono_tz::Tz;

use crate::{
    Kind, When,
    when::resolve_local,
    write::{Draft, Repeat},
};

impl Quick {
    /// A draft to create from it: an event, or with `task` a task. An event
    /// with no time given starts at `default_start` for an hour (a day, if
    /// that is a date).
    pub fn draft(&self, task: bool, default_start: When) -> Draft {
        let day = TimeDelta::days(1);
        let mut d = if task {
            // a task's dates are optional; start/end hold them as for an
            // event, the end exclusive (due on a day covers that day)
            let end = self
                .due
                .map(|due| if due.is_date() { due.add(day) } else { due });
            let start = self.start.or(self.due).unwrap_or(default_start);
            let mut d = Draft::new(start, end.unwrap_or(start));
            d.kind = Kind::Todo;
            d.has_start = self.start.is_some();
            d.has_due = self.due.is_some();
            d.priority = self.priority;
            d
        } else {
            let start = self.start.unwrap_or(default_start);
            let end = self.end.unwrap_or_else(|| {
                start.add(if start.is_date() {
                    day
                } else {
                    TimeDelta::hours(1)
                })
            });
            Draft::new(start, end)
        };
        d.summary = self.title.clone();
        d.location = self.location.clone().unwrap_or_default();
        d.repeat = Repeat::from_rrule(self.repeat.as_deref());
        // times said in a zone are written in it
        d.exact_zones = self.zone.is_some();
        d
    }
}

/// What a line says.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Quick {
    pub title: String,
    /// For an event: when it starts and (exclusive) ends. For a task: its
    /// start date, if it says one.
    pub start: Option<When>,
    pub end: Option<When>,
    /// For a task: when it is due.
    pub due: Option<When>,
    pub location: Option<String>,
    /// A calendar named with #.
    pub calendar: Option<String>,
    /// 1 (highest) to 9.
    pub priority: Option<u8>,
    /// An RRULE value, e.g. "FREQ=WEEKLY;BYDAY=MO,TH".
    pub repeat: Option<String>,
    /// The zone its times are in, if it names one.
    pub zone: Option<Tz>,
}

#[derive(Debug, Clone)]
struct Tok {
    text: String,
    /// Lower case, without trailing commas or full stops.
    word: String,
    quoted: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Piece {
    Date(NaiveDate),
    Dates(NaiveDate, NaiveDate),
    Time(NaiveTime),
    Times(NaiveTime, NaiveTime),
    Until(NaiveTime),
    Length(TimeDelta),
    Zone(Tz),
    Place(String),
    Calendar(String),
    Priority(u8),
    /// An RRULE and the weekdays it is on, if it says.
    Repeat(String, Vec<Weekday>),
    /// due/by: a date and maybe a time.
    Due(Option<NaiveDate>, Option<NaiveTime>),
    /// starts/starting: a date.
    Start(NaiveDate),
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut rest = text.trim();
    while !rest.is_empty() {
        if let Some(q) = rest.strip_prefix('"')
            && let Some(close) = q.find('"')
        {
            out.push(Tok {
                text: q[..close].to_owned(),
                word: String::new(),
                quoted: true,
            });
            rest = q[close + 1..].trim_start();
            continue;
        }
        let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let text = &rest[..end];
        out.push(Tok {
            text: text.to_owned(),
            word: text
                .trim_end_matches([',', '.', ';'])
                .to_lowercase()
                .replace(['–', '—'], "-"),
            quoted: false,
        });
        rest = rest[end..].trim_start();
    }
    out
}

fn weekday(w: &str) -> Option<Weekday> {
    Some(match w {
        "mon" | "monday" | "mondays" => Weekday::Mon,
        "tue" | "tues" | "tuesday" | "tuesdays" => Weekday::Tue,
        "wed" | "weds" | "wednesday" | "wednesdays" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" | "thursdays" => Weekday::Thu,
        "fri" | "friday" | "fridays" => Weekday::Fri,
        "sat" | "saturday" | "saturdays" => Weekday::Sat,
        "sun" | "sunday" | "sundays" => Weekday::Sun,
        _ => return None,
    })
}

fn month(w: &str) -> Option<u32> {
    const NAMES: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    // a name or its first three letters or more: "nov", "sept", "november"
    let w = w.trim_end_matches('.');
    if w.len() < 3 {
        return None;
    }
    NAMES
        .iter()
        .position(|n| n.starts_with(w))
        .map(|i| i as u32 + 1)
}

/// "14", "14th", "1st", "22nd", "3rd".
fn day_number(w: &str) -> Option<u32> {
    let digits = w
        .strip_suffix("st")
        .or_else(|| w.strip_suffix("nd"))
        .or_else(|| w.strip_suffix("rd"))
        .or_else(|| w.strip_suffix("th"))
        .unwrap_or(w);
    let n: u32 = digits.parse().ok()?;
    (1..=31).contains(&n).then_some(n)
}

fn year_number(w: &str) -> Option<i32> {
    let n: i32 = w.parse().ok()?;
    (1900..=2200).contains(&n).then_some(n)
}

/// The next `wd` from `today`, today included.
fn next_weekday(today: NaiveDate, wd: Weekday) -> NaiveDate {
    let ahead = (wd.num_days_from_monday() + 7 - today.weekday().num_days_from_monday()) % 7;
    today + Days::new(ahead as u64)
}

/// A day and month in the coming year: next year's if this year's is past.
fn coming(today: NaiveDate, m: u32, d: u32, year: Option<i32>) -> Option<NaiveDate> {
    match year {
        Some(y) => NaiveDate::from_ymd_opt(y, m, d),
        None => {
            let this = NaiveDate::from_ymd_opt(today.year(), m, d)?;
            if this < today {
                NaiveDate::from_ymd_opt(today.year() + 1, m, d)
            } else {
                Some(this)
            }
        }
    }
}

/// A date at `t[i..]`: how many tokens it takes, and the date.
fn date_at(t: &[Tok], i: usize, today: NaiveDate) -> Option<(usize, NaiveDate)> {
    let w = |k: usize| t.get(i + k).filter(|t| !t.quoted).map(|t| t.word.as_str());
    let w0 = w(0)?;
    match w0 {
        "today" | "tonight" => return Some((1, today)),
        "tomorrow" | "tmr" | "tmrw" | "tomorrow's" => return Some((1, today.succ_opt()?)),
        "yesterday" => return Some((1, today.pred_opt()?)),
        _ => {}
    }
    if let Some(wd) = weekday(w0) {
        return Some((1, next_weekday(today, wd)));
    }
    // next fri: in next week; next week: its Monday
    if w0 == "next" {
        let w1 = w(1)?;
        if let Some(wd) = weekday(w1) {
            let monday = today - Days::new(today.weekday().num_days_from_monday() as u64);
            return Some((2, next_weekday(monday + Days::new(7), wd)));
        }
        if w1 == "week" {
            let monday = today - Days::new(today.weekday().num_days_from_monday() as u64);
            return Some((2, monday + Days::new(7)));
        }
        if w1 == "month" {
            return Some((2, today.with_day(1)?.checked_add_months(Months::new(1))?));
        }
        return None;
    }
    // in 3 days / weeks / months
    if w0 == "in"
        && let Some(n) = w(1).and_then(|n| match n {
            "a" | "an" | "one" => Some(1),
            n => n.parse::<u32>().ok(),
        })
    {
        let unit = w(2)?;
        let d = match unit.trim_end_matches('s') {
            "day" => today.checked_add_days(Days::new(n as u64))?,
            "week" => today.checked_add_days(Days::new(7 * n as u64))?,
            "month" => today.checked_add_months(Months::new(n))?,
            "year" => today.checked_add_months(Months::new(n.checked_mul(12)?))?,
            _ => return None,
        };
        return Some((3, d));
    }
    // 2026-11-14
    if let Ok(d) = NaiveDate::parse_from_str(w0, "%Y-%m-%d") {
        return Some((1, d));
    }
    // 14/11, 14/11/2027 (day first)
    if w0.contains('/') {
        let parts = w0.split('/').collect::<Vec<_>>();
        let (d, m) = (parts.first()?.parse().ok()?, parts.get(1)?.parse().ok()?);
        let y = match parts.get(2) {
            Some(y) if y.len() == 2 => Some(2000 + y.parse::<i32>().ok()?),
            Some(y) => Some(y.parse().ok()?),
            None => None,
        };
        return Some((1, coming(today, m, d, y)?));
    }
    // the 14th
    if w0 == "the"
        && let Some(d) = w(1)
            .filter(|w| w.ends_with(['h', 't', 'd']))
            .and_then(day_number)
    {
        let this = today.with_day(d);
        let date = match this {
            Some(x) if x >= today => x,
            _ => today
                .with_day(1)?
                .checked_add_months(Months::new(1))?
                .with_day(d)?,
        };
        return Some((2, date));
    }
    // 14 nov [2027] / 14th of november
    if let Some(d) = day_number(w0) {
        let (k, m) = match (w(1), w(2)) {
            (Some("of"), Some(m)) => (3, month(m)?),
            (Some(m), _) => (2, month(m)?),
            _ => return None,
        };
        let y = w(k).and_then(year_number);
        return Some((k + y.is_some() as usize, coming(today, m, d, y)?));
    }
    // nov 14 [2027]
    if let Some(m) = month(w0) {
        let d = day_number(w(1)?)?;
        let y = w(2).and_then(year_number);
        return Some((2 + y.is_some() as usize, coming(today, m, d, y)?));
    }
    None
}

/// A date range at `t[i..]`: "3-5 dec", "dec 3-5", "mon to wed",
/// "3 dec to 5 dec", "from 3 dec until 5 dec".
fn dates_at(t: &[Tok], i: usize, today: NaiveDate) -> Option<(usize, NaiveDate, NaiveDate)> {
    let w = |k: usize| t.get(i + k).filter(|t| !t.quoted).map(|t| t.word.as_str());
    // 3-5 dec / dec 3-5
    let span = |s: &str| {
        let (a, b) = s.split_once('-')?;
        Some((day_number(a)?, day_number(b)?))
    };
    if let Some((a, b)) = w(0).and_then(span)
        && let Some(m) = w(1).and_then(month)
    {
        let y = w(2).and_then(year_number);
        let start = coming(today, m, a, y)?;
        let end = NaiveDate::from_ymd_opt(start.year(), m, b)?;
        return (end >= start).then_some((2 + y.is_some() as usize, start, end));
    }
    if let Some(m) = w(0).and_then(month)
        && let Some((a, b)) = w(1).and_then(span)
    {
        let y = w(2).and_then(year_number);
        let start = coming(today, m, a, y)?;
        let end = NaiveDate::from_ymd_opt(start.year(), m, b)?;
        return (end >= start).then_some((2 + y.is_some() as usize, start, end));
    }
    // [from] A to|until|- B
    let skip = usize::from(w(0) == Some("from"));
    let (n1, a) = date_at(t, i + skip, today)?;
    let joiner = w(skip + n1)?;
    if !matches!(joiner, "to" | "until" | "till" | "-" | "through" | "thru") {
        return None;
    }
    let (n2, mut b) = date_at(t, i + skip + n1 + 1, a)?;
    // "fri to mon" runs into next week
    while b < a {
        b = b.checked_add_days(Days::new(7))?;
    }
    Some((skip + n1 + 1 + n2, a, b))
}

/// A clock time in one word: "9am", "9:30pm", "21:30", "9.30", "noon".
/// `bare` allows a lone hour ("9"), read as afternoon from 1 to 6.
fn time_word(w: &str, bare: bool) -> Option<(NaiveTime, bool)> {
    match w {
        "noon" | "midday" => return Some((NaiveTime::from_hms_opt(12, 0, 0)?, true)),
        "midnight" => return Some((NaiveTime::MIN, true)),
        _ => {}
    }
    let (body, ampm) = if let Some(b) = w.strip_suffix("am").or_else(|| w.strip_suffix("a.m")) {
        (b, Some(false))
    } else if let Some(b) = w.strip_suffix("pm").or_else(|| w.strip_suffix("p.m")) {
        (b, Some(true))
    } else {
        (w, None)
    };
    let (h, m, has_minutes) = match body.split_once([':', '.']) {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?, true),
        Some(_) => return None,
        None => (body.parse::<u32>().ok()?, 0, false),
    };
    if body.is_empty() || m > 59 {
        return None;
    }
    let h = match ampm {
        Some(pm) if (1..=12).contains(&h) => h % 12 + if pm { 12 } else { 0 },
        Some(_) => return None,
        None if has_minutes && h <= 23 => h,
        None if bare && (1..=6).contains(&h) => h + 12,
        None if bare && h <= 23 => h,
        None => return None,
    };
    Some((NaiveTime::from_hms_opt(h, m, 0)?, ampm.is_some()))
}

/// A time or a time range at `t[i..]`.
fn times_at(t: &[Tok], i: usize) -> Option<(usize, Piece)> {
    let w = |k: usize| t.get(i + k).filter(|t| !t.quoted).map(|t| t.word.as_str());
    let mut k = 0;
    let lead = w(0)?;
    let led = matches!(lead, "at" | "from" | "@");
    if led {
        k = 1;
    }
    if lead == "until" || lead == "till" || lead == "til" {
        let (end, _) = time_word(w(1)?, true)?;
        return Some((2, Piece::Until(end)));
    }
    let first = w(k)?;
    // "9-10", "2-4pm", "9am-10:30am"
    if let Some((a, b)) = first.split_once('-')
        && !a.is_empty()
        && !b.is_empty()
    {
        let (end, end_marked) = time_word(b, true)?;
        let (mut start, start_marked) = time_word(a, true)?;
        // "2-4pm": the second's am/pm goes for the first too, if it fits
        if !start_marked && end_marked && end.hour() >= 12 {
            let pm = NaiveTime::from_hms_opt(start.hour() % 12 + 12, start.minute(), 0)?;
            if pm < end {
                start = pm;
            } else if start.hour() > 12 {
                start = NaiveTime::from_hms_opt(start.hour() - 12, start.minute(), 0)?;
            }
        }
        return Some((k + 1, Piece::Times(start, end)));
    }
    // "9 am" as two words
    let (start, marked) = match w(k + 1) {
        Some(ap @ ("am" | "pm")) => {
            let (t0, _) = time_word(&format!("{first}{ap}"), true)?;
            k += 1;
            (t0, true)
        }
        // a lone number is only a time after "at" or "from", or before a
        // range word
        _ => time_word(
            first,
            led || matches!(w(k + 1), Some("to" | "until" | "till" | "-")),
        )?,
    };
    // a range: A to B / A until B / A - B
    if let Some("to" | "until" | "till" | "-") = w(k + 1)
        && let Some((end, end_marked)) = w(k + 2).and_then(|b| time_word(b, true))
    {
        let mut start = start;
        if !marked && end_marked && end.hour() >= 12 {
            let pm = NaiveTime::from_hms_opt(start.hour() % 12 + 12, start.minute(), 0)?;
            if pm < end {
                start = pm;
            }
        }
        return Some((k + 3, Piece::Times(start, end)));
    }
    Some((k + 1, Piece::Time(start)))
}

use chrono::Timelike as _;

/// "45m", "1h30", "1h30m", "2h", "90min", "1.5h", or a number then a
/// unit word ("2 hours", "30 minutes", "3 days", "half an hour").
fn length_at(t: &[Tok], i: usize) -> Option<(usize, TimeDelta)> {
    let w = |k: usize| t.get(i + k).filter(|t| !t.quoted).map(|t| t.word.as_str());
    let w0 = w(0)?;
    if w0 == "half" && w(1) == Some("an") && w(2).is_some_and(|u| u.starts_with("hour")) {
        return Some((3, TimeDelta::minutes(30)));
    }
    if matches!(w0, "an" | "a" | "one") {
        return unit_length(w(1)?, 1.0).map(|d| (2, d));
    }
    if let Ok(n) = w0.parse::<f64>() {
        return unit_length(w(1)?, n).map(|d| (2, d));
    }
    // compact: 1h30m, 45m, 2d, 1.5h
    let mut total = TimeDelta::zero();
    let mut num = String::new();
    let mut any = false;
    for c in w0.chars() {
        if c.is_ascii_digit() || c == '.' {
            num.push(c);
            continue;
        }
        let n: f64 = num.parse().ok()?;
        num.clear();
        let minutes = match c {
            'w' => n * 7.0 * 24.0 * 60.0,
            'd' => n * 24.0 * 60.0,
            'h' => n * 60.0,
            'm' => n,
            _ => return None,
        };
        // no length longer than about 19 years (found by fuzzing)
        if !(0.0..1e7).contains(&minutes) {
            return None;
        }
        total = total.checked_add(&TimeDelta::minutes(minutes.round() as i64))?;
        any = true;
    }
    // "1h30": minutes after hours without the m
    if !num.is_empty() {
        if !any {
            return None;
        }
        let minutes = num.parse::<f64>().ok()?;
        if !(0.0..60.0).contains(&minutes) {
            return None;
        }
        total = total.checked_add(&TimeDelta::minutes(minutes.round() as i64))?;
    }
    (any && total > TimeDelta::zero()).then_some((1, total))
}

fn unit_length(unit: &str, n: f64) -> Option<TimeDelta> {
    let minutes = match unit.trim_end_matches('s') {
        "minute" | "min" | "mins" | "m" => n,
        "hour" | "hr" | "h" => n * 60.0,
        "day" | "d" => n * 24.0 * 60.0,
        "week" | "wk" => n * 7.0 * 24.0 * 60.0,
        _ => return None,
    };
    (minutes > 0.0 && minutes < 1e7).then(|| TimeDelta::minutes(minutes.round() as i64))
}

/// A time zone: "London time", "UK time", "New York time", "Europe/London",
/// or a common abbreviation.
fn zone_at(t: &[Tok], i: usize) -> Option<(usize, Tz)> {
    let w = |k: usize| t.get(i + k).filter(|t| !t.quoted).map(|t| t.word.as_str());
    let w0 = w(0)?;
    let abbreviation = match w0.trim_matches(['(', ')']) {
        "utc" | "z" => Some(Tz::UTC),
        "gmt" => Some(Tz::Etc__GMT),
        "bst" => Some(Tz::Europe__London),
        "sgt" => Some(Tz::Asia__Singapore),
        "cet" | "cest" => Some(Tz::Europe__Paris),
        "eet" | "eest" => Some(Tz::Europe__Athens),
        "est" | "edt" | "et" => Some(Tz::America__New_York),
        "cst" | "cdt" | "ct" => Some(Tz::America__Chicago),
        "mst" | "mdt" | "mt" => Some(Tz::America__Denver),
        "pst" | "pdt" | "pt" => Some(Tz::America__Los_Angeles),
        "jst" => Some(Tz::Asia__Tokyo),
        "hkt" => Some(Tz::Asia__Hong_Kong),
        "ist" => Some(Tz::Asia__Kolkata),
        "aest" | "aedt" => Some(Tz::Australia__Sydney),
        "nzst" | "nzdt" => Some(Tz::Pacific__Auckland),
        _ => None,
    };
    if let Some(tz) = abbreviation {
        return Some((1, tz));
    }
    if w0.contains('/')
        && let Ok(tz) = t[i].text.trim_end_matches([',', '.']).parse::<Tz>()
    {
        return Some((1, tz));
    }
    // "[in] <place> time": a zone's city, or a country with one zone
    let skip = usize::from(w0 == "in");
    for n in 1..=3 {
        if w(skip + n) != Some("time") {
            continue;
        }
        let place = (0..n)
            .map(|k| w(skip + k))
            .collect::<Option<Vec<_>>>()?
            .join(" ");
        let tz = match place.as_str() {
            "uk" | "british" | "england" | "scotland" | "wales" => Some(Tz::Europe__London),
            "singapore" | "sg" => Some(Tz::Asia__Singapore),
            "us eastern" | "eastern" => Some(Tz::America__New_York),
            "us pacific" | "pacific" => Some(Tz::America__Los_Angeles),
            "japan" => Some(Tz::Asia__Tokyo),
            "india" => Some(Tz::Asia__Kolkata),
            "france" => Some(Tz::Europe__Paris),
            "germany" => Some(Tz::Europe__Berlin),
            _ => city_zone(&place),
        };
        return tz.map(|tz| (skip + n + 1, tz));
    }
    None
}

/// The zone named after `city` ("london", "new york", "kuala lumpur").
fn city_zone(city: &str) -> Option<Tz> {
    chrono_tz::TZ_VARIANTS.iter().copied().find(|tz| {
        let name = tz.name();
        name.contains('/')
            && !name.starts_with("Etc/")
            && name
                .rsplit('/')
                .next()
                .is_some_and(|c| c.replace('_', " ").eq_ignore_ascii_case(city))
    })
}

/// "every …" or daily/weekly/…: an RRULE, and the weekday it starts on.
fn repeat_at(t: &[Tok], i: usize) -> Option<(usize, String, Vec<Weekday>)> {
    let w = |k: usize| t.get(i + k).filter(|t| !t.quoted).map(|t| t.word.as_str());
    let simple = |rule: &str| Some((1, rule.to_owned(), Vec::new()));
    match w(0)? {
        "daily" => return simple("FREQ=DAILY"),
        "weekly" => return simple("FREQ=WEEKLY"),
        "fortnightly" | "biweekly" => return simple("FREQ=WEEKLY;INTERVAL=2"),
        "monthly" => return simple("FREQ=MONTHLY"),
        "yearly" | "annually" => return simple("FREQ=YEARLY"),
        "every" | "each" => {}
        _ => return None,
    }
    let mut k = 1;
    let mut interval = 1u32;
    match w(k)? {
        "other" => {
            interval = 2;
            k += 1;
        }
        n if n.parse::<u32>().is_ok_and(|n| (2..=99).contains(&n)) => {
            interval = n.parse().unwrap();
            k += 1;
        }
        _ => {}
    }
    let with_interval = |freq: &str| {
        if interval > 1 {
            format!("FREQ={freq};INTERVAL={interval}")
        } else {
            format!("FREQ={freq}")
        }
    };
    let unit = w(k)?;
    let freq = match unit.trim_end_matches('s') {
        "day" => Some("DAILY"),
        "week" => Some("WEEKLY"),
        "month" => Some("MONTHLY"),
        "year" => Some("YEARLY"),
        _ => None,
    };
    if let Some(freq) = freq {
        return Some((k + 1, with_interval(freq), Vec::new()));
    }
    if matches!(unit, "weekday" | "weekdays") {
        use Weekday::*;
        return Some((
            k + 1,
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_owned(),
            vec![Mon, Tue, Wed, Thu, Fri],
        ));
    }
    // every mon[, wed] [and fri]
    let mut days = Vec::new();
    while let Some(word) = w(k) {
        if let Some(wd) = weekday(word) {
            days.push(wd);
            k += 1;
        } else if matches!(word, "and" | "&" | ",") && w(k + 1).and_then(weekday).is_some() {
            k += 1;
        } else {
            break;
        }
    }
    if days.is_empty() {
        return None;
    }
    let by = days
        .iter()
        .map(|d| d.to_string()[..2].to_uppercase())
        .collect::<Vec<_>>()
        .join(",");
    Some((k, format!("{};BYDAY={by}", with_interval("WEEKLY")), days))
}

/// Digits shaped like a clock time, valid or not: "13pm", "25:00", "9.75".
fn looks_like_time(w: &str) -> bool {
    let body = w.trim_end_matches("am").trim_end_matches("pm");
    body != w
        || body.contains([':', '.'])
            && body
                .chars()
                .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
}

/// A piece at `t[i..]`, or None if `t[i]` belongs to the title.
fn piece_at(t: &[Tok], i: usize, today: NaiveDate) -> Option<(usize, Piece)> {
    let tok = &t[i];
    if tok.quoted {
        return None;
    }
    let w = |k: usize| t.get(i + k).filter(|t| !t.quoted).map(|t| t.word.as_str());
    let w0 = w(0)?;

    if let Some(name) = tok.text.strip_prefix('#').filter(|n| !n.is_empty()) {
        return Some((1, Piece::Calendar(name.trim_end_matches(',').to_owned())));
    }
    // priority
    if let Some(n) = w0.strip_prefix('!') {
        let p = match n {
            "" | "!" | "!!" => Some(1),
            n => n.parse::<u8>().ok().filter(|p| (1..=9).contains(p)),
        };
        if let Some(p) = p {
            return Some((1, Piece::Priority(p)));
        }
    }
    if matches!(w0, "urgent" | "important" | "asap") {
        return Some((1, Piece::Priority(1)));
    }
    if matches!(w0, "high" | "medium" | "low") && w(1) == Some("priority") {
        let p = match w0 {
            "high" => 1,
            "medium" => 5,
            _ => 9,
        };
        return Some((2, Piece::Priority(p)));
    }
    if let Some((n, rule, day)) = repeat_at(t, i) {
        return Some((n, Piece::Repeat(rule, day)));
    }
    // due / by: a date, a time, or both
    if matches!(w0, "due" | "by" | "before") {
        let mut k = 1;
        if w0 == "due" && w(1) == Some("on") {
            k = 2;
        }
        let date = date_at(t, i + k, today);
        if let Some((n, _)) = date {
            k += n;
        }
        let time = times_at(t, i + k).and_then(|(n, p)| match p {
            Piece::Time(t) => Some((n, t)),
            _ => None,
        });
        if let Some((n, _)) = time {
            k += n;
        }
        if date.is_some() || time.is_some() {
            return Some((k, Piece::Due(date.map(|d| d.1), time.map(|t| t.1))));
        }
        return None;
    }
    if matches!(w0, "starts" | "starting" | "start") {
        let skip = 1 + usize::from(w(1) == Some("on"));
        if let Some((n, d)) = date_at(t, i + skip, today) {
            return Some((skip + n, Piece::Start(d)));
        }
    }
    if w0 == "for"
        && let Some((n, d)) = length_at(t, i + 1)
    {
        return Some((n + 1, Piece::Length(d)));
    }
    if let Some((n, tz)) = zone_at(t, i) {
        return Some((n, Piece::Zone(tz)));
    }
    if let Some((n, a, b)) = dates_at(t, i, today) {
        return Some((n, Piece::Dates(a, b)));
    }
    let skip = usize::from(matches!(w0, "on" | "this"));
    if let Some((n, d)) = date_at(t, i + skip, today) {
        return Some((skip + n, Piece::Date(d)));
    }
    if let Some(found) = times_at(t, i) {
        return Some(found);
    }
    // a place: "@ The Mill", "@Mill", "at The Mill" (not a time)
    let place_from = if w0 == "@" {
        Some(1)
    } else if tok.text.starts_with('@') && tok.text.len() > 1 {
        Some(0)
    } else if w0 == "at"
        && t.get(i + 1).is_some_and(|n| {
            n.quoted
                || n.text.starts_with(char::is_uppercase)
                // "at 10 Downing St", but not a time that is wrong ("13pm")
                || (n.text.starts_with(|c: char| c.is_ascii_digit()) && !looks_like_time(&n.word))
        })
    {
        Some(1)
    } else {
        None
    };
    if let Some(from) = place_from {
        let mut words = Vec::new();
        let mut k = from;
        while i + k < t.len() {
            // up to the next thing that is not part of the place
            if k > from && piece_at(t, i + k, today).is_some() {
                break;
            }
            let text = t[i + k].text.as_str();
            words.push(if k == 0 { &text[1..] } else { text });
            k += 1;
        }
        let place = words.join(" ").trim_end_matches(',').to_owned();
        if !place.is_empty() {
            return Some((k, Piece::Place(place)));
        }
    }
    None
}

/// Read a line. `task` says whether it is a task (its date is when it is
/// due); `focused` is the day a time alone is on.
pub fn parse(text: &str, today: NaiveDate, focused: NaiveDate, local: Tz, task: bool) -> Quick {
    let t = tokens(text);
    let mut q = Quick::default();
    let mut title = Vec::new();
    let (mut dates, mut times) = (None::<(NaiveDate, Option<NaiveDate>)>, None);
    let (mut until, mut length) = (None, None);
    let (mut due_date, mut due_time, mut start_date) = (None, None, None);
    let mut repeat_days = Vec::new();

    let mut i = 0;
    while i < t.len() {
        match piece_at(&t, i, today) {
            Some((n, piece)) => {
                match piece {
                    Piece::Date(d) if dates.is_none() => dates = Some((d, None)),
                    Piece::Dates(a, b) if dates.is_none() => dates = Some((a, Some(b))),
                    Piece::Time(s) if times.is_none() => times = Some((s, None)),
                    Piece::Times(s, e) if times.is_none() => times = Some((s, Some(e))),
                    Piece::Until(e) => until = Some(e),
                    Piece::Length(d) => length = Some(d),
                    Piece::Zone(z) => q.zone = Some(z),
                    Piece::Place(p) if q.location.is_none() => q.location = Some(p),
                    Piece::Calendar(c) => q.calendar = Some(c),
                    Piece::Priority(p) => q.priority = Some(p),
                    Piece::Repeat(r, d) => {
                        q.repeat = Some(r);
                        repeat_days = d;
                    }
                    Piece::Due(d, tm) => {
                        due_date = due_date.or(d);
                        due_time = due_time.or(tm);
                    }
                    Piece::Start(d) => start_date = Some(d),
                    // a second date or time: part of the title after all
                    _ => title.extend(t[i..i + n].iter().map(|t| t.text.clone())),
                }
                i += n;
            }
            None => {
                title.push(t[i].text.clone());
                i += 1;
            }
        }
    }
    q.title = title.join(" ").trim().trim_matches(',').trim().to_owned();
    // "every mon and thu" with no date starts on the first of them to come
    // (the start is always one of its days in iCalendar, so it must match)
    if dates.is_none()
        && let Some(first) = repeat_days.iter().map(|wd| next_weekday(today, *wd)).min()
    {
        dates = Some((first, None));
    }
    let tz = q.zone.unwrap_or(local);
    let at = |d: NaiveDate, t: NaiveTime| When::Zoned(resolve_local(tz, d.and_time(t)));

    if task {
        // a task: any date is its due date; "due"/"by" says so outright
        let due_day = due_date.or(dates.map(|(a, b)| b.unwrap_or(a)));
        let due_at = due_time.or(times.map(|(s, e)| e.unwrap_or(s))).or(until);
        q.due = match (due_day, due_at) {
            (Some(d), Some(tm)) => Some(at(d, tm)),
            (Some(d), None) => Some(When::Date(d)),
            (None, Some(tm)) => Some(at(focused, tm)),
            (None, None) => None,
        };
        q.start = start_date
            .or(dates.and_then(|(a, b)| b.map(|_| a)))
            .map(When::Date);
        return q;
    }

    // an event
    let day = dates.map(|(a, _)| a).or(due_date);
    let last_day = dates.and_then(|(_, b)| b);
    match times {
        Some((s, e)) => {
            let d = day.unwrap_or(focused);
            let start = at(d, s);
            let end = match e.or(until) {
                Some(e) => {
                    let end_day = last_day.unwrap_or(d);
                    let mut end = at(end_day, e);
                    // "11pm-1am" ends the next day
                    if end.to_utc(tz) <= start.to_utc(tz) {
                        end = at(end_day.succ_opt().unwrap_or(end_day), e);
                    }
                    end
                }
                None => match length {
                    Some(l) => start.add(l),
                    None => start.add(TimeDelta::hours(1)),
                },
            };
            q.start = Some(start);
            q.end = Some(end);
        }
        None => {
            if let Some(d) = day {
                // all day, through the last day (or for a length in days)
                let end = match (last_day, length) {
                    (Some(b), _) => b.succ_opt(),
                    (None, Some(l)) if l >= TimeDelta::days(1) => {
                        d.checked_add_days(Days::new(l.num_days() as u64))
                    }
                    _ => d.succ_opt(),
                };
                q.start = Some(When::Date(d));
                q.end = end.map(When::Date);
            }
        }
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONDON: Tz = Tz::Europe__London;
    const SG: Tz = Tz::Asia__Singapore;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// Wednesday 21 October 2026, looking at that day, in Singapore.
    fn event(text: &str) -> Quick {
        parse(text, d("2026-10-21"), d("2026-10-21"), SG, false)
    }

    fn task(text: &str) -> Quick {
        parse(text, d("2026-10-21"), d("2026-10-21"), SG, true)
    }

    fn show(w: &Option<When>) -> String {
        match w {
            None => "-".into(),
            Some(When::Date(d)) => d.to_string(),
            Some(When::Zoned(z)) => z.format("%Y-%m-%d %H:%M %Z").to_string(),
            Some(When::Floating(f)) => f.to_string(),
        }
    }

    /// title | start | end
    fn ev(text: &str) -> String {
        let q = event(text);
        format!("{} | {} | {}", q.title, show(&q.start), show(&q.end))
    }

    #[test]
    fn events_on_days_and_times() {
        for (text, want) in [
            (
                "Lunch with Sam tomorrow 1pm",
                "Lunch with Sam | 2026-10-22 13:00 +08 | 2026-10-22 14:00 +08",
            ),
            (
                "Dentist fri 3:30pm for 45m",
                "Dentist | 2026-10-23 15:30 +08 | 2026-10-23 16:15 +08",
            ),
            (
                "Call mum sunday 7-8pm",
                "Call mum | 2026-10-25 19:00 +08 | 2026-10-25 20:00 +08",
            ),
            (
                "Standup 9:30 for 15 minutes",
                "Standup | 2026-10-21 09:30 +08 | 2026-10-21 09:45 +08",
            ),
            (
                "Review at 5",
                "Review | 2026-10-21 17:00 +08 | 2026-10-21 18:00 +08",
            ),
            (
                "Breakfast at 8",
                "Breakfast | 2026-10-21 08:00 +08 | 2026-10-21 09:00 +08",
            ),
            (
                "Workshop from 9 to 11:30 on 14 nov",
                "Workshop | 2026-11-14 09:00 +08 | 2026-11-14 11:30 +08",
            ),
            (
                "Gig 11pm-1am sat",
                "Gig | 2026-10-24 23:00 +08 | 2026-10-25 01:00 +08",
            ),
            (
                "Lunch noon",
                "Lunch | 2026-10-21 12:00 +08 | 2026-10-21 13:00 +08",
            ),
            (
                "Plan 2-4pm next mon",
                "Plan | 2026-10-26 14:00 +08 | 2026-10-26 16:00 +08",
            ),
            (
                "Sync 11-1pm",
                "Sync | 2026-10-21 11:00 +08 | 2026-10-21 13:00 +08",
            ),
            (
                "Talk 9 am",
                "Talk | 2026-10-21 09:00 +08 | 2026-10-21 10:00 +08",
            ),
            (
                "Meet at 10 until 12",
                "Meet | 2026-10-21 10:00 +08 | 2026-10-21 12:00 +08",
            ),
            (
                "Pitch 14/11 10:00",
                "Pitch | 2026-11-14 10:00 +08 | 2026-11-14 11:00 +08",
            ),
            (
                "Pitch nov 14th at 10am",
                "Pitch | 2026-11-14 10:00 +08 | 2026-11-14 11:00 +08",
            ),
            (
                "Retro in 2 weeks 4pm for 1h30",
                "Retro | 2026-11-04 16:00 +08 | 2026-11-04 17:30 +08",
            ),
            (
                "Call 9.30pm",
                "Call | 2026-10-21 21:30 +08 | 2026-10-21 22:30 +08",
            ),
        ] {
            assert_eq!(ev(text), want, "{text}");
        }
    }

    #[test]
    fn all_day_and_multi_day() {
        for (text, want) in [
            (
                "Mum's birthday 15 nov",
                "Mum's birthday | 2026-11-15 | 2026-11-16",
            ),
            (
                "Team offsite 3-5 dec",
                "Team offsite | 2026-12-03 | 2026-12-06",
            ),
            ("Holiday dec 20-31", "Holiday | 2026-12-20 | 2027-01-01"),
            (
                "Conference mon to wed",
                "Conference | 2026-10-26 | 2026-10-29",
            ),
            ("Trip fri to mon", "Trip | 2026-10-23 | 2026-10-27"),
            (
                "Ski trip 14 jan for 5 days",
                "Ski trip | 2027-01-14 | 2027-01-19",
            ),
            (
                "New year party 2027-01-01",
                "New year party | 2027-01-01 | 2027-01-02",
            ),
            ("Pay rent on the 1st", "Pay rent | 2026-11-01 | 2026-11-02"),
            ("Exam 3 march 2027", "Exam | 2027-03-03 | 2027-03-04"),
        ] {
            assert_eq!(ev(text), want, "{text}");
        }
    }

    #[test]
    fn zones_places_calendars_repeats() {
        // in London time, from Singapore: 9am there is 4pm here (GMT in Nov)
        let q = event("Board meeting 12 nov 9am London time @ Head office #work");
        assert_eq!(q.title, "Board meeting");
        assert_eq!(q.zone, Some(LONDON));
        assert_eq!(show(&q.start), "2026-11-12 09:00 GMT");
        assert_eq!(
            q.start
                .unwrap()
                .to_utc(SG)
                .with_timezone(&SG)
                .format("%H:%M")
                .to_string(),
            "17:00"
        );
        assert_eq!(q.location.as_deref(), Some("Head office"));
        assert_eq!(q.calendar.as_deref(), Some("work"));

        let q = event("Dinner at Bistro 21 tomorrow 7pm");
        assert_eq!(
            (q.title.as_str(), q.location.as_deref()),
            ("Dinner", Some("Bistro 21"))
        );
        assert_eq!(show(&q.start), "2026-10-22 19:00 +08");

        assert_eq!(event("Call 3pm BST").zone, Some(LONDON));
        assert_eq!(
            event("Call 3pm New York time").zone,
            Some(Tz::America__New_York)
        );
        assert_eq!(event("Call 3pm UK time").zone, Some(LONDON));
        assert_eq!(event("Call 3pm Europe/Paris").zone, Some(Tz::Europe__Paris));

        let q = event("Standup every weekday 9:30 for 15m");
        assert_eq!(
            q.repeat.as_deref(),
            Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
        );
        assert_eq!(q.title, "Standup");
        let q = event("Climbing every tue and thu 7pm");
        assert_eq!(q.repeat.as_deref(), Some("FREQ=WEEKLY;BYDAY=TU,TH"));
        // the first is the first of its days to come: from Wednesday, Thursday
        assert_eq!(show(&q.start), "2026-10-22 19:00 +08");
        // and a weekday rule typed on a Sunday starts on Monday, not that Sunday
        let q = parse(
            "Standup every weekday 9:30",
            d("2026-10-25"),
            d("2026-10-25"),
            SG,
            false,
        );
        assert_eq!(show(&q.start), "2026-10-26 09:30 +08");
        assert_eq!(
            event("Bins every other week").repeat.as_deref(),
            Some("FREQ=WEEKLY;INTERVAL=2")
        );
        assert_eq!(
            event("Rent monthly").repeat.as_deref(),
            Some("FREQ=MONTHLY")
        );
    }

    #[test]
    fn tasks() {
        let q = task("Submit expenses by fri 5pm !");
        assert_eq!(q.title, "Submit expenses");
        assert_eq!(show(&q.due), "2026-10-23 17:00 +08");
        assert_eq!(q.priority, Some(1));
        let q = task("Buy milk tomorrow");
        assert_eq!(
            (q.title.as_str(), show(&q.due)),
            ("Buy milk", "2026-10-22".into())
        );
        let q = task("Renew passport due 1 dec low priority #personal");
        assert_eq!(show(&q.due), "2026-12-01");
        assert_eq!(
            (q.priority, q.calendar.as_deref()),
            (Some(9), Some("personal"))
        );
        let q = task("Write report starts mon due fri");
        assert_eq!(
            (show(&q.start), show(&q.due)),
            ("2026-10-26".into(), "2026-10-23".into())
        );
        assert_eq!(
            task("Water plants every 3 days").repeat.as_deref(),
            Some("FREQ=DAILY;INTERVAL=3")
        );
        assert_eq!(task("Just a thing").due, None);
    }

    #[test]
    fn titles_keep_what_is_not_a_date() {
        for (text, title) in [
            ("Read Kafka by the sea", "Read Kafka by the sea"),
            ("Book by Murakami", "Book by Murakami"),
            ("Buy 3 apples", "Buy 3 apples"),
            ("Run 5k tomorrow", "Run 5k"),
            ("\"Meeting at 3\" tomorrow", "Meeting at 3"),
            ("Room 101 at 3pm", "Room 101"),
            ("Mayday parade", "Mayday parade"),
            ("Marching band practice", "Marching band practice"),
            ("Call the dentist", "Call the dentist"),
            ("Sunday roast prep", "roast prep"),
        ] {
            assert_eq!(event(text).title, title, "{text}");
        }
        // a time with nothing else is today's
        assert_eq!(
            ev("Gym 6pm"),
            "Gym | 2026-10-21 18:00 +08 | 2026-10-21 19:00 +08"
        );
        // nothing understood: no time at all
        assert_eq!(ev("Think about things"), "Think about things | - | -");
    }

    #[test]
    fn harder_cases() {
        for (text, want) in [
            // words that look like dates or times but are not, here
            ("Sprint 23 planning", "Sprint 23 planning | - | -"),
            (
                "March for climate 14 mar 10am",
                "March for climate | 2027-03-14 10:00 +08 | 2027-03-14 11:00 +08",
            ),
            ("May day 1 may", "May day | 2027-05-01 | 2027-05-02"),
            ("Watch 24 tomorrow", "Watch 24 | 2026-10-22 | 2026-10-23"),
            (
                "Pick up 2 parcels at 4",
                "Pick up 2 parcels | 2026-10-21 16:00 +08 | 2026-10-21 17:00 +08",
            ),
            (
                "Cook for 6 friday 7pm",
                "Cook for 6 | 2026-10-23 19:00 +08 | 2026-10-23 20:00 +08",
            ),
            (
                "Flight SQ322 14 nov 23:55 for 14h",
                "Flight SQ322 | 2026-11-14 23:55 +08 | 2026-11-15 13:55 +08",
            ),
            // punctuation and case
            (
                "LUNCH, Tomorrow, 1PM",
                "LUNCH | 2026-10-22 13:00 +08 | 2026-10-22 14:00 +08",
            ),
            (
                "Call Jo – 9–10am",
                "Call Jo – | 2026-10-21 09:00 +08 | 2026-10-21 10:00 +08",
            ),
            (
                "Review 10:00-10:30 thursday",
                "Review | 2026-10-22 10:00 +08 | 2026-10-22 10:30 +08",
            ),
            (
                "Coffee in 1 day at 9",
                "Coffee | 2026-10-22 09:00 +08 | 2026-10-22 10:00 +08",
            ),
            ("Party next week", "Party | 2026-10-26 | 2026-10-27"),
            (
                "Review today midday",
                "Review | 2026-10-21 12:00 +08 | 2026-10-21 13:00 +08",
            ),
            (
                "Sleep 23:00 until 7am",
                "Sleep | 2026-10-21 23:00 +08 | 2026-10-22 07:00 +08",
            ),
            ("Thing 31/12/27", "Thing | 2027-12-31 | 2028-01-01"),
            ("Thing 30 feb", "Thing 30 feb | - | -"),
            ("Thing 25:00", "Thing 25:00 | - | -"),
            ("Call at 13pm", "Call at 13pm | - | -"),
            // found by fuzzing: lengths and offsets too large are not read
            ("Nap for 99999999999999w", "Nap for 99999999999999w | - | -"),
            ("Nap in 999999999 years", "Nap in 999999999 years | - | -"),
            ("", " | - | -"),
            ("   ", " | - | -"),
        ] {
            assert_eq!(ev(text), want, "{text:?}");
        }
    }

    #[test]
    fn the_singapore_to_london_trip() {
        // planning from Singapore for a UK trip in British summer time
        let q = event("Meet Priya 3 jun 2027 10am London time @ King's Cross for 1h30");
        assert_eq!(show(&q.start), "2027-06-03 10:00 BST");
        assert_eq!(show(&q.end), "2027-06-03 11:30 BST");
        assert_eq!(q.location.as_deref(), Some("King's Cross"));
        assert_eq!(
            q.start
                .unwrap()
                .to_utc(SG)
                .with_timezone(&SG)
                .format("%H:%M")
                .to_string(),
            "17:00"
        );
        // and a flight across zones, written in the departure zone
        let q = event("Fly home 10 jun 2027 21:00 BST for 13h");
        assert_eq!(show(&q.start), "2027-06-10 21:00 BST");
        assert_eq!(
            q.end
                .unwrap()
                .to_utc(SG)
                .with_timezone(&SG)
                .format("%Y-%m-%d %H:%M")
                .to_string(),
            "2027-06-11 17:00"
        );
    }
}

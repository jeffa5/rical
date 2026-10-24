//! Human-readable text for dates, times and occurrences.

use adw::prelude::*;
use chrono::{Datelike, Days, NaiveDate};
use chrono_tz::Tz;
use rical_core::{Occurrence, Rgb, When};

use crate::{
    draw,
    layout::{self, ViewKind},
};

pub fn time(w: &When, local: Tz) -> String {
    w.to_tz(local).format("%H:%M").to_string()
}

/// e.g. "Thu 23 Oct, 09:00 – 10:30" or "Thu 23 – Sat 25 Oct, all day".
pub fn time_range(o: &Occurrence, local: Tz) -> String {
    let start_day = o.start.date(local);
    if o.all_day {
        let last = o
            .end
            .date(local)
            .pred_opt()
            .unwrap_or(start_day)
            .max(start_day);
        if last == start_day {
            return format!("{}, all day", day(start_day));
        }
        return format!("{} – {}, all day", day(start_day), day(last));
    }
    let end_day = o.end.date(local);
    if o.start == o.end {
        return format!("{}, {}", day(start_day), time(&o.start, local));
    }
    if end_day == start_day {
        format!(
            "{}, {} – {}",
            day(start_day),
            time(&o.start, local),
            time(&o.end, local)
        )
    } else {
        format!(
            "{}, {} – {}, {}",
            day(start_day),
            time(&o.start, local),
            day(end_day),
            time(&o.end, local)
        )
    }
}

pub fn day(d: NaiveDate) -> String {
    d.format("%a %-d %b").to_string()
}

pub fn todo_mark(o: &Occurrence) -> &'static str {
    match o.status.as_deref() {
        Some("COMPLETED") => "☑",
        Some("CANCELLED") => "☒",
        _ => "☐",
    }
}

/// Header title and subtitle for the current view.
pub fn view_title(view: ViewKind, date: NaiveDate) -> (String, String) {
    match view {
        ViewKind::Day => (
            date.format("%A %-d %B").to_string(),
            date.format("%Y · week %V").to_string(),
        ),
        ViewKind::Week => {
            let start = layout::week_start(date);
            let end = start + Days::new(6);
            let title = if start.month() == end.month() {
                format!("{} – {}", start.format("%-d"), end.format("%-d %B %Y"))
            } else if start.year() == end.year() {
                format!("{} – {}", start.format("%-d %b"), end.format("%-d %b %Y"))
            } else {
                format!(
                    "{} – {}",
                    start.format("%-d %b %Y"),
                    end.format("%-d %b %Y")
                )
            };
            (title, format!("Week {}", date.format("%V")))
        }
        ViewKind::Month => (date.format("%B %Y").to_string(), String::new()),
        ViewKind::Agenda => (
            format!("From {}", date.format("%-d %B %Y")),
            format!("Next {} days", layout::AGENDA_DAYS),
        ),
        ViewKind::Tasks => ("Tasks".to_owned(), String::new()),
    }
}

/// A task shown on a later day than it was due: still to do, carried
/// forward so it is not lost.
pub fn is_overdue(o: &Occurrence, local: Tz) -> bool {
    o.kind == rical_core::Kind::Todo
        && !is_inactive(o)
        && o.due
            .is_some_and(|due| due.date(local) < o.start.date(local))
}

/// Cancelled, or a completed task: drawn faded and struck through.
pub fn is_inactive(o: &Occurrence) -> bool {
    matches!(o.status.as_deref(), Some("CANCELLED" | "COMPLETED"))
}

/// The text to show for an occurrence: tasks get a checkbox, and overdue
/// tasks say when they were due.
pub fn title(o: &Occurrence, local: Tz) -> String {
    match o.kind {
        rical_core::Kind::Todo => {
            let mut t = format!("{} {}", todo_mark(o), o.summary);
            if is_overdue(o, local)
                && let Some(due) = o.due
            {
                t += &format!(" (due {})", due.date(local).format("%-d %b"));
            }
            t
        }
        _ => o.summary.clone(),
    }
}

/// A small filled circle in a calendar colour.
pub fn color_dot(color: Rgb, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(size)
        .content_height(size)
        .valign(gtk::Align::Center)
        .build();
    area.set_draw_func(move |_, cr, w, h| {
        let r = w.min(h) as f64 / 2.0;
        cr.arc(
            w as f64 / 2.0,
            h as f64 / 2.0,
            r,
            0.0,
            std::f64::consts::TAU,
        );
        draw::set_rgb(cr, color, 1.0);
        let _ = cr.fill();
    });
    area
}

/// Text for a date/time entry: "2025-03-04 09:00", or "2025-03-04" for dates.
pub fn input(w: &When, local: Tz) -> String {
    match w {
        When::Date(d) => d.format("%Y-%m-%d").to_string(),
        _ => w.to_tz(local).format("%Y-%m-%d %H:%M").to_string(),
    }
}

/// Parse what was typed in a start or end field. Accepts "YYYY-MM-DD HH:MM",
/// "YYYY-MM-DD", "HH:MM" (on `base`'s day) and, for ends, a duration after
/// `base` such as "+1h30m" or "45m". All-day values are dates.
pub fn parse_input(text: &str, all_day: bool, base: &When, local: Tz) -> Option<When> {
    use chrono::{NaiveDateTime, NaiveTime, TimeDelta};

    let text = text.trim();
    let duration = text.strip_prefix('+').unwrap_or(text);
    if !duration.is_empty()
        && duration.ends_with(['m', 'h', 'd', 'w'])
        && duration.starts_with(|c: char| c.is_ascii_digit())
    {
        let mut total = TimeDelta::zero();
        let mut num = String::new();
        for c in duration.chars() {
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
                ' ' => {}
                _ => return None,
            }
        }
        return Some(base.add(total));
    }

    if all_day {
        let d = NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .ok()
            .or_else(|| {
                NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M")
                    .ok()
                    .map(|dt| dt.date())
            })?;
        return Some(When::Date(d));
    }
    let ndt = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M")
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .ok()
                .map(|d| d.and_time(NaiveTime::from_hms_opt(9, 0, 0).unwrap()))
        })
        .or_else(|| {
            let t = NaiveTime::parse_from_str(text, "%H:%M").ok()?;
            Some(base.date(local).and_time(t))
        })?;
    Some(When::Zoned(rical_core::when::resolve_local(local, ndt)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LONDON: Tz = Tz::Europe__London;

    #[test]
    fn inputs() {
        let base = When::parse("20250304T090000", Some("Europe/London")).unwrap();
        let p = |s, all_day| parse_input(s, all_day, &base, LONDON).map(|w| input(&w, LONDON));
        assert_eq!(
            p("2025-03-05 14:30", false).as_deref(),
            Some("2025-03-05 14:30")
        );
        assert_eq!(p("2025-03-05", false).as_deref(), Some("2025-03-05 09:00"));
        assert_eq!(p("17:15", false).as_deref(), Some("2025-03-04 17:15"));
        assert_eq!(p("+1h30m", false).as_deref(), Some("2025-03-04 10:30"));
        assert_eq!(p("45m", false).as_deref(), Some("2025-03-04 09:45"));
        assert_eq!(p("2025-03-05", true).as_deref(), Some("2025-03-05"));
        assert_eq!(p("tomorrow", false), None);
        assert_eq!(p("", false), None);

        let day = When::parse("20250304", None).unwrap();
        assert_eq!(
            parse_input("+2d", true, &day, LONDON)
                .map(|w| input(&w, LONDON))
                .as_deref(),
            Some("2025-03-06")
        );
    }
}

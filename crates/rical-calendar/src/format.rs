//! Human-readable text for dates, times and occurrences.

use adw::prelude::*;
use chrono::{Datelike, NaiveDate};
use chrono_tz::Tz;
use rical_core::{Kind, Occurrence, Rgb, When};

use crate::{draw, layout::ViewKind};

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

/// e.g. "Thu 23 Oct", with the year when it is not this year.
pub fn day(d: NaiveDate) -> String {
    if d.year() == chrono::Utc::now().year() {
        d.format("%a %-d %b").to_string()
    } else {
        d.format("%a %-d %b %Y").to_string()
    }
}

/// Header title and subtitle for the current view.
pub fn view_title(view: ViewKind, date: NaiveDate, days: &[NaiveDate]) -> (String, String) {
    let this_year = chrono::Utc::now().year();
    match view {
        ViewKind::Day => (
            date.format("%A %-d %B").to_string(),
            date.format("%Y · week %V").to_string(),
        ),
        ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_) => {
            let (start, end) = (days[0], *days.last().unwrap());
            // short enough for narrow windows: "19 – 25 Oct"
            let mut title = if start.month() == end.month() {
                format!("{} – {}", start.format("%-d"), end.format("%-d %b"))
            } else {
                format!("{} – {}", start.format("%-d %b"), end.format("%-d %b"))
            };
            if end.year() != this_year || start.year() != end.year() {
                title = format!("{title} {}", end.year());
            }
            let sub = match view {
                ViewKind::Days(n) => format!("{n} days · week {}", start.format("%V")),
                ViewKind::WorkWeek => format!("Work week {}, {}", start.format("%V"), start.year()),
                _ => format!("Week {}, {}", start.format("%V"), start.year()),
            };
            (title, sub)
        }
        ViewKind::Month => (date.format("%B %Y").to_string(), String::new()),
        ViewKind::Year => (date.format("%Y").to_string(), String::new()),
        ViewKind::Agenda => (
            "Agenda".to_owned(),
            // the days the list covers, which selecting in it does not move
            match (days.first(), days.last()) {
                (Some(a), Some(b)) => format!("{} – {}", a.format("%-d %b"), b.format("%-d %b %Y")),
                _ => String::new(),
            },
        ),
        ViewKind::Tasks => ("Tasks".to_owned(), String::new()),
        ViewKind::Invitations => (
            "Invitations".to_owned(),
            "Waiting for your reply".to_owned(),
        ),
        ViewKind::Journal => ("Journal".to_owned(), String::new()),
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

/// An attendee's PARTSTAT in words.
pub fn reply(partstat: &str) -> &'static str {
    match partstat {
        "ACCEPTED" => "going",
        "DECLINED" => "not going",
        "TENTATIVE" => "maybe",
        "DELEGATED" => "delegated",
        _ => "not answered",
    }
}

pub fn reply_icon(partstat: &str) -> &'static str {
    match partstat {
        "ACCEPTED" => "object-select-symbolic",
        "DECLINED" => "window-close-symbolic",
        "TENTATIVE" => "dialog-question-symbolic",
        "DELEGATED" => "mail-forward-symbolic",
        _ => "mail-unread-symbolic",
    }
}

/// Cancelled, or a completed task: drawn faded and struck through.
pub fn is_inactive(o: &Occurrence) -> bool {
    matches!(o.status.as_deref(), Some("CANCELLED" | "COMPLETED"))
}

/// The text to show for an occurrence; overdue tasks say when they were
/// due. Views draw the task circle themselves.
pub fn label(o: &Occurrence, local: Tz) -> String {
    if o.kind == Kind::Journal {
        return format!("✎ {}", journal_title(o));
    }
    let mut t = o.summary.clone();
    if is_overdue(o, local)
        && let Some(due) = o.due
    {
        t += &format!(" (due {})", due.date(local).format("%-d %b"));
    }
    t
}

/// A journal entry's title: its summary, else the first line of its text.
pub fn journal_title(o: &Occurrence) -> String {
    let title = o.summary.trim();
    if !title.is_empty() {
        return title.to_owned();
    }
    o.description
        .as_deref()
        .and_then(|d| d.lines().map(str::trim).find(|l| !l.is_empty()))
        .map_or_else(|| "(untitled)".to_owned(), str::to_owned)
}

pub fn is_done(o: &Occurrence) -> bool {
    matches!(o.status.as_deref(), Some("COMPLETED"))
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

/// The first web link in `text`, e.g. a video call in a location or notes.
pub fn first_url(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | ')'))
        .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        .map(|w| w.trim_end_matches(['.', ',', ';']).to_owned())
}

/// A link to look `place` up on a map.
pub use rical_ui::util::map_url;

/// Where to join `o` online: its video conference, else any conference
/// that opens in a browser (RFC 7986), else a link in its location or
/// notes.
pub fn join_url(o: &Occurrence) -> Option<String> {
    let c = &o.conferences;
    c.iter()
        .find(|c| c.has("VIDEO") && c.is_web())
        .or_else(|| c.iter().find(|c| c.is_web()))
        .map(|c| c.uri.clone())
        .or_else(|| {
            [o.location.as_deref(), o.description.as_deref()]
                .into_iter()
                .flatten()
                .find_map(first_url)
        })
}

/// A place to show on a map: the location, unless it is just a link.
pub fn place(o: &Occurrence) -> Option<&str> {
    o.location
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty() && first_url(l).as_deref() != Some(*l))
}

#[cfg(test)]
mod location_tests {
    use super::*;
    use rical_ui::util::{geo_url, web_map_url};

    #[test]
    fn urls() {
        assert_eq!(
            first_url("Join: https://meet.example.com/abc-def. Thanks").as_deref(),
            Some("https://meet.example.com/abc-def")
        );
        assert_eq!(
            first_url("<https://x.org/a>").as_deref(),
            Some("https://x.org/a")
        );
        assert_eq!(first_url("Room 4"), None);
        assert_eq!(
            web_map_url("Bistro 21, Leeds"),
            "https://www.openstreetmap.org/search?query=Bistro+21%2C+Leeds"
        );
        assert_eq!(
            web_map_url("Bistro 21,\r\n Leeds\n"),
            "https://www.openstreetmap.org/search?query=Bistro+21%2C+Leeds"
        );
        assert_eq!(
            geo_url("Bistro 21,\n Leeds"),
            "geo:0,0?q=Bistro+21%2C+Leeds"
        );
    }
}

/// The tick circle used for tasks everywhere: a ring, ticked when done,
/// drawn just like the circles in the time grid and month. Clicking it
/// toggles it and calls `on_toggle`; with None it only shows the state.
pub fn task_check(done: bool, on_toggle: Option<Box<dyn Fn(bool)>>) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::builder()
        .content_width(18)
        .content_height(18)
        .valign(gtk::Align::Center)
        .accessible_role(gtk::AccessibleRole::Checkbox)
        .build();
    let state = std::rc::Rc::new(std::cell::Cell::new(done));
    let set_accessible = |area: &gtk::DrawingArea, done: bool| {
        area.update_state(&[gtk::accessible::State::Checked(if done {
            gtk::AccessibleTristate::True
        } else {
            gtk::AccessibleTristate::False
        })]);
    };
    set_accessible(&area, done);
    let s = state.clone();
    area.set_draw_func(move |area, cr, w, h| {
        let pal = draw::Palette::of(area);
        pal.fg(cr, if s.get() { 0.55 } else { 0.8 });
        let r = w.min(h) as f64 / 2.0 - 2.0;
        draw::task_circle(cr, w as f64 / 2.0, h as f64 / 2.0, r, s.get());
    });
    match on_toggle {
        Some(on_toggle) => {
            area.set_tooltip_text(Some("Done"));
            area.set_cursor_from_name(Some("pointer"));
            let click = gtk::GestureClick::new();
            let a = area.clone();
            click.connect_released(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                let done = !state.get();
                state.set(done);
                set_accessible(&a, done);
                a.queue_draw();
                on_toggle(done);
            });
            area.add_controller(click);
        }
        None => area.set_tooltip_text(Some(if done { "Done" } else { "To do" })),
    }
    area
}

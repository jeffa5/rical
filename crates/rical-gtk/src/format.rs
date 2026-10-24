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

pub fn is_cancelled(o: &Occurrence) -> bool {
    matches!(o.status.as_deref(), Some("CANCELLED"))
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

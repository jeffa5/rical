//! Reminders: when the alarms of events and tasks go off.

use std::ops::Range;

use chrono::{DateTime, TimeDelta, Utc};
use chrono_tz::Tz;

use crate::{
    occurrence::{Kind, Occurrence},
    vdir::Vdir,
};

/// Alarms are looked for on occurrences up to this far from the window, so
/// "two weeks before" or "9 hours after the start of the day" are found.
const REACH: TimeDelta = TimeDelta::days(16);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    pub at: DateTime<Utc>,
    pub occurrence: Occurrence,
}

impl Vdir {
    /// Alarms going off in `window`. Items without alarms of their own get
    /// `default` before their start, if given (timed events only).
    /// Cancelled events and finished tasks are skipped.
    pub fn alarms(
        &self,
        window: &Range<DateTime<Utc>>,
        local: Tz,
        default: Option<TimeDelta>,
    ) -> Vec<Alarm> {
        let search = (window.start - REACH)..(window.end + REACH);
        let mut out = self
            .query(&search, &[Kind::Event, Kind::Todo], local)
            .into_iter()
            .filter(|o| !matches!(o.status.as_deref(), Some("CANCELLED" | "COMPLETED")))
            .flat_map(|o| {
                let mut times = o.alarm_times(local);
                if times.is_empty()
                    && let Some(before) = default
                    && o.kind == Kind::Event
                    && !o.all_day
                {
                    times.push(o.start.to_utc(local) - before);
                }
                times.sort();
                times.dedup();
                times
                    .into_iter()
                    .filter(|at| window.contains(at))
                    .map(move |at| Alarm {
                        at,
                        occurrence: o.clone(),
                    })
            })
            .collect::<Vec<_>>();
        out.sort_by_key(|a| a.at);
        // an absolute alarm on a series goes off once, not for every instance
        let mut seen = std::collections::HashSet::new();
        out.retain(|a| seen.insert((a.occurrence.path.clone(), a.occurrence.uid.clone(), a.at)));
        out
    }
}

impl Alarm {
    /// Notification text: the title, then when (relative and absolute)
    /// and where.
    pub fn describe(&self, now: DateTime<Utc>, local: Tz) -> (String, String) {
        let o = &self.occurrence;
        let mut title = o.summary.clone();
        if o.kind == Kind::Todo {
            title = format!("Task: {title}");
        }
        let start = o.start.to_tz(local);
        let mut body = vec![relative(o.start.to_utc(local) - now, o.all_day)];
        body.push(if o.all_day {
            start.format("%a %-d %b, all day").to_string()
        } else if o.start == o.end {
            start.format("%a %-d %b, %H:%M").to_string()
        } else {
            format!(
                "{} – {}",
                start.format("%a %-d %b, %H:%M"),
                o.end.to_tz(local).format("%H:%M")
            )
        });
        if let Some(loc) = o.location.as_deref().filter(|l| !l.is_empty()) {
            body.push(loc.to_owned());
        }
        (title, body.join("\n"))
    }
}

/// "in 30 min", "starting now", "started 5 min ago"; for all-day items
/// "today", "tomorrow" or "in N days".
pub fn relative(until: TimeDelta, all_day: bool) -> String {
    if all_day {
        return match until.num_hours() {
            h if h <= 0 => "today".to_owned(),
            h if h < 24 => "tomorrow".to_owned(),
            h => format!("in {} days", h / 24),
        };
    }
    match until.num_minutes() {
        m if m.abs() < 1 => "starting now".to_owned(),
        m if m > 0 && m < 120 => format!("in {m} min"),
        m if m > 0 && m < 48 * 60 => format!("in {} h {} min", m / 60, m % 60),
        m if m > 0 => format!("in {} days", m / (24 * 60)),
        m => format!("started {} min ago", -m),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(relative(TimeDelta::minutes(30), false), "in 30 min");
        assert_eq!(relative(TimeDelta::seconds(20), false), "starting now");
        assert_eq!(relative(TimeDelta::minutes(150), false), "in 2 h 30 min");
        assert_eq!(relative(TimeDelta::days(14), false), "in 14 days");
        assert_eq!(relative(-TimeDelta::minutes(5), false), "started 5 min ago");
        assert_eq!(relative(-TimeDelta::hours(9), true), "today");
        assert_eq!(relative(TimeDelta::days(14), true), "in 14 days");
    }
}

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

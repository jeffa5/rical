//! Reminders: desktop notifications (and a toast) when alarms go off.

use std::{collections::HashSet, path::PathBuf, rc::Rc};

use chrono::{DateTime, TimeDelta, Utc};
use log::{info, warn};
use rical_core::alarm::Alarm;

use super::Window;
use crate::format;

/// Alarms this long before the app started still go off.
pub const CATCH_UP: TimeDelta = TimeDelta::minutes(5);

/// Alarms already shown: (file, UID, when).
pub type Fired = HashSet<(PathBuf, String, DateTime<Utc>)>;

impl Window {
    /// Show any alarms that went off since the last check.
    pub(super) fn check_alarms(self: &Rc<Self>) {
        let now = Utc::now();
        let from = self.alarm_check.get();
        if now <= from {
            return;
        }
        self.alarm_check.set(now);
        let alarms = {
            let s = self.state.borrow();
            if !s.notifications {
                return;
            }
            s.vdir.alarms(&(from..now), s.local, s.default_reminder)
        };
        for alarm in alarms {
            let o = &alarm.occurrence;
            let key = (o.path.clone(), o.uid.clone(), alarm.at);
            if self.alarms_fired.borrow_mut().insert(key) {
                self.remind(&alarm, now);
            }
        }
    }

    fn remind(&self, alarm: &Alarm, now: DateTime<Utc>) {
        let local = self.state.borrow().local;
        let o = &alarm.occurrence;
        let title = format::title(o, local);
        let mut body = vec![relative(o.start.to_utc(local) - now, o.all_day)];
        body.push(format::time_range(o, local));
        if let Some(loc) = o.location.as_deref().filter(|l| !l.is_empty()) {
            body.push(loc.to_owned());
        }
        let body = body.join("\n");
        info!("reminder: {title} ({})", body.replace('\n', "; "));
        self.toast(&format!("{title} — {}", body.lines().next().unwrap_or("")));
        // talking to the notification daemon blocks, so not on this thread
        std::thread::spawn(move || {
            if let Err(e) = notify_rust::Notification::new()
                .appname("rical")
                .summary(&title)
                .body(&body)
                .icon("x-office-calendar")
                .show()
            {
                warn!("could not show a notification: {e}");
            }
        });
    }
}

/// "in 30 min", "now", "started 5 min ago", "today" (all day).
fn relative(until: TimeDelta, all_day: bool) -> String {
    let mins = until.num_minutes();
    if all_day {
        return match until.num_hours() {
            h if h <= 0 => "today".to_owned(),
            h if h < 24 => "tomorrow".to_owned(),
            h => format!("in {} days", h / 24),
        };
    }
    match mins {
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

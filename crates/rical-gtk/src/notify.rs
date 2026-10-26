//! Reminders: desktop notifications (and a toast) when alarms go off.

use std::{collections::HashSet, path::PathBuf, rc::Rc};

use chrono::{DateTime, TimeDelta, Utc};
use log::{info, warn};
use rical_core::alarm::{Alarm, AlarmPlan};

use super::Window;

/// Alarms this long before the app started still go off.
pub const CATCH_UP: TimeDelta = TimeDelta::minutes(5);

/// Alarms already shown: (file, UID, when).
pub type Fired = HashSet<(PathBuf, String, DateTime<Utc>)>;

/// The planned alarms, and what they were planned from: the items'
/// generation, the default reminder and the time zone.
pub type Plan = Option<((u64, Option<TimeDelta>, chrono_tz::Tz), AlarmPlan)>;

impl Window {
    /// Show any alarms that went off since the last check.
    pub(super) fn check_alarms(self: &Rc<Self>) {
        let now = Utc::now();
        let from = self.alarm_check.get();
        if now <= from {
            return;
        }
        self.alarm_check.set(now);
        // the alarms of the hours ahead, planned once (and again when the
        // items, the default reminder or the time zone change): expanding
        // every repeating event around now on each check costs more the
        // bigger the calendar
        let window = from..now;
        let alarms: Vec<Alarm> = {
            let s = self.state.borrow();
            if !s.notifications {
                return;
            }
            let key = (s.generation(), s.default_reminder, s.local);
            let mut plan = self.alarm_plan.borrow_mut();
            let stale = match &*plan {
                Some((k, p)) => *k != key || !p.covers(&window),
                None => true,
            };
            if stale {
                let p = AlarmPlan::new(&s.vdir, from, now, s.local, s.default_reminder);
                *plan = Some((key, p));
            }
            plan.as_ref().unwrap().1.due(&window).cloned().collect()
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
        let (title, body) = alarm.describe(now, local);
        info!("reminder: {title} ({})", body.replace('\n', "; "));
        self.toast(&format!("{title} — {}", body.lines().next().unwrap_or("")));
        // talking to the notification daemon blocks, so not on this thread
        std::thread::spawn(move || {
            if let Err(e) = notify_rust::Notification::new()
                .appname(crate::APP_NAME)
                .summary(&title)
                .body(&body)
                .icon(crate::APP_ID)
                .show()
            {
                warn!("could not show a notification: {e}");
            }
        });
    }
}

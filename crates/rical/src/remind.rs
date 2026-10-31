//! `rical remind`: show reminders as desktop notifications while running,
//! so they arrive without the GUI open (run it as a user service).

use std::{collections::HashSet, path::Path, thread::sleep, time::Duration};

use anyhow::Context as _;
use chrono::{TimeDelta, Utc};
use log::warn;
use rical_core::{Vdir, alarm::AlarmPlan, input::parse_shift, when::local_timezone};

/// Alarms this long before starting still go off.
const CATCH_UP: TimeDelta = TimeDelta::minutes(5);
const INTERVAL: Duration = Duration::from_secs(30);

pub fn remind(vdir: &Path, before: Option<&str>, once: bool) -> anyhow::Result<()> {
    let default = match before {
        Some(text) => Some(
            parse_shift(text)
                .filter(|d| *d > TimeDelta::zero())
                .with_context(|| format!("not a duration: {text:?}"))?,
        ),
        None => None,
    };
    let local = local_timezone();
    let mut fired = HashSet::new();
    let mut from = Utc::now() - CATCH_UP;
    // loaded once; after that only files that changed are read again, and
    // the alarms of the hours ahead are planned once rather than every check
    let mut vdir = Vdir::open(vdir)?;
    let mut plan: Option<AlarmPlan> = None;
    loop {
        let now = Utc::now();
        match vdir.refresh() {
            Ok(true) => plan = None,
            Ok(false) => {}
            Err(e) => {
                warn!("could not read {:?}: {e:#}", vdir.root);
                // calendars come and go too: start again
                if let Ok(v) = Vdir::open(&vdir.root) {
                    vdir = v;
                    plan = None;
                }
            }
        }
        let window = from..now;
        if plan.as_ref().is_none_or(|p| !p.covers(&window)) {
            plan = Some(AlarmPlan::new(&vdir, from, now, local, default));
        }
        for alarm in plan.as_ref().unwrap().due(&window) {
            let o = &alarm.occurrence;
            if !fired.insert((o.path.clone(), o.uid.clone(), alarm.at)) {
                continue;
            }
            let (title, body) = alarm.describe(now, local);
            println!(
                "{} {title}: {}",
                alarm.at.with_timezone(&local).format("%H:%M"),
                body.replace('\n', "; ")
            );
            if let Err(e) = notify_rust::Notification::new()
                .appname("Calendar")
                .summary(&title)
                .body(&body)
                .icon("net.jeffas.Rical")
                .show()
            {
                warn!("could not show a notification: {e}");
            }
        }
        from = now;
        if once {
            return Ok(());
        }
        sleep(INTERVAL);
    }
}

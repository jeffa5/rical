//! `rical remind`: show reminders as desktop notifications while running,
//! so they arrive without the GUI open (run it as a user service).

use std::{collections::HashSet, path::Path, thread::sleep, time::Duration};

use anyhow::Context as _;
use chrono::{TimeDelta, Utc};
use log::warn;
use rical_core::{Vdir, input::parse_shift, when::local_timezone};

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
    loop {
        // reload each time: vdirsyncer may have changed anything
        let now = Utc::now();
        match Vdir::load(vdir) {
            Ok(v) => {
                for alarm in v.alarms(&(from..now), local, default) {
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
                        .appname("rical")
                        .summary(&title)
                        .body(&body)
                        .icon("x-office-calendar")
                        .show()
                    {
                        warn!("could not show a notification: {e}");
                    }
                }
                from = now;
            }
            Err(e) => warn!("could not load {vdir:?}: {e:#}"),
        }
        if once {
            return Ok(());
        }
        sleep(INTERVAL);
    }
}

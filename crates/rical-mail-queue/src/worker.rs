//! Taking messages off the queue and handing them to the send command.

use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Result;
use chrono::{DateTime, Utc};

use crate::{
    config::Config,
    message,
    queue::{self, Claim, Queue},
    send::{self, Outcome},
};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub sent: usize,
    pub retrying: usize,
    pub failed: usize,
}

impl Report {
    pub fn is_empty(&self) -> bool {
        *self == Report::default()
    }
}

pub struct Worker<'a> {
    pub queue: &'a Queue,
    pub config: &'a Config,
}

impl Worker<'_> {
    /// Send everything that is due at `now`. Messages that fail and come
    /// due again later are left for the next call.
    pub fn run_once(&self, now: DateTime<Utc>) -> Result<Report> {
        self.queue.recover_stale(now)?;
        let mut report = Report::default();
        while let Some(claim) = self.queue.claim_next(now)? {
            match self.deliver(claim, now)? {
                Outcome::Sent => report.sent += 1,
                Outcome::Temporary(_) => report.retrying += 1,
                Outcome::Permanent(_) => report.failed += 1,
            }
        }
        Ok(report)
    }

    /// Keep going until told to stop: sending what arrives as it arrives
    /// (watching the queue), what is held back or retrying when it comes
    /// due, and looking again after `poll` at the longest.
    pub fn run(&self, stop: &AtomicBool) -> Result<()> {
        let watch = crate::watch::Watch::new(&self.queue.arrivals());
        while !stop.load(Ordering::Relaxed) {
            let r = self.run_once(Utc::now())?;
            if !r.is_empty() {
                log::info!(
                    "{} sent, {} to retry, {} failed",
                    r.sent,
                    r.retrying,
                    r.failed
                );
            }
            // a message that arrives after run_once looked raises the watch
            // as well, so nothing is missed between looking and waiting
            let mut wait = self.config.poll;
            if let Some(due) = self.queue.next_due()? {
                let until = (due - Utc::now()).to_std().unwrap_or_default();
                // a moment over, so that it is due when looked at
                wait = wait.min(until + std::time::Duration::from_millis(50));
            }
            watch.wait(wait);
        }
        Ok(())
    }

    fn deliver(&self, claim: Claim, now: DateTime<Utc>) -> Result<Outcome> {
        let (from, recipients) = queue::envelope(&claim.raw);
        let outcome = match from {
            None => Outcome::Permanent("no From address".into()),
            Some(from) => match self.config.command_for(&from) {
                None => Outcome::Permanent(format!("no send command for {from}")),
                Some(command) => {
                    // the wire copy has no Bcc and none of our headers
                    let wire = message::remove_headers(&claim.raw, |h| {
                        h.eq_ignore_ascii_case("Bcc") || queue::is_ours(h)
                    });
                    log::debug!("{}: sending from {from} to {recipients:?}", claim.id);
                    send::run(&send::Job {
                        id: &claim.id,
                        command,
                        from: &from,
                        recipients: &recipients,
                        message: &wire,
                        timeout: self.config.timeout,
                    })
                }
            },
        };
        match &outcome {
            Outcome::Sent => {
                let mut copy = message::remove_headers(&claim.raw, |h| {
                    [queue::LAST_ERROR, queue::NEXT_ATTEMPT, queue::SENT_FOLDER]
                        .iter()
                        .any(|n| n.eq_ignore_ascii_case(h))
                });
                copy = message::set_header(&copy, queue::SENT_AT, &now.to_rfc3339());
                // What happened has happened: if the copy cannot be filed
                // the message must still not go again, so fall back to
                // the queue's own folder, and failing that say so and carry on.
                let dir = message::header(&claim.raw, queue::SENT_FOLDER)
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_absolute())
                    .unwrap_or_else(|| self.config.sent_dir());
                if let Err(e) = queue::file_sent(&dir, &claim.id, &copy) {
                    log::error!(
                        "{}: sent, but could not file a copy in {}: {e:#}",
                        claim.id,
                        dir.display()
                    );
                    let own = self.queue.root().join("sent");
                    if let Err(e) = queue::file_sent(&own, &claim.id, &copy) {
                        log::error!("{}: nor in {}: {e:#}", claim.id, own.display());
                    }
                }
                log::info!("{}: sent", claim.id);
                claim.sent()?;
            }
            Outcome::Temporary(why) => {
                let attempts = queue::attempts(&claim.raw) + 1;
                if attempts >= self.config.max_attempts {
                    let why = format!("gave up after {attempts} attempts: {why}");
                    log::warn!("{}: {why}", claim.id);
                    let raw = queue::bumped(&claim.raw, &why, None, now);
                    claim.failed(&raw)?;
                    return Ok(Outcome::Permanent(why));
                }
                let next = now + self.config.delay_after(attempts);
                log::warn!("{}: {why}; trying again at {}", claim.id, next.to_rfc3339());
                let raw = queue::bumped(&claim.raw, why, Some(next), now);
                claim.retry_later(&raw)?;
            }
            Outcome::Permanent(why) => {
                log::warn!("{}: failed: {why}", claim.id);
                let raw = queue::bumped(&claim.raw, why, None, now);
                claim.failed(&raw)?;
            }
        }
        Ok(outcome)
    }
}

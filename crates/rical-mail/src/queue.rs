//! The send queue: a folder of messages, one file each.
//!
//! ```text
//! new/      waiting: not tried yet, or tried and due again at its next time
//! sending/  a worker has it, and holds a lock on the file
//! failed/   given up on; `retry` puts it back
//! tmp/      files being written, renamed into place when whole
//! sent/     (by default) a Maildir of copies of what went
//! ```
//!
//! A message keeps its file name (its id) for its whole life. State a
//! person might want to see is in headers on the message itself, so any
//! mail reader pointed at a folder here shows it: `X-Rical-Attempts`,
//! `X-Rical-Last-Error`, `X-Rical-Next-Attempt`.
//!
//! Workers need no coordination beyond the file system. To take a message
//! one locks its file, checks the name still leads to that file, and
//! renames it into `sending/`; the lock is held until it is done, and the
//! kernel drops it if the worker dies, which is how a message left in
//! `sending/` by a dead worker is told from one being sent right now.
//! Every step that replaces a file writes the new one before removing the
//! old, so a crash leaves a message twice rather than not at all.

use std::{
    fs::{self, File, TryLockError},
    io::{Read, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};

use anyhow::{Context as _, Result, bail};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::message;

pub const ATTEMPTS: &str = "X-Rical-Attempts";
pub const LAST_ERROR: &str = "X-Rical-Last-Error";
pub const NEXT_ATTEMPT: &str = "X-Rical-Next-Attempt";
pub const SENT_AT: &str = "X-Rical-Sent";

/// Whether a header is the queue's own, and is not for the wire.
pub fn is_ours(name: &str) -> bool {
    name.len() >= 9 && name[..9].eq_ignore_ascii_case("X-Rical-")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Queued,
    Retrying,
    Sending,
    Failed,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            State::Queued => "queued",
            State::Retrying => "retrying",
            State::Sending => "sending",
            State::Failed => "failed",
        }
    }
}

/// What the queue shows about one message.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: String,
    pub state: State,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub attempts: u32,
    pub next_attempt: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

pub fn attempts(raw: &[u8]) -> u32 {
    message::header(raw, ATTEMPTS)
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub fn next_attempt(raw: &[u8]) -> Option<DateTime<Utc>> {
    let v = message::header(raw, NEXT_ATTEMPT)?;
    DateTime::parse_from_rfc3339(&v).ok().map(|d| d.to_utc())
}

/// The sender and the recipients (To, Cc and Bcc) a message names.
pub fn envelope(raw: &[u8]) -> (Option<String>, Vec<String>) {
    let from = message::headers(raw, "From")
        .iter()
        .flat_map(|v| message::addresses(v))
        .next();
    let mut to: Vec<String> = Vec::new();
    for name in ["To", "Cc", "Bcc"] {
        for v in message::headers(raw, name) {
            for a in message::addresses(&v) {
                if !to.iter().any(|t| t.eq_ignore_ascii_case(&a)) {
                    to.push(a);
                }
            }
        }
    }
    (from, to)
}

fn new_id() -> String {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let now = Utc::now();
    format!(
        "{}.{:09}-{}-{}",
        now.timestamp(),
        now.timestamp_subsec_nanos(),
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Write `bytes` as `dir/name` so it appears whole or not at all: into a
/// temporary file in `tmp`, which must be on the same file system.
fn publish(tmp: &Path, dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let temp = tmp.join(name);
    let mut f = File::options()
        .write(true)
        .create_new(true)
        .open(&temp)
        .with_context(|| format!("{}", temp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    fs::rename(&temp, dir.join(name))
        .with_context(|| format!("moving {} into {}", name, dir.display()))?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

/// Put a copy of a sent message in a Maildir (made if it is not there).
pub fn file_sent(maildir: &Path, id: &str, bytes: &[u8]) -> Result<PathBuf> {
    for sub in ["tmp", "new", "cur"] {
        fs::create_dir_all(maildir.join(sub))?;
    }
    let name = format!("{id}.rical:2,S");
    publish(&maildir.join("tmp"), &maildir.join("cur"), &name, bytes)?;
    Ok(maildir.join("cur").join(name))
}

#[derive(Debug, Clone)]
pub struct Queue {
    root: PathBuf,
}

impl Queue {
    pub fn open(root: &Path) -> Result<Queue> {
        for sub in ["tmp", "new", "sending", "failed"] {
            fs::create_dir_all(root.join(sub))
                .with_context(|| format!("making {}", root.join(sub).display()))?;
        }
        Ok(Queue {
            root: root.to_owned(),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn dir(&self, sub: &str) -> PathBuf {
        self.root.join(sub)
    }

    fn publish(&self, sub: &str, id: &str, bytes: &[u8]) -> Result<()> {
        publish(&self.dir("tmp"), &self.dir(sub), id, bytes)
    }

    fn ids(&self, sub: &str) -> Result<Vec<String>> {
        let mut ids: Vec<String> = fs::read_dir(self.dir(sub))?
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| !n.starts_with('.'))
            .collect();
        ids.sort();
        Ok(ids)
    }

    /// Add a message to be sent. It must say who it is from and who it is
    /// to; a `Message-ID` and `Date` are added if it has none, so that a
    /// message sent twice after a crash at least has one identity.
    pub fn enqueue(&self, raw: &[u8]) -> Result<String> {
        let (from, to) = envelope(raw);
        if from.is_none() {
            bail!("the message has no From address");
        }
        if to.is_empty() {
            bail!("the message has no To, Cc or Bcc address");
        }
        let id = new_id();
        let mut raw = raw.to_vec();
        if message::header(&raw, "Date").is_none() {
            raw = message::set_header(&raw, "Date", &Utc::now().to_rfc2822());
        }
        if message::header(&raw, "Message-ID").is_none() {
            let domain = from
                .as_deref()
                .and_then(|f| f.rsplit_once('@'))
                .map_or("rical-mail", |(_, d)| d);
            raw = message::set_header(&raw, "Message-ID", &format!("<{id}@{domain}>"));
        }
        self.publish("new", &id, &raw)?;
        Ok(id)
    }

    /// Everything in the queue, oldest first within each state.
    pub fn list(&self) -> Result<Vec<Entry>> {
        let mut out = Vec::new();
        for (sub, state) in [
            ("sending", State::Sending),
            ("new", State::Queued),
            ("failed", State::Failed),
        ] {
            for id in self.ids(sub)? {
                // it may have moved on since the listing
                let Ok(raw) = fs::read(self.dir(sub).join(&id)) else {
                    continue;
                };
                let attempts = attempts(&raw);
                let state = match state {
                    State::Queued if attempts > 0 => State::Retrying,
                    s => s,
                };
                let (from, _) = envelope(&raw);
                out.push(Entry {
                    id,
                    state,
                    from: from.unwrap_or_default(),
                    to: message::one_line(&message::headers(&raw, "To").join(", "), 60),
                    subject: message::one_line(
                        &message::header(&raw, "Subject").unwrap_or_default(),
                        60,
                    ),
                    attempts,
                    next_attempt: next_attempt(&raw),
                    last_error: message::header(&raw, LAST_ERROR),
                });
            }
        }
        Ok(out)
    }

    /// Take the oldest message that is due at `now`, if there is one.
    pub fn claim_next(&self, now: DateTime<Utc>) -> Result<Option<Claim>> {
        for id in self.ids("new")? {
            let path = self.dir("new").join(&id);
            // Look before locking, so that polling past messages that are
            // waiting for their time takes no locks; then check again
            // under the lock, as it may have changed.
            let due = |raw: &[u8]| !next_attempt(raw).is_some_and(|t| t > now);
            match fs::read(&path) {
                Ok(raw) if !due(&raw) => continue,
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e).with_context(|| format!("{}", path.display())),
            }
            let Some(mut file) = open_locked(&path)? else {
                continue;
            };
            let mut raw = Vec::new();
            file.read_to_end(&mut raw)?;
            if !due(&raw) {
                continue;
            }
            let to = self.dir("sending").join(&id);
            if let Err(e) = fs::rename(&path, &to) {
                if e.kind() == std::io::ErrorKind::NotFound {
                    continue;
                }
                return Err(e).with_context(|| format!("claiming {id}"));
            }
            return Ok(Some(Claim {
                queue: self.clone(),
                id,
                raw,
                _lock: file,
            }));
        }
        Ok(None)
    }

    /// Put back what a worker that died left in `sending/`. Returns how
    /// many.
    pub fn recover_stale(&self, now: DateTime<Utc>) -> Result<usize> {
        let mut n = 0;
        for id in self.ids("sending")? {
            let path = self.dir("sending").join(&id);
            // locked: someone is sending it right now
            let Some(mut file) = open_locked(&path)? else {
                continue;
            };
            let mut raw = Vec::new();
            file.read_to_end(&mut raw)?;
            let raw = bumped(&raw, "the worker stopped while sending it", None, now);
            self.publish("new", &id, &raw)?;
            let _ = fs::remove_file(&path);
            log::warn!("{id}: put back after a worker stopped while sending it");
            n += 1;
        }
        Ok(n)
    }

    /// Move a failed message back to the queue, or all of them. Returns
    /// how many.
    pub fn retry(&self, id: Option<&str>) -> Result<usize> {
        let mut n = 0;
        for name in self.ids("failed")? {
            if id.is_some_and(|i| i != name) {
                continue;
            }
            let path = self.dir("failed").join(&name);
            let raw = fs::read(&path)?;
            let raw = message::remove_headers(&raw, |h| {
                [ATTEMPTS, LAST_ERROR, NEXT_ATTEMPT]
                    .iter()
                    .any(|n| n.eq_ignore_ascii_case(h))
            });
            self.publish("new", &name, &raw)?;
            fs::remove_file(&path)?;
            n += 1;
        }
        if id.is_some() && n == 0 {
            bail!("no failed message {}", id.unwrap_or_default());
        }
        Ok(n)
    }
}

/// Open a file and take its lock, or `None` if someone else has it, or it
/// has gone, or its name now leads somewhere else.
fn open_locked(path: &Path) -> Result<Option<File>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("{}", path.display())),
    };
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Ok(None),
        Err(TryLockError::Error(e)) => return Err(e.into()),
    }
    // the file may have been replaced between opening and locking: only
    // the lock holder replaces, so once it is ours this check is final
    let (Ok(a), Ok(b)) = (file.metadata(), fs::metadata(path)) else {
        return Ok(None);
    };
    Ok((a.ino() == b.ino() && a.dev() == b.dev()).then_some(file))
}

/// The message after one more failed attempt.
pub fn bumped(
    raw: &[u8],
    error: &str,
    next: Option<DateTime<Utc>>,
    _now: DateTime<Utc>,
) -> Vec<u8> {
    let mut out = message::set_header(raw, ATTEMPTS, &(attempts(raw) + 1).to_string());
    out = message::set_header(&out, LAST_ERROR, &message::one_line(error, 300));
    out = match next {
        Some(t) => message::set_header(
            &out,
            NEXT_ATTEMPT,
            &t.to_rfc3339_opts(SecondsFormat::Secs, true),
        ),
        None => message::remove_headers(&out, |h| h.eq_ignore_ascii_case(NEXT_ATTEMPT)),
    };
    out
}

/// A message a worker has taken. Dropping it without finishing leaves the
/// file in `sending/` for [`Queue::recover_stale`].
pub struct Claim {
    queue: Queue,
    pub id: String,
    pub raw: Vec<u8>,
    _lock: File,
}

impl Claim {
    fn remove(self) -> Result<()> {
        fs::remove_file(self.queue.dir("sending").join(&self.id))
            .with_context(|| format!("removing {} from sending/", self.id))
    }

    /// It went: forget it.
    pub fn sent(self) -> Result<()> {
        self.remove()
    }

    /// Wait and try again later, with `raw` (already marked) as the message.
    pub fn retry_later(self, raw: &[u8]) -> Result<()> {
        self.queue.publish("new", &self.id, raw)?;
        self.remove()
    }

    /// Give up, with `raw` (already marked) as the message.
    pub fn failed(self, raw: &[u8]) -> Result<()> {
        self.queue.publish("failed", &self.id, raw)?;
        self.remove()
    }
}

//! A lasting history of what the apps changed, to undo any of it, even
//! several actions back or after a restart.
//!
//! Neither iCalendar nor vCard (nor CalDAV and CardDAV) has a rubbish bin,
//! so this is rical's own: a folder with an append-only log, one JSON
//! object per line, and each change's files as they were before and after,
//! kept as the `.ics` or `.vcf` they are. A deleted file can be fished out
//! by hand as well as restored by undo.
//!
//! ```text
//! history/
//!   log.jsonl                    {"op":"do","id":…,"what":"Deleted “Lunch”",…}
//!   files/<id>/0.before.ics      the first file as it was
//!   files/<id>/0.after.ics       … and as it was left (absent if deleted)
//! ```
//!
//! Undo applies an entry's changes backwards and logs that it did; redo
//! the reverse. Either is refused when a file has changed since (a sync,
//! another app), rather than losing that change. Entries older than the
//! days kept are pruned with their files when the history is opened.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::write::Change;

/// One action: what it was, by which app, and the files it changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    /// When it was done (RFC 3339).
    pub time: String,
    /// The app that did it ("calendar", "contacts"): only it can undo it.
    pub app: String,
    /// What it was, as a person would say: "Deleted “Lunch”".
    pub what: String,
    pub files: Vec<File>,
    /// Undone, and not redone since.
    #[serde(default)]
    pub undone: bool,
}

/// One file an entry changed, and its copies in the history (none when it
/// did not exist before, or was deleted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct File {
    pub path: PathBuf,
    pub before: Option<String>,
    pub after: Option<String>,
}

impl Entry {
    pub fn time(&self) -> Option<DateTime<Utc>> {
        DateTime::parse_from_rfc3339(&self.time)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    }
}

/// A line of the log.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
enum Line {
    Do(Entry),
    Undo { id: String, time: String },
    Redo { id: String, time: String },
}

/// The history kept in a folder, as last read.
pub struct History {
    dir: PathBuf,
    entries: Vec<Entry>,
}

/// The default place: `$XDG_DATA_HOME/rical/history`, shared by the apps.
pub fn default_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .map(|d| d.join("rical/history"))
}

impl History {
    /// Open (or start) the history in `dir`, forgetting entries older than
    /// `keep_days` and their files.
    pub fn open(dir: impl Into<PathBuf>, keep_days: u32) -> anyhow::Result<Self> {
        let dir = dir.into();
        fs::create_dir_all(dir.join("files")).with_context(|| format!("creating {dir:?}"))?;
        let mut h = Self {
            dir,
            entries: Vec::new(),
        };
        h.reload()?;
        h.prune(Utc::now() - TimeDelta::days(keep_days.into()))?;
        Ok(h)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn log(&self) -> PathBuf {
        self.dir.join("log.jsonl")
    }

    /// Read the log again (the other app may have added to it).
    pub fn reload(&mut self) -> anyhow::Result<()> {
        let text = match fs::read_to_string(self.log()) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {:?}", self.log())),
        };
        let mut entries: Vec<Entry> = Vec::new();
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Line>(line) {
                Ok(Line::Do(e)) => entries.push(e),
                Ok(Line::Undo { id, .. }) => {
                    if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                        e.undone = true;
                    }
                }
                Ok(Line::Redo { id, .. }) => {
                    if let Some(e) = entries.iter_mut().find(|e| e.id == id) {
                        e.undone = false;
                    }
                }
                // a line half written when something crashed: skip it
                Err(e) => log::warn!("history line {}: {e}", n + 1),
            }
        }
        self.entries = entries;
        Ok(())
    }

    /// Every entry, oldest first.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    fn append(&self, line: &Line) -> anyhow::Result<()> {
        let mut text = serde_json::to_string(line)?;
        text.push('\n');
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log())
            .with_context(|| format!("opening {:?}", self.log()))?;
        // one write, so two apps appending don't interleave a line
        f.write_all(text.as_bytes())?;
        Ok(())
    }

    /// Keep `changes` (already made) as one entry by `app`, saying `what`.
    pub fn record(&mut self, app: &str, what: &str, changes: &[Change]) -> anyhow::Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let id = uuid::Uuid::new_v4().to_string();
        let folder = self.dir.join("files").join(&id);
        fs::create_dir_all(&folder).with_context(|| format!("creating {folder:?}"))?;
        let mut files = Vec::new();
        for (n, c) in changes.iter().enumerate() {
            let ext = c
                .path
                .extension()
                .map(|e| e.to_string_lossy().into_owned())
                .unwrap_or_else(|| "txt".into());
            let keep = |which: &str, text: &Option<String>| -> anyhow::Result<Option<String>> {
                let Some(text) = text else { return Ok(None) };
                let name = format!("{n}.{which}.{ext}");
                fs::write(folder.join(&name), text)
                    .with_context(|| format!("writing {:?}", folder.join(&name)))?;
                Ok(Some(name))
            };
            files.push(File {
                path: c.path.clone(),
                before: keep("before", &c.before)?,
                after: keep("after", &c.after)?,
            });
        }
        let entry = Entry {
            id,
            time: Utc::now().to_rfc3339(),
            app: app.to_owned(),
            what: what.to_owned(),
            files,
            undone: false,
        };
        self.append(&Line::Do(entry.clone()))?;
        self.entries.push(entry);
        Ok(())
    }

    /// The changes an entry made, read back from its files.
    pub fn changes(&self, id: &str) -> anyhow::Result<Vec<Change>> {
        let entry = self
            .entries
            .iter()
            .find(|e| e.id == id)
            .context("no such entry in the history")?;
        let folder = self.dir.join("files").join(id);
        let read = |name: &Option<String>| -> anyhow::Result<Option<String>> {
            name.as_ref()
                .map(|n| {
                    fs::read_to_string(folder.join(n))
                        .with_context(|| format!("reading {:?} (pruned?)", folder.join(n)))
                })
                .transpose()
        };
        entry
            .files
            .iter()
            .map(|f| {
                Ok(Change {
                    path: f.path.clone(),
                    before: read(&f.before)?,
                    after: read(&f.after)?,
                })
            })
            .collect()
    }

    /// The latest of `app`'s entries not undone: what undo undoes.
    pub fn last_done(&self, app: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .rev()
            .find(|e| e.app == app && !e.undone)
    }

    /// What redo redoes: `app`'s earliest undone entry after its last done
    /// one, so undoing three then redoing goes back in order.
    pub fn next_undone(&self, app: &str) -> Option<&Entry> {
        let mine: Vec<&Entry> = self.entries.iter().filter(|e| e.app == app).collect();
        let last_done = mine.iter().rposition(|e| !e.undone);
        let from = last_done.map_or(0, |i| i + 1);
        mine.get(from).copied().filter(|e| e.undone)
    }

    /// Note that an entry was undone (or redone).
    pub fn mark(&mut self, id: &str, undone: bool) -> anyhow::Result<()> {
        let time = Utc::now().to_rfc3339();
        let id = id.to_owned();
        self.append(&if undone {
            Line::Undo {
                id: id.clone(),
                time,
            }
        } else {
            Line::Redo {
                id: id.clone(),
                time,
            }
        })?;
        if let Some(e) = self.entries.iter_mut().find(|e| e.id == id) {
            e.undone = undone;
        }
        Ok(())
    }

    /// Undo entry `id` (or redo it, if `undo` is false) with `apply`,
    /// which writes one change, refusing if its file isn't as the change
    /// expects. All of an entry's changes go, or none: on a refusal the ones
    /// made are put back.
    pub fn undo_with(
        &mut self,
        id: &str,
        undo: bool,
        mut apply: impl FnMut(&Change) -> anyhow::Result<()>,
    ) -> anyhow::Result<()> {
        let changes = self.changes(id)?;
        let steps: Vec<Change> = if undo {
            changes.iter().rev().map(Change::inverse).collect()
        } else {
            changes
        };
        for (i, step) in steps.iter().enumerate() {
            if let Err(e) = apply(step) {
                for done in steps[..i].iter().rev() {
                    if let Err(back) = apply(&done.inverse()) {
                        log::warn!("could not put {:?} back: {back:#}", done.path);
                    }
                }
                return Err(e);
            }
        }
        self.mark(id, undo)
    }

    /// Forget entries from before `cutoff`, and their files.
    fn prune(&mut self, cutoff: DateTime<Utc>) -> anyhow::Result<()> {
        let old = |e: &Entry| e.time().is_some_and(|t| t < cutoff);
        if !self.entries.iter().any(old) {
            return Ok(());
        }
        for e in self.entries.iter().filter(|e| old(e)) {
            let _ = fs::remove_dir_all(self.dir.join("files").join(&e.id));
        }
        self.entries.retain(|e| !old(e));
        // the log again, without them: the remaining entries as they stand
        let mut text = String::new();
        for e in &self.entries {
            text += &serde_json::to_string(&Line::Do(e.clone()))?;
            text.push('\n');
        }
        crate::write::write_atomic(&self.log(), Some(&text))
    }
}

/// An entry's description from what an app was doing ("delete the
/// event") and what it did it to: "Delete the event · Lunch".
pub fn describe(what: &str, changes: &[Change]) -> String {
    let mut chars = what.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if let Some(s) = subject(changes) {
        out += " · ";
        out += &s;
    }
    out
}

/// What a change is about, for an entry's description: the first
/// `SUMMARY` (an event, task or journal entry) or `FN` (a contact) in it.
pub fn subject(changes: &[Change]) -> Option<String> {
    for c in changes {
        for text in [&c.after, &c.before].into_iter().flatten() {
            for line in text.lines() {
                let (head, value) = match line.split_once(':') {
                    Some(hv) => hv,
                    None => continue,
                };
                let name = head.split(';').next().unwrap_or("");
                let name = name.rsplit('.').next().unwrap_or(name);
                if name.eq_ignore_ascii_case("SUMMARY") || name.eq_ignore_ascii_case("FN") {
                    let v = value.trim().replace("\\,", ",").replace("\\;", ";");
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(path: &Path, before: Option<&str>, after: Option<&str>) -> Change {
        Change {
            path: path.to_owned(),
            before: before.map(str::to_owned),
            after: after.map(str::to_owned),
        }
    }

    #[test]
    fn keeps_undoes_and_survives_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("lunch.ics");
        let mut h = History::open(dir.path().join("history"), 30).unwrap();
        h.record(
            "calendar",
            "Made “Lunch”",
            &[change(&file, None, Some("A"))],
        )
        .unwrap();
        h.record(
            "calendar",
            "Deleted “Lunch”",
            &[change(&file, Some("A"), None)],
        )
        .unwrap();
        h.record("contacts", "Added Sam", &[change(&file, None, Some("S"))])
            .unwrap();

        // each app undoes its own, latest first
        let last = h.last_done("calendar").unwrap().clone();
        assert_eq!(last.what, "Deleted “Lunch”");
        assert_eq!(
            h.changes(&last.id).unwrap(),
            [change(&file, Some("A"), None)]
        );
        h.mark(&last.id, true).unwrap();
        assert_eq!(h.last_done("calendar").unwrap().what, "Made “Lunch”");
        assert_eq!(h.next_undone("calendar").unwrap().id, last.id);

        // after a restart, the same
        let h = History::open(dir.path().join("history"), 30).unwrap();
        assert_eq!(h.entries().len(), 3);
        assert_eq!(h.last_done("calendar").unwrap().what, "Made “Lunch”");
        assert_eq!(h.next_undone("calendar").unwrap().id, last.id);
        assert_eq!(h.last_done("contacts").unwrap().what, "Added Sam");
        // and the deleted file is there to fish out
        let kept = h.dir().join("files").join(&last.id).join("0.before.ics");
        assert_eq!(fs::read_to_string(kept).unwrap(), "A");
    }

    #[test]
    fn redo_goes_back_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.ics");
        let mut h = History::open(dir.path().join("h"), 30).unwrap();
        for what in ["one", "two", "three"] {
            h.record("calendar", what, &[change(&f, None, Some(what))])
                .unwrap();
        }
        let ids: Vec<String> = h.entries().iter().map(|e| e.id.clone()).collect();
        h.mark(&ids[2], true).unwrap();
        h.mark(&ids[1], true).unwrap();
        assert_eq!(h.next_undone("calendar").unwrap().what, "two");
        h.mark(&ids[1], false).unwrap();
        assert_eq!(h.next_undone("calendar").unwrap().what, "three");
    }

    #[test]
    fn old_entries_go_with_their_files() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.ics");
        let mut h = History::open(dir.path().join("h"), 30).unwrap();
        h.record("calendar", "old", &[change(&f, Some("A"), None)])
            .unwrap();
        let id = h.entries()[0].id.clone();
        h.prune(Utc::now() + TimeDelta::seconds(1)).unwrap();
        assert!(h.entries().is_empty());
        assert!(!h.dir().join("files").join(&id).exists());
        let h = History::open(dir.path().join("h"), 30).unwrap();
        assert!(h.entries().is_empty());
    }

    /// Writes a change as Vdir::apply does: only over what it expects.
    fn apply(c: &Change) -> anyhow::Result<()> {
        let now = fs::read_to_string(&c.path).ok();
        anyhow::ensure!(now == c.before, "{:?} changed", c.path);
        crate::write::write_atomic(&c.path, c.after.as_deref())
    }

    #[test]
    fn undoes_any_entry_unless_its_files_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a.ics"), dir.path().join("b.ics"));
        let mut h = History::open(dir.path().join("h"), 30).unwrap();
        // two actions: write a, then (later) change b and delete a
        let first = [change(&a, None, Some("A"))];
        first.iter().for_each(|c| apply(c).unwrap());
        h.record("calendar", "one", &first).unwrap();
        fs::write(&b, "B").unwrap();
        let second = [
            change(&b, Some("B"), Some("B2")),
            change(&a, Some("A"), None),
        ];
        second.iter().for_each(|c| apply(c).unwrap());
        h.record("calendar", "two", &second).unwrap();
        let ids: Vec<String> = h.entries().iter().map(|e| e.id.clone()).collect();

        // the first can't be undone while a is gone: nothing changes
        assert!(h.undo_with(&ids[0], true, apply).is_err());
        assert!(!h.entries()[0].undone);
        // the second can, both files at once
        h.undo_with(&ids[1], true, apply).unwrap();
        assert_eq!(fs::read_to_string(&a).unwrap(), "A");
        assert_eq!(fs::read_to_string(&b).unwrap(), "B");
        // and redone
        h.undo_with(&ids[1], false, apply).unwrap();
        assert!(!a.exists());
        // something else changing b blocks undoing it, leaving a alone
        fs::write(&b, "synced").unwrap();
        assert!(h.undo_with(&ids[1], true, apply).is_err());
        assert!(!a.exists());
        assert_eq!(fs::read_to_string(&b).unwrap(), "synced");
    }

    #[test]
    fn subjects() {
        let c = change(
            Path::new("a.ics"),
            None,
            Some("BEGIN:VEVENT\r\nSUMMARY:Lunch\\, Sam\r\nEND:VEVENT\r\n"),
        );
        assert_eq!(subject(&[c]).as_deref(), Some("Lunch, Sam"));
        let c = change(
            Path::new("a.vcf"),
            Some("BEGIN:VCARD\r\nitem1.FN:Jo Park\r\nEND:VCARD\r\n"),
            None,
        );
        assert_eq!(subject(&[c]).as_deref(), Some("Jo Park"));
    }
}

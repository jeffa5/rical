//! Remembering what the headers of a folder said, so that opening a big
//! folder reads an index (SQLite, see [`rical_index`]) and not every
//! message. A message in Maildir never changes (only its name's flags
//! do), so an entry is the message's folder, unique name and size, and is
//! right until the file goes. Only headers and the preview line are kept:
//! a message's body is read from its file when it is opened.

use std::{
    collections::HashMap,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use rical_index::rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::store::Summary;

/// What of a [`Summary`] comes from the message, not from where it is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Headers {
    pub size: u64,
    pub date: i64,
    pub from_name: String,
    pub from_address: String,
    pub to: String,
    pub subject: String,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub attachments: bool,
    #[serde(default)]
    pub preview: String,
}

impl Headers {
    pub fn of(s: &Summary) -> Headers {
        Headers {
            size: s.size,
            date: s.date,
            from_name: s.from_name.clone(),
            from_address: s.from_address.clone(),
            to: s.to.clone(),
            subject: s.subject.clone(),
            message_id: s.message_id.clone(),
            in_reply_to: s.in_reply_to.clone(),
            references: s.references.clone(),
            attachments: s.attachments,
            preview: s.preview.clone(),
        }
    }

    pub fn into_summary(
        self,
        path: PathBuf,
        folder: &str,
        flags: crate::flags::Flags,
        is_new: bool,
    ) -> Summary {
        Summary {
            path,
            folder: folder.to_owned(),
            flags,
            is_new,
            date: self.date,
            from_name: self.from_name,
            from_address: self.from_address,
            to: self.to,
            subject: self.subject,
            message_id: self.message_id,
            in_reply_to: self.in_reply_to,
            references: self.references,
            size: self.size,
            attachments: self.attachments,
            preview: self.preview,
        }
    }
}

/// Changed whenever what is kept changes: the index is filled again.
const VERSION: i64 = 2;

const TABLES: &str = "
    CREATE TABLE messages (
        folder BLOB NOT NULL,
        name TEXT NOT NULL,
        -- where it is: `new` or `cur`, and its file's name (with its flags)
        sub TEXT NOT NULL,
        file TEXT NOT NULL,
        size INTEGER NOT NULL,
        date INTEGER NOT NULL,
        from_name TEXT NOT NULL,
        from_address TEXT NOT NULL,
        recipients TEXT NOT NULL,
        subject TEXT NOT NULL,
        message_id TEXT,
        -- one per line
        in_reply_to TEXT NOT NULL,
        refs TEXT NOT NULL,
        attachments INTEGER NOT NULL,
        preview TEXT NOT NULL,
        PRIMARY KEY (folder, name)
    ) WITHOUT ROWID;
    -- the folders listed into it, so an empty one is known from one not yet
    CREATE TABLE folders (folder BLOB PRIMARY KEY) WITHOUT ROWID;
";

/// One message as the index has it: where it was, and its headers.
#[derive(Debug, Clone)]
struct Entry {
    sub: String,
    file: String,
    headers: Headers,
}

/// The cache of one folder: unique name → where the message is and its
/// headers, read from the index when it is opened, and what changed
/// written back when it is saved.
#[derive(Default)]
pub struct Cache {
    index: Option<rical_index::Index>,
    folder: Vec<u8>,
    entries: HashMap<String, Entry>,
    /// Whether the folder was listed into the index before.
    listed: bool,
    /// Entries new since it was opened, or that moved (a flag changed).
    changed: Vec<String>,
    /// Names seen this time, to drop what has gone.
    seen: std::collections::HashSet<String>,
}

impl std::fmt::Debug for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cache")
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// Where earlier versions kept the cache of a folder, as JSON.
fn old_file(dir: &Path, folder: &Path) -> PathBuf {
    let name: String = folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let tail = name.len().saturating_sub(120);
    dir.join(format!("{}.v3.json", &name[tail..]))
}

fn lines(v: &[String]) -> String {
    v.join("\n")
}

fn unlines(s: String) -> Vec<String> {
    if s.is_empty() {
        Vec::new()
    } else {
        s.split('\n').map(str::to_owned).collect()
    }
}

/// The index of message headers kept in `dir`.
fn open_index(dir: &Path) -> rical_index::Index {
    rical_index::Index::open(Some(&dir.join("headers.sqlite")), VERSION, TABLES)
}

/// What the index in `dir` had of `folder` when it was last listed: each
/// message's place (`new` or `cur`), file name and headers. `None` if it
/// was never listed.
pub fn indexed(dir: &Path, folder: &Path) -> Option<Vec<(String, String, Headers)>> {
    let index = open_index(dir);
    let folder = folder.as_os_str().as_bytes();
    let listed = read_listed(&index, folder);
    if !listed {
        return None;
    }
    match read(&index, folder) {
        Ok(entries) => Some(
            entries
                .into_values()
                .map(|e| (e.sub, e.file, e.headers))
                .collect(),
        ),
        Err(e) => {
            log::warn!("could not read the header index: {e}");
            None
        }
    }
}

impl Cache {
    /// The cache of `folder`, in the index kept in `dir` (an empty one if
    /// there is none yet).
    pub fn open(dir: &Path, folder: &Path) -> Cache {
        let _ = std::fs::remove_file(old_file(dir, folder));
        let index = open_index(dir);
        let folder = folder.as_os_str().as_bytes().to_vec();
        let entries = read(&index, &folder).unwrap_or_else(|e| {
            log::warn!("could not read the header index: {e}");
            HashMap::new()
        });
        Cache {
            listed: read_listed(&index, &folder),
            index: Some(index),
            folder,
            entries,
            ..Cache::default()
        }
    }

    pub fn none() -> Cache {
        Cache::default()
    }

    /// The headers of message `unique`, if it is known at this `size`;
    /// noting where it is now (`sub`, `file`).
    pub fn get(&mut self, unique: &str, size: u64, sub: &str, file: &str) -> Option<&Headers> {
        self.seen.insert(unique.to_owned());
        let e = self.entries.get_mut(unique)?;
        if e.headers.size != size {
            return None;
        }
        if e.sub != sub || e.file != file {
            sub.clone_into(&mut e.sub);
            file.clone_into(&mut e.file);
            self.changed.push(unique.to_owned());
        }
        Some(&e.headers)
    }

    pub fn put(&mut self, unique: &str, h: Headers, sub: &str, file: &str) {
        self.seen.insert(unique.to_owned());
        self.entries.insert(
            unique.to_owned(),
            Entry {
                sub: sub.to_owned(),
                file: file.to_owned(),
                headers: h,
            },
        );
        self.changed.push(unique.to_owned());
    }

    /// Write what changed back, and forget what is gone.
    pub fn save(self) {
        let Some(index) = &self.index else { return };
        let gone: Vec<&String> = self
            .entries
            .keys()
            .filter(|k| !self.seen.contains(*k))
            .collect();
        if gone.is_empty() && self.changed.is_empty() && self.listed {
            return;
        }
        if let Err(e) = write(index, &self.folder, &self.entries, &self.changed, &gone) {
            log::warn!("could not keep the headers: {e}");
        }
    }
}

fn read_listed(index: &rical_index::Index, folder: &[u8]) -> bool {
    index
        .conn()
        .query_row(
            "SELECT EXISTS (SELECT 1 FROM folders WHERE folder = ?1)",
            [folder],
            |r| r.get(0),
        )
        .unwrap_or(false)
}

fn read(
    index: &rical_index::Index,
    folder: &[u8],
) -> rical_index::rusqlite::Result<HashMap<String, Entry>> {
    let conn = index.conn();
    let mut st = conn.prepare_cached(
        "SELECT name, sub, file, size, date, from_name, from_address, recipients, subject,
                message_id, in_reply_to, refs, attachments, preview
         FROM messages WHERE folder = ?1",
    )?;
    let rows = st.query_map([folder], |r| {
        Ok((
            r.get::<_, String>(0)?,
            Entry {
                sub: r.get(1)?,
                file: r.get(2)?,
                headers: Headers {
                    size: r.get::<_, i64>(3)? as u64,
                    date: r.get(4)?,
                    from_name: r.get(5)?,
                    from_address: r.get(6)?,
                    to: r.get(7)?,
                    subject: r.get(8)?,
                    message_id: r.get(9)?,
                    in_reply_to: unlines(r.get(10)?),
                    references: unlines(r.get(11)?),
                    attachments: r.get(12)?,
                    preview: r.get(13)?,
                },
            },
        ))
    })?;
    rows.collect()
}

fn write(
    index: &rical_index::Index,
    folder: &[u8],
    entries: &HashMap<String, Entry>,
    changed: &[String],
    gone: &[&String],
) -> rical_index::rusqlite::Result<()> {
    let mut conn = index.conn();
    let tx = conn.transaction()?;
    {
        let mut delete =
            tx.prepare_cached("DELETE FROM messages WHERE folder = ?1 AND name = ?2")?;
        for name in gone {
            delete.execute(params![folder, name])?;
        }
        let mut put = tx.prepare_cached(
            "INSERT OR REPLACE INTO messages (folder, name, sub, file, size, date, from_name,
                from_address, recipients, subject, message_id, in_reply_to, refs, attachments,
                preview)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        )?;
        for name in changed {
            let Some(e) = entries.get(name) else { continue };
            let h = &e.headers;
            put.execute(params![
                folder,
                name,
                e.sub,
                e.file,
                h.size as i64,
                h.date,
                h.from_name,
                h.from_address,
                h.to,
                h.subject,
                h.message_id,
                lines(&h.in_reply_to),
                lines(&h.references),
                h.attachments,
                h.preview
            ])?;
        }
        tx.prepare_cached("INSERT OR IGNORE INTO folders (folder) VALUES (?1)")?
            .execute([folder])?;
    }
    tx.commit()
}

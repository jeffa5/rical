//! Remembering what the headers of a folder said, so that opening a big
//! folder reads a cache file and not every message. A message in Maildir
//! never changes (only its name's flags do), so an entry is the message's
//! unique name and its size, and is right until the file goes.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

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

/// The cache of one folder: unique name → headers.
#[derive(Debug, Default)]
pub struct Cache {
    file: Option<PathBuf>,
    entries: HashMap<String, Headers>,
    changed: bool,
    /// Names seen this time, to drop what has gone.
    seen: std::collections::HashSet<String>,
}

fn file_for(dir: &Path, folder: &Path) -> PathBuf {
    // one file per folder, named by where it is
    let name: String = folder
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let tail = name.len().saturating_sub(120);
    // v3: with the preview line (read as the reader reads it)
    dir.join(format!("{}.v3.json", &name[tail..]))
}

impl Cache {
    /// The cache of `folder` kept in `dir` (an empty one if there is none yet).
    pub fn open(dir: &Path, folder: &Path) -> Cache {
        let file = file_for(dir, folder);
        let entries = fs::read(&file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Cache {
            file: Some(file),
            entries,
            changed: false,
            seen: Default::default(),
        }
    }

    pub fn none() -> Cache {
        Cache::default()
    }

    pub fn get(&mut self, unique: &str, size: u64) -> Option<&Headers> {
        self.seen.insert(unique.to_owned());
        self.entries.get(unique).filter(|h| h.size == size)
    }

    pub fn put(&mut self, unique: &str, h: Headers) {
        self.seen.insert(unique.to_owned());
        self.entries.insert(unique.to_owned(), h);
        self.changed = true;
    }

    /// Write it back if it changed, without what is gone.
    pub fn save(mut self) {
        let before = self.entries.len();
        let seen = std::mem::take(&mut self.seen);
        self.entries.retain(|k, _| seen.contains(k));
        if self.entries.len() != before {
            self.changed = true;
        }
        let (Some(file), true) = (self.file, self.changed) else {
            return;
        };
        let Ok(bytes) = serde_json::to_vec(&self.entries) else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if fs::write(&tmp, bytes).is_ok() {
            let _ = fs::rename(&tmp, &file);
        }
    }
}

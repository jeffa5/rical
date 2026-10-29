//! A tree of Maildir folders: finding them, listing what is in each, and
//! the changes mail clients make (flags, moving, deleting) as renames, so
//! other programs working on the same folders (mbsync, a second client)
//! see them as they would from any Maildir client.

use std::{
    fs::{self, File},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};

use anyhow::{Context as _, Result, bail};
use mail_parser::MessageParser;

use crate::flags::{self, Flags, name_with, split_name};

/// What a folder is for, from its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
    Other,
}

impl Role {
    fn of(name: &str) -> Role {
        let last = name.rsplit('/').next().unwrap_or(name).to_lowercase();
        match last.as_str() {
            "inbox" => Role::Inbox,
            "drafts" | "draft" => Role::Drafts,
            "sent" | "sent items" | "sent mail" | "sent messages" => Role::Sent,
            "archive" | "archives" | "all mail" => Role::Archive,
            "junk" | "spam" | "junk email" | "bulk mail" => Role::Junk,
            "trash" | "deleted" | "deleted items" | "bin" | "deleted messages" => Role::Trash,
            _ => Role::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folder {
    /// Shown and typed: `INBOX`, `Sent`, `work/Projects`.
    pub name: String,
    pub path: PathBuf,
    pub role: Role,
    pub unread: usize,
    pub total: usize,
}

fn is_maildir(p: &Path) -> bool {
    p.join("cur").is_dir() && p.join("new").is_dir()
}

fn count(path: &Path) -> (usize, usize) {
    let (mut unread, mut total) = (0, 0);
    if let Ok(rd) = fs::read_dir(path.join("new")) {
        let n = rd
            .flatten()
            .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
            .count();
        unread += n;
        total += n;
    }
    if let Ok(rd) = fs::read_dir(path.join("cur")) {
        for e in rd.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            total += 1;
            if !split_name(&name).1.seen {
                unread += 1;
            }
        }
    }
    (unread, total)
}

/// Everything under `root` that is a Maildir, in Maildir++ (`.Sent`,
/// `.work.Projects`) or plain nested (`Sent`, `work/Projects`) layout.
pub fn folders(root: &Path) -> Result<Vec<Folder>> {
    let mut out = Vec::new();
    if is_maildir(root) {
        out.push(make("INBOX", root));
    }
    walk(root, "", 0, &mut out)?;
    out.sort_by_key(|a| (a.role, a.name.to_lowercase()));
    Ok(out)
}

fn make(name: &str, path: &Path) -> Folder {
    let (unread, total) = count(path);
    Folder {
        name: name.to_owned(),
        path: path.to_owned(),
        role: Role::of(name),
        unread,
        total,
    }
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<Folder>) -> Result<()> {
    if depth > 4 {
        return Ok(());
    }
    let mut entries: Vec<_> = fs::read_dir(dir)
        .with_context(|| format!("{}", dir.display()))?
        .flatten()
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name().to_string_lossy().into_owned();
        if matches!(name.as_str(), "cur" | "new" | "tmp") || !e.path().is_dir() {
            continue;
        }
        // Maildir++: `.a.b` is a/b inside the root
        let shown = name
            .strip_prefix('.')
            .map_or(name.clone(), |n| n.replace('.', "/"));
        if name == ".." || name == "." {
            continue;
        }
        let full = if prefix.is_empty() {
            shown
        } else {
            format!("{prefix}/{shown}")
        };
        if is_maildir(&e.path()) {
            out.push(make(&full, &e.path()));
        }
        // a folder can hold folders (plain layout); plain directories too
        walk(&e.path(), &full, depth + 1, out)?;
    }
    Ok(())
}

/// What the list of a folder shows of one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub path: PathBuf,
    pub folder: String,
    pub flags: Flags,
    /// Still in `new/`: never opened by any client.
    pub is_new: bool,
    /// Seconds since the epoch: the Date header, else when the file changed.
    pub date: i64,
    pub from_name: String,
    pub from_address: String,
    pub to: String,
    pub subject: String,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub size: u64,
    pub attachments: bool,
}

impl Summary {
    pub fn unread(&self) -> bool {
        !self.flags.seen
    }

    /// The sender as a list shows them.
    pub fn from_shown(&self) -> &str {
        if self.from_name.is_empty() {
            &self.from_address
        } else {
            &self.from_name
        }
    }
}

fn read_head(path: &Path) -> std::io::Result<(Vec<u8>, u64, i64)> {
    let f = File::open(path)?;
    let meta = f.metadata()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let mut buf = Vec::new();
    f.take(48 * 1024).read_to_end(&mut buf)?;
    Ok((buf, meta.len(), mtime))
}

/// Read one message's headers. `None` for a file that is not a message.
pub fn summarize(path: &Path, folder: &str, is_new: bool) -> Option<Summary> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (_, mut flags) = split_name(&name);
    if is_new {
        flags.seen = false;
    }
    let (head, size, mtime) = read_head(path).ok()?;
    let msg = MessageParser::new().parse_headers(&head)?;
    let root = msg.root();
    let from = root.from().and_then(|l| l.first());
    let to = root
        .to()
        .map(|l| {
            l.mailboxes()
                .map(|m| {
                    m.name()
                        .filter(|n| !n.is_empty())
                        .or(m.address())
                        .unwrap_or("")
                        .to_owned()
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let attachments = root
        .content_type()
        .is_some_and(|c| c.ctype().eq_ignore_ascii_case("multipart") && matches!(c.subtype(), Some(s) if s.eq_ignore_ascii_case("mixed") || s.eq_ignore_ascii_case("report")));
    Some(Summary {
        path: path.to_owned(),
        folder: folder.to_owned(),
        flags,
        is_new,
        date: root.date().map_or(mtime, |d| d.to_timestamp()),
        from_name: from
            .and_then(|m| m.name())
            .unwrap_or_default()
            .trim()
            .to_owned(),
        from_address: from
            .and_then(|m| m.address())
            .unwrap_or_default()
            .to_owned(),
        to,
        subject: root.subject().unwrap_or_default().trim().to_owned(),
        message_id: root.message_id().map(str::to_owned),
        in_reply_to: root
            .in_reply_to()
            .map(|l| l.iter().map(str::to_owned).collect())
            .unwrap_or_default(),
        references: root
            .references()
            .map(|l| l.iter().map(str::to_owned).collect())
            .unwrap_or_default(),
        size,
        attachments,
    })
}

/// Every message in a folder, newest first.
pub fn list(folder: &Folder) -> Vec<Summary> {
    let mut out = Vec::new();
    for (sub, is_new) in [("new", true), ("cur", false)] {
        let Ok(rd) = fs::read_dir(folder.path.join(sub)) else {
            continue;
        };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            match summarize(&e.path(), &folder.name, is_new) {
                Some(s) => out.push(s),
                None => log::debug!("{}: not a message", e.path().display()),
            }
        }
    }
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.path.cmp(&b.path)));
    out
}

/// A name for a new message that no other has.
pub fn unique_name() -> String {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let host = std::env::var("HOSTNAME")
        .ok()
        .or_else(|| fs::read_to_string("/etc/hostname").ok())
        .unwrap_or_else(|| "localhost".to_owned());
    let host: String = host
        .trim()
        .chars()
        .map(|c| if matches!(c, '/' | ':') { '_' } else { c })
        .collect();
    format!(
        "{}.M{}P{}Q{}.{host}",
        now.as_secs(),
        now.subsec_micros(),
        std::process::id(),
        COUNT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Put a message in a folder (as `cur/` with `flags`, or `new/` if it is
/// to count as unread) and return where it is. Written to `tmp/` first.
pub fn deliver(folder: &Path, bytes: &[u8], flags: Flags, into_new: bool) -> Result<PathBuf> {
    for sub in ["tmp", "new", "cur"] {
        fs::create_dir_all(folder.join(sub))?;
    }
    let unique = unique_name();
    let temp = folder.join("tmp").join(&unique);
    let mut f = File::options().write(true).create_new(true).open(&temp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    let to = if into_new {
        folder.join("new").join(&unique)
    } else {
        folder.join("cur").join(name_with(&unique, flags))
    };
    fs::rename(&temp, &to)?;
    Ok(to)
}

fn folder_of(path: &Path) -> Result<&Path> {
    path.parent()
        .and_then(Path::parent)
        .context("not in a Maildir folder")
}

/// Give a message these flags: renamed in `cur/` (and moved there from
/// `new/`). Returns the new path.
pub fn set_flags(path: &Path, flags: Flags) -> Result<PathBuf> {
    let name = path
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .into_owned();
    let (unique, _) = split_name(&name);
    let to = folder_of(path)?.join("cur").join(name_with(unique, flags));
    if to != path {
        fs::rename(path, &to).with_context(|| format!("{}", path.display()))?;
    }
    Ok(to)
}

/// The flags of a message by its path.
pub fn flags_of(path: &Path) -> Flags {
    path.file_name()
        .map(|n| split_name(&n.to_string_lossy()).1)
        .unwrap_or_default()
}

/// Move a message to another folder, keeping its flags (an unopened one
/// stays unopened). Returns the new path.
pub fn move_to(path: &Path, dest: &Path) -> Result<PathBuf> {
    if folder_of(path)? == dest {
        bail!("it is already there");
    }
    for sub in ["tmp", "new", "cur"] {
        fs::create_dir_all(dest.join(sub))?;
    }
    let in_new = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|n| n == "new");
    let name = path
        .file_name()
        .context("no file name")?
        .to_string_lossy()
        .into_owned();
    let (unique, f) = split_name(&name);
    let mut to = if in_new {
        dest.join("new").join(unique)
    } else {
        dest.join("cur").join(name_with(unique, f))
    };
    if to.exists() {
        // the same name from another folder: take a new one
        let fresh = unique_name();
        to = if in_new {
            dest.join("new").join(fresh)
        } else {
            dest.join("cur").join(name_with(&fresh, f))
        };
    }
    // rename needs one file system; mail trees nearly always are
    match fs::rename(path, &to) {
        Ok(()) => {}
        Err(_) => {
            fs::copy(path, &to)?;
            fs::remove_file(path)?;
        }
    }
    Ok(to)
}

/// Delete a message for good.
pub fn remove(path: &Path) -> Result<()> {
    fs::remove_file(path).with_context(|| format!("{}", path.display()))
}

#[allow(unused_imports)]
use flags as _;

#[cfg(test)]
mod tests {
    use super::*;

    const MAIL: &[u8] = b"From: Ann Lee <ann@x.example>\nTo: me@y.example\nSubject: Hello there\nDate: Thu, 5 Nov 2026 14:00:00 +0000\nMessage-ID: <a1@x.example>\n\nHi\n";

    #[test]
    fn finds_folders_in_both_layouts() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        for f in ["INBOX", "Sent", "work/Projects", ".Drafts", ".a.b"] {
            for s in ["cur", "new", "tmp"] {
                fs::create_dir_all(r.join(f).join(s)).unwrap();
            }
        }
        let names: Vec<String> = folders(r).unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["INBOX", "Drafts", "Sent", "a/b", "work/Projects"]);
    }

    #[test]
    fn lists_marks_moves_and_counts() {
        let d = tempfile::tempdir().unwrap();
        let inbox = d.path().join("INBOX");
        let archive = d.path().join("Archive");
        deliver(&inbox, MAIL, Flags::default(), true).unwrap();
        let fs_ = folders(d.path()).unwrap();
        assert_eq!((fs_[0].unread, fs_[0].total), (1, 1));
        let list_ = list(&fs_[0]);
        let m = &list_[0];
        assert!(m.is_new && m.unread());
        assert_eq!(
            (m.from_shown(), m.subject.as_str()),
            ("Ann Lee", "Hello there")
        );
        assert_eq!(m.date, 1793887200);
        assert_eq!(m.message_id.as_deref(), Some("a1@x.example"));

        let p = set_flags(
            &m.path,
            Flags {
                seen: true,
                flagged: true,
                ..Flags::default()
            },
        )
        .unwrap();
        assert!(p.starts_with(inbox.join("cur")) && p.to_string_lossy().ends_with(":2,FS"));
        assert_eq!(folders(d.path()).unwrap()[0].unread, 0);

        let q = move_to(&p, &archive).unwrap();
        assert!(q.starts_with(archive.join("cur")));
        assert!(move_to(&q, &archive).is_err());
        let now = folders(d.path()).unwrap();
        assert_eq!((now[0].name.as_str(), now[0].total), ("INBOX", 0));
        assert!(list(&now[1])[0].flags.flagged);
        remove(&q).unwrap();
    }
}

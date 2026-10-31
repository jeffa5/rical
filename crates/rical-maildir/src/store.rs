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
    folders_counted(root, &count)
}

/// [`folders`], with their counts as the header index kept in `dir` has
/// them (as last counted, or listed) rather than looking at any message:
/// quick, to show before [`folders_kept`] counts them. One it has no
/// count of shows none until then: counting hundreds of folders here,
/// on the way to the window, is seconds from a cold disk.
pub fn folders_indexed(root: &Path, dir: &Path) -> Result<Vec<Folder>> {
    let counts = crate::cache::counts(dir);
    folders_counted(root, &|p: &Path| {
        counts
            .get(p.as_os_str().as_encoded_bytes())
            .copied()
            .unwrap_or_default()
    })
}

/// [`folders`], keeping their counts in the header index in `dir` for
/// [`folders_indexed`] to show next time.
pub fn folders_kept(root: &Path, dir: &Path) -> Result<Vec<Folder>> {
    let found = folders(root)?;
    let counts: Vec<(Vec<u8>, usize, usize)> = found
        .iter()
        .map(|f| {
            (
                f.path.as_os_str().as_encoded_bytes().to_vec(),
                f.unread,
                f.total,
            )
        })
        .collect();
    crate::cache::keep_counts(dir, &counts);
    Ok(found)
}

/// The folders, with (unread, total) of each from `count`.
fn folders_counted(root: &Path, count: &dyn Fn(&Path) -> (usize, usize)) -> Result<Vec<Folder>> {
    let mut out = Vec::new();
    if is_maildir(root) {
        out.push(make("INBOX", root, count));
    }
    walk(root, "", 0, &mut out, count)?;
    out.sort_by_key(|a| (a.role, a.name.to_lowercase()));
    Ok(out)
}

fn make(name: &str, path: &Path, count: &dyn Fn(&Path) -> (usize, usize)) -> Folder {
    let (unread, total) = count(path);
    Folder {
        name: name.to_owned(),
        path: path.to_owned(),
        role: Role::of(name),
        unread,
        total,
    }
}

fn walk(
    dir: &Path,
    prefix: &str,
    depth: usize,
    out: &mut Vec<Folder>,
    count: &dyn Fn(&Path) -> (usize, usize),
) -> Result<()> {
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
            out.push(make(&full, &e.path(), count));
        }
        // a folder can hold folders (plain layout); plain directories too
        walk(&e.path(), &full, depth + 1, out, count)?;
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
    /// The start of what it says, on one line, for the list.
    pub preview: String,
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

/// The start of a message (`bytes` of it), its size and when it changed.
fn read_head(path: &Path, bytes: u64) -> std::io::Result<(Vec<u8>, u64, i64)> {
    let f = File::open(path)?;
    let meta = f.metadata()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64);
    let mut buf = Vec::new();
    f.take(bytes).read_to_end(&mut buf)?;
    Ok((buf, meta.len(), mtime))
}

/// How much of a message is read for the list: enough for the headers and
/// a preview line from most. One whose preview is not in that (a
/// newsletter whose styles come first) is read further, up to `MOST`.
const HEAD: u64 = 16 * 1024;
const MOST: u64 = 48 * 1024;

/// Read one message's headers. `None` for a file that is not a message.
pub fn summarize(path: &Path, folder: &str, is_new: bool) -> Option<Summary> {
    let s = summarize_from(path, folder, is_new, HEAD)?;
    if s.preview.is_empty() && s.size > HEAD {
        return summarize_from(path, folder, is_new, MOST);
    }
    Some(s)
}

fn summarize_from(path: &Path, folder: &str, is_new: bool, bytes: u64) -> Option<Summary> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (_, mut flags) = split_name(&name);
    if is_new {
        flags.seen = false;
    }
    let (head, size, mtime) = read_head(path, bytes).ok()?;
    // the head and the start of the body: enough for the list's preview line
    let msg = MessageParser::new().parse(&head)?;
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
        preview: preview_of(&root),
    })
}

/// Every message in a folder, newest first.
pub fn list(folder: &Folder) -> Vec<Summary> {
    list_cached(folder, crate::cache::Cache::none())
}

/// The same, reading headers only for messages the cache does not know,
/// and saving it again.
pub fn list_cached(folder: &Folder, mut cache: crate::cache::Cache) -> Vec<Summary> {
    let mut out = Vec::new();
    // those the cache does not know are read afterwards, on every core: a
    // folder of thousands read cold is mostly waiting for the disk
    let mut unknown: Vec<(PathBuf, &str, bool, String)> = Vec::new();
    for (sub, is_new) in [("new", true), ("cur", false)] {
        let Ok(rd) = fs::read_dir(folder.path.join(sub)) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let (unique, mut flags) = split_name(&name);
            if is_new {
                flags.seen = false;
            }
            let size = e.metadata().map_or(0, |m| m.len());
            if let Some(h) = cache.get(unique, size, sub, &name) {
                out.push(
                    h.clone()
                        .into_summary(e.path(), &folder.name, flags, is_new),
                );
                continue;
            }
            unknown.push((e.path(), sub, is_new, name));
        }
    }
    let read = rical_index::on_every_core(&unknown, |(path, _, is_new, _)| {
        summarize(path, &folder.name, *is_new)
    });
    for ((path, sub, _, name), s) in unknown.iter().zip(read) {
        match s {
            Some(s) => {
                let (unique, _) = split_name(name);
                cache.put(unique, crate::cache::Headers::of(&s), sub, name);
                out.push(s);
            }
            None => log::debug!("{}: not a message", path.display()),
        }
    }
    cache.save();
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.path.cmp(&b.path)));
    out
}

/// The messages of a folder as the header index (kept in `dir`) had them
/// when the folder was last listed, newest first, without looking at the
/// folder: to show at once, before [`list_cached`] catches up with it.
/// `None` if the folder was never listed into it.
pub fn list_indexed(folder: &Folder, dir: &Path) -> Option<Vec<Summary>> {
    let mut out: Vec<Summary> = crate::cache::indexed(dir, &folder.path)?
        .into_iter()
        .map(|(sub, file, h)| {
            let is_new = sub == "new";
            let (_, mut flags) = split_name(&file);
            if is_new {
                flags.seen = false;
            }
            let path = folder.path.join(&sub).join(&file);
            h.into_summary(path, &folder.name, flags, is_new)
        })
        .collect();
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.path.cmp(&b.path)));
    Some(out)
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

#[cfg(test)]
mod cache_tests {
    use super::*;

    #[test]
    fn a_second_listing_comes_from_the_cache() {
        let d = tempfile::tempdir().unwrap();
        let inbox = d.path().join("INBOX");
        for i in 0..50 {
            let raw = format!(
                "From: A <a{i}@x.example>\nSubject: Hello {i}\nDate: Thu, 5 Nov 2026 14:00:{:02} +0000\nMessage-ID: <m{i}@x>\n\nHi\n",
                i % 60
            );
            deliver(&inbox, raw.as_bytes(), Flags::default(), true).unwrap();
        }
        let folder = &folders(d.path()).unwrap()[0];
        let cache_dir = d.path().join("cache");
        let first = list_cached(folder, crate::cache::Cache::open(&cache_dir, &folder.path));
        assert!(cache_dir.join("headers.sqlite").is_file());
        // break every message: only the cache can answer now
        for e in fs::read_dir(inbox.join("new")).unwrap().flatten() {
            let len = e.metadata().unwrap().len() as usize;
            fs::write(e.path(), vec![b'x'; len]).unwrap();
        }
        let second = list_cached(folder, crate::cache::Cache::open(&cache_dir, &folder.path));
        assert_eq!(first, second);
        assert_eq!(second.len(), 50);
        // a message that goes leaves the cache
        let gone = second[0].path.clone();
        fs::remove_file(&gone).unwrap();
        assert_eq!(
            list_cached(folder, crate::cache::Cache::open(&cache_dir, &folder.path)).len(),
            49
        );
    }

    #[test]
    fn the_index_alone_shows_a_folder_as_last_listed() {
        let d = tempfile::tempdir().unwrap();
        let inbox = d.path().join("INBOX");
        for i in 0..5 {
            let raw = format!(
                "From: A <a{i}@x.example>\nSubject: Hello {i}\nDate: Thu, 5 Nov 2026 14:00:0{i} +0000\n\nHi\n"
            );
            deliver(&inbox, raw.as_bytes(), Flags::default(), true).unwrap();
        }
        let folder = &folders(d.path()).unwrap()[0];
        let cache_dir = d.path().join("cache");
        // never listed: nothing to show from it
        assert!(list_indexed(folder, &cache_dir).is_none());
        let listed = list_cached(folder, crate::cache::Cache::open(&cache_dir, &folder.path));
        assert_eq!(list_indexed(folder, &cache_dir).unwrap(), listed);

        // read one (it moves to cur/ with its flag), and one goes
        let read = move_to_cur_seen(&listed[0].path);
        fs::remove_file(&listed[1].path).unwrap();
        // the index still says what it said, until the folder is listed
        assert_eq!(list_indexed(folder, &cache_dir).unwrap(), listed);
        let now = list_cached(folder, crate::cache::Cache::open(&cache_dir, &folder.path));
        assert_eq!(now.len(), 4);
        assert_eq!(list_indexed(folder, &cache_dir).unwrap(), now);
        let first = &list_indexed(folder, &cache_dir).unwrap()[0];
        assert_eq!(first.path, read);
        assert!(first.flags.seen && !first.is_new);
    }

    #[test]
    fn folders_are_counted_from_the_index() {
        let d = tempfile::tempdir().unwrap();
        let inbox = d.path().join("INBOX");
        for seen in [false, false, true] {
            let raw = "From: a@x.example\nSubject: Hi\n\nHi\n";
            let flags = Flags {
                seen,
                ..Flags::default()
            };
            deliver(&inbox, raw.as_bytes(), flags, !seen).unwrap();
        }
        fs::create_dir_all(d.path().join("Other/cur")).unwrap();
        let cache_dir = d.path().join("cache");
        let counted = |fs: Vec<Folder>| -> Vec<(String, usize, usize)> {
            fs.into_iter()
                .map(|f| (f.name, f.unread, f.total))
                .collect()
        };
        let real = counted(folders(d.path()).unwrap());
        // never counted: nothing, rather than counting on the way to the window
        assert!(
            counted(folders_indexed(d.path(), &cache_dir).unwrap())
                .iter()
                .all(|(_, unread, total)| (*unread, *total) == (0, 0))
        );
        // counted away from it, and kept for next time
        assert_eq!(counted(folders_kept(d.path(), &cache_dir).unwrap()), real);
        assert_eq!(
            counted(folders_indexed(d.path(), &cache_dir).unwrap()),
            real
        );
        let inbox_folder = folders(d.path())
            .unwrap()
            .into_iter()
            .find(|f| f.path == inbox)
            .unwrap();
        // the index says what it was told, until listed (or counted) again
        deliver(&inbox, b"Subject: More\n\nx\n", Flags::default(), true).unwrap();
        let stale = counted(folders_indexed(d.path(), &cache_dir).unwrap());
        assert_eq!(stale, real);
        list_cached(&inbox_folder, crate::cache::Cache::open(&cache_dir, &inbox));
        assert_eq!(
            counted(folders_indexed(d.path(), &cache_dir).unwrap()),
            counted(folders(d.path()).unwrap())
        );
    }

    /// Mark a message in new/ read, as a mail client does.
    fn move_to_cur_seen(path: &Path) -> PathBuf {
        let folder = path.parent().unwrap().parent().unwrap();
        let cur = folder.join("cur").join(format!(
            "{}:2,S",
            path.file_name().unwrap().to_string_lossy()
        ));
        fs::rename(path, &cur).unwrap();
        cur
    }
}

#[cfg(test)]
mod timing {
    use super::*;

    /// `cargo test -p rical-maildir --release big_folder -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn big_folder_timing() {
        let d = tempfile::tempdir().unwrap();
        let inbox = d.path().join("INBOX");
        for i in 0..20_000 {
            let raw = format!(
                "From: A <a{i}@x.example>\nTo: me@y.example\nSubject: Hello number {i}\nDate: Thu, 5 Nov 2026 14:00:00 +0000\nMessage-ID: <m{i}@x>\n\n{}\n",
                "body text ".repeat(40)
            );
            deliver(
                &inbox,
                raw.as_bytes(),
                Flags {
                    seen: true,
                    ..Flags::default()
                },
                false,
            )
            .unwrap();
        }
        let folder = &folders(d.path()).unwrap()[0];
        let cache_dir = d.path().join("cache");
        for pass in ["cold", "warm"] {
            let t = std::time::Instant::now();
            let n = list_cached(folder, crate::cache::Cache::open(&cache_dir, &folder.path)).len();
            println!("{pass}: {n} messages in {:?}", t.elapsed());
        }
        let t = std::time::Instant::now();
        let rows = crate::thread::threads(&list(folder));
        println!(
            "uncached list + threads: {} rows in {:?}",
            rows.len(),
            t.elapsed()
        );
    }
}

/// Make a folder (a Maildir) called `name`, in the layout the tree uses:
/// `.a.b` beside the others if they are that way (Maildir++), else nested.
pub fn create_folder(root: &Path, name: &str) -> Result<PathBuf> {
    let name = name.trim().trim_matches('/');
    if name.is_empty()
        || name
            .split('/')
            .any(|p| p.is_empty() || p == "." || p == "..")
    {
        bail!("not a folder name: “{name}”");
    }
    let dotted = fs::read_dir(root)
        .map(|rd| {
            rd.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with('.') && n.len() > 1 && is_maildir(&e.path())
            })
        })
        .unwrap_or(false);
    let path = if dotted {
        root.join(format!(".{}", name.replace('/', ".")))
    } else {
        root.join(name)
    };
    if is_maildir(&path) {
        bail!("there is already a folder “{name}”");
    }
    for sub in ["tmp", "new", "cur"] {
        fs::create_dir_all(path.join(sub))?;
    }
    Ok(path)
}

#[cfg(test)]
mod folder_tests {
    use super::*;

    #[test]
    fn new_folders_follow_the_layout() {
        let d = tempfile::tempdir().unwrap();
        for s in ["cur", "new", "tmp"] {
            fs::create_dir_all(d.path().join("INBOX").join(s)).unwrap();
        }
        let p = create_folder(d.path(), "work/Projects").unwrap();
        assert_eq!(p, d.path().join("work/Projects"));
        assert!(create_folder(d.path(), "work/Projects").is_err());
        assert!(create_folder(d.path(), "../x").is_err());

        let dotted = tempfile::tempdir().unwrap();
        for s in ["cur", "new", "tmp"] {
            fs::create_dir_all(dotted.path().join(".Sent").join(s)).unwrap();
        }
        let p = create_folder(dotted.path(), "work/Projects").unwrap();
        assert_eq!(p, dotted.path().join(".work.Projects"));
        assert!(
            folders(dotted.path())
                .unwrap()
                .iter()
                .any(|f| f.name == "work/Projects")
        );
    }
}

/// The messages a search left as a Maildir of symlinks (mu's links format):
/// each is the message it points to, so changing one changes the real file.
pub fn list_links(dir: &Path) -> Vec<Summary> {
    let mut out = Vec::new();
    for sub in ["new", "cur"] {
        let Ok(rd) = fs::read_dir(dir.join(sub)) else {
            continue;
        };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            // the target may have been renamed since the search tool made
            // the link (read, flagged): follow it by its unique name
            let real = match fs::canonicalize(e.path()) {
                Ok(r) => r,
                Err(_) => match fs::read_link(e.path()).ok().and_then(|t| relocate(&t)) {
                    Some(r) => r,
                    None => continue,
                },
            };
            out.push(real);
        }
    }
    summaries_of(out)
}

/// Summaries of message files wherever they are, newest first.
pub fn summaries_of(paths: Vec<PathBuf>) -> Vec<Summary> {
    let mut seen = std::collections::HashSet::new();
    let paths: Vec<PathBuf> = paths
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect();
    let read = rical_index::on_every_core(&paths, |real| {
        let in_new = real
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "new");
        let folder = real
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .map(|n| {
                n.to_string_lossy()
                    .trim_start_matches('.')
                    .replace('.', "/")
            })
            .unwrap_or_default();
        summarize(real, &folder, in_new)
    });
    // two names can still be one message once read
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<Summary> = read
        .into_iter()
        .flatten()
        .filter(|s| seen.insert(s.path.clone()))
        .collect();
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.path.cmp(&b.path)));
    out
}

/// Make `dir` a Maildir of symlinks to `paths`, as a search command that
/// only lists files leaves for us to do. What was there is cleared.
pub fn write_links(dir: &Path, paths: &[PathBuf]) -> Result<()> {
    for sub in ["new", "cur"] {
        let d = dir.join(sub);
        if d.is_dir() {
            for e in fs::read_dir(&d)?.flatten() {
                let _ = fs::remove_file(e.path());
            }
        }
        fs::create_dir_all(&d)?;
    }
    fs::create_dir_all(dir.join("tmp"))?;
    for (i, p) in paths.iter().enumerate() {
        let name = p
            .file_name()
            .map_or(i.to_string(), |n| n.to_string_lossy().into_owned());
        let sub = if p
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|n| n == "new")
        {
            "new"
        } else {
            "cur"
        };
        let link = dir.join(sub).join(format!("{i}-{name}"));
        #[cfg(unix)]
        std::os::unix::fs::symlink(p, &link).with_context(|| format!("{}", link.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod link_tests {
    use super::*;

    #[test]
    fn results_are_the_real_messages() {
        let d = tempfile::tempdir().unwrap();
        let a = deliver(
            &d.path().join("Sent"),
            b"From: a@x\nSubject: one\nDate: Thu, 5 Nov 2026 14:00:00 +0000\n\nx\n",
            Flags {
                seen: true,
                ..Flags::default()
            },
            false,
        )
        .unwrap();
        let b = deliver(
            &d.path().join("work/Projects"),
            b"From: b@x\nSubject: two\nDate: Fri, 6 Nov 2026 14:00:00 +0000\n\nx\n",
            Flags::default(),
            true,
        )
        .unwrap();
        let links = d.path().join("results");
        write_links(&links, &[a.clone(), b.clone()]).unwrap();
        let found = list_links(&links);
        let by: Vec<(&str, &str, bool)> = found
            .iter()
            .map(|s| (s.subject.as_str(), s.folder.as_str(), s.is_new))
            .collect();
        assert_eq!(by, [("two", "Projects", true), ("one", "Sent", false)]);
        // acting on a result acts on the message, not the link
        assert_eq!(found[1].path, fs::canonicalize(&a).unwrap());
        // a message renamed after the search (read) is still found
        let read = set_flags(
            &b,
            Flags {
                seen: true,
                ..Flags::default()
            },
        )
        .unwrap();
        let after = list_links(&links);
        assert_eq!(after.len(), 2);
        assert!(
            after
                .iter()
                .any(|s| s.path == fs::canonicalize(&read).unwrap())
        );
        let b = read;
        // one that has gone is not listed
        fs::remove_file(&a).unwrap();
        assert_eq!(list_links(&links).len(), 1);
        // searching again clears the old results
        write_links(&links, &[b]).unwrap();
        assert_eq!(
            fs::read_dir(links.join("new")).unwrap().count()
                + fs::read_dir(links.join("cur")).unwrap().count(),
            1
        );
    }
}

/// Where a message is now if its name has changed (flags rename files, and
/// so does opening one from `new/`), found by the unique part of its name.
pub fn relocate(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_owned());
    }
    let folder = path.parent()?.parent()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let unique = split_name(&name).0.to_owned();
    for sub in ["cur", "new"] {
        for e in fs::read_dir(folder.join(sub)).ok()?.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if split_name(&n).0 == unique {
                return Some(e.path());
            }
        }
    }
    None
}

/// The start of what a message says, on one line: its text, or its HTML
/// read the way the reader reads it (hidden "preheader" text left out).
fn preview_of(root: &mail_parser::MessageRef) -> String {
    let text = root.text_body().next().and_then(|p| {
        if p.is_content_type("text", "html") {
            p.text().map(|h| crate::html::convert(&h).text)
        } else {
            p.text().map(|t| t.into_owned())
        }
    });
    let flat = text
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim_start().starts_with('>'))
        .collect::<Vec<_>>()
        .join(" ");
    let words: Vec<&str> = flat.split_whitespace().collect();
    let mut out = String::new();
    for w in words {
        if out.len() + w.len() > 200 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    out
}

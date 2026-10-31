//! A SQLite index of a folder of files: a few things about each (when an
//! event can show, a contact's name, a message's headers), so that the apps
//! can show and search a collection without reading all of it, and read
//! only the files on screen.
//!
//! The files stay the source of truth. Each file's size and modification
//! time are kept with what was taken from it, and [`Index::sync`] takes it
//! again from the files that changed since. The index is only ever a copy:
//! one from another version of the app is emptied and filled again, and
//! where there is nowhere to keep one it lives in memory.
//!
//! The apps share an index (the calendar, the mail client and `rical
//! remind` all read the same calendars): writes are transactions, made
//! so that two at once come to the same thing.

use std::{
    collections::HashMap,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
    time::Duration,
};

pub use rusqlite;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

/// What is taken from each file of a kind, and the tables it goes in.
pub trait Schema: Sync {
    /// What is taken from one file.
    type Row: Send;
    /// Changed whenever [`Schema::TABLES`] or what [`Schema::extract`]
    /// gives changes: an index of another version is filled again.
    const VERSION: i64;
    /// `CREATE` statements for the kind's tables, each with a `file`
    /// column for the id in `files` (its own `id` for an R*Tree).
    const TABLES: &'static str;
    /// Statements forgetting what was taken from a file, `?1` its id.
    const FORGET: &'static [&'static str];
    /// Read `path` and take what is kept of it. An error is warned about
    /// once: the file is noted and not read again until it changes.
    fn extract(&self, path: &Path) -> Result<Self::Row, String>;
    /// Keep what was taken from file `file`.
    fn store(&self, tx: &Transaction, file: i64, row: &Self::Row) -> rusqlite::Result<()>;
}

/// What tells that a file has changed: its size and modification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub secs: i64,
    pub nanos: u32,
}

impl Stamp {
    pub fn of(path: &Path) -> Option<Stamp> {
        let m = std::fs::metadata(path).ok()?;
        let t = m
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some(Stamp {
            size: m.len(),
            secs: t.as_secs() as i64,
            nanos: t.subsec_nanos(),
        })
    }
}

/// What a [`Index::sync`] found.
#[derive(Debug, Default, Clone)]
pub struct Changes {
    /// Files read again (new or changed).
    pub read: Vec<PathBuf>,
    /// Files that are gone.
    pub gone: Vec<PathBuf>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.gone.is_empty()
    }
}

/// An open index.
pub struct Index {
    conn: Mutex<Connection>,
    /// Where it is kept; `None` in memory.
    pub file: Option<PathBuf>,
}

impl std::fmt::Debug for Index {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Index").field("file", &self.file).finish()
    }
}

const FILES: &str = "CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY,
    path BLOB NOT NULL UNIQUE,
    size INTEGER NOT NULL,
    secs INTEGER NOT NULL,
    nanos INTEGER NOT NULL,
    ok INTEGER NOT NULL
);";

impl Index {
    /// The index kept in `file` (made if need be), or one in memory if
    /// `file` is `None` or cannot be opened. `version` and `tables` are
    /// as in [`Schema`].
    pub fn open(file: Option<&Path>, version: i64, tables: &str) -> Index {
        if let Some(path) = file {
            match open_file(path, version, tables) {
                Ok(conn) => {
                    return Index {
                        conn: Mutex::new(conn),
                        file: Some(path.to_owned()),
                    };
                }
                Err(e) => {
                    log::warn!("could not open the index {path:?}, keeping one in memory: {e:#}")
                }
            }
        }
        Index {
            conn: Mutex::new(open_memory(version, tables).expect("an index in memory")),
            file: None,
        }
    }

    /// [`Index::open`] for a [`Schema`].
    pub fn for_schema<S: Schema>(file: Option<&Path>) -> Index {
        Self::open(file, S::VERSION, S::TABLES)
    }

    /// The connection, to read with (or write with, for a kind that keeps
    /// its own tables up to date).
    pub fn conn(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Bring the index up to date with `paths`, the files there are now:
    /// files new or changed since are read again (on every core), and any
    /// it has that are not among `paths` are forgotten.
    pub fn sync<S: Schema>(&self, schema: &S, paths: &[PathBuf]) -> anyhow::Result<Changes> {
        let stamps = on_every_core(paths, |p| Stamp::of(p));
        let known = self.known()?;
        let mut on_disk = std::collections::HashSet::with_capacity(paths.len());
        let mut changed = Vec::new();
        for (path, stamp) in paths.iter().zip(&stamps) {
            let Some(stamp) = stamp else { continue };
            on_disk.insert(path.as_os_str());
            if known.get(path.as_os_str()).is_none_or(|(_, s)| s != stamp) {
                changed.push((path.clone(), *stamp));
            }
        }
        let gone: Vec<PathBuf> = known
            .keys()
            .filter(|p| !on_disk.contains(p.as_os_str()))
            .map(PathBuf::from)
            .collect();
        self.write(schema, changed, gone)
    }

    /// The part of [`Index::sync`] that can be done quickly, to show it
    /// first: files among `paths` it has not got (new ones), files it has
    /// that are not among them (gone), and of `first` (those on screen,
    /// say), those that changed. Only `first` and new files are looked
    /// at, not every file; [`Index::sync`] does the rest after.
    pub fn sync_first<S: Schema>(
        &self,
        schema: &S,
        paths: &[PathBuf],
        first: &[PathBuf],
    ) -> anyhow::Result<Changes> {
        let known = self.known()?;
        let on_disk: std::collections::HashSet<&std::ffi::OsStr> =
            paths.iter().map(|p| p.as_os_str()).collect();
        let new: Vec<&PathBuf> = paths
            .iter()
            .filter(|p| !known.contains_key(p.as_os_str()))
            .collect();
        let shown: Vec<&PathBuf> = first
            .iter()
            .filter(|p| on_disk.contains(p.as_os_str()) && known.contains_key(p.as_os_str()))
            .collect();
        let look: Vec<&PathBuf> = new.into_iter().chain(shown).collect();
        let stamps = on_every_core(&look, |p| Stamp::of(p));
        let changed = look
            .into_iter()
            .zip(stamps)
            .filter_map(|(p, s)| {
                let s = s?;
                known
                    .get(p.as_os_str())
                    .is_none_or(|(_, k)| *k != s)
                    .then(|| (p.clone(), s))
            })
            .collect();
        let gone = known
            .keys()
            .filter(|p| !on_disk.contains(p.as_os_str()))
            .map(PathBuf::from)
            .collect();
        self.write(schema, changed, gone)
    }

    /// Read `paths` again (or forget those that are gone), leaving the
    /// rest: after a change to a few files.
    pub fn sync_files<S: Schema>(&self, schema: &S, paths: &[PathBuf]) -> anyhow::Result<Changes> {
        let mut changed = Vec::new();
        let mut gone = Vec::new();
        for p in paths {
            match Stamp::of(p) {
                Some(s) if p.is_file() => changed.push((p.clone(), s)),
                _ => gone.push(p.clone()),
            }
        }
        self.write(schema, changed, gone)
    }

    /// Every file it has, with its id and stamp.
    fn known(&self) -> anyhow::Result<HashMap<std::ffi::OsString, (i64, Stamp)>> {
        let conn = self.conn();
        let mut st = conn.prepare_cached("SELECT id, path, size, secs, nanos FROM files")?;
        let rows = st.query_map([], |r| {
            Ok((
                std::ffi::OsString::from_vec(r.get::<_, Vec<u8>>(1)?),
                (
                    r.get::<_, i64>(0)?,
                    Stamp {
                        size: r.get::<_, i64>(2)? as u64,
                        secs: r.get(3)?,
                        nanos: r.get::<_, i64>(4)? as u32,
                    },
                ),
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    fn write<S: Schema>(
        &self,
        schema: &S,
        changed: Vec<(PathBuf, Stamp)>,
        gone: Vec<PathBuf>,
    ) -> anyhow::Result<Changes> {
        if changed.is_empty() && gone.is_empty() {
            return Ok(Changes::default());
        }
        // read away from the connection, which others may want meanwhile
        let started = std::time::Instant::now();
        let rows = on_every_core(&changed, |(p, _)| schema.extract(p));
        let read = started.elapsed();
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // only what it had is gone
        let mut forgotten = Vec::new();
        {
            let mut find = tx.prepare_cached("SELECT id FROM files WHERE path = ?1")?;
            let mut delete = tx.prepare_cached("DELETE FROM files WHERE id = ?1")?;
            // a file it has not got yet is added, with nothing to forget
            let mut insert = tx.prepare_cached(
                "INSERT INTO files (path, size, secs, nanos, ok) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (path) DO NOTHING",
            )?;
            let mut update = tx.prepare_cached(
                "UPDATE files SET size = ?2, secs = ?3, nanos = ?4, ok = ?5 WHERE path = ?1
                 RETURNING id",
            )?;
            let mut forgets = S::FORGET
                .iter()
                .map(|sql| tx.prepare_cached(sql))
                .collect::<Result<Vec<_>, _>>()?;
            let mut forget = |id: i64| -> rusqlite::Result<()> {
                for f in &mut forgets {
                    f.execute([id])?;
                }
                Ok(())
            };
            for p in &gone {
                let id: Option<i64> = find
                    .query_row([p.as_os_str().as_bytes()], |r| r.get(0))
                    .optional()?;
                if let Some(id) = id {
                    forget(id)?;
                    delete.execute([id])?;
                    forgotten.push(p.clone());
                }
            }
            for ((path, stamp), row) in changed.iter().zip(&rows) {
                if let Err(e) = row {
                    log::warn!("skipping {path:?}: {e}");
                }
                let values = params![
                    path.as_os_str().as_bytes(),
                    stamp.size as i64,
                    stamp.secs,
                    stamp.nanos as i64,
                    row.is_ok()
                ];
                let id = if insert.execute(values)? == 1 {
                    tx.last_insert_rowid()
                } else {
                    // had (or another wrote it meanwhile): what was
                    // taken from it before goes
                    let id: i64 = update.query_row(values, |r| r.get(0))?;
                    forget(id)?;
                    id
                };
                if let Ok(row) = row {
                    schema.store(&tx, id, row)?;
                }
            }
        }
        tx.commit()?;
        log::debug!(
            "indexed {} files ({} gone): read in {read:?}, written in {:?}",
            changed.len(),
            forgotten.len(),
            started.elapsed() - read
        );
        Ok(Changes {
            read: changed.into_iter().map(|(p, _)| p).collect(),
            gone: forgotten,
        })
    }

    /// Whether it has no files at all (never filled, or another version's).
    pub fn is_empty(&self) -> bool {
        self.conn()
            .query_row("SELECT NOT EXISTS (SELECT 1 FROM files)", [], |r| r.get(0))
            .unwrap_or(true)
    }

    /// The id of a file it has.
    pub fn id_of(&self, path: &Path) -> Option<i64> {
        self.conn()
            .prepare_cached("SELECT id FROM files WHERE path = ?1")
            .and_then(|mut s| {
                s.query_row([path.as_os_str().as_bytes()], |r| r.get(0))
                    .optional()
            })
            .ok()
            .flatten()
    }

    /// The stamp a file had when it was read for the index, if it was.
    pub fn stamp_of(&self, path: &Path) -> Option<Stamp> {
        self.conn()
            .prepare_cached("SELECT size, secs, nanos FROM files WHERE path = ?1")
            .and_then(|mut s| {
                s.query_row([path.as_os_str().as_bytes()], |r| {
                    Ok(Stamp {
                        size: r.get::<_, i64>(0)? as u64,
                        secs: r.get(1)?,
                        nanos: r.get::<_, i64>(2)? as u32,
                    })
                })
                .optional()
            })
            .ok()
            .flatten()
    }
}

/// A path kept in the index, back as a path.
pub fn path(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

/// A path as the index keeps it.
pub fn bytes(path: &Path) -> &[u8] {
    path.as_os_str().as_bytes()
}

fn open_file(path: &Path, version: i64, tables: &str) -> anyhow::Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(Duration::from_secs(30))?;
    // a crash loses at most the last change, which is read again
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    // room for the pages a first fill of a big folder touches (32 MB)
    conn.pragma_update(None, "cache_size", -32768)?;
    prepare(conn, version, tables)
}

fn open_memory(version: i64, tables: &str) -> anyhow::Result<Connection> {
    prepare(Connection::open_in_memory()?, version, tables)
}

/// Make the tables, emptying an index of another version first.
fn prepare(mut conn: Connection, version: i64, tables: &str) -> anyhow::Result<Connection> {
    let have: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if have != version {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // another may have done it meanwhile
        let have: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if have != version {
            // virtual tables first, which take their own tables with them
            let names: Vec<String> = tx
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'table'
                     AND name NOT LIKE 'sqlite_%'
                     ORDER BY sql NOT LIKE 'CREATE VIRTUAL%'",
                )?
                .query_map([], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            for name in &names {
                tx.execute_batch(&format!("DROP TABLE IF EXISTS \"{name}\";"))?;
            }
            tx.execute_batch(FILES)?;
            tx.execute_batch(tables)?;
            tx.pragma_update(None, "user_version", version)?;
        }
        tx.commit()?;
    }
    Ok(conn)
}

/// Where the index of `kind` for the files under `root` is kept: in
/// `$XDG_CACHE_HOME/rical/` (`~/.cache/rical/`), named by `kind` and the
/// root, so that two roots do not share one.
pub fn cache_file(kind: &str, root: &Path) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
    let file = base.join("rical").join(format!(
        "{kind}-{:016x}.sqlite",
        fnv(root.as_os_str().as_bytes())
    ));
    // the cache of earlier versions, which this replaces
    let _ = std::fs::remove_file(file.with_extension("cache"));
    Some(file)
}

/// A stable hash (unlike std's), so a root keeps its index.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    })
}

/// `f` over `items` on every core (up to 16), keeping their order.
pub fn on_every_core<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(16);
    let chunk = items.len().div_ceil(threads).max(64);
    if items.len() <= chunk {
        return items.iter().map(f).collect();
    }
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|slice| s.spawn(move || slice.iter().map(f).collect::<Vec<_>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a reading thread panicked"))
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Files of numbers, the index their lines' lengths.
    struct Lines;

    impl Schema for Lines {
        type Row = Vec<i64>;
        const VERSION: i64 = 1;
        const TABLES: &'static str =
            "CREATE TABLE lines (file INTEGER NOT NULL, len INTEGER NOT NULL);
            CREATE INDEX lines_file ON lines (file);";
        const FORGET: &'static [&'static str] = &["DELETE FROM lines WHERE file = ?1"];

        fn extract(&self, path: &Path) -> Result<Vec<i64>, String> {
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            if text.starts_with('!') {
                return Err("not lines".into());
            }
            Ok(text.lines().map(|l| l.len() as i64).collect())
        }

        fn store(&self, tx: &Transaction, file: i64, row: &Vec<i64>) -> rusqlite::Result<()> {
            let mut st = tx.prepare_cached("INSERT INTO lines (file, len) VALUES (?1, ?2)")?;
            for len in row {
                st.execute([file, *len])?;
            }
            Ok(())
        }
    }

    fn total(index: &Index) -> i64 {
        index
            .conn()
            .query_row("SELECT coalesce(sum(len), 0) FROM lines", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn reads_only_what_changed_and_forgets_what_went() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (d.path().join("a"), d.path().join("b"), d.path().join("c"));
        std::fs::write(&a, "one\ntwo\n").unwrap();
        std::fs::write(&b, "three\n").unwrap();
        std::fs::write(&c, "!broken").unwrap();
        let file = d.path().join("cache/index.sqlite");
        let index = Index::for_schema::<Lines>(Some(&file));
        assert_eq!(index.file.as_deref(), Some(file.as_path()));
        let all = [a.clone(), b.clone(), c.clone()];
        let first = index.sync(&Lines, &all).unwrap();
        assert_eq!(first.read.len(), 3);
        assert_eq!(total(&index), 11);
        // nothing changed: nothing read, the broken one included
        assert!(index.sync(&Lines, &all).unwrap().is_empty());

        // kept between runs
        drop(index);
        let index = Index::for_schema::<Lines>(Some(&file));
        assert!(index.sync(&Lines, &all).unwrap().is_empty());
        assert_eq!(total(&index), 11);

        std::fs::write(&a, "four!\n").unwrap();
        std::fs::remove_file(&b).unwrap();
        let next = index.sync(&Lines, &[a.clone(), c.clone()]).unwrap();
        assert_eq!(next.read, std::slice::from_ref(&a));
        assert_eq!(next.gone, std::slice::from_ref(&b));
        assert_eq!(total(&index), 5);

        // one file at a time
        std::fs::write(&a, "x\n").unwrap();
        index.sync_files(&Lines, std::slice::from_ref(&a)).unwrap();
        assert_eq!(total(&index), 1);
        assert!(index.stamp_of(&a).is_some() && index.id_of(&b).is_none());

        // another version starts again
        drop(index);
        let index = Index::open(Some(&file), 2, Lines::TABLES);
        assert_eq!(total(&index), 0);
        assert_eq!(index.sync(&Lines, &[a.clone(), c]).unwrap().read.len(), 2);
    }

    /// The quick part finds new and gone files, and changes to those it
    /// is told to look at first; the rest wait for a full sync.
    #[test]
    fn new_gone_and_shown_files_first() {
        let d = tempfile::tempdir().unwrap();
        let [a, b, c, n] = ["a", "b", "c", "n"].map(|f| d.path().join(f));
        for f in [&a, &b, &c] {
            std::fs::write(f, "one\n").unwrap();
        }
        let index = Index::for_schema::<Lines>(Some(&d.path().join("index.sqlite")));
        index
            .sync(&Lines, &[a.clone(), b.clone(), c.clone()])
            .unwrap();

        // later: a and b change, c goes, n is new; a is on screen
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        for f in [&a, &b] {
            std::fs::write(f, "changed\n").unwrap();
            std::fs::File::options()
                .write(true)
                .open(f)
                .unwrap()
                .set_modified(later)
                .unwrap();
        }
        std::fs::remove_file(&c).unwrap();
        std::fs::write(&n, "new\n").unwrap();
        let now = [a.clone(), b.clone(), n.clone()];
        let first = index
            .sync_first(&Lines, &now, std::slice::from_ref(&a))
            .unwrap();
        let mut read = first.read.clone();
        read.sort();
        assert_eq!(read, [a.clone(), n.clone()]);
        assert_eq!(first.gone, [c]);
        // b, which no one was looking at, is found by the full sync
        assert_eq!(index.sync(&Lines, &now).unwrap().read, [b]);
    }

    #[test]
    fn in_memory_when_there_is_nowhere_to_keep_it() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("not-a-dir"), "").unwrap();
        let index = Index::for_schema::<Lines>(Some(&d.path().join("not-a-dir/index")));
        assert!(index.file.is_none());
        let a = d.path().join("a");
        std::fs::write(&a, "abc\n").unwrap();
        index.sync(&Lines, &[a]).unwrap();
        assert_eq!(total(&index), 3);
    }
}

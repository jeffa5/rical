//! Address books: a folder of folders of `.vcf` files (as vdirsyncer
//! keeps a CardDAV account), with an index (see [`rical_index`]) of what
//! lists, searches and finding duplicates need of each card: its name,
//! the line under it, its words, numbers, addresses and UID. Cards
//! themselves are read from their files when wanted, and the last few
//! thousand read are kept. [`Books::refresh`] catches up with the files.

use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::Context as _;
use compact_ics::Component;
pub use rical_index::Changes;
use rical_index::rusqlite::{self, Transaction, params};

use crate::{
    model::Card,
    search::{self, Field},
    text::{email_key, name_key, phone_tail},
};

/// One address book: a folder of `.vcf` files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressBook {
    /// Its path under the root ("nextcloud/contacts"), unique.
    pub id: String,
    /// Its `displayname`, else the folder's name.
    pub name: String,
    pub path: PathBuf,
}

/// A card: the file it is in and which card in it (nearly always the
/// only one).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CardId {
    pub path: PathBuf,
    pub index: usize,
}

/// Bringing the index of address books up to date on another thread,
/// with its own connection to the index: see [`Books::check`].
#[derive(Debug, Clone)]
pub struct Check {
    index: PathBuf,
    /// The books' folders.
    books: Vec<PathBuf>,
    /// Only these files; else every card in `books`.
    files: Option<Vec<PathBuf>>,
    /// The files read lately (the people shown among them), to look at
    /// first.
    shown: Vec<PathBuf>,
}

impl Check {
    /// Whether it looks at every file (not only some the watcher saw), so
    /// that [`Check::run_first`] leaves some for [`Check::run`].
    pub fn is_full(&self) -> bool {
        self.files.is_none()
    }

    /// What can be found quickly, to show before the rest: new cards and
    /// gone ones (from the folders' listings alone), and changes to those
    /// read lately, which are those shown. [`Check::run`] after finds the
    /// rest.
    pub fn run_first(&self) -> anyhow::Result<rical_index::Changes> {
        let index = rical_index::Index::for_schema::<Cards>(Some(&self.index));
        match &self.files {
            Some(files) => index.sync_files(&Cards, files),
            None => {
                let mut paths = Vec::new();
                for dir in &self.books {
                    paths.extend(card_paths(dir)?);
                }
                index.sync_first(&Cards, &paths, &self.shown)
            }
        }
    }

    /// Look at the files and index what changed. The index is shared, so
    /// the books see the changes at once; [`Books::take_changes`] tells
    /// them which cards to read again.
    pub fn run(self) -> anyhow::Result<rical_index::Changes> {
        let index = rical_index::Index::for_schema::<Cards>(Some(&self.index));
        match self.files {
            Some(files) => index.sync_files(&Cards, &files),
            None => {
                let mut paths = Vec::new();
                for dir in &self.books {
                    paths.extend(card_paths(dir)?);
                }
                index.sync(&Cards, &paths)
            }
        }
    }
}

/// A card read from its file.
#[derive(Clone)]
pub struct CardRef {
    file: Arc<[Component]>,
    index: usize,
}

impl CardRef {
    pub fn card(&self) -> Card<'_> {
        Card(&self.file[self.index])
    }
}

/// Every address book under a root, and an index of their cards.
pub struct Books {
    pub root: PathBuf,
    pub books: Vec<AddressBook>,
    /// Each book's folder, to its place in `books`.
    by_dir: HashMap<PathBuf, usize>,
    index: Arc<rical_index::Index>,
    read: Mutex<Read>,
    /// Counts changes, so views know when to rebuild.
    generation: u64,
}

/// Files read lately, by path, with when each was last wanted.
#[derive(Default)]
struct Read {
    files: HashMap<OsString, (Arc<[Component]>, u64)>,
    tick: u64,
}

/// How many read files are kept.
const KEEP: usize = 4096;

fn read_cards(path: &Path) -> Result<Vec<Component>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let (cards, _) = compact_ics::parse(&text).map_err(|e| e.to_string())?;
    Ok(cards
        .into_iter()
        .filter(|c| c.kind.eq_ignore_ascii_case("VCARD"))
        .collect())
}

fn is_card_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("vcf"))
}

/// Folders under `root` holding cards or address book metadata, at any
/// depth, skipping hidden ones.
fn find_books(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut is_book = false;
        for e in entries.flatten() {
            let p = e.path();
            let hidden = e.file_name().to_string_lossy().starts_with('.');
            // the entry's type, without a look at the file unless a link
            let dir = e
                .file_type()
                .is_ok_and(|t| t.is_dir() || (t.is_symlink() && p.is_dir()));
            if dir && !hidden {
                stack.push(p);
            } else if is_card_file(&p)
                || matches!(e.file_name().to_str(), Some("displayname" | "color"))
            {
                is_book = true;
            }
        }
        if is_book {
            out.push(dir);
        }
    }
    out.sort();
    out
}

/// The `.vcf` files in folder `dir`, sorted.
fn card_paths(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {dir:?}"))?
        .flatten()
        .filter(|e| is_card_file(Path::new(&e.file_name())))
        // the entry's type, without a look at the file unless a link
        .filter(|e| {
            e.file_type()
                .is_ok_and(|t| t.is_file() || (t.is_symlink() && e.path().is_file()))
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    Ok(out)
}

/// What the index keeps of one card.
pub struct CardRow {
    uid: Option<String>,
    kind: String,
    name: String,
    sort: String,
    detail: String,
    name_key: String,
    /// Its words, each with the best field it is in.
    words: Vec<(String, Field)>,
    /// Its numbers' digits (without a leading `+`), for search.
    phones: Vec<String>,
    /// Its addresses as written, for completing addresses elsewhere.
    emails: Vec<String>,
    /// What tells the same person: addresses as compared, and the last
    /// digits of numbers.
    email_keys: Vec<String>,
    phone_tails: Vec<String>,
}

impl CardRow {
    fn of(card: Card) -> CardRow {
        let (words, phones, detail) = search::words_of(card);
        let mut best: HashMap<String, Field> = HashMap::new();
        for (w, f) in words {
            best.entry(w).and_modify(|b| *b = (*b).min(f)).or_insert(f);
        }
        let emails = card.emails();
        let mut email_keys: Vec<String> = emails.iter().map(|e| email_key(&e.value)).collect();
        email_keys.sort();
        email_keys.dedup();
        let mut phone_tails: Vec<String> = card
            .phones()
            .iter()
            .filter_map(|p| phone_tail(&p.value))
            .collect();
        phone_tails.sort();
        phone_tails.dedup();
        let name = card.display_name();
        CardRow {
            uid: card.uid().map(str::to_owned),
            kind: card.kind(),
            sort: card.sort_key(),
            detail,
            name_key: name_key(&name),
            name,
            words: best.into_iter().collect(),
            phones,
            emails: emails
                .into_iter()
                .map(|e| e.value.trim().to_owned())
                .collect(),
            email_keys,
            phone_tails,
        }
    }
}

/// The schema of an address book index.
pub struct Cards;

impl rical_index::Schema for Cards {
    type Row = Vec<CardRow>;
    const VERSION: i64 = 3;
    const TABLES: &'static str = "
        CREATE TABLE cards (
            id INTEGER PRIMARY KEY,
            file INTEGER NOT NULL,
            idx INTEGER NOT NULL,
            uid TEXT,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            sort TEXT NOT NULL,
            detail TEXT NOT NULL,
            name_key TEXT NOT NULL
        );
        CREATE INDEX cards_file ON cards (file);
        CREATE INDEX cards_uid ON cards (uid);
        -- each card's words, folded, and the best field each is in; by file
        -- and place in it, which stay as they are when a file is read again
        CREATE TABLE words (
            word TEXT NOT NULL,
            file INTEGER NOT NULL,
            idx INTEGER NOT NULL,
            field INTEGER NOT NULL,
            PRIMARY KEY (word, file, idx)
        ) WITHOUT ROWID;
        CREATE INDEX words_file ON words (file);
        CREATE TABLE phones (file INTEGER NOT NULL, idx INTEGER NOT NULL, digits TEXT NOT NULL);
        CREATE INDEX phones_file ON phones (file);
        CREATE TABLE emails (card INTEGER NOT NULL, file INTEGER NOT NULL, address TEXT NOT NULL);
        CREATE INDEX emails_file ON emails (file);
        -- what tells the same person: 0 an address, 1 a number's last digits
        CREATE TABLE keys (card INTEGER NOT NULL, file INTEGER NOT NULL, kind INTEGER NOT NULL, key TEXT NOT NULL);
        CREATE INDEX keys_key ON keys (kind, key);
        CREATE INDEX keys_file ON keys (file);
    ";
    const FORGET: &'static [&'static str] = &[
        "DELETE FROM cards WHERE file = ?1",
        "DELETE FROM words WHERE file = ?1",
        "DELETE FROM phones WHERE file = ?1",
        "DELETE FROM emails WHERE file = ?1",
        "DELETE FROM keys WHERE file = ?1",
    ];

    fn extract(&self, path: &Path) -> Result<Vec<CardRow>, String> {
        Ok(read_cards(path)?
            .iter()
            .map(|c| CardRow::of(Card(c)))
            .collect())
    }

    fn store(&self, tx: &Transaction, file: i64, rows: &Vec<CardRow>) -> rusqlite::Result<()> {
        let mut card = tx.prepare_cached(
            "INSERT INTO cards (file, idx, uid, kind, name, sort, detail, name_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )?;
        let mut word = tx
            .prepare_cached("INSERT INTO words (word, file, idx, field) VALUES (?1, ?2, ?3, ?4)")?;
        let mut phone =
            tx.prepare_cached("INSERT INTO phones (file, idx, digits) VALUES (?1, ?2, ?3)")?;
        let mut email =
            tx.prepare_cached("INSERT INTO emails (card, file, address) VALUES (?1, ?2, ?3)")?;
        let mut key =
            tx.prepare_cached("INSERT INTO keys (card, file, kind, key) VALUES (?1, ?2, ?3, ?4)")?;
        for (idx, r) in rows.iter().enumerate() {
            card.execute(params![
                file, idx as i64, r.uid, r.kind, r.name, r.sort, r.detail, r.name_key
            ])?;
            let id = tx.last_insert_rowid();
            for (w, f) in &r.words {
                word.execute(params![w, file, idx as i64, *f as i64])?;
            }
            for p in &r.phones {
                phone.execute(params![file, idx as i64, p])?;
            }
            for e in &r.emails {
                email.execute(params![id, file, e])?;
            }
            for k in &r.email_keys {
                key.execute(params![id, file, 0, k])?;
            }
            for k in &r.phone_tails {
                key.execute(params![id, file, 1, k])?;
            }
        }
        Ok(())
    }
}

/// One card as the index has it, for lists.
#[derive(Debug, Clone)]
pub struct Listed {
    /// Its file's id in the index (which stays while the file does).
    pub file: i64,
    pub id: CardId,
    pub uid: Option<String>,
    pub name: String,
    pub sort: String,
    pub detail: String,
}

/// One card as finding duplicates needs it.
#[derive(Debug, Clone)]
pub struct Keyed {
    pub id: CardId,
    pub uid: Option<String>,
    pub kind: String,
    pub name: String,
    pub email_keys: Vec<String>,
    pub phone_tails: Vec<String>,
}

impl Books {
    /// Every address book under `root` (or `root` itself, if it holds
    /// cards directly), indexed in memory, reading every card.
    pub fn load(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::load_cached(root, None)
    }

    /// [`Books::load`], with the index kept where the apps keep it (see
    /// [`Books::default_cache`]): only cards changed since it was last
    /// brought up to date are read.
    pub fn open(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let cache = Self::default_cache(root.as_ref());
        Self::load_cached(root, cache.as_deref())
    }

    /// [`Books::load`], with the index kept in `cache` (in memory if
    /// `None`), brought up to date.
    pub fn load_cached(root: impl AsRef<Path>, cache: Option<&Path>) -> anyhow::Result<Self> {
        let mut out = Self::scan(root)?;
        out.index = Arc::new(rical_index::Index::for_schema::<Cards>(cache));
        out.refresh()?;
        Ok(out)
    }

    /// [`Books::open`] without looking at the cards first: what the index
    /// had when last brought up to date, at once. [`Books::check`] brings
    /// it up to date, away from the window. An index never filled is
    /// filled here, as there is nothing to show from it.
    pub fn open_unchecked(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut out = Self::scan(root.as_ref())?;
        let cache = Self::default_cache(root.as_ref());
        out.index = Arc::new(rical_index::Index::for_schema::<Cards>(cache.as_deref()));
        if out.index.file.is_none() || out.index.is_empty() {
            out.refresh()?;
        }
        Ok(out)
    }

    /// The books under `root` with their index as it is, read by no one
    /// here and filled by no one: for another look at books open
    /// elsewhere (finding duplicates on another thread, say), which never
    /// races that one to fill it.
    pub fn open_index(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut out = Self::scan(root.as_ref())?;
        let cache = Self::default_cache(root.as_ref());
        out.index = Arc::new(rical_index::Index::for_schema::<Cards>(cache.as_deref()));
        // nowhere to keep one: this one is all there is
        if out.index.file.is_none() {
            out.refresh()?;
        }
        Ok(out)
    }

    /// A check of every card against the index, to run on another thread
    /// (see [`Check::run`]) and give back to [`Books::take_changes`].
    /// `None` for an index kept in memory, which only these books can
    /// bring up to date ([`Books::refresh`]).
    pub fn check(&self) -> Option<Check> {
        let shown = self
            .read
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .files
            .keys()
            .map(PathBuf::from)
            .collect();
        Some(Check {
            index: self.index.file.clone()?,
            books: self.books.iter().map(|b| b.path.clone()).collect(),
            files: None,
            shown,
        })
    }

    /// [`Books::check`] of only `paths` (after the watcher saw them change).
    pub fn check_files(&self, paths: &[PathBuf]) -> Option<Check> {
        let files: Vec<PathBuf> = paths
            .iter()
            .filter(|p| self.book_of(p).is_some() && is_card_file(p))
            .cloned()
            .collect();
        (!files.is_empty()).then_some(())?;
        Some(Check {
            index: self.index.file.clone()?,
            books: Vec::new(),
            files: Some(files),
            shown: Vec::new(),
        })
    }

    /// Take what a [`Check`] found: cards it read again are read again
    /// here when next wanted. Returns whether anything changed.
    pub fn take_changes(&mut self, changes: &rical_index::Changes) -> bool {
        self.changed(changes)
    }

    /// Where [`Books::open`] keeps the index for `root`.
    pub fn default_cache(root: &Path) -> Option<PathBuf> {
        rical_index::cache_file("contacts", root)
    }

    /// The address books under `root`, with an empty index: quick, to
    /// show something while [`Books::open`] brings the index up to date.
    pub fn scan(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        anyhow::ensure!(root.is_dir(), "not a folder: {root:?}");
        let mut dirs = find_books(root);
        if dirs.is_empty() {
            dirs.push(root.to_owned());
        }
        let books: Vec<AddressBook> = dirs
            .into_iter()
            .map(|path| {
                let id = path
                    .strip_prefix(root)
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| ".".to_owned());
                let folder = path
                    .file_name()
                    .map_or_else(|| id.clone(), |n| n.to_string_lossy().into_owned());
                let name = std::fs::read_to_string(path.join("displayname"))
                    .ok()
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .unwrap_or(folder);
                AddressBook { id, name, path }
            })
            .collect();
        let by_dir = books
            .iter()
            .enumerate()
            .map(|(i, b)| (b.path.clone(), i))
            .collect();
        Ok(Self {
            root: root.to_owned(),
            books,
            by_dir,
            index: Arc::new(rical_index::Index::for_schema::<Cards>(None)),
            read: Mutex::default(),
            generation: 0,
        })
    }

    fn card_paths(&self, book: usize) -> anyhow::Result<Vec<PathBuf>> {
        card_paths(&self.books[book].path)
    }

    /// Where the index is kept (`None`: in memory).
    pub fn index_file(&self) -> Option<&Path> {
        self.index.file.as_deref()
    }

    /// The index, for searching it.
    pub(crate) fn index(&self) -> Arc<rical_index::Index> {
        self.index.clone()
    }

    /// The book a path is in, by its folder.
    pub fn book_of(&self, path: &Path) -> Option<usize> {
        self.by_dir.get(path.parent()?).copied()
    }

    /// Read one file again after it changed on disk (or drop it if gone).
    /// Returns whether anything changed.
    pub fn reload(&mut self, path: &Path) -> bool {
        self.reload_files(&[path.to_owned()])
    }

    /// [`Books::reload`] for many at once.
    pub fn reload_files(&mut self, paths: &[PathBuf]) -> bool {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter(|p| self.book_of(p).is_some() && is_card_file(p))
            .cloned()
            .collect();
        if paths.is_empty() {
            return false;
        }
        match self.index.sync_files(&Cards, &paths) {
            Ok(changes) => self.changed(&changes),
            Err(e) => {
                log::warn!("could not index {} cards: {e:#}", paths.len());
                false
            }
        }
    }

    /// Catch up with the files: read those changed since they were read
    /// (by size and modification time) and new ones, and drop removed
    /// ones.
    pub fn refresh(&mut self) -> anyhow::Result<bool> {
        let mut paths = Vec::new();
        for book in 0..self.books.len() {
            paths.extend(self.card_paths(book)?);
        }
        let changes = self.index.sync(&Cards, &paths)?;
        Ok(self.changed(&changes))
    }

    fn changed(&mut self, changes: &rical_index::Changes) -> bool {
        if changes.is_empty() {
            return false;
        }
        let read = self.read.get_mut().unwrap_or_else(|e| e.into_inner());
        for p in changes.read.iter().chain(&changes.gone) {
            read.files.remove(p.as_os_str());
        }
        self.generation += 1;
        true
    }

    /// Changes so far: compare to know whether to rebuild.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Count on from `after`: for books read anew to replace older ones,
    /// so that what was worked out from those is seen to be out of date.
    pub fn follow(&mut self, after: u64) {
        self.generation = after + 1;
    }

    /// Files, from those read lately or from disk (on every core).
    fn read_files(&self, paths: &[PathBuf]) -> Vec<Option<Arc<[Component]>>> {
        let mut out = Vec::with_capacity(paths.len());
        let mut missing = Vec::new();
        {
            let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
            read.tick += 1;
            let tick = read.tick;
            for (i, p) in paths.iter().enumerate() {
                match read.files.get_mut(p.as_os_str()) {
                    Some((f, used)) => {
                        *used = tick;
                        out.push(Some(f.clone()));
                    }
                    None => {
                        out.push(None);
                        missing.push(i);
                    }
                }
            }
        }
        if missing.is_empty() {
            return out;
        }
        let fresh = rical_index::on_every_core(&missing, |&i| {
            read_cards(&paths[i])
                .map_err(|e| log::warn!("skipping {:?}: {e}", paths[i]))
                .ok()
                .map(Arc::<[Component]>::from)
        });
        let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        let tick = read.tick;
        for (i, f) in missing.into_iter().zip(fresh) {
            if let Some(f) = &f {
                read.files
                    .insert(paths[i].as_os_str().to_owned(), (f.clone(), tick));
            }
            out[i] = f;
        }
        if read.files.len() > KEEP {
            let mut ticks: Vec<u64> = read.files.values().map(|(_, t)| *t).collect();
            let n = read.files.len() - KEEP / 2;
            let (_, cut, _) = ticks.select_nth_unstable(n);
            let cut = *cut;
            read.files.retain(|_, (_, t)| *t >= cut);
        }
        out
    }

    /// Every card, with its id and book: reads them all, so for tools
    /// rather than views (which have [`Books::listed`] and the search).
    pub fn cards(&self) -> Vec<(CardId, &AddressBook, CardRef)> {
        let ids: Vec<CardId> = self.listed().into_iter().map(|l| l.id).collect();
        let mut paths: Vec<PathBuf> = ids.iter().map(|id| id.path.clone()).collect();
        paths.dedup();
        let files: HashMap<PathBuf, Arc<[Component]>> = paths
            .iter()
            .cloned()
            .zip(self.read_files(&paths))
            .filter_map(|(p, f)| Some((p, f?)))
            .collect();
        ids.into_iter()
            .filter_map(|id| {
                let file = files.get(&id.path)?.clone();
                let book = &self.books[self.book_of(&id.path)?];
                (id.index < file.len()).then(|| {
                    let index = id.index;
                    (id, book, CardRef { file, index })
                })
            })
            .collect()
    }

    /// Every card as the index has it, in the order of their files.
    pub fn listed(&self) -> Vec<Listed> {
        let conn = self.index.conn();
        let st = conn.prepare_cached(
            "SELECT c.file, f.path, c.idx, c.uid, c.name, c.sort, c.detail
             FROM cards c JOIN files f ON f.id = c.file ORDER BY f.path, c.idx",
        );
        st.and_then(|mut st| {
            st.query_map([], |r| {
                Ok(Listed {
                    file: r.get(0)?,
                    id: CardId {
                        path: rical_index::path(r.get(1)?),
                        index: r.get::<_, i64>(2)? as usize,
                    },
                    uid: r.get(3)?,
                    name: r.get(4)?,
                    sort: r.get(5)?,
                    detail: r.get(6)?,
                })
            })?
            .collect()
        })
        .unwrap_or_default()
    }

    /// Every card with what tells the same person, for finding duplicates.
    pub fn keyed(&self) -> Vec<Keyed> {
        let conn = self.index.conn();
        let mut keys: HashMap<i64, (Vec<String>, Vec<String>)> = HashMap::new();
        if let Ok(mut st) = conn.prepare_cached("SELECT card, kind, key FROM keys ORDER BY rowid")
            && let Ok(rows) = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
        {
            for (card, kind, key) in rows.flatten() {
                let k = keys.entry(card).or_default();
                if kind == 0 { &mut k.0 } else { &mut k.1 }.push(key);
            }
        }
        let st = conn.prepare_cached(
            "SELECT c.id, f.path, c.idx, c.uid, c.kind, c.name
             FROM cards c JOIN files f ON f.id = c.file ORDER BY f.path, c.idx",
        );
        st.and_then(|mut st| {
            st.query_map([], |r| {
                let (email_keys, phone_tails) =
                    keys.remove(&r.get::<_, i64>(0)?).unwrap_or_default();
                Ok(Keyed {
                    id: CardId {
                        path: rical_index::path(r.get(1)?),
                        index: r.get::<_, i64>(2)? as usize,
                    },
                    uid: r.get(3)?,
                    kind: r.get(4)?,
                    name: r.get(5)?,
                    email_keys,
                    phone_tails,
                })
            })?
            .collect()
        })
        .unwrap_or_default()
    }

    /// Everyone's addresses: (card, name, address as written), in the
    /// order of their files, for completing addresses.
    pub fn addresses(&self) -> Vec<(CardId, String, String)> {
        let conn = self.index.conn();
        let st = conn.prepare_cached(
            "SELECT f.path, c.idx, c.name, e.address FROM emails e
             JOIN cards c ON c.id = e.card JOIN files f ON f.id = c.file
             ORDER BY f.path, c.idx, e.rowid",
        );
        st.and_then(|mut st| {
            st.query_map([], |r| {
                Ok((
                    CardId {
                        path: rical_index::path(r.get(0)?),
                        index: r.get::<_, i64>(1)? as usize,
                    },
                    r.get(2)?,
                    r.get(3)?,
                ))
            })?
            .collect()
        })
        .unwrap_or_default()
    }

    /// A card with any of these addresses (as compared) or numbers (by
    /// their last digits): the first, by file.
    pub fn with_keys(&self, email_keys: &[String], phone_tails: &[String]) -> Option<CardId> {
        let conn = self.index.conn();
        let mut st = conn
            .prepare_cached(
                "SELECT f.path, c.idx FROM keys k JOIN cards c ON c.id = k.card
                 JOIN files f ON f.id = c.file WHERE k.kind = ?1 AND k.key = ?2",
            )
            .ok()?;
        let mut found: Vec<(Vec<u8>, i64)> = Vec::new();
        for (kind, keys) in [(0, email_keys), (1, phone_tails)] {
            for k in keys {
                if let Ok(rows) = st.query_map(params![kind, k], |r| Ok((r.get(0)?, r.get(1)?))) {
                    found.extend(rows.flatten());
                }
            }
        }
        found.sort();
        found.into_iter().next().map(|(p, i)| CardId {
            path: rical_index::path(p),
            index: i as usize,
        })
    }

    /// How many cards there are.
    pub fn len(&self) -> usize {
        self.index
            .conn()
            .query_row("SELECT count(*) FROM cards", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A card, read from its file if it was not lately.
    pub fn get(&self, id: &CardId) -> Option<CardRef> {
        self.book_of(&id.path)?;
        if !is_card_file(&id.path) {
            return None;
        }
        let file = self
            .read_files(std::slice::from_ref(&id.path))
            .pop()
            .flatten()?;
        (id.index < file.len()).then_some(CardRef {
            file,
            index: id.index,
        })
    }

    /// The book a card is in.
    pub fn book(&self, id: &CardId) -> Option<&AddressBook> {
        self.book_of(&id.path).map(|i| &self.books[i])
    }

    /// The cards with this UID (in any book).
    pub fn by_uid(&self, uid: &str) -> Vec<CardId> {
        let conn = self.index.conn();
        let st = conn.prepare_cached(
            "SELECT f.path, c.idx FROM cards c JOIN files f ON f.id = c.file
             WHERE c.uid = ?1 ORDER BY f.path, c.idx",
        );
        st.and_then(|mut st| {
            st.query_map([uid], |r| {
                Ok(CardId {
                    path: rical_index::path(r.get(0)?),
                    index: r.get::<_, i64>(1)? as usize,
                })
            })?
            .collect()
        })
        .unwrap_or_default()
    }

    /// The book with this id or display name.
    pub fn find_book(&self, name: &str) -> Option<usize> {
        self.books.iter().position(|b| b.id == name).or_else(|| {
            self.books
                .iter()
                .position(|b| b.name.eq_ignore_ascii_case(name))
        })
    }
}

impl Books {
    /// Where a file is, from the contacts folder itself (named, so a book
    /// that is the folder still says something): `contacts/work/sam.vcf`.
    /// Something outside it is shown in full.
    pub fn place(&self, path: &Path) -> String {
        let base = self.root.parent().unwrap_or(&self.root);
        match path.strip_prefix(base) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
            _ => path.display().to_string(),
        }
    }

    /// Where address book `i`'s new cards go, as [`Books::place`] does,
    /// ending in `/`: `contacts/work/`.
    pub fn book_place(&self, i: usize) -> String {
        self.books
            .get(i)
            .map(|b| format!("{}/", self.place(&b.path)))
            .unwrap_or_default()
    }

    /// Whether a file changed on disk since it was indexed (or is gone),
    /// so an edit based on what is shown would undo someone else's.
    pub fn is_stale(&self, path: &Path) -> bool {
        match self.index.stamp_of(path) {
            Some(s) => rical_index::Stamp::of(path) != Some(s),
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_from_the_contacts_folder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("contacts");
        std::fs::create_dir_all(root.join("work")).unwrap();
        std::fs::write(
            root.join("work/sam.vcf"),
            "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:s\r\nFN:Sam\r\nEND:VCARD\r\n",
        )
        .unwrap();
        let b = Books::load(&root).unwrap();
        assert_eq!(b.place(&root.join("work/sam.vcf")), "contacts/work/sam.vcf");
        assert_eq!(b.book_place(0), "contacts/work/");
        assert_eq!(b.place(Path::new("/elsewhere/x.vcf")), "/elsewhere/x.vcf");
        // a folder that is itself the address book
        let b = Books::load(root.join("work")).unwrap();
        assert_eq!(b.book_place(0), "work/");
    }

    #[test]
    fn the_index_is_kept_and_only_changes_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("contacts");
        std::fs::create_dir_all(&root).unwrap();
        let card = |uid: &str, name: &str| {
            format!(
                "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{uid}\r\nFN:{name}\r\nEMAIL:{uid}@example.com\r\nEND:VCARD\r\n"
            )
        };
        std::fs::write(root.join("a.vcf"), card("a", "Ada Lovelace")).unwrap();
        std::fs::write(root.join("b.vcf"), card("b", "Bo Diddley")).unwrap();
        let cache = dir.path().join("cache/contacts.sqlite");
        let b = Books::load_cached(&root, Some(&cache)).unwrap();
        assert_eq!(b.index_file(), Some(cache.as_path()));
        assert_eq!(b.len(), 2);
        // nothing read again
        let mut b = Books::load_cached(&root, Some(&cache)).unwrap();
        assert!(!b.refresh().unwrap());
        let addresses: Vec<(String, String)> =
            b.addresses().into_iter().map(|(_, n, a)| (n, a)).collect();
        assert_eq!(
            addresses,
            [
                ("Ada Lovelace".to_owned(), "a@example.com".to_owned()),
                ("Bo Diddley".to_owned(), "b@example.com".to_owned())
            ]
        );
        assert_eq!(b.by_uid("b")[0].path, root.join("b.vcf"));
        // a card is read when wanted
        let ada = b.get(&b.by_uid("a")[0]).unwrap();
        assert_eq!(ada.card().display_name(), "Ada Lovelace");

        std::fs::write(root.join("a.vcf"), card("a", "Ada King")).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(root.join("a.vcf"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(b.is_stale(&root.join("a.vcf")));
        assert!(b.refresh().unwrap());
        assert!(!b.is_stale(&root.join("a.vcf")));
        let ada = b.get(&b.by_uid("a")[0]).unwrap();
        assert_eq!(ada.card().display_name(), "Ada King");
        assert_eq!(
            b.with_keys(&["b@example.com".to_owned()], &[]),
            Some(CardId {
                path: root.join("b.vcf"),
                index: 0
            })
        );
    }
}

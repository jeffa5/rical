//! A vdir: a folder of collections (calendars), each a folder of `.ics` items.
//!
//! See <https://vdirsyncer.pimutils.org/en/stable/vdir.html>.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    fmt,
    fs::{self, read_dir},
    ops::Range,
    path::{Path, PathBuf},
    str::FromStr,
    sync::{Arc, Mutex},
};

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use log::warn;
use serde::Serialize;

pub use rical_index::Changes;
use rical_index::rusqlite::OptionalExtension as _;

use crate::{
    component::AnyComponent,
    index::{Items, PersonRow},
    occurrence::{self, Kind, Occurrence},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl FromStr for Rgb {
    type Err = String;

    /// Parse `#rrggbb` or `#rrggbbaa` (alpha is ignored).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.trim().trim_start_matches('#');
        if !(hex.len() == 6 || hex.len() == 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("invalid colour {s:?}"));
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
        Ok(Self {
            r: byte(0),
            g: byte(2),
            b: byte(4),
        })
    }
}

impl fmt::Display for Rgb {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl Serialize for Rgb {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// One collection in a vdir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Calendar {
    /// Path relative to the vdir root, e.g. "nextcloud/personal"; stable
    /// across runs.
    pub id: String,
    pub path: PathBuf,
    /// From the `displayname` file, falling back to the folder name; the
    /// whole relative path when names would otherwise clash.
    pub name: String,
    /// The display name or folder name alone, even when it clashes.
    pub label: String,
    /// From the `color` file.
    pub color: Option<Rgb>,
}

impl Calendar {
    /// The calendar in folder `path` under the vdir `root`.
    pub fn load(path: &Path, root: &Path) -> anyhow::Result<Self> {
        let id = match path.strip_prefix(root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy())
                .collect::<Vec<_>>()
                .join("/"),
            _ => path
                .file_name()
                .and_then(|n| n.to_str())
                .with_context(|| format!("calendar path has no name: {path:?}"))?
                .to_owned(),
        };
        let folder = id.rsplit('/').next().unwrap_or(&id).to_owned();
        let label = read_metadata(path, "displayname").unwrap_or(folder);
        let color = read_metadata(path, "color").and_then(|c| match c.parse() {
            Ok(c) => Some(c),
            Err(e) => {
                warn!("{path:?}: {e}");
                None
            }
        });

        Ok(Self {
            id,
            path: path.to_owned(),
            name: label.clone(),
            label,
            color,
        })
    }

    /// Paths of the `.ics` items in this calendar, sorted.
    pub fn item_paths(&self) -> anyhow::Result<Vec<PathBuf>> {
        let mut paths = read_dir(&self.path)
            .with_context(|| format!("reading calendar {:?}", self.path))?
            .filter_map(|e| e.ok())
            .filter(|e| entry_is(e, false))
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "ics"))
            .collect::<Vec<_>>();
        paths.sort();
        Ok(paths)
    }

    /// Every component of every item in this calendar. Unparseable items are
    /// skipped with a warning.
    pub fn components(&self) -> anyhow::Result<Vec<AnyComponent>> {
        Ok(self
            .item_paths()?
            .iter()
            .filter_map(|p| read_item(p))
            .flatten()
            .collect())
    }
}

/// A vdir: its calendars, and an index of their items (see
/// [`crate::index`]) that tells which items can show when, without
/// reading them. Items are read from their files when they are wanted (a
/// query reads those that can show in its range), and the last few
/// thousand read are kept. The files stay the source of truth:
/// [`Vdir::refresh`] catches up with changes to them.
pub struct Vdir {
    pub root: PathBuf,
    /// Sorted by id.
    pub calendars: Vec<Calendar>,
    /// Every folder scanned (the root and those under it), for watching.
    pub dirs: Vec<PathBuf>,
    /// Each calendar's folder, to its place in `calendars`.
    by_dir: HashMap<PathBuf, usize>,
    index: rical_index::Index,
    read: Mutex<Read>,
    /// Counts changes to the items, for what is worked out from them all.
    generation: u64,
    memo: Mutex<Memo>,
}

/// Bringing the index of a vdir up to date on another thread, with its
/// own connection to the index: see [`Vdir::check`].
#[derive(Debug, Clone)]
pub struct Check {
    index: PathBuf,
    calendars: Vec<Calendar>,
    /// Only these files; else every item of `calendars`.
    files: Option<Vec<PathBuf>>,
    /// The items read lately (those on screen among them), to look at
    /// first.
    shown: Vec<PathBuf>,
}

impl Check {
    /// Whether it looks at every file (not only some the watcher saw), so
    /// that [`Check::run_first`] leaves some for [`Check::run`].
    pub fn is_full(&self) -> bool {
        self.files.is_none()
    }

    /// What can be found quickly, to show before the rest: new items and
    /// gone ones (from the folders' listings alone), and changes to those
    /// read lately, which are what is on screen. [`Check::run`] after
    /// finds the rest.
    pub fn run_first(&self) -> anyhow::Result<rical_index::Changes> {
        let index = rical_index::Index::for_schema::<Items>(Some(&self.index));
        match &self.files {
            Some(files) => index.sync_files(&Items, files),
            None => {
                let mut paths = Vec::new();
                for calendar in &self.calendars {
                    paths.extend(calendar.item_paths()?);
                }
                index.sync_first(&Items, &paths, &self.shown)
            }
        }
    }

    /// Look at the files and index what changed. The index is shared, so
    /// the vdir sees the changes at once; [`Vdir::take_changes`] tells it
    /// which items to read again.
    pub fn run(self) -> anyhow::Result<rical_index::Changes> {
        let index = rical_index::Index::for_schema::<Items>(Some(&self.index));
        match self.files {
            Some(files) => index.sync_files(&Items, &files),
            None => {
                let mut paths = Vec::new();
                for calendar in &self.calendars {
                    paths.extend(calendar.item_paths()?);
                }
                index.sync(&Items, &paths)
            }
        }
    }
}

/// Items read lately, by path, with when each was last wanted.
#[derive(Default)]
struct Read {
    files: HashMap<OsString, (Arc<[AnyComponent]>, u64)>,
    tick: u64,
}

/// How many read items are kept: a busy month of a big calendar.
const KEEP: usize = 8192;

/// Everyone on every item: (calendar, file, who).
pub(crate) type People = Vec<(usize, i64, PersonRow)>;

/// What is worked out from every item, at a generation.
#[derive(Default)]
struct Memo {
    tasks: Option<(u64, Arc<Vec<crate::Task>>)>,
    people: Option<(u64, Arc<People>)>,
    counts: Option<(u64, Vec<usize>)>,
}

impl fmt::Debug for Vdir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vdir")
            .field("root", &self.root)
            .field("calendars", &self.calendars)
            .field("index", &self.index)
            .finish()
    }
}

impl Vdir {
    /// Find the calendars under `root`: every folder (at any depth, skipping
    /// hidden ones) that holds `.ics` items or vdir metadata (`displayname`,
    /// `color`). If there are none, `root` itself is one, empty, calendar.
    /// Its items are indexed in memory, reading every one.
    pub fn load(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::load_cached(root, None)
    }

    /// [`Vdir::load`], with the index kept where the apps keep it (see
    /// [`Vdir::default_cache`]): only items changed since it was last
    /// brought up to date are read.
    pub fn open(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let cache = Self::default_cache(root.as_ref());
        Self::load_cached(root, cache.as_deref())
    }

    /// [`Vdir::load`], with the index kept in `cache` (in memory if
    /// `None`), brought up to date.
    pub fn load_cached(root: impl AsRef<Path>, cache: Option<&Path>) -> anyhow::Result<Self> {
        let mut vdir = Self::scan(root)?;
        vdir.index = rical_index::Index::for_schema::<Items>(cache);
        vdir.refresh()?;
        Ok(vdir)
    }

    /// [`Vdir::open`] without looking at the items first: what the index
    /// had when last brought up to date, at once. [`Vdir::check`] brings it
    /// up to date, away from the window. An index never filled is filled
    /// here, as there is nothing to show from it.
    pub fn open_unchecked(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut vdir = Self::scan(root.as_ref())?;
        let cache = Self::default_cache(root.as_ref());
        vdir.index = rical_index::Index::for_schema::<Items>(cache.as_deref());
        if vdir.index.file.is_none() || vdir.index.is_empty() {
            vdir.refresh()?;
        }
        Ok(vdir)
    }

    /// A check of every item against the index, to run on another thread
    /// (see [`Check::run`]) and give back to [`Vdir::take_changes`].
    /// `None` for an index kept in memory, which only this vdir can bring up
    /// to date ([`Vdir::refresh`]).
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
            calendars: self.calendars.clone(),
            files: None,
            shown,
        })
    }

    /// [`Vdir::check`] of only `paths` (after the watcher saw them change).
    pub fn check_files(&self, paths: &[PathBuf]) -> Option<Check> {
        let files: Vec<PathBuf> = paths
            .iter()
            .filter(|p| {
                self.calendar_index(p).is_some() && p.extension().is_some_and(|e| e == "ics")
            })
            .cloned()
            .collect();
        (!files.is_empty()).then_some(())?;
        Some(Check {
            index: self.index.file.clone()?,
            calendars: Vec::new(),
            files: Some(files),
            shown: Vec::new(),
        })
    }

    /// Take what a [`Check`] found: items it read again are read again
    /// here when next wanted. Returns whether anything changed.
    pub fn take_changes(&mut self, changes: &rical_index::Changes) -> bool {
        self.changed(changes)
    }

    /// Where [`Vdir::open`] keeps the index for `root`.
    pub fn default_cache(root: &Path) -> Option<PathBuf> {
        rical_index::cache_file("calendar", root)
    }

    /// The calendars under `root`, with an empty index: quick, to show
    /// something while [`Vdir::open`] brings the index up to date.
    pub fn scan(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        if !root.is_dir() {
            anyhow::bail!("vdir root is not a directory: {root:?}");
        }

        let mut dirs = Vec::new();
        let mut seen = std::collections::HashSet::new();
        scan(root, 0, &mut dirs, &mut seen);
        let mut calendars = dirs
            .iter()
            .filter(|d| is_calendar_dir(d))
            .map(|d| Calendar::load(d, root))
            .collect::<anyhow::Result<Vec<_>>>()?;
        if calendars.is_empty() {
            calendars.push(Calendar::load(root, root)?);
        }
        calendars.sort_by(|a, b| a.id.cmp(&b.id));

        // the same display name twice (two accounts, say): use the paths
        let mut counts = BTreeMap::<String, usize>::new();
        for c in &calendars {
            *counts.entry(c.name.clone()).or_default() += 1;
        }
        for c in &mut calendars {
            if counts[&c.name] > 1 {
                c.name = c.id.clone();
            }
        }
        let by_dir = calendars
            .iter()
            .enumerate()
            .map(|(i, c)| (c.path.clone(), i))
            .collect();

        Ok(Self {
            root: root.to_owned(),
            calendars,
            dirs,
            by_dir,
            index: rical_index::Index::for_schema::<Items>(None),
            read: Mutex::default(),
            generation: 0,
            memo: Mutex::default(),
        })
    }

    /// Where the index is kept (`None`: in memory).
    pub fn index_file(&self) -> Option<&Path> {
        self.index.file.as_deref()
    }

    /// Changes so far to the items: compare to know when what was worked
    /// out from them is out of date.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// An item with a VTIMEZONE for `tzid`, if any.
    pub fn timezone_file(&self, tzid: &str) -> Option<PathBuf> {
        self.index
            .conn()
            .prepare_cached(
                "SELECT f.path FROM items i JOIN files f ON f.id = i.file
                 WHERE instr(i.tzids, ?1) > 0 ORDER BY f.path LIMIT 1",
            )
            .and_then(|mut s| {
                s.query_row([format!("\n{tzid}\n")], |r| r.get(0))
                    .optional()
            })
            .ok()
            .flatten()
            .map(rical_index::path)
    }

    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    /// The calendar whose folder contains `path`.
    pub fn calendar_for_path(&self, path: &Path) -> Option<&Calendar> {
        self.calendar_index(path).map(|i| &self.calendars[i])
    }

    fn calendar_index(&self, path: &Path) -> Option<usize> {
        self.by_dir.get(path.parent()?).copied()
    }

    /// The parsed components of one item (without its VTIMEZONEs), read
    /// from its file if it was not lately.
    pub fn get(&self, path: &Path) -> Option<Arc<[AnyComponent]>> {
        self.calendar_index(path)?;
        if path.extension().is_none_or(|e| e != "ics") {
            return None;
        }
        self.read_items(std::slice::from_ref(&path.to_owned()))
            .pop()
            .flatten()
    }

    /// How many items there are.
    pub fn len(&self) -> usize {
        self.index
            .conn()
            .query_row("SELECT count(*) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many items each calendar has, in the order of `calendars`:
    /// counted once per change.
    pub fn item_counts(&self) -> Vec<usize> {
        if let Some((g, counts)) = &self.memo.lock().unwrap_or_else(|e| e.into_inner()).counts
            && *g == self.generation
        {
            return counts.clone();
        }
        let mut counts = vec![0; self.calendars.len()];
        for path in self.paths_where("1", &[]) {
            if let Some(i) = self.calendar_index(&path) {
                counts[i] += 1;
            }
        }
        self.memo.lock().unwrap_or_else(|e| e.into_inner()).counts =
            Some((self.generation, counts.clone()));
        counts
    }

    /// Every component of every item, with the calendar it belongs to:
    /// reads them all, so for tools rather than views.
    pub fn components(&self) -> impl Iterator<Item = (&Calendar, AnyComponent)> {
        self.items_where("1", &[])
            .into_iter()
            .flat_map(|(cal, item)| {
                let calendar = &self.calendars[cal];
                item.iter()
                    .map(|c| (calendar, c.clone()))
                    .collect::<Vec<_>>()
            })
    }

    /// Every item holding a component of `kind`, with its calendar.
    pub(crate) fn items_of(&self, kind: Kind) -> Vec<(usize, Arc<[AnyComponent]>)> {
        self.items_where("(i.kinds & ?1) != 0", &[&crate::index::kind_bit(kind)])
    }

    /// The items `filter` (on `items i`) takes, in path order.
    fn paths_where(
        &self,
        filter: &str,
        params: &[&dyn rical_index::rusqlite::ToSql],
    ) -> Vec<PathBuf> {
        let conn = self.index.conn();
        let sql = format!(
            "SELECT f.path FROM items i JOIN files f ON f.id = i.file WHERE {filter} ORDER BY f.path"
        );
        let Ok(mut st) = conn.prepare_cached(&sql) else {
            return Vec::new();
        };
        st.query_map(params, |r| r.get(0))
            .map(|rows| rows.flatten().map(rical_index::path).collect())
            .unwrap_or_default()
    }

    fn items_where(
        &self,
        filter: &str,
        params: &[&dyn rical_index::rusqlite::ToSql],
    ) -> Vec<(usize, Arc<[AnyComponent]>)> {
        let paths = self.paths_where(filter, params);
        paths
            .iter()
            .zip(self.read_items(&paths))
            .filter_map(|(p, item)| Some((self.calendar_index(p)?, item?)))
            .collect()
    }

    /// Items, from those read lately or their files (on every core).
    fn read_items(&self, paths: &[PathBuf]) -> Vec<Option<Arc<[AnyComponent]>>> {
        let mut out: Vec<Option<Arc<[AnyComponent]>>> = Vec::with_capacity(paths.len());
        let mut missing = Vec::new();
        {
            let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
            read.tick += 1;
            let tick = read.tick;
            for (i, p) in paths.iter().enumerate() {
                match read.files.get_mut(p.as_os_str()) {
                    Some((item, used)) => {
                        *used = tick;
                        out.push(Some(item.clone()));
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
            read_item(&paths[i]).map(Arc::<[AnyComponent]>::from)
        });
        let mut read = self.read.lock().unwrap_or_else(|e| e.into_inner());
        let tick = read.tick;
        for (i, item) in missing.into_iter().zip(fresh) {
            if let Some(item) = &item {
                read.files
                    .insert(paths[i].as_os_str().to_owned(), (item.clone(), tick));
            }
            out[i] = item;
        }
        // too many: keep the half wanted latest
        if read.files.len() > KEEP {
            let mut ticks: Vec<u64> = read.files.values().map(|(_, t)| *t).collect();
            let (_, cut, _) = ticks.select_nth_unstable(read.files.len() - KEEP / 2);
            let cut = *cut;
            read.files.retain(|_, (_, t)| *t >= cut);
        }
        out
    }

    /// Forget items read before `changes`, and what was worked out from
    /// them all.
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

    /// The tasks, worked out by `build` from the items once per change.
    pub(crate) fn remembered_tasks(
        &self,
        build: impl FnOnce() -> Vec<crate::Task>,
    ) -> Arc<Vec<crate::Task>> {
        let memo = self.memo.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((g, tasks)) = &memo.tasks
            && *g == self.generation
        {
            return tasks.clone();
        }
        drop(memo);
        let tasks = Arc::new(build());
        self.memo.lock().unwrap_or_else(|e| e.into_inner()).tasks =
            Some((self.generation, tasks.clone()));
        tasks
    }

    /// Everyone on every item: (calendar, file, who), in the order of the
    /// files and their components.
    pub(crate) fn people_rows(&self) -> Arc<People> {
        if let Some((g, rows)) = &self.memo.lock().unwrap_or_else(|e| e.into_inner()).people
            && *g == self.generation
        {
            return rows.clone();
        }
        let rows: People = {
            let conn = self.index.conn();
            let st = conn.prepare_cached(
                "SELECT f.path, i.file, i.people FROM items i JOIN files f ON f.id = i.file
                 WHERE i.people != '' ORDER BY f.path",
            );
            st.and_then(|mut st| {
                st.query_map([], |r| {
                    Ok((
                        rical_index::path(r.get(0)?),
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()
            })
            .unwrap_or_default()
            .into_iter()
            .flat_map(|(path, file, people)| {
                PersonRow::parse(&people)
                    .map(|p| (path.clone(), file, p))
                    .collect::<Vec<_>>()
            })
            .filter_map(|(path, file, p)| Some((self.calendar_index(&path)?, file, p)))
            .collect()
        };
        let rows = Arc::new(rows);
        self.memo.lock().unwrap_or_else(|e| e.into_inner()).people =
            Some((self.generation, rows.clone()));
        rows
    }

    /// The file holding `uid`, in any calendar.
    pub(crate) fn path_of(&self, uid: &str) -> Option<PathBuf> {
        self.index
            .conn()
            .prepare_cached(
                "SELECT f.path FROM uids u JOIN files f ON f.id = u.file
                 WHERE u.uid = ?1 ORDER BY f.path LIMIT 1",
            )
            .and_then(|mut s| s.query_row([uid], |r| r.get(0)).optional())
            .ok()
            .flatten()
            .map(rical_index::path)
    }

    /// Addresses that are probably the owner's: per calendar, the one on
    /// (as organiser or attendee) at least half of the items with
    /// attendees. Invitations land in the invitee's calendar, so this finds
    /// each account's address without being told.
    pub fn owner_addresses(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for email in self.owners().into_values() {
            if !out.contains(&email) {
                out.push(email);
            }
        }
        out
    }

    /// Per calendar id, the address its server sends invitations as: the
    /// organiser of the events it scheduled (marked SCHEDULE-STATUS).
    /// Organising as anyone else, the server would send nothing.
    pub fn senders(&self) -> BTreeMap<String, String> {
        let rows = self.people_rows();
        let mut counts: BTreeMap<usize, BTreeMap<String, usize>> = BTreeMap::new();
        let mut done = std::collections::HashSet::new();
        for (cal, file, p) in rows.iter() {
            // a component's first organiser
            if !p.scheduled || p.role != 0 || !done.insert((*file, p.part)) {
                continue;
            }
            if let Some(email) = &p.email {
                *counts
                    .entry(*cal)
                    .or_default()
                    .entry(email.to_lowercase())
                    .or_default() += 1;
            }
        }
        counts
            .into_iter()
            .filter_map(|(cal, c)| {
                let (email, _) = c.into_iter().max_by_key(|(_, n)| *n)?;
                Some((self.calendars[cal].id.clone(), email))
            })
            .collect()
    }

    /// The owner's address per calendar id, where it can be told; see
    /// [`Vdir::owner_addresses`].
    pub fn owners(&self) -> BTreeMap<String, String> {
        let rows = self.people_rows();
        let mut by_calendar: BTreeMap<usize, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
        // each component's people together: they are in order
        let mut i = 0;
        while i < rows.len() {
            let (cal, file, first) = &rows[i];
            let mut j = i;
            while j < rows.len() && rows[j].1 == *file && rows[j].2.part == first.part {
                j += 1;
            }
            let part = &rows[i..j];
            i = j;
            if !part.iter().any(|(_, _, p)| p.role == 1) {
                continue;
            }
            let mut emails = part
                .iter()
                .filter_map(|(_, _, p)| p.email.as_deref().map(str::to_lowercase))
                .collect::<Vec<_>>();
            emails.sort();
            emails.dedup();
            let (items, counts) = by_calendar.entry(*cal).or_default();
            *items += 1;
            for e in emails {
                *counts.entry(e).or_default() += 1;
            }
        }
        let mut out = BTreeMap::new();
        for (calendar, (items, counts)) in by_calendar {
            let top = counts.into_iter().max_by_key(|(_, n)| *n);
            if let Some((email, n)) = top
                && items >= 2
                && n * 2 >= items
            {
                out.insert(self.calendars[calendar].id.clone(), email);
            }
        }
        out
    }

    /// Read one item again after it changed on disk; one that is gone (or
    /// cannot be read) is dropped. Returns whether anything changed.
    pub fn reload(&mut self, path: &Path) -> bool {
        self.reload_files(&[path.to_owned()])
    }

    /// [`Vdir::reload`] for many at once (one write to the index).
    pub fn reload_files(&mut self, paths: &[PathBuf]) -> bool {
        let paths: Vec<PathBuf> = paths
            .iter()
            .filter(|p| {
                self.calendar_index(p).is_some() && p.extension().is_some_and(|e| e == "ics")
            })
            .cloned()
            .collect();
        if paths.is_empty() {
            return false;
        }
        match self.index.sync_files(&Items, &paths) {
            Ok(changes) => self.changed(&changes),
            Err(e) => {
                warn!("could not index {} items: {e:#}", paths.len());
                false
            }
        }
    }

    /// Catch up with the files: read those changed since they were read
    /// (by their size and modification time) and new ones, and drop
    /// removed ones. Calendars added or removed need
    /// [`Vdir::reload_all`]. Returns whether anything changed.
    pub fn refresh(&mut self) -> anyhow::Result<bool> {
        let mut paths = Vec::new();
        for calendar in &self.calendars {
            paths.extend(calendar.item_paths()?);
        }
        let changes = self.index.sync(&Items, &paths)?;
        Ok(self.changed(&changes))
    }

    /// Look again for calendars, their metadata and items.
    pub fn reload_all(&mut self) -> anyhow::Result<()> {
        let fresh = Self::scan(&self.root)?;
        self.calendars = fresh.calendars;
        self.dirs = fresh.dirs;
        self.by_dir = fresh.by_dir;
        self.read
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .files
            .clear();
        self.generation += 1;
        self.refresh()?;
        Ok(())
    }

    /// Read what showing `range` reads, and the tasks, so that they are
    /// there when shown: for a vdir being opened away from the window.
    pub fn warm(&self, range: &Range<DateTime<Utc>>, local: Tz) {
        self.query(range, &[Kind::Event, Kind::Todo, Kind::Journal], local);
        self.all_tasks();
    }

    /// Every occurrence of the given kinds that overlaps `range`, for a
    /// viewer in `local`, sorted by start.
    pub fn query(
        &self,
        range: &Range<DateTime<Utc>>,
        kinds: &[Kind],
        local: Tz,
    ) -> Vec<Occurrence> {
        self.query_matching(range, kinds, local, None)
    }

    /// [`Vdir::query`], of only the items whose text (titles, places,
    /// notes and people, in lower case) has `text` in it, if given: for a
    /// search, which need not work out the occurrences of everything
    /// else.
    pub fn query_matching(
        &self,
        range: &Range<DateTime<Utc>>,
        kinds: &[Kind],
        local: Tz,
        text: Option<&str>,
    ) -> Vec<Occurrence> {
        let paths = self.can_show(range, kinds, text);
        let read = self.read_items(&paths);
        // by calendar, as a series is matched up within one
        let mut items: Vec<(usize, Arc<[AnyComponent]>)> = paths
            .iter()
            .zip(read)
            .filter_map(|(p, item)| Some((self.calendar_index(p)?, item?)))
            .collect();
        items.sort_by_key(|(cal, _)| *cal);
        let expand = |items: &[(usize, Arc<[AnyComponent]>)]| -> Vec<Occurrence> {
            let mut out = Vec::new();
            for (cal, item) in items {
                let calendar = &self.calendars[*cal].id;
                out.extend(
                    occurrence::expand(calendar, item.iter(), range, local)
                        .into_iter()
                        .filter(|o| kinds.contains(&o.kind)),
                );
            }
            out
        };
        // threads pay off past a few hundred files
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(8);
        let out: Vec<Occurrence> = if items.len() < 400 || threads < 2 {
            expand(&items)
        } else {
            let chunk = items.len().div_ceil(threads);
            std::thread::scope(|s| {
                let handles: Vec<_> = items
                    .chunks(chunk)
                    .map(|c| s.spawn(move || expand(c)))
                    .collect();
                handles
                    .into_iter()
                    .flat_map(|h| h.join().expect("a query thread panicked"))
                    .collect()
            })
        };
        // by start, then title: sorted as numbers (each start worked out
        // once), and the occurrences moved into place once
        let starts: Vec<DateTime<Utc>> = out.iter().map(|o| o.start.to_utc(local)).collect();
        let mut order: Vec<usize> = (0..out.len()).collect();
        order.sort_by(|&a, &b| {
            (starts[a], &out[a].summary, a).cmp(&(starts[b], &out[b].summary, b))
        });
        let mut slots: Vec<Option<Occurrence>> = out.into_iter().map(Some).collect();
        order.into_iter().filter_map(|i| slots[i].take()).collect()
    }

    /// The items of `kinds` that can show in `range` (by the index), and
    /// whose text has `text`, in path order.
    fn can_show(
        &self,
        range: &Range<DateTime<Utc>>,
        kinds: &[Kind],
        text: Option<&str>,
    ) -> Vec<PathBuf> {
        let bits = kinds.iter().fold(0, |b, k| b | crate::index::kind_bit(*k));
        let matching = if text.is_some() {
            "AND instr(i.search, ?4) > 0"
        } else {
            "AND ?4 IS NULL"
        };
        // those bounded, through the R*Tree, and those that are not
        let sql = format!(
            "SELECT f.path FROM spans s JOIN items i ON i.file = s.id JOIN files f ON f.id = s.id
             WHERE s.lo <= ?3 AND s.hi >= ?2 AND (i.kinds & ?1) != 0 {matching}
             UNION ALL
             SELECT f.path FROM items i JOIN files f ON f.id = i.file
             WHERE i.bounded = 0 AND (i.kinds & ?1) != 0 {matching}"
        );
        let conn = self.index.conn();
        let Ok(mut st) = conn.prepare_cached(&sql) else {
            return Vec::new();
        };
        let mut paths: Vec<Vec<u8>> = st
            .query_map(
                rical_index::rusqlite::params![
                    bits,
                    range.start.timestamp() as f64,
                    range.end.timestamp() as f64,
                    text
                ],
                |r| r.get(0),
            )
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default();
        paths.sort_unstable();
        paths.into_iter().map(rical_index::path).collect()
    }
}

/// The components of an item, without its VTIMEZONEs.
fn read_item(path: &Path) -> Option<Vec<AnyComponent>> {
    let parsed = std::fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(crate::compact::parse(&text)?));
    match parsed {
        Ok((components, _)) => {
            // one copy of the path for all of a file's components
            let shared: Arc<Path> = path.into();
            Some(
                components
                    .into_iter()
                    .map(|c| AnyComponent::new(shared.clone(), c))
                    .collect(),
            )
        }
        Err(e) => {
            warn!("skipping {path:?}: {e}");
            None
        }
    }
}
/// Collect `dir` and the non-hidden folders under it, following symlinks
/// but never into a folder twice.
fn scan(
    dir: &Path,
    depth: usize,
    dirs: &mut Vec<PathBuf>,
    seen: &mut std::collections::HashSet<PathBuf>,
) {
    const MAX_DEPTH: usize = 12;
    let Ok(canonical) = dir.canonicalize() else {
        return;
    };
    if depth > MAX_DEPTH || !seen.insert(canonical) {
        return;
    }
    dirs.push(dir.to_owned());
    let Ok(entries) = read_dir(dir) else {
        warn!("cannot read {dir:?}");
        return;
    };
    let mut children = entries
        .filter_map(|e| e.ok())
        .filter(|e| entry_is(e, true))
        .map(|e| e.path())
        .filter(|p| !is_hidden(p))
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        scan(&child, depth + 1, dirs, seen);
    }
}

/// A folder holding `.ics` files or vdir calendar metadata, or an empty
/// folder (a collection vdirsyncer has created but not filled yet).
fn is_calendar_dir(dir: &Path) -> bool {
    if dir.join("displayname").is_file() || dir.join("color").is_file() {
        return true;
    }
    let Ok(entries) = read_dir(dir) else {
        return false;
    };
    let mut empty = true;
    for e in entries.filter_map(|e| e.ok()) {
        empty = false;
        if Path::new(&e.file_name())
            .extension()
            .is_some_and(|x| x == "ics")
            && entry_is(&e, false)
        {
            return true;
        }
    }
    empty
}

/// Whether a directory entry is a folder (`dir`) or a file, following a
/// symlink; without a look at the file itself unless it is one.
fn entry_is(e: &fs::DirEntry, dir: bool) -> bool {
    match e.file_type() {
        Ok(t) if t.is_symlink() => {
            let p = e.path();
            if dir { p.is_dir() } else { p.is_file() }
        }
        Ok(t) => {
            if dir {
                t.is_dir()
            } else {
                t.is_file()
            }
        }
        Err(_) => false,
    }
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.starts_with('.'))
}

fn read_metadata(dir: &Path, name: &str) -> Option<String> {
    let s = fs::read_to_string(dir.join(name)).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Passing over items by their span finds every occurrence that
    /// working out all of them does.
    #[test]
    fn spans_never_hide_an_occurrence() {
        let d = tempfile::tempdir().unwrap();
        let cal = d.path().join("c");
        fs::create_dir(&cal).unwrap();
        let items = [
            "DTSTART:20250310T090000Z\r\nDTEND:20250310T100000Z",
            "DTSTART;VALUE=DATE:20250311\r\nDTEND;VALUE=DATE:20250312",
            "DTSTART:20250312T233000\r\nDTEND:20250313T003000",
            "DTSTART:20250101T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=WEEKLY;COUNT=12",
            "DTSTART;TZID=Europe/London:20240105T090000\r\nDURATION:PT1H\r\nRRULE:FREQ=MONTHLY;UNTIL=20250401T000000Z",
            "DTSTART:20200101T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=YEARLY",
            "DTSTART:20250601T090000Z\r\nDURATION:PT1H\r\nRDATE:20250301T090000Z,20250315T090000Z",
        ];
        for (i, body) in items.iter().enumerate() {
            fs::write(
                cal.join(format!("{i}.ics")),
                format!(
                    "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u{i}\r\n{body}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
                ),
            )
            .unwrap();
        }
        // an instance of the weekly series moved months away
        fs::write(
            cal.join("moved.ics"),
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:m\r\nDTSTART:20250101T090000Z\r\nDURATION:PT1H\r\nRRULE:FREQ=WEEKLY;COUNT=3\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:m\r\nRECURRENCE-ID:20250108T090000Z\r\nDTSTART:20250910T090000Z\r\nDURATION:PT1H\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let v = Vdir::load(d.path()).unwrap();
        let key = |o: &Occurrence| (o.uid.clone(), o.start.to_utc(Tz::UTC));
        for local in [
            Tz::UTC,
            chrono_tz::Pacific::Kiritimati,
            chrono_tz::Pacific::Pago_Pago,
        ] {
            for (from, days) in [
                ((2025, 3, 10), 1),
                ((2025, 3, 11), 1),
                ((2025, 3, 13), 1),
                ((2025, 3, 1), 31),
                ((2025, 9, 8), 7),
                ((2024, 1, 1), 800),
                ((2030, 1, 1), 7),
            ] {
                let start = chrono::NaiveDate::from_ymd_opt(from.0, from.1, from.2)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap()
                    .and_utc();
                let range = start..start + chrono::TimeDelta::days(days);
                let mut quick: Vec<_> = v
                    .query(&range, &[Kind::Event], local)
                    .iter()
                    .map(key)
                    .collect();
                let mut all: Vec<_> = occurrence::expand(
                    &v.calendars[0].id,
                    &v.components().map(|(_, c)| c).collect::<Vec<_>>(),
                    &range,
                    local,
                )
                .iter()
                .map(key)
                .collect();
                quick.sort();
                all.sort();
                assert_eq!(quick, all, "{range:?} in {local}");
                assert!(!all.is_empty() || from.0 == 2030 || days == 1);
            }
        }
    }

    /// The index is kept between runs, only changes are read again, and
    /// what it knows (search text, UIDs, people, tasks) follows the files.
    #[test]
    fn the_index_is_kept_and_follows_the_files() {
        let d = tempfile::tempdir().unwrap();
        let cal = d.path().join("home");
        fs::create_dir(&cal).unwrap();
        let event = |uid: &str, extra: &str| {
            format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:20250310T090000Z\r\n\
                 DURATION:PT1H\r\nSUMMARY:Standup\r\n{extra}END:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        };
        fs::write(
            cal.join("a.ics"),
            event("a", "DESCRIPTION:Bring the\\, quarterly numbers\r\n"),
        )
        .unwrap();
        fs::write(
            cal.join("b.ics"),
            event("b", "ATTENDEE;CN=Priya Shah:mailto:priya@example.com\r\n"),
        )
        .unwrap();
        let cache = d.path().join("cache/calendar.sqlite");
        let v = Vdir::load_cached(d.path(), Some(&cache)).unwrap();
        assert_eq!(v.index_file(), Some(cache.as_path()));
        assert_eq!(v.len(), 2);
        drop(v);

        let mut v = Vdir::load_cached(d.path(), Some(&cache)).unwrap();
        assert!(!v.refresh().unwrap(), "nothing to read again");
        let range = |day| {
            let s = chrono::NaiveDate::from_ymd_opt(2025, 3, day)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc();
            s..s + chrono::TimeDelta::days(1)
        };
        let found = |v: &Vdir, text| {
            v.query_matching(&range(10), &[Kind::Event], Tz::UTC, Some(text))
                .into_iter()
                .map(|o| o.uid)
                .collect::<Vec<_>>()
        };
        assert_eq!(found(&v, "quarterly numbers"), ["a"]);
        assert_eq!(found(&v, ", quarterly"), ["a"]);
        assert_eq!(found(&v, "priya"), ["b"]);
        assert_eq!(found(&v, "standup").len(), 2);
        assert!(v.query(&range(11), &[Kind::Event], Tz::UTC).is_empty());
        assert_eq!(v.path_of("b"), Some(cal.join("b.ics")));
        assert_eq!(v.people(&[])[0].guest.email, "priya@example.com");
        assert!(v.tasks().is_empty());

        fs::write(
            cal.join("t.ics"),
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t\r\nSUMMARY:Pay rent\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        fs::remove_file(cal.join("a.ics")).unwrap();
        assert!(v.refresh().unwrap());
        assert_eq!(v.tasks()[0].summary, "Pay rent");
        assert!(found(&v, "quarterly").is_empty());
        assert_eq!(v.item_counts(), [2]);
    }

    /// A check on another thread finds what changed, and the vdir reads
    /// those items again.
    #[test]
    fn a_check_elsewhere_brings_the_vdir_up_to_date() {
        let d = tempfile::tempdir().unwrap();
        let cal = d.path().join("home");
        fs::create_dir(&cal).unwrap();
        let event = |summary: &str| {
            format!(
                "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART:20250310T090000Z\r\n\
                 DURATION:PT1H\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
            )
        };
        fs::write(cal.join("a.ics"), event("Before")).unwrap();
        let cache = d.path().join("cache/calendar.sqlite");
        let mut v = Vdir::load_cached(d.path(), Some(&cache)).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2025, 3, 10)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let range = day..day + chrono::TimeDelta::days(1);
        let summary = |v: &Vdir| v.query(&range, &[Kind::Event], Tz::UTC)[0].summary.clone();
        assert_eq!(summary(&v), "Before");

        fs::write(cal.join("a.ics"), event("After!")).unwrap();
        fs::write(cal.join("b.ics"), event("New")).unwrap();
        let check = v.check().unwrap();
        let changes = std::thread::spawn(move || check.run())
            .join()
            .unwrap()
            .unwrap();
        assert_eq!(changes.read.len(), 2);
        assert!(v.take_changes(&changes));
        assert_eq!(summary(&v), "After!");
        assert_eq!(v.len(), 2);
        // and only the files it is given
        fs::remove_file(cal.join("b.ics")).unwrap();
        let check = v.check_files(&[cal.join("b.ics")]).unwrap();
        assert!(v.take_changes(&check.run().unwrap()));
        assert_eq!(v.len(), 1);
        // an index in memory is only this vdir's to bring up to date
        assert!(Vdir::load(d.path()).unwrap().check().is_none());
    }

    #[test]
    fn parse_rgb() {
        assert_eq!(
            "#0082c9".parse::<Rgb>().unwrap(),
            Rgb {
                r: 0x00,
                g: 0x82,
                b: 0xc9
            }
        );
        assert_eq!(
            "#A5B872FF\n".parse::<Rgb>().unwrap(),
            Rgb {
                r: 0xa5,
                g: 0xb8,
                b: 0x72
            }
        );
        assert!("#123".parse::<Rgb>().is_err());
        assert!("#zzzzzz".parse::<Rgb>().is_err());
    }

    #[test]
    fn display_rgb() {
        assert_eq!(
            Rgb {
                r: 1,
                g: 0xab,
                b: 0xff
            }
            .to_string(),
            "#01abff"
        );
    }
}

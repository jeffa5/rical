//! A vdir: a folder of collections (calendars), each a folder of `.ics` items.
//!
//! See <https://vdirsyncer.pimutils.org/en/stable/vdir.html>.

use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, read_dir},
    ops::Range,
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use log::warn;
use serde::Serialize;

use crate::{
    component::AnyComponent,
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
            .map(|e| e.path())
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "ics"))
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

/// A loaded vdir. Every item is parsed into memory on load; the files stay
/// the source of truth and [`Vdir::reload`] picks up changes to them.
#[derive(Clone)]
pub struct Vdir {
    pub root: PathBuf,
    /// Sorted by id.
    pub calendars: Vec<Calendar>,
    /// Every folder scanned (the root and those under it), for watching.
    pub dirs: Vec<PathBuf>,
    /// Parsed components of each item (without VTIMEZONEs) with the index of
    /// its calendar, so queries never compare paths; keyed by the path's raw
    /// text, which orders by plain byte comparison (paths compare piece by
    /// piece, slowly).
    items: BTreeMap<std::ffi::OsString, Item>,
    /// The TZIDs each item has a VTIMEZONE for, to copy one into a file
    /// being written; read from the file when needed.
    timezones: BTreeMap<std::ffi::OsString, Vec<String>>,
}

/// One loaded item: its calendar, when its file last changed (to tell
/// when it needs reading again), and its parsed components.
#[derive(Clone)]
struct Item {
    calendar: usize,
    modified: Option<std::time::SystemTime>,
    components: Vec<AnyComponent>,
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
impl fmt::Debug for Vdir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vdir")
            .field("root", &self.root)
            .field("calendars", &self.calendars)
            .field("items", &self.items.len())
            .finish()
    }
}

impl Vdir {
    /// Find the calendars under `root`: every folder (at any depth, skipping
    /// hidden ones) that holds `.ics` items or vdir metadata (`displayname`,
    /// `color`). If there are none, `root` itself is one, empty, calendar.
    pub fn load(root: impl AsRef<Path>) -> anyhow::Result<Self> {
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

        let mut vdir = Self {
            root: root.to_owned(),
            calendars,
            dirs,
            items: BTreeMap::new(),
            timezones: BTreeMap::new(),
        };
        for (index, calendar) in vdir.calendars.iter().enumerate() {
            for path in calendar.item_paths()? {
                if let Some((components, tzids)) = read_item_without_timezones(&path) {
                    if !tzids.is_empty() {
                        vdir.timezones.insert(path.clone().into_os_string(), tzids);
                    }
                    let modified = modified(&path);
                    vdir.items.insert(
                        path.into_os_string(),
                        Item {
                            calendar: index,
                            modified,
                            components,
                        },
                    );
                }
            }
        }
        Ok(vdir)
    }

    /// An item with a VTIMEZONE for `tzid`, if any.
    pub fn timezone_file(&self, tzid: &str) -> Option<&Path> {
        self.timezones
            .iter()
            .find(|(_, ids)| ids.iter().any(|i| i == tzid))
            .map(|(p, _)| Path::new(p))
    }

    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    /// The calendar whose folder contains `path`.
    pub fn calendar_for_path(&self, path: &Path) -> Option<&Calendar> {
        self.calendars
            .iter()
            .find(|c| path.parent() == Some(c.path.as_path()))
    }

    /// The parsed components of one item.
    pub fn get(&self, path: &Path) -> Option<&[AnyComponent]> {
        self.items
            .get(path.as_os_str())
            .map(|i| i.components.as_slice())
    }

    /// Every loaded component, with the calendar it belongs to.
    pub fn components(&self) -> impl Iterator<Item = (&Calendar, &AnyComponent)> {
        self.items.values().flat_map(|item| {
            let calendar = &self.calendars[item.calendar];
            item.components.iter().map(move |c| (calendar, c))
        })
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
        let mut counts: BTreeMap<&str, BTreeMap<String, usize>> = BTreeMap::new();
        for (calendar, component) in self.components() {
            if !occurrence::server_scheduled(component) {
                continue;
            }
            if let Some(email) = occurrence::people(component, "ORGANIZER")
                .into_iter()
                .next()
                .and_then(|p| p.email)
            {
                *counts
                    .entry(&calendar.id)
                    .or_default()
                    .entry(email.to_lowercase())
                    .or_default() += 1;
            }
        }
        counts
            .into_iter()
            .filter_map(|(cal, c)| {
                let (email, _) = c.into_iter().max_by_key(|(_, n)| *n)?;
                Some((cal.to_owned(), email))
            })
            .collect()
    }

    /// The owner's address per calendar id, where it can be told; see
    /// [`Vdir::owner_addresses`].
    pub fn owners(&self) -> BTreeMap<String, String> {
        let mut by_calendar: BTreeMap<&str, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
        for (calendar, component) in self.components() {
            let attendees = occurrence::people(component, "ATTENDEE");
            if attendees.is_empty() {
                continue;
            }
            let mut emails = attendees
                .iter()
                .chain(&occurrence::people(component, "ORGANIZER"))
                .filter_map(|p| p.email.as_deref().map(str::to_lowercase))
                .collect::<Vec<_>>();
            emails.sort();
            emails.dedup();
            let (items, counts) = by_calendar.entry(&calendar.id).or_default();
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
                out.insert(calendar.to_owned(), email);
            }
        }
        out
    }

    /// Re-read one item after it changed on disk. A missing or unparseable
    /// file is dropped. Returns whether anything changed.
    pub fn reload(&mut self, path: &Path) -> bool {
        let Some(index) = self
            .calendars
            .iter()
            .position(|c| path.parent() == Some(c.path.as_path()))
        else {
            return false;
        };
        if path.extension().is_none_or(|e| e != "ics") {
            return false;
        }
        match path
            .is_file()
            .then(|| read_item_without_timezones(path))
            .flatten()
        {
            Some((components, tzids)) => {
                if tzids.is_empty() {
                    self.timezones.remove(path.as_os_str());
                } else {
                    self.timezones.insert(path.as_os_str().to_owned(), tzids);
                }
                self.items.insert(
                    path.as_os_str().to_owned(),
                    Item {
                        calendar: index,
                        modified: modified(path),
                        components,
                    },
                );
                true
            }
            None => {
                self.timezones.remove(path.as_os_str());
                self.items.remove(path.as_os_str()).is_some()
            }
        }
    }

    /// Catch up with the files: re-read those changed since they were read
    /// (by their modification time), read new ones and drop removed ones.
    /// Far cheaper than [`Vdir::reload_all`] when little has changed, which
    /// is most of the time. Calendars added or removed need `reload_all`.
    /// Returns whether anything changed.
    pub fn refresh(&mut self) -> anyhow::Result<bool> {
        let mut on_disk = std::collections::BTreeSet::new();
        let mut changed = Vec::new();
        for calendar in &self.calendars {
            for path in calendar.item_paths()? {
                let stale = match self.items.get(path.as_os_str()) {
                    Some(item) => item.modified.is_none() || item.modified != modified(&path),
                    None => true,
                };
                if stale {
                    changed.push(path.clone());
                }
                on_disk.insert(path.into_os_string());
            }
        }
        let gone: Vec<_> = self
            .items
            .keys()
            .filter(|p| !on_disk.contains(*p))
            .cloned()
            .collect();
        let mut any = !gone.is_empty();
        for p in gone {
            self.items.remove(&p);
            self.timezones.remove(&p);
        }
        for path in changed {
            any |= self.reload(&path);
        }
        Ok(any)
    }

    /// Re-read everything: calendars, their metadata and items.
    pub fn reload_all(&mut self) -> anyhow::Result<()> {
        *self = Self::load(&self.root)?;
        Ok(())
    }

    /// Every occurrence of the given kinds that overlaps `range`, for a
    /// viewer in `local`, sorted by start.
    pub fn query(
        &self,
        range: &Range<DateTime<Utc>>,
        kinds: &[Kind],
        local: Tz,
    ) -> Vec<Occurrence> {
        let mut out = Vec::new();
        for (index, calendar) in self.calendars.iter().enumerate() {
            let components = self
                .items
                .values()
                .filter(|i| i.calendar == index)
                .flat_map(|i| &i.components);
            out.extend(
                occurrence::expand(&calendar.id, components, range, local)
                    .into_iter()
                    .filter(|o| kinds.contains(&o.kind)),
            );
        }
        out.sort_by(|a, b| {
            (a.start.to_utc(local), &a.summary).cmp(&(b.start.to_utc(local), &b.summary))
        });
        out
    }
}

fn read_item(path: &Path) -> Option<Vec<AnyComponent>> {
    read_item_without_timezones(path).map(|(c, _)| c)
}

/// The components of an item, skipping its VTIMEZONEs (not parsed at all),
/// and the TZIDs those defined.
fn read_item_without_timezones(path: &Path) -> Option<(Vec<AnyComponent>, Vec<String>)> {
    let parsed = std::fs::read_to_string(path)
        .map_err(anyhow::Error::from)
        .and_then(|text| Ok(crate::compact::parse(&text)?));
    match parsed {
        Ok((components, tzids)) => {
            // one copy of the path for all of a file's components
            let shared: std::sync::Arc<Path> = path.into();
            Some((
                components
                    .into_iter()
                    .map(|c| AnyComponent::new(shared.clone(), c))
                    .collect(),
                tzids,
            ))
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
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !is_hidden(p))
        .collect::<Vec<_>>();
    children.sort();
    for child in children {
        scan(&child, depth + 1, dirs, seen);
    }
}

/// A folder holding `.ics` files or vdir calendar metadata, or an empty
/// folder (a collection vdirsyncer has created but not filled yet).
fn is_calendar_dir(dir: &Path) -> bool {
    let Ok(entries) = read_dir(dir) else {
        return false;
    };
    let entries = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect::<Vec<_>>();
    entries.is_empty()
        || dir.join("displayname").is_file()
        || dir.join("color").is_file()
        || entries
            .iter()
            .any(|p| p.extension().is_some_and(|x| x == "ics") && p.is_file())
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

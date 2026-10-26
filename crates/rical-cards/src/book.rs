//! Address books: a folder of folders of `.vcf` files (as vdirsyncer
//! keeps a CardDAV account), loaded into memory in their compact parsed
//! form and kept up with: [`Books::refresh`] re-reads only files that
//! changed.

use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::Context as _;
use compact_ics::Component;

use crate::model::Card;

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

#[derive(Clone)]
struct Item {
    book: usize,
    modified: Option<SystemTime>,
    cards: Vec<Component>,
}

/// Every address book under a root, and their cards.
#[derive(Clone)]
pub struct Books {
    pub root: PathBuf,
    pub books: Vec<AddressBook>,
    items: BTreeMap<OsString, Item>,
    /// Which cards have each UID, for matching cards from elsewhere.
    by_uid: HashMap<String, Vec<CardId>>,
    /// Counts changes, so views know when to rebuild.
    generation: u64,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read_cards(path: &Path) -> Option<Vec<Component>> {
    let text = std::fs::read_to_string(path).ok()?;
    match compact_ics::parse(&text) {
        Ok((cards, _)) => Some(
            cards
                .into_iter()
                .filter(|c| c.kind.eq_ignore_ascii_case("VCARD"))
                .collect(),
        ),
        Err(e) => {
            log::warn!("skipping {path:?}: {e}");
            None
        }
    }
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
            if p.is_dir() && !hidden {
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

impl Books {
    /// Load every address book under `root` (or `root` itself, if it holds
    /// cards directly).
    pub fn load(root: impl AsRef<Path>) -> anyhow::Result<Self> {
        let root = root.as_ref();
        anyhow::ensure!(root.is_dir(), "not a folder: {root:?}");
        let mut dirs = find_books(root);
        if dirs.is_empty() {
            dirs.push(root.to_owned());
        }
        let books = dirs
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
        let mut out = Self {
            root: root.to_owned(),
            books,
            items: BTreeMap::new(),
            by_uid: HashMap::new(),
            generation: 0,
        };
        for i in 0..out.books.len() {
            for path in out.card_paths(i)? {
                out.read(i, &path);
            }
        }
        Ok(out)
    }

    fn card_paths(&self, book: usize) -> anyhow::Result<Vec<PathBuf>> {
        let dir = &self.books[book].path;
        let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
            .with_context(|| format!("reading {dir:?}"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_file() && is_card_file(p))
            .collect();
        out.sort();
        Ok(out)
    }

    /// Read one file into `book`, replacing what was there. Returns whether
    /// it holds cards now.
    fn read(&mut self, book: usize, path: &Path) -> bool {
        self.forget_uids(path);
        match read_cards(path) {
            Some(cards) => {
                for (index, c) in cards.iter().enumerate() {
                    if let Some(uid) = Card(c).uid() {
                        self.by_uid.entry(uid.to_owned()).or_default().push(CardId {
                            path: path.to_owned(),
                            index,
                        });
                    }
                }
                self.items.insert(
                    path.as_os_str().to_owned(),
                    Item {
                        book,
                        modified: modified(path),
                        cards,
                    },
                );
                true
            }
            None => {
                self.items.remove(path.as_os_str());
                false
            }
        }
    }

    fn forget_uids(&mut self, path: &Path) {
        let Some(item) = self.items.get(path.as_os_str()) else {
            return;
        };
        for c in &item.cards {
            if let Some(uid) = Card(c).uid()
                && let Some(ids) = self.by_uid.get_mut(uid)
            {
                ids.retain(|id| id.path != path);
                if ids.is_empty() {
                    self.by_uid.remove(uid);
                }
            }
        }
    }

    /// The book a path is in, by its folder.
    pub fn book_of(&self, path: &Path) -> Option<usize> {
        self.books
            .iter()
            .position(|b| path.parent() == Some(b.path.as_path()))
    }

    /// Re-read one file after it changed on disk (or drop it if gone).
    /// Returns whether anything changed.
    pub fn reload(&mut self, path: &Path) -> bool {
        let Some(book) = self.book_of(path) else {
            return false;
        };
        if !is_card_file(path) {
            return false;
        }
        let changed = if path.is_file() {
            self.read(book, path);
            true
        } else {
            self.forget_uids(path);
            self.items.remove(path.as_os_str()).is_some()
        };
        if changed {
            self.generation += 1;
        }
        changed
    }

    /// Catch up with the files: re-read those changed since they were read
    /// (by modification time), read new ones and drop removed ones.
    pub fn refresh(&mut self) -> anyhow::Result<bool> {
        let mut seen = std::collections::BTreeSet::new();
        let mut stale = Vec::new();
        for book in 0..self.books.len() {
            for path in self.card_paths(book)? {
                let changed = match self.items.get(path.as_os_str()) {
                    Some(item) => item.modified.is_none() || item.modified != modified(&path),
                    None => true,
                };
                if changed {
                    stale.push(path.clone());
                }
                seen.insert(path.into_os_string());
            }
        }
        let gone: Vec<OsString> = self
            .items
            .keys()
            .filter(|p| !seen.contains(*p))
            .cloned()
            .collect();
        let mut any = !gone.is_empty();
        for p in gone {
            let path = PathBuf::from(&p);
            self.forget_uids(&path);
            self.items.remove(&p);
        }
        for path in stale {
            any |= self.reload(&path);
        }
        if any {
            self.generation += 1;
        }
        Ok(any)
    }

    /// Changes so far: compare to know whether to rebuild.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Every card, with its id and book.
    pub fn cards(&self) -> impl Iterator<Item = (CardId, &AddressBook, Card<'_>)> {
        self.items.iter().flat_map(|(path, item)| {
            let book = &self.books[item.book];
            item.cards.iter().enumerate().map(move |(index, c)| {
                (
                    CardId {
                        path: PathBuf::from(path),
                        index,
                    },
                    book,
                    Card(c),
                )
            })
        })
    }

    /// How many cards there are.
    pub fn len(&self) -> usize {
        self.items.values().map(|i| i.cards.len()).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, id: &CardId) -> Option<Card<'_>> {
        self.items
            .get(id.path.as_os_str())
            .and_then(|i| i.cards.get(id.index))
            .map(Card)
    }

    /// The book a card is in.
    pub fn book(&self, id: &CardId) -> Option<&AddressBook> {
        self.items
            .get(id.path.as_os_str())
            .map(|i| &self.books[i.book])
    }

    /// The cards with this UID (in any book).
    pub fn by_uid(&self, uid: &str) -> &[CardId] {
        self.by_uid.get(uid).map_or(&[], Vec::as_slice)
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
    /// Whether a file changed on disk since it was read here (or is gone),
    /// so an edit based on what is shown would undo someone else's.
    pub fn is_stale(&self, path: &Path) -> bool {
        match self.items.get(path.as_os_str()) {
            Some(item) => item.modified.is_none() || item.modified != modified(path),
            None => true,
        }
    }
}

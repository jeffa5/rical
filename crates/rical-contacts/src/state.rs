//! What the app knows: the address books and their search index, what is
//! shown and selected, and the history to undo.

use std::{cell::RefCell, collections::BTreeSet, rc::Rc};

use rical_cards::{
    Books, CardId,
    dedupe::{self, Group},
    search::Index,
};
use rical_core::write::Change;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    People,
    Duplicates,
}

impl View {
    pub fn section(self) -> &'static str {
        match self {
            View::People => "people",
            View::Duplicates => "duplicates",
        }
    }
}

pub struct State {
    pub books: Books,
    /// Whether `books` holds the cards yet: the window opens with only the
    /// address books and reads the cards in the background.
    pub loaded: bool,
    /// The cards being read (and indexed) since the app started, while the
    /// window is built: [`crate::window::Window::load`] takes them.
    pub preload: Option<(Preload, rical_ui::util::EarlyWatch)>,
    pub index: Index,
    pub view: View,
    pub query: String,
    /// What the list shows: entries of the index, best first.
    pub shown: Vec<usize>,
    pub selected: Option<CardId>,
    /// Duplicate groups, worked out when first wanted after a change.
    groups: Option<(u64, Rc<Vec<Group>>)>,
    /// Something changed that the groups depend on (beyond the cards).
    groups_stale: bool,
    pub selected_group: Option<usize>,
    /// Pairs of cards (by UID, sorted) said not to be the same person.
    pub not_same: BTreeSet<(String, String)>,
    /// Address book new contacts go in, by id or name.
    pub default_book: Option<String>,
    /// How the list of everyone is sorted.
    pub sort: rical_cards::search::SortBy,
    /// What was changed, to undo (shared with the other apps, and kept).
    pub history: Option<rical_core::history::History>,
}

pub type SharedState = Rc<RefCell<State>>;

/// A read of the address books and their index, running.
pub type Preload = std::thread::JoinHandle<anyhow::Result<(Books, Index)>>;

/// Who did it, in the history the apps share.
pub const APP: &str = "contacts";

impl State {
    pub fn new(books: Books) -> Self {
        let index = Index::build(&books);
        let mut s = Self {
            books,
            loaded: true,
            preload: None,
            index,
            view: View::People,
            query: String::new(),
            shown: Vec::new(),
            selected: None,
            groups: None,
            groups_stale: false,
            selected_group: None,
            not_same: load_not_same(),
            default_book: None,
            sort: Default::default(),
            history: None,
        };
        s.search();
        s
    }

    /// Take what was read in the background: the books and their index.
    pub fn set_loaded(&mut self, mut books: Books, mut index: Index) {
        books.follow(self.books.generation());
        index.generation = books.generation();
        self.books = books;
        self.index = index;
        self.index.sort_by(self.sort);
        self.groups = None;
        self.loaded = true;
        self.search();
    }

    /// Rebuild the index if the books changed, and search again.
    pub fn refresh(&mut self) {
        if self.index.generation != self.books.generation() {
            self.index = Index::build(&self.books);
            self.index.sort_by(self.sort);
        }
        self.search();
    }

    /// Sort everyone by `by` from now on.
    pub fn set_sort(&mut self, by: rical_cards::search::SortBy) {
        self.sort = by;
        self.index.sort_by(by);
        self.search();
    }

    pub fn search(&mut self) {
        self.shown = self.index.search(&self.query);
    }

    /// The duplicate groups to show, as last worked out (see
    /// [`State::groups_job`]): none before the first time.
    pub fn groups(&mut self) -> Rc<Vec<Group>> {
        self.cached_groups()
    }

    /// The duplicate groups as last worked out (none if not yet).
    pub fn cached_groups(&self) -> Rc<Vec<Group>> {
        self.groups
            .as_ref()
            .map(|(_, g)| g.clone())
            .unwrap_or_default()
    }

    /// Whether the groups need working out again.
    pub fn groups_stale(&self) -> bool {
        self.groups_stale
            || self
                .groups
                .as_ref()
                .is_none_or(|(g, _)| *g != self.books.generation())
    }

    /// Working out the groups, for the background pool, and the generation
    /// of the books it is for (to give back to [`State::set_groups`]).
    pub fn groups_job(&mut self) -> (u64, impl FnOnce() -> Vec<Group> + Send + 'static) {
        self.groups_stale = false;
        let root = self.books.root.clone();
        let not_same = self.not_same.clone();
        (self.books.generation(), move || {
            find_groups(&root, &not_same)
        })
    }

    /// Take groups worked out for `generation` of the books, if it is
    /// still the current one.
    pub fn set_groups(&mut self, generation: u64, groups: Vec<Group>) -> bool {
        if generation != self.books.generation() {
            return false;
        }
        self.groups = Some((generation, Rc::new(groups)));
        true
    }

    /// Work the duplicate groups out again (keeping those there are until
    /// then).
    pub fn invalidate_groups(&mut self) {
        self.groups_stale = true;
    }

    fn uid_of(&self, id: &CardId) -> Option<String> {
        self.books.get(id)?.card().uid().map(str::to_owned)
    }

    /// Remember that the cards of a group are different people.
    pub fn say_different(&mut self, g: &Group) {
        let uids: Vec<String> = g.cards.iter().filter_map(|id| self.uid_of(id)).collect();
        for (i, a) in uids.iter().enumerate() {
            for b in &uids[i + 1..] {
                self.not_same.insert(pair(a, b));
            }
        }
        save_not_same(&self.not_same);
        self.invalidate_groups();
    }

    /// Record changes made by one action (`what` it was doing: "delete the
    /// contact") so it can be undone, now or later.
    pub fn record(&mut self, what: &str, changes: Vec<Change>) {
        if let Some(h) = self.history.as_mut() {
            let what = rical_core::history::describe(what, &changes);
            if let Err(e) = h.record(APP, &what, &changes) {
                log::warn!("could not keep “{what}” in the history: {e:#}");
            }
        }
        self.invalidate_groups();
    }

    /// Undo (or redo) history entry `id`, if its files are as it left them.
    pub fn undo_entry(&mut self, id: &str, undo: bool) -> anyhow::Result<()> {
        let Self { history, books, .. } = self;
        let history = history
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no history is kept"))?;
        let result = history.undo_with(id, undo, |c| books.apply(c));
        self.invalidate_groups();
        result
    }

    /// The book new contacts go in: as set, else the one with most cards.
    pub fn default_book(&self) -> Option<usize> {
        if let Some(name) = &self.default_book
            && let Some(i) = self.books.find_book(name)
        {
            return Some(i);
        }
        let mut counts = vec![0usize; self.books.books.len()];
        for e in &self.index.entries {
            if let Some(c) = counts.get_mut(e.book) {
                *c += 1;
            }
        }
        (0..counts.len()).max_by_key(|&i| (counts[i], std::cmp::Reverse(i)))
    }
}

/// The duplicate groups among the address books under `root`, leaving out
/// those said to be different people: from the index, on any thread.
pub fn find_groups(root: &std::path::Path, not_same: &BTreeSet<(String, String)>) -> Vec<Group> {
    let books = match Books::open_index(root) {
        Ok(b) => b,
        Err(e) => {
            log::warn!("could not look for duplicates: {e:#}");
            return Vec::new();
        }
    };
    let uids: std::collections::HashMap<CardId, String> = books
        .listed()
        .into_iter()
        .filter_map(|l| Some((l.id, l.uid?)))
        .collect();
    // whether every pair in the group was said not to be the same person
    let all_said_different = |g: &Group| {
        let ids: Vec<&String> = g.cards.iter().filter_map(|id| uids.get(id)).collect();
        ids.len() == g.cards.len()
            && ids
                .iter()
                .enumerate()
                .all(|(i, a)| ids[i + 1..].iter().all(|b| not_same.contains(&pair(a, b))))
    };
    dedupe::find(&books)
        .into_iter()
        .filter(|g| !all_said_different(g))
        .collect()
}

fn pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

/// Pairs said to be different people, kept with the window's state as
/// `not-same = uid1 uid2; uid3 uid4`.
fn load_not_same() -> BTreeSet<(String, String)> {
    rical_ui::uistate::get("not-same")
        .map(|v| {
            v.split(';')
                .filter_map(|p| {
                    let mut it = p.split_whitespace();
                    Some(pair(it.next()?, it.next()?))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn save_not_same(pairs: &BTreeSet<(String, String)>) {
    let v = pairs
        .iter()
        .map(|(a, b)| format!("{a} {b}"))
        .collect::<Vec<_>>()
        .join("; ");
    rical_ui::uistate::set("not-same", &v);
}

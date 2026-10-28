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
    pub index: Index,
    pub view: View,
    pub query: String,
    /// What the list shows: entries of the index, best first.
    pub shown: Vec<usize>,
    pub selected: Option<CardId>,
    /// Duplicate groups, worked out when first wanted after a change.
    groups: Option<(u64, Rc<Vec<Group>>)>,
    pub selected_group: Option<usize>,
    /// Pairs of cards (by UID, sorted) said not to be the same person.
    pub not_same: BTreeSet<(String, String)>,
    /// Address book new contacts go in, by id or name.
    pub default_book: Option<String>,
    /// How the list of everyone is sorted.
    pub sort: rical_cards::search::SortBy,
    pub undo: Vec<Vec<Change>>,
    pub redo: Vec<Vec<Change>>,
}

pub type SharedState = Rc<RefCell<State>>;

impl State {
    pub fn new(books: Books) -> Self {
        let index = Index::build(&books);
        let mut s = Self {
            books,
            index,
            view: View::People,
            query: String::new(),
            shown: Vec::new(),
            selected: None,
            groups: None,
            selected_group: None,
            not_same: load_not_same(),
            default_book: None,
            sort: Default::default(),
            undo: Vec::new(),
            redo: Vec::new(),
        };
        s.search();
        s
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

    /// The duplicate groups to show, leaving out those said not to be.
    pub fn groups(&mut self) -> Rc<Vec<Group>> {
        let generation = self.books.generation();
        match &self.groups {
            Some((g, groups)) if *g == generation => groups.clone(),
            _ => {
                let found: Vec<Group> = dedupe::find(&self.books)
                    .into_iter()
                    .filter(|g| !self.all_said_different(g))
                    .collect();
                let found = Rc::new(found);
                self.groups = Some((generation, found.clone()));
                found
            }
        }
    }

    /// The duplicate groups as last worked out (none if not yet).
    pub fn cached_groups(&self) -> Rc<Vec<Group>> {
        self.groups
            .as_ref()
            .map(|(_, g)| g.clone())
            .unwrap_or_default()
    }

    /// Forget the duplicate groups, to work them out again.
    pub fn invalidate_groups(&mut self) {
        self.groups = None;
    }

    fn uid_of(&self, id: &CardId) -> Option<String> {
        self.books.get(id)?.uid().map(str::to_owned)
    }

    /// Whether every pair in the group was said not to be the same person.
    fn all_said_different(&self, g: &Group) -> bool {
        let uids: Vec<String> = g.cards.iter().filter_map(|id| self.uid_of(id)).collect();
        if uids.len() < g.cards.len() {
            return false;
        }
        uids.iter().enumerate().all(|(i, a)| {
            uids[i + 1..]
                .iter()
                .all(|b| self.not_same.contains(&pair(a, b)))
        })
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

    /// Record changes made, for undo.
    pub fn record(&mut self, changes: Vec<Change>) {
        if !changes.is_empty() {
            self.undo.push(changes);
            self.redo.clear();
        }
        self.invalidate_groups();
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

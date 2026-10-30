//! What the app knows: the folders and the messages in the one shown, how
//! they are arranged and filtered, which is selected, and what can be undone.

use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc};

use rical_cards::Books;
use rical_maildir::{
    addr::Mailbox,
    backend::Backend,
    flags::Flags,
    read::Opened,
    search::Query,
    store::{self, Folder, Role, Summary},
    thread::{self, Row},
};

use crate::config::Options;

/// Where the list is looking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// A folder, by its path.
    Folder(PathBuf),
    /// What rical-mail-queue has queued to send.
    Outbox,
    /// The messages a search found, by its query.
    Search(String),
}

/// Someone to complete an address with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub address: String,
    /// In the address book, not just written to or heard from.
    pub contact: bool,
}

impl Person {
    pub fn mailbox(&self) -> Mailbox {
        Mailbox::new(&self.name, &self.address)
    }
}

/// A change that can be undone.
#[derive(Debug, Clone)]
pub enum Undo {
    Moved {
        now: PathBuf,
        back_to: PathBuf,
    },
    Flags {
        now: PathBuf,
        before: Flags,
    },
    /// Several at once, undone together.
    Batch(Vec<Undo>),
}

pub struct State {
    pub opts: Options,
    pub root: PathBuf,
    pub folders: Vec<Folder>,
    pub place: Place,
    /// Every message in the place, newest first.
    pub messages: Vec<Summary>,
    /// The messages as shown: conversations or not.
    pub rows: Vec<Row>,
    /// What the search leaves of `rows`: positions in it.
    pub shown: Vec<usize>,
    pub query: String,
    pub threaded: bool,
    pub selected: Option<PathBuf>,
    /// The message in the reader.
    pub opened: Option<Rc<Opened>>,
    /// What to say under each message of the Outbox.
    pub outbox_notes: HashMap<PathBuf, String>,
    pub people: Vec<Person>,
    pub books: Option<Books>,
    /// The calendars, to compare an invitation with what is there.
    pub vdir: Option<rical_core::Vdir>,
    pub undo: Vec<Undo>,
    /// Messages picked to act on together.
    pub picked: std::collections::BTreeSet<PathBuf>,
    /// The message last sent, while it can still be taken back.
    pub last_sent: Option<(String, rical_maildir::compose::Draft)>,
    /// Where the list was before a search, to go back to.
    pub before_search: Option<Place>,
}

pub type SharedState = Rc<RefCell<State>>;

impl State {
    pub fn new(opts: Options, root: PathBuf, books: Option<Books>) -> State {
        let threaded = opts.threads;
        let mut s = State {
            opts,
            root,
            folders: Vec::new(),
            place: Place::Outbox,
            messages: Vec::new(),
            rows: Vec::new(),
            shown: Vec::new(),
            query: String::new(),
            threaded,
            selected: None,
            opened: None,
            outbox_notes: HashMap::new(),
            people: Vec::new(),
            books,
            vdir: None,
            undo: Vec::new(),
            picked: Default::default(),
            last_sent: None,
            before_search: None,
        };
        s.reload_folders();
        s.build_people();
        let first = s
            .folders
            .iter()
            .find(|f| f.role == Role::Inbox)
            .or(s.folders.first())
            .map(|f| f.path.clone());
        s.place = first.map_or(Place::Outbox, Place::Folder);
        s.reload_messages();
        s
    }

    pub fn reload_folders(&mut self) {
        match store::folders(&self.root) {
            Ok(f) => self.folders = f,
            Err(e) => log::warn!("{}: {e:#}", self.root.display()),
        }
    }

    pub fn folder(&self) -> Option<&Folder> {
        match &self.place {
            Place::Folder(p) => self.folders.iter().find(|f| &f.path == p),
            Place::Outbox | Place::Search(_) => None,
        }
    }

    pub fn place_name(&self) -> String {
        match &self.place {
            Place::Outbox => "Outbox".to_owned(),
            Place::Search(q) => format!("Search: {q}"),
            Place::Folder(_) => self.folder().map_or("Mail".to_owned(), |f| f.name.clone()),
        }
    }

    pub fn folder_with_role(&self, role: Role) -> Option<&Folder> {
        self.folders.iter().find(|f| f.role == role)
    }

    /// A folder by what is typed: its name, case ignored, or the end of it.
    pub fn find_folder(&self, name: &str) -> Option<&Folder> {
        let n = name.trim().to_lowercase();
        self.folders
            .iter()
            .find(|f| f.name.to_lowercase() == n)
            .or_else(|| {
                self.folders
                    .iter()
                    .find(|f| f.name.to_lowercase().ends_with(&format!("/{n}")))
            })
            .or_else(|| {
                self.folders
                    .iter()
                    .find(|f| f.name.to_lowercase().starts_with(&n))
            })
    }

    /// The folder drafts are kept in, as set or by its name.
    pub fn drafts_folder(&self) -> PathBuf {
        self.opts
            .drafts
            .as_deref()
            .and_then(|d| self.find_folder(d))
            .or_else(|| self.folder_with_role(Role::Drafts))
            .map_or_else(|| self.root.join("Drafts"), |f| f.path.clone())
    }

    pub fn archive_folder(&self) -> PathBuf {
        self.opts
            .archive
            .as_deref()
            .and_then(|d| self.find_folder(d))
            .or_else(|| self.folder_with_role(Role::Archive))
            .map_or_else(|| self.root.join("Archive"), |f| f.path.clone())
    }

    pub fn trash_folder(&self) -> PathBuf {
        self.folder_with_role(Role::Trash)
            .map_or_else(|| self.root.join("Trash"), |f| f.path.clone())
    }

    pub fn junk_folder(&self) -> PathBuf {
        self.folder_with_role(Role::Junk)
            .map_or_else(|| self.root.join("Junk"), |f| f.path.clone())
    }

    pub fn in_drafts(&self) -> bool {
        self.folder().is_some_and(|f| f.role == Role::Drafts)
    }

    pub fn in_trash(&self) -> bool {
        self.folder().is_some_and(|f| f.role == Role::Trash)
    }

    /// Read the messages of the place again.
    pub fn reload_messages(&mut self) {
        match self.place.clone() {
            Place::Folder(p) => {
                self.messages = self
                    .folders
                    .iter()
                    .find(|f| f.path == p)
                    .map(|f| store::list_cached(f, cache_for(f)))
                    .unwrap_or_default();
                self.outbox_notes.clear();
            }
            Place::Outbox => self.load_outbox(),
            Place::Search(q) => self.load_search(&q),
        }
        self.arrange();
    }

    fn load_outbox(&mut self) {
        self.messages.clear();
        self.outbox_notes.clear();
        let Ok(queue) = rical_mail_queue::queue::Queue::open(&self.opts.queue) else {
            return;
        };
        for e in queue.list().unwrap_or_default() {
            let mut m = store::summarize(&e.path, "Outbox", false);
            if let Some(m) = &mut m {
                m.flags.seen = true;
                let note = match e.state {
                    rical_mail_queue::queue::State::Queued => match e.next_attempt {
                        Some(t) if t > chrono::Utc::now() => {
                            format!(
                                "held back until {} (Undo, or delete it here)",
                                t.with_timezone(&chrono::Local).format("%H:%M:%S")
                            )
                        }
                        _ => "waiting to be sent".to_owned(),
                    },
                    rical_mail_queue::queue::State::Sending => "sending now".to_owned(),
                    rical_mail_queue::queue::State::Retrying => format!(
                        "will try again{}",
                        e.last_error
                            .as_deref()
                            .map(|l| format!(" · {l}"))
                            .unwrap_or_default()
                    ),
                    rical_mail_queue::queue::State::Failed => format!(
                        "failed{}",
                        e.last_error
                            .as_deref()
                            .map(|l| format!(" · {l}"))
                            .unwrap_or_default()
                    ),
                };
                self.outbox_notes.insert(m.path.clone(), note);
            }
            self.messages.extend(m);
        }
        self.messages.sort_by_key(|m| std::cmp::Reverse(m.date));
    }

    /// How many messages wait in the send queue.
    pub fn outbox_count(&self) -> usize {
        rical_mail_queue::queue::Queue::open(&self.opts.queue)
            .and_then(|q| q.list())
            .map_or(0, |l| l.len())
    }

    /// Lay the messages out and apply the search.
    pub fn arrange(&mut self) {
        self.rows = if self.threaded && !matches!(self.place, Place::Outbox | Place::Search(_)) {
            thread::threads(&self.messages)
        } else {
            (0..self.messages.len())
                .map(|index| Row {
                    index,
                    depth: 0,
                    head: true,
                })
                .collect()
        };
        self.apply_query();
    }

    pub fn apply_query(&mut self) {
        let q = Query::parse(&self.query);
        self.shown = if q.is_empty() {
            (0..self.rows.len()).collect()
        } else {
            (0..self.rows.len())
                .filter(|&i| {
                    let m = &self.messages[self.rows[i].index];
                    q.matches(m) && q.matches_body(m)
                })
                .collect()
        };
        // keep the selection if it is still shown, else the first
        let keep = self.selected.as_ref().is_some_and(|p| {
            self.shown
                .iter()
                .any(|&i| &self.messages[self.rows[i].index].path == p)
        });
        if !keep {
            self.selected = self
                .shown
                .first()
                .map(|&i| self.messages[self.rows[i].index].path.clone());
            self.opened = None;
        }
    }

    pub fn message_at(&self, shown: usize) -> Option<&Summary> {
        let i = *self.shown.get(shown)?;
        self.messages.get(self.rows.get(i)?.index)
    }

    pub fn depth_at(&self, shown: usize) -> usize {
        self.shown
            .get(shown)
            .and_then(|&i| self.rows.get(i))
            .map_or(0, |r| r.depth)
    }

    /// Where the selected message is in the list.
    pub fn position(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        (0..self.shown.len()).find(|&n| self.message_at(n).is_some_and(|m| &m.path == sel))
    }

    pub fn selected_summary(&self) -> Option<&Summary> {
        self.message_at(self.position()?)
    }

    /// Everyone to complete an address with: the address books, then
    /// the people the loaded mail is from.
    pub fn build_people(&mut self) {
        let mut people: Vec<Person> = Vec::new();
        if let Some(books) = &self.books {
            for (_, _, card) in books.cards() {
                let name = card.display_name();
                for e in card.emails() {
                    people.push(Person {
                        name: if name.contains('@') {
                            String::new()
                        } else {
                            name.clone()
                        },
                        address: e.value.trim().to_owned(),
                        contact: true,
                    });
                }
            }
        }
        people.sort_by_key(|p| p.name.to_lowercase());
        self.people = people;
    }

    /// People the mail in view is from or to, who are not in the books.
    pub fn people_from_mail(&self) -> Vec<Person> {
        let mut out: Vec<Person> = Vec::new();
        for m in &self.messages {
            if m.from_address.is_empty()
                || out
                    .iter()
                    .any(|p| p.address.eq_ignore_ascii_case(&m.from_address))
                || self
                    .people
                    .iter()
                    .any(|p| p.address.eq_ignore_ascii_case(&m.from_address))
            {
                continue;
            }
            out.push(Person {
                name: m.from_name.clone(),
                address: m.from_address.clone(),
                contact: false,
            });
        }
        out
    }

    /// Whether an address is in the address books.
    pub fn is_contact(&self, address: &str) -> bool {
        self.people
            .iter()
            .any(|p| p.contact && p.address.eq_ignore_ascii_case(address))
    }

    /// The best matches for what is typed so far.
    pub fn complete_address(&self, typed: &str, limit: usize) -> Vec<Person> {
        let t = typed.trim().to_lowercase();
        if t.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u8, usize, Person)> = Vec::new();
        let heard = self.people_from_mail();
        for (i, p) in self.people.iter().chain(&heard).enumerate() {
            let name = p.name.to_lowercase();
            let addr = p.address.to_lowercase();
            let rank = if addr.starts_with(&t) || name.starts_with(&t) {
                0
            } else if name.split_whitespace().any(|w| w.starts_with(&t))
                || addr.split(['@', '.', '-', '_']).any(|w| w.starts_with(&t))
            {
                1
            } else if name.contains(&t) || addr.contains(&t) {
                2
            } else {
                continue;
            };
            scored.push((rank * 2 + u8::from(!p.contact), i, p.clone()));
        }
        scored.sort_by_key(|(r, i, _)| (*r, *i));
        let mut out: Vec<Person> = Vec::new();
        for (_, _, p) in scored {
            if out
                .iter()
                .any(|o| o.address.eq_ignore_ascii_case(&p.address))
            {
                continue;
            }
            out.push(p);
            if out.len() >= limit {
                break;
            }
        }
        out
    }

    pub fn push_undo(&mut self, u: Undo) {
        self.undo.push(u);
        if self.undo.len() > 50 {
            self.undo.remove(0);
        }
    }
}

impl State {
    /// A message was renamed (its flags changed): follow it.
    pub fn renamed(&mut self, old: &std::path::Path, new: PathBuf, flags: Flags) {
        for m in &mut self.messages {
            if m.path == old {
                m.path = new.clone();
                m.flags = flags;
                m.is_new = false;
            }
        }
        if self.picked.remove(old) {
            self.picked.insert(new.clone());
        }
        if self.selected.as_deref() == Some(old) {
            self.selected = Some(new);
        }
    }

    /// A message left the folder. Returns where it was in the list, and
    /// selects what is now there.
    pub fn removed(&mut self, path: &std::path::Path) -> Option<usize> {
        self.picked.remove(path);
        let pos =
            (0..self.shown.len()).find(|&n| self.message_at(n).is_some_and(|m| m.path == path));
        self.messages.retain(|m| m.path != path);
        let was_selected = self.selected.as_deref() == Some(path);
        self.arrange();
        if was_selected {
            let n = self.shown.len();
            self.selected = pos
                .filter(|_| n > 0)
                .map(|p| p.min(n - 1))
                .and_then(|p| self.message_at(p).map(|m| m.path.clone()));
            self.opened = None;
        }
        pos
    }
}

impl State {
    /// What the invitation in a message changes in the calendars (if a
    /// calendar folder is set and the message has one).
    pub fn invite_diffs(&mut self, path: &std::path::Path) -> Vec<rical_core::diff::Diff> {
        let Some(vdir) = &mut self.vdir else {
            return Vec::new();
        };
        let Ok(ics) = rical_maildir::read::calendar_text(path) else {
            return Vec::new();
        };
        if let Err(e) = vdir.refresh() {
            log::warn!("calendar: {e:#}");
        }
        vdir.diff_invite(&ics).unwrap_or_else(|e| {
            log::warn!("invitation: {e:#}");
            Vec::new()
        })
    }
}

/// The header cache of a folder, kept with the app's state.
fn cache_for(f: &Folder) -> rical_maildir::cache::Cache {
    match rical_ui::app::state_dir() {
        Some(d) => rical_maildir::cache::Cache::open(&d.join("cache"), &f.path),
        None => rical_maildir::cache::Cache::none(),
    }
}

impl State {
    /// What a command acts on: the picked messages, else the selected one.
    pub fn targets(&self) -> Vec<PathBuf> {
        let picked: Vec<PathBuf> = self
            .messages
            .iter()
            .filter(|m| self.picked.contains(&m.path))
            .map(|m| m.path.clone())
            .collect();
        if picked.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            picked
        }
    }
}

impl State {
    /// Where a search tool leaves its results (symlinks, as mu makes them).
    pub fn search_dir() -> PathBuf {
        rical_ui::app::state_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("search")
            .join("results")
    }

    /// The results of `query`: what the tool left in the results folder, or
    /// for the built-in search every folder's messages that match.
    fn load_search(&mut self, query: &str) {
        self.outbox_notes.clear();
        self.messages = if self.opts.search == Backend::Builtin {
            let q = rical_maildir::search::Query::parse(query);
            let mut all: Vec<_> = Vec::new();
            for f in &self.folders {
                // junk and trash stay out of a search across everything, unless asked
                if matches!(f.role, Role::Junk | Role::Trash) && q.folder.is_empty() {
                    continue;
                }
                all.extend(
                    store::list_cached(f, cache_for(f))
                        .into_iter()
                        .filter(|m| q.matches(m)),
                );
            }
            all.retain(|m| q.matches_body(m));
            all.sort_by_key(|m| std::cmp::Reverse(m.date));
            all
        } else {
            store::list_links(&Self::search_dir())
        };
        for m in &self.messages {
            self.outbox_notes
                .insert(m.path.clone(), format!("in {}", m.folder));
        }
        self.messages.truncate(5000);
    }
}

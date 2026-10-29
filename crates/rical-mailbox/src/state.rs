//! What the app knows: the folders and the messages in the one shown, how
//! they are arranged and filtered, which is selected, and what can be undone.

use std::{cell::RefCell, collections::HashMap, path::PathBuf, rc::Rc};

use rical_cards::Books;
use rical_maildir::{
    addr::Mailbox,
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
    /// What rical-mail has queued to send.
    Outbox,
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
    Moved { now: PathBuf, back_to: PathBuf },
    Flags { now: PathBuf, before: Flags },
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
    pub undo: Vec<Undo>,
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
            undo: Vec::new(),
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
            Place::Outbox => None,
        }
    }

    pub fn place_name(&self) -> String {
        match &self.place {
            Place::Outbox => "Outbox".to_owned(),
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
                    .map(store::list)
                    .unwrap_or_default();
                self.outbox_notes.clear();
            }
            Place::Outbox => self.load_outbox(),
        }
        self.arrange();
    }

    fn load_outbox(&mut self) {
        self.messages.clear();
        self.outbox_notes.clear();
        let Ok(queue) = rical_mail::queue::Queue::open(&self.opts.queue) else {
            return;
        };
        for e in queue.list().unwrap_or_default() {
            let mut m = store::summarize(&e.path, "Outbox", false);
            if let Some(m) = &mut m {
                m.flags.seen = true;
                let note = match e.state {
                    rical_mail::queue::State::Queued => "waiting to be sent".to_owned(),
                    rical_mail::queue::State::Sending => "sending now".to_owned(),
                    rical_mail::queue::State::Retrying => format!(
                        "will try again{}",
                        e.last_error
                            .as_deref()
                            .map(|l| format!(" · {l}"))
                            .unwrap_or_default()
                    ),
                    rical_mail::queue::State::Failed => format!(
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
        rical_mail::queue::Queue::open(&self.opts.queue)
            .and_then(|q| q.list())
            .map_or(0, |l| l.len())
    }

    /// Lay the messages out and apply the search.
    pub fn arrange(&mut self) {
        self.rows = if self.threaded && self.place != Place::Outbox {
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
        if self.selected.as_deref() == Some(old) {
            self.selected = Some(new);
        }
    }

    /// A message left the folder. Returns where it was in the list, and
    /// selects what is now there.
    pub fn removed(&mut self, path: &std::path::Path) -> Option<usize> {
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

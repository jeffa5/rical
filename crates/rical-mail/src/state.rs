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
    /// What rical-mail-queue has queued to send.
    Outbox,
    /// Every folder of one kind across the accounts (all inboxes…).
    Unified(Role),
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
    /// What the list is ordered by, and whether the other way round.
    pub sort: thread::SortKey,
    pub sort_reverse: bool,
    /// Conversations folded to their first message, by that message.
    pub folded: std::collections::HashSet<PathBuf>,
    /// For the first message of each conversation: how many it has, how many unread.
    pub thread_info: HashMap<PathBuf, (usize, usize)>,
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
    /// Counts the readings of a place, so a slow one that finishes late is dropped.
    pub generation: u64,
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
        // how it was left last time, else the setting
        let (mut sort, mut sort_reverse) = (opts.sort, opts.sort_reverse);
        if let Some(saved) = rical_ui::uistate::get("sort") {
            let mut w = saved.split_whitespace();
            if let Some(k) = w.next().and_then(thread::SortKey::parse) {
                sort = k;
                sort_reverse = w.next() == Some("reverse");
            }
        }
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
            sort,
            sort_reverse,
            folded: Default::default(),
            thread_info: HashMap::new(),
            selected: None,
            opened: None,
            outbox_notes: HashMap::new(),
            people: Vec::new(),
            books,
            vdir: None,
            undo: Vec::new(),
            generation: 0,
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
        // the messages are read by the window, away from it
        s.arrange();
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
            Place::Outbox | Place::Search(_) | Place::Unified(_) => None,
        }
    }

    pub fn place_name(&self) -> String {
        match &self.place {
            Place::Outbox => "Outbox".to_owned(),
            Place::Search(q) => format!("Search: {q}"),
            Place::Unified(r) => unified_title(*r).to_owned(),
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

    pub fn in_drafts(&self) -> bool {
        self.folder().is_some_and(|f| f.role == Role::Drafts)
            || self.place == Place::Unified(Role::Drafts)
    }

    pub fn in_trash(&self) -> bool {
        self.folder().is_some_and(|f| f.role == Role::Trash)
            || self.place == Place::Unified(Role::Trash)
    }

    /// Read the messages of the place again.
    #[cfg(test)]
    pub fn reload_messages(&mut self) {
        let job = self.load_job();
        self.apply_loaded(job.run());
    }

    /// How many messages wait in the send queue.
    pub fn outbox_count(&self) -> usize {
        rical_mail_queue::queue::Queue::open(&self.opts.queue)
            .and_then(|q| q.list())
            .map_or(0, |l| l.len())
    }

    /// Lay the messages out and apply the search.
    pub fn arrange(&mut self) {
        let threaded = self.threaded && !matches!(self.place, Place::Outbox | Place::Search(_));
        self.rows = if threaded {
            thread::threads_sorted(&self.messages, self.sort, self.sort_reverse)
        } else {
            thread::sorted(&self.messages, self.sort, self.sort_reverse)
        };
        // how big each conversation is, and how much of it is unread
        self.thread_info.clear();
        let mut at = 0;
        while at < self.rows.len() {
            let mut end = at + 1;
            while end < self.rows.len() && !self.rows[end].head {
                end += 1;
            }
            if end - at > 1 {
                let unread = self.rows[at..end]
                    .iter()
                    .filter(|r| self.messages[r.index].unread())
                    .count();
                self.thread_info.insert(
                    self.messages[self.rows[at].index].path.clone(),
                    (end - at, unread),
                );
            }
            at = end;
        }
        self.apply_query();
    }

    pub fn apply_query(&mut self) {
        let q = Query::parse(&self.query);
        self.shown = if q.is_empty() {
            // a folded conversation shows its first message only
            let mut head: Option<&PathBuf> = None;
            (0..self.rows.len())
                .filter(|&i| {
                    let r = self.rows[i];
                    if r.head {
                        head = Some(&self.messages[r.index].path);
                        true
                    } else {
                        !head.is_some_and(|h| self.folded.contains(h))
                    }
                })
                .collect()
        } else {
            (0..self.rows.len())
                .filter(|&i| {
                    let m = &self.messages[self.rows[i].index];
                    q.matches(m)
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

impl State {
    /// What a command acts on: the picked messages, else the selected one.
    pub fn targets(&self) -> Vec<PathBuf> {
        let picked: Vec<PathBuf> = self
            .messages
            .iter()
            .filter(|m| self.picked.contains(&m.path))
            .map(|m| m.path.clone())
            .collect();
        if !picked.is_empty() {
            return picked;
        }
        // a folded conversation is one thing: all of it
        match &self.selected {
            Some(p) if self.folded.contains(p) => self.thread_of(p),
            Some(p) => vec![p.clone()],
            None => Vec::new(),
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
}

/// What a unified list is called.
pub fn unified_title(role: Role) -> &'static str {
    match role {
        Role::Inbox => "All inboxes",
        Role::Drafts => "All drafts",
        Role::Sent => "All sent",
        Role::Archive => "All archives",
        Role::Junk => "All junk",
        Role::Trash => "All trash",
        Role::Other => "Everything",
    }
}

/// The folder a role has by default, in a new account.
fn default_name(role: Role) -> &'static str {
    match role {
        Role::Inbox => "INBOX",
        Role::Drafts => "Drafts",
        Role::Sent => "Sent",
        Role::Archive => "Archive",
        Role::Junk => "Junk",
        Role::Trash => "Trash",
        Role::Other => "Other",
    }
}

/// One account: a top-level folder holding the folders of a mailbox.
#[derive(Debug, Clone)]
pub struct Account {
    pub name: String,
    /// Indexes into the state's folders.
    pub folders: Vec<usize>,
}

impl State {
    /// Whether the folders are in accounts (`work/INBOX`, `home/INBOX`…):
    /// at least two top-level names that are not folders themselves.
    pub fn grouped(&self) -> bool {
        self.accounts_raw().len() >= 2
    }

    fn accounts_raw(&self) -> std::collections::BTreeSet<String> {
        self.folders
            .iter()
            .filter_map(|f| f.name.split_once('/').map(|(a, _)| a.to_owned()))
            .filter(|a| !self.folders.iter().any(|f| &f.name == a))
            .collect()
    }

    /// The account a folder name belongs to, when there are accounts.
    pub fn account_of(&self, name: &str) -> Option<String> {
        if !self.grouped() {
            return None;
        }
        let a = name.split_once('/')?.0;
        self.accounts_raw().contains(a).then(|| a.to_owned())
    }

    /// The accounts, with their folders in a useful order: inbox, drafts,
    /// sent, archive, junk, trash, then the rest by name.
    pub fn accounts(&self) -> Vec<Account> {
        let names = self.accounts_raw();
        names
            .into_iter()
            .map(|name| {
                let mut folders: Vec<usize> = (0..self.folders.len())
                    .filter(|&i| self.account_of(&self.folders[i].name).as_deref() == Some(&name))
                    .collect();
                folders
                    .sort_by_key(|&i| (self.folders[i].role, self.folders[i].name.to_lowercase()));
                Account { name, folders }
            })
            .collect()
    }

    /// A folder's name without its account: `Inbox`, `[Gmail]/Drafts`.
    pub fn short_name(&self, f: &Folder) -> String {
        match self.account_of(&f.name) {
            Some(a) => f.name[a.len() + 1..].to_owned(),
            None => f.name.clone(),
        }
    }

    /// The folders of a role, across accounts.
    pub fn with_role(&self, role: Role) -> Vec<&Folder> {
        self.folders.iter().filter(|f| f.role == role).collect()
    }

    /// Where a message moves to for `role` (archive, trash, junk): the folder
    /// of that kind in the message's own account.
    pub fn folder_for(&self, path: &std::path::Path, role: Role) -> PathBuf {
        let home = path.parent().and_then(std::path::Path::parent);
        let name = self
            .folders
            .iter()
            .find(|f| Some(f.path.as_path()) == home)
            .map(|f| f.name.clone());
        let account = name.and_then(|n| self.account_of(&n));
        let configured = (role == Role::Archive && account.is_none())
            .then(|| {
                self.opts
                    .archive
                    .as_deref()
                    .and_then(|d| self.find_folder(d))
            })
            .flatten();
        if let Some(f) = configured {
            return f.path.clone();
        }
        let found = self
            .folders
            .iter()
            .find(|f| f.role == role && self.account_of(&f.name) == account);
        match (found, &account) {
            (Some(f), _) => f.path.clone(),
            (None, Some(a)) => self.root.join(a).join(default_name(role)),
            (None, None) => self.root.join(default_name(role)),
        }
    }
}

impl State {
    /// The messages of the conversation a message is in, first message first.
    pub fn thread_of(&self, path: &std::path::Path) -> Vec<PathBuf> {
        let Some(at) = self
            .rows
            .iter()
            .position(|r| self.messages[r.index].path == path)
        else {
            return vec![path.to_owned()];
        };
        let mut start = at;
        while start > 0 && !self.rows[start].head {
            start -= 1;
        }
        let mut end = start + 1;
        while end < self.rows.len() && !self.rows[end].head {
            end += 1;
        }
        self.rows[start..end]
            .iter()
            .map(|r| self.messages[r.index].path.clone())
            .collect()
    }

    /// Fold or open the conversation of the selected message. Returns what
    /// it is now (true: folded).
    pub fn fold_selected(&mut self, how: &str) -> Option<bool> {
        let sel = self.selected.clone()?;
        let head = self.thread_of(&sel).into_iter().next()?;
        if !self.thread_info.contains_key(&head) {
            return None;
        }
        let fold = match how {
            "open" => false,
            "close" => true,
            _ => !self.folded.contains(&head),
        };
        if fold {
            self.folded.insert(head.clone());
            // the one in front is the first of it now
            self.selected = Some(head);
        } else {
            self.folded.remove(&head);
        }
        self.apply_query();
        Some(fold)
    }

    /// Fold every conversation, or open them all.
    pub fn fold_all(&mut self, fold: bool) {
        self.folded = if fold {
            self.thread_info.keys().cloned().collect()
        } else {
            Default::default()
        };
        self.apply_query();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use rical_maildir::flags::Flags;

    fn tree(root: &std::path::Path) {
        for f in [
            "home/INBOX",
            "home/Archive",
            "work/INBOX",
            "work/Sent",
            "gmail/[Gmail]/Drafts",
            "gmail/INBOX",
        ] {
            for s in ["cur", "new", "tmp"] {
                std::fs::create_dir_all(root.join(f).join(s)).unwrap();
            }
        }
    }

    #[test]
    fn accounts_unified_and_each_messages_own_archive() {
        let d = tempfile::tempdir().unwrap();
        tree(d.path());
        let mut s = State::new(Options::default(), d.path().to_owned(), None);
        s.reload_messages();
        assert!(s.grouped());
        let names: Vec<String> = s.accounts().into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["gmail", "home", "work"]);
        let work = &s.accounts()[2];
        let shown: Vec<String> = work
            .folders
            .iter()
            .map(|&i| s.short_name(&s.folders[i]))
            .collect();
        assert_eq!(shown, ["INBOX", "Sent"]);
        assert_eq!(s.with_role(Role::Inbox).len(), 3);

        // a message in work/INBOX archives to work's Archive (made if needed),
        // one in home/INBOX to home's
        let w = store::deliver(
            &d.path().join("work/INBOX"),
            b"Subject: x\n\nx\n",
            Flags::default(),
            true,
        )
        .unwrap();
        let h = store::deliver(
            &d.path().join("home/INBOX"),
            b"Subject: x\n\nx\n",
            Flags::default(),
            true,
        )
        .unwrap();
        assert_eq!(
            s.folder_for(&w, Role::Archive),
            d.path().join("work/Archive")
        );
        assert_eq!(
            s.folder_for(&h, Role::Archive),
            d.path().join("home/Archive")
        );
        assert_eq!(s.folder_for(&w, Role::Trash), d.path().join("work/Trash"));
    }

    #[test]
    fn a_single_account_stays_flat() {
        let d = tempfile::tempdir().unwrap();
        for f in ["INBOX", "Sent"] {
            for s in ["cur", "new", "tmp"] {
                std::fs::create_dir_all(d.path().join(f).join(s)).unwrap();
            }
        }
        let mut s = State::new(Options::default(), d.path().to_owned(), None);
        s.reload_messages();
        assert!(!s.grouped());
        assert_eq!(s.account_of("INBOX"), None);
    }
}

#[cfg(test)]
mod conversation_tests {
    use super::*;

    fn deliver(root: &std::path::Path, id: &str, reply_to: Option<&str>, date: &str) -> PathBuf {
        let mut raw = format!(
            "From: a@x\nSubject: {}{id}\nDate: {date}\nMessage-ID: <{id}@x>\n",
            if reply_to.is_some() { "Re: " } else { "" }
        );
        if let Some(r) = reply_to {
            raw += &format!("In-Reply-To: <{r}@x>\n");
        }
        raw += "\nbody\n";
        store::deliver(
            &root.join("INBOX"),
            raw.as_bytes(),
            rical_maildir::flags::Flags {
                seen: true,
                ..Default::default()
            },
            false,
        )
        .unwrap()
    }

    #[test]
    fn folding_hides_replies_and_acts_on_the_whole_conversation() {
        let d = tempfile::tempdir().unwrap();
        let head = deliver(d.path(), "a", None, "Thu, 5 Nov 2026 10:00:00 +0000");
        let r1 = deliver(d.path(), "b", Some("a"), "Thu, 5 Nov 2026 11:00:00 +0000");
        let r2 = deliver(d.path(), "c", Some("b"), "Thu, 5 Nov 2026 12:00:00 +0000");
        let other = deliver(d.path(), "z", None, "Wed, 4 Nov 2026 09:00:00 +0000");
        let mut s = State::new(Options::default(), d.path().to_owned(), None);
        s.reload_messages();
        assert_eq!(s.shown.len(), 4);
        s.selected = Some(s.thread_of(&r1)[0].clone());
        assert_eq!(s.fold_selected("toggle"), Some(true));
        // the conversation shows as its first message
        assert_eq!(s.shown.len(), 2);
        assert_eq!(s.selected.as_deref(), Some(head.as_path()));
        assert_eq!(s.thread_info[&head].0, 3);
        // acting on it acts on all three
        let mut t = s.targets();
        t.sort();
        let mut want = vec![head.clone(), r1.clone(), r2.clone()];
        want.sort();
        assert_eq!(t, want);
        // a message not in a conversation cannot be folded
        s.selected = Some(other);
        assert_eq!(s.fold_selected("toggle"), None);
        s.fold_all(false);
        assert_eq!(s.shown.len(), 4);
    }

    #[test]
    fn sorting_changes_the_order_and_keeps_conversations_together() {
        let d = tempfile::tempdir().unwrap();
        deliver(d.path(), "a", None, "Thu, 5 Nov 2026 10:00:00 +0000");
        deliver(d.path(), "b", Some("a"), "Thu, 5 Nov 2026 11:00:00 +0000");
        deliver(d.path(), "z", None, "Wed, 4 Nov 2026 09:00:00 +0000");
        let mut s = State::new(Options::default(), d.path().to_owned(), None);
        s.reload_messages();
        let subjects = |s: &State| -> Vec<String> {
            (0..s.shown.len())
                .map(|n| s.message_at(n).unwrap().subject.clone())
                .collect()
        };
        assert_eq!(subjects(&s), ["a", "Re: b", "z"]);
        s.sort = rical_maildir::thread::SortKey::Subject;
        s.sort_reverse = true;
        s.arrange();
        assert_eq!(subjects(&s), ["z", "a", "Re: b"]);
    }
}

/// An address you send from, and the account it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub mailbox: Mailbox,
    pub account: Option<String>,
}

impl Identity {
    /// As the From chooser shows it.
    pub fn label(&self) -> String {
        match &self.account {
            Some(a) => format!("{} · {a}", self.mailbox.display()),
            None => self.mailbox.display(),
        }
    }
}

impl State {
    /// Every address you send from: each account's, then the rest of `me`.
    pub fn identities(&self) -> Vec<Identity> {
        let mut out: Vec<Identity> = Vec::new();
        for (account, list) in &self.opts.accounts {
            for m in list {
                out.push(Identity {
                    mailbox: m.clone(),
                    account: Some(account.clone()),
                });
            }
        }
        for m in &self.opts.me {
            if !out
                .iter()
                .any(|i| i.mailbox.address.eq_ignore_ascii_case(&m.address))
            {
                out.push(Identity {
                    mailbox: m.clone(),
                    account: None,
                });
            }
        }
        out
    }

    /// The account of the folder in front, if it is in one.
    pub fn current_account(&self) -> Option<String> {
        self.folder().and_then(|f| self.account_of(&f.name))
    }

    /// The account a message is in, by its file.
    pub fn account_of_path(&self, path: &std::path::Path) -> Option<String> {
        let home = path.parent().and_then(std::path::Path::parent)?;
        let f = self.folders.iter().find(|f| f.path == home)?;
        self.account_of(&f.name)
    }

    /// Where a draft is kept: the account's Drafts, else the usual one.
    pub fn drafts_for(&self, account: Option<&str>) -> PathBuf {
        account
            .and_then(|a| {
                self.folders.iter().find(|f| {
                    f.role == Role::Drafts && self.account_of(&f.name).as_deref() == Some(a)
                })
            })
            .map_or_else(|| self.drafts_folder(), |f| f.path.clone())
    }

    /// Where the copy of sent mail goes for an account, when it has a Sent.
    pub fn sent_for(&self, account: Option<&str>) -> Option<PathBuf> {
        let a = account?;
        self.folders
            .iter()
            .find(|f| f.role == Role::Sent && self.account_of(&f.name).as_deref() == Some(a))
            .map(|f| f.path.clone())
    }
}

impl State {
    /// Keep only these of what is shown (what a search inside the messages
    /// found, after the headers matched).
    pub fn restrict_shown(&mut self, keep: &std::collections::HashSet<PathBuf>) {
        let rows = &self.rows;
        let messages = &self.messages;
        self.shown
            .retain(|&i| keep.contains(&messages[rows[i].index].path));
        if !self.selected.as_ref().is_some_and(|p| keep.contains(p)) {
            self.selected = self
                .shown
                .first()
                .map(|&i| self.messages[self.rows[i].index].path.clone());
            self.opened = None;
        }
    }
}

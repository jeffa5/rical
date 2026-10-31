//! Reading the messages of a place, away from the window: a [`Job`] holds
//! everything it needs (no GTK, no shared state) and runs on another thread;
//! what it gives back is put in the state on the main one.

use std::{collections::HashMap, path::PathBuf};

use rical_maildir::{
    backend::Backend,
    search::Query,
    store::{self, Folder, Role, Summary},
};

use crate::state::{Layout, Place, State};

/// Where the header index is kept, with the app's state.
fn cache_dir() -> Option<PathBuf> {
    rical_ui::app::state_dir().map(|d| d.join("cache"))
}

/// The header cache of a folder, kept with the app's state.
pub fn cache_for(f: &Folder) -> rical_maildir::cache::Cache {
    match cache_dir() {
        Some(d) => rical_maildir::cache::Cache::open(&d, &f.path),
        None => rical_maildir::cache::Cache::none(),
    }
}

/// A folder's messages: from the header index alone if `quick` (as the
/// folder was when last listed), else listed (reading what is new).
fn list(f: &Folder, quick: bool) -> Vec<Summary> {
    if quick
        && let Some(dir) = cache_dir()
        && let Some(found) = store::list_indexed(f, &dir)
    {
        return found;
    }
    store::list_cached(f, cache_for(f))
}

/// What to read, with what reading it needs.
#[derive(Clone)]
pub struct Job {
    pub generation: u64,
    pub place: Place,
    /// The folders it reads, each with what to call it in a list.
    folders: Vec<(Folder, String)>,
    queue: PathBuf,
    builtin: bool,
    /// How to lay the list out: done here too, as for a big folder it is
    /// a noticeable part of the work.
    layout: Layout,
    /// From the header index alone, without looking at the folders.
    quick: bool,
}

pub struct Loaded {
    pub generation: u64,
    pub place: Place,
    pub messages: Vec<Summary>,
    pub notes: HashMap<PathBuf, String>,
    pub layout: Layout,
    pub rows: Vec<rical_maildir::thread::Row>,
}

impl State {
    /// The reading the place in front needs.
    pub fn load_job(&mut self) -> Job {
        self.generation += 1;
        let label = |f: &Folder| self.account_of(&f.name).unwrap_or_else(|| f.name.clone());
        let folders: Vec<(Folder, String)> = match &self.place {
            Place::Folder(p) => self
                .folders
                .iter()
                .filter(|f| &f.path == p)
                .map(|f| (f.clone(), label(f)))
                .collect(),
            Place::Unified(r) => self
                .folders
                .iter()
                .filter(|f| f.role == *r)
                .map(|f| (f.clone(), label(f)))
                .collect(),
            Place::Search(q) => {
                let q = Query::parse(q);
                self.folders
                    .iter()
                    // junk and trash stay out of a search across everything, unless asked
                    .filter(|f| !matches!(f.role, Role::Junk | Role::Trash) || !q.folder.is_empty())
                    .map(|f| (f.clone(), f.name.clone()))
                    .collect()
            }
            Place::Outbox => Vec::new(),
        };
        Job {
            generation: self.generation,
            place: self.place.clone(),
            folders,
            queue: self.opts.queue.clone(),
            builtin: self.opts.search == Backend::Builtin,
            layout: self.layout(),
            quick: false,
        }
    }

    /// Put what a job read in place, unless the place moved on since.
    pub fn apply_loaded(&mut self, l: Loaded) -> bool {
        if l.generation != self.generation || l.place != self.place {
            return false;
        }
        self.put_loaded(l);
        true
    }

    /// [`State::apply_loaded`] of what catches up with what was shown
    /// from the header index: only if it differs.
    pub fn apply_caught_up(&mut self, l: Loaded) -> bool {
        if l.generation != self.generation
            || l.place != self.place
            || (l.messages == self.messages && l.notes == self.outbox_notes)
        {
            return false;
        }
        self.put_loaded(l);
        true
    }

    fn put_loaded(&mut self, l: Loaded) {
        self.messages = l.messages;
        self.outbox_notes = l.notes;
        // laid out as it is still wanted, or again if that changed
        if l.layout == self.layout() {
            self.rows = l.rows;
            self.arranged();
        } else {
            self.arrange();
        }
    }
}

impl Job {
    /// The same reading from the header index alone, to show at once
    /// (before this one catches up with the folders), if the place is
    /// folders.
    pub fn quick(&self) -> Option<Job> {
        matches!(self.place, Place::Folder(_) | Place::Unified(_)).then(|| Job {
            quick: true,
            ..self.clone()
        })
    }

    /// Read: on any thread.
    pub fn run(self) -> Loaded {
        let mut notes = HashMap::new();
        let messages = match &self.place {
            Place::Folder(_) => self
                .folders
                .first()
                .map(|(f, _)| list(f, self.quick))
                .unwrap_or_default(),
            Place::Unified(_) => {
                let mut all = Vec::new();
                for (f, label) in &self.folders {
                    for m in list(f, self.quick) {
                        notes.insert(m.path.clone(), label.clone());
                        all.push(m);
                    }
                }
                all.sort_by_key(|m| std::cmp::Reverse(m.date));
                all
            }
            Place::Search(q) => {
                let mut all: Vec<Summary> = if self.builtin {
                    let q = Query::parse(q);
                    let mut found = Vec::new();
                    for (f, _) in &self.folders {
                        found.extend(
                            store::list_cached(f, cache_for(f))
                                .into_iter()
                                .filter(|m| q.matches(m)),
                        );
                    }
                    found.retain(|m| q.matches_body(m));
                    found
                } else {
                    store::list_links(&State::search_dir())
                };
                all.sort_by_key(|m| std::cmp::Reverse(m.date));
                for m in &all {
                    notes.insert(m.path.clone(), format!("in {}", m.folder));
                }
                all
            }
            Place::Outbox => outbox(&self.queue, &mut notes),
        };
        let rows = self.layout.rows(&messages);
        Loaded {
            generation: self.generation,
            place: self.place,
            messages,
            notes,
            layout: self.layout,
            rows,
        }
    }
}

fn outbox(queue: &std::path::Path, notes: &mut HashMap<PathBuf, String>) -> Vec<Summary> {
    use rical_mail_queue::queue::{Queue, State as Q};
    let Ok(queue) = Queue::open(queue) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in queue.list().unwrap_or_default() {
        let Some(mut m) = store::summarize(&e.path, "Outbox", false) else {
            continue;
        };
        m.flags.seen = true;
        let why = |l: Option<&str>| l.map(|l| format!(" · {l}")).unwrap_or_default();
        let note = match e.state {
            Q::Queued => match e.next_attempt {
                Some(t) if t > chrono::Utc::now() => format!(
                    "held back until {} (Undo, or delete it here)",
                    t.with_timezone(&chrono::Local).format("%H:%M:%S")
                ),
                _ => "waiting to be sent".to_owned(),
            },
            Q::Sending => "sending now".to_owned(),
            Q::Retrying => format!("will try again{}", why(e.last_error.as_deref())),
            Q::Failed => format!("failed{}", why(e.last_error.as_deref())),
        };
        notes.insert(m.path.clone(), note);
        out.push(m);
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.date));
    out
}

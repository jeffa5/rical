//! Opening cards from other apps (a `.vcf` from a mail, a calendar's
//! guest, an export). Each is matched by UID first, so opening someone you
//! have shows yours instead of adding a copy:
//!
//! - the same card: it is shown;
//! - the same card, changed: offered as an update (yes by default when
//!   it is newer);
//! - someone you probably have (an address or number in common): merge
//!   or add;
//! - someone new: added, to the address book chosen.

use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use adw::prelude::*;
use gtk::gio;
use rical_cards::import::{self, Incoming, Match};

use crate::window::Window;

/// Cards still to ask about, and the new ones to add at the end.
#[derive(Default)]
struct Queue {
    asks: VecDeque<Incoming>,
    new: Vec<Incoming>,
}

impl Window {
    /// Import the files given on the command line or by another app.
    pub fn import_files(self: &Rc<Self>, files: &[gio::File]) {
        let mut incoming = Vec::new();
        for f in files {
            let Some(path) = f.path() else {
                self.toast("Only local files can be opened");
                continue;
            };
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    self.toast(&format!("Could not read {}: {e}", path.display()));
                    continue;
                }
            };
            // a card already in the address books, opened as a file (say,
            // from a file manager): just show it
            let already = {
                let s = self.state.borrow();
                s.books
                    .book_of(&path)
                    .is_some()
                    .then(|| rical_cards::CardId {
                        path: path.clone(),
                        index: 0,
                    })
            };
            if let Some(id) = already {
                self.show_person(&id);
                continue;
            }
            match import::plan(&self.state.borrow().books, &text) {
                Ok(cards) => incoming.extend(cards),
                Err(e) => self.toast(&format!("{}: {e:#}", path.display())),
            }
        }
        log::info!("importing {} cards", incoming.len());
        let mut queue = Queue::default();
        for i in incoming {
            match &i.matched {
                Match::Same(id) => {
                    let id = id.clone();
                    self.show_person(&id);
                    self.toast(&format!("{} is in your contacts", i.card().display_name()));
                }
                Match::New => queue.new.push(i),
                _ => queue.asks.push_back(i),
            }
        }
        self.ask_next(Rc::new(RefCell::new(queue)));
    }

    /// Show someone in the people list.
    pub fn show_person(self: &Rc<Self>, id: &rical_cards::CardId) {
        if self.state.borrow().view != crate::state::View::People {
            self.set_view(crate::state::View::People);
        }
        self.select_card(id);
        self.split.set_show_content(true);
    }

    /// Ask about the cards that may be someone you have, one at a time,
    /// then add the new ones together.
    fn ask_next(self: &Rc<Self>, queue: Rc<RefCell<Queue>>) {
        let next = queue.borrow_mut().asks.pop_front();
        let Some(incoming) = next else {
            let new = std::mem::take(&mut queue.borrow_mut().new);
            if !new.is_empty() {
                self.add_new(new);
            }
            return;
        };
        let theirs = incoming.card().display_name();
        let (id, alert) = match &incoming.matched {
            Match::Changed { id, newer } => {
                let mine = self.name_of(id);
                let alert = adw::AlertDialog::builder()
                    .heading(format!("Update {mine}?"))
                    .body(if *newer {
                        format!("The card opened is a newer version of {mine}'s. Yours, in {}, is replaced with it.", self.state.borrow().books.place(&id.path))
                    } else {
                        format!(
                            "The card opened is {mine}'s, but differs from yours and isn't newer. Updating replaces yours, in {}.",
                            self.state.borrow().books.place(&id.path)
                        )
                    })
                    .close_response("keep")
                    .default_response(if *newer { "update" } else { "keep" })
                    .build();
                alert.add_response("keep", "Keep Mine");
                alert.add_response("update", "Update");
                alert.set_response_appearance(
                    "update",
                    if *newer {
                        adw::ResponseAppearance::Suggested
                    } else {
                        adw::ResponseAppearance::Default
                    },
                );
                (id.clone(), alert)
            }
            Match::Likely(id) => {
                let mine = self.name_of(id);
                let heading = if theirs == mine {
                    format!("Is This the {mine} You Have?")
                } else {
                    format!("Is {theirs} the Same as {mine}?")
                };
                let alert = adw::AlertDialog::builder()
                    .heading(heading)
                    .body(format!(
                        "The card opened shares an email address or number with {mine}. \
                         Merge it into theirs, or add {theirs} as someone new."
                    ))
                    .close_response("skip")
                    .default_response("merge")
                    .build();
                alert.add_response("skip", "Skip");
                alert.add_response("add", "Add as New");
                alert.add_response("merge", "Merge…");
                alert.set_response_appearance("merge", adw::ResponseAppearance::Suggested);
                (id.clone(), alert)
            }
            _ => return self.ask_next(queue),
        };
        let w = self.clone();
        alert.connect_response(None, move |_, response| {
            let result = match response {
                "update" => w.replace(&id, &incoming),
                "add" => {
                    queue.borrow_mut().new.push(incoming.clone());
                    Ok(())
                }
                "merge" => {
                    // the rest wait for the merge
                    let (w2, q) = (w.clone(), queue.clone());
                    match w.merge_incoming(&id, &incoming, Rc::new(move || w2.ask_next(q.clone())))
                    {
                        Ok(()) => return,
                        Err(e) => Err(e),
                    }
                }
                _ => {
                    w.show_person(&id);
                    Ok(())
                }
            };
            if let Err(e) = result {
                w.toast(&e);
            }
            w.ask_next(queue.clone());
        });
        alert.present(Some(&self.window));
    }

    fn name_of(&self, id: &rical_cards::CardId) -> String {
        self.state
            .borrow()
            .books
            .get(id)
            .map_or("them".to_owned(), |c| c.display_name())
    }

    fn replace(
        self: &Rc<Self>,
        id: &rical_cards::CardId,
        incoming: &Incoming,
    ) -> Result<(), String> {
        let change = self
            .state
            .borrow_mut()
            .books
            .replace_with(id, incoming)
            .map_err(|e| format!("{e:#}"))?;
        self.state
            .borrow_mut()
            .record("update the contact from a file", vec![change]);
        self.after_change(Some(id));
        self.split.set_show_content(true);
        self.undo_toast(&format!("Updated {}", self.name_of(id)));
        Ok(())
    }

    /// Add the incoming card, then merge it into the one you have.
    fn merge_incoming(
        self: &Rc<Self>,
        id: &rical_cards::CardId,
        incoming: &Incoming,
        then: Rc<dyn Fn()>,
    ) -> Result<(), String> {
        let book = self
            .state
            .borrow()
            .books
            .book_of(&id.path)
            .ok_or("that address book is gone")?;
        let (added, new_id) = self
            .state
            .borrow_mut()
            .books
            .add_incoming(book, incoming)
            .map_err(|e| format!("{e:#}"))?;
        self.after_change(Some(id));
        // the two, as a group of duplicates to merge
        let group = rical_cards::dedupe::Group {
            cards: vec![id.clone(), new_id],
            reasons: Vec::new(),
        };
        crate::tidy::merge_group(self, group, Some(crate::tidy::Imported { added, then }))
    }

    /// Add someone (or several) new, asking which address book when there
    /// is a choice.
    fn add_new(self: &Rc<Self>, cards: Vec<Incoming>) {
        let (names, default) = {
            let s = self.state.borrow();
            let names: Vec<String> = s.books.books.iter().map(|b| b.name.clone()).collect();
            (names, s.default_book().unwrap_or(0))
        };
        if names.is_empty() {
            self.toast("There are no address books to add to");
            return;
        }
        let who = if cards.len() == 1 {
            cards[0].card().display_name()
        } else {
            format!("{} people", cards.len())
        };
        let add = {
            let w = self.clone();
            let who = who.clone();
            move |book: usize| {
                let mut changes = Vec::new();
                let mut last = None;
                for c in &cards {
                    match w.state.borrow_mut().books.add_incoming(book, c) {
                        Ok((change, id)) => {
                            changes.push(change);
                            last = Some(id);
                        }
                        Err(e) => w.toast(&format!("{e:#}")),
                    }
                }
                let n = changes.len();
                w.state.borrow_mut().record("add from a file", changes);
                if let Some(id) = last {
                    w.after_change(Some(&id));
                    w.show_person(&id);
                }
                if n > 0 {
                    let book = w.state.borrow().books.books[book].name.clone();
                    let who = if n == 1 {
                        who.clone()
                    } else {
                        format!("{n} people")
                    };
                    w.undo_toast(&format!("Added {who} to {book}"));
                }
            }
        };
        let alert = adw::AlertDialog::builder()
            .heading(format!("Add {who}?"))
            .close_response("cancel")
            .default_response("add")
            .build();
        alert.add_response("cancel", "Cancel");
        alert.add_response("add", "Add");
        alert.set_response_appearance("add", adw::ResponseAppearance::Suggested);
        let dropdown =
            gtk::DropDown::from_strings(&names.iter().map(String::as_str).collect::<Vec<_>>());
        dropdown.set_selected(default as u32);
        // where they go, and it follows the choice
        let place = gtk::Label::builder()
            .css_classes(["dim-label", "caption"])
            .selectable(true)
            .wrap(true)
            .build();
        let places: Vec<String> = {
            let s = self.state.borrow();
            (0..names.len()).map(|i| s.books.book_place(i)).collect()
        };
        let show = {
            let (place, places) = (place.clone(), places.clone());
            move |i: u32| {
                place.set_label(&format!(
                    "Saved in {}",
                    places.get(i as usize).cloned().unwrap_or_default()
                ))
            }
        };
        show(default as u32);
        dropdown.connect_selected_notify(move |d| show(d.selected()));
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .halign(gtk::Align::Center)
            .build();
        if names.len() > 1 {
            let row = gtk::Box::builder()
                .spacing(12)
                .halign(gtk::Align::Center)
                .build();
            row.append(&gtk::Label::new(Some("To")));
            row.append(&dropdown);
            b.append(&row);
        }
        b.append(&place);
        alert.set_extra_child(Some(&b));
        alert.connect_response(None, move |_, response| {
            if response == "add" {
                add(dropdown.selected() as usize);
            }
        });
        alert.present(Some(&self.window));
    }
}

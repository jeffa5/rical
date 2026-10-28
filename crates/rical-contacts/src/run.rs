//! Running commands: from the command line, key bindings and buttons.

use std::rc::Rc;

use adw::prelude::*;
use gtk::gio;
use rical_cards::{Card, CardId, Field, edit::Draft};
use rical_ui::{
    command::{Parsed, Registry, Values},
    palette,
    util::{map_url, open_uri},
};

use crate::{
    cmds,
    state::{State, View},
    window::Window,
};

pub const REGISTRY: Registry = Registry { specs: cmds::SPECS };

fn context(view: View) -> rical_ui::command::Context {
    match view {
        View::People => cmds::PEOPLE,
        View::Duplicates => cmds::DUPLICATES,
    }
}

/// What `:` completes from the address books.
struct StateValues<'a>(&'a State);

impl Values for StateValues<'_> {
    fn values(&self, key: &str) -> Vec<String> {
        match key {
            "book" => self.0.books.books.iter().map(|b| b.name.clone()).collect(),
            _ => Vec::new(),
        }
    }
}

/// The preferred of some fields, else the first.
fn preferred(fields: &[Field]) -> Option<&Field> {
    fields.iter().find(|f| f.pref).or(fields.first())
}

impl Window {
    /// Run one command line, e.g. "search leeds" or "new Jo Park jo@example.com".
    pub fn execute(self: &Rc<Self>, line: &str) -> Result<(), String> {
        let ctx = context(self.state.borrow().view);
        let p = REGISTRY.parse(line, ctx).map_err(|e| e.to_string())?;
        self.run(&p)
    }

    fn run(self: &Rc<Self>, p: &Parsed) -> Result<(), String> {
        match p.spec {
            "view" => {
                let view = match p.arg(0).unwrap() {
                    "duplicates" => View::Duplicates,
                    _ => View::People,
                };
                self.set_view(view);
            }
            "search" => {
                let text = p.arg(0).unwrap_or("");
                if self.state.borrow().view != View::People {
                    self.set_view(View::People);
                }
                self.search.set_text(text);
                self.search.grab_focus();
                self.search.set_position(-1);
            }
            "select" => self.select(p.arg(0).unwrap()),
            "back" => self.back(),
            "new" => match p.arg(0).filter(|t| !t.trim().is_empty()) {
                Some(text) => self.quick_add(text, p.flag("book"))?,
                None => crate::editor::open(self, None),
            },
            "edit" => {
                let id = self.selected_card()?;
                crate::editor::open(self, Some(id));
            }
            "delete" => self.delete()?,
            "mail" => {
                let to = self.with_card(|c| preferred(&c.emails()).map(|f| f.value.clone()))?;
                let to = to.ok_or("they have no email address")?;
                open_uri(Some(self.window.upcast_ref()), &format!("mailto:{to}"));
            }
            "call" => {
                let number = self.with_card(|c| preferred(&c.phones()).map(|f| f.value.clone()))?;
                let number: String = number
                    .ok_or("they have no phone number")?
                    .chars()
                    .filter(|c| c.is_ascii_digit() || *c == '+')
                    .collect();
                open_uri(Some(self.window.upcast_ref()), &format!("tel:{number}"));
            }
            "map" => {
                let place = self.with_card(|c| {
                    let a = c.addresses();
                    a.iter()
                        .find(|a| a.pref)
                        .or(a.first())
                        .map(|a| a.one_line())
                })?;
                let place = place.ok_or("they have no address")?;
                open_uri(Some(self.window.upcast_ref()), &map_url(&place));
            }
            "copy" => {
                let what = p.arg(0).unwrap();
                let text = match what {
                    "card" => {
                        let id = self.selected_card()?;
                        std::fs::read_to_string(&id.path).map_err(|e| e.to_string())?
                    }
                    _ => self
                        .with_card(|c| match what {
                            "email" => preferred(&c.emails()).map(|f| f.value.clone()),
                            "phone" => preferred(&c.phones()).map(|f| f.value.clone()),
                            "address" => c.addresses().first().map(|a| a.one_line()),
                            _ => Some(c.display_name()),
                        })?
                        .ok_or(format!("they have no {what}"))?,
                };
                self.window.clipboard().set_text(&text);
                self.toast(&format!("Copied {}", ellipsize(&text)));
            }
            "open-file" => {
                let id = self.selected_card()?;
                let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&id.path)));
                // this app may be the one that opens vCards: let them choose
                launcher.set_always_ask(true);
                launcher.launch(Some(&self.window), None::<&gio::Cancellable>, |r| {
                    if let Err(e) = r {
                        log::warn!("could not open the file: {e}");
                    }
                });
            }
            "merge" => crate::tidy::merge(self)?,
            "not-duplicates" => {
                let group = self.selected_group().ok_or("no duplicates selected")?;
                self.state.borrow_mut().say_different(&group);
                self.fill_duplicates();
                self.show_detail();
                self.focus_view();
                self.toast("They won't be suggested again");
            }
            "undo" => self.undo(true)?,
            "redo" => self.undo(false)?,
            "history" => self.show_history(),
            "reload" => {
                let changed = self
                    .state
                    .borrow_mut()
                    .books
                    .refresh()
                    .map_err(|e| format!("{e:#}"))?;
                self.refresh();
                self.toast(if changed {
                    "Reloaded"
                } else {
                    "Nothing changed"
                });
            }
            "settings" => crate::prefs::show(self),
            "about" => self.about(),
            "help" => {
                let section = self.state.borrow().view.section();
                rical_ui::help::dialog(&cmds::HELP, &self.binds, section)
                    .present(Some(&self.window));
            }
            "quit" => {
                if let Some(app) = self.window.application() {
                    app.quit();
                }
            }
            other => return Err(format!("“{other}” isn't done yet")),
        }
        Ok(())
    }

    pub fn set_view(self: &Rc<Self>, view: View) {
        {
            let mut s = self.state.borrow_mut();
            s.view = view;
            if view == View::Duplicates && s.selected_group.is_none() {
                s.selected_group = Some(0);
            }
        }
        self.refresh();
        self.focus_view();
    }

    fn selected_card(&self) -> Result<CardId, String> {
        let s = self.state.borrow();
        s.selected
            .clone()
            .filter(|id| s.books.get(id).is_some())
            .ok_or_else(|| "no one is selected".to_owned())
    }

    fn with_card<T>(&self, f: impl FnOnce(&Card) -> T) -> Result<T, String> {
        let id = self.selected_card()?;
        let s = self.state.borrow();
        let card = s.books.get(&id).ok_or("no one is selected")?;
        Ok(f(&card))
    }

    pub fn selected_group(&self) -> Option<rical_cards::dedupe::Group> {
        let mut s = self.state.borrow_mut();
        let i = s.selected_group?;
        s.groups().get(i).cloned()
    }

    fn select(&self, which: &str) {
        let view = self.state.borrow().view;
        match view {
            View::People => {
                let n = self.selection.n_items() as usize;
                if n == 0 {
                    return;
                }
                let at = self.selection.selected();
                let at = (at != gtk::INVALID_LIST_POSITION).then_some(at as usize);
                let to = match which {
                    "first" => 0,
                    "last" => n - 1,
                    "next" => at.map_or(0, |a| (a + 1).min(n - 1)),
                    _ => at.map_or(0, |a| a.saturating_sub(1)),
                };
                self.select_index(to);
                self.list.grab_focus();
            }
            View::Duplicates => {
                let n = self.state.borrow_mut().groups().len();
                if n == 0 {
                    return;
                }
                let at = self.state.borrow().selected_group;
                let to = match which {
                    "first" => 0,
                    "last" => n - 1,
                    "next" => at.map_or(0, |a| (a + 1).min(n - 1)),
                    _ => at.map_or(0, |a| a.saturating_sub(1)),
                };
                if let Some(row) = self.dup_list.row_at_index(to as i32) {
                    self.dup_list.select_row(Some(&row));
                    row.grab_focus();
                }
            }
        }
    }

    /// Back out one step: from the person to the list (when narrow), then
    /// out of the search, then from the duplicates to everyone.
    fn back(self: &Rc<Self>) {
        if self.split.is_collapsed() && self.split.shows_content() {
            self.split.set_show_content(false);
            self.focus_view();
        } else if !self.search.text().is_empty() {
            self.syncing.set(true);
            self.search.set_text("");
            self.syncing.set(false);
            self.search_now();
            self.focus_view();
        } else if self.state.borrow().view == View::Duplicates {
            self.set_view(View::People);
        } else {
            self.focus_view();
        }
    }

    /// A contact made at once from a line: emails, numbers, and the rest
    /// is the name.
    fn quick_add(self: &Rc<Self>, text: &str, book: Option<&str>) -> Result<(), String> {
        let draft = quick_draft(text).ok_or("give a name, an email address or a number")?;
        let book = match book {
            Some(name) => self
                .state
                .borrow()
                .books
                .find_book(name)
                .ok_or(format!("no address book “{name}”"))?,
            None => self
                .state
                .borrow()
                .default_book()
                .ok_or("there are no address books to add to")?,
        };
        self.create(book, &draft)
    }

    /// Add someone new to `book`, select them and say so.
    pub fn create(self: &Rc<Self>, book: usize, draft: &Draft) -> Result<(), String> {
        let (change, id) = self
            .state
            .borrow_mut()
            .books
            .create(book, draft)
            .map_err(|e| format!("{e:#}"))?;
        self.state
            .borrow_mut()
            .record("add the contact", vec![change]);
        let (name, book_name) = {
            let s = self.state.borrow();
            (
                s.books
                    .get(&id)
                    .map(|c| c.display_name())
                    .unwrap_or_default(),
                s.books.books[book].name.clone(),
            )
        };
        self.after_change(Some(&id));
        self.undo_toast(&format!("Added {name} to {book_name}"));
        Ok(())
    }

    /// Save an edit of `id`.
    pub fn update(self: &Rc<Self>, id: &CardId, draft: &Draft) -> Result<(), String> {
        let change = self
            .state
            .borrow_mut()
            .books
            .update(id, draft)
            .map_err(|e| format!("{e:#}"))?;
        if change.before == change.after {
            return Ok(());
        }
        self.state
            .borrow_mut()
            .record("edit the contact", vec![change]);
        self.after_change(Some(id));
        self.undo_toast("Saved");
        Ok(())
    }

    /// Delete the selected person, after asking (Delete the default); undo
    /// and the history bring them back.
    fn delete(self: &Rc<Self>) -> Result<(), String> {
        let id = self.selected_card()?;
        let name = self.with_card(|c| c.display_name())?;
        let place = self.state.borrow().books.place(&id.path);
        let alert = adw::AlertDialog::builder()
            .heading(format!("Delete {name}?"))
            .body(format!(
                "{place} is deleted. Undo (u), or History (Ctrl+H) later, brings it back."
            ))
            .default_response("delete")
            .close_response("cancel")
            .build();
        alert.add_response("cancel", "Cancel");
        alert.add_response("delete", "Delete");
        alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        let w = self.clone();
        alert.connect_response(None, move |_, response| {
            if response == "delete"
                && let Err(e) = w.delete_now(&id, &name)
            {
                w.toast(&e);
            }
        });
        alert.present(Some(&self.window));
        Ok(())
    }

    fn delete_now(self: &Rc<Self>, id: &CardId, name: &str) -> Result<(), String> {
        let at = self.selection.selected();
        let change = self
            .state
            .borrow_mut()
            .books
            .delete(id)
            .map_err(|e| format!("{e:#}"))?;
        self.state
            .borrow_mut()
            .record("delete the contact", vec![change]);
        self.state.borrow_mut().selected = None;
        self.after_change(None);
        // the next person takes their place
        if at != gtk::INVALID_LIST_POSITION {
            self.select_index(at as usize);
        }
        self.undo_toast(&format!("Deleted {name}"));
        Ok(())
    }

    /// Show the books as they now are, with `select` selected.
    pub fn after_change(&self, select: Option<&CardId>) {
        self.state.borrow_mut().refresh();
        match select {
            Some(id) => self.select_card(id),
            None => self.fill_list(),
        }
        self.duplicates_soon();
        self.show_detail();
        self.focus_view();
    }

    /// A toast with an Undo button.
    pub fn undo_toast(self: &Rc<Self>, text: &str) {
        let toast = adw::Toast::builder()
            .title(text)
            .button_label("Undo")
            .action_name("win.run")
            .action_target(&"undo".to_variant())
            .build();
        self.toasts.add_toast(toast);
    }

    /// Undo (or redo) the latest change, from the history: after a restart
    /// too, and a merge as one.
    fn undo(self: &Rc<Self>, undo: bool) -> Result<(), String> {
        let entry = {
            let mut s = self.state.borrow_mut();
            s.history.as_mut().and_then(|h| {
                // the other app may have added to it
                if let Err(e) = h.reload() {
                    log::warn!("reading the history: {e:#}");
                }
                if undo {
                    h.last_done(crate::state::APP)
                } else {
                    h.next_undone(crate::state::APP)
                }
                .cloned()
            })
        };
        let entry = entry.ok_or(if undo {
            "nothing to undo"
        } else {
            "nothing to redo"
        })?;
        self.undo_entry(&entry.id, undo)?;
        self.toast(&format!(
            "{}: {}",
            if undo { "Undone" } else { "Redone" },
            entry.what
        ));
        Ok(())
    }

    /// Undo (or redo) one entry of the history, showing the person it
    /// brings back (or leaves).
    pub fn undo_entry(self: &Rc<Self>, id: &str, undo: bool) -> Result<(), String> {
        let result = self.state.borrow_mut().undo_entry(id, undo);
        // the first file there afterwards, to select
        let select = {
            let s = self.state.borrow();
            s.history.as_ref().and_then(|h| {
                h.entries()
                    .iter()
                    .find(|e| e.id == id)?
                    .files
                    .iter()
                    .find(|f| {
                        if undo {
                            f.before.is_some()
                        } else {
                            f.after.is_some()
                        }
                    })
                    .map(|f| CardId {
                        path: f.path.clone(),
                        index: 0,
                    })
            })
        };
        self.after_change(select.as_ref());
        result.map_err(|e| format!("Could not {}: {e:#}", if undo { "undo" } else { "redo" }))
    }

    /// Everything done here, to undo or redo any of it.
    fn show_history(self: &Rc<Self>) {
        let state = self.state.clone();
        let entries: rical_ui::history::Entries = Rc::new(move || {
            let mut s = state.borrow_mut();
            match s.history.as_mut() {
                Some(h) => {
                    let _ = h.reload();
                    (h.entries().to_vec(), h.dir().to_owned())
                }
                None => (Vec::new(), std::path::PathBuf::new()),
            }
        });
        let weak = Rc::downgrade(self);
        let toggle: rical_ui::history::Toggle = Rc::new(move |id, undo| match weak.upgrade() {
            Some(w) => w.undo_entry(id, undo),
            None => Ok(()),
        });
        rical_ui::history::dialog(crate::state::APP, entries, toggle).present(Some(&self.window));
    }

    fn about(&self) {
        let s = self.state.borrow();
        let details = format!(
            "contacts: {}\naddress books: {}\npeople: {}\n",
            s.books.root.display(),
            s.books
                .books
                .iter()
                .map(|b| b.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
            s.index.entries.len(),
        );
        rical_ui::util::about(
            &self.window,
            "A keyboard-first address book for folders of vCards: search every field, \
             open contacts from other apps, and tidy up duplicates.",
            &details,
        );
    }
}

fn ellipsize(s: &str) -> String {
    let first = s.lines().next().unwrap_or("");
    if first.chars().count() > 40 || s.lines().count() > 1 {
        format!("{}…", first.chars().take(40).collect::<String>())
    } else {
        first.to_owned()
    }
}

fn is_phoneish(word: &str) -> bool {
    let digits = word.chars().filter(char::is_ascii_digit).count();
    digits > 0
        && word
            .chars()
            .all(|c| c.is_ascii_digit() || "+-().".contains(c))
}

/// A new contact from a line of text: words with an @ are email
/// addresses, runs of digits a phone number, the rest the name.
pub fn quick_draft(text: &str) -> Option<Draft> {
    let mut d = Draft::default();
    let mut name = Vec::new();
    let mut phone = String::new();
    let flush = |phone: &mut String, d: &mut Draft| {
        if phone.chars().filter(char::is_ascii_digit).count() >= 3 {
            d.phones.push(Field {
                value: std::mem::take(phone),
                labels: Vec::new(),
                pref: false,
            });
        }
        phone.clear();
    };
    for word in text.split_whitespace() {
        if is_phoneish(word) {
            if !phone.is_empty() {
                phone.push(' ');
            }
            phone.push_str(word);
            continue;
        }
        flush(&mut phone, &mut d);
        let word = word.trim_matches(|c| matches!(c, '<' | '>' | ',' | ';'));
        if word.contains('@') && !word.starts_with('@') {
            d.emails.push(Field {
                value: word.trim_start_matches("mailto:").to_owned(),
                labels: Vec::new(),
                pref: false,
            });
        } else if !word.is_empty() {
            name.push(word.to_owned());
        }
    }
    flush(&mut phone, &mut d);
    if let Some((family, given)) = name.split_last() {
        d.formatted_name = name.join(" ");
        if !given.is_empty() {
            d.name.given = given.join(" ");
        }
        d.name.family = family.clone();
        if given.is_empty() {
            // one word: a given name
            d.name.given = std::mem::take(&mut d.name.family);
        }
    }
    (!d.formatted_name.is_empty() || !d.emails.is_empty() || !d.phones.is_empty()).then_some(d)
}

/// The command line runs and completes the contacts' commands.
impl palette::Host for Window {
    fn anchor(&self) -> gtk::Widget {
        self.search.clone().upcast()
    }

    fn window(&self) -> adw::ApplicationWindow {
        self.window.clone()
    }

    fn complete(&self, line: &str) -> rical_ui::command::Completion {
        let state = self.state.borrow();
        REGISTRY.complete(line, context(state.view), &StateValues(&state))
    }

    fn preview(&self, line: &str) -> Option<String> {
        let p = REGISTRY.parse(line, cmds::ALL).ok()?;
        if p.spec != "new" {
            return None;
        }
        let d = quick_draft(p.arg(0)?)?;
        let s = self.state.borrow();
        let book = match p.flag("book") {
            Some(b) => s.books.find_book(b),
            None => s.default_book(),
        }
        .map_or("?".to_owned(), |i| s.books.book_place(i));
        let mut parts = vec![if d.formatted_name.is_empty() {
            "(no name)".to_owned()
        } else {
            d.formatted_name.clone()
        }];
        parts.extend(d.emails.iter().map(|e| e.value.clone()));
        parts.extend(d.phones.iter().map(|e| e.value.clone()));
        parts.push(format!("in {book}"));
        Some(parts.join(" · "))
    }

    fn execute(self: &Rc<Self>, line: &str) -> Result<(), String> {
        Window::execute(self, line)
    }

    fn history(&self) -> &std::cell::RefCell<Vec<String>> {
        &self.history
    }

    fn focus_view(&self) {
        Window::focus_view(self);
    }

    fn placeholder(&self) -> &'static str {
        "command, e.g. new Jo Park jo@example.com, search leeds, view duplicates"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_add_reads_names_emails_and_numbers() {
        let d = quick_draft("Jo Park <jo@example.com> +44 113 496 0000").unwrap();
        assert_eq!(d.formatted_name, "Jo Park");
        assert_eq!(
            (d.name.given.as_str(), d.name.family.as_str()),
            ("Jo", "Park")
        );
        assert_eq!(d.emails[0].value, "jo@example.com");
        assert_eq!(d.phones[0].value, "+44 113 496 0000");

        let d = quick_draft("Priya").unwrap();
        assert_eq!(d.name.given, "Priya");
        assert!(d.name.family.is_empty());

        let d = quick_draft("sam@example.com").unwrap();
        assert!(d.formatted_name.is_empty());
        assert!(quick_draft("  ").is_none());
    }
}

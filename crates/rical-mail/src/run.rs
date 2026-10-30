//! Running commands: from the command line, key bindings and buttons.

use std::{path::Path, rc::Rc};

use adw::prelude::*;
use gtk::{gio, glib};
use rical_maildir::{
    backend::{Backend, Gives},
    compose::{self, Draft},
    flags::Flags,
    read,
    store::{self, Role},
};
use rical_ui::{
    command::{Parsed, Registry, Values},
    palette,
    util::open_uri,
};

use crate::{
    cmds,
    state::{Place, State, Undo},
    window::Window,
};

pub const REGISTRY: Registry = Registry { specs: cmds::SPECS };

/// What `:` completes from the folders and identities.
pub struct StateValues<'a>(pub &'a State);

impl Values for StateValues<'_> {
    fn values(&self, key: &str) -> Vec<String> {
        match key {
            "folder" => self.0.folders.iter().map(|f| f.name.clone()).collect(),
            "identity" => self.0.opts.me.iter().map(|m| m.address.clone()).collect(),
            _ => Vec::new(),
        }
    }
    fn suggestions(&self, key: &str) -> Vec<String> {
        if key == "tab" {
            return ["next", "prev", "mail", "last", "1", "2", "3", "4"]
                .iter()
                .map(|s| s.to_string())
                .collect();
        }
        if key != "query" {
            return Vec::new();
        }
        let mu = matches!(self.0.opts.search, Backend::Mu);
        let mut v: Vec<String> = [
            "from:",
            "to:",
            "subject:",
            "date:1w..",
            "unread",
            "flagged",
            "attachment",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if mu {
            v = [
                "from:",
                "to:",
                "subject:",
                "date:1w..",
                "maildir:/",
                "flag:unread",
                "flag:attach",
                "mime:",
                "size:1M..",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect();
        }
        v
    }
}

/// Run a shell command with a file as its argument, detached.
pub fn spawn_with(command: &str, file: &Path) -> Result<(), String> {
    let child = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("{command} \"$1\""))
        .arg("sh")
        .arg(file)
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("{command}: {e}"))?;
    // reaped when it ends
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

impl Window {
    /// Run one command line, e.g. "folder Sent" or "move Archive".
    pub fn execute(self: &Rc<Self>, line: &str) -> Result<(), String> {
        let p = REGISTRY
            .parse(line, cmds::LIST)
            .map_err(|e| e.to_string())?;
        self.run(&p)
    }

    fn selected_path(&self) -> Result<std::path::PathBuf, String> {
        self.state
            .borrow()
            .selected
            .clone()
            .ok_or_else(|| "no message is selected".to_owned())
    }

    /// The selected message, read: as the reader has it, or read now if the
    /// reader has not caught up (a command straight after moving on).
    fn opened(&self) -> Result<Rc<read::Opened>, String> {
        let path = self.selected_path()?;
        if let Some(o) = self
            .state
            .borrow()
            .opened
            .clone()
            .filter(|o| o.path == path)
        {
            return Ok(o);
        }
        let o = Rc::new(read::open(&path).map_err(|e| format!("{e:#}"))?);
        self.state.borrow_mut().opened = Some(o.clone());
        Ok(o)
    }

    fn run(self: &Rc<Self>, p: &Parsed) -> Result<(), String> {
        match p.spec {
            "select" => self.select(p.arg(0).unwrap_or("next")),
            "open" => {
                if self.state.borrow().in_drafts() {
                    return self.run(
                        &REGISTRY
                            .parse("edit", cmds::LIST)
                            .map_err(|e| e.to_string())?,
                    );
                }
                self.selected_path()?;
                self.split.set_show_content(true);
                self.reader.body.grab_focus();
            }
            "back" => self.back(),
            "folder" => {
                let name = p.arg(0).ok_or("which folder?")?;
                let (path, shown) = {
                    let s = self.state.borrow();
                    let f = s.find_folder(name).ok_or(format!("no folder “{name}”"))?;
                    (f.path.clone(), f.name.clone())
                };
                let _ = shown;
                self.go(Place::Folder(path));
            }
            "outbox" => self.go(Place::Outbox),
            "unified" => {
                let role = match p.arg(0).unwrap_or("inbox") {
                    "drafts" => Role::Drafts,
                    "sent" => Role::Sent,
                    "archive" => Role::Archive,
                    "junk" => Role::Junk,
                    "trash" => Role::Trash,
                    _ => Role::Inbox,
                };
                if self.state.borrow().with_role(role).is_empty() {
                    return Err("no folder of that kind".into());
                }
                self.go(Place::Unified(role));
            }
            "folders" => {
                let on = self.outer.shows_sidebar();
                self.outer.set_show_sidebar(!on);
            }
            "filter" => {
                let text = p.arg(0).unwrap_or("");
                self.search.set_text(text);
                self.search.grab_focus();
                self.search.set_position(-1);
            }
            "search" => match p.arg(0).map(str::trim).filter(|q| !q.is_empty()) {
                Some(q) => self.run_search(q)?,
                None => crate::filters::open(self),
            },
            "search-helper" => crate::filters::open(self),
            "search-box" => {
                let text = self.search.text().to_string();
                if text.trim().is_empty() {
                    crate::filters::open(self);
                } else {
                    self.search.set_text("");
                    self.run_search(&text)?;
                }
            }
            "index" => self.index()?,
            "threads" => {
                let now = self.state.borrow().threaded;
                let on = match p.arg(0) {
                    Some("on") => true,
                    Some("off") => false,
                    _ => !now,
                };
                {
                    let mut s = self.state.borrow_mut();
                    s.threaded = on;
                    s.arrange();
                }
                self.fill_list();
                self.show_message();
                self.toast(if on { "Conversations" } else { "One by one" });
            }
            "compose" => {
                let mut d = self.new_draft();
                if let Some(to) = p.arg(0) {
                    d.to = to.to_owned();
                }
                crate::compose::open(self, d, None);
            }
            "reply" | "replyall" => {
                let o = self.opened()?;
                let (me, width, top) = {
                    let s = self.state.borrow();
                    (s.opts.me.clone(), s.opts.wrap, s.opts.reply_top)
                };
                let d = compose::reply(&o, &me, p.spec == "replyall", width);
                crate::compose::open_reply(self, d, top);
            }
            "forward" => {
                let o = self.opened()?;
                let me = self.state.borrow().opts.me.clone();
                let mut d = compose::forward(&o, &me);
                // the attachments go along
                let dir = crate::compose::scratch_dir("forward");
                for a in o.attachments.iter().filter(|a| !a.calendar || !a.inline) {
                    if let Ok(bytes) = read::part_bytes(&o.path, a.part) {
                        let f = dir.join(read::safe_name(&a.name));
                        if std::fs::write(&f, bytes).is_ok() {
                            d.attachments.push(f);
                        }
                    }
                }
                crate::compose::open(self, d, None);
            }
            "edit" => {
                if !self.state.borrow().in_drafts() {
                    return Err("only a draft can be edited: reply or forward instead".into());
                }
                let path = self.selected_path()?;
                let o = read::open(&path).map_err(|e| format!("{e:#}"))?;
                let mut d = Draft::from_saved(&o);
                let dir = crate::compose::scratch_dir("draft");
                for a in &o.attachments {
                    if let Ok(bytes) = read::part_bytes(&o.path, a.part) {
                        let f = dir.join(read::safe_name(&a.name));
                        if std::fs::write(&f, bytes).is_ok() {
                            d.attachments.push(f);
                        }
                    }
                }
                crate::compose::open(self, d, Some(path));
            }
            "archive" => self.move_selected(Dest::Role(Role::Archive), "Archived")?,
            "junk" => self.move_selected(Dest::Role(Role::Junk), "Moved to junk")?,
            "delete" => self.delete()?,
            "move" => {
                let name = p.arg(0).ok_or("move to which folder?")?;
                let dest = {
                    let s = self.state.borrow();
                    s.find_folder(name)
                        .map(|f| f.path.clone())
                        .ok_or(format!("no folder “{name}”"))?
                };
                self.move_selected(Dest::Folder(dest), &format!("Moved to {name}"))?;
            }
            "mark" => self.mark(p.arg(0))?,
            "flag" => {
                let paths = self.state.borrow().targets();
                if paths.is_empty() {
                    return Err("no message is selected".into());
                }
                let on = !store::flags_of(&paths[0]).flagged;
                self.change_flags(&paths, |f| f.flagged = on)?;
            }
            "pick" => self.pick(p.arg(0).unwrap_or("toggle"))?,
            "fold" => self.fold(p.arg(0).unwrap_or("toggle"))?,
            "sort" => self.sort(p.arg(0).unwrap_or("date"), p.arg(1))?,
            "undo" => self.undo()?,
            "links" => self.show_links()?,
            "open-link" => {
                let n: usize = p
                    .arg(0)
                    .and_then(|a| a.trim().parse().ok())
                    .ok_or("which link? A number, see :links")?;
                self.confirm_link(n)?;
            }
            "attachment" => self.attachment(p.arg(0).unwrap_or("open"), p.arg(1))?,
            "invite" => self.open_invite()?,
            "unsubscribe" => self.unsubscribe()?,
            "rsvp" => self.rsvp(p.arg(0).unwrap_or("accept"))?,
            "contact" => self.open_contact()?,
            "view" => {
                let html = match p.arg(0) {
                    Some("html") => true,
                    Some("text") => false,
                    _ => !self.reader.showing_html(),
                };
                self.reader.prefer_html.set(html);
                self.show_message();
            }
            "headers" => self.show_headers()?,
            "source" => self.show_source()?,
            "mkdir" => {
                let name = p.arg(0).ok_or("what should it be called?")?;
                let root = self.state.borrow().root.clone();
                store::create_folder(&root, name).map_err(|e| format!("{e:#}"))?;
                self.reload();
                self.toast(&format!("Made {name}"));
            }
            "export" => self.export()?,
            "reload" => {
                let again = match &self.state.borrow().place {
                    Place::Search(q) => Some(q.clone()),
                    _ => None,
                };
                match again {
                    Some(q) => self.run_search(&q)?,
                    None => {
                        self.reload();
                        self.toast("Reloaded");
                    }
                }
            }
            "wake" => self.wake()?,
            "unsend" => self.unsend()?,
            "retry" => {
                let n = rical_mail_queue::queue::Queue::open(&self.state.borrow().opts.queue)
                    .and_then(|q| q.retry(None))
                    .map_err(|e| format!("{e:#}"))?;
                self.toast(&format!("{n} back in the queue"));
                self.reload();
                if n > 0 {
                    self.wake()?;
                }
            }
            "settings" => crate::prefs::show(self),
            "about" => self.about(),
            "help" => {
                rical_ui::help::dialog(&cmds::HELP, &self.binds, "list")
                    .present(Some(&self.window));
            }
            "tab" => self.tab_command(p.arg(0).unwrap_or("next"))?,
            "tab-close" => match self.current_compose() {
                Some(c) => c.execute("discard")?,
                None => return Err("the mail tab stays; close the window with :quit".into()),
            },
            "quit" => {
                if self.unsaved_tab().is_some() {
                    self.warn_unsaved();
                } else {
                    self.app.quit();
                }
            }
            other => return Err(format!("“{other}” is for the window you write in")),
        }
        Ok(())
    }

    pub fn new_draft(&self) -> Draft {
        let s = self.state.borrow();
        Draft {
            from: s.opts.me.first().map(|m| m.display()).unwrap_or_default(),
            ..Draft::default()
        }
    }

    fn go(self: &Rc<Self>, place: Place) {
        {
            let mut s = self.state.borrow_mut();
            s.place = place;
            s.query.clear();
            s.selected = None;
            s.opened = None;
            s.reload_folders();
            // empty until it is read, so the old place's messages do not show
            s.messages.clear();
            s.outbox_notes.clear();
            s.arrange();
        }
        self.syncing.set(true);
        self.search.set_text("");
        self.syncing.set(false);
        self.refresh();
        self.split.set_show_content(false);
        self.focus_view();
        self.load();
    }

    fn select(&self, which: &str) {
        let (n, at, unread): (usize, Option<usize>, Vec<bool>) = {
            let s = self.state.borrow();
            (
                s.shown.len(),
                s.position(),
                (0..s.shown.len())
                    .map(|i| s.message_at(i).is_some_and(|m| m.unread()))
                    .collect(),
            )
        };
        if n == 0 {
            return;
        }
        let target = match which {
            "first" => Some(0),
            "last" => Some(n - 1),
            "prev" => Some(at.map_or(0, |a| a.saturating_sub(1))),
            "next-unread" => (at.map_or(0, |a| a + 1)..n).find(|&i| unread[i]),
            "prev-unread" => (0..at.unwrap_or(0)).rev().find(|&i| unread[i]),
            _ => Some(at.map_or(0, |a| (a + 1).min(n - 1))),
        };
        match target {
            Some(t) => self.select_position(t),
            None => self.toast("No more unread"),
        }
    }

    fn back(self: &Rc<Self>) {
        if !self.state.borrow().picked.is_empty() {
            self.state.borrow_mut().picked.clear();
            self.fill_list();
            return;
        }
        if !self.state.borrow().query.is_empty() || !self.search.text().is_empty() {
            self.search.set_text("");
            self.search_now();
            self.focus_view();
        } else if self.split.shows_content() {
            self.split.set_show_content(false);
            self.focus_view();
        } else if matches!(self.state.borrow().place, Place::Search(_)) {
            // out of the results: back to where the search began
            let before = self.state.borrow_mut().before_search.take();
            let place = before.unwrap_or_else(|| {
                let s = self.state.borrow();
                s.folder_with_role(Role::Inbox)
                    .or(s.folders.first())
                    .map_or(Place::Outbox, |f| Place::Folder(f.path.clone()))
            });
            self.go(place);
        } else {
            self.focus_view();
        }
    }

    /// Mark the picked messages (else the selected one) read or unread.
    fn mark(self: &Rc<Self>, how: Option<&str>) -> Result<(), String> {
        if self.state.borrow().place == Place::Outbox {
            return Ok(());
        }
        let paths = self.state.borrow().targets();
        if paths.is_empty() {
            return Err("no message is selected".into());
        }
        // one answer for all: by the first
        let first_unread = self
            .state
            .borrow()
            .messages
            .iter()
            .find(|m| m.path == paths[0])
            .is_some_and(|m| m.unread());
        let want_seen = match how.unwrap_or("toggle") {
            "read" => true,
            "unread" => false,
            _ => first_unread,
        };
        self.change_flags(&paths, |f| f.seen = want_seen)
    }

    /// Change the flags of messages, as one undoable step.
    fn change_flags(
        self: &Rc<Self>,
        paths: &[std::path::PathBuf],
        change: impl Fn(&mut Flags),
    ) -> Result<(), String> {
        self.quiet();
        let mut done = Vec::new();
        for path in paths {
            let unread = self
                .state
                .borrow()
                .messages
                .iter()
                .find(|m| &m.path == path)
                .is_some_and(|m| m.unread());
            let before = store::flags_of(path);
            let mut now = before;
            // a message still in new/ has no flags yet but is unread
            now.seen = before.seen && !unread;
            let was = now;
            change(&mut now);
            if now == was && now.seen == !unread {
                continue;
            }
            let new = store::set_flags(path, now).map_err(|e| format!("{e:#}"))?;
            let mut before = before;
            before.seen = !unread && before.seen;
            self.state.borrow_mut().renamed(path, new.clone(), now);
            done.push(Undo::Flags { now: new, before });
        }
        if done.is_empty() {
            return Ok(());
        }
        let single_at = (done.len() == 1)
            .then(|| self.state.borrow().position())
            .flatten();
        {
            let mut s = self.state.borrow_mut();
            s.push_undo(if done.len() == 1 {
                done.remove(0)
            } else {
                Undo::Batch(done)
            });
            s.reload_folders();
        }
        match single_at {
            Some(p) => self.rebind(p),
            None => self.fill_list(),
        }
        self.fill_folders();
        Ok(())
    }

    /// Move the picked messages (else the selected one) to a folder.
    fn move_selected(self: &Rc<Self>, dest: Dest, said: &str) -> Result<(), String> {
        if self.state.borrow().place == Place::Outbox {
            return Err("these are waiting to be sent: delete removes one".into());
        }
        let paths = self.state.borrow().targets();
        if paths.is_empty() {
            return Err("no message is selected".into());
        }
        self.quiet();
        let mut done = Vec::new();
        for path in &paths {
            let from = path
                .parent()
                .and_then(Path::parent)
                .map(Path::to_owned)
                .ok_or("not in a folder")?;
            // each to the right folder of its own account
            let to = match &dest {
                Dest::Folder(p) => p.clone(),
                Dest::Role(r) => self.state.borrow().folder_for(path, *r),
            };
            let now = store::move_to(path, &to).map_err(|e| format!("{e:#}"))?;
            done.push(Undo::Moved { now, back_to: from });
        }
        let n = paths.len();
        {
            let mut s = self.state.borrow_mut();
            s.push_undo(if n == 1 {
                done.remove(0)
            } else {
                Undo::Batch(done)
            });
            for p in &paths {
                s.removed(p);
            }
            s.picked.clear();
            s.reload_folders();
        }
        self.refresh();
        self.toast(&format!(
            "{said}{}. U to undo",
            if n > 1 {
                format!(" ({n} messages)")
            } else {
                String::new()
            }
        ));
        Ok(())
    }

    /// Pick messages to act on together: toggle this one (and move on),
    /// pick everything shown, or nothing.
    fn pick(self: &Rc<Self>, how: &str) -> Result<(), String> {
        match how {
            "all" => {
                let mut s = self.state.borrow_mut();
                let all: Vec<_> = (0..s.shown.len())
                    .filter_map(|n| s.message_at(n).map(|m| m.path.clone()))
                    .collect();
                s.picked = all.into_iter().collect();
            }
            "none" => self.state.borrow_mut().picked.clear(),
            "thread" => {
                let path = self.selected_path()?;
                {
                    let mut s = self.state.borrow_mut();
                    let all = s.thread_of(&path);
                    // already all picked: let them go
                    if all.iter().all(|p| s.picked.contains(p)) {
                        for p in &all {
                            s.picked.remove(p);
                        }
                    } else {
                        s.picked.extend(all);
                    }
                }
                self.fill_list();
                self.update_picked_title();
                return Ok(());
            }
            _ => {
                let path = self.selected_path()?;
                {
                    let mut s = self.state.borrow_mut();
                    if !s.picked.remove(&path) {
                        s.picked.insert(path);
                    }
                }
                let at = self.state.borrow().position();
                if let Some(p) = at {
                    self.rebind(p);
                }
                self.select("next");
                self.update_picked_title();
                return Ok(());
            }
        }
        self.fill_list();
        Ok(())
    }

    fn delete(self: &Rc<Self>) -> Result<(), String> {
        let (outbox, in_trash) = {
            let s = self.state.borrow();
            (s.place == Place::Outbox, s.in_trash())
        };
        let path = self.selected_path()?;
        if outbox {
            if self
                .state
                .borrow()
                .outbox_notes
                .get(&path)
                .is_some_and(|n| n.starts_with("sending"))
            {
                return Err("it is being sent right now".into());
            }
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
            self.state.borrow_mut().removed(&path);
            self.refresh();
            self.toast("Taken out of the queue; it will not be sent");
            return Ok(());
        }
        if !in_trash {
            return self.move_selected(Dest::Role(Role::Trash), "Deleted");
        }
        let dialog = adw::AlertDialog::builder()
            .heading("Delete for good?")
            .body("It is in the trash already; this cannot be undone.")
            .build();
        dialog.add_responses(&[("cancel", "_Cancel"), ("delete", "_Delete")]);
        dialog.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("delete"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, r| {
            let Some(w) = weak.upgrade() else { return };
            if r == "delete" {
                w.quiet();
                match store::remove(&path) {
                    Ok(()) => {
                        w.state.borrow_mut().removed(&path);
                        w.state.borrow_mut().reload_folders();
                        w.refresh();
                        w.toast("Deleted for good");
                    }
                    Err(e) => w.toast(&format!("{e:#}")),
                }
            }
            w.focus_view();
        });
        dialog.present(Some(&self.window));
        Ok(())
    }

    fn undo(self: &Rc<Self>) -> Result<(), String> {
        let u = self
            .state
            .borrow_mut()
            .undo
            .pop()
            .ok_or("nothing to undo")?;
        self.quiet();
        let moved = Self::undo_step(u)?;
        // a move needs the folders read again; flags only the rows
        self.reload();
        if moved {
            self.toast("Moved back");
        }
        Ok(())
    }

    /// Undo one step (or each of a batch). Whether anything was moved.
    fn undo_step(u: Undo) -> Result<bool, String> {
        match u {
            Undo::Moved { now, back_to } => {
                store::move_to(&now, &back_to).map_err(|e| format!("{e:#}"))?;
                Ok(true)
            }
            Undo::Flags { now, before } => {
                store::set_flags(&now, before).map_err(|e| format!("{e:#}"))?;
                Ok(false)
            }
            Undo::Batch(all) => {
                let mut moved = false;
                for u in all.into_iter().rev() {
                    moved |= Self::undo_step(u)?;
                }
                Ok(moved)
            }
        }
    }

    fn show_links(self: &Rc<Self>) -> Result<(), String> {
        let o = self.opened()?;
        if o.body.links.is_empty() {
            return Err("there are no links in this message".into());
        }
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["boxed-list"])
            .build();
        for (i, l) in o.body.links.iter().enumerate() {
            let words = if l.text == l.url {
                String::new()
            } else {
                l.text.clone()
            };
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&format!(
                    "{}  {}",
                    i + 1,
                    if words.is_empty() {
                        l.url.clone()
                    } else {
                        words
                    }
                )))
                .subtitle(glib::markup_escape_text(&l.url))
                .activatable(true)
                .build();
            row.set_title_lines(1);
            row.set_subtitle_lines(1);
            if l.misleading {
                row.add_suffix(&gtk::Image::from_icon_name("dialog-warning-symbolic"));
            }
            row.set_widget_name(&(i + 1).to_string());
            list.append(&row);
        }
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .min_content_height(120)
            .max_content_height(420)
            .propagate_natural_height(true)
            .build();
        let dialog = adw::Dialog::builder()
            .title("Links")
            .content_width(560)
            .child(&{
                let v = adw::ToolbarView::new();
                v.add_top_bar(&adw::HeaderBar::new());
                let b = gtk::Box::builder()
                    .margin_start(12)
                    .margin_end(12)
                    .margin_bottom(12)
                    .build();
                b.append(&scroll);
                v.set_content(Some(&b));
                v
            })
            .build();
        let weak = Rc::downgrade(self);
        let d = dialog.clone();
        list.connect_row_activated(move |_, row| {
            d.close();
            if let Some(w) = weak.upgrade() {
                let _ = w.execute(&format!("open-link {}", row.widget_name()));
            }
        });
        // number keys: 1–9 open that link
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        let d = dialog.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if let Some(c) = key.to_unicode()
                && let Some(n) = c.to_digit(10)
                && n >= 1
                && let Some(w) = weak.upgrade()
            {
                d.close();
                let _ = w.execute(&format!("open-link {n}"));
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        dialog.add_controller(keys);
        dialog.present(Some(&self.window));
        Ok(())
    }

    /// Show where a link goes, and open it if that is wanted.
    fn confirm_link(self: &Rc<Self>, n: usize) -> Result<(), String> {
        let o = self.opened()?;
        let l = o
            .body
            .links
            .get(n.wrapping_sub(1))
            .ok_or(format!("this message has no link {n}"))?
            .clone();
        self.confirm_link_of(l)
    }

    /// A link clicked in the HTML view: by its address, with what the text
    /// view knows of its words (to say if they mislead).
    pub fn confirm_url(self: &Rc<Self>, url: String) -> Result<(), String> {
        let known = self
            .opened()
            .ok()
            .and_then(|o| o.body.links.iter().find(|l| l.url == url).cloned());
        let l = known.unwrap_or(rical_maildir::html::Link {
            text: url.clone(),
            url,
            range: 0..0,
            misleading: false,
        });
        self.confirm_link_of(l)
    }

    fn confirm_link_of(self: &Rc<Self>, l: rical_maildir::html::Link) -> Result<(), String> {
        let host = l
            .url
            .split_once("://")
            .map(|(_, r)| r.split(['/', '?', '#']).next().unwrap_or(""))
            .unwrap_or("")
            .to_owned();
        let mut body = String::new();
        if l.url.starts_with("mailto:") {
            body += &format!("Write to {}", l.url.trim_start_matches("mailto:"));
        } else {
            body += &format!("This goes to {host}\n\n{}", l.url);
        }
        if l.misleading {
            body += &format!(
                "\n\nThe message shows it as “{}”, which is somewhere else.",
                l.text
            );
        }
        let dialog = adw::AlertDialog::builder()
            .heading("Open this link?")
            .body(body)
            .build();
        dialog.add_responses(&[
            ("cancel", "_Cancel"),
            ("copy", "Co_py Address"),
            ("open", "_Open"),
        ]);
        dialog.set_response_appearance(
            "open",
            if l.misleading {
                adw::ResponseAppearance::Destructive
            } else {
                adw::ResponseAppearance::Suggested
            },
        );
        dialog.set_default_response(Some(if l.misleading { "cancel" } else { "open" }));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, r| {
            let Some(w) = weak.upgrade() else { return };
            match r {
                "open" => {
                    if let Some(rest) = l.url.strip_prefix("mailto:") {
                        let to = rest.split('?').next().unwrap_or("").to_owned();
                        let mut d = w.new_draft();
                        d.to = to;
                        crate::compose::open(&w, d, None);
                    } else {
                        open_uri(Some(w.window.upcast_ref()), &l.url);
                    }
                }
                "copy" => {
                    w.window.clipboard().set_text(&l.url);
                    w.toast("Copied the address");
                }
                _ => {}
            }
        });
        dialog.present(Some(&self.window));
        Ok(())
    }

    /// Where attachments are put to be opened.
    fn attachment_dir(&self, path: &Path) -> std::path::PathBuf {
        let name = path.file_name().map_or("message".to_owned(), |n| {
            read::safe_name(&n.to_string_lossy())
        });
        let dir = rical_ui::app::state_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("attachments")
            .join(name.split(':').next().unwrap_or("message"));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn attachment(self: &Rc<Self>, what: &str, which: Option<&str>) -> Result<(), String> {
        let o = self.opened()?;
        if o.attachments.is_empty() {
            return Err("this message has no attachments".into());
        }
        let pick = |w: &str| -> Result<Vec<usize>, String> {
            let w = w.trim();
            if w.is_empty() || w == "all" {
                return Ok((0..o.attachments.len()).collect());
            }
            if let Ok(n) = w.parse::<usize>() {
                return (1..=o.attachments.len())
                    .contains(&n)
                    .then(|| vec![n - 1])
                    .ok_or(format!("there is no attachment {n}"));
            }
            let found: Vec<usize> = o
                .attachments
                .iter()
                .enumerate()
                .filter(|(_, a)| a.name.to_lowercase().contains(&w.to_lowercase()))
                .map(|(i, _)| i)
                .collect();
            if found.is_empty() {
                Err(format!("no attachment like “{w}”"))
            } else {
                Ok(found)
            }
        };
        let chosen = match (what, which) {
            ("all", _) => pick("all")?,
            (_, Some(w)) => pick(w)?,
            (_, None) => pick("1")?,
        };
        match what {
            "open" => {
                let dir = self.attachment_dir(&o.path);
                for i in chosen {
                    let a = &o.attachments[i];
                    let bytes = read::part_bytes(&o.path, a.part).map_err(|e| format!("{e:#}"))?;
                    let f = dir.join(read::safe_name(&a.name));
                    std::fs::write(&f, bytes).map_err(|e| e.to_string())?;
                    let launcher = gtk::FileLauncher::new(Some(&gio::File::for_path(&f)));
                    launcher.launch(Some(&self.window), None::<&gio::Cancellable>, |r| {
                        if let Err(e) = r {
                            log::warn!("could not open the attachment: {e}");
                        }
                    });
                }
            }
            _ => {
                let dialog = gtk::FileDialog::builder().title("Save to folder").build();
                let weak = Rc::downgrade(self);
                let path = o.path.clone();
                let items: Vec<(u32, String)> = chosen
                    .iter()
                    .map(|&i| {
                        (
                            o.attachments[i].part,
                            read::safe_name(&o.attachments[i].name),
                        )
                    })
                    .collect();
                dialog.select_folder(Some(&self.window), None::<&gio::Cancellable>, move |r| {
                    let (Some(w), Ok(dir)) = (weak.upgrade(), r) else {
                        return;
                    };
                    let Some(dir) = dir.path() else { return };
                    let mut saved = 0;
                    for (part, name) in &items {
                        let Ok(bytes) = read::part_bytes(&path, *part) else {
                            continue;
                        };
                        // never overwrite
                        let mut f = dir.join(name);
                        let mut n = 1;
                        while f.exists() {
                            n += 1;
                            f = dir.join(format!("{n}-{name}"));
                        }
                        if std::fs::write(&f, bytes).is_ok() {
                            saved += 1;
                        }
                    }
                    w.toast(&format!("Saved {saved} to {}", dir.display()));
                });
            }
        }
        Ok(())
    }

    fn open_invite(self: &Rc<Self>) -> Result<(), String> {
        let o = self.opened()?;
        if o.invites.is_empty() {
            return Err("this message has no invitation".into());
        }
        let ics = read::calendar_text(&o.path).map_err(|e| format!("{e:#}"))?;
        let dir = self.attachment_dir(&o.path);
        let f = dir.join("invitation.ics");
        std::fs::write(&f, ics).map_err(|e| e.to_string())?;
        let cmd = self.state.borrow().opts.calendar.clone();
        spawn_with(&cmd, &f)?;
        self.toast("Opening the invitation in Calendar");
        Ok(())
    }

    fn open_contact(self: &Rc<Self>) -> Result<(), String> {
        let o = self.opened()?;
        let who = o.from.first().ok_or("the message has no sender")?.clone();
        let cmd = self.state.borrow().opts.contacts_app.clone();
        // in the address books already: open that card
        let found = {
            let s = self.state.borrow();
            s.books.as_ref().and_then(|books| {
                books
                    .cards()
                    .find(|(_, _, c)| {
                        c.emails()
                            .iter()
                            .any(|e| e.value.trim().eq_ignore_ascii_case(&who.address))
                    })
                    .map(|(id, _, _)| id.path)
            })
        };
        let file = match found {
            Some(p) => p,
            None => {
                // new to us: a card with what the message says, for the
                // contacts app to add (it asks first)
                let dir = rical_ui::app::state_dir()
                    .unwrap_or_else(std::env::temp_dir)
                    .join("new-contacts");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let name = if who.name.is_empty() {
                    who.address.clone()
                } else {
                    who.name.clone()
                };
                let card = format!(
                    "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:{}\r\nFN:{}\r\nEMAIL:{}\r\nEND:VCARD\r\n",
                    store::unique_name().replace(['.', ':'], "-"),
                    name.replace([',', ';', '\n'], " "),
                    who.address
                );
                let f = dir.join(format!("{}.vcf", read::safe_name(&who.address)));
                std::fs::write(&f, card).map_err(|e| e.to_string())?;
                f
            }
        };
        spawn_with(&cmd, &file)?;
        Ok(())
    }

    fn text_dialog(self: &Rc<Self>, title: &str, text: &str) {
        let view = gtk::TextView::builder()
            .editable(false)
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(10)
            .right_margin(10)
            .top_margin(8)
            .bottom_margin(8)
            .build();
        view.buffer().set_text(text);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&view)
            .vexpand(true)
            .build();
        let tv = adw::ToolbarView::new();
        tv.add_top_bar(&adw::HeaderBar::new());
        tv.set_content(Some(&scroll));
        let dialog = adw::Dialog::builder()
            .title(title)
            .content_width(820)
            .content_height(620)
            .child(&tv)
            .build();
        dialog.present(Some(&self.window));
    }

    fn show_headers(self: &Rc<Self>) -> Result<(), String> {
        let o = self.opened()?;
        let text = o
            .headers
            .iter()
            .map(|(k, v)| format!("{k}: {v}"))
            .collect::<Vec<_>>()
            .join("\n");
        self.text_dialog("Headers", &text);
        Ok(())
    }

    fn show_source(self: &Rc<Self>) -> Result<(), String> {
        let path = self.selected_path()?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        let cut = bytes.len().min(300_000);
        let mut text = String::from_utf8_lossy(&bytes[..cut]).into_owned();
        if cut < bytes.len() {
            text += "\n… (the rest is not shown)";
        }
        self.text_dialog(&path.display().to_string(), &text);
        Ok(())
    }

    /// Run a command in the background and say how it went.
    pub fn run_shell(self: &Rc<Self>, command: &str, doing: &str, done: &str, then_reload: bool) {
        let proc = gio::Subprocess::newv(
            &[
                std::ffi::OsStr::new("sh"),
                std::ffi::OsStr::new("-c"),
                std::ffi::OsStr::new(command),
            ],
            gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_MERGE,
        );
        let proc = match proc {
            Ok(p) => p,
            Err(e) => {
                self.toast(&format!("{command}: {e}"));
                return;
            }
        };
        self.toast(doing);
        let (weak, done, command) = (Rc::downgrade(self), done.to_owned(), command.to_owned());
        let finished = proc.clone();
        proc.communicate_utf8_async(None::<String>, None::<&gio::Cancellable>, move |r| {
            let Some(w) = weak.upgrade() else { return };
            w.syncing.set(false);
            match r {
                Ok((out, _)) => {
                    let ok = finished.is_successful();
                    if ok {
                        w.toast(&done);
                    } else {
                        let last = out
                            .as_deref()
                            .unwrap_or("")
                            .lines()
                            .rev()
                            .find(|l| !l.trim().is_empty())
                            .unwrap_or("it failed")
                            .to_owned();
                        log::warn!("{command}: {}", out.as_deref().unwrap_or(""));
                        w.toast(&format!("{command}: {last}"));
                    }
                    if then_reload {
                        w.reload();
                    }
                }
                Err(e) => w.toast(&format!("{command}: {e}")),
            }
        });
    }

    pub fn wake(self: &Rc<Self>) -> Result<(), String> {
        let cmd = self
            .state
            .borrow()
            .opts
            .send
            .clone()
            .ok_or("set `send = rical-mail-queue run --once` in the settings to send from here; otherwise your rical-mail-queue service sends")?;
        self.run_shell(&cmd, "Sending…", "Sent what was queued", true);
        Ok(())
    }

    fn about(&self) {
        let s = self.state.borrow();
        let details = format!(
            "mail: {}\nfolders: {}\nsend queue: {}\ncontacts: {}\n",
            s.root.display(),
            s.folders.len(),
            s.opts.queue.display(),
            s.books
                .as_ref()
                .map_or("none".to_owned(), |b| format!("{} cards", b.len())),
        );
        rical_ui::util::about(
            &self.window,
            "A keyboard-first mail client for Maildir folders. Plain text first, and private: \
             nothing in a message is ever loaded, HTML is shown as text, and no read receipt is sent.",
            &details,
        );
    }
}

impl palette::Host for Window {
    fn anchor(&self) -> gtk::Widget {
        self.search.clone().upcast()
    }

    fn window(&self) -> adw::ApplicationWindow {
        self.window.clone()
    }

    fn complete(&self, line: &str) -> rical_ui::command::Completion {
        let state = self.state.borrow();
        REGISTRY.complete(line, cmds::LIST, &StateValues(&state))
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
        "command, e.g. folder Sent, move Archive, search from:ann unread, compose bo@example.com"
    }
}

#[allow(unused_imports)]
use Role as _;

impl Window {
    /// What follows queueing a message: say so, with Undo while it is held
    /// back, and send at once (after the hold) if a `send` command is set.
    pub fn after_queued(self: &Rc<Self>, delay: u32) {
        let toast = adw::Toast::builder()
            .priority(adw::ToastPriority::High)
            .build();
        if delay > 0 {
            toast.set_title(&format!("Sending in {delay} seconds"));
            toast.set_button_label(Some("Undo"));
            toast.set_action_name(Some("win.run"));
            toast.set_action_target_value(Some(&"unsend".to_variant()));
            toast.set_timeout(delay.min(30));
        } else {
            toast.set_title("Queued to be sent");
        }
        self.toasts.add_toast(toast);
        if self.state.borrow().opts.send.is_some() {
            let weak = Rc::downgrade(self);
            glib::timeout_add_local_once(
                std::time::Duration::from_secs(u64::from(delay) + 1),
                move || {
                    if let Some(w) = weak.upgrade() {
                        let _ = w.wake();
                    }
                },
            );
        }
    }

    /// Take back what was just sent, and open it again to carry on.
    fn unsend(self: &Rc<Self>) -> Result<(), String> {
        let (id, draft) = self
            .state
            .borrow_mut()
            .last_sent
            .take()
            .ok_or("nothing was just sent")?;
        let queue = rical_mail_queue::queue::Queue::open(&self.state.borrow().opts.queue)
            .map_err(|e| format!("{e:#}"))?;
        if queue.cancel(&id).map_err(|e| format!("{e:#}"))? {
            self.quiet();
            self.reload();
            crate::compose::open(self, draft, None);
            Ok(())
        } else {
            Err("too late: it is already on its way".into())
        }
    }
}

impl Window {
    /// Answer an invitation by mail: a message to the organiser with a
    /// `text/calendar; method=REPLY`, opened to add a note and send.
    fn rsvp(self: &Rc<Self>, how: &str) -> Result<(), String> {
        let o = self.opened()?;
        let ics = read::calendar_text(&o.path).map_err(|e| format!("{e:#}"))?;
        let partstat = match how {
            "accept" => "ACCEPTED",
            "maybe" => "TENTATIVE",
            "decline" => "DECLINED",
            other => return Err(format!("accept, maybe or decline, not “{other}”")),
        };
        // the address that was invited, else the first of mine
        let me = {
            let s = self.state.borrow();
            let lower = ics.to_lowercase();
            s.opts
                .me
                .iter()
                .find(|m| lower.contains(&format!("mailto:{}", m.address.to_lowercase())))
                .or(s.opts.me.first())
                .cloned()
                .ok_or("set `me = Name <address>` in the settings first")?
        };
        let r = rical_maildir::invite::reply_to(
            &ics,
            &me.address,
            &me.name,
            partstat,
            chrono::Utc::now(),
        )
        .ok_or("this invitation names no organiser to answer")?;
        let mut references = o.references.clone();
        references.extend(o.message_id.clone());
        let draft = Draft {
            from: me.display(),
            to: r.to,
            subject: r.subject,
            body: format!("{}\n", r.words),
            in_reply_to: o.message_id.clone(),
            references,
            answering: Some(o.path.clone()),
            calendar: Some(("REPLY".to_owned(), r.ics)),
            ..Draft::default()
        };
        crate::compose::open(self, draft, None);
        Ok(())
    }
}

impl Window {
    /// Leave a list by the way its message offers: write the mail it asks
    /// for, or open its page, after saying where that goes.
    fn unsubscribe(self: &Rc<Self>) -> Result<(), String> {
        let o = self.opened()?;
        let target = o
            .unsubscribe
            .iter()
            .find(|u| u.to_lowercase().starts_with("mailto:"))
            .or(o.unsubscribe.first())
            .cloned()
            .ok_or("this message does not say how to unsubscribe")?;
        if let Some(rest) = target
            .strip_prefix("mailto:")
            .or_else(|| target.strip_prefix("MAILTO:"))
        {
            let mut d = crate::compose::parse_mailto(&format!("mailto:{rest}"));
            if d.subject.is_empty() {
                d.subject = "unsubscribe".to_owned();
            }
            if d.body.is_empty() {
                d.body = "unsubscribe\n".to_owned();
            }
            d.from = self.new_draft().from;
            crate::compose::open(self, d, None);
            return Ok(());
        }
        let host = target
            .split_once("://")
            .map(|(_, r)| r.split(['/', '?', '#']).next().unwrap_or(""))
            .unwrap_or("")
            .to_owned();
        let dialog = adw::AlertDialog::builder()
            .heading("Unsubscribe?")
            .body(format!(
                "This opens a page on {host} that unsubscribes you.\n\n{target}\n\n\
                 Visiting it tells them your address is read."
            ))
            .build();
        dialog.add_responses(&[("cancel", "_Cancel"), ("open", "_Open the Page")]);
        dialog.set_default_response(Some("cancel"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, r| {
            if r == "open"
                && let Some(w) = weak.upgrade()
            {
                open_uri(Some(w.window.upcast_ref()), &target);
            }
        });
        dialog.present(Some(&self.window));
        Ok(())
    }
}

impl Window {
    /// Save the message as it is on disk.
    fn export(self: &Rc<Self>) -> Result<(), String> {
        let path = self.selected_path()?;
        let name = self
            .state
            .borrow()
            .selected_summary()
            .map(|m| read::safe_name(&m.subject))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "message".to_owned());
        let dialog = gtk::FileDialog::builder()
            .title("Save the message")
            .initial_name(format!("{name}.eml"))
            .build();
        let weak = Rc::downgrade(self);
        dialog.save(Some(&self.window), None::<&gio::Cancellable>, move |r| {
            let (Some(w), Ok(file)) = (weak.upgrade(), r) else {
                return;
            };
            let Some(to) = file.path() else { return };
            match std::fs::copy(&path, &to) {
                Ok(_) => w.toast(&format!("Saved {}", to.display())),
                Err(e) => w.toast(&format!("{}: {e}", to.display())),
            }
        });
        Ok(())
    }
}

impl Window {
    /// Search everything: with the search tool if there is one, else the
    /// messages of every folder. The matches are shown as a list of their own.
    pub fn run_search(self: &Rc<Self>, query: &str) -> Result<(), String> {
        let backend = self.state.borrow().opts.search.clone();
        {
            let mut s = self.state.borrow_mut();
            if !matches!(s.place, Place::Search(_)) {
                s.before_search = Some(s.place.clone());
            }
        }
        let dir = crate::state::State::search_dir();
        let Some(cmd) = backend.command(query, &dir) else {
            self.show_results(query);
            return Ok(());
        };
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let proc = gio::Subprocess::newv(
            &[
                std::ffi::OsStr::new("sh"),
                std::ffi::OsStr::new("-c"),
                std::ffi::OsStr::new(&cmd),
            ],
            gio::SubprocessFlags::STDOUT_PIPE | gio::SubprocessFlags::STDERR_PIPE,
        )
        .map_err(|e| format!("{}: {e}", backend.name()))?;
        self.toast(&format!("Searching with {}…", backend.name()));
        let (weak, query, finished) = (Rc::downgrade(self), query.to_owned(), proc.clone());
        proc.communicate_utf8_async(None::<String>, None::<&gio::Cancellable>, move |r| {
            let Some(w) = weak.upgrade() else { return };
            let (out, err) = match r {
                Ok((o, e)) => (
                    o.unwrap_or_default().to_string(),
                    e.unwrap_or_default().to_string(),
                ),
                Err(e) => {
                    w.toast(&format!("{}: {e}", backend.name()));
                    return;
                }
            };
            let code = finished.exit_status();
            // mu says 4 for "found nothing"; anything else that failed is a problem
            let nothing = backend == Backend::Mu && code == 4;
            if code != 0 && !nothing {
                let last = err
                    .lines()
                    .rev()
                    .chain(out.lines().rev())
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("it failed");
                log::warn!("{}: {err}", backend.name());
                w.toast(&format!("{}: {last}", backend.name()));
                return;
            }
            if backend.gives() == Gives::Paths || nothing {
                let paths: Vec<std::path::PathBuf> = out
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(std::path::PathBuf::from)
                    .collect();
                if let Err(e) = store::write_links(&crate::state::State::search_dir(), &paths) {
                    w.toast(&format!("{e:#}"));
                    return;
                }
            }
            w.show_results(&query);
        });
        Ok(())
    }

    fn show_results(self: &Rc<Self>, query: &str) {
        self.go(Place::Search(query.to_owned()));
        let n = self.state.borrow().messages.len();
        self.toast(&if n == 0 {
            "Nothing found".to_owned()
        } else {
            format!("{n} found. Esc goes back")
        });
    }

    /// Update the search index: your `index` command, or the tool's own.
    fn index(self: &Rc<Self>) -> Result<(), String> {
        let (custom, backend) = {
            let s = self.state.borrow();
            (s.opts.index.clone(), s.opts.search.clone())
        };
        let cmd = custom
            .or_else(|| backend.index_command().map(str::to_owned))
            .ok_or("set `index = …` (for example mu index) in the settings")?;
        self.run_shell(
            &cmd,
            "Updating the search index…",
            "Search index updated",
            false,
        );
        Ok(())
    }
}

/// Where moved messages go: a folder, or each one's own account's folder of
/// a kind (archive, trash, junk).
pub enum Dest {
    Folder(std::path::PathBuf),
    Role(Role),
}

impl Window {
    /// Fold a conversation to its first message, or open it.
    fn fold(self: &Rc<Self>, how: &str) -> Result<(), String> {
        match how {
            "close-all" => self.state.borrow_mut().fold_all(true),
            "open-all" => self.state.borrow_mut().fold_all(false),
            h => {
                let done = self.state.borrow_mut().fold_selected(h);
                if done.is_none() {
                    return Err("this message is not part of a conversation".into());
                }
            }
        }
        self.fill_list();
        self.show_message();
        Ok(())
    }

    /// Order the list.
    fn sort(self: &Rc<Self>, by: &str, then: Option<&str>) -> Result<(), String> {
        use rical_maildir::thread::SortKey;
        {
            let mut s = self.state.borrow_mut();
            if by == "reverse" {
                s.sort_reverse = !s.sort_reverse;
            } else {
                let key = SortKey::parse(by)
                    .ok_or(format!("sort by date, from, subject or size, not “{by}”"))?;
                s.sort = key;
                s.sort_reverse = then == Some("reverse");
            }
            s.arrange();
            rical_ui::uistate::set(
                "sort",
                &format!(
                    "{}{}",
                    s.sort.name(),
                    if s.sort_reverse { " reverse" } else { "" }
                ),
            );
        }
        self.fill_list();
        self.show_message();
        let s = self.state.borrow();
        self.toast(&format!(
            "By {}{}",
            s.sort.name(),
            if s.sort_reverse { ", reversed" } else { "" }
        ));
        Ok(())
    }
}

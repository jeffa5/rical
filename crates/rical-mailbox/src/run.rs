//! Running commands: from the command line, key bindings and buttons.

use std::{path::Path, rc::Rc};

use adw::prelude::*;
use gtk::{gio, glib};
use rical_maildir::{
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

    fn opened(&self) -> Result<Rc<read::Opened>, String> {
        self.state
            .borrow()
            .opened
            .clone()
            .ok_or_else(|| "no message is open".to_owned())
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
                self.mark(Some("read"))?;
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
            "folders" => {
                let on = self.outer.shows_sidebar();
                self.outer.set_show_sidebar(!on);
            }
            "search" => {
                let text = p.arg(0).unwrap_or("");
                self.search.set_text(text);
                self.search.grab_focus();
                self.search.set_position(-1);
            }
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
            "archive" => {
                let dest = self.state.borrow().archive_folder();
                self.move_selected(&dest, "Archived")?;
            }
            "junk" => {
                let dest = self.state.borrow().junk_folder();
                self.move_selected(&dest, "Moved to junk")?;
            }
            "delete" => self.delete()?,
            "move" => {
                let name = p.arg(0).ok_or("move to which folder?")?;
                let dest = {
                    let s = self.state.borrow();
                    s.find_folder(name)
                        .map(|f| f.path.clone())
                        .ok_or(format!("no folder “{name}”"))?
                };
                self.move_selected(&dest, &format!("Moved to {name}"))?;
            }
            "mark" => self.mark(p.arg(0))?,
            "flag" => {
                let path = self.selected_path()?;
                let before = store::flags_of(&path);
                let mut now = before;
                now.flagged = !before.flagged;
                self.set_flags(&path, before, now)?;
            }
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
            "contact" => self.open_contact()?,
            "headers" => self.show_headers()?,
            "source" => self.show_source()?,
            "sync" => self.sync()?,
            "reload" => {
                self.reload();
                self.toast("Reloaded");
            }
            "wake" => self.wake()?,
            "retry" => {
                let n = rical_mail::queue::Queue::open(&self.state.borrow().opts.queue)
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
            "quit" => self.app.quit(),
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
            s.reload_messages();
        }
        self.syncing.set(true);
        self.search.set_text("");
        self.syncing.set(false);
        self.refresh();
        self.split.set_show_content(false);
        self.focus_view();
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
        if !self.state.borrow().query.is_empty() || !self.search.text().is_empty() {
            self.search.set_text("");
            self.search_now();
            self.focus_view();
        } else if self.split.shows_content() {
            self.split.set_show_content(false);
            self.focus_view();
        } else {
            self.focus_view();
        }
    }

    /// Mark the selected message read or unread (or the other way).
    fn mark(self: &Rc<Self>, how: Option<&str>) -> Result<(), String> {
        if self.state.borrow().place == Place::Outbox {
            return Ok(());
        }
        let path = self.selected_path()?;
        let before = store::flags_of(&path);
        // a message still in new/ has no flags yet but is unread
        let unread = self
            .state
            .borrow()
            .selected_summary()
            .is_some_and(|m| m.unread());
        let mut now = before;
        now.seen = match how.unwrap_or("toggle") {
            "read" => true,
            "unread" => false,
            _ => unread,
        };
        if now == before && !unread {
            return Ok(());
        }
        if unread == !now.seen && now == before {
            return Ok(());
        }
        self.set_flags(&path, before, now)
    }

    fn set_flags(self: &Rc<Self>, path: &Path, before: Flags, now: Flags) -> Result<(), String> {
        self.quiet();
        let new = store::set_flags(path, now).map_err(|e| format!("{e:#}"))?;
        let pos = {
            let mut s = self.state.borrow_mut();
            let pos = s.position();
            s.renamed(path, new.clone(), now);
            s.push_undo(Undo::Flags { now: new, before });
            s.reload_folders();
            pos
        };
        if let Some(p) = pos {
            self.rebind(p);
        }
        self.fill_folders();
        Ok(())
    }

    fn move_selected(self: &Rc<Self>, dest: &Path, said: &str) -> Result<(), String> {
        if self.state.borrow().place == Place::Outbox {
            return Err("these are waiting to be sent: delete removes one".into());
        }
        let path = self.selected_path()?;
        let from = path
            .parent()
            .and_then(Path::parent)
            .map(Path::to_owned)
            .ok_or("not in a folder")?;
        self.quiet();
        let now = store::move_to(&path, dest).map_err(|e| format!("{e:#}"))?;
        let pos = {
            let mut s = self.state.borrow_mut();
            s.push_undo(Undo::Moved { now, back_to: from });
            let pos = s.removed(&path);
            s.reload_folders();
            pos
        };
        let _ = pos;
        self.refresh();
        self.toast(&format!("{said}. U to undo"));
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
            let dest = self.state.borrow().trash_folder();
            return self.move_selected(&dest, "Deleted");
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
        match u {
            Undo::Moved { now, back_to } => {
                store::move_to(&now, &back_to).map_err(|e| format!("{e:#}"))?;
                self.reload();
                self.toast("Moved back");
            }
            Undo::Flags { now, before } => {
                let new = store::set_flags(&now, before).map_err(|e| format!("{e:#}"))?;
                let pos = {
                    let mut s = self.state.borrow_mut();
                    let pos = s.position();
                    s.renamed(&now, new, before);
                    s.reload_folders();
                    pos
                };
                if let Some(p) = pos {
                    self.rebind(p);
                }
                self.fill_folders();
            }
        }
        Ok(())
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

    fn sync(self: &Rc<Self>) -> Result<(), String> {
        let cmd = self.state.borrow().opts.sync.clone().ok_or(
            "set `sync = …` (for example mbsync -a) in the settings to fetch mail from here",
        )?;
        self.run_shell(&cmd, "Fetching mail…", "Fetched", true);
        Ok(())
    }

    pub fn wake(self: &Rc<Self>) -> Result<(), String> {
        let cmd = self
            .state
            .borrow()
            .opts
            .send
            .clone()
            .ok_or("set `send = rical-mail run --once` in the settings to send from here; otherwise your rical-mail service sends")?;
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

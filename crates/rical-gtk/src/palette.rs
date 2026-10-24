//! The command line: a popover with an entry and completions, opened with
//! `:` (or by a binding that ends without `<Enter>`).
//!
//! Typing filters the completions for the word under the cursor. Tab and
//! Shift+Tab (or Down/Up) cycle through them, filling the word in. Enter
//! runs the line; a lone matching completion is filled in first. Up on an
//! empty line recalls history. Errors are shown inline.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

use adw::prelude::*;
use gtk::{gdk, glib};

use super::{Window, run::REGISTRY};
use crate::{
    cmds::{self, StateValues},
    command::Completion,
};

const MAX_ROWS: usize = 40;

struct Palette {
    popover: gtk::Popover,
    entry: gtk::Entry,
    list: gtk::ListBox,
    error: gtk::Label,
    completion: RefCell<Completion>,
    /// Line before cycling through completions, and the current index.
    cycle: RefCell<Option<(String, usize)>>,
    /// Set while changing the entry ourselves.
    programmatic: Cell<bool>,
    history_pos: Cell<Option<usize>>,
}

/// Open the command line with `text` typed in.
pub fn open(window: &Rc<Window>, text: &str) {
    show(window, text, None);
}

fn show(window: &Rc<Window>, text: &str, problem: Option<&str>) {
    crate::details::close();
    let entry = gtk::Entry::builder()
        .hexpand(true)
        .placeholder_text("command, e.g. view week, goto fri, new --at 14:00 Lunch")
        .primary_icon_name("utilities-terminal-symbolic")
        .build();
    entry.update_property(&[gtk::accessible::Property::Label("Command")]);
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::Single)
        .css_classes(["navigation-sidebar"])
        .can_focus(false)
        .build();
    let scroller = gtk::ScrolledWindow::builder()
        .child(&list)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .propagate_natural_height(true)
        .max_content_height(380)
        .build();
    let error = gtk::Label::builder()
        .css_classes(["error"])
        .xalign(0.0)
        .wrap(true)
        .visible(false)
        .build();
    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .width_request(600)
        .build();
    content.append(&entry);
    content.append(&error);
    content.append(&scroller);

    let popover = gtk::Popover::builder()
        .child(&content)
        .position(gtk::PositionType::Bottom)
        .has_arrow(false)
        .build();
    popover.set_parent(&window.title);
    let w = Rc::downgrade(window);
    popover.connect_closed(move |p| {
        // back to the calendar, unless a command opened a dialog
        if let Some(w) = w.upgrade()
            && w.window.visible_dialog().is_none()
        {
            w.focus_view();
        }
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });

    let p = Rc::new(Palette {
        popover,
        entry,
        list,
        error,
        completion: Default::default(),
        cycle: Default::default(),
        programmatic: Cell::new(false),
        history_pos: Cell::new(None),
    });

    let (w, pal) = (Rc::downgrade(window), p.clone());
    p.entry.connect_changed(move |_| {
        if pal.programmatic.get() {
            return;
        }
        pal.cycle.replace(None);
        pal.history_pos.set(None);
        pal.error.set_visible(false);
        if let Some(w) = w.upgrade() {
            pal.update(&w);
        }
    });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (w, pal) = (Rc::downgrade(window), p.clone());
    keys.connect_key_pressed(move |_, key, _, mods| {
        let Some(w) = w.upgrade() else {
            return glib::Propagation::Proceed;
        };
        pal.key(&w, key, mods)
    });
    p.entry.add_controller(keys);

    let (w, pal) = (Rc::downgrade(window), p.clone());
    p.list.connect_row_activated(move |_, row| {
        if let Some(w) = w.upgrade() {
            pal.choose(row.index() as usize);
            pal.submit(&w);
        }
    });

    p.programmatic.set(true);
    p.entry.set_text(text);
    p.programmatic.set(false);
    p.update(window);
    if let Some(problem) = problem {
        p.error.set_label(problem);
        p.error.set_visible(true);
    }
    p.popover.popup();
    p.entry.grab_focus();
    p.entry.set_position(-1);
}

impl Palette {
    fn update(&self, w: &Rc<Window>) {
        let text = self.entry.text().to_string();
        let completion = {
            let state = w.state.borrow();
            REGISTRY.complete(&text, cmds::context(state.view), &StateValues(&state))
        };
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        for c in completion.candidates.iter().take(MAX_ROWS) {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            b.append(
                &gtk::Label::builder()
                    .label(&c.value)
                    .css_classes(["monospace"])
                    .xalign(0.0)
                    .build(),
            );
            let detail = if c.usage.is_empty() {
                c.help.clone()
            } else {
                format!("{} — {}", c.help, c.usage)
            };
            b.append(
                &gtk::Label::builder()
                    .label(&detail)
                    .css_classes(["dim-label", "caption"])
                    .xalign(1.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::Middle)
                    .build(),
            );
            self.list.append(
                &gtk::ListBoxRow::builder()
                    .child(&b)
                    .focusable(false)
                    .build(),
            );
        }
        self.list
            .parent()
            .and_then(|v| v.parent())
            .inspect(|s| s.set_visible(!completion.candidates.is_empty()));
        self.completion.replace(completion);
    }

    /// Fill in candidate `i` for the word being completed.
    fn choose(&self, i: usize) {
        let mut cycle = self.cycle.borrow_mut();
        let base = match &*cycle {
            Some((base, _)) => base.clone(),
            None => self.entry.text().to_string(),
        };
        let completion = self.completion.borrow();
        let Some(candidate) = completion.candidates.get(i) else {
            return;
        };
        let line = completion.apply(&base, candidate);
        *cycle = Some((base, i));
        self.programmatic.set(true);
        self.entry.set_text(&line);
        self.entry.set_position(-1);
        self.programmatic.set(false);
        if let Some(row) = self.list.row_at_index(i as i32) {
            self.list.select_row(Some(&row));
            // keep the chosen row in view
            row.grab_focus();
            self.entry.grab_focus_without_selecting();
        }
    }

    fn step(&self, w: &Rc<Window>, forward: bool) {
        let n = self.completion.borrow().candidates.len().min(MAX_ROWS);
        if n == 0 {
            return;
        }
        if n == 1 && self.cycle.borrow().is_none() {
            // a single match: fill it in and move on to the next word
            self.choose(0);
            self.cycle.replace(None);
            self.update(w);
            return;
        }
        let next = match *self.cycle.borrow() {
            Some((_, i)) if forward => (i + 1) % n,
            Some((_, i)) => (i + n - 1) % n,
            None if forward => 0,
            None => n - 1,
        };
        self.choose(next);
    }

    fn history(&self, w: &Rc<Window>, back: bool) -> bool {
        let history = w.history.borrow();
        if history.is_empty() {
            return false;
        }
        let pos = match (self.history_pos.get(), back) {
            (None, true) => history.len() - 1,
            (None, false) => return false,
            (Some(p), true) => p.saturating_sub(1),
            (Some(p), false) if p + 1 < history.len() => p + 1,
            (Some(_), false) => {
                self.history_pos.set(None);
                self.set_quietly("");
                return true;
            }
        };
        self.history_pos.set(Some(pos));
        self.set_quietly(&history[pos]);
        true
    }

    fn set_quietly(&self, text: &str) {
        self.programmatic.set(true);
        self.entry.set_text(text);
        self.entry.set_position(-1);
        self.programmatic.set(false);
    }

    fn key(&self, w: &Rc<Window>, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use gdk::Key as K;
        use glib::Propagation::{Proceed, Stop};
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let empty = self.entry.text().is_empty();
        match key {
            K::Tab => self.step(w, true),
            K::ISO_Left_Tab => self.step(w, false),
            K::Up if empty || self.history_pos.get().is_some() => {
                self.history(w, true);
            }
            K::Down if self.history_pos.get().is_some() => {
                self.history(w, false);
            }
            K::p if ctrl => {
                self.history(w, true);
            }
            K::n if ctrl => {
                self.history(w, false);
            }
            K::Down => self.step(w, true),
            K::Up => self.step(w, false),
            K::Return | K::KP_Enter => self.submit(w),
            K::Escape => self.popover.popdown(),
            _ => return Proceed,
        }
        Stop
    }

    fn submit(&self, w: &Rc<Window>) {
        // fill in a single unambiguous completion of a half-typed word
        {
            let completion = self.completion.borrow();
            if self.cycle.borrow().is_none()
                && !completion.prefix.is_empty()
                && completion.candidates.len() == 1
                && completion.candidates[0].value != completion.prefix
            {
                drop(completion);
                self.choose(0);
            }
        }
        let line = self.entry.text().trim().to_owned();
        if line.is_empty() {
            self.popover.popdown();
            return;
        }
        // close first so focus returns to the calendar for what runs next
        self.popover.popdown();
        match w.execute(&line) {
            Ok(()) => {
                let mut history = w.history.borrow_mut();
                history.retain(|h| *h != line);
                history.push(line);
                drop(history);
                // unless the command opened something else (a dialog), go back to
                // the calendar
                if w.window.visible_dialog().is_none() {
                    w.focus_view();
                }
            }
            Err(e) => {
                // reopen with the line and the problem
                show(w, &line, Some(&e));
            }
        }
    }
}

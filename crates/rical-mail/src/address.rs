//! An entry for addresses that completes from the address books and from
//! the people the mail is from.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::state::{Person, SharedState};

pub struct AddressEntry {
    pub entry: gtk::Entry,
    popover: gtk::Popover,
    list: gtk::ListBox,
    matches: RefCell<Vec<Person>>,
    state: SharedState,
    /// Set while the entry is changed here, so it does not search again.
    quiet: std::cell::Cell<bool>,
}

/// Where the address being typed starts: after the last comma outside
/// quotes and angle brackets.
fn token_start(text: &str) -> usize {
    let (mut quote, mut angle) = (false, false);
    let mut start = 0;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quote = !quote,
            '<' if !quote => angle = true,
            '>' if !quote => angle = false,
            ',' | ';' if !quote && !angle => start = i + 1,
            _ => {}
        }
    }
    start
}

impl AddressEntry {
    pub fn new(state: SharedState, label: &str) -> Rc<AddressEntry> {
        let entry = gtk::Entry::builder().hexpand(true).build();
        entry.update_property(&[gtk::accessible::Property::Label(label)]);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .can_focus(false)
            .build();
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .propagate_natural_height(true)
            .max_content_height(300)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroll)
            .autohide(false)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .halign(gtk::Align::Start)
            .build();
        popover.set_parent(&entry);
        popover.set_can_focus(false);
        let this = Rc::new(AddressEntry {
            entry,
            popover,
            list,
            matches: RefCell::default(),
            state,
            quiet: Default::default(),
        });
        let weak = Rc::downgrade(&this);
        this.entry.connect_changed(move |_| {
            if let Some(t) = weak.upgrade()
                && !t.quiet.get()
            {
                t.update();
            }
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_row_activated(move |_, row| {
            if let Some(t) = weak.upgrade() {
                t.accept(row.index() as usize);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&this);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(t) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            t.key(key, mods)
        });
        this.entry.add_controller(keys);
        let weak = Rc::downgrade(&this);
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| {
            if let Some(t) = weak.upgrade() {
                t.popover.popdown();
            }
        });
        this.entry.add_controller(focus);
        this
    }

    fn update(&self) {
        let text = self.entry.text().to_string();
        let start = token_start(&text);
        let typed = text[start..].trim();
        // an address already complete needs no help
        let found = if typed.is_empty()
            || typed.contains('<')
            || typed.contains('@')
                && typed.contains('.')
                && !typed.contains(' ')
                && self.state.borrow().complete_address(typed, 1).is_empty()
        {
            Vec::new()
        } else {
            self.state.borrow().complete_address(typed, 8)
        };
        while let Some(c) = self.list.first_child() {
            self.list.remove(&c);
        }
        if found.is_empty() {
            self.popover.popdown();
            self.matches.borrow_mut().clear();
            return;
        }
        for p in &found {
            let b = gtk::Box::builder()
                .spacing(8)
                .margin_top(4)
                .margin_bottom(4)
                .build();
            b.append(&gtk::Image::from_icon_name(if p.contact {
                "avatar-default-symbolic"
            } else {
                "mail-replied-symbolic"
            }));
            let name = gtk::Label::builder()
                .label(if p.name.is_empty() {
                    &p.address
                } else {
                    &p.name
                })
                .xalign(0.0)
                .build();
            b.append(&name);
            if !p.name.is_empty() {
                b.append(
                    &gtk::Label::builder()
                        .label(&p.address)
                        .css_classes(["dim-label"])
                        .hexpand(true)
                        .xalign(0.0)
                        .build(),
                );
            }
            self.list.append(&b);
        }
        *self.matches.borrow_mut() = found;
        if let Some(row) = self.list.row_at_index(0) {
            self.list.select_row(Some(&row));
        }
        // under the address being typed, not the start of the entry
        let chars = text[..start].chars().count();
        let x = self
            .entry
            .delegate()
            .and_downcast::<gtk::Text>()
            .and_then(|t| {
                let (strong, _) = t.compute_cursor_extents(chars);
                t.compute_point(&self.entry, &gtk::graphene::Point::new(strong.x(), 0.0))
            })
            .map_or(0, |p| p.x() as i32);
        self.popover
            .set_pointing_to(Some(&gdk::Rectangle::new(x, 0, 1, self.entry.height())));
        self.popover.popup();
    }

    fn key(&self, key: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use glib::Propagation::{Proceed, Stop};
        if !self.popover.is_visible()
            || mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK)
        {
            return Proceed;
        }
        let n = self.matches.borrow().len();
        let at = self.list.selected_row().map_or(0, |r| r.index() as usize);
        match key {
            gdk::Key::Down => {
                self.select((at + 1) % n);
                Stop
            }
            gdk::Key::Up => {
                self.select((at + n - 1) % n);
                Stop
            }
            gdk::Key::Tab | gdk::Key::Return | gdk::Key::KP_Enter => {
                self.accept(at);
                Stop
            }
            gdk::Key::Escape => {
                self.popover.popdown();
                Stop
            }
            _ => Proceed,
        }
    }

    fn select(&self, i: usize) {
        if let Some(row) = self.list.row_at_index(i as i32) {
            self.list.select_row(Some(&row));
        }
    }

    fn accept(&self, i: usize) {
        let Some(p) = self.matches.borrow().get(i).cloned() else {
            return;
        };
        let text = self.entry.text().to_string();
        let start = token_start(&text);
        let lead = text[..start].trim_end();
        let lead = if lead.is_empty() {
            String::new()
        } else {
            format!("{lead} ")
        };
        let new = format!("{lead}{}, ", p.mailbox().display());
        self.quiet.set(true);
        self.entry.set_text(&new);
        self.entry.set_position(-1);
        self.quiet.set(false);
        self.popover.popdown();
        self.matches.borrow_mut().clear();
    }
}

impl Drop for AddressEntry {
    fn drop(&mut self) {
        self.popover.unparent();
    }
}

impl AddressEntry {
    /// Put text in the entry without offering completions for it.
    pub fn set(&self, text: &str) {
        self.quiet.set(true);
        self.entry.set_text(text);
        self.quiet.set(false);
    }
}

//! The editor's guest list: a row per guest and an entry that suggests
//! people as you type (from past events and contacts). Up and Down pick a
//! suggestion, Enter or Tab adds it; an address that is not known can be
//! typed in full.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, glib};
use rical_core::{
    people::{Known, suggest},
    write::Guest,
};

const SUGGESTIONS: usize = 8;

pub struct GuestsEditor {
    pub group: adw::PreferencesGroup,
    guests: RefCell<Vec<Guest>>,
    rows: RefCell<Vec<adw::ActionRow>>,
    entry: adw::EntryRow,
    popover: gtk::Popover,
    list: gtk::ListBox,
    known: Rc<Vec<Known>>,
    /// What each suggestion row adds.
    shown: RefCell<Vec<Guest>>,
}

/// Text typed as a whole address, perhaps as `Name <addr>`.
pub fn typed_guest(text: &str) -> Option<Guest> {
    let text = text.trim();
    let (name, email) = match (text.find('<'), text.rfind('>')) {
        (Some(a), Some(b)) if a < b => {
            let name = text[..a].trim().trim_matches('"');
            (
                (!name.is_empty()).then(|| name.to_owned()),
                text[a + 1..b].trim(),
            )
        }
        _ => (None, text),
    };
    let email = email.trim_start_matches("mailto:");
    let (user, domain) = email.split_once('@')?;
    (!user.is_empty() && domain.contains('.') && !email.contains(char::is_whitespace)).then(|| {
        Guest {
            email: email.to_lowercase(),
            name,
        }
    })
}

impl GuestsEditor {
    /// `editable` is false for invitations from others: their guests are
    /// shown but not theirs to change here.
    pub fn new(initial: &[Guest], known: Rc<Vec<Known>>, editable: bool) -> Rc<Self> {
        let group = adw::PreferencesGroup::builder()
            .title("Guests")
            .description(if editable {
                "With guests you are the organiser; your server sends the \
                 invitations when this syncs."
            } else {
                "Organised by someone else."
            })
            .build();
        let entry = adw::EntryRow::builder()
            .title("Add a guest (name or address)")
            .visible(editable)
            .build();
        group.add(&entry);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        let popover = gtk::Popover::builder()
            .child(&list)
            .position(gtk::PositionType::Bottom)
            .has_arrow(false)
            .autohide(false)
            .can_focus(false)
            .build();
        popover.set_parent(&entry);
        let this = Rc::new(Self {
            group,
            guests: RefCell::new(Vec::new()),
            rows: RefCell::new(Vec::new()),
            entry,
            popover,
            list,
            known,
            shown: RefCell::new(Vec::new()),
        });
        for g in initial {
            this.add(g.clone(), editable);
        }

        let t = Rc::downgrade(&this);
        this.entry.connect_changed(move |_| {
            if let Some(t) = t.upgrade() {
                t.update_suggestions();
            }
        });
        let t = Rc::downgrade(&this);
        this.entry.connect_entry_activated(move |_| {
            if let Some(t) = t.upgrade() {
                t.accept();
            }
        });
        let t = Rc::downgrade(&this);
        this.list.connect_row_activated(move |_, row| {
            if let Some(t) = t.upgrade() {
                t.list.select_row(Some(row));
                t.accept();
                t.entry.grab_focus();
            }
        });
        // Up/Down move through the suggestions, Tab takes one, Escape
        // closes them before it closes the dialog
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let t = Rc::downgrade(&this);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(t) = t.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if !t.popover.is_visible() {
                return glib::Propagation::Proceed;
            }
            let n = t.shown.borrow().len() as i32;
            let at = t.list.selected_row().map_or(-1, |r| r.index());
            match key {
                gdk::Key::Down => t.select((at + 1).min(n - 1)),
                gdk::Key::Up => t.select((at - 1).max(0)),
                gdk::Key::Tab => t.accept(),
                gdk::Key::Escape => t.popover.popdown(),
                _ => return glib::Propagation::Proceed,
            }
            glib::Propagation::Stop
        });
        this.entry.add_controller(keys);
        // the popover hangs off the entry; let it go with the dialog
        let p = this.popover.clone();
        this.entry.connect_destroy(move |_| p.unparent());
        this
    }

    /// Say who the invitations go out as, for the chosen calendar.
    pub fn set_sender(&self, sender: Option<&str>) {
        if !self.entry.is_visible() {
            return;
        }
        self.group.remove_css_class("error");
        self.group.set_description(Some(&match sender {
            Some(s) => format!("Invitations go out from your server as {s} when this syncs."),
            None => "Your address is not known: set `me = you@example.com` in \
                     ~/.config/rical/config to invite people."
                .to_owned(),
        }));
    }

    pub fn value(&self) -> Vec<Guest> {
        self.guests.borrow().clone()
    }

    fn select(&self, i: i32) {
        if let Some(row) = self.list.row_at_index(i) {
            self.list.select_row(Some(&row));
        }
    }

    fn update_suggestions(&self) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let text = self.entry.text();
        let skip = self
            .guests
            .borrow()
            .iter()
            .map(|g| g.email.clone())
            .collect::<Vec<_>>();
        let mut shown = suggest(&self.known, &text, &skip, SUGGESTIONS)
            .into_iter()
            .map(|k| k.guest.clone())
            .collect::<Vec<_>>();
        // a full address nobody has used yet
        if let Some(g) = typed_guest(&text)
            && !shown.iter().any(|s| s.email == g.email)
            && !skip.contains(&g.email)
        {
            shown.insert(0, g);
        }
        for g in &shown {
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.append(
                &gtk::Label::builder()
                    .label(g.name.as_deref().unwrap_or(&g.email))
                    .xalign(0.0)
                    .build(),
            );
            if g.name.is_some() {
                b.append(
                    &gtk::Label::builder()
                        .label(&g.email)
                        .xalign(0.0)
                        .css_classes(["dim-label", "caption"])
                        .build(),
                );
            }
            self.list.append(&b);
        }
        let any = !shown.is_empty();
        *self.shown.borrow_mut() = shown;
        if any {
            self.select(0);
            // as wide as the entry
            self.list.set_size_request(self.entry.width() - 12, -1);
            self.popover.set_pointing_to(Some(&gdk::Rectangle::new(
                0,
                0,
                self.entry.width(),
                self.entry.height(),
            )));
            self.popover.popup();
        } else {
            self.popover.popdown();
        }
    }

    /// Add the chosen suggestion, or what was typed if it is an address.
    fn accept(self: &Rc<Self>) {
        let chosen = self
            .list
            .selected_row()
            .and_then(|r| self.shown.borrow().get(r.index() as usize).cloned())
            .or_else(|| typed_guest(&self.entry.text()));
        match chosen {
            Some(g) => {
                self.entry.remove_css_class("error");
                self.add(g, true);
                self.entry.set_text("");
                self.popover.popdown();
            }
            None if !self.entry.text().trim().is_empty() => self.entry.add_css_class("error"),
            None => {}
        }
    }

    fn add(self: &Rc<Self>, guest: Guest, removable: bool) {
        if self.guests.borrow().iter().any(|g| g.email == guest.email) {
            return;
        }
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(
                guest.name.as_deref().unwrap_or(&guest.email),
            ))
            .build();
        if guest.name.is_some() {
            row.set_subtitle(&glib::markup_escape_text(&guest.email));
        }
        row.add_prefix(&gtk::Image::from_icon_name("avatar-default-symbolic"));
        if removable {
            let remove = gtk::Button::builder()
                .icon_name("list-remove-symbolic")
                .tooltip_text("Remove")
                .valign(gtk::Align::Center)
                .css_classes(["flat", "circular"])
                .build();
            let (t, email) = (Rc::downgrade(self), guest.email.clone());
            remove.connect_clicked(move |_| {
                if let Some(t) = t.upgrade() {
                    t.remove(&email);
                }
            });
            row.add_suffix(&remove);
        }
        self.group.add(&row);
        self.guests.borrow_mut().push(guest);
        self.rows.borrow_mut().push(row);
    }

    fn remove(&self, email: &str) {
        let Some(i) = self.guests.borrow().iter().position(|g| g.email == email) else {
            return;
        };
        self.guests.borrow_mut().remove(i);
        let row = self.rows.borrow_mut().remove(i);
        self.group.remove(&row);
        self.entry.grab_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_addresses() {
        assert_eq!(
            typed_guest("Sam@Example.com"),
            Some(Guest {
                email: "sam@example.com".into(),
                name: None
            })
        );
        assert_eq!(
            typed_guest("\"Sam Jones\" <sam@example.com>"),
            Some(Guest {
                email: "sam@example.com".into(),
                name: Some("Sam Jones".into())
            })
        );
        assert_eq!(typed_guest("sam"), None);
        assert_eq!(typed_guest("sam@localhost"), None);
        assert_eq!(typed_guest("a b@example.com"), None);
    }
}

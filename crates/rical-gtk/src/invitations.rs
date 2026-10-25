//! The invitations view: upcoming events I am invited to and have not
//! answered (or answered "maybe"), each once however often it repeats.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use rical_core::{Kind, Occurrence};

use crate::{
    details, format, listhead,
    state::{OccurrenceKey, SharedState, State},
};

/// Answer an invitation: its key and ACCEPTED, TENTATIVE or DECLINED.
pub type OnReply = Rc<dyn Fn(OccurrenceKey, &'static str)>;

pub struct InvitationsView {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    headings: listhead::Headings,
    state: SharedState,
    rows: Rc<RefCell<Vec<OccurrenceKey>>>,
    syncing: Rc<std::cell::Cell<bool>>,
    on_reply: OnReply,
    built: std::cell::Cell<Option<(u64, chrono::NaiveDate)>>,
}

/// The invitations waiting on me, (needing a reply, answered maybe), each
/// by its next occurrence.
pub fn waiting(state: &State) -> (Vec<Occurrence>, Vec<Occurrence>) {
    let occ = state.occurrences(&state.visible_days());
    let local = state.local;
    let now = chrono::Utc::now();
    let mut seen = std::collections::HashSet::new();
    let mut sorted = occ
        .iter()
        .filter(|o| o.kind == Kind::Event && o.end.to_utc(local) > now)
        .collect::<Vec<_>>();
    sorted.sort_by_key(|o| o.start.to_utc(local));
    let (mut open, mut maybe) = (Vec::new(), Vec::new());
    for o in sorted {
        let Some(reply) = state.invitation(o).and_then(|i| i.reply) else {
            continue;
        };
        if format::is_inactive(o) || !seen.insert((o.calendar.clone(), o.uid.clone())) {
            continue;
        }
        match reply {
            "NEEDS-ACTION" => open.push(o.clone()),
            "TENTATIVE" => maybe.push(o.clone()),
            _ => {}
        }
    }
    (open, maybe)
}

impl InvitationsView {
    pub fn new(state: SharedState, on_change: Rc<dyn Fn()>, on_reply: OnReply) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Invitations")]);
        let clamp = adw::Clamp::builder()
            .maximum_size(760)
            .child(&list)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let empty = adw::StatusPage::builder()
            .icon_name("mail-read-symbolic")
            .title("All answered")
            .description("No invitations are waiting for a reply")
            .build();
        let widget = gtk::Stack::new();
        widget.add_named(&scroller, Some("list"));
        widget.add_named(&empty, Some("empty"));

        let this = Rc::new(Self {
            widget,
            headings: listhead::install(&list),
            list,
            state,
            rows: Default::default(),
            syncing: Default::default(),
            on_reply,
            built: Default::default(),
        });

        let weak = Rc::downgrade(&this);
        this.list.connect_row_selected(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            if this.syncing.get() {
                return;
            }
            let key = row.and_then(|r| this.rows.borrow().get(r.index() as usize).cloned());
            if let Some(key) = key {
                this.state.borrow_mut().selected = Some(key);
                on_change();
            }
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_row_activated(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            let state = this.state.borrow();
            if let Some(occ) = state.selected_occurrence() {
                let rect = gtk::gdk::Rectangle::new(0, 0, row.width(), row.height());
                details::show(row, rect, &occ, &state);
            }
        });
        let list = this.list.downgrade();
        details::edit_on_double_click(&this.list, move |_, y| {
            list.upgrade()
                .and_then(|l| l.row_at_y(y as i32))
                .is_some_and(|r| r.is_activatable())
        });
        this
    }

    pub fn refresh(&self) {
        let key = {
            let s = self.state.borrow();
            (s.generation(), s.today())
        };
        if self.built.get() != Some(key) {
            let had_focus = self.list.focus_child().is_some();
            self.rebuild();
            self.built.set(Some(key));
            if had_focus {
                self.focus();
            }
            return;
        }
        let index = {
            let s = self.state.borrow();
            self.rows
                .borrow()
                .iter()
                .position(|k| s.selected.as_ref() == Some(k))
        };
        self.syncing.set(true);
        match index.and_then(|i| self.list.row_at_index(i as i32)) {
            Some(row) => self.list.select_row(Some(&row)),
            None => self.list.unselect_all(),
        }
        self.syncing.set(false);
    }

    fn rebuild(&self) {
        self.syncing.set(true);
        self.headings.borrow_mut().clear();
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        let state = self.state.borrow();
        let (open, maybe) = waiting(&state);
        let mut rows = Vec::new();
        let mut selected_row = None;
        for (title, group) in [("Needs a reply", &open), ("Maybe", &maybe)] {
            let mut heading = Some(listhead::label(&format!("{title} · {}", group.len())));
            for o in group {
                let row = self.row(o, &state);
                self.headings
                    .borrow_mut()
                    .push(heading.take().map(|l| l.upcast()));
                if state.selected.as_ref().is_some_and(|k| k.matches(o)) {
                    selected_row = Some(row.clone());
                }
                self.list.append(&row);
                rows.push(OccurrenceKey::of(o));
            }
        }
        self.widget
            .set_visible_child_name(if rows.is_empty() { "empty" } else { "list" });
        *self.rows.borrow_mut() = rows;
        self.list.select_row(selected_row.as_ref());
        self.syncing.set(false);
    }

    pub fn focus(&self) {
        match self
            .list
            .selected_row()
            .or_else(|| self.list.row_at_index(0))
        {
            Some(row) => {
                row.grab_focus();
            }
            None => {
                self.list.grab_focus();
            }
        }
    }

    pub fn show_selected_details(&self) {
        if let Some(row) = self.list.selected_row() {
            row.activate();
        }
    }

    fn row(&self, o: &Occurrence, state: &State) -> gtk::ListBoxRow {
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        b.append(&format::color_dot(state.display_color(o), 10));

        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        text.append(
            &gtk::Label::builder()
                .label(&o.summary)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        let mut sub = format::time_range(o, state.local);
        if o.recurring {
            sub += " · repeats";
        }
        if let Some(invite) = state.invitation(o) {
            sub = format!("{sub} · from {}", invite.from.display());
        }
        text.append(
            &gtk::Label::builder()
                .label(&sub)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        b.append(&text);

        let buttons = gtk::Box::builder()
            .css_classes(["linked"])
            .valign(gtk::Align::Center)
            .build();
        for (label, status, key) in [
            ("Accept", "ACCEPTED", "y"),
            ("Maybe", "TENTATIVE", "Y"),
            ("Decline", "DECLINED", "N"),
        ] {
            let button = gtk::Button::builder()
                .label(label)
                .tooltip_text(format!("{label} ({key})"))
                .focusable(false)
                .build();
            let (reply, key) = (self.on_reply.clone(), OccurrenceKey::of(o));
            button.connect_clicked(move |_| reply(key.clone(), status));
            buttons.append(&button);
        }
        b.append(&buttons);
        gtk::ListBoxRow::builder().child(&b).build()
    }
}

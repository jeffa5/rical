//! The journal view: journal entries (VJOURNAL), newest first, and undated
//! notes after them.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use adw::prelude::*;
use rical_core::Occurrence;

use crate::{
    format, listhead,
    state::{SharedState, State},
};

pub struct JournalsView {
    pub widget: gtk::Stack,
    list: gtk::ListBox,
    headings: listhead::Headings,
    state: SharedState,
    /// (file, UID) of each row.
    rows: Rc<RefCell<Vec<(PathBuf, String)>>>,
    syncing: Rc<std::cell::Cell<bool>>,
    built: std::cell::Cell<Option<u64>>,
}

/// The entries to show: dated ones newest first, then undated notes,
/// newest written first; hidden calendars left out.
pub fn entries(state: &State) -> (Vec<Occurrence>, Vec<Occurrence>) {
    let local = state.local;
    let (mut dated, mut notes): (Vec<_>, Vec<_>) = state
        .vdir
        .journals()
        .into_iter()
        .filter(|(o, _)| !state.hidden.contains(&o.calendar))
        .partition(|(_, d)| *d);
    for list in [&mut dated, &mut notes] {
        list.sort_by_key(|(o, _)| std::cmp::Reverse(o.start.to_utc(local)));
    }
    (
        dated.into_iter().map(|(o, _)| o).collect(),
        notes.into_iter().map(|(o, _)| o).collect(),
    )
}

impl JournalsView {
    pub fn new(state: SharedState, on_change: Rc<dyn Fn()>) -> Rc<Self> {
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Journal")]);
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
            .icon_name("document-edit-symbolic")
            .title("No journal entries")
            .description("Journal entries and notes (VJOURNAL), e.g. from jtx Board, show here")
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
            built: Default::default(),
        });
        let weak = Rc::downgrade(&this);
        this.list.connect_row_selected(move |_, row| {
            let Some(this) = weak.upgrade() else { return };
            if this.syncing.get() {
                return;
            }
            let id = row.and_then(|r| this.rows.borrow().get(r.index() as usize).cloned());
            if id.is_some() {
                this.state.borrow_mut().selected_journal = id;
                on_change();
            }
        });
        crate::details::edit_on_double_click(&this.list, |_, _| true);
        this
    }

    pub fn refresh(&self) {
        let generation = self.state.borrow().generation();
        if self.built.get() != Some(generation) {
            let had_focus = self.list.focus_child().is_some();
            self.rebuild();
            self.built.set(Some(generation));
            if had_focus {
                self.focus();
            }
            return;
        }
        let selected = self.state.borrow().selected_journal.clone();
        let index = self
            .rows
            .borrow()
            .iter()
            .position(|r| selected.as_ref() == Some(r));
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
        let (dated, notes) = entries(&state);
        let mut rows = Vec::new();
        let mut selected_row = None;
        let mut last_month = None;
        for (o, is_note) in dated
            .iter()
            .map(|o| (o, false))
            .chain(notes.iter().map(|o| (o, true)))
        {
            // a heading per month, then one for the notes
            let group = if is_note {
                "Notes".to_owned()
            } else {
                o.start.date(state.local).format("%B %Y").to_string()
            };
            let heading = (last_month.as_ref() != Some(&group)).then(|| listhead::label(&group));
            last_month = Some(group);
            let row = self.row(o, is_note, &state);
            self.headings.borrow_mut().push(heading.map(|l| l.upcast()));
            let id = (o.path.clone(), o.uid.clone());
            if state.selected_journal.as_ref() == Some(&id) {
                selected_row = Some(row.clone());
            }
            self.list.append(&row);
            rows.push(id);
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

    pub fn step(&self, forward: bool) {
        listhead::step(&self.list, forward);
    }

    pub fn selected_row(&self) -> Option<gtk::ListBoxRow> {
        self.list.selected_row()
    }

    fn row(&self, o: &Occurrence, is_note: bool, state: &State) -> gtk::ListBoxRow {
        let b = gtk::Box::builder()
            .orientation(gtk::Orientation::Horizontal)
            .spacing(12)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();
        let when = if is_note {
            String::new()
        } else {
            o.start.date(state.local).format("%a %-d").to_string()
        };
        b.append(
            &gtk::Label::builder()
                .label(&when)
                .width_chars(7)
                .xalign(0.0)
                .valign(gtk::Align::Start)
                .css_classes(["numeric", "dim-label"])
                .build(),
        );
        let dot = format::color_dot(state.display_color(o), 10);
        dot.set_valign(gtk::Align::Start);
        dot.set_margin_top(4);
        b.append(&dot);
        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(2)
            .hexpand(true)
            .build();
        let title = gtk::Label::builder()
            .label(format::journal_title(o))
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        if format::is_inactive(o) {
            title.add_css_class("dim-label");
        }
        text.append(&title);
        // a preview of the text, unless it is only the title again
        let title_text = format::journal_title(o);
        let preview = o
            .description
            .as_deref()
            .unwrap_or_default()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .skip_while(|l| *l == title_text)
            .take(2)
            .collect::<Vec<_>>()
            .join(" · ");
        if !preview.is_empty() {
            text.append(
                &gtk::Label::builder()
                    .label(&preview)
                    .xalign(0.0)
                    .wrap(true)
                    .lines(2)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .css_classes(["caption", "dim-label"])
                    .build(),
            );
        }
        b.append(&text);
        gtk::ListBoxRow::builder().child(&b).build()
    }
}

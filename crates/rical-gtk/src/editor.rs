//! The dialog for creating and editing an event or todo. Fully usable from
//! the keyboard: Tab between fields, Enter or Ctrl+Enter to save, Escape to
//! cancel.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{Days, TimeDelta};
use gtk::glib;
use rical_core::{Kind, When, write::Draft};

use crate::{format, repeat::RepeatEditor, state::SharedState};

pub struct Options {
    pub title: &'static str,
    pub draft: Draft,
    pub calendar: String,
    /// New items can choose between event and todo.
    pub is_new: bool,
}

pub fn open(
    parent: &impl IsA<gtk::Widget>,
    state: &SharedState,
    options: Options,
    on_save: impl Fn(Draft, String) + 'static,
    on_delete: Option<Box<dyn Fn()>>,
) {
    let local = state.borrow().local;
    let calendars = state
        .borrow()
        .vdir
        .calendars
        .iter()
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect::<Vec<_>>();
    let draft = options.draft;

    let title = adw::EntryRow::builder()
        .title("Title")
        .text(&draft.summary)
        .build();

    let kind = adw::ComboRow::builder()
        .title("Type")
        .model(&gtk::StringList::new(&["Event", "Todo"]))
        .selected(u32::from(draft.kind == Kind::Todo))
        .visible(options.is_new)
        .build();

    let names = calendars
        .iter()
        .map(|(_, n)| n.as_str())
        .collect::<Vec<_>>();
    let calendar = adw::ComboRow::builder()
        .title("Calendar")
        .model(&gtk::StringList::new(&names))
        .selected(
            calendars
                .iter()
                .position(|(id, _)| *id == options.calendar)
                .unwrap_or(0) as u32,
        )
        .build();

    let all_day = adw::SwitchRow::builder()
        .title("All day")
        .active(draft.start.is_date())
        .build();
    // all-day ends are shown inclusive, stored exclusive
    let shown_end = |d: &Draft| match d.end {
        When::Date(e) if d.start.is_date() => When::Date(
            e.checked_sub_days(Days::new(1))
                .unwrap_or(e)
                .max(d.start.date(local)),
        ),
        e => e,
    };
    let starts = adw::EntryRow::builder()
        .title("Starts")
        .text(format::input(&draft.start, local))
        .build();
    let ends = adw::EntryRow::builder()
        .title("Ends  (time, date or +duration)")
        .text(format::input(&shown_end(&draft), local))
        .build();

    let repeat = RepeatEditor::new(&draft.repeat);

    let location = adw::EntryRow::builder()
        .title("Location")
        .text(&draft.location)
        .build();
    let notes = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .height_request(90)
        .build();
    notes.buffer().set_text(&draft.description);
    notes.update_property(&[gtk::accessible::Property::Label("Notes")]);

    let what = adw::PreferencesGroup::new();
    what.add(&title);
    what.add(&kind);
    what.add(&calendar);
    let when = adw::PreferencesGroup::new();
    when.add(&all_day);
    when.add(&starts);
    when.add(&ends);
    for row in repeat.rows() {
        when.add(&row);
    }
    let more = adw::PreferencesGroup::new();
    more.add(&location);
    let notes_group = adw::PreferencesGroup::builder().title("Notes").build();
    notes_group.add(
        &gtk::Frame::builder()
            .child(&notes)
            .css_classes(["view"])
            .build(),
    );

    let page = adw::PreferencesPage::new();
    page.add(&what);
    page.add(&when);
    page.add(&more);
    page.add(&notes_group);

    let dialog = adw::Dialog::builder()
        .title(options.title)
        .content_width(520)
        .build();

    if let Some(on_delete) = on_delete {
        let group = adw::PreferencesGroup::new();
        let delete = adw::ButtonRow::builder()
            .title("Delete")
            .css_classes(["destructive-action"])
            .build();
        let d = dialog.clone();
        delete.connect_activated(move |_| {
            d.close();
            on_delete();
        });
        group.add(&delete);
        page.add(&group);
    }

    // switching all-day converts the fields between dates and times
    {
        let (starts, ends) = (starts.clone(), ends.clone());
        all_day.connect_active_notify(move |sw| {
            let base = When::Date(chrono::Utc::now().with_timezone(&local).date_naive());
            let convert = |row: &adw::EntryRow, base: &When| {
                let w = format::parse_input(&row.text(), !sw.is_active(), base, local)?;
                let w = if sw.is_active() {
                    When::Date(w.date(local))
                } else {
                    let ndt = w.date(local).and_hms_opt(9, 0, 0).unwrap();
                    When::Zoned(rical_core::when::resolve_local(local, ndt))
                };
                row.set_text(&format::input(&w, local));
                Some(w)
            };
            if let Some(s) = convert(&starts, &base) {
                if sw.is_active() {
                    convert(&ends, &s);
                } else {
                    ends.set_text(&format::input(&s.add(TimeDelta::hours(1)), local));
                }
            }
        });
    }

    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .tooltip_text("Save (Ctrl+Enter)")
        .build();
    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));

    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });

    let original = draft.clone();
    let do_save: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let (title, kind, calendar, all_day, starts, ends, repeat, location, notes) = (
            title.clone(),
            kind.clone(),
            calendar.clone(),
            all_day.clone(),
            starts.clone(),
            ends.clone(),
            repeat.clone(),
            location.clone(),
            notes.clone(),
        );
        move || {
            let is_all_day = all_day.is_active();
            starts.remove_css_class("error");
            ends.remove_css_class("error");
            let Some(start) =
                format::parse_input(&starts.text(), is_all_day, &original.start, local)
            else {
                starts.add_css_class("error");
                starts.grab_focus();
                return;
            };
            let end = match format::parse_input(&ends.text(), is_all_day, &start, local) {
                // shown inclusive, stored exclusive
                Some(When::Date(d)) => d.checked_add_days(Days::new(1)).map(When::Date),
                other => other,
            };
            let Some(end) = end.filter(|e| e.to_utc(local) >= start.to_utc(local)) else {
                ends.add_css_class("error");
                ends.grab_focus();
                return;
            };
            repeat.choice.remove_css_class("error");
            let rule = match repeat.value() {
                Ok(r) => r,
                Err(_) => {
                    repeat.choice.add_css_class("error");
                    repeat.choice.grab_focus();
                    return;
                }
            };
            let buffer = notes.buffer();
            let draft = Draft {
                kind: if kind.selected() == 1 {
                    Kind::Todo
                } else {
                    original.kind
                },
                summary: title.text().trim().to_owned(),
                location: location.text().trim().to_owned(),
                description: buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .trim_end()
                    .to_owned(),
                start,
                end,
                repeat: rule,
            };
            let calendar = calendars
                .get(calendar.selected() as usize)
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
            dialog.close();
            on_save(draft, calendar);
        }
    });

    let s = do_save.clone();
    save.connect_clicked(move |_| s());
    for row in [&title, &starts, &ends, &location] {
        let s = do_save.clone();
        row.connect_entry_activated(move |_| s());
    }
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Managed);
    let s = do_save.clone();
    shortcuts.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("<Control>Return"),
        Some(gtk::CallbackAction::new(move |_, _| {
            s();
            glib::Propagation::Stop
        })),
    ));
    dialog.add_controller(shortcuts);

    dialog.set_focus(Some(&title));
    dialog.present(Some(parent));
}

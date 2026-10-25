//! The dialog for creating and editing an event or todo. Fully usable from
//! the keyboard: Tab between fields, Enter or Ctrl+Enter to save, Escape to
//! cancel.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{Days, TimeDelta};
use gtk::glib;
use rical_core::{Kind, When, write::Draft};

use crate::{
    format, guests::GuestsEditor, repeat::RepeatEditor, state::SharedState, zonepick::ZonePicker,
};

pub struct Options {
    pub title: &'static str,
    pub draft: Draft,
    pub calendar: String,
    /// New items can choose between event and todo.
    pub is_new: bool,
    /// Someone else's event: its guests are shown but not changed.
    pub invitation: bool,
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
    // the zones the times are typed in: the event's own, mine for new ones
    let zones = Rc::new(ZonePicker::new(local, &draft));
    let starts = adw::EntryRow::builder()
        .title("Starts")
        .text(format::input(&draft.start, zones.start_tz()))
        .build();
    let ends = adw::EntryRow::builder()
        .title("Ends  (time, date or +duration)")
        .text(format::input(&shown_end(&draft), zones.end_tz()))
        .build();
    zones.set_visible(!draft.start.is_date());
    // what the times are for me, when typed in another zone
    let hints: Rc<dyn Fn()> = {
        let (zones, starts, ends, all_day) =
            (zones.clone(), starts.clone(), ends.clone(), all_day.clone());
        let base = draft.start;
        Rc::new(move || {
            let hint = |w: Option<When>| match w {
                Some(When::Zoned(dt)) if dt.timezone() != local => {
                    format!(
                        "  ·  {} for you",
                        dt.with_timezone(&local).format("%H:%M %a")
                    )
                }
                _ => String::new(),
            };
            if all_day.is_active() {
                starts.set_title("Starts");
                ends.set_title("Ends");
                return;
            }
            let start = zones.parse_start(&starts.text(), &base);
            starts.set_title(&format!("Starts{}", hint(start)));
            let end = start.and_then(|s| zones.parse_end(&ends.text(), &s));
            ends.set_title(&format!("Ends  (time, date or +duration){}", hint(end)));
        })
    };
    hints();
    for row in [&starts, &ends] {
        let h = hints.clone();
        row.connect_changed(move |_| h());
    }
    zones.on_change(hints.clone());

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
    when.add(&zones.start);
    when.add(&zones.end);
    for row in repeat.rows() {
        when.add(&row);
    }
    let more = adw::PreferencesGroup::new();
    more.add(&location);
    let known = state.borrow().people();
    let guests = GuestsEditor::new(
        draft.guests.as_deref().unwrap_or_default(),
        known,
        !options.invitation,
    );
    // tasks have no guests
    guests.group.set_visible(draft.kind == Kind::Event);
    {
        let g = guests.group.clone();
        kind.connect_selected_notify(move |k| g.set_visible(k.selected() == 0));
    }
    // who the invitations go out as depends on the calendar's account
    let sender_of = {
        let (state, calendars) = (state.clone(), calendars.clone());
        move |i: u32| {
            let id = calendars.get(i as usize).map(|(id, _)| id.clone())?;
            state.borrow().organizer_for(&id).map(|g| g.email)
        }
    };
    guests.set_sender(sender_of(calendar.selected()).as_deref());
    {
        let g = guests.clone();
        calendar.connect_selected_notify(move |c| g.set_sender(sender_of(c.selected()).as_deref()));
    }
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
    page.add(&guests.group);
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
        let (starts, ends, zones, hints) =
            (starts.clone(), ends.clone(), zones.clone(), hints.clone());
        all_day.connect_active_notify(move |sw| {
            // times are in the chosen zone; all-day items have none
            let tz = zones.start_tz();
            zones.set_visible(!sw.is_active());
            let base = When::Date(chrono::Utc::now().with_timezone(&tz).date_naive());
            let convert = |row: &adw::EntryRow, base: &When| {
                let w = format::parse_input(&row.text(), !sw.is_active(), base, tz)?;
                let w = if sw.is_active() {
                    When::Date(w.date(tz))
                } else {
                    let ndt = w.date(tz).and_hms_opt(9, 0, 0).unwrap();
                    When::Zoned(rical_core::when::resolve_local(tz, ndt))
                };
                row.set_text(&format::input(&w, tz));
                Some(w)
            };
            if let Some(s) = convert(&starts, &base) {
                if sw.is_active() {
                    convert(&ends, &s);
                } else {
                    ends.set_text(&format::input(&s.add(TimeDelta::hours(1)), tz));
                }
            }
            hints();
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
        let (guests, state, invitation) = (guests.clone(), state.clone(), options.invitation);
        let zones = zones.clone();
        move || {
            let is_all_day = all_day.is_active();
            starts.remove_css_class("error");
            ends.remove_css_class("error");
            // times are read in their zones; all-day ones are plain dates
            let start = if is_all_day {
                format::parse_input(&starts.text(), true, &original.start, local)
            } else {
                zones.parse_start(&starts.text(), &original.start)
            };
            let Some(start) = start else {
                starts.add_css_class("error");
                starts.grab_focus();
                return;
            };
            let end = if is_all_day {
                format::parse_input(&ends.text(), true, &start, local)
            } else {
                zones.parse_end(&ends.text(), &start)
            };
            let end = match end {
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
            let calendar = calendars
                .get(calendar.selected() as usize)
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
            let kind = if kind.selected() == 1 {
                Kind::Todo
            } else {
                original.kind
            };
            // guests, unless they are someone else's to change
            let chosen = (kind == Kind::Event && !invitation).then(|| guests.value());
            let organizer = state.borrow().organizer_for(&calendar);
            let had_guests = original.guests.as_ref().is_some_and(|g| !g.is_empty());
            if chosen.as_ref().is_some_and(|g| !g.is_empty()) && organizer.is_none() && !had_guests
            {
                guests.set_sender(None);
                guests.group.add_css_class("error");
                return;
            }
            let draft = Draft {
                kind,
                guests: chosen,
                organizer,
                summary: title.text().trim().to_owned(),
                location: location.text().trim().to_owned(),
                description: buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .trim_end()
                    .to_owned(),
                start,
                end,
                repeat: rule,
                // the zones chosen here are the ones to write
                exact_zones: !is_all_day,
            };
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

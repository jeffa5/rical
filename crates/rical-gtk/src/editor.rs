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
    /// The item being edited, if it exists (so a task is not offered as
    /// its own parent).
    pub uid: Option<String>,
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
    // a task's dates are each optional: empty for none
    let task = draft.kind == Kind::Todo;
    let starts = adw::EntryRow::builder()
        .title(labels(task).0)
        .text(if task && !draft.has_start {
            String::new()
        } else {
            format::input(&draft.start, zones.start_tz())
        })
        .build();
    let ends = adw::EntryRow::builder()
        .title(labels(task).1)
        .text(if task && !draft.has_due {
            String::new()
        } else {
            format::input(&shown_end(&draft), zones.end_tz())
        })
        .build();
    zones.set_visible(!draft.start.is_date());
    // what the times are for me, when typed in another zone
    let hints: Rc<dyn Fn()> = {
        let (zones, starts, ends, all_day, kind) = (
            zones.clone(),
            starts.clone(),
            ends.clone(),
            all_day.clone(),
            kind.clone(),
        );
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
            let (start_label, end_label) = labels(kind.selected() == 1);
            if all_day.is_active() {
                starts.set_title(start_label);
                ends.set_title(end_label);
                return;
            }
            let start = zones.parse_start(&starts.text(), &base);
            starts.set_title(&format!("{start_label}{}", hint(start)));
            let end = zones.parse_end(&ends.text(), &start.unwrap_or(base));
            ends.set_title(&format!("{end_label}{}", hint(end)));
        })
    };
    hints();
    for row in [&starts, &ends] {
        let h = hints.clone();
        row.connect_changed(move |_| h());
    }
    zones.on_change(hints.clone());

    let repeat = RepeatEditor::new(&draft.repeat);

    // for tasks: how important, and whether done
    let priority = adw::ComboRow::builder()
        .title("Priority")
        .model(&gtk::StringList::new(&["None", "High", "Medium", "Low"]))
        .selected(match draft.priority {
            None => 0,
            Some(1..=4) => 1,
            Some(5) => 2,
            Some(_) => 3,
        })
        .build();
    let done = adw::SwitchRow::builder()
        .title("Done")
        .active(draft.done == Some(true))
        .visible(!options.is_new)
        .build();
    // which task it is a subtask of: an open task in the same calendar
    // (subtasks never cross calendars), but not itself or its own subtasks
    let candidates = {
        let s = state.borrow();
        let mut tasks = s
            .vdir
            .tasks()
            .into_iter()
            // open tasks, and its current parent even if that is done
            .filter(|t| {
                (!t.is_done() || draft.parent.as_ref() == Some(&t.uid)) && !t.summary.is_empty()
            })
            .filter(|t| {
                options
                    .uid
                    .as_ref()
                    .is_none_or(|me| !s.vdir.is_within(&t.calendar, &t.uid, me))
            })
            .map(|t| (t.uid, t.calendar, t.summary))
            .collect::<Vec<_>>();
        tasks.sort_by_key(|t| t.2.to_lowercase());
        tasks
    };
    let parent_row = adw::ComboRow::builder()
        .title("Subtask of")
        .enable_search(true)
        .search_match_mode(gtk::StringFilterMatchMode::Substring)
        .expression(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<gtk::Expression>,
            "string",
        ))
        .build();
    // the choices for a calendar, keeping `keep` chosen if it is there
    let parents: Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let fill = {
        let (parent_row, parents) = (parent_row.clone(), parents.clone());
        move |calendar: &str, keep: Option<&str>| {
            let here = candidates
                .iter()
                .filter(|(_, cal, _)| cal == calendar)
                .collect::<Vec<_>>();
            let labels = std::iter::once("None (a task of its own)")
                .chain(here.iter().map(|(_, _, s)| s.as_str()))
                .collect::<Vec<_>>();
            parent_row.set_model(Some(&gtk::StringList::new(&labels)));
            *parents.borrow_mut() = here.iter().map(|(uid, _, _)| uid.clone()).collect();
            let at = keep.and_then(|k| here.iter().position(|(uid, _, _)| uid == k));
            parent_row.set_selected(at.map_or(0, |i| i as u32 + 1));
        }
    };
    let fill = Rc::new(fill);
    fill(&options.calendar, draft.parent.as_deref());
    {
        // another calendar: only its tasks can be the parent
        let (fill, parents, parent_row, calendars) = (
            fill.clone(),
            parents.clone(),
            parent_row.clone(),
            calendars.clone(),
        );
        calendar.connect_selected_notify(move |c| {
            let keep = match parent_row.selected() {
                0 => None,
                i => parents.borrow().get(i as usize - 1).cloned(),
            };
            if let Some((id, _)) = calendars.get(c.selected() as usize) {
                fill(id, keep.as_deref());
            }
        });
    }
    let task_group = adw::PreferencesGroup::builder().visible(task).build();
    task_group.add(&priority);
    task_group.add(&done);
    task_group.add(&parent_row);
    {
        let (g, hints) = (task_group.clone(), hints.clone());
        kind.connect_selected_notify(move |k| {
            g.set_visible(k.selected() == 1);
            hints();
        });
    }

    let location = adw::EntryRow::builder()
        .title("Location")
        .text(&draft.location)
        .build();
    // the video call (RFC 7986 CONFERENCE); events only
    let video = adw::EntryRow::builder()
        .title("Video call link")
        .text(draft.video.as_deref().unwrap_or_default())
        .visible(draft.kind == Kind::Event)
        .build();
    {
        let v = video.clone();
        kind.connect_selected_notify(move |k| v.set_visible(k.selected() == 0));
    }
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
    // a journal entry is dated, not timed: no end, no guests or call
    if draft.kind == Kind::Journal {
        ends.set_visible(false);
        zones.end.set_visible(false);
        video.set_visible(false);
    }
    when.add(&zones.start);
    when.add(&zones.end);
    for row in repeat.rows() {
        when.add(&row);
    }
    let more = adw::PreferencesGroup::new();
    more.add(&location);
    more.add(&video);
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
    page.add(&task_group);
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
        let video = video.clone();
        let (zones, priority, done) = (zones.clone(), priority.clone(), done.clone());
        let (parent_row, parents) = (parent_row.clone(), parents.clone());
        move || {
            let is_all_day = all_day.is_active();
            starts.remove_css_class("error");
            ends.remove_css_class("error");
            let kind = if kind.selected() == 1 {
                Kind::Todo
            } else {
                original.kind
            };
            // a task may leave either date empty
            let task = kind == Kind::Todo;
            let (start_text, end_text) = (starts.text(), ends.text());
            let has_start = !(task && start_text.trim().is_empty());
            // a journal entry has only its date
            let has_due = kind != Kind::Journal && !(task && end_text.trim().is_empty());
            // times are read in their zones; all-day ones are plain dates
            let start = if !has_start {
                None
            } else {
                let start = if is_all_day {
                    format::parse_input(&start_text, true, &original.start, local)
                } else {
                    zones.parse_start(&start_text, &original.start)
                };
                if start.is_none() {
                    starts.add_css_class("error");
                    starts.grab_focus();
                    return;
                }
                start
            };
            let end = if !has_due {
                None
            } else {
                let base = start.unwrap_or(original.start);
                let end = if is_all_day {
                    format::parse_input(&end_text, true, &base, local)
                } else {
                    zones.parse_end(&end_text, &base)
                };
                let end = match end {
                    // shown inclusive, stored exclusive
                    Some(When::Date(d)) => d.checked_add_days(Days::new(1)).map(When::Date),
                    other => other,
                };
                let ok = end.filter(|e| start.is_none_or(|s| e.to_utc(local) >= s.to_utc(local)));
                if ok.is_none() {
                    ends.add_css_class("error");
                    ends.grab_focus();
                    return;
                }
                ok
            };
            // a task with one date (or none) still has a start and an
            // exclusive end to show it by
            let day = TimeDelta::days(1);
            let (start, end) = match (start, end) {
                (Some(s), Some(e)) => (s, e),
                (Some(s), None) => (s, if s.is_date() { s.add(day) } else { s }),
                (None, Some(e)) => (if e.is_date() { e.add(-day) } else { e }, e),
                (None, None) => (original.start, original.end),
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
                // the organiser's to change on an invitation
                video: (kind == Kind::Event && !invitation).then(|| video.text().trim().to_owned()),
                description: buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .trim_end()
                    .to_owned(),
                start,
                end,
                repeat: rule,
                // the zones chosen here are the ones to write
                exact_zones: !is_all_day,
                has_start,
                has_due,
                priority: match (task, priority.selected()) {
                    (true, 1) => Some(1),
                    (true, 2) => Some(5),
                    (true, 3) => Some(9),
                    _ => None,
                },
                done: (task && done.is_visible()).then(|| done.is_active()),
                parent: match parent_row.selected() {
                    0 => None,
                    i => parents.borrow().get(i as usize - 1).cloned(),
                },
            };
            dialog.close();
            on_save(draft, calendar);
        }
    });

    let s = do_save.clone();
    save.connect_clicked(move |_| s());
    for row in [&title, &starts, &ends, &location, &video] {
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

/// The start and end rows' titles: an event's, or a task's optional dates.
fn labels(task: bool) -> (&'static str, &'static str) {
    if task {
        (
            "Starts (optional)",
            "Due (optional; time, date or +duration)",
        )
    } else {
        ("Starts", "Ends  (time, date or +duration)")
    }
}

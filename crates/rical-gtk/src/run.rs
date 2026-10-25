//! Running commands: from the command line, key bindings and buttons.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{TimeDelta, Utc};
use gtk::{gdk, glib};
use rical_core::{
    Kind, When,
    input::parse_at,
    rule::{End, Freq, Rule, parse_weekday},
    write::{Draft, Repeat, Scope},
};

use super::{Window, palette, search, section};
use crate::{
    binds::{self, Key, Lookup},
    cmds::{self, parse_date, parse_shift},
    command::{Parsed, Registry},
    details, format, help,
    layout::ViewKind,
};

pub const REGISTRY: Registry = Registry { specs: cmds::SPECS };

impl Window {
    /// Run one command line, e.g. "view week" or "new --at 14:00 Lunch".
    pub fn execute(self: &Rc<Self>, line: &str) -> Result<(), String> {
        let ctx = cmds::context(self.state.borrow().view);
        let p = REGISTRY.parse(line, ctx).map_err(|e| e.to_string())?;
        self.run(&p)
    }

    fn run(self: &Rc<Self>, p: &Parsed) -> Result<(), String> {
        let scope = |i: usize| match p.arg(i) {
            Some("this") => Some(Scope::This),
            Some("all") => Some(Scope::All),
            _ => None,
        };
        match p.spec {
            "today" => self.today(),
            "next" => self.step(true),
            "prev" => self.step(false),
            "view" => {
                let name = p.arg(0).unwrap();
                let view = self.state.borrow().view_named(name).ok_or(format!(
                    "no view “{name}” (try day, week, workweek, 3days, month)"
                ))?;
                self.set_view(view);
            }
            "week" => {
                let name = p.arg(0).unwrap();
                let view = ViewKind::from_name(name)
                    .filter(|v| {
                        matches!(v, ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_))
                    })
                    .ok_or(format!(
                        "“{name}” is not a week view (fullweek, workweek, Ndays)"
                    ))?;
                let in_week = {
                    let mut s = self.state.borrow_mut();
                    s.week_view = view;
                    matches!(
                        s.view,
                        ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_)
                    )
                };
                if in_week {
                    self.set_view(view);
                }
                self.toast(&format!("The week view shows {}", view.name()));
            }
            "goto" => {
                let today = self.today_date();
                let text = p.arg(0).unwrap();
                let date = parse_date(text, today).ok_or(format!("not a date: “{text}”"))?;
                {
                    let mut s = self.state.borrow_mut();
                    s.date = date;
                    s.selected = None;
                }
                self.refresh();
            }
            "focus" => {
                let text = p.arg(0).unwrap();
                let days: i64 = text
                    .trim_start_matches('+')
                    .parse()
                    .map_err(|_| format!("not a number of days: “{text}”"))?;
                self.move_day(days);
            }
            "select" => match p.arg(0).unwrap() {
                "next" => self.select_relative(true),
                "prev" => self.select_relative(false),
                "first" => {
                    self.state.borrow_mut().select_first_on_date();
                    self.refresh();
                }
                _ => {
                    if self.state.borrow().selected.is_none() && !details::close() {
                        return Ok(());
                    }
                    self.state.borrow_mut().selected = None;
                    self.refresh();
                }
            },
            "open" => {
                let view = self.state.borrow().view;
                // the year shows days, not events: open the focused one
                if view == ViewKind::Year {
                    self.state.borrow_mut().selected = None;
                    self.set_view(ViewKind::Day);
                // tasks and journal entries are chosen in their own lists
                } else if !view.is_calendar() || self.state.borrow().selected.is_some() {
                    self.show_details();
                } else if view != ViewKind::Day {
                    self.set_view(ViewKind::Day);
                }
            }
            "details" => self.show_details(),
            "reveal" => self.reveal()?,
            "link" => {
                let occ = self.current_item().ok_or("select an event first")?;
                let url =
                    format::join_url(&occ).ok_or("it has no link in its location or notes")?;
                details::open_uri(Some(self.window.upcast_ref()), &url);
            }
            "map" => {
                let occ = self.current_item().ok_or("select an event first")?;
                let place = format::place(&occ).ok_or("it has no location")?;
                details::open_uri(Some(self.window.upcast_ref()), &format::map_url(place));
            }
            "new" => self.run_new(p)?,
            "edit" => self.edit_selected(),
            "invite" | "uninvite" => {
                let words = p.arg(0).unwrap_or_default();
                let known = self.state.borrow().people();
                let mut who = Vec::new();
                for word in words.split_whitespace() {
                    // an address, or a name only one person matches
                    let guest = crate::guests::typed_guest(word)
                        .map(|mut g| {
                            g.name = known
                                .iter()
                                .find(|k| k.guest.email == g.email)
                                .and_then(|k| k.guest.name.clone());
                            g
                        })
                        .or_else(
                            || match rical_core::people::suggest(&known, word, &[], 2)[..] {
                                [one] => Some(one.guest.clone()),
                                _ => None,
                            },
                        )
                        .ok_or(format!("who is “{word}”? Give an address"))?;
                    who.push(guest);
                }
                if p.spec == "invite" {
                    self.change_guests(who, &[])?;
                } else {
                    let emails = who.into_iter().map(|g| g.email).collect::<Vec<_>>();
                    self.change_guests(Vec::new(), &emails)?;
                }
            }
            "raw" => {
                let occ = self.current_item().ok_or("select an event first")?;
                details::close();
                crate::raw::dialog(&occ).present(Some(&self.window));
            }
            "accept" => self.reply("ACCEPTED")?,
            "maybe" => self.reply("TENTATIVE")?,
            "decline" => self.reply("DECLINED")?,
            "done" => self.toggle_done()?,
            "task" => {
                let calendar = match p.flag("calendar") {
                    Some(name) => self.find_calendar(name)?,
                    None => self
                        .state
                        .borrow()
                        .default_calendar()
                        .ok_or("there are no calendars to add to")?,
                };
                let due = match p.flag("due") {
                    None | Some("none") => None,
                    Some(text) => Some(When::Date(
                        parse_date(text, self.today_date())
                            .ok_or(format!("not a date: “{text}”"))?,
                    )),
                };
                self.add_task(calendar, p.arg(0).unwrap(), due, None);
            }
            "up" => self.go_to_parent()?,
            "subtask-of" => {
                let text = p.arg(0).unwrap().trim();
                let parent = if text.eq_ignore_ascii_case("none") {
                    None
                } else {
                    // the open task with that title, or the only one it is
                    // part of, in the same calendar (subtasks never cross)
                    let calendar = self.selected_task().ok_or("select a task first")?.calendar;
                    let tasks = self.state.borrow().vdir.tasks();
                    let open = tasks
                        .iter()
                        .filter(|t| !t.is_done() && t.calendar == calendar);
                    let exact = open
                        .clone()
                        .filter(|t| t.summary.eq_ignore_ascii_case(text))
                        .collect::<Vec<_>>();
                    let lower = text.to_lowercase();
                    let partial = open
                        .filter(|t| t.summary.to_lowercase().contains(&lower))
                        .collect::<Vec<_>>();
                    match (exact.as_slice(), partial.as_slice()) {
                        ([one], _) | ([], [one]) => Some(one.uid.clone()),
                        ([], []) => {
                            return Err(format!("no open task called “{text}” in this calendar"));
                        }
                        _ => return Err(format!("more than one task matches “{text}”")),
                    }
                };
                self.set_parent(parent)?;
            }
            "subtask" => {
                // under the selected task, in its calendar
                let parent = self.selected_task().ok_or("select a task first")?;
                let due = match p.flag("due") {
                    None | Some("none") => None,
                    Some(text) => {
                        let (focused, local) = {
                            let s = self.state.borrow();
                            (s.date, s.local)
                        };
                        Some(
                            parse_at(text, focused, self.today_date(), local)
                                .ok_or(format!("not a date: “{text}”"))?,
                        )
                    }
                };
                self.add_task(parent.calendar, p.arg(0).unwrap(), due, Some(parent.uid));
            }
            "schedule" => {
                let (local, focused) = {
                    let s = self.state.borrow();
                    (s.local, s.date)
                };
                let at = p.arg(0).unwrap();
                let start = parse_at(at, focused, self.today_date(), local)
                    .filter(|w| !w.is_date())
                    .ok_or(format!("not a time: “{at}” (e.g. 14:00 or fri 09:30)"))?;
                let length = match p.flag("for") {
                    Some(text) => parse_shift(text)
                        .filter(|d| *d > TimeDelta::zero())
                        .ok_or(format!("not a duration: “{text}”"))?,
                    None => TimeDelta::hours(1),
                };
                self.schedule_selected(start, length)?;
            }
            "due" | "start" => {
                // a date (all day), or a date and time
                let text = p.arg(0).unwrap().trim();
                let when = match text {
                    "none" => None,
                    _ => {
                        let (focused, local) = {
                            let s = self.state.borrow();
                            (s.date, s.local)
                        };
                        Some(
                            rical_core::input::parse_at(text, focused, self.today_date(), local)
                                .ok_or(format!("not a date: “{text}”"))?,
                        )
                    }
                };
                if p.spec == "due" {
                    self.set_selected_due(when)?;
                } else {
                    self.set_selected_start(when)?;
                }
            }
            "delete" => {
                self.forced_scope.set(scope(0));
                self.delete_selected();
            }
            "move" | "resize" => {
                let text = p.arg(0).unwrap();
                let shift = parse_shift(text).ok_or(format!("not a duration: “{text}”"))?;
                self.forced_scope.set(scope(1));
                self.nudge_selected(shift, p.spec == "resize");
            }
            "repeat" => {
                let today = self.today_date();
                let repeat = parse_repeat(p, today)?;
                self.set_repeat(repeat)?;
            }
            "undo" => self.undo(),
            "redo" => self.redo(),
            "calendar" => self.run_calendar(p.arg(0).unwrap(), p.arg(1))?,
            "sidebar" => {
                let show = match p.arg(0) {
                    Some("show") => true,
                    Some("hide") => false,
                    _ => !self.split.shows_sidebar(),
                };
                self.split.set_show_sidebar(show);
            }
            "zones" => {
                {
                    let mut s = self.state.borrow_mut();
                    let text = p.arg(0).unwrap_or("toggle").trim();
                    let (word, rest) = text.split_once(' ').unwrap_or((text, ""));
                    let rest = Some(rest.trim()).filter(|r| !r.is_empty());
                    match (word, rest) {
                        ("toggle", None) => s.zones_shown = !s.zones_shown,
                        ("show", None) => s.zones_shown = true,
                        ("hide", None) => s.zones_shown = false,
                        ("none" | "clear", None) => s.zones.clear(),
                        ("add", Some(list)) => {
                            for z in crate::zones::Zone::parse_list(list)? {
                                if !s.zones.contains(&z) {
                                    s.zones.push(z);
                                }
                            }
                            s.zones_shown = true;
                        }
                        ("remove", Some(name)) => {
                            let before = s.zones.len();
                            s.zones.retain(|z| {
                                !(z.tz.name().eq_ignore_ascii_case(name)
                                    || z.label.eq_ignore_ascii_case(name))
                            });
                            if s.zones.len() == before {
                                return Err(format!("“{name}” is not shown"));
                            }
                        }
                        ("toggle" | "show" | "hide" | "none" | "clear" | "add" | "remove", _) => {
                            return Err(format!("“{text}”: see :help zones"));
                        }
                        // anything else is a list of zones to show instead
                        _ => {
                            s.zones = crate::zones::Zone::parse_list(text)?;
                            s.zones_shown = true;
                        }
                    }
                    if s.zones.is_empty() && s.zones_shown && word != "none" && word != "clear" {
                        drop(s);
                        self.toast("No extra zones: set zones = … in the config, or :zones NAME");
                    }
                }
                self.refresh();
            }
            "reload" => self.reload_all(),
            "section" => {
                let name = p.arg(0).unwrap();
                let sections = self.sections.borrow();
                if let Some(s) = sections.iter().find(|s| s.name == name) {
                    s.set_expanded(match p.arg(1) {
                        Some("show") => true,
                        Some("hide") => false,
                        _ => !s.is_expanded(),
                    });
                    self.split.set_show_sidebar(true);
                }
                section::save_collapsed(&sections);
            }
            "notifications" => {
                let on = {
                    let mut s = self.state.borrow_mut();
                    s.notifications = match p.arg(0) {
                        Some("on") => true,
                        Some("off") => false,
                        _ => !s.notifications,
                    };
                    s.notifications
                };
                self.toast(if on { "Reminders on" } else { "Reminders off" });
            }
            "remind" => {
                let text = p.arg(0).unwrap();
                let before = match text {
                    "none" | "off" => None,
                    _ => Some(
                        parse_shift(text)
                            .filter(|d| *d > TimeDelta::zero())
                            .ok_or(format!("not a duration: “{text}”"))?,
                    ),
                };
                self.state.borrow_mut().default_reminder = before;
                self.toast(&match before {
                    Some(_) => format!("Reminding {text} before events without alarms"),
                    None => "Only events with alarms remind".to_owned(),
                });
            }
            "search" => search::open(self, p.arg(0).unwrap_or_default()),
            "help" => {
                help::dialog(&self.binds, self.state.borrow().view).present(Some(&self.window))
            }
            "quit" => {
                if let Some(app) = self.window.application() {
                    app.quit();
                }
            }
            other => return Err(format!("“{other}” is not implemented")),
        }
        // a forced scope is only for the command that set it
        self.forced_scope.set(None);
        Ok(())
    }

    fn today_date(&self) -> chrono::NaiveDate {
        Utc::now()
            .with_timezone(&self.state.borrow().local)
            .date_naive()
    }

    /// `new [--calendar C] [--at WHEN] [--for DURATION] [--kind K] [title]`:
    /// with a title the item is created straight away, otherwise the editor
    /// opens with these values.
    fn run_new(self: &Rc<Self>, p: &Parsed) -> Result<(), String> {
        let (local, focused) = {
            let s = self.state.borrow();
            (s.local, s.date)
        };
        let calendar = match p.flag("calendar") {
            Some(name) => self.find_calendar(name)?,
            None => self
                .state
                .borrow()
                .default_calendar()
                .ok_or("there are no calendars to add to")?,
        };
        let (mut start, _) = self.default_slot();
        if let Some(at) = p.flag("at") {
            start = parse_at(at, focused, self.today_date(), local)
                .ok_or(format!("not a time: “{at}”"))?;
        }
        let length = match p.flag("for") {
            Some(text) => parse_shift(text)
                .filter(|d| *d >= TimeDelta::zero())
                .ok_or(format!("not a duration: “{text}”"))?,
            None if start.is_date() => TimeDelta::days(1),
            None => TimeDelta::hours(1),
        };
        let mut draft = Draft::new(start, start.add(length));
        if let Some(location) = p.flag("location") {
            draft.location = location.to_owned();
        }
        if p.flag("kind") == Some("todo") {
            draft.kind = Kind::Todo;
        }
        match p.arg(0) {
            Some(title) => {
                draft.summary = title.to_owned();
                self.create(draft, calendar);
            }
            None => self.open_new(draft, calendar),
        }
        Ok(())
    }

    /// A calendar by number (1-based), folder name or display name.
    fn find_calendar(&self, name: &str) -> Result<String, String> {
        let s = self.state.borrow();
        let cals = &s.vdir.calendars;
        name.parse::<usize>()
            .ok()
            .and_then(|n| cals.get(n.wrapping_sub(1)))
            .or_else(|| cals.iter().find(|c| c.id == name))
            .or_else(|| cals.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
            .map(|c| c.id.clone())
            .ok_or(format!("no calendar “{name}”"))
    }

    fn run_calendar(self: &Rc<Self>, action: &str, name: Option<&str>) -> Result<(), String> {
        let id = match (action, name) {
            ("all", _) => None,
            (_, Some(name)) => Some(self.find_calendar(name)?),
            (_, None) => return Err(format!("calendar {action}: which calendar?")),
        };
        {
            let mut s = self.state.borrow_mut();
            let all = s
                .vdir
                .calendars
                .iter()
                .map(|c| c.id.clone())
                .collect::<Vec<_>>();
            match (action, id) {
                ("all", _) => s.hidden.clear(),
                ("only", Some(id)) => {
                    s.hidden = all.into_iter().filter(|c| *c != id).collect();
                }
                ("show", Some(id)) => {
                    s.hidden.remove(&id);
                }
                ("hide", Some(id)) => {
                    s.hidden.insert(id);
                }
                // toggle
                (_, Some(id)) if !s.hidden.remove(&id) => {
                    s.hidden.insert(id);
                }
                _ => {}
            }
            s.invalidate();
        }
        self.sync_calendar_list();
        self.refresh();
        Ok(())
    }

    /// Run the commands a key binding maps to, then maybe open the command
    /// line prefilled.
    fn run_action(self: &Rc<Self>, action: &binds::Action) {
        for line in &action.run {
            if let Err(e) = self.execute(line) {
                self.toast(&e);
                return;
            }
        }
        if let Some(text) = &action.prefill {
            palette::open(self, text);
        }
    }

    /// Key presses go to the bindings, except while typing in a text field
    /// or a dialog is open. Calendar keys (arrows, Enter...) only apply
    /// when the calendar has focus.
    pub(super) fn handle_key(
        self: &Rc<Self>,
        keyval: gdk::Key,
        mods: gdk::ModifierType,
    ) -> glib::Propagation {
        use glib::Propagation::{Proceed, Stop};

        if self.window.visible_dialog().is_some() {
            return Proceed;
        }
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window);
        if focus.as_ref().is_some_and(|f| {
            f.is::<gtk::Text>() || f.is::<gtk::Editable>() || f.is::<gtk::TextView>()
        }) {
            return Proceed;
        }
        let Some(key) = key_of(keyval, mods) else {
            return Proceed;
        };
        let in_views = focus.as_ref().is_none_or(|f| {
            f.is_ancestor(&self.views) || f == self.views.upcast_ref::<gtk::Widget>()
        });
        if !in_views && is_navigation(&key) {
            self.pending_keys.borrow_mut().clear();
            return Proceed;
        }

        let section = cmds::section(self.state.borrow().view);
        let mut pending = self.pending_keys.borrow_mut();
        pending.push(key.clone());
        let mut found = self.binds.lookup(section, &pending);
        if matches!(found, Lookup::None) && pending.len() > 1 {
            // the sequence went nowhere: start again from this key
            pending.clear();
            pending.push(key);
            found = self.binds.lookup(section, &pending);
        }
        match found {
            Lookup::Found(action) => {
                pending.clear();
                drop(pending);
                self.run_action(&action);
                Stop
            }
            Lookup::Prefix => Stop,
            Lookup::None => {
                pending.clear();
                Proceed
            }
        }
    }
}

fn is_navigation(key: &Key) -> bool {
    !key.ctrl
        && !key.alt
        && matches!(
            key.name.as_str(),
            "up" | "down"
                | "left"
                | "right"
                | "enter"
                | "space"
                | "esc"
                | "pageup"
                | "pagedown"
                | "tab"
                | "home"
                | "end"
        )
}

/// The binding key for a GDK key press; None for bare modifiers.
pub fn key_of(keyval: gdk::Key, mods: gdk::ModifierType) -> Option<Key> {
    use gdk::Key as K;
    let named = match keyval {
        K::Return | K::KP_Enter | K::ISO_Enter => "enter",
        K::Escape => "esc",
        K::space | K::KP_Space => "space",
        K::Tab | K::ISO_Left_Tab | K::KP_Tab => "tab",
        K::BackSpace => "backspace",
        K::Delete | K::KP_Delete => "delete",
        K::Insert | K::KP_Insert => "insert",
        K::Up | K::KP_Up => "up",
        K::Down | K::KP_Down => "down",
        K::Left | K::KP_Left => "left",
        K::Right | K::KP_Right => "right",
        K::Home | K::KP_Home => "home",
        K::End | K::KP_End => "end",
        K::Page_Up | K::KP_Page_Up => "pageup",
        K::Page_Down | K::KP_Page_Down => "pagedown",
        K::F1 => "f1",
        K::F2 => "f2",
        K::F3 => "f3",
        K::F4 => "f4",
        K::F5 => "f5",
        K::F6 => "f6",
        K::F7 => "f7",
        K::F8 => "f8",
        K::F9 => "f9",
        K::F10 => "f10",
        K::F11 => "f11",
        K::F12 => "f12",
        _ => "",
    };
    let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = mods.contains(gdk::ModifierType::ALT_MASK);
    if !named.is_empty() {
        return Some(Key {
            name: named.to_owned(),
            ctrl,
            alt,
            shift: mods.contains(gdk::ModifierType::SHIFT_MASK),
        });
    }
    let c = keyval.to_unicode().filter(|c| !c.is_control())?;
    Some(Key {
        name: c.to_string(),
        ctrl,
        alt,
        shift: false,
    })
}

/// The rule `:repeat` describes.
fn parse_repeat(p: &Parsed, today: chrono::NaiveDate) -> Result<Repeat, String> {
    let text = p.arg(0).unwrap().trim();
    if text.to_ascii_uppercase().starts_with("FREQ=") {
        return Ok(Repeat::from_rrule(Some(text)));
    }
    let freq = match text {
        "none" | "never" => return Ok(Repeat::None),
        _ => Freq::ALL
            .into_iter()
            .find(|f| f.name() == text)
            .ok_or(format!(
                "“{text}”: say none, daily, weekly, monthly, yearly or an RRULE"
            ))?,
    };
    let mut rule = Rule::new(freq);
    if let Some(n) = p.flag("every") {
        rule.interval = n
            .parse()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or(format!("--every: not a number: “{n}”"))?;
    }
    if let Some(days) = p.flag("on") {
        if freq != Freq::Weekly {
            return Err("--on only goes with weekly".into());
        }
        rule.weekdays = days
            .split(',')
            .map(|d| parse_weekday(d).ok_or(format!("--on: not a weekday: “{d}”")))
            .collect::<Result<_, _>>()?;
    }
    match (p.flag("count"), p.flag("until")) {
        (Some(_), Some(_)) => return Err("give --count or --until, not both".into()),
        (Some(n), None) => {
            rule.end = End::Count(
                n.parse()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or(format!("--count: not a number: “{n}”"))?,
            )
        }
        (None, Some(d)) => {
            rule.end =
                End::Until(parse_date(d, today).ok_or(format!("--until: not a date: “{d}”"))?)
        }
        (None, None) => {}
    }
    Ok(Repeat::Rule(rule))
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn gdk_keys() {
        use gdk::ModifierType as M;
        let k = |kv, m| key_of(kv, m).map(|k| k.to_string());
        assert_eq!(k(gdk::Key::t, M::empty()).as_deref(), Some("t"));
        assert_eq!(k(gdk::Key::T, M::SHIFT_MASK).as_deref(), Some("T"));
        assert_eq!(
            k(gdk::Key::Z, M::SHIFT_MASK | M::CONTROL_MASK).as_deref(),
            Some("<C-Z>")
        );
        assert_eq!(
            k(gdk::Key::Down, M::SHIFT_MASK).as_deref(),
            Some("<S-Down>")
        );
        assert_eq!(k(gdk::Key::bracketright, M::empty()).as_deref(), Some("]"));
        assert_eq!(k(gdk::Key::_1, M::ALT_MASK).as_deref(), Some("<A-1>"));
        assert_eq!(k(gdk::Key::Shift_L, M::empty()), None);
    }

    #[test]
    fn repeat_commands() {
        let today = chrono::NaiveDate::from_ymd_opt(2025, 3, 1).unwrap();
        let rep = |line: &str| {
            let p = REGISTRY.parse(line, cmds::ALL).map_err(|e| e.to_string())?;
            parse_repeat(&p, today)
        };
        let Ok(Repeat::Rule(r)) = rep("repeat weekly --on mo,we --count 3") else {
            panic!()
        };
        assert_eq!(r.to_string(), "every week on Mon, Wed, 3 times");
        let Ok(Repeat::Rule(r)) = rep("repeat monthly --every 2 --until 2025-12-31") else {
            panic!()
        };
        assert_eq!(r.to_string(), "every 2 months, until 31 Dec 2025");
        assert_eq!(rep("repeat none"), Ok(Repeat::None));
        assert!(matches!(
            rep("repeat FREQ=MONTHLY;BYDAY=1TU"),
            Ok(Repeat::Custom(_))
        ));
        assert!(rep("repeat daily --on mo").is_err());
        assert!(rep("repeat fortnightly").is_err());
        assert!(rep("repeat weekly --count 2 --until fri").is_err());
    }
}

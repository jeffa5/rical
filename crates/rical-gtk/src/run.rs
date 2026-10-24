//! Running commands: from the command line, key bindings and buttons.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{TimeDelta, Utc};
use gtk::{gdk, glib};
use rical_core::{
    Kind, When,
    input::parse_at,
    write::{Draft, Scope},
};

use super::{Window, palette, search};
use crate::{
    binds::{self, Key, Lookup},
    cmds::{self, parse_date, parse_shift},
    command::{Parsed, Registry},
    details, help,
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
            "view" => self.set_view(ViewKind::from_name(p.arg(0).unwrap()).unwrap()),
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
                if self.state.borrow().selected.is_some() {
                    self.show_details();
                } else if view != ViewKind::Day {
                    self.set_view(ViewKind::Day);
                }
            }
            "details" => self.show_details(),
            "new" => self.run_new(p)?,
            "edit" => self.edit_selected(),
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
                self.add_task(calendar, p.arg(0).unwrap(), due);
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
            "due" => {
                let text = p.arg(0).unwrap();
                let due = match text {
                    "none" => None,
                    _ => Some(When::Date(
                        parse_date(text, self.today_date())
                            .ok_or(format!("not a date: “{text}”"))?,
                    )),
                };
                self.set_selected_due(due)?;
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
            "reload" => self.reload_all(),
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
}

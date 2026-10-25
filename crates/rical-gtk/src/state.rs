//! Application state shared by the views.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    ops::Range,
    rc::Rc,
};

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use rical_core::{
    Calendar, Kind, Occurrence, Rgb, Vdir, When,
    people::Known,
    write::{Change, Guest},
};

use crate::layout::{self, ViewKind};

/// Identifies one occurrence across reloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OccurrenceKey {
    pub uid: String,
    pub path: std::path::PathBuf,
    pub start: When,
}

impl OccurrenceKey {
    pub fn of(o: &Occurrence) -> Self {
        Self {
            uid: o.uid.clone(),
            path: o.path.clone(),
            start: o.recurrence_id.unwrap_or(o.start),
        }
    }

    pub fn matches(&self, o: &Occurrence) -> bool {
        self.uid == o.uid && self.path == o.path && self.start == o.recurrence_id.unwrap_or(o.start)
    }
}

pub struct State {
    pub vdir: Vdir,
    pub local: Tz,
    pub hidden: BTreeSet<String>,
    pub view: ViewKind,
    /// The focused day; views are positioned around it.
    pub date: NaiveDate,
    pub selected: Option<OccurrenceKey>,
    /// In the tasks view: (file, UID) of the selected task.
    pub selected_task: Option<(std::path::PathBuf, String)>,
    /// The journal entry selected in the journal view, as (file, UID).
    pub selected_journal: Option<(std::path::PathBuf, String)>,
    /// Calendar new items go in; the last one used.
    pub last_calendar: Option<String>,
    /// Each entry is the file changes one action made.
    pub undo: Vec<Vec<Change>>,
    pub redo: Vec<Vec<Change>>,
    /// Whether to show reminders.
    pub notifications: bool,
    /// What "week" means: the full week, the work week or a number of days.
    pub week_view: ViewKind,
    /// The calendar view last shown, to go back to from tasks or invitations.
    pub calendar_view: ViewKind,
    /// Reminder for timed events that have no alarms of their own.
    pub default_reminder: Option<chrono::TimeDelta>,
    /// Bumped whenever the data or the hidden set changes.
    generation: u64,
    cache: RefCell<Option<Cache>>,
    /// Each calendar's colour, for a generation.
    colors: RefCell<(Option<u64>, Vec<Rgb>)>,
    /// First day of the N-day view; see `visible_days`.
    days_from: Cell<NaiveDate>,
    /// My addresses, from the `me` setting; guessed from the vdir when empty.
    pub me: Vec<String>,
    guessed_me: RefCell<(Option<u64>, Vec<String>)>,
    /// A folder of vCards to suggest guests from.
    pub contacts: Option<std::path::PathBuf>,
    /// Extra time zones beside the grid's hour labels, and whether shown.
    pub zones: Vec<crate::zones::Zone>,
    pub zones_shown: bool,
    people: RefCell<(Option<u64>, Rc<Vec<Known>>)>,
}

/// An event someone else organised, with me (perhaps) invited.
pub struct Invite<'a> {
    pub from: &'a rical_core::Person,
    /// My PARTSTAT, if I am among the attendees.
    pub reply: Option<&'a str>,
}

impl Invite<'_> {
    pub fn needs_reply(&self) -> bool {
        self.reply == Some("NEEDS-ACTION")
    }
}

struct Cache {
    generation: u64,
    range: Range<DateTime<Utc>>,
    today: NaiveDate,
    occurrences: Rc<Vec<Occurrence>>,
}

pub type SharedState = Rc<RefCell<State>>;

impl State {
    pub fn new(vdir: Vdir, local: Tz, date: NaiveDate) -> Self {
        Self {
            vdir,
            local,
            hidden: BTreeSet::new(),
            view: ViewKind::Week,
            date,
            selected: None,
            selected_task: None,
            selected_journal: None,
            last_calendar: None,
            undo: Vec::new(),
            redo: Vec::new(),
            notifications: true,
            week_view: ViewKind::Week,
            calendar_view: ViewKind::Week,
            default_reminder: None,
            generation: 0,
            cache: RefCell::new(None),
            colors: RefCell::new((None, Vec::new())),
            days_from: Cell::new(date),
            me: Vec::new(),
            guessed_me: RefCell::new((None, Vec::new())),
            contacts: None,
            zones: Vec::new(),
            zones_shown: true,
            people: RefCell::new((None, Rc::new(Vec::new()))),
        }
    }

    /// Call after changing `vdir` or `hidden`.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.vdir.calendar(id)
    }

    /// The calendar's colour, nudged if another calendar looks the same.
    pub fn color(&self, calendar: &str) -> Rgb {
        let mut colors = self.colors.borrow_mut();
        if colors.0 != Some(self.generation) {
            let own = self
                .vdir
                .calendars
                .iter()
                .map(|c| c.color)
                .collect::<Vec<_>>();
            *colors = (Some(self.generation), crate::colors::assign(&own));
        }
        let index = self
            .vdir
            .calendars
            .iter()
            .position(|c| c.id == calendar)
            .unwrap_or(0);
        colors.1.get(index).copied().unwrap_or(Rgb {
            r: 0x35,
            g: 0x84,
            b: 0xe4,
        })
    }

    /// My addresses: as given, or guessed from who is on most invitations.
    pub fn my_addresses(&self) -> Vec<String> {
        let mut guess = self.guessed_me.borrow_mut();
        if guess.0 != Some(self.generation) {
            // as given (else guessed), and those my servers send as
            let mut all = if self.me.is_empty() {
                self.vdir.owner_addresses()
            } else {
                self.me.iter().map(|m| m.to_lowercase()).collect()
            };
            for e in self.vdir.senders().into_values() {
                if !all.contains(&e) {
                    all.push(e);
                }
            }
            *guess = (Some(self.generation), all);
        }
        guess.1.clone()
    }

    /// Whether `o` is an invitation: organised by someone else, with
    /// attendees. My reply is known when I am one of them.
    pub fn invitation<'a>(&self, o: &'a Occurrence) -> Option<Invite<'a>> {
        let from = o.organizer.as_ref()?;
        if o.attendees.is_empty() {
            return None;
        }
        let me = self.my_addresses();
        if from.is_any_of(&me) {
            return None;
        }
        let reply = o
            .attendees
            .iter()
            .find(|a| a.is_any_of(&me))
            // RFC 5545: no PARTSTAT means not yet answered
            .map(|a| a.status.as_deref().unwrap_or("NEEDS-ACTION"));
        Some(Invite { from, reply })
    }

    /// The task meant now, as (file, UID): selected in the tasks view, or
    /// a selected task occurrence elsewhere.
    pub fn selected_task_id(&self) -> Option<(std::path::PathBuf, String)> {
        if self.view == ViewKind::Tasks {
            return self.selected_task.clone();
        }
        self.selected_occurrence()
            .filter(|o| o.kind == Kind::Todo)
            .map(|o| (o.path, o.uid))
    }

    /// Me as the organiser of events in `calendar`. Best is the address
    /// its server has sent invitations as before, as a scheduling server
    /// only sends for that one; then one of mine seen on its invitations;
    /// then my first.
    pub fn organizer_for(&self, calendar: &str) -> Option<Guest> {
        let email = self.vdir.senders().remove(calendar).or_else(|| {
            let owners = self.vdir.owners();
            let seen = owners.get(calendar);
            if self.me.is_empty() {
                seen.cloned()
                    .or_else(|| self.my_addresses().into_iter().next())
            } else {
                seen.filter(|e| self.me.iter().any(|m| m.eq_ignore_ascii_case(e)))
                    .cloned()
                    .or_else(|| self.me.first().cloned())
            }
        })?;
        Some(Guest { email, name: None })
    }

    /// People to invite: from events (most frequent first), then contacts.
    pub fn people(&self) -> Rc<Vec<Known>> {
        let mut cache = self.people.borrow_mut();
        if cache.0 != Some(self.generation) {
            let mut known = self.vdir.people(&self.my_addresses());
            if let Some(dir) = &self.contacts {
                for guest in rical_core::people::contacts(dir) {
                    match known.iter_mut().find(|k| k.guest.email == guest.email) {
                        // a contact's name beats one from an invitation
                        Some(k) if guest.name.is_some() => k.guest.name = guest.name,
                        Some(_) => {}
                        None => known.push(Known { guest, count: 0 }),
                    }
                }
            }
            *cache = (Some(self.generation), Rc::new(known));
        }
        cache.1.clone()
    }

    /// The days on screen. The N-day view keeps its first day while the
    /// focused day stays among them, so selecting something on another
    /// shown day does not scroll it; it starts afresh from a day outside.
    pub fn visible_days(&self) -> Vec<NaiveDate> {
        // the agenda too: selecting a row below must not make it the top
        let n = match self.view {
            ViewKind::Days(n) => n as u64,
            ViewKind::Agenda => layout::AGENDA_DAYS,
            // always from today: nothing earlier needs an answer
            ViewKind::Invitations => return layout::visible_days(self.view, self.today()),
            _ => return layout::visible_days(self.view, self.date),
        };
        let from = self.days_from.get();
        let shown = self.date >= from && self.date < from + chrono::Days::new(n);
        let from = if shown { from } else { self.date };
        self.days_from.set(from);
        layout::visible_days(self.view, from)
    }

    /// Move a view-length forward or back.
    pub fn step(&mut self, forward: bool) {
        if matches!(self.view, ViewKind::Days(_) | ViewKind::Agenda) {
            let from = self.visible_days()[0];
            self.days_from.set(layout::step(self.view, from, forward));
        }
        self.date = layout::step(self.view, self.date, forward);
    }

    /// Visible occurrences (events and tasks of shown calendars) in `days`.
    /// If today is among them, tasks still to do from earlier days are
    /// carried forward onto today so they do not get lost.
    pub fn occurrences(&self, days: &[NaiveDate]) -> Rc<Vec<Occurrence>> {
        let range = layout::days_range(days, self.local);
        let today = self.today();
        if let Some(cache) = &*self.cache.borrow()
            && cache.generation == self.generation
            && cache.range == range
            && cache.today == today
        {
            return cache.occurrences.clone();
        }
        let mut occurrences = self
            .vdir
            .query(
                &range,
                &[Kind::Event, Kind::Todo, Kind::Journal],
                self.local,
            )
            .into_iter()
            .filter(|o| !self.hidden.contains(&o.calendar))
            .collect::<Vec<_>>();
        if days.contains(&today) {
            occurrences.extend(
                self.vdir
                    .tasks()
                    .into_iter()
                    .filter(|t| {
                        !self.hidden.contains(&t.calendar) && t.is_overdue(today, self.local)
                    })
                    .map(|t| t.on_day(today)),
            );
        }
        let occurrences = Rc::new(occurrences);
        *self.cache.borrow_mut() = Some(Cache {
            generation: self.generation,
            range,
            today,
            occurrences: occurrences.clone(),
        });
        occurrences
    }

    pub fn visible_occurrences(&self) -> Rc<Vec<Occurrence>> {
        self.occurrences(&self.visible_days())
    }

    pub fn selected_occurrence(&self) -> Option<Occurrence> {
        let key = self.selected.as_ref()?;
        self.visible_occurrences()
            .iter()
            .find(|o| key.matches(o))
            .cloned()
    }

    /// Visible occurrences in keyboard order: by day, all-day row first,
    /// then by start.
    pub fn navigation_order(&self) -> Vec<Occurrence> {
        if self.view == ViewKind::Invitations {
            let (open, maybe) = crate::invitations::waiting(self);
            return [open, maybe].concat();
        }
        let mut occ = self.visible_occurrences().as_ref().clone();
        let local = self.local;
        occ.sort_by_key(|o| {
            (
                o.start.date(local),
                !layout::is_all_day_row(o),
                o.start.to_utc(local),
                o.summary.clone(),
            )
        });
        occ
    }

    /// Select the next (or previous) occurrence in keyboard order.
    pub fn select_relative(&mut self, forward: bool) {
        let order = self.navigation_order();
        if order.is_empty() {
            self.selected = None;
            return;
        }
        let current = self
            .selected
            .as_ref()
            .and_then(|k| order.iter().position(|o| k.matches(o)));
        let next = match current {
            Some(i) if forward => (i + 1).min(order.len() - 1),
            Some(i) => i.saturating_sub(1),
            None => {
                // start from the focused day
                let local = self.local;
                order
                    .iter()
                    .position(|o| o.start.date(local) >= self.date)
                    .unwrap_or(0)
            }
        };
        self.date = order[next]
            .start
            .date(self.local)
            .max(self.visible_days()[0]);
        self.selected = Some(OccurrenceKey::of(&order[next]));
    }

    /// Select the first occurrence on the focused day, if any.
    pub fn select_first_on_date(&mut self) {
        let local = self.local;
        let date = self.date;
        self.selected = self
            .navigation_order()
            .iter()
            .find(|o| o.start.date(local) <= date && o.end.date(local) >= date)
            .map(OccurrenceKey::of);
    }
}

impl State {
    /// The calendar for new items: the last used, else the shown calendar
    /// with the most items (birthday calendars aside), else the first.
    pub fn default_calendar(&self) -> Option<String> {
        if let Some(id) = self
            .last_calendar
            .as_ref()
            .filter(|id| self.vdir.calendar(id).is_some())
        {
            return Some(id.clone());
        }
        let mut counts = std::collections::BTreeMap::<&str, usize>::new();
        for (cal, _) in self.vdir.components() {
            *counts.entry(cal.id.as_str()).or_default() += 1;
        }
        self.vdir
            .calendars
            .iter()
            .filter(|c| !self.hidden.contains(&c.id) && !c.id.contains("birthday"))
            .max_by_key(|c| counts.get(c.id.as_str()).copied().unwrap_or(0))
            .or_else(|| self.vdir.calendars.first())
            .map(|c| c.id.clone())
    }

    /// Record changes made by one action so it can be undone.
    pub fn record(&mut self, changes: Vec<Change>) {
        self.undo.push(changes);
        self.redo.clear();
        self.invalidate();
    }

    /// Find an occurrence of `uid` starting at `start` among visible ones.
    pub fn find(&self, uid: &str, start: &When) -> Option<Occurrence> {
        let local = self.local;
        self.visible_occurrences()
            .iter()
            .find(|o| o.uid == uid && o.start.to_utc(local) == start.to_utc(local))
            .cloned()
    }
}

/// Overdue tasks stand out in red whatever their calendar.
const OVERDUE: Rgb = Rgb {
    r: 0xe0,
    g: 0x1b,
    b: 0x24,
};

impl State {
    /// The colour to draw an occurrence in.
    pub fn display_color(&self, o: &Occurrence) -> Rgb {
        if crate::format::is_overdue(o, self.local) {
            OVERDUE
        } else {
            self.color(&o.calendar)
        }
    }

    pub fn today(&self) -> NaiveDate {
        chrono::Utc::now().with_timezone(&self.local).date_naive()
    }
}

impl State {
    /// Changes whenever what is shown could have changed (data, hidden
    /// calendars); views rebuild only then.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl State {
    /// A view by name, with "week" meaning the configured week view.
    pub fn view_named(&self, name: &str) -> Option<ViewKind> {
        match name {
            "week" => Some(self.week_view),
            "calendar" => Some(self.calendar_view),
            _ => ViewKind::from_name(name),
        }
    }
}

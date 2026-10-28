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

/// Who did it, in the history the apps share.
pub const APP: &str = "calendar";

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
    /// What was changed, to undo (shared with the other apps, and kept).
    pub history: Option<rical_core::history::History>,
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
    /// Whether the all-day row shows every lane, or folds those past a few.
    pub all_day_expanded: bool,
    /// Everyone known, by the generation and contacts they were read from.
    people: RefCell<(Option<PeopleKey>, Rc<People>)>,
    /// The contacts read, by what the folder looked like then; and their
    /// names by address (lower case).
    contact_cache: RefCell<(Option<ContactsSeen>, Rc<Contacts>)>,
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
            history: None,
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
            all_day_expanded: false,
            people: RefCell::new((None, Rc::new(Vec::new()))),
            contact_cache: RefCell::new((None, Rc::default())),
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
        let (seen, contacts) = self.contacts();
        let mut cache = self.people.borrow_mut();
        if cache.0 != Some((self.generation, seen)) {
            let mut known = self.vdir.people(&self.my_addresses());
            for guest in &contacts.guests {
                match known.iter_mut().find(|k| k.guest.email == guest.email) {
                    // a contact's name beats one from an invitation
                    Some(k) if guest.name.is_some() => k.guest.name = guest.name.clone(),
                    Some(_) => {}
                    None => known.push(Known {
                        guest: guest.clone(),
                        count: 0,
                    }),
                }
            }
            *cache = (Some((self.generation, seen)), Rc::new(known));
        }
        cache.1.clone()
    }

    /// The contacts folder's vCards, read again only when a file in it
    /// has changed (looking costs a few hundred stats, reading megabytes).
    fn contacts(&self) -> (ContactsSeen, Rc<Contacts>) {
        let Some(dir) = &self.contacts else {
            return ((0, None), Rc::default());
        };
        let seen = rical_core::people::contacts_fingerprint(dir);
        let mut cache = self.contact_cache.borrow_mut();
        if cache.0 != Some(seen) {
            let books = match rical_cards::Books::load(dir) {
                Ok(b) => Some(b),
                Err(e) => {
                    log::warn!("could not read the contacts in {dir:?}: {e:#}");
                    None
                }
            };
            let mut guests = Vec::new();
            let mut by_email = std::collections::HashMap::new();
            for (id, _, card) in books.iter().flat_map(|b| b.cards()) {
                let name = Some(card.display_name()).filter(|n| !n.is_empty());
                for e in card.emails() {
                    let email = rical_cards::text::email_key(&e.value);
                    if !email.contains('@') {
                        continue;
                    }
                    by_email.entry(email.clone()).or_insert_with(|| id.clone());
                    guests.push(Guest {
                        email,
                        name: name.clone(),
                    });
                }
            }
            guests.sort_by(|a, b| a.email.cmp(&b.email));
            guests.dedup_by(|a, b| a.email == b.email);
            let names = guests
                .iter()
                .filter_map(|g| Some((g.email.clone(), g.name.clone()?)))
                .collect();
            *cache = (
                Some(seen),
                Rc::new(Contacts {
                    guests,
                    names,
                    books,
                    by_email,
                }),
            );
        }
        (seen, cache.1.clone())
    }

    /// `f` of the contact card with `email` on it, and where it is, if any.
    pub fn with_card<T>(
        &self,
        email: &str,
        f: impl FnOnce(&rical_cards::Card, &rical_cards::CardId) -> T,
    ) -> Option<T> {
        let contacts = self.contacts().1;
        let id = contacts
            .by_email
            .get(&rical_cards::text::email_key(email))?;
        let card = contacts.books.as_ref()?.get(id)?;
        Some(f(&card, id))
    }

    /// Who someone on an event is, by the name you know them by: your
    /// contact's name, else the name the event gives, else one they go by
    /// on other events, else their address.
    pub fn name_of(&self, p: &rical_core::Person) -> String {
        let email = p.email.as_deref().map(str::to_lowercase);
        if let Some(name) = email
            .as_ref()
            .and_then(|e| self.contacts().1.names.get(e).cloned())
        {
            return name;
        }
        if let Some(name) = &p.name {
            return name.clone();
        }
        let Some(email) = email else {
            return "someone".into();
        };
        self.people()
            .iter()
            .find(|k| k.guest.email == email)
            .and_then(|k| k.guest.name.clone())
            .unwrap_or(email)
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

    /// Visible occurrences in keyboard order, each with the day it is
    /// reached on: day by day as shown, the all-day row top to bottom, then
    /// by time. Something spanning days is on each of them, so it can be
    /// reached from any.
    pub fn navigation_entries(&self) -> Vec<(NaiveDate, Occurrence)> {
        if self.view == ViewKind::Invitations {
            let (open, maybe) = crate::invitations::waiting(self);
            return [open, maybe]
                .concat()
                .into_iter()
                .map(|o| (self.date, o))
                .collect();
        }
        let days = self.visible_days();
        let occ = self.occurrences(&days);
        let local = self.local;
        let mut out = Vec::new();
        if self.view == ViewKind::Month {
            // each cell lists its week's bars in lane order
            for week in days.chunks(7) {
                let bars = layout::month_week_bars(&occ, week, local);
                for (i, day) in week.iter().enumerate() {
                    let mut on: Vec<_> = bars
                        .iter()
                        .filter(|b| b.first <= i && i <= b.last)
                        .collect();
                    on.sort_by_key(|b| b.lane);
                    out.extend(on.iter().map(|b| (*day, occ[b.index].clone())));
                }
            }
            return out;
        }
        let bars = layout::all_day_bars(&occ, &days, local);
        for (i, day) in days.iter().enumerate() {
            let mut on: Vec<_> = bars
                .iter()
                .filter(|b| b.first <= i && i <= b.last)
                .collect();
            on.sort_by_key(|b| b.lane);
            out.extend(on.iter().map(|b| (*day, occ[b.index].clone())));
            let mut segs = layout::day_segments(&occ, *day, local, 0.0);
            segs.sort_by(|a, b| {
                a.start_min
                    .total_cmp(&b.start_min)
                    .then(a.column.cmp(&b.column))
            });
            out.extend(segs.iter().map(|s| (*day, occ[s.index].clone())));
        }
        out
    }

    /// Visible occurrences in keyboard order, each once.
    pub fn navigation_order(&self) -> Vec<Occurrence> {
        let mut out: Vec<Occurrence> = Vec::new();
        for (_, o) in self.navigation_entries() {
            let key = OccurrenceKey::of(&o);
            if !out.iter().any(|seen| key.matches(seen)) {
                out.push(o);
            }
        }
        out
    }

    /// Select the next (or previous) occurrence in keyboard order, from
    /// the one selected on the focused day: day by day as shown, so an
    /// event spanning days is reached on each and the focus stays put.
    pub fn select_relative(&mut self, forward: bool) {
        let order = self.navigation_entries();
        if order.is_empty() {
            self.selected = None;
            return;
        }
        let current = self.selected.as_ref().and_then(|k| {
            order
                .iter()
                .position(|(d, o)| *d == self.date && k.matches(o))
                .or_else(|| order.iter().position(|(_, o)| k.matches(o)))
        });
        let next = match current {
            Some(i) if forward => (i + 1).min(order.len() - 1),
            Some(i) => i.saturating_sub(1),
            // from the focused day: its first, or (going back) the last
            // before it
            None if forward => order
                .iter()
                .position(|(d, _)| *d >= self.date)
                .unwrap_or(order.len() - 1),
            None => order
                .iter()
                .rposition(|(d, _)| *d <= self.date)
                .unwrap_or(0),
        };
        let (day, occ) = &order[next];
        self.date = *day;
        self.selected = Some(OccurrenceKey::of(occ));
    }

    /// Select the first occurrence on the focused day, if any: the top of
    /// its all-day row (something that began days before included), else
    /// its first by time.
    pub fn select_first_on_date(&mut self) {
        let date = self.date;
        self.selected = self
            .navigation_entries()
            .iter()
            .find(|(d, _)| *d == date)
            .map(|(_, o)| OccurrenceKey::of(o));
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

    /// Record changes made by one action (`what` it was doing: "delete
    /// the event") so it can be undone, now or later.
    pub fn record(&mut self, what: &str, changes: Vec<Change>) {
        if let Some(h) = self.history.as_mut() {
            let what = rical_core::history::describe(what, &changes);
            if let Err(e) = h.record(APP, &what, &changes) {
                log::warn!("could not keep “{what}” in the history: {e:#}");
            }
        }
        self.invalidate();
    }

    /// Undo (or redo) history entry `id`, if its files are as it left them.
    pub fn undo_entry(&mut self, id: &str, undo: bool) -> anyhow::Result<()> {
        let Self { history, vdir, .. } = self;
        let history = history
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("no history is kept"))?;
        let result = history.undo_with(id, undo, |c| vdir.apply(c));
        self.invalidate();
        result
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

/// What a contacts folder looked like: how many vCards, newest change.
type ContactsSeen = (usize, Option<std::time::SystemTime>);

type People = Vec<Known>;

/// What `People` were read from: the vdir's generation and the contacts.
type PeopleKey = (u64, ContactsSeen);

/// The contacts read from the folder.
#[derive(Default)]
struct Contacts {
    guests: Vec<rical_core::write::Guest>,
    /// Each address (lower case) with its contact's name.
    names: std::collections::HashMap<String, String>,
    books: Option<rical_cards::Books>,
    /// Each address (lower case) with the card it is on.
    by_email: std::collections::HashMap<String, rical_cards::CardId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(dir: &std::path::Path, uid: &str, lines: &str) {
        std::fs::write(
            dir.join(format!("{uid}.ics")),
            format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//t//EN\r\nBEGIN:VEVENT\r\n\
                 UID:{uid}\r\nDTSTAMP:20250101T000000Z\r\nSUMMARY:{uid}\r\n{lines}END:VEVENT\r\nEND:VCALENDAR\r\n"
            ),
        )
        .unwrap();
    }

    fn names(s: &State) -> Vec<String> {
        s.selected_occurrence()
            .map(|o| o.summary)
            .into_iter()
            .collect()
    }

    #[test]
    fn keys_go_day_by_day_as_shown() {
        let dir = tempfile::tempdir().unwrap();
        let cal = dir.path().join("cal");
        std::fs::create_dir(&cal).unwrap();
        // the week of Mon 26 Oct 2026; "away" began the week before
        event(
            &cal,
            "away",
            "DTSTART;VALUE=DATE:20261021\r\nDTEND;VALUE=DATE:20261030\r\n",
        );
        event(
            &cal,
            "bins",
            "DTSTART;VALUE=DATE:20261028\r\nDTEND;VALUE=DATE:20261029\r\n",
        );
        event(
            &cal,
            "standup",
            "DTSTART:20261028T090000Z\r\nDTEND:20261028T091500Z\r\n",
        );
        event(
            &cal,
            "lunch",
            "DTSTART:20261027T120000Z\r\nDTEND:20261027T130000Z\r\n",
        );
        let vdir = Vdir::load(dir.path()).unwrap();
        let wed = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
        let mut s = State::new(vdir, chrono_tz::UTC, wed);
        s.view = ViewKind::Week;

        // a day's first is the top of its all-day row, even begun before
        s.select_first_on_date();
        assert_eq!(names(&s), ["away"]);
        assert_eq!(s.date, wed);
        // on through Wednesday as drawn, not back to where "away" began
        s.select_relative(true);
        assert_eq!((names(&s), s.date), (vec!["bins".to_owned()], wed));
        s.select_relative(true);
        assert_eq!((names(&s), s.date), (vec!["standup".to_owned()], wed));
        // then Thursday, where "away" is reached again
        s.select_relative(true);
        assert_eq!(names(&s), ["away"]);
        assert_eq!(s.date, wed.succ_opt().unwrap());
        // and back: Wednesday's, then Tuesday's
        s.select_relative(false);
        s.select_relative(false);
        s.select_relative(false);
        assert_eq!((names(&s), s.date), (vec!["away".to_owned()], wed));
        s.select_relative(false);
        assert_eq!(names(&s), ["lunch"]);
        assert_eq!(s.date, wed.pred_opt().unwrap());
    }
}

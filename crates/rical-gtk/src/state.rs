//! Application state shared by the views.

use std::{cell::RefCell, collections::BTreeSet, ops::Range, rc::Rc};

use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use rical_core::{Calendar, Kind, Occurrence, Rgb, Vdir, When};

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
    /// Bumped whenever the data or the hidden set changes.
    generation: u64,
    cache: RefCell<Option<Cache>>,
}

struct Cache {
    generation: u64,
    range: Range<DateTime<Utc>>,
    occurrences: Rc<Vec<Occurrence>>,
}

pub type SharedState = Rc<RefCell<State>>;

/// Fallback colours for calendars without one.
const PALETTE: [Rgb; 6] = [
    Rgb {
        r: 0x35,
        g: 0x84,
        b: 0xe4,
    },
    Rgb {
        r: 0x2e,
        g: 0xc2,
        b: 0x7e,
    },
    Rgb {
        r: 0xe6,
        g: 0x61,
        b: 0x00,
    },
    Rgb {
        r: 0x91,
        g: 0x41,
        b: 0xac,
    },
    Rgb {
        r: 0xe0,
        g: 0x1b,
        b: 0x24,
    },
    Rgb {
        r: 0x98,
        g: 0x6a,
        b: 0x44,
    },
];

impl State {
    pub fn new(vdir: Vdir, local: Tz, date: NaiveDate) -> Self {
        Self {
            vdir,
            local,
            hidden: BTreeSet::new(),
            view: ViewKind::Week,
            date,
            selected: None,
            generation: 0,
            cache: RefCell::new(None),
        }
    }

    /// Call after changing `vdir` or `hidden`.
    pub fn invalidate(&mut self) {
        self.generation += 1;
    }

    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.vdir.calendar(id)
    }

    pub fn color(&self, calendar: &str) -> Rgb {
        let index = self
            .vdir
            .calendars
            .iter()
            .position(|c| c.id == calendar)
            .unwrap_or(0);
        self.vdir
            .calendars
            .get(index)
            .and_then(|c| c.color)
            .unwrap_or(PALETTE[index % PALETTE.len()])
    }

    pub fn visible_days(&self) -> Vec<NaiveDate> {
        layout::visible_days(self.view, self.date)
    }

    /// Visible occurrences (events and todos of shown calendars) in `days`.
    pub fn occurrences(&self, days: &[NaiveDate]) -> Rc<Vec<Occurrence>> {
        let range = layout::days_range(days, self.local);
        if let Some(cache) = &*self.cache.borrow()
            && cache.generation == self.generation
            && cache.range == range
        {
            return cache.occurrences.clone();
        }
        let occurrences = Rc::new(
            self.vdir
                .query(&range, &[Kind::Event, Kind::Todo], self.local)
                .into_iter()
                .filter(|o| !self.hidden.contains(&o.calendar))
                .collect::<Vec<_>>(),
        );
        *self.cache.borrow_mut() = Some(Cache {
            generation: self.generation,
            range,
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

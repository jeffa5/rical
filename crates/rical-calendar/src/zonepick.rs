//! The editor's time zone rows: which zone the start and end are typed in.
//! An event keeps its own zone (09:00 London stays 09:00 London wherever
//! you are when you look), new ones default to yours, and the end can be
//! somewhere else (a flight lands in another zone).

use std::rc::Rc;

use adw::prelude::*;
use chrono_tz::Tz;
use rical_core::{When, write::Draft};

use crate::format;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// No zone: the same clock time wherever you are.
    Floating,
    Tz(Tz),
}

pub struct ZonePicker {
    pub start: adw::ComboRow,
    pub end: adw::ComboRow,
    choices: Vec<Zone>,
    local: Tz,
}

fn label(z: Zone, local: Tz) -> String {
    match z {
        Zone::Floating => "No zone (the same clock time anywhere)".into(),
        Zone::Tz(t) if t == local => format!("{} (yours)", t.name().replace('_', " ")),
        Zone::Tz(t) => t.name().replace('_', " "),
    }
}

fn zone_of(w: &When) -> Option<Zone> {
    match w {
        When::Zoned(dt) => Some(Zone::Tz(dt.timezone())),
        When::Floating(_) => Some(Zone::Floating),
        When::Date(_) => None,
    }
}

/// A searchable list: type part of a name ("london", "los ang").
fn combo(title: &str, labels: &[String]) -> adw::ComboRow {
    let labels = labels.iter().map(String::as_str).collect::<Vec<_>>();
    adw::ComboRow::builder()
        .title(title)
        .model(&gtk::StringList::new(&labels))
        .enable_search(true)
        .search_match_mode(gtk::StringFilterMatchMode::Substring)
        .expression(gtk::PropertyExpression::new(
            gtk::StringObject::static_type(),
            None::<gtk::Expression>,
            "string",
        ))
        .build()
}

impl ZonePicker {
    pub fn new(local: Tz, draft: &Draft) -> Self {
        // mine first, then UTC, every named zone, and no zone at all
        let mut choices = vec![Zone::Tz(local)];
        if local != Tz::UTC {
            choices.push(Zone::Tz(Tz::UTC));
        }
        let start_zone = zone_of(&draft.start).unwrap_or(Zone::Tz(local));
        let end_zone = zone_of(&draft.end).filter(|z| *z != start_zone);
        choices.extend(
            crate::zones::names()
                .iter()
                .filter_map(|n| n.parse::<Tz>().ok())
                .filter(|t| *t != local && *t != Tz::UTC)
                .map(Zone::Tz),
        );
        choices.push(Zone::Floating);
        // an unusual zone of the event's own, so it can be kept
        for z in [Some(start_zone), end_zone].into_iter().flatten() {
            if !choices.contains(&z) {
                choices.insert(1, z);
            }
        }
        let labels = choices.iter().map(|z| label(*z, local)).collect::<Vec<_>>();
        let start = combo("Time zone", &labels);
        let at = |z: Zone| choices.iter().position(|c| *c == z).unwrap_or(0) as u32;
        start.set_selected(at(start_zone));
        let end_labels = std::iter::once("Same as the start".to_owned())
            .chain(labels.iter().cloned())
            .collect::<Vec<_>>();
        let end = combo("Ends in", &end_labels);
        end.set_selected(end_zone.map_or(0, |z| at(z) + 1));
        Self {
            start,
            end,
            choices,
            local,
        }
    }

    pub fn start_zone(&self) -> Zone {
        self.choices
            .get(self.start.selected() as usize)
            .copied()
            .unwrap_or(Zone::Tz(self.local))
    }

    pub fn end_zone(&self) -> Zone {
        match self.end.selected() {
            0 => self.start_zone(),
            i => self
                .choices
                .get(i as usize - 1)
                .copied()
                .unwrap_or(self.start_zone()),
        }
    }

    fn tz(&self, z: Zone) -> Tz {
        match z {
            Zone::Tz(t) => t,
            // floating times are typed and shown as my clock time
            Zone::Floating => self.local,
        }
    }

    pub fn start_tz(&self) -> Tz {
        self.tz(self.start_zone())
    }

    pub fn end_tz(&self) -> Tz {
        self.tz(self.end_zone())
    }

    /// The start as typed, in its zone ("HH:MM" on `base`'s day).
    pub fn parse_start(&self, text: &str, base: &When) -> Option<When> {
        let tz = self.start_tz();
        let w = format::parse_input(text, false, &When::Zoned(base.to_tz(tz)), tz)?;
        Some(self.form(w, self.start_zone()))
    }

    /// The end as typed in its zone, or a duration after `start`.
    pub fn parse_end(&self, text: &str, start: &When) -> Option<When> {
        let tz = self.end_tz();
        let w = format::parse_input(text, false, &When::Zoned(start.to_tz(tz)), tz)?;
        Some(self.form(w, self.end_zone()))
    }

    /// A parsed value in `zone`'s form: there, or as plain clock time.
    fn form(&self, w: When, zone: Zone) -> When {
        match (zone, w) {
            (Zone::Floating, When::Zoned(dt)) => When::Floating(dt.naive_local()),
            (Zone::Tz(t), When::Zoned(dt)) => When::Zoned(dt.with_timezone(&t)),
            (_, w) => w,
        }
    }

    pub fn set_visible(&self, visible: bool) {
        self.start.set_visible(visible);
        self.end.set_visible(visible);
    }

    /// Run `f` when either zone changes.
    pub fn on_change(&self, f: Rc<dyn Fn()>) {
        for row in [&self.start, &self.end] {
            let f = f.clone();
            row.connect_selected_notify(move |_| f());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker(start: When, end: When) -> ZonePicker {
        gtk::init().ok();
        ZonePicker::new(Tz::Asia__Singapore, &Draft::new(start, end))
    }

    #[test]
    fn times_are_read_in_their_zones() {
        if gtk::init().is_err() {
            return; // no display to build widgets on
        }
        let london = When::parse("20261221T090000", Some("Europe/London")).unwrap();
        let p = picker(london, london.add(chrono::TimeDelta::hours(1)));
        assert_eq!(p.start_tz(), Tz::Europe__London);
        // typed times are London time
        let s = p.parse_start("2026-12-21 09:00", &london).unwrap();
        assert_eq!(s, london);
        // a duration ends an hour later, a time on the same day
        let e = p.parse_end("+1h", &s).unwrap();
        assert_eq!(
            e.to_tz(Tz::Europe__London).format("%H:%M").to_string(),
            "10:00"
        );
        let e = p.parse_end("11:30", &s).unwrap();
        assert_eq!(
            e.to_tz(Tz::Europe__London).format("%H:%M").to_string(),
            "11:30"
        );
    }
}

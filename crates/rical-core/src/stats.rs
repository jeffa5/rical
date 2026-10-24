use std::{collections::BTreeMap, path::PathBuf};

use icalendar::Component;
use serde::Serialize;

use crate::{files::paths_to_files, ics::read_ics_to_calendar};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Stats {
    pub total: u64,
    pub by_kind: BTreeMap<String, u64>,
}

pub fn stats(paths: &[PathBuf], recursive: bool) -> Stats {
    let paths = paths_to_files(paths, recursive);

    let calendars = paths
        .into_iter()
        .filter_map(|p| read_ics_to_calendar(&p).ok())
        .collect::<Vec<_>>();

    let mut total_components = 0;

    let mut counts: BTreeMap<_, u64> = BTreeMap::new();

    for calendar in calendars {
        let total = calendar.components.len();
        total_components += total as u64;

        calendar.components.iter().for_each(|component| {
            let kind = component_kind(component);
            *counts.entry(kind).or_default() += 1;
        });
    }

    Stats {
        total: total_components,
        by_kind: counts,
    }
}

fn component_kind(component: &icalendar::CalendarComponent) -> String {
    match component {
        icalendar::CalendarComponent::Todo(todo) => todo.component_kind(),
        icalendar::CalendarComponent::Event(event) => event.component_kind(),
        icalendar::CalendarComponent::Venue(venue) => venue.component_kind(),
        icalendar::CalendarComponent::Other(other) => other.component_kind(),
        _ => "unknown".to_owned(),
    }
}

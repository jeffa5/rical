use std::{collections::BTreeMap, path::Path};

use icalendar::Component;

use crate::{calendar::read_ics_to_calendar, list::paths_to_files};

pub fn stats(path: &Path, recursive: bool) -> anyhow::Result<()> {
    let paths = paths_to_files(&[path.to_owned()], recursive);

    let calendars = paths
        .into_iter()
        .filter_map(|p| read_ics_to_calendar(&p).ok())
        .collect::<Vec<_>>();

    let mut total_components = 0;

    let mut counts: BTreeMap<_, usize> = BTreeMap::new();

    for calendar in calendars {
        let total = calendar.components.len();
        total_components += total;

        calendar.components.iter().for_each(|component| {
            let kind = component_kind(component);
            *counts.entry(kind).or_default() += 1;
        });
    }

    println!("total: {total_components}");
    for (kind, count) in counts {
        println!("{kind}: {count}");
    }

    Ok(())
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

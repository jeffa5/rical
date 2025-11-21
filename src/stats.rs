use std::{collections::BTreeMap, fs::read_dir, path::Path};

use icalendar::Component;

use crate::calendar::read_ics_to_calendar;

pub fn stats(path: &Path) -> anyhow::Result<()> {
    if !path.is_dir() {
        anyhow::bail!("only supports directories for now")
    }

    let calendars = read_dir(path)?
        .filter_map(|e| e.ok())
        .filter_map(|e| read_ics_to_calendar(&e.path()).ok())
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

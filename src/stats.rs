use std::{collections::BTreeMap, path::PathBuf};

use icalendar::Component;
use serde::Serialize;

use crate::{
    calendar::read_ics_to_calendar,
    format::{Formatable, OutputFormat},
    list::paths_to_files,
};

pub struct StatsArgs {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    pub format: OutputFormat,
}

#[derive(Debug, Serialize)]
struct Stats {
    total: u64,
    #[serde(flatten)]
    by_kind: BTreeMap<String, u64>,
}

impl Formatable for Stats {
    fn keys(&self) -> Vec<String> {
        let mut keys = vec!["total".to_owned()];
        keys.extend(self.by_kind.keys().cloned());
        keys
    }

    fn verbose_keys(&self) -> Vec<String> {
        self.keys()
    }
}

pub fn stats(args: &StatsArgs) -> anyhow::Result<()> {
    let paths = paths_to_files(&args.paths, args.recursive);

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

    let stats = Stats {
        total: total_components,
        by_kind: counts,
    };

    let s = args.format.render(&stats)?;
    println!("{}", s);

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

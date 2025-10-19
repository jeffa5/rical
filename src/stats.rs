use std::{fs::read_dir, path::Path};

use crate::calendar::read_ics_to_calendar;

pub fn stats(path: &Path) -> anyhow::Result<()> {
    if !path.is_dir() {
        anyhow::bail!("only supports directories for now")
    }

    let calendars = read_dir(path)?
        .filter_map(|e| e.ok())
        .filter_map(|e| read_ics_to_calendar(&e.path()).ok())
        .collect::<Vec<_>>();

    let mut num_events = 0;
    let mut num_todos = 0;

    for calendar in calendars {
        let events = calendar.events().count();
        let todos = calendar.todos().count();

        num_events += events;
        num_todos += todos;
    }

    println!("events: {num_events}");
    println!("todos: {num_todos}");

    Ok(())
}

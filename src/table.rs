use std::io::Write;

use comfy_table::{Table, presets::NOTHING};

use crate::Event;

pub struct TableOptions {
    pub verbose: bool,
    pub symbols: bool,
}

pub struct TableWriter {
    writer: Box<dyn Write>,
    options: TableOptions,
}

impl TableWriter {
    pub fn new(writer: Box<dyn Write>, options: TableOptions) -> Self {
        Self { writer, options }
    }

    pub fn write_events(&mut self, events: &[Event]) -> anyhow::Result<()> {
        let mut table = Table::new();
        let header = if self.options.verbose {
            vec![
                "uid".to_owned(),
                "created".to_owned(),
                "start".to_owned(),
                "status".to_owned(),
                "summary".to_owned(),
                "organizer".to_owned(),
                "attendees".to_owned(),
            ]
        } else {
            vec![
                "start".to_owned(),
                "status".to_owned(),
                "summary".to_owned(),
                "organizer".to_owned(),
                "attendees".to_owned(),
            ]
        };

        table.load_preset(NOTHING).set_header(header);

        for event in events {
            let status = if self.options.symbols {
                match event.get_status() {
                    Some(icalendar::EventStatus::Tentative) => "t".to_owned(),
                    Some(icalendar::EventStatus::Confirmed) => "y".to_owned(),
                    Some(icalendar::EventStatus::Cancelled) => "n".to_owned(),
                    None => "".to_owned(),
                }
            } else {
                event.status_str().unwrap_or_default().to_owned()
            };
            let row = if self.options.verbose {
                vec![
                    event.uid().unwrap().to_owned(),
                    event
                        .created()
                        .unwrap_or_else(|| event.dtstamp().expect("no created or dtstamp in event"))
                        .to_string(),
                    event.start().map_or(String::new(), |d| d.to_string()),
                    status,
                    event.summary().unwrap_or_default().to_owned(),
                    event.organizer().unwrap_or_default().to_owned(),
                    event.attendees().len().to_string(),
                ]
            } else {
                vec![
                    event.start().map_or(String::new(), |d| d.to_string()),
                    status,
                    event.summary().unwrap_or_default().to_owned(),
                    event.organizer().unwrap_or_default().to_owned(),
                    event.attendees().len().to_string(),
                ]
            };
            table.add_row(row);
        }

        writeln!(self.writer, "{table}")?;

        Ok(())
    }
}

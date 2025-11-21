use std::io::Write;

use comfy_table::{Table, presets::NOTHING};
use icalendar::Component;

use crate::{component::AnyComponent, list::date_perhaps_time_to_datetime_utc};

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

    pub fn write_any_components(&mut self, components: &[AnyComponent]) -> anyhow::Result<()> {
        let mut table = Table::new();
        let header = if self.options.verbose {
            vec![
                "kind".to_owned(),
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
                "kind".to_owned(),
                "start".to_owned(),
                "status".to_owned(),
                "summary".to_owned(),
                "organizer".to_owned(),
                "attendees".to_owned(),
            ]
        };

        table.load_preset(NOTHING).set_header(header);

        for component in components {
            let status = if self.options.symbols {
                match component.property_value("STATUS") {
                    Some("TENTATIVE") => "t".to_owned(),
                    Some("CONFIRMED") => "y".to_owned(),
                    Some("CANCELLED") => "n".to_owned(),
                    Some(_) => "?".to_owned(),
                    None => "".to_owned(),
                }
            } else {
                component
                    .property_value("STATUS")
                    .unwrap_or_default()
                    .to_owned()
            };
            let row = if self.options.verbose {
                vec![
                    component.component_kind(),
                    component.get_uid().unwrap().to_owned(),
                    component
                        .get_created()
                        .unwrap_or_else(|| {
                            component
                                .get_timestamp()
                                .expect("no created or dtstamp in component")
                        })
                        .to_string(),
                    component.get_start().map_or(String::new(), |d| {
                        date_perhaps_time_to_datetime_utc(d).to_string()
                    }),
                    status,
                    component.get_summary().unwrap_or_default().to_owned(),
                    component
                        .property_value("ORGANIZER")
                        .unwrap_or_default()
                        .to_owned(),
                    component
                        .property_value("ATTENDEES")
                        .map(|a| a.split(',').count().to_string())
                        .unwrap_or_default(),
                ]
            } else {
                vec![
                    component.component_kind(),
                    component.get_start().map_or(String::new(), |d| {
                        date_perhaps_time_to_datetime_utc(d).to_string()
                    }),
                    status,
                    component.get_summary().unwrap_or_default().to_owned(),
                    component
                        .property_value("ORGANIZER")
                        .unwrap_or_default()
                        .to_owned(),
                    component
                        .property_value("ATTENDEES")
                        .map(|a| a.split(',').count().to_string())
                        .unwrap_or_default(),
                ]
            };
            table.add_row(row);
        }

        writeln!(self.writer, "{table}")?;

        Ok(())
    }
}

use std::{fs::read_dir, path::PathBuf};

use chrono::NaiveDateTime;
use log::warn;

use crate::{calendar::read_ics_to_calendar, table::TableWriter};

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    VerboseTable,
    Table,
    Json,
}

pub struct ListArgs {
    pub paths: Vec<PathBuf>,
    pub format: OutputFormat,
    pub limit: usize,
    pub from: NaiveDateTime,
}

pub fn list(args: &ListArgs) -> anyhow::Result<()> {
    let paths = args.paths.iter().flat_map(|path| {
        if path.is_file() {
            vec![path.clone()]
        } else if path.is_dir() {
            read_dir(path)
                .map(|rd| {
                    rd.into_iter()
                        .filter_map(|e| e.ok())
                        .filter(|e| e.metadata().is_ok_and(|m| m.is_file()))
                        .map(|e| e.path())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        } else {
            warn!("Path was neither file or directory: {:?}", path.metadata());
            Vec::new()
        }
    });

    let mut events = paths
        .into_iter()
        .filter_map(|p| read_ics_to_calendar(&p).ok())
        .flat_map(|cal| {
            cal.events()
                .map(|e| crate::Event::from(e.clone()))
                .filter(|e| e.start().is_some_and(|ndt| ndt > args.from))
                .collect::<Vec<_>>()
        })
        .take(args.limit)
        .collect::<Vec<_>>();

    events.sort_by_key(|e| {
        e.start().unwrap_or_else(|| {
            e.created().unwrap_or_else(|| {
                e.dtstamp()
                    .expect("start, created, or dtstamp should be there")
            })
        })
    });

    match args.format {
        OutputFormat::VerboseTable => {
            let mut table_writer = TableWriter::new(
                Box::new(std::io::stdout()),
                crate::table::TableOptions {
                    verbose: true,
                    symbols: false,
                },
            );
            table_writer.write_events(&events)?;
        }
        OutputFormat::Table => {
            let mut table_writer = TableWriter::new(
                Box::new(std::io::stdout()),
                crate::table::TableOptions {
                    verbose: false,
                    symbols: true,
                },
            );
            table_writer.write_events(&events)?;
        }
        OutputFormat::Json => {
            todo!()
        }
    }

    Ok(())
}

use std::{
    fs::{read_dir, read_to_string},
    path::{Path, PathBuf},
};

use chrono::NaiveDateTime;
use icalendar::{
    Calendar,
    parser::{read_calendar, unfold},
};
use log::{debug, warn};

use crate::table::TableWriter;

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    VerboseTable,
    Table,
    Json,
}

pub struct ListArgs {
    pub path: PathBuf,
    pub format: OutputFormat,
    pub limit: usize,
    pub from: NaiveDateTime,
}

pub fn list(args: &ListArgs) -> anyhow::Result<()> {
    let paths = if args.path.is_file() {
        vec![args.path.clone()]
    } else if args.path.is_dir() {
        read_dir(&args.path)?
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.metadata().map_or(false, |m| m.is_file()))
            .map(|e| e.path())
            .collect()
    } else {
        anyhow::bail!(
            "Path was neither file or directory: {:?}",
            args.path.metadata()
        );
    };

    let mut events = paths
        .into_iter()
        .filter_map(|p| read_ics_to_calendar(&p).ok())
        .flat_map(|cal| {
            cal.events()
                .map(|e| crate::Event::from(e.clone()))
                .filter(|e| e.start().map_or(false, |ndt| ndt > args.from))
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

fn read_ics_to_calendar(path: &Path) -> anyhow::Result<Calendar> {
    debug!("Loading file {:?}", path);

    let s = read_to_string(path)?;

    let unfolded = unfold(&s);

    let cal_ref = match read_calendar(&unfolded) {
        Ok(cal) => cal,
        Err(e) => {
            warn!("{e}");
            anyhow::bail!(e)
        }
    };

    // make a nicer to work with version
    let cal = Calendar::from(cal_ref);
    Ok(cal)
}

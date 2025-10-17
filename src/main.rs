use std::{
    fs::{read_dir, read_to_string},
    path::PathBuf,
};

use clap::Parser as _;
use icalendar::{Calendar, Component as _, parser::read_calendar};

#[derive(Debug, Clone, clap::ValueEnum)]
enum OutputFormat {
    Table,
    Json,
}

#[derive(Debug, Clone, clap::Parser)]
struct Args {
    #[clap(short, long, default_value = "table")]
    format: OutputFormat,

    #[clap()]
    path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    eprintln!("{:?}", args);

    if !args.path.is_dir() {
        anyhow::bail!("currently only directories are supported")
    }

    let mut events = vec![];

    for entry in read_dir(args.path)? {
        let entry = entry?;
        if !entry.metadata()?.is_file() {
            continue;
        }

        println!("Loading file {:?}", entry.path());

        let s = read_to_string(entry.path())?;

        let cal_ref = match read_calendar(&s) {
            Ok(cal) => cal,
            Err(e) => {
                eprintln!("{e}");
                continue;
            }
        };

        // make a nicer to work with version
        let cal = Calendar::from(cal_ref);

        events.extend(cal.events().cloned());
    }

    println!("found {} events", events.len());

    events.sort_by_key(|e| e.property_value("DTSTAMP").map(|s| s.to_owned()));

    for event in events {
        println!(
            "{} {}",
            event.property_value("DTSTAMP").unwrap_or_default(),
            event.property_value("SUMMARY").unwrap_or_default(),
        )
    }

    Ok(())
}

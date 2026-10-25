use std::{path::PathBuf, str::FromStr};

use chrono::{NaiveDate, NaiveDateTime, Utc};
use clap::Parser as _;
use log::debug;
use rical_core::{
    Kind,
    query::{ListKind, ListQuery},
};

use crate::{agenda::AgendaArgs, format::OutputFormat, show::ShowArgs, stats::StatsArgs};

mod agenda;
mod edit;
mod format;
mod list;
mod pretty;
mod show;
mod stats;
#[allow(dead_code)]
mod table;
mod unfold;

fn level_filter_from_verbosity(verbosity: u8) -> log::LevelFilter {
    match verbosity {
        0 => log::LevelFilter::Off,
        1 => log::LevelFilter::Error,
        2 => log::LevelFilter::Warn,
        3 => log::LevelFilter::Info,
        4 => log::LevelFilter::Debug,
        _ => log::LevelFilter::Trace,
    }
}

#[derive(Debug, Clone, clap::Parser)]
struct Args {
    #[clap(subcommand)]
    cmd: Subcommand,

    #[clap(short, action=clap::ArgAction::Count, global = true)]
    verbosity: u8,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Subcommand {
    Stats {
        #[clap()]
        path: PathBuf,

        #[clap(long, default_value = "false")]
        recursive: bool,

        #[clap(short, long, default_value = "table")]
        output: OutputFormat,
    },
    Unfold {
        #[clap()]
        path: PathBuf,
    },
    List {
        #[clap(short, long, default_value = "table")]
        output: OutputFormat,

        #[clap(long)]
        raw: bool,

        #[clap(long, default_value = "10")]
        limit: usize,

        #[clap(long, value_parser = parse_datetime)]
        from: Option<NaiveDateTime>,

        #[clap(long, value_parser = parse_datetime)]
        to: Option<NaiveDateTime>,

        #[clap(long, default_value = "false")]
        recursive: bool,

        #[clap(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,

        #[clap(long, value_delimiter = ',', default_value = "todo,event,journal")]
        kinds: Vec<ListKind>,
    },
    Show {
        #[clap(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,

        #[clap(long, default_value = "false")]
        recursive: bool,

        #[clap(long)]
        id: String,
    },
    /// Occurrences in a date range, with recurrences expanded.
    Agenda {
        /// Root of the vdir.
        #[clap()]
        vdir: PathBuf,

        /// First day to show (YYYY-MM-DD), default today.
        #[clap(long)]
        from: Option<NaiveDate>,

        #[clap(long, default_value = "7")]
        days: u64,

        #[clap(long, value_delimiter = ',', default_value = "event,todo")]
        kinds: Vec<Kind>,

        #[clap(short, long, default_value = "table")]
        output: OutputFormat,
    },
    /// Add an event (or with --todo, a task) and print its file.
    New {
        /// Root of the vdir.
        vdir: PathBuf,

        /// Calendar: folder name, display name or number. Default: the one
        /// with the most items.
        #[clap(long)]
        calendar: Option<String>,

        /// Start: "[date] [HH:MM]", e.g. "fri 14:00", "2025-03-04" (all
        /// day). Default: the next full hour.
        #[clap(long)]
        at: Option<String>,

        /// Length, e.g. 30m, 1h30m, 2d. Default: 1h, or 1d for all day.
        #[clap(long = "for")]
        length: Option<String>,

        /// Where it is: a place, or a link to join online.
        #[clap(long)]
        location: Option<String>,

        /// Add a task instead of an event.
        #[clap(long)]
        todo: bool,

        /// Due date for a task, e.g. fri, +1w, 2025-03-04.
        #[clap(long, requires = "todo")]
        due: Option<String>,

        #[clap(required = true, num_args = 1..)]
        title: Vec<String>,
    },
    /// Mark a task done (or with --undo, not done).
    Done {
        vdir: PathBuf,
        /// The start of the item's UID (see `list`).
        #[clap(long)]
        id: String,
        #[clap(long)]
        undo: bool,
    },
    /// Delete an item (every occurrence of a repeating one).
    Delete {
        vdir: PathBuf,
        /// The start of the item's UID (see `list`).
        #[clap(long)]
        id: String,
    },
    /// Edit an item's file in $VISUAL or $EDITOR.
    Edit {
        vdir: PathBuf,
        /// The start of the item's UID (see `list`).
        #[clap(long)]
        id: String,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_default_env()
        .filter_level(level_filter_from_verbosity(args.verbosity))
        .init();

    debug!("{:?}", args);

    match args.cmd {
        Subcommand::Stats {
            path,
            recursive,
            output,
        } => {
            let stats_args = StatsArgs {
                paths: vec![path],
                recursive,
                format: output,
            };
            stats::stats(&stats_args)?;
        }
        Subcommand::Unfold { path } => {
            unfold::unfold(&path)?;
        }
        Subcommand::List {
            kinds,
            output,
            raw,
            limit,
            from,
            to,
            recursive,
            paths,
        } => {
            list::list(
                &ListQuery {
                    paths,
                    limit,
                    from,
                    to,
                    recursive,
                    kinds,
                    short_id: None,
                },
                &output,
                raw,
            )?;
        }
        Subcommand::Show {
            paths,
            recursive,
            id,
        } => {
            show::show(&ShowArgs {
                paths,
                recursive,
                id,
            })?;
        }
        Subcommand::Agenda {
            vdir,
            from,
            days,
            kinds,
            output,
        } => {
            agenda::agenda(&AgendaArgs {
                vdir,
                from,
                days,
                kinds,
                format: output,
            })?;
        }
        Subcommand::New {
            vdir,
            calendar,
            at,
            length,
            location,
            todo,
            due,
            title,
        } => {
            edit::new(&edit::NewArgs {
                vdir,
                calendar,
                at,
                length,
                location,
                todo,
                due,
                title: title.join(" "),
            })?;
        }
        Subcommand::Done { vdir, id, undo } => edit::done(&vdir, &id, undo)?,
        Subcommand::Delete { vdir, id } => edit::delete(&vdir, &id)?,
        Subcommand::Edit { vdir, id } => edit::edit(&vdir, &id)?,
    }

    Ok(())
}

fn parse_datetime(
    s: &str,
) -> Result<NaiveDateTime, Box<dyn std::error::Error + Send + Sync + 'static>> {
    match s {
        "now" => Ok(Utc::now().naive_utc()),
        s => {
            let ndt = NaiveDateTime::from_str(s)?;
            Ok(ndt)
        }
    }
}

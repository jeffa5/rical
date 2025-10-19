use std::path::PathBuf;

use chrono::{NaiveDateTime, Utc};
use clap::Parser as _;
use log::debug;
use rical::{
    list::{self, ListArgs, OutputFormat},
    unfold,
};

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
    cmd: Option<SubCommand>,

    #[clap(short, action=clap::ArgAction::Count, global = true)]
    verbosity: u8,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum SubCommand {
    List {
        #[clap(short, long, default_value = "table")]
        output: OutputFormat,

        #[clap(long, default_value = "10")]
        limit: usize,

        #[clap(long)]
        from: Option<NaiveDateTime>,

        #[clap(required = true, num_args = 1..)]
        paths: Vec<PathBuf>,
    },
    Unfold {
        #[clap()]
        path: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_default_env()
        .filter_level(level_filter_from_verbosity(args.verbosity))
        .init();

    debug!("{:?}", args);

    match args.cmd {
        Some(SubCommand::List {
            output,
            limit,
            from,
            paths,
        }) => {
            let list_args = ListArgs {
                paths,
                format: output,
                limit: limit,
                from: from.unwrap_or_else(|| Utc::now().naive_utc()),
            };
            list::list(&list_args)?;
        }
        Some(SubCommand::Unfold { path }) => {
            unfold::unfold(&path)?;
        }
        None => {
            list::list(&ListArgs {
                paths: vec![PathBuf::from(".")],
                format: OutputFormat::Table,
                limit: 10,
                from: Utc::now().naive_utc(),
            })?;
        }
    }

    Ok(())
}

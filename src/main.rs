use std::{path::PathBuf, str::FromStr};

use chrono::{NaiveDateTime, Utc};
use clap::Parser as _;
use log::debug;
use rical::{
    format::OutputFormat,
    list::{self, ListArgs, ListKind},
    stats::{self, StatsArgs},
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
    cmd: Subcommand,

    #[clap(short, action=clap::ArgAction::Count, global = true)]
    verbosity: u8,
}

#[derive(Debug, Clone, clap::Args)]
struct ListCliArgs {
    #[clap(short, long, default_value = "table")]
    output: OutputFormat,

    #[clap(long)]
    raw: bool,

    #[clap(long, default_value = "10")]
    limit: usize,

    #[clap(long, value_parser = parse_datetime)]
    from: Option<NaiveDateTime>,

    #[clap(long, default_value = "false")]
    recursive: bool,

    #[clap(required = true, num_args = 1..)]
    paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, clap::Subcommand)]
enum Subcommand {
    Events {
        #[clap(subcommand)]
        cmd: EventsSubcommand,
    },
    Todos {
        #[clap(subcommand)]
        cmd: TodosSubcommand,
    },
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
        #[clap(flatten)]
        common: ListCliArgs,

        #[clap(long, value_delimiter = ',', default_value = "todo,event,journal")]
        kinds: Vec<ListKind>,
    },
}

#[derive(Debug, Clone, clap::Subcommand)]
enum EventsSubcommand {
    List {
        #[clap(flatten)]
        common: ListCliArgs,
    },
}

#[derive(Debug, Clone, clap::Subcommand)]
enum TodosSubcommand {
    List {
        #[clap(flatten)]
        common: ListCliArgs,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    env_logger::Builder::from_default_env()
        .filter_level(level_filter_from_verbosity(args.verbosity))
        .init();

    debug!("{:?}", args);

    match args.cmd {
        Subcommand::Events { cmd } => match cmd {
            EventsSubcommand::List {
                common:
                    ListCliArgs {
                        output,
                        raw,
                        limit,
                        from,
                        recursive,
                        paths,
                    },
            } => {
                let list_args = ListArgs {
                    paths,
                    format: output,
                    raw,
                    limit,
                    from,
                    recursive,
                    kinds: vec![list::ListKind::Event],
                };
                list::list(&list_args)?;
            }
        },
        Subcommand::Todos { cmd } => match cmd {
            TodosSubcommand::List {
                common:
                    ListCliArgs {
                        output,
                        raw,
                        limit,
                        from,
                        recursive,
                        paths,
                    },
            } => {
                let list_args = ListArgs {
                    paths,
                    format: output,
                    raw,
                    limit,
                    from,
                    recursive,
                    kinds: vec![list::ListKind::Todo],
                };
                list::list(&list_args)?;
            }
        },
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
            common:
                ListCliArgs {
                    output,
                    raw,
                    limit,
                    from,
                    recursive,
                    paths,
                },
        } => {
            list::list(&ListArgs {
                paths,
                format: output,
                raw,
                limit,
                from,
                recursive,
                kinds,
            })?;
        }
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

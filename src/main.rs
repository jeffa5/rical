use std::path::PathBuf;

use chrono::{NaiveDateTime, Utc};
use clap::Parser as _;
use rical::list::{self, ListArgs, OutputFormat};

#[derive(Debug, Clone, clap::Parser)]
struct Args {
    #[clap(subcommand)]
    cmd: Option<SubCommand>,
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

        #[clap()]
        path: PathBuf,
    },
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    eprintln!("{:?}", args);

    match args.cmd {
        Some(SubCommand::List {
            output,
            limit,
            from,
            path,
        }) => {
            if !path.is_dir() {
                anyhow::bail!("currently only directories are supported")
            }

            let list_args = ListArgs {
                path: path,
                format: output,
                limit: limit,
                from: from.unwrap_or_else(|| Utc::now().naive_utc()),
            };
            list::list(&list_args)?;
        }
        None => {
            list::list(&ListArgs {
                path: PathBuf::from("."),
                format: OutputFormat::Table,
                limit: 10,
                from: Utc::now().naive_utc(),
            })?;
        }
    }

    Ok(())
}

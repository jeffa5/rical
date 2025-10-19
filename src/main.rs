use std::path::PathBuf;

use chrono::{NaiveDateTime, Utc};
use clap::Parser as _;
use rical::list::{self, ListArgs, OutputFormat};

#[derive(Debug, Clone, clap::Parser)]
struct Args {
    #[clap(short, long, default_value = "table")]
    output: OutputFormat,

    #[clap(long, default_value = "10")]
    limit: usize,

    #[clap(long)]
    from: Option<NaiveDateTime>,

    #[clap()]
    path: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    eprintln!("{:?}", args);

    if !args.path.is_dir() {
        anyhow::bail!("currently only directories are supported")
    }

    let list_args = ListArgs {
        path: args.path,
        format: args.output,
        limit: args.limit,
        from: args.from.unwrap_or_else(|| Utc::now().naive_utc()),
    };
    list::list(&list_args)?;

    Ok(())
}

use std::path::PathBuf;

use clap::Parser as _;
use rical::list::{self, ListArgs, OutputFormat};

#[derive(Debug, Clone, clap::Parser)]
struct Args {
    #[clap(short, long, default_value = "table")]
    format: OutputFormat,

    #[clap(long, default_value = "10")]
    limit: usize,

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
        format: args.format,
        limit: args.limit,
    };
    list::list(&list_args)?;

    Ok(())
}

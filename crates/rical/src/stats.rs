use std::path::PathBuf;

use rical_core::stats::Stats;

use crate::format::{Formatable, OutputFormat};

pub struct StatsArgs {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    pub format: OutputFormat,
}

impl Formatable for Stats {
    fn keys(&self) -> Vec<String> {
        let mut keys = vec!["total".to_owned()];
        keys.extend(self.by_kind.keys().cloned());
        keys
    }
}

pub fn stats(args: &StatsArgs) -> anyhow::Result<()> {
    let stats = rical_core::stats::stats(&args.paths, args.recursive);

    let s = args.format.render(&stats)?;
    println!("{}", s);

    Ok(())
}

use std::path::PathBuf;

use crate::list::{self, ListArgs, ListKind};

#[derive(Debug)]
pub struct ShowArgs {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    pub id: String,
}

pub fn show(args: &ShowArgs) -> anyhow::Result<()> {
    list::list(&ListArgs {
        paths: args.paths.clone(),
        format: crate::format::OutputFormat::Table,
        raw: true,
        limit: 1,
        from: None,
        to: None,
        kinds: vec![ListKind::All],
        recursive: args.recursive,
        short_id: Some(args.id.clone()),
    })
}

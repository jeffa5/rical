use std::path::PathBuf;

use rical_core::query::{ListKind, ListQuery};

use crate::{format::OutputFormat, list};

#[derive(Debug)]
pub struct ShowArgs {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    pub id: String,
}

pub fn show(args: &ShowArgs) -> anyhow::Result<()> {
    list::list(
        &ListQuery {
            paths: args.paths.clone(),
            limit: 1,
            from: None,
            to: None,
            kinds: vec![ListKind::All],
            recursive: args.recursive,
            short_id: Some(args.id.clone()),
        },
        &OutputFormat::Table,
        true,
    )
}

use std::{fs::read_to_string, path::Path};

use log::debug;

pub fn unfold(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        anyhow::bail!("Path must be a file")
    }

    debug!("Loading file {:?}", path);

    let s = read_to_string(path)?;

    let unfolded = icalendar::parser::unfold(&s);

    println!("{unfolded}");

    Ok(())
}

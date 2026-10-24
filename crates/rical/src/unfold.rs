use std::path::Path;

pub fn unfold(path: &Path) -> anyhow::Result<()> {
    let unfolded = rical_core::ics::unfold_file(path)?;

    println!("{unfolded}");

    Ok(())
}

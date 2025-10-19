use std::{fs::read_to_string, path::Path};

use icalendar::{
    Calendar,
    parser::{read_calendar, unfold},
};
use log::{debug, warn};

pub fn read_ics_to_calendar(path: &Path) -> anyhow::Result<Calendar> {
    debug!("Loading file {:?}", path);

    let s = read_to_string(path)?;

    let unfolded = unfold(&s);

    let cal_ref = match read_calendar(&unfolded) {
        Ok(cal) => cal,
        Err(e) => {
            warn!("{e}");
            anyhow::bail!(e)
        }
    };

    // make a nicer to work with version
    let cal = Calendar::from(cal_ref);
    Ok(cal)
}

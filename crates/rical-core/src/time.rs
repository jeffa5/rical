use chrono::{DateTime, Utc};

use crate::When;

/// An instant to list and sort by: zoned values as they are, floating
/// times and dates as UTC.
pub fn when_to_utc(w: When) -> DateTime<Utc> {
    w.to_utc(chrono_tz::Tz::UTC)
}

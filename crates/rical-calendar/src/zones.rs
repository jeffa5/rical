//! Extra time zones, shown as columns of hour labels beside the grid's
//! own: `zones = America/New_York, Taipei=Asia/Taipei` in the config.

use chrono::{DateTime, NaiveDate, Offset as _, TimeZone as _};
use chrono_tz::Tz;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Zone {
    /// What its column is headed: given, or the place in its name.
    pub label: String,
    pub tz: Tz,
}

impl Zone {
    /// `America/New_York`, or `NYC=America/New_York` with a label.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (label, name) = match text.split_once('=') {
            Some((l, n)) => (Some(l.trim().to_owned()), n.trim()),
            None => (None, text),
        };
        let tz = name
            .parse::<Tz>()
            .map_err(|_| format!("“{name}” is not a time zone name, e.g. America/New_York"))?;
        let label =
            label.unwrap_or_else(|| name.rsplit('/').next().unwrap_or(name).replace('_', " "));
        Ok(Self { label, tz })
    }

    /// A comma-separated list.
    pub fn parse_list(text: &str) -> Result<Vec<Self>, String> {
        text.split(',')
            .filter(|s| !s.trim().is_empty())
            .map(Self::parse)
            .collect()
    }

    /// The time here at `t`, and how many days that is from `day` (the
    /// day of the grid row), e.g. +1 when it is already tomorrow there.
    pub fn at<T: chrono::TimeZone>(&self, t: DateTime<T>, day: NaiveDate) -> (String, i64) {
        let here = t.with_timezone(&self.tz);
        (
            here.format("%H:%M").to_string(),
            (here.date_naive() - day).num_days(),
        )
    }

    /// The zone's abbreviation now, e.g. EDT, or its offset when it has
    /// no letters (+08).
    pub fn abbreviation(&self, t: DateTime<chrono::Utc>) -> String {
        let here = t.with_timezone(&self.tz);
        let abbr = here.format("%Z").to_string();
        if abbr.starts_with(['+', '-']) {
            let secs = self
                .tz
                .offset_from_utc_datetime(&t.naive_utc())
                .fix()
                .local_minus_utc();
            let (h, m) = (secs / 3600, (secs.abs() % 3600) / 60);
            if m == 0 {
                format!("UTC{h:+}")
            } else {
                format!("UTC{h:+}:{m:02}")
            }
        } else {
            abbr
        }
    }
}

/// Every time zone name, for completion.
pub fn names() -> Vec<String> {
    chrono_tz::TZ_VARIANTS
        .iter()
        .map(|t| t.name().to_owned())
        .filter(|n| n.contains('/') && !n.starts_with("Etc/"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_names_and_labels() {
        let z = Zone::parse_list("America/New_York, Taipei=Asia/Taipei").unwrap();
        assert_eq!(z[0].label, "New York");
        assert_eq!(z[1].label, "Taipei");
        assert_eq!(z[1].tz, Tz::Asia__Taipei);
        assert!(Zone::parse("Mars/Olympus").is_err());
    }

    #[test]
    fn times_and_days() {
        let ny = Zone::parse("America/New_York").unwrap();
        let tp = Zone::parse("Asia/Taipei").unwrap();
        let india = Zone::parse("Asia/Kolkata").unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let t = chrono::Utc.with_ymd_and_hms(2026, 10, 26, 2, 0, 0).unwrap();
        // the evening before in New York, morning in Taipei, half past in India
        assert_eq!(ny.at(t, day), ("22:00".to_owned(), -1));
        assert_eq!(tp.at(t, day), ("10:00".to_owned(), 0));
        assert_eq!(india.at(t, day), ("07:30".to_owned(), 0));
        assert_eq!(ny.abbreviation(t), "EDT");
        assert_eq!(tp.abbreviation(t), "CST");
    }
}

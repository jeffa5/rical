use std::path::PathBuf;

use rical_core::{AnyComponent, time::date_perhaps_time_to_datetime_utc};
use serde::Serialize;

use crate::format::Formatable;

#[derive(Debug, Serialize)]
pub struct PrettyComponent {
    path: PathBuf,
    kind: String,
    short_id: Option<String>,
    properties: PrettyProperties,
}

#[derive(Debug, Serialize)]
pub struct PrettyProperties {
    uid: String,
    created: String,
    start: String,
    end: String,
    duration: String,
    status: String,
    summary: String,
    attendees: String,
}

impl PrettyComponent {
    pub fn new(component: AnyComponent, short_id: Option<String>) -> Self {
        let mut s = Self::from(component);
        s.short_id = short_id;
        s
    }
}

impl From<AnyComponent> for PrettyComponent {
    fn from(value: AnyComponent) -> Self {
        Self {
            path: value.path().to_owned(),
            kind: pretty_kind(value.component_kind()),
            short_id: None,
            properties: PrettyProperties::from(value),
        }
    }
}

fn pretty_kind(component_kind: String) -> String {
    match component_kind.as_str() {
        "VEVENT" => "event".to_owned(),
        "VTODO" => "todo".to_owned(),
        "VJOURNAL" => "journal".to_owned(),
        k => k.to_owned(),
    }
}

impl Formatable for PrettyComponent {
    fn keys(&self) -> Vec<String> {
        vec![
            "short_id".to_owned(),
            "kind".to_owned(),
            "start".to_owned(),
            "end".to_owned(),
            "duration".to_owned(),
            "status".to_owned(),
            "summary".to_owned(),
            "attendees".to_owned(),
        ]
    }
}

impl From<AnyComponent> for PrettyProperties {
    fn from(value: AnyComponent) -> Self {
        Self {
            uid: value.get_uid().map(|s| s.to_owned()).unwrap_or_default(),
            created: value
                .get_created()
                .or_else(|| value.get_timestamp())
                .map(|t| t.to_string())
                .unwrap_or_default(),
            start: value.get_start().map_or(String::new(), |d| {
                date_perhaps_time_to_datetime_utc(d).to_string()
            }),
            end: value.get_end().map_or(String::new(), |d| {
                date_perhaps_time_to_datetime_utc(d).to_string()
            }),
            duration: value
                .get_end()
                .and_then(|end| {
                    value.get_start().map(|start| {
                        date_perhaps_time_to_datetime_utc(end)
                            - date_perhaps_time_to_datetime_utc(start)
                    })
                })
                .map(time_delta_to_string)
                .unwrap_or_default(),
            status: value
                .property_value("STATUS")
                .unwrap_or_default()
                .to_owned(),
            summary: value.get_summary().unwrap_or_default().to_owned(),
            attendees: value
                .property_value("ATTENDEES")
                .map(|a| a.split(',').count().to_string())
                .unwrap_or_default(),
        }
    }
}

fn time_delta_to_string(d: chrono::TimeDelta) -> String {
    let mut s = String::new();
    if d.num_weeks() > 0 {
        s += &format!("{}w", d.num_weeks());
    }
    if d.num_days() % 7 > 0 {
        s += &format!("{}d", d.num_days() % 7);
    }
    if d.num_hours() % 24 > 0 {
        s += &format!("{}h", d.num_hours() % 24);
    }
    if d.num_minutes() % 60 > 0 {
        s += &format!("{}m", d.num_minutes() % 60);
    }
    if d.num_seconds() % 60 > 0 {
        s += &format!("{}s", d.num_seconds() % 60);
    }
    s
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;

    use super::time_delta_to_string;

    #[test]
    fn durations() {
        assert_eq!(time_delta_to_string(TimeDelta::minutes(15)), "15m");
        assert_eq!(time_delta_to_string(TimeDelta::minutes(90)), "1h30m");
        assert_eq!(time_delta_to_string(TimeDelta::days(1)), "1d");
        assert_eq!(
            time_delta_to_string(TimeDelta::days(8) + TimeDelta::hours(2)),
            "1w1d2h"
        );
    }
}

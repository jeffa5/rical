use std::{ops::Deref, str::FromStr};

use chrono::NaiveDateTime;
use icalendar::{Component, Property};

#[derive(Debug)]
pub struct Event(icalendar::Event);

impl Deref for Event {
    type Target = icalendar::Event;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<icalendar::Event> for Event {
    fn from(value: icalendar::Event) -> Self {
        Self(value)
    }
}

impl Event {
    pub fn created(&self) -> Option<NaiveDateTime> {
        self.datetime_property("CREATED")
    }

    pub fn start(&self) -> Option<NaiveDateTime> {
        self.datetime_property("DTSTART")
    }

    pub fn end(&self) -> Option<NaiveDateTime> {
        self.datetime_property("DTEND")
    }

    pub fn summary(&self) -> Option<&str> {
        self.property_value("SUMMARY")
    }

    pub fn status_str(&self) -> Option<&str> {
        self.property_value("STATUS")
    }

    fn datetime_property(&self, name: &str) -> Option<NaiveDateTime> {
        self.0
            .properties()
            .get(name)
            .and_then(|p| parse_datetime(p))
    }
}

fn parse_datetime(prop: &Property) -> Option<NaiveDateTime> {
    // TODO: handle tzid and timezone
    let val = prop.value();
    if val.ends_with("Z") {
        // UTC
        NaiveDateTime::parse_from_str(prop.value(), "%Y%m%dT%H%M%SZ").ok()
    } else {
        NaiveDateTime::parse_from_str(prop.value(), "%Y%m%dT%H%M%S").ok()
    }
}

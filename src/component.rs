use std::path::PathBuf;

use icalendar::{CalendarComponent, Component, Other};
use serde::Serialize;

use crate::{format::Formatable, list::date_perhaps_time_to_datetime_utc};

pub struct AnyComponent {
    path: PathBuf,
    component: CalendarComponent,
}

impl AnyComponent {
    pub fn new(path: PathBuf, component: CalendarComponent) -> Self {
        Self { path, component }
    }
}

impl Component for AnyComponent {
    fn component_kind(&self) -> String {
        match &self.component {
            CalendarComponent::Todo(todo) => todo.component_kind(),
            CalendarComponent::Event(event) => event.component_kind(),
            CalendarComponent::Venue(venue) => venue.component_kind(),
            CalendarComponent::Other(other) => other.component_kind(),
            _ => todo!(),
        }
    }

    fn properties(&self) -> &std::collections::BTreeMap<String, icalendar::Property> {
        match &self.component {
            CalendarComponent::Todo(todo) => todo.properties(),
            CalendarComponent::Event(event) => event.properties(),
            CalendarComponent::Venue(venue) => venue.properties(),
            CalendarComponent::Other(other) => other.properties(),
            _ => todo!(),
        }
    }

    fn components(&self) -> &[Other] {
        match &self.component {
            CalendarComponent::Todo(todo) => todo.components(),
            CalendarComponent::Event(event) => event.components(),
            CalendarComponent::Venue(venue) => venue.components(),
            CalendarComponent::Other(other) => other.components(),
            _ => todo!(),
        }
    }

    fn multi_properties(&self) -> &std::collections::BTreeMap<String, Vec<icalendar::Property>> {
        match &self.component {
            CalendarComponent::Todo(todo) => todo.multi_properties(),
            CalendarComponent::Event(event) => event.multi_properties(),
            CalendarComponent::Venue(venue) => venue.multi_properties(),
            CalendarComponent::Other(other) => other.multi_properties(),
            _ => todo!(),
        }
    }

    fn append_property(&mut self, property: impl Into<icalendar::Property>) -> &mut Self {
        match &mut self.component {
            CalendarComponent::Todo(todo) => {
                todo.append_property(property);
            }
            CalendarComponent::Event(event) => {
                event.append_property(property);
            }
            CalendarComponent::Venue(venue) => {
                venue.append_property(property);
            }
            CalendarComponent::Other(other) => {
                other.append_property(property);
            }
            _ => todo!(),
        }
        self
    }

    fn append_component(&mut self, child: impl Into<Other>) -> &mut Self {
        match &mut self.component {
            CalendarComponent::Todo(todo) => {
                todo.append_component(child);
            }
            CalendarComponent::Event(event) => {
                event.append_component(child);
            }
            CalendarComponent::Venue(venue) => {
                venue.append_component(child);
            }
            CalendarComponent::Other(other) => {
                other.append_component(child);
            }
            _ => todo!(),
        }
        self
    }

    fn append_multi_property(&mut self, property: impl Into<icalendar::Property>) -> &mut Self {
        match &mut self.component {
            CalendarComponent::Todo(todo) => {
                todo.append_multi_property(property);
            }
            CalendarComponent::Event(event) => {
                event.append_multi_property(property);
            }
            CalendarComponent::Venue(venue) => {
                venue.append_multi_property(property);
            }
            CalendarComponent::Other(other) => {
                other.append_multi_property(property);
            }
            _ => todo!(),
        }
        self
    }

    fn remove_property(&mut self, key: &str) -> &mut Self {
        match &mut self.component {
            CalendarComponent::Todo(todo) => {
                todo.remove_property(key);
            }
            CalendarComponent::Event(event) => {
                event.remove_property(key);
            }
            CalendarComponent::Venue(venue) => {
                venue.remove_property(key);
            }
            CalendarComponent::Other(other) => {
                other.remove_property(key);
            }
            _ => todo!(),
        }
        self
    }

    fn remove_multi_property(&mut self, key: &str) -> &mut Self {
        match &mut self.component {
            CalendarComponent::Todo(todo) => {
                todo.remove_multi_property(key);
            }
            CalendarComponent::Event(event) => {
                event.remove_multi_property(key);
            }
            CalendarComponent::Venue(venue) => {
                venue.remove_multi_property(key);
            }
            CalendarComponent::Other(other) => {
                other.remove_multi_property(key);
            }
            _ => todo!(),
        }
        self
    }
}

#[derive(Debug, Serialize)]
pub struct FormatableComponent {
    path: PathBuf,
    kind: String,
    uid: String,
    created: String,
    start: String,
    status: String,
    summary: String,
    organizer: String,
    attendees: String,
}

impl From<AnyComponent> for FormatableComponent {
    fn from(value: AnyComponent) -> Self {
        Self {
            path: value.path.clone(),
            kind: value.component_kind(),
            uid: value.get_uid().map(|s| s.to_owned()).unwrap_or_default(),
            created: value
                .get_created()
                .or_else(|| value.get_timestamp())
                .map(|t| t.to_string())
                .unwrap_or_default(),
            start: value.get_start().map_or(String::new(), |d| {
                date_perhaps_time_to_datetime_utc(d).to_string()
            }),
            status: value
                .property_value("STATUS")
                .unwrap_or_default()
                .to_owned(),
            summary: value.get_summary().unwrap_or_default().to_owned(),
            organizer: value
                .property_value("ORGANIZER")
                .unwrap_or_default()
                .to_owned(),
            attendees: value
                .property_value("ATTENDEES")
                .map(|a| a.split(',').count().to_string())
                .unwrap_or_default(),
        }
    }
}

impl Formatable for FormatableComponent {
    fn keys(&self) -> Vec<String> {
        vec![
            "kind".to_owned(),
            "start".to_owned(),
            "status".to_owned(),
            "summary".to_owned(),
            "organizer".to_owned(),
            "attendees".to_owned(),
            "path".to_owned(),
        ]
    }

    fn verbose_keys(&self) -> Vec<String> {
        vec![
            "kind".to_owned(),
            "uid".to_owned(),
            "created".to_owned(),
            "start".to_owned(),
            "status".to_owned(),
            "summary".to_owned(),
            "organizer".to_owned(),
            "attendees".to_owned(),
            "path".to_owned(),
        ]
    }
}

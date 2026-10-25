use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use icalendar::{CalendarComponent, Component, Property};
use serde::{Serialize, ser::SerializeStruct};

/// A component of an item (an event, todo, ...) and the file it is in.
///
/// It offers the read side of icalendar's `Component` trait as methods of
/// its own rather than implementing the trait, whose signatures name the
/// crate's catch-all `Other` type that it does not export.
#[derive(Debug, Clone)]
pub struct AnyComponent {
    path: PathBuf,
    component: CalendarComponent,
}

/// Run `$f` on the component inside whichever variant it is.
macro_rules! inner {
    ($self:expr, $c:ident => $f:expr) => {
        match &$self.component {
            CalendarComponent::Todo($c) => $f,
            CalendarComponent::Event($c) => $f,
            CalendarComponent::Venue($c) => $f,
            CalendarComponent::Other($c) => $f,
            _ => unreachable!("icalendar added a component kind"),
        }
    };
}

impl AnyComponent {
    pub fn new(path: PathBuf, component: CalendarComponent) -> Self {
        Self { path, component }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn component(&self) -> &CalendarComponent {
        &self.component
    }

    /// "VEVENT", "VTODO", ...
    pub fn component_kind(&self) -> String {
        inner!(self, c => c.component_kind())
    }

    /// Properties that occur once, by name.
    pub fn properties(&self) -> &BTreeMap<String, Property> {
        inner!(self, c => c.properties())
    }

    /// Properties that may repeat (ATTENDEE, EXDATE, ...), by name.
    pub fn multi_properties(&self) -> &BTreeMap<String, Vec<Property>> {
        inner!(self, c => c.multi_properties())
    }

    /// Nested components, such as VALARMs.
    pub fn components(&self) -> &[impl Component] {
        inner!(self, c => c.components())
    }

    /// The value of a property that occurs once.
    pub fn property_value(&self, key: &str) -> Option<&str> {
        self.properties().get(key).map(Property::value)
    }

    pub fn get_uid(&self) -> Option<&str> {
        self.property_value("UID")
    }

    pub fn get_summary(&self) -> Option<&str> {
        self.property_value("SUMMARY")
    }

    /// DTSTART.
    pub fn get_start(&self) -> Option<icalendar::DatePerhapsTime> {
        inner!(self, c => c.get_start())
    }

    /// DTEND.
    pub fn get_end(&self) -> Option<icalendar::DatePerhapsTime> {
        inner!(self, c => c.get_end())
    }

    /// CREATED.
    pub fn get_created(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        inner!(self, c => c.get_created())
    }

    /// DTSTAMP.
    pub fn get_timestamp(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        inner!(self, c => c.get_timestamp())
    }
}

impl Serialize for AnyComponent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_struct("AnyComponent", 3)?;
        s.serialize_field("path", &self.path)?;
        s.serialize_field("kind", &self.component_kind())?;
        let props = self
            .properties()
            .iter()
            .map(|(k, v)| (k, v.value()))
            .collect::<BTreeMap<_, _>>();
        s.serialize_field("properties", &props)?;
        s.end()
    }
}

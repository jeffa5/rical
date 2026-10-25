use std::{collections::BTreeMap, path::Path, sync::Arc};

use chrono::{DateTime, Utc};
use serde::{Serialize, ser::SerializeStruct};

use crate::{
    compact::{Component, Prop},
    when::When,
};

/// A component of an item (an event, todo, ...) and the file it is in, in
/// the compact parsed form kept in memory (see [`crate::compact`]).
#[derive(Debug, Clone)]
pub struct AnyComponent {
    /// Shared by the components of one file.
    path: Arc<Path>,
    component: Component,
}

impl AnyComponent {
    pub fn new(path: Arc<Path>, component: Component) -> Self {
        Self { path, component }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// "VEVENT", "VTODO", ...
    pub fn component_kind(&self) -> &str {
        &self.component.kind
    }

    /// The first property called `name` (upper case).
    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.component.prop(name)
    }

    /// Every property called `name`, in order (ATTENDEE, EXDATE, ...).
    pub fn props<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Prop> {
        self.component.props(name)
    }

    /// All its properties, in file order.
    pub fn all_props(&self) -> &[Prop] {
        self.component.all_props()
    }

    /// Nested components, such as VALARMs.
    pub fn components(&self) -> &[Component] {
        self.component.children()
    }

    /// The value of the first property called `name`.
    pub fn property_value(&self, name: &str) -> Option<&str> {
        self.component.value(name)
    }

    pub fn get_uid(&self) -> Option<&str> {
        self.property_value("UID")
    }

    pub fn get_summary(&self) -> Option<&str> {
        self.property_value("SUMMARY")
    }

    /// DTSTART.
    pub fn get_start(&self) -> Option<When> {
        self.prop("DTSTART").and_then(When::from_property)
    }

    /// DTEND.
    pub fn get_end(&self) -> Option<When> {
        self.prop("DTEND").and_then(When::from_property)
    }

    /// CREATED.
    pub fn get_created(&self) -> Option<DateTime<Utc>> {
        self.utc("CREATED")
    }

    /// DTSTAMP.
    pub fn get_timestamp(&self) -> Option<DateTime<Utc>> {
        self.utc("DTSTAMP")
    }

    fn utc(&self, name: &str) -> Option<DateTime<Utc>> {
        self.prop(name)
            .and_then(When::from_property)
            .map(|w| w.to_utc(chrono_tz::Tz::UTC))
    }
}

impl Serialize for AnyComponent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut s = serializer.serialize_struct("AnyComponent", 3)?;
        s.serialize_field("path", &*self.path)?;
        s.serialize_field("kind", self.component_kind())?;
        // as before: one value per name (the last, as a map keeps it)
        let props = self
            .all_props()
            .iter()
            .map(|p| (&*p.name, p.value()))
            .collect::<BTreeMap<_, _>>();
        s.serialize_field("properties", &props)?;
        s.end()
    }
}

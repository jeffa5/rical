use icalendar::{CalendarComponent, Component, Other};

pub struct AnyComponent(CalendarComponent);

impl From<CalendarComponent> for AnyComponent {
    fn from(value: CalendarComponent) -> Self {
        Self(value)
    }
}

impl Component for AnyComponent {
    fn component_kind(&self) -> String {
        match &self.0 {
            CalendarComponent::Todo(todo) => todo.component_kind(),
            CalendarComponent::Event(event) => event.component_kind(),
            CalendarComponent::Venue(venue) => venue.component_kind(),
            CalendarComponent::Other(other) => other.component_kind(),
            _ => todo!(),
        }
    }

    fn properties(&self) -> &std::collections::BTreeMap<String, icalendar::Property> {
        match &self.0 {
            CalendarComponent::Todo(todo) => todo.properties(),
            CalendarComponent::Event(event) => event.properties(),
            CalendarComponent::Venue(venue) => venue.properties(),
            CalendarComponent::Other(other) => other.properties(),
            _ => todo!(),
        }
    }

    fn components(&self) -> &[Other] {
        match &self.0 {
            CalendarComponent::Todo(todo) => todo.components(),
            CalendarComponent::Event(event) => event.components(),
            CalendarComponent::Venue(venue) => venue.components(),
            CalendarComponent::Other(other) => other.components(),
            _ => todo!(),
        }
    }

    fn multi_properties(&self) -> &std::collections::BTreeMap<String, Vec<icalendar::Property>> {
        match &self.0 {
            CalendarComponent::Todo(todo) => todo.multi_properties(),
            CalendarComponent::Event(event) => event.multi_properties(),
            CalendarComponent::Venue(venue) => venue.multi_properties(),
            CalendarComponent::Other(other) => other.multi_properties(),
            _ => todo!(),
        }
    }

    fn append_property(&mut self, property: impl Into<icalendar::Property>) -> &mut Self {
        match &mut self.0 {
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
        match &mut self.0 {
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
        match &mut self.0 {
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
        match &mut self.0 {
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
        match &mut self.0 {
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

//! A line-preserving model of an `.ics` file for editing.
//!
//! Parsing keeps every content line's original bytes (including folding and
//! line endings), so writing a document back out reproduces the file exactly
//! except for the lines that were changed. This is how edits avoid losing
//! properties, parameters or components that rical does not understand.

use std::fmt::Write as _;

use crate::when::When;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Original text including folds and the trailing line ending; empty for
    /// lines created by edits until written.
    raw: String,
    /// Unfolded content without the line ending.
    content: String,
}

impl Line {
    fn parsed(raw: String) -> Self {
        let content = unfold(&raw);
        Self { raw, content }
    }

    /// A new line from unfolded content; folded when written.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            raw: String::new(),
            content: content.into(),
        }
    }

    /// Build `NAME;K=V;...:value`.
    pub fn property(name: &str, params: &[(&str, &str)], value: &str) -> Self {
        let mut s = name.to_owned();
        for (k, v) in params {
            let _ = write!(s, ";{k}={}", quote_param(v));
        }
        s.push(':');
        s.push_str(value);
        Self::new(s)
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    /// Property name, upper-cased.
    pub fn name(&self) -> String {
        let end = self.content.find([';', ':']).unwrap_or(self.content.len());
        self.content[..end].to_ascii_uppercase()
    }

    /// Parameters as written, with quotes removed.
    pub fn params(&self) -> Vec<(String, String)> {
        let (head, _) = split_value(&self.content);
        let mut params = Vec::new();
        let mut rest = match head.find(';') {
            Some(i) => &head[i + 1..],
            None => return params,
        };
        while !rest.is_empty() {
            let Some(eq) = rest.find('=') else { break };
            let key = rest[..eq].to_ascii_uppercase();
            let after = &rest[eq + 1..];
            let (value, next) = if let Some(stripped) = after.strip_prefix('"') {
                let close = stripped.find('"').unwrap_or(stripped.len());
                let next = stripped[close..].trim_start_matches('"');
                (stripped[..close].to_owned(), next)
            } else {
                let end = after.find(';').unwrap_or(after.len());
                (after[..end].to_owned(), &after[end..])
            };
            params.push((key, value));
            rest = next.strip_prefix(';').unwrap_or(next);
        }
        params
    }

    pub fn param(&self, key: &str) -> Option<String> {
        self.params()
            .into_iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn value(&self) -> &str {
        split_value(&self.content).1
    }

    pub fn when(&self) -> Option<When> {
        When::parse(self.value(), self.param("TZID").as_deref())
    }

    fn write(&self, out: &mut String, eol: &str) {
        if self.raw.is_empty() {
            fold(&self.content, eol, out);
        } else {
            out.push_str(&self.raw);
        }
    }
}

/// Split content into the part before the value and the value, at the first
/// colon outside quotes.
fn split_value(content: &str) -> (&str, &str) {
    let mut quoted = false;
    for (i, c) in content.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => return (&content[..i], &content[i + 1..]),
            _ => {}
        }
    }
    (content, "")
}

fn quote_param(v: &str) -> String {
    if v.contains([':', ';', ',']) {
        format!("\"{v}\"")
    } else {
        v.to_owned()
    }
}

fn unfold(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for (i, part) in raw.split('\n').enumerate() {
        let part = part.strip_suffix('\r').unwrap_or(part);
        if i == 0 {
            out.push_str(part);
        } else if let Some(rest) = part.strip_prefix([' ', '\t']) {
            out.push_str(rest);
        }
    }
    out
}

/// Fold to lines of at most 75 octets, never splitting a character.
fn fold(content: &str, eol: &str, out: &mut String) {
    let mut width = 0;
    for c in content.chars() {
        let len = c.len_utf8();
        if width + len > 75 {
            out.push_str(eol);
            out.push(' ');
            width = 1;
        }
        out.push(c);
        width += len;
    }
    out.push_str(eol);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Line(Line),
    Component(Component),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    begin: Line,
    pub items: Vec<Item>,
    end: Line,
}

impl Component {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_ascii_uppercase(),
            begin: Line::new(format!("BEGIN:{name}")),
            items: Vec::new(),
            end: Line::new(format!("END:{name}")),
        }
    }

    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.items.iter().filter_map(|i| match i {
            Item::Line(l) => Some(l),
            Item::Component(_) => None,
        })
    }

    pub fn components(&self) -> impl Iterator<Item = &Component> {
        self.items.iter().filter_map(|i| match i {
            Item::Component(c) => Some(c),
            Item::Line(_) => None,
        })
    }

    pub fn components_mut(&mut self) -> impl Iterator<Item = &mut Component> {
        self.items.iter_mut().filter_map(|i| match i {
            Item::Component(c) => Some(c),
            Item::Line(_) => None,
        })
    }

    pub fn property(&self, name: &str) -> Option<&Line> {
        self.lines().find(|l| l.name() == name)
    }

    pub fn properties<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Line> {
        self.lines().filter(move |l| l.name() == name)
    }

    pub fn uid(&self) -> Option<&str> {
        self.property("UID").map(|l| l.value())
    }

    pub fn recurrence_id(&self) -> Option<When> {
        self.property("RECURRENCE-ID").and_then(Line::when)
    }

    /// Replace the first `name` property with `line` (removing any others),
    /// or add it after the existing properties.
    pub fn set(&mut self, line: Line) {
        let name = line.name();
        let mut line = Some(line);
        self.items.retain_mut(|item| match item {
            Item::Line(l) if l.name() == name => match line.take() {
                Some(new) => {
                    if *l != new {
                        *l = new;
                    }
                    true
                }
                None => false,
            },
            _ => true,
        });
        if let Some(line) = line {
            self.insert(line);
        }
    }

    /// Add a property after the existing ones, before any subcomponents.
    pub fn insert(&mut self, line: Line) {
        let at = self
            .items
            .iter()
            .position(|i| matches!(i, Item::Component(_)))
            .unwrap_or(self.items.len());
        self.items.insert(at, Item::Line(line));
    }

    pub fn remove(&mut self, name: &str) {
        self.items
            .retain(|i| !matches!(i, Item::Line(l) if l.name() == name));
    }

    pub fn set_text(&mut self, name: &str, value: &str) {
        self.set(Line::property(name, &[], &escape_text(value)));
    }

    /// Set a date/time property, writing `when` in its own form.
    pub fn set_when(&mut self, name: &str, when: &When) {
        self.set(when_line(name, when));
    }

    /// Forget the original text of every line so the component is written
    /// fresh, e.g. before moving it into a file with other line endings.
    pub fn detach(&mut self) {
        self.begin.raw.clear();
        self.end.raw.clear();
        for item in &mut self.items {
            match item {
                Item::Line(l) => l.raw.clear(),
                Item::Component(c) => c.detach(),
            }
        }
    }

    fn write(&self, out: &mut String, eol: &str) {
        self.begin.write(out, eol);
        for item in &self.items {
            match item {
                Item::Line(l) => l.write(out, eol),
                Item::Component(c) => c.write(out, eol),
            }
        }
        self.end.write(out, eol);
    }
}

pub fn when_line(name: &str, when: &When) -> Line {
    match when {
        When::Date(d) => {
            Line::property(name, &[("VALUE", "DATE")], &d.format("%Y%m%d").to_string())
        }
        When::Floating(ndt) => Line::property(name, &[], &ndt.format("%Y%m%dT%H%M%S").to_string()),
        When::Zoned(dt) if dt.timezone() == chrono_tz::Tz::UTC => {
            Line::property(name, &[], &dt.format("%Y%m%dT%H%M%SZ").to_string())
        }
        When::Zoned(dt) => Line::property(
            name,
            &[("TZID", dt.timezone().name())],
            &dt.format("%Y%m%dT%H%M%S").to_string(),
        ),
    }
}

/// Escape iCalendar TEXT.
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// A whole `.ics` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    /// Lines outside any component (normally none), kept as they were.
    leading: Vec<Line>,
    pub root: Component,
    trailing: String,
    eol: &'static str,
}

impl Document {
    pub fn parse(text: &str) -> anyhow::Result<Self> {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };

        // group physical lines into logical (unfolded) lines
        let mut logical: Vec<String> = Vec::new();
        for physical in text.split_inclusive('\n') {
            let is_continuation = physical.starts_with([' ', '\t']);
            match logical.last_mut() {
                Some(last) if is_continuation => last.push_str(physical),
                _ => logical.push(physical.to_owned()),
            }
        }

        let mut leading = Vec::new();
        let mut stack: Vec<Component> = Vec::new();
        let mut root = None;
        let mut trailing = String::new();
        for raw in logical {
            if root.is_some() {
                trailing.push_str(&raw);
                continue;
            }
            let line = Line::parsed(raw);
            let name = line.name();
            if name == "BEGIN" {
                stack.push(Component {
                    name: line.value().trim().to_ascii_uppercase(),
                    begin: line,
                    items: Vec::new(),
                    end: Line::new(""),
                });
            } else if name == "END" {
                let mut c = stack
                    .pop()
                    .ok_or_else(|| anyhow::anyhow!("END without BEGIN"))?;
                if !line.value().trim().eq_ignore_ascii_case(&c.name) {
                    anyhow::bail!("END:{} closes BEGIN:{}", line.value(), c.name);
                }
                c.end = line;
                match stack.last_mut() {
                    Some(parent) => parent.items.push(Item::Component(c)),
                    None => root = Some(c),
                }
            } else {
                match stack.last_mut() {
                    Some(c) => c.items.push(Item::Line(line)),
                    None => leading.push(line),
                }
            }
        }
        if let Some(c) = stack.pop() {
            anyhow::bail!("BEGIN:{} is never closed", c.name);
        }
        let root = root.ok_or_else(|| anyhow::anyhow!("no calendar in file"))?;
        if root.name != "VCALENDAR" {
            anyhow::bail!("expected VCALENDAR, found {}", root.name);
        }
        Ok(Self {
            leading,
            root,
            trailing,
            eol,
        })
    }

    /// A new, empty calendar.
    pub fn new_calendar() -> Self {
        let mut root = Component::new("VCALENDAR");
        root.insert(Line::new("VERSION:2.0"));
        root.insert(Line::new("PRODID:-//rical//rical//EN"));
        Self {
            leading: Vec::new(),
            root,
            trailing: String::new(),
            eol: "\r\n",
        }
    }

    pub fn write(&self) -> String {
        let mut out = String::new();
        for l in &self.leading {
            l.write(&mut out, self.eol);
        }
        self.root.write(&mut out, self.eol);
        out.push_str(&self.trailing);
        out
    }

    /// Events, todos and journals in the calendar.
    pub fn items(&self) -> impl Iterator<Item = &Component> {
        self.root
            .components()
            .filter(|c| matches!(c.name.as_str(), "VEVENT" | "VTODO" | "VJOURNAL"))
    }

    /// The component for `uid` with the given RECURRENCE-ID (None for the
    /// master).
    pub fn find_mut(&mut self, uid: &str, recurrence_id: Option<&When>) -> Option<&mut Component> {
        self.root.components_mut().find(|c| {
            matches!(c.name.as_str(), "VEVENT" | "VTODO" | "VJOURNAL")
                && c.uid() == Some(uid)
                && match (recurrence_id, c.recurrence_id()) {
                    (None, None) => true,
                    (Some(want), Some(have)) => have.coerce_like(want) == *want,
                    _ => false,
                }
        })
    }

    pub fn find(&self, uid: &str, recurrence_id: Option<&When>) -> Option<&Component> {
        self.items().find(|c| {
            c.uid() == Some(uid)
                && match (recurrence_id, c.recurrence_id()) {
                    (None, None) => true,
                    (Some(want), Some(have)) => have.coerce_like(want) == *want,
                    _ => false,
                }
        })
    }

    /// Add a component after the existing ones.
    pub fn push(&mut self, c: Component) {
        self.root.items.push(Item::Component(c));
    }

    /// Remove components matching `f`.
    pub fn remove_where(&mut self, f: impl Fn(&Component) -> bool) {
        self.root
            .items
            .retain(|i| !matches!(i, Item::Component(c) if f(c)));
    }

    pub fn has_timezone(&self, tzid: &str) -> bool {
        self.root
            .components()
            .any(|c| c.name == "VTIMEZONE" && c.property("TZID").is_some_and(|l| l.value() == tzid))
    }

    /// The VTIMEZONE for `tzid`, if this file has one.
    pub fn timezone(&self, tzid: &str) -> Option<&Component> {
        self.root.components().find(|c| {
            c.name == "VTIMEZONE" && c.property("TZID").is_some_and(|l| l.value() == tzid)
        })
    }

    /// Insert a VTIMEZONE before the first event-like component. It may
    /// come from another file, so it is rewritten in this file's line
    /// endings.
    pub fn add_timezone(&mut self, mut tz: Component) {
        tz.detach();
        let at = self
            .root
            .items
            .iter()
            .position(|i| matches!(i, Item::Component(c) if c.name != "VTIMEZONE"))
            .unwrap_or(self.root.items.len());
        self.root.items.insert(at, Item::Component(tz));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//y//EN\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:abc\r\nDTSTART;TZID=Europe/London:20250304T090000\r\nDTEND;TZID=Europe/London:20250304T100000\r\nSUMMARY:A long summary that goes on and on so that it has to be folded acros\r\n s more than one line\r\nX-CUSTOM;X-PARAM=\"a:b;c\":keep me\r\nATTENDEE;CN=\"Smith, J\":mailto:j@example.com\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn round_trip_is_exact() {
        let doc = Document::parse(SAMPLE).unwrap();
        assert_eq!(doc.write(), SAMPLE);

        let lf = SAMPLE.replace("\r\n", "\n");
        assert_eq!(Document::parse(&lf).unwrap().write(), lf);

        // odd folding and trailing junk survive too
        let odd = "BEGIN:VCALENDAR\nX-A:1\n\t2\nEND:VCALENDAR\n\n";
        assert_eq!(Document::parse(odd).unwrap().write(), odd);
    }

    #[test]
    fn parse_lines() {
        let doc = Document::parse(SAMPLE).unwrap();
        let ev = doc.find("abc", None).unwrap();
        assert_eq!(
            ev.property("SUMMARY").unwrap().value(),
            "A long summary that goes on and on so that it has to be folded across more than one line"
        );
        let x = ev.property("X-CUSTOM").unwrap();
        assert_eq!(x.param("X-PARAM").as_deref(), Some("a:b;c"));
        assert_eq!(x.value(), "keep me");
        assert_eq!(
            ev.property("ATTENDEE").unwrap().value(),
            "mailto:j@example.com"
        );
        assert_eq!(
            ev.property("DTSTART").unwrap().when(),
            When::parse("20250304T090000", Some("Europe/London"))
        );
    }

    #[test]
    fn edits_touch_only_changed_lines() {
        let mut doc = Document::parse(SAMPLE).unwrap();
        let ev = doc.find_mut("abc", None).unwrap();
        ev.set_text("SUMMARY", "Short, sweet");
        ev.set_when(
            "DTSTART",
            &When::parse("20250304T100000", Some("Europe/London")).unwrap(),
        );
        ev.set(Line::new("SEQUENCE:1"));
        let out = doc.write();
        let expected = SAMPLE
            .replace(
                "SUMMARY:A long summary that goes on and on so that it has to be folded acros\r\n s more than one line\r\n",
                "SUMMARY:Short\\, sweet\r\n",
            )
            .replace(
                "DTSTART;TZID=Europe/London:20250304T090000",
                "DTSTART;TZID=Europe/London:20250304T100000",
            )
            .replace(
                "ATTENDEE;CN=\"Smith, J\":mailto:j@example.com\r\n",
                "ATTENDEE;CN=\"Smith, J\":mailto:j@example.com\r\nSEQUENCE:1\r\n",
            );
        assert_eq!(out, expected);
    }

    #[test]
    fn long_lines_are_folded() {
        let mut doc = Document::parse(SAMPLE).unwrap();
        let summary = "é".repeat(60);
        doc.find_mut("abc", None)
            .unwrap()
            .set_text("SUMMARY", &summary);
        let out = doc.write();
        for line in out.split("\r\n") {
            assert!(line.len() <= 75, "{line:?} is too long");
        }
        let reparsed = Document::parse(&out).unwrap();
        assert_eq!(
            reparsed
                .find("abc", None)
                .unwrap()
                .property("SUMMARY")
                .unwrap()
                .value(),
            summary
        );
    }

    #[test]
    fn remove_and_insert() {
        let mut doc = Document::parse(SAMPLE).unwrap();
        let ev = doc.find_mut("abc", None).unwrap();
        ev.remove("X-CUSTOM");
        ev.set(Line::new("RRULE:FREQ=WEEKLY"));
        let out = doc.write();
        assert!(!out.contains("X-CUSTOM"));
        // new properties go before the VALARM
        assert!(out.find("RRULE").unwrap() < out.find("BEGIN:VALARM").unwrap());
    }

    #[test]
    fn rejects_broken_files() {
        assert!(Document::parse("BEGIN:VCALENDAR\nBEGIN:VEVENT\nEND:VCALENDAR\n").is_err());
        assert!(Document::parse("garbage\n").is_err());
        assert!(Document::parse("BEGIN:VCALENDAR\nEND:VEVENT\n").is_err());
    }
}

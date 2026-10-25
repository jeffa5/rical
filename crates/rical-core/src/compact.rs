//! A compact parsed form of `.ics` items, for keeping a whole vdir in
//! memory cheaply: one pass over the text, properties in file order with
//! their names shared (not a string each), and VTIMEZONEs skipped (zone
//! rules come from the built-in time zone database; their TZIDs are noted
//! so a file can be found to copy one from).

use std::borrow::Cow;

/// A property or component name, upper-cased; common ones are shared.
pub type Name = Cow<'static, str>;

/// Names that occur over and over, shared rather than allocated each time.
const COMMON: &[&str] = &[
    "ACTION",
    "ATTACH",
    "ATTENDEE",
    "CATEGORIES",
    "CLASS",
    "CN",
    "COMPLETED",
    "CREATED",
    "CUTYPE",
    "DESCRIPTION",
    "DTEND",
    "DTSTAMP",
    "DTSTART",
    "DUE",
    "DURATION",
    "EMAIL",
    "EXDATE",
    "LANGUAGE",
    "LAST-MODIFIED",
    "LOCATION",
    "ORGANIZER",
    "PARTSTAT",
    "PERCENT-COMPLETE",
    "PRIORITY",
    "RANGE",
    "RDATE",
    "RECURRENCE-ID",
    "RELATED",
    "RELATED-TO",
    "RELTYPE",
    "ROLE",
    "RRULE",
    "RSVP",
    "SCHEDULE-STATUS",
    "SEQUENCE",
    "STATUS",
    "SUMMARY",
    "TRANSP",
    "TRIGGER",
    "TZID",
    "UID",
    "URL",
    "VALARM",
    "VALUE",
    "VEVENT",
    "VJOURNAL",
    "VTODO",
    "X-MOZ-GENERATION",
    "X-MOZ-LASTACK",
];

fn name(raw: &str) -> Name {
    let upper = raw.to_ascii_uppercase();
    match COMMON.binary_search(&upper.as_str()) {
        Ok(i) => Cow::Borrowed(COMMON[i]),
        Err(_) => Cow::Owned(upper),
    }
}

/// One property: `NAME;PARAM=value:value`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prop {
    pub name: Name,
    params: Box<[(Name, Box<str>)]>,
    value: Box<str>,
}

impl Prop {
    pub fn value(&self) -> &str {
        &self.value
    }

    /// A parameter's value (quotes removed), by name in any case.
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| &**v)
    }

    pub fn params(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(k, v)| (&**k, &**v))
    }
}

/// A component (VEVENT, VTODO, VALARM, ...) with its properties in file
/// order and any nested components.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Component {
    pub kind: Name,
    props: Vec<Prop>,
    children: Vec<Component>,
}

impl Component {
    /// The first property called `name` (upper case).
    pub fn prop(&self, name: &str) -> Option<&Prop> {
        self.props.iter().find(|p| p.name == name)
    }

    /// Every property called `name`, in order.
    pub fn props<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Prop> {
        self.props.iter().filter(move |p| p.name == name)
    }

    pub fn all_props(&self) -> &[Prop] {
        &self.props
    }

    pub fn value(&self, name: &str) -> Option<&str> {
        self.prop(name).map(Prop::value)
    }

    pub fn children(&self) -> &[Component] {
        &self.children
    }
}

/// The parts of a content line: name, parameters and value. The value
/// starts after the first colon outside double quotes.
fn split_line(line: &str) -> Option<Prop> {
    let mut quoted = false;
    let mut colon = None;
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                colon = Some(i);
                break;
            }
            _ => {}
        }
    }
    let colon = colon?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let n = parts.next()?.trim();
    if n.is_empty() {
        return None;
    }
    // parameters: KEY=value or KEY="quoted; value"
    let mut params = Vec::new();
    let rest = &head[n.len()..];
    let mut s = rest.strip_prefix(';').unwrap_or(rest);
    while !s.is_empty() {
        let Some(eq) = s.find('=') else { break };
        let key = &s[..eq];
        let after = &s[eq + 1..];
        let (v, next) = if let Some(q) = after.strip_prefix('"') {
            let close = q.find('"').unwrap_or(q.len());
            (&q[..close], q.get(close + 1..).unwrap_or(""))
        } else {
            let end = after.find(';').unwrap_or(after.len());
            (&after[..end], &after[end..])
        };
        params.push((name(key.trim()), Box::from(v)));
        s = next.strip_prefix(';').unwrap_or(next);
    }
    Some(Prop {
        name: name(n),
        params: params.into_boxed_slice(),
        value: Box::from(value),
    })
}

/// The top-level components in `text` (those inside VCALENDAR), and the
/// TZIDs of the VTIMEZONEs it skipped.
pub fn parse(text: &str) -> anyhow::Result<(Vec<Component>, Vec<String>)> {
    let mut out = Vec::new();
    let mut tzids = Vec::new();
    let mut stack: Vec<Component> = Vec::new();
    let mut in_timezone = 0usize;
    let mut seen_calendar = false;
    let mut logical = String::new();

    let mut handle =
        |line: &str, stack: &mut Vec<Component>, out: &mut Vec<Component>| -> anyhow::Result<()> {
            if line.is_empty() {
                return Ok(());
            }
            let begin = line
                .get(..6)
                .is_some_and(|p| p.eq_ignore_ascii_case("BEGIN:"));
            let end = line
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case("END:"));
            if begin {
                let kind = name(line[6..].trim());
                if in_timezone > 0 || kind == "VTIMEZONE" {
                    in_timezone += 1;
                } else if kind == "VCALENDAR" && stack.is_empty() {
                    seen_calendar = true;
                } else {
                    stack.push(Component {
                        kind,
                        ..Component::default()
                    });
                }
            } else if end {
                let kind = line[4..].trim();
                if in_timezone > 0 {
                    in_timezone -= 1;
                } else if kind.eq_ignore_ascii_case("VCALENDAR") && stack.is_empty() {
                } else {
                    let mut c = stack
                        .pop()
                        .ok_or_else(|| anyhow::anyhow!("END:{kind} without BEGIN"))?;
                    if !c.kind.eq_ignore_ascii_case(kind) {
                        anyhow::bail!("END:{kind} closes BEGIN:{}", c.kind);
                    }
                    c.props.shrink_to_fit();
                    c.children.shrink_to_fit();
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(c),
                        None => out.push(c),
                    }
                }
            } else if in_timezone > 0 {
                if line
                    .get(..5)
                    .is_some_and(|p| p.eq_ignore_ascii_case("TZID:"))
                {
                    tzids.push(line[5..].trim().to_owned());
                }
            } else if let Some(c) = stack.last_mut()
                && let Some(p) = split_line(line)
            {
                c.props.push(p);
            }
            Ok(())
        };

    // unfold: a line starting with a space or tab continues the last one
    for raw in text.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(cont) = raw.strip_prefix([' ', '\t']) {
            logical.push_str(cont);
            continue;
        }
        handle(&logical, &mut stack, &mut out)?;
        logical.clear();
        logical.push_str(raw);
    }
    handle(&logical, &mut stack, &mut out)?;
    if let Some(c) = stack.last() {
        anyhow::bail!("BEGIN:{} is never closed", c.kind);
    }
    if !seen_calendar && out.is_empty() {
        anyhow::bail!("no calendar in file");
    }
    out.shrink_to_fit();
    Ok((out, tzids))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_names_are_sorted() {
        assert!(COMMON.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn parses_items() {
        let text = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
            BEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nBEGIN:STANDARD\r\nDTSTART:19701025T020000\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
            BEGIN:VEVENT\r\nUID:a\r\nDTSTART;TZID=Europe/London:20250304T090000\r\n\
            SUMMARY:A long summary that is fol\r\n ded\r\n\
            ATTENDEE;CN=\"Smith, J\";PARTSTAT=ACCEPTED:mailto:j@example.com\r\n\
            ATTENDEE;CN=Jo:mailto:jo@example.com\r\n\
            X-CUSTOM;X-P=\"a:b;c\":keep me\r\n\
            BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n\
            END:VEVENT\r\nEND:VCALENDAR\r\n";
        let (items, tzids) = parse(text).unwrap();
        assert_eq!(tzids, ["Europe/London"]);
        assert_eq!(items.len(), 1);
        let e = &items[0];
        assert_eq!(e.kind, "VEVENT");
        assert_eq!(e.value("UID"), Some("a"));
        assert_eq!(e.value("SUMMARY"), Some("A long summary that is folded"));
        let start = e.prop("DTSTART").unwrap();
        assert_eq!(start.param("tzid"), Some("Europe/London"));
        assert_eq!(start.value(), "20250304T090000");
        let people = e.props("ATTENDEE").collect::<Vec<_>>();
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].param("CN"), Some("Smith, J"));
        assert_eq!(people[0].value(), "mailto:j@example.com");
        let x = e.prop("X-CUSTOM").unwrap();
        assert_eq!((x.param("X-P"), x.value()), (Some("a:b;c"), "keep me"));
        assert_eq!(e.children()[0].value("TRIGGER"), Some("-PT15M"));
        // shared names, not a string each
        assert!(matches!(e.prop("DTSTART").unwrap().name, Cow::Borrowed(_)));
        assert!(parse("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n").is_err());
    }
}

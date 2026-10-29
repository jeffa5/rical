//! Just enough of RFC 5322 to run a send queue: find, add and remove
//! headers without touching the rest of the message's bytes, and pull the
//! addresses out of address lists.

use std::ops::Range;

/// One header and the bytes it takes up, continuation lines included.
struct Span {
    name: String,
    range: Range<usize>,
}

fn spans(raw: &[u8]) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut pos = 0;
    while pos < raw.len() {
        let end = raw[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(raw.len(), |i| pos + i);
        let next = (end + 1).min(raw.len());
        let line = raw[pos..end].strip_suffix(b"\r").unwrap_or(&raw[pos..end]);
        if line.is_empty() {
            break;
        }
        if matches!(line[0], b' ' | b'\t') {
            match out.last_mut() {
                Some(last) => last.range.end = next,
                None => break,
            }
        } else {
            let Some(colon) = line.iter().position(|&b| b == b':') else {
                break;
            };
            out.push(Span {
                name: String::from_utf8_lossy(&line[..colon]).trim().to_owned(),
                range: pos..next,
            });
        }
        pos = next;
    }
    out
}

fn value(raw: &[u8], span: &Span) -> String {
    let text = String::from_utf8_lossy(&raw[span.range.clone()]);
    let after = text.split_once(':').map_or("", |(_, v)| v);
    // unfold: a line break before whitespace is just that whitespace
    let mut out = String::new();
    for line in after.lines() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(line.trim());
    }
    out.trim().to_owned()
}

/// The first header called `name` (case does not matter), unfolded.
pub fn header(raw: &[u8], name: &str) -> Option<String> {
    spans(raw)
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name))
        .map(|s| value(raw, s))
}

/// Every header called `name`, unfolded, in order.
pub fn headers(raw: &[u8], name: &str) -> Vec<String> {
    spans(raw)
        .iter()
        .filter(|s| s.name.eq_ignore_ascii_case(name))
        .map(|s| value(raw, s))
        .collect()
}

/// The message without the headers `drop` says to. Everything else is
/// copied byte for byte.
pub fn remove_headers(raw: &[u8], drop: impl Fn(&str) -> bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len());
    let mut at = 0;
    for s in spans(raw).iter().filter(|s| drop(&s.name)) {
        out.extend_from_slice(&raw[at..s.range.start]);
        at = s.range.end;
    }
    out.extend_from_slice(&raw[at..]);
    out
}

/// The message with `name` set to `value`, replacing any it had, placed
/// first. Line breaks in the value are dropped.
pub fn set_header(raw: &[u8], name: &str, value: &str) -> Vec<u8> {
    let crlf = raw
        .iter()
        .position(|&b| b == b'\n')
        .is_some_and(|i| i > 0 && raw[i - 1] == b'\r');
    let nl = if crlf { "\r\n" } else { "\n" };
    let value = value.replace(['\r', '\n'], " ");
    let mut out = format!("{name}: {}{nl}", value.trim()).into_bytes();
    out.extend(remove_headers(raw, |n| n.eq_ignore_ascii_case(name)));
    out
}

/// The mailboxes in an address list ("Ann <ann@x>, bob@y (Bob)"), as bare
/// addresses. Groups are flattened; anything without an `@` is ignored.
pub fn addresses(list: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut cur = String::new();
    let (mut quote, mut angle, mut paren, mut escaped) = (false, false, 0u32, false);
    for c in list.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' if quote || paren > 0 => {
                escaped = true;
                cur.push(c);
            }
            '"' if paren == 0 => {
                quote = !quote;
                cur.push(c);
            }
            '(' if !quote => {
                paren += 1;
                cur.push(c);
            }
            ')' if !quote && paren > 0 => {
                paren -= 1;
                cur.push(c);
            }
            '<' if !quote && paren == 0 => {
                angle = true;
                cur.push(c);
            }
            '>' if !quote && paren == 0 => {
                angle = false;
                cur.push(c);
            }
            ',' | ';' if !quote && !angle && paren == 0 => parts.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts.iter().filter_map(|p| address(p)).collect()
}

fn address(part: &str) -> Option<String> {
    let addr = match (part.rfind('<'), part.rfind('>')) {
        (Some(a), Some(b)) if a < b => part[a + 1..b].trim().to_owned(),
        _ => {
            // no angle brackets: drop (comments) and "quotes", and a
            // group's "name:" lead
            let mut s = String::new();
            let mut depth = 0;
            for c in part.chars() {
                match c {
                    '(' => depth += 1,
                    ')' if depth > 0 => depth -= 1,
                    '"' => {}
                    _ if depth == 0 => s.push(c),
                    _ => {}
                }
            }
            s.rsplit(':').next().unwrap_or("").trim().to_owned()
        }
    };
    (addr.contains('@') && !addr.contains(char::is_whitespace)).then_some(addr)
}

/// One line of at most `max` characters, for a header or a table.
pub fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        flat
    } else {
        let cut: String = flat.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MSG: &[u8] = b"From: Ann <ann@x.example>\nTo: bob@y.example,\n carol@z.example\nSubject: Hi\n\nBody: not a header\n";

    #[test]
    fn finds_and_unfolds() {
        assert_eq!(header(MSG, "to").unwrap(), "bob@y.example, carol@z.example");
        assert_eq!(header(MSG, "Body"), None);
    }

    #[test]
    fn removes_only_what_is_asked() {
        let out = remove_headers(MSG, |n| n.eq_ignore_ascii_case("to"));
        assert_eq!(
            out,
            b"From: Ann <ann@x.example>\nSubject: Hi\n\nBody: not a header\n"
        );
    }

    #[test]
    fn set_replaces_and_keeps_line_endings() {
        let crlf = b"From: a@x\r\nSubject: s\r\n\r\nhi\r\n";
        let out = set_header(crlf, "subject", "new\nline");
        assert_eq!(out, b"subject: new line\r\nFrom: a@x\r\n\r\nhi\r\n");
        assert_eq!(header(&out, "Subject").unwrap(), "new line");
    }

    #[test]
    fn address_lists() {
        assert_eq!(
            addresses(
                r#""Doe, Jo" <jo@x.example>, bob@y.example (Bob, B), team: a@b.example, c@d.example;"#
            ),
            [
                "jo@x.example",
                "bob@y.example",
                "a@b.example",
                "c@d.example"
            ]
        );
        assert!(addresses("undisclosed-recipients:;").is_empty());
    }
}

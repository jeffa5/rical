//! Addresses as people type and mail carries them: "Ann Lee <ann@x>,
//! bob@y".

/// One mailbox: an address and, if there is one, a name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Mailbox {
    pub name: String,
    pub address: String,
}

impl Mailbox {
    pub fn new(name: &str, address: &str) -> Mailbox {
        Mailbox {
            name: name.trim().to_owned(),
            address: address.trim().to_owned(),
        }
    }

    /// "Name <addr>" or the bare address, for showing and typing.
    pub fn display(&self) -> String {
        if self.name.is_empty() {
            self.address.clone()
        } else if self.name.chars().any(needs_quotes) {
            format!(
                "\"{}\" <{}>",
                self.name.replace(['\\', '"'], ""),
                self.address
            )
        } else {
            format!("{} <{}>", self.name, self.address)
        }
    }

    /// What to show of a person: their name, else their address.
    pub fn shown(&self) -> &str {
        if self.name.is_empty() {
            &self.address
        } else {
            &self.name
        }
    }

    /// For a header: non-ASCII names as encoded words.
    pub fn encoded(&self) -> String {
        if self.name.is_empty() {
            self.address.clone()
        } else if self.name.is_ascii() {
            self.display()
        } else {
            format!("{} <{}>", encode_word(&self.name), self.address)
        }
    }
}

fn needs_quotes(c: char) -> bool {
    matches!(
        c,
        '(' | ')' | '<' | '>' | '[' | ']' | ':' | ';' | '@' | '\\' | ',' | '.' | '"'
    )
}

/// RFC 2047 "B" encoded words for `text`, split to fit a header line.
pub fn encode_word(text: &str) -> String {
    use base64::Engine as _;
    let mut words = Vec::new();
    let mut chunk = String::new();
    // 45 bytes of text make 60 of base64, with the 12 of "=?UTF-8?B??="
    for c in text.chars() {
        if chunk.len() + c.len_utf8() > 42 {
            words.push(std::mem::take(&mut chunk));
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(chunk);
    }
    words
        .iter()
        .map(|w| {
            format!(
                "=?UTF-8?B?{}?=",
                base64::engine::general_purpose::STANDARD.encode(w)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse an address list as typed or found in a header (already decoded).
/// Names may be quoted; groups are flattened; things without an `@` are
/// dropped.
pub fn parse_list(list: &str) -> Vec<Mailbox> {
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
            ',' | ';' | '\n' if !quote && !angle && paren == 0 => {
                parts.push(std::mem::take(&mut cur))
            }
            _ => cur.push(c),
        }
    }
    parts.push(cur);
    parts.iter().filter_map(|p| one(p)).collect()
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s);
    let mut out = String::new();
    let mut esc = false;
    for c in inner.chars() {
        if esc {
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    out.trim().to_owned()
}

fn one(part: &str) -> Option<Mailbox> {
    let part = part.trim();
    if let (Some(a), Some(b)) = (part.rfind('<'), part.rfind('>'))
        && a < b
    {
        let address = part[a + 1..b].trim();
        let name = unquote(part[..a].trim());
        return (address.contains('@') && !address.contains(char::is_whitespace))
            .then(|| Mailbox::new(&name, address));
    }
    // bare address, maybe with a (comment) and a group's "name:" lead
    let mut s = String::new();
    let mut depth = 0;
    for c in part.chars() {
        match c {
            '(' => depth += 1,
            ')' if depth > 0 => depth -= 1,
            _ if depth == 0 => s.push(c),
            _ => {}
        }
    }
    let address = s.rsplit(':').next().unwrap_or("").trim();
    (address.contains('@') && !address.contains(char::is_whitespace))
        .then(|| Mailbox::new("", address))
}

/// The list as one line for a header or the editor.
pub fn join(list: &[Mailbox]) -> String {
    list.iter()
        .map(Mailbox::display)
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn same(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_what_people_type() {
        let l = parse_list(
            r#""Lee, Ann" <ann@x.example>, bob@y.example (Bob), Cy <cy@z.example>; junk"#,
        );
        assert_eq!(
            l,
            [
                Mailbox::new("Lee, Ann", "ann@x.example"),
                Mailbox::new("", "bob@y.example"),
                Mailbox::new("Cy", "cy@z.example"),
            ]
        );
        assert_eq!(l[0].display(), "\"Lee, Ann\" <ann@x.example>");
        assert_eq!(l[2].display(), "Cy <cy@z.example>");
    }

    #[test]
    fn non_ascii_names_become_encoded_words() {
        let m = Mailbox::new("Zoë Müller", "zoe@x.example");
        let e = m.encoded();
        assert!(
            e.starts_with("=?UTF-8?B?") && e.ends_with("<zoe@x.example>"),
            "{e}"
        );
        assert!(e.is_ascii());
    }
}

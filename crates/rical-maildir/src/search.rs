//! Searching a folder: words, and `from:`, `to:`, `subject:`, `body:` and
//! the flags `unread`, `read`, `flagged`, `attachment`.

use std::fs;

use crate::store::Summary;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    pub body: Vec<String>,
    pub unread: bool,
    pub read: bool,
    pub flagged: bool,
    pub attachment: bool,
}

impl Query {
    pub fn parse(text: &str) -> Query {
        let mut q = Query::default();
        for tok in tokens(text) {
            let lower = tok.to_lowercase();
            match lower.as_str() {
                "unread" | "is:unread" => q.unread = true,
                "read" | "is:read" => q.read = true,
                "flagged" | "is:flagged" | "starred" => q.flagged = true,
                "attachment" | "has:attachment" => q.attachment = true,
                _ => match tok.split_once(':') {
                    Some((k, v)) if !v.is_empty() => {
                        let v = v.to_lowercase();
                        match k.to_lowercase().as_str() {
                            "from" => q.from.push(v),
                            "to" => q.to.push(v),
                            "subject" => q.subject.push(v),
                            "body" => q.body.push(v),
                            _ => q.words.push(lower),
                        }
                    }
                    _ => q.words.push(lower),
                },
            }
        }
        q
    }

    pub fn is_empty(&self) -> bool {
        *self == Query::default()
    }

    /// Whether the search needs to read the messages themselves.
    pub fn needs_body(&self) -> bool {
        !self.body.is_empty()
    }

    /// Whether a message matches on what the list knows of it.
    pub fn matches(&self, m: &Summary) -> bool {
        if self.unread && !m.unread() || self.read && m.unread() {
            return false;
        }
        if self.flagged && !m.flags.flagged || self.attachment && !m.attachments {
            return false;
        }
        let from = format!("{} {}", m.from_name, m.from_address).to_lowercase();
        let to = m.to.to_lowercase();
        let subject = m.subject.to_lowercase();
        self.from.iter().all(|w| from.contains(w))
            && self.to.iter().all(|w| to.contains(w))
            && self.subject.iter().all(|w| subject.contains(w))
            && self
                .words
                .iter()
                .all(|w| from.contains(w) || to.contains(w) || subject.contains(w))
    }

    /// Whether the message's text has all the `body:` words.
    pub fn matches_body(&self, m: &Summary) -> bool {
        if self.body.is_empty() {
            return true;
        }
        let Ok(bytes) = fs::read(&m.path) else {
            return false;
        };
        let Ok(o) = crate::read::parse(&m.path, &bytes) else {
            return false;
        };
        let text = o.body.text.to_lowercase();
        self.body.iter().all(|w| text.contains(w))
    }
}

/// Words, with "quoted phrases" kept together (the quotes dropped).
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote = false;
    for c in text.chars() {
        match c {
            '"' => quote = !quote,
            c if c.is_whitespace() && !quote => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::Flags;

    #[test]
    fn parses_and_matches() {
        let q = Query::parse(r#"from:ann subject:"q4 plan" unread budget"#);
        assert_eq!(q.from, ["ann"]);
        assert_eq!(q.subject, ["q4 plan"]);
        assert!(q.unread);
        let m = Summary {
            path: "p".into(),
            folder: String::new(),
            flags: Flags::default(),
            is_new: true,
            date: 0,
            from_name: "Ann Lee".into(),
            from_address: "ann@x.example".into(),
            to: "me".into(),
            subject: "Re: Q4 plan budget".into(),
            message_id: None,
            in_reply_to: vec![],
            references: vec![],
            size: 0,
            attachments: false,
        };
        assert!(q.matches(&m));
        assert!(!Query::parse("from:bob").matches(&m));
        assert!(!Query::parse("read").matches(&m));
        assert!(Query::parse("").is_empty());
    }
}

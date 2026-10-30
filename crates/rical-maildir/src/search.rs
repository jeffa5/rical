//! Searching: words, and `from:`, `to:`, `subject:`, `body:`, `in:` (a
//! folder), `date:` (a range) and the flags `unread`, `read`, `flagged`,
//! `attachment`. The same words work for mu's (`flag:unread`,
//! `maildir:/Sent`, `date:1w..`) so a query moves between them.

use std::fs;

use chrono::{Datelike as _, Local, NaiveDate, TimeZone as _};

use crate::store::Summary;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    pub words: Vec<String>,
    pub from: Vec<String>,
    pub to: Vec<String>,
    pub subject: Vec<String>,
    pub body: Vec<String>,
    /// The folder's name contains this.
    pub folder: Vec<String>,
    pub unread: bool,
    pub read: bool,
    pub flagged: bool,
    pub attachment: bool,
    /// Seconds since the epoch: from here (inclusive) …
    pub after: Option<i64>,
    /// … up to here (exclusive).
    pub before: Option<i64>,
}

/// A date as typed: `2026-10-30`, `2026-10`, `2026`, or how long ago:
/// `3d`, `2w`, `6m`, `1y`, `today`, `yesterday`.
pub fn parse_date(s: &str, now: chrono::DateTime<Local>) -> Option<i64> {
    let s = s.trim().to_lowercase();
    let midnight = |d: NaiveDate| {
        Local
            .from_local_datetime(&d.and_hms_opt(0, 0, 0)?)
            .earliest()
            .map(|t| t.timestamp())
    };
    let today = now.date_naive();
    match s.as_str() {
        "now" => return Some(now.timestamp()),
        "today" => return midnight(today),
        "yesterday" => return midnight(today - chrono::Duration::days(1)),
        _ => {}
    }
    if let Some(unit) = s.chars().last().filter(|c| c.is_ascii_alphabetic())
        && let Ok(n) = s[..s.len() - 1].parse::<i64>()
    {
        let back = match unit {
            'd' => chrono::Duration::days(n),
            'w' => chrono::Duration::weeks(n),
            'm' => chrono::Duration::days(30 * n),
            'y' => chrono::Duration::days(365 * n),
            'h' => chrono::Duration::hours(n),
            _ => return None,
        };
        return Some((now - back).timestamp());
    }
    let parts: Vec<&str> = s.split('-').collect();
    let num = |i: usize, default: u32| -> Option<u32> {
        parts.get(i).map_or(Some(default), |p| p.parse().ok())
    };
    let (y, m, d) = (num(0, 0)?, num(1, 1)?, num(2, 1)?);
    midnight(NaiveDate::from_ymd_opt(i32::try_from(y).ok()?, m, d)?)
}

/// The end of a range given as a date: a whole day (or month, or year)
/// includes its last moment, so the bound is the start of the next.
fn end_of(s: &str, now: chrono::DateTime<Local>) -> Option<i64> {
    let start = parse_date(s, now)?;
    let parts = s.trim().split('-').count();
    let is_absolute = s.trim().chars().next().is_some_and(|c| c.is_ascii_digit())
        && s.trim().chars().last().is_some_and(|c| c.is_ascii_digit());
    if !is_absolute {
        return Some(start);
    }
    let d = Local.timestamp_opt(start, 0).single()?.date_naive();
    let next = match parts {
        1 => NaiveDate::from_ymd_opt(d.year() + 1, 1, 1)?,
        2 => {
            let (y, m) = if d.month() == 12 {
                (d.year() + 1, 1)
            } else {
                (d.year(), d.month() + 1)
            };
            NaiveDate::from_ymd_opt(y, m, 1)?
        }
        _ => d + chrono::Duration::days(1),
    };
    Local
        .from_local_datetime(&next.and_hms_opt(0, 0, 0)?)
        .earliest()
        .map(|t| t.timestamp())
}

impl Query {
    pub fn parse(text: &str) -> Query {
        Query::parse_at(text, Local::now())
    }

    pub fn parse_at(text: &str, now: chrono::DateTime<Local>) -> Query {
        let mut q = Query::default();
        for tok in tokens(text) {
            let lower = tok.to_lowercase();
            match lower.as_str() {
                "unread" | "is:unread" | "flag:unread" | "flag:new" => q.unread = true,
                "read" | "is:read" | "flag:seen" => q.read = true,
                "flagged" | "is:flagged" | "starred" | "flag:flagged" => q.flagged = true,
                "attachment" | "has:attachment" | "flag:attach" => q.attachment = true,
                _ => match tok.split_once(':') {
                    Some((k, v)) if !v.is_empty() => {
                        let v = v.to_lowercase();
                        match k.to_lowercase().as_str() {
                            "from" => q.from.push(v),
                            "to" => q.to.push(v),
                            "subject" | "s" => q.subject.push(v),
                            "body" => q.body.push(v),
                            "in" | "folder" | "maildir" => {
                                q.folder.push(v.trim_start_matches('/').to_owned())
                            }
                            "date" | "d" => {
                                let (a, b) = v.split_once("..").unwrap_or((&v, &v));
                                if !a.is_empty() {
                                    q.after = parse_date(a, now);
                                }
                                if !b.is_empty() {
                                    q.before = end_of(b, now);
                                }
                            }
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
        if self.after.is_some_and(|a| m.date < a) || self.before.is_some_and(|b| m.date >= b) {
            return false;
        }
        let from = format!("{} {}", m.from_name, m.from_address).to_lowercase();
        let to = m.to.to_lowercase();
        let subject = m.subject.to_lowercase();
        let folder = m.folder.to_lowercase();
        self.from.iter().all(|w| from.contains(w))
            && self.to.iter().all(|w| to.contains(w))
            && self.subject.iter().all(|w| subject.contains(w))
            && self.folder.iter().all(|w| folder.contains(w))
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

// ---- the helper's form, written as a query for whichever search is used ----

/// Whose words a query is written in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Syntax {
    /// This app's own search.
    Builtin,
    Mu,
    Notmuch,
}

/// What the search helper asks for, as a form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Filters {
    pub words: String,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    /// A folder's name (`Sent`, `work/Projects`).
    pub folder: String,
    pub unread: bool,
    pub flagged: bool,
    pub attachment: bool,
    /// A date: `2026-10-01`, `2w`, `yesterday`…
    pub since: String,
    pub until: String,
}

fn quoted(v: &str) -> String {
    let v = v.trim();
    if v.contains(char::is_whitespace) {
        format!("\"{}\"", v.replace('"', ""))
    } else {
        v.to_owned()
    }
}

impl Filters {
    pub fn is_empty(&self) -> bool {
        *self == Filters::default()
    }

    /// The query, in `syntax`'s words.
    pub fn to_query(&self, syntax: Syntax) -> String {
        let mut parts: Vec<String> = Vec::new();
        let words = self.words.trim();
        if !words.is_empty() {
            parts.push(words.to_owned());
        }
        let field = |parts: &mut Vec<String>, name: &str, v: &str| {
            if !v.trim().is_empty() {
                parts.push(format!("{name}:{}", quoted(v)));
            }
        };
        field(&mut parts, "from", &self.from);
        field(&mut parts, "to", &self.to);
        field(&mut parts, "subject", &self.subject);
        match syntax {
            // mu and notmuch search the text with bare words
            Syntax::Mu | Syntax::Notmuch => {
                let b = self.body.trim();
                if !b.is_empty() {
                    parts.push(quoted(b));
                }
            }
            Syntax::Builtin => field(&mut parts, "body", &self.body),
        }
        let folder = self.folder.trim().trim_matches('/');
        if !folder.is_empty() {
            parts.push(match syntax {
                Syntax::Builtin => format!("in:{}", quoted(folder)),
                Syntax::Mu => format!("maildir:{}", quoted(&format!("/{folder}"))),
                Syntax::Notmuch => format!("folder:{}", quoted(folder)),
            });
        }
        for (on, builtin, mu, notmuch) in [
            (self.unread, "unread", "flag:unread", "tag:unread"),
            (self.flagged, "flagged", "flag:flagged", "tag:flagged"),
            (
                self.attachment,
                "attachment",
                "flag:attach",
                "tag:attachment",
            ),
        ] {
            if on {
                parts.push(
                    match syntax {
                        Syntax::Builtin => builtin,
                        Syntax::Mu => mu,
                        Syntax::Notmuch => notmuch,
                    }
                    .to_owned(),
                );
            }
        }
        let (since, until) = (self.since.trim(), self.until.trim());
        if !since.is_empty() || !until.is_empty() {
            parts.push(format!("date:{since}..{until}"));
        }
        parts.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::Flags;

    fn summary(date: i64, folder: &str) -> Summary {
        Summary {
            path: "p".into(),
            folder: folder.into(),
            flags: Flags::default(),
            is_new: true,
            date,
            from_name: "Ann Lee".into(),
            from_address: "ann@x.example".into(),
            to: "me".into(),
            subject: "Re: Q4 plan budget".into(),
            message_id: None,
            in_reply_to: vec![],
            references: vec![],
            size: 0,
            attachments: false,
            preview: String::new(),
        }
    }

    fn at(y: i32, m: u32, d: u32) -> chrono::DateTime<Local> {
        Local.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    #[test]
    fn parses_and_matches() {
        let q = Query::parse(r#"from:ann subject:"q4 plan" unread budget"#);
        assert_eq!(q.from, ["ann"]);
        assert_eq!(q.subject, ["q4 plan"]);
        assert!(q.unread);
        let m = summary(0, "INBOX");
        assert!(q.matches(&m));
        assert!(!Query::parse("from:bob").matches(&m));
        assert!(!Query::parse("read").matches(&m));
        assert!(Query::parse("").is_empty());
    }

    #[test]
    fn mu_words_work_too() {
        let q = Query::parse("flag:unread flag:attach maildir:/Sent date:2026-10-01..2026-10-31");
        assert!(q.unread && q.attachment);
        assert_eq!(q.folder, ["sent"]);
        assert!(q.after.is_some() && q.before.is_some());
    }

    #[test]
    fn dates_and_ranges() {
        let now = at(2026, 10, 30);
        let q = Query::parse_at("date:2026-10-01..2026-10-31", now);
        let day = |d| at(2026, 10, d).timestamp();
        assert!(q.after.unwrap() <= day(1) && q.before.unwrap() > day(31));
        let one = Query::parse_at("date:2026-10-15", now);
        assert!(
            one.after.unwrap() <= day(15)
                && one.before.unwrap() > day(15)
                && one.before.unwrap() <= day(16)
        );
        let recent = Query::parse_at("date:3d..", now);
        assert!(recent.before.is_none() && recent.after.unwrap() < day(28) + 86_400);
        let m = summary(day(20), "INBOX");
        assert!(
            Query::parse_at("date:..2026-10-15", now)
                .before
                .is_some_and(|b| m.date >= b)
        );
        assert!(!Query::parse_at("date:..2026-10-15", now).matches(&m));
        assert!(Query::parse_at("date:2026-10..2026-10", now).matches(&m));
        assert!(Query::parse_at("in:inbox", now).matches(&m));
        assert!(!Query::parse_at("in:sent", now).matches(&m));
    }

    #[test]
    fn the_form_writes_each_syntax() {
        let f = Filters {
            words: "budget".into(),
            from: "ann lee".into(),
            folder: "Sent".into(),
            unread: true,
            attachment: true,
            since: "2w".into(),
            body: "migration plan".into(),
            ..Filters::default()
        };
        assert_eq!(
            f.to_query(Syntax::Mu),
            "budget from:\"ann lee\" \"migration plan\" maildir:/Sent flag:unread flag:attach date:2w.."
        );
        assert_eq!(
            f.to_query(Syntax::Notmuch),
            "budget from:\"ann lee\" \"migration plan\" folder:Sent tag:unread tag:attachment date:2w.."
        );
        let own = f.to_query(Syntax::Builtin);
        assert!(own.contains("body:\"migration plan\"") && own.contains("in:Sent"));
        // what the form writes, the built-in search reads back
        let q = Query::parse(&own);
        assert!(q.unread && q.attachment && q.folder == ["sent"] && q.from == ["ann lee"]);
        assert!(Filters::default().is_empty());
    }
}

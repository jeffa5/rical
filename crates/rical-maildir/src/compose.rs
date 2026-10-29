//! Writing mail: replies and forwards from a message, plain text wrapped
//! and quoted the way plain-text mail expects, and the message itself
//! (headers encoded and folded, the body as UTF-8 text, attachments).

use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use chrono::{DateTime, Local, TimeZone as _};

use crate::{
    addr::{self, Mailbox},
    read::Opened,
    store::unique_name,
};

/// A message being written. Addresses are text, as typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub from: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<PathBuf>,
    /// The message this answers, to mark replied once it is sent.
    pub answering: Option<PathBuf>,
    /// The body is Markdown: sent as written with an HTML alternative.
    pub markdown: bool,
}

fn local(ts: i64) -> DateTime<Local> {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .unwrap_or_else(|| Local.timestamp_opt(0, 0).unwrap())
}

/// "Re: " once, however many times it was already there.
fn prefixed(prefix: &str, subject: &str) -> String {
    let s = subject.trim();
    let lower = s.to_lowercase();
    let want = prefix.to_lowercase();
    if lower.starts_with(&want)
        || (prefix == "Re:" && (lower.starts_with("aw:") || lower.starts_with("sv:")))
    {
        s.to_owned()
    } else {
        format!("{prefix} {s}")
    }
}

/// `text` quoted with "> " (and ">" on lines already quoted).
pub fn quote(text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.is_empty() {
                ">".to_owned()
            } else if l.starts_with('>') {
                format!(">{l}")
            } else {
                format!("> {l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The signature split off a text: (before, signature including "-- ").
pub fn split_signature(text: &str) -> (&str, &str) {
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_end_matches('\n') == "-- " {
            return (&text[..at], &text[at..]);
        }
        at += line.len();
    }
    (text, "")
}

fn same_person(a: &Mailbox, b: &Mailbox) -> bool {
    addr::same(&a.address, &b.address)
}

fn pick_from(o: &Opened, me: &[Mailbox]) -> String {
    // the address it was written to (To before Cc), else the first
    [&o.to, &o.cc]
        .iter()
        .find_map(|list| me.iter().find(|m| list.iter().any(|x| same_person(x, m))))
        .or(me.first())
        .map(Mailbox::display)
        .unwrap_or_default()
}

/// The reply to a message. `all` includes the others it was sent to.
pub fn reply(o: &Opened, me: &[Mailbox], all: bool, width: usize) -> Draft {
    let is_me = |m: &Mailbox| me.iter().any(|x| same_person(x, m));
    let sender: Vec<Mailbox> = if !o.reply_to.is_empty() {
        o.reply_to.clone()
    } else {
        o.from.clone()
    };
    // answering something I sent: write to the people I wrote to
    let mut to: Vec<Mailbox> = if o.from.iter().any(is_me) && !o.to.is_empty() {
        o.to.clone()
    } else {
        sender
    };
    let mut cc: Vec<Mailbox> = Vec::new();
    if all {
        for m in o.to.iter().chain(&o.cc) {
            if !to.iter().chain(&cc).any(|x| same_person(x, m)) {
                cc.push(m.clone());
            }
        }
    }
    cc.retain(|m| !is_me(m));
    // never write to myself, unless there is no one else
    let others: Vec<Mailbox> = to.iter().filter(|m| !is_me(m)).cloned().collect();
    if !others.is_empty() {
        to = others;
    }
    let who = o
        .from
        .first()
        .map_or("someone".to_owned(), |m| m.shown().to_owned());
    let when = o
        .date
        .map(|d| local(d).format("%a, %-d %b %Y at %H:%M").to_string());
    let attribution = match when {
        Some(w) => format!("On {w}, {who} wrote:"),
        None => format!("{who} wrote:"),
    };
    let (text, _) = split_signature(&o.body.text);
    let mut refs = o.references.clone();
    if let Some(id) = &o.message_id {
        refs.push(id.clone());
    }
    Draft {
        from: pick_from(o, me),
        to: addr::join(&to),
        cc: addr::join(&cc),
        bcc: String::new(),
        subject: prefixed("Re:", &o.subject),
        body: rewrap_overlong(
            &format!("{attribution}\n{}\n\n", quote(text.trim_end())),
            width,
        ),
        in_reply_to: o.message_id.clone(),
        references: refs,
        attachments: Vec::new(),
        answering: Some(o.path.clone()),
        markdown: false,
    }
}

/// The message forwarded in the body.
pub fn forward(o: &Opened, me: &[Mailbox]) -> Draft {
    let people = |l: &[Mailbox]| addr::join(l);
    let mut head = String::from("\n\n---------- Forwarded message ----------\n");
    head += &format!("From: {}\n", people(&o.from));
    if let Some(d) = o.date {
        head += &format!("Date: {}\n", local(d).format("%a, %-d %b %Y at %H:%M"));
    }
    head += &format!("Subject: {}\n", o.subject);
    if !o.to.is_empty() {
        head += &format!("To: {}\n", people(&o.to));
    }
    if !o.cc.is_empty() {
        head += &format!("Cc: {}\n", people(&o.cc));
    }
    head += "\n";
    head += o.body.text.trim_end();
    head.push('\n');
    Draft {
        from: pick_from(o, me),
        subject: prefixed("Fwd:", &o.subject),
        body: head,
        references: o
            .references
            .iter()
            .cloned()
            .chain(o.message_id.clone())
            .collect(),
        answering: Some(o.path.clone()),
        ..Draft::default()
    }
}

impl Draft {
    /// A draft to carry on with, from a message that was saved as one.
    pub fn from_saved(o: &Opened) -> Draft {
        Draft {
            from: addr::join(&o.from),
            to: addr::join(&o.to),
            cc: addr::join(&o.cc),
            bcc: addr::join(&o.bcc),
            subject: o.subject.clone(),
            body: o.body.text.clone(),
            in_reply_to: o.in_reply_to.first().cloned(),
            references: o.references.clone(),
            attachments: Vec::new(),
            answering: None,
            markdown: o
                .headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("X-Rical-Markdown")),
        }
    }
}

// ---- wrapping ----

fn prefix_of(line: &str) -> (String, &str) {
    // leading ">" marks (with optional spaces between), then the text
    let mut depth = 0;
    let mut rest = line;
    loop {
        let t = rest.trim_start_matches(' ');
        match t.strip_prefix('>') {
            Some(r) => {
                depth += 1;
                rest = r;
            }
            None => break,
        }
    }
    if depth == 0 {
        return (String::new(), line);
    }
    let text = rest.strip_prefix(' ').unwrap_or(rest);
    (format!("{} ", ">".repeat(depth)), text)
}

fn wrap_one(prefix: &str, indent: &str, text: &str, width: usize, out: &mut Vec<String>) {
    let cont = format!("{prefix}{indent}");
    let mut line = format!("{prefix}{indent}");
    let mut empty = true;
    let base = line.chars().count();
    let _ = base;
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        let have = line.chars().count();
        if !empty && have + 1 + word.chars().count() > width {
            out.push(
                std::mem::replace(&mut line, cont.clone())
                    .trim_end()
                    .to_owned(),
            );
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line.push_str(word);
        empty = false;
    }
    out.push(line.trim_end().to_owned());
}

/// Break lines longer than `width` at spaces, keeping their quote marks and
/// indentation. Lines with nothing to break at (a long link) stay whole.
pub fn wrap_long_lines(text: &str, width: usize) -> String {
    if width < 20 {
        return text.to_owned();
    }
    let mut out: Vec<String> = Vec::new();
    for line in text.split('\n') {
        if line.chars().count() <= width || line == "-- " {
            out.push(line.to_owned());
            continue;
        }
        let (prefix, rest) = prefix_of(line);
        let indent: String = rest.chars().take_while(|c| *c == ' ').collect();
        wrap_one(&prefix, &indent, rest.trim_start(), width, &mut out);
    }
    out.join("\n")
}

fn starts_item(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("- ")
        || t.starts_with("* ")
        || t.starts_with("• ")
        || t.split_once(". ").is_some_and(|(n, _)| {
            !n.is_empty() && n.len() < 4 && n.chars().all(|c| c.is_ascii_digit())
        })
}

/// Re-wrap paragraphs to `width`: lines that follow each other with the
/// same quote marks and indentation are joined and broken again. Blank
/// lines, list items, the signature and indented blocks stay as they are.
pub fn reflow(text: &str, width: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    // the paragraph being gathered: prefix, indent, words
    let mut para: Option<(String, String, String)> = None;
    let flush = |para: &mut Option<(String, String, String)>, out: &mut Vec<String>| {
        if let Some((prefix, indent, text)) = para.take() {
            wrap_one(&prefix, &indent, &text, width, out);
        }
    };
    let mut in_signature = false;
    for line in text.split('\n') {
        if line == "-- " {
            flush(&mut para, &mut out);
            in_signature = true;
        }
        if in_signature {
            out.push(line.to_owned());
            continue;
        }
        let (prefix, rest) = prefix_of(line);
        let indent: String = rest.chars().take_while(|c| *c == ' ').collect();
        let body = rest.trim_start();
        let code = indent.len() >= 4;
        if body.is_empty() || code {
            flush(&mut para, &mut out);
            out.push(line.trim_end().to_owned());
            continue;
        }
        let continues =
            matches!(&para, Some((p, i, _)) if *p == prefix && *i == indent) && !starts_item(body);
        if continues {
            let (_, _, t) = para.as_mut().unwrap();
            t.push(' ');
            t.push_str(body);
        } else {
            flush(&mut para, &mut out);
            // a list item continues on lines indented past its marker
            para = Some((prefix, indent, body.to_owned()));
        }
    }
    flush(&mut para, &mut out);
    out.join("\n")
}

/// Re-wrap just the paragraphs that have a line past `width`, as a quoted
/// line does once the quote marks are added, and leave the rest as written.
pub fn rewrap_overlong(text: &str, width: usize) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let belongs = |line: &str, prefix: &str| {
        let (p, rest) = prefix_of(line);
        let indent = rest.chars().take_while(|c| *c == ' ').count();
        p == prefix && !rest.trim().is_empty() && line != "-- " && indent < 4 && !starts_item(rest)
    };
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let (prefix, rest) = prefix_of(lines[i]);
        let indent = rest.chars().take_while(|c| *c == ' ').count();
        if rest.trim().is_empty() || lines[i] == "-- " || indent >= 4 {
            out.push(lines[i].to_owned());
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < lines.len() && belongs(lines[j], &prefix) {
            j += 1;
        }
        let group = &lines[i..j];
        if group
            .iter()
            .any(|l| l.chars().count() > width && !l.contains("://"))
        {
            out.push(reflow(&group.join("\n"), width));
        } else {
            out.extend(group.iter().map(|l| (*l).to_owned()));
        }
        i = j;
    }
    out.join("\n")
}

// ---- building the message ----

fn fold(name: &str, value: &str) -> String {
    let mut out = String::new();
    let mut line = format!("{name}:");
    for word in value.split(' ').filter(|w| !w.is_empty()) {
        if line.len() + 1 + word.len() > 76 && line.len() > name.len() + 1 {
            out.push_str(&line);
            out.push('\n');
            line = String::from(" ");
            line.push_str(word);
        } else {
            line.push(' ');
            line.push_str(word);
        }
    }
    out.push_str(&line);
    out.push('\n');
    out
}

fn addresses_header(name: &str, text: &str) -> Result<Option<String>> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let list = addr::parse_list(text);
    if list.is_empty() {
        bail!("{name}: no address in “{}”", text.trim());
    }
    // every piece of text that was typed has to have become an address
    let typed = text
        .split([',', ';'])
        .filter(|p| !p.trim().is_empty())
        .count();
    if typed > list.len() {
        bail!("{name}: some of “{}” is not an address", text.trim());
    }
    let encoded = list
        .iter()
        .map(Mailbox::encoded)
        .collect::<Vec<_>>()
        .join(", ");
    Ok(Some(fold(name, &encoded)))
}

fn qp(text: &str) -> String {
    let mut out = String::new();
    for line in text.split('\n') {
        let mut cur = String::new();
        let bytes = line.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            let last = i + 1 == bytes.len();
            let piece = if ((b == b' ' || b == b'\t') && last)
                || b == b'='
                || b >= 127
                || (b < 32 && b != b'\t')
            {
                format!("={b:02X}")
            } else {
                (b as char).to_string()
            };
            if cur.len() + piece.len() > 75 {
                out.push_str(&cur);
                out.push_str("=\n");
                cur.clear();
            }
            cur.push_str(&piece);
        }
        out.push_str(&cur);
        out.push('\n');
    }
    out.pop();
    out
}

fn mime_of(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase)
        .as_deref()
    {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("txt" | "md" | "log") => "text/plain",
        Some("html" | "htm") => "text/html",
        Some("csv") => "text/csv",
        Some("ics") => "text/calendar",
        Some("vcf") => "text/vcard",
        Some("zip") => "application/zip",
        Some("gz" | "tgz") => "application/gzip",
        Some("json") => "application/json",
        Some("odt") => "application/vnd.oasis.opendocument.text",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("mp3") => "audio/mpeg",
        Some("mp4") => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn filename_params(name: &str) -> String {
    let ascii: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && c != '"' && c != '\\' && !c.is_control() {
                c
            } else {
                '_'
            }
        })
        .collect();
    if name.is_ascii() {
        format!("filename=\"{ascii}\"")
    } else {
        let mut pct = String::new();
        for b in name.bytes() {
            if b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_') {
                pct.push(b as char);
            } else {
                pct.push_str(&format!("%{b:02X}"));
            }
        }
        format!("filename=\"{ascii}\"; filename*=UTF-8''{pct}")
    }
}

impl Draft {
    /// The message as it is sent and stored: headers, and the body as
    /// UTF-8 text, in a multipart with its attachments if it has any.
    /// It must say who it is from and to.
    pub fn to_bytes(&self, now: DateTime<Local>) -> Result<Vec<u8>> {
        self.build(now, true)
    }

    /// The same for keeping as a draft: nothing has to be filled in yet.
    pub fn to_draft_bytes(&self, now: DateTime<Local>) -> Result<Vec<u8>> {
        self.build(now, false)
    }

    fn build(&self, now: DateTime<Local>, strict: bool) -> Result<Vec<u8>> {
        let from = addr::parse_list(&self.from);
        let sender = match from.first() {
            Some(s) => s.clone(),
            None if strict => bail!("who is it from?"),
            None => Mailbox::new("", "draft@localhost"),
        };
        let mut head = String::new();
        head += &fold("Date", &now.to_rfc2822());
        if !from.is_empty() {
            head += &fold(
                "From",
                &from
                    .iter()
                    .map(Mailbox::encoded)
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
        let header = |name: &str, text: &str| -> Result<Option<String>> {
            if strict {
                addresses_header(name, text)
            } else {
                Ok(addresses_header(name, text).unwrap_or(None))
            }
        };
        let to = header("To", &self.to)?;
        let cc = header("Cc", &self.cc)?;
        let bcc = header("Bcc", &self.bcc)?;
        if strict && to.is_none() && cc.is_none() && bcc.is_none() {
            bail!("who is it to?");
        }
        for h in [to, cc, bcc].into_iter().flatten() {
            head += &h;
        }
        let subject = self.subject.trim();
        let subject = if subject.is_ascii() {
            subject.to_owned()
        } else {
            addr::encode_word(subject)
        };
        head += &fold("Subject", &subject);
        let domain = sender
            .address
            .rsplit_once('@')
            .map_or("localhost", |(_, d)| d);
        head += &format!("Message-ID: <{}@{domain}>\n", unique_name());
        if let Some(id) = &self.in_reply_to {
            head += &format!("In-Reply-To: <{}>\n", id.trim_matches(['<', '>']));
        }
        if !self.references.is_empty() {
            let refs = self
                .references
                .iter()
                .map(|r| format!("<{}>", r.trim_matches(['<', '>'])))
                .collect::<Vec<_>>()
                .join(" ");
            head += &fold("References", &refs);
        }
        if self.markdown {
            head += "X-Rical-Markdown: yes\n";
        }
        head += "MIME-Version: 1.0\n";

        let body = self.body.replace("\r\n", "\n");
        let body = body.trim_end_matches([' ', '\n', '\t']).to_owned() + "\n";
        let simple = body.is_ascii() && body.split('\n').all(|l| l.len() <= 900);
        let (cte, encoded) = if simple {
            ("7bit", body)
        } else {
            ("quoted-printable", qp(&body))
        };
        let plain_part = format!(
            "Content-Type: text/plain; charset=utf-8\nContent-Transfer-Encoding: {cte}\n\n{encoded}\n"
        );
        let text_part = if self.markdown {
            let alt = format!("rical-alt-{}", unique_name().replace(['.', ':'], "-"));
            let html = crate::markdown::to_html(&self.body.replace("\r\n", "\n"));
            format!(
                "Content-Type: multipart/alternative; boundary=\"{alt}\"\n\n--{alt}\n{plain_part}--{alt}\nContent-Type: text/html; charset=utf-8\nContent-Transfer-Encoding: quoted-printable\n\n{}\n--{alt}--\n",
                qp(&html)
            )
        } else {
            plain_part
        };

        let mut out = head;
        if self.attachments.is_empty() {
            out += &text_part;
            out.pop();
            return Ok(out.into_bytes());
        }
        let boundary = format!("rical-{}", unique_name().replace(['.', ':'], "-"));
        out += &format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\n\n");
        out += "This is a multi-part message in MIME format.\n";
        out += &format!("--{boundary}\n{text_part}");
        for path in &self.attachments {
            let data = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
            let name = path.file_name().map_or("attachment".to_owned(), |n| {
                n.to_string_lossy().into_owned()
            });
            out += &format!(
                "--{boundary}\nContent-Type: {}; name=\"{}\"\nContent-Transfer-Encoding: base64\nContent-Disposition: attachment; {}\n\n",
                mime_of(path),
                name.chars()
                    .map(|c| if c.is_ascii() && c != '"' { c } else { '_' })
                    .collect::<String>(),
                filename_params(&name)
            );
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
            for chunk in b64.as_bytes().chunks(76) {
                out += std::str::from_utf8(chunk).unwrap_or("");
                out.push('\n');
            }
        }
        out += &format!("--{boundary}--\n");
        Ok(out.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read;

    fn opened(raw: &str) -> Opened {
        read::parse(std::path::Path::new("/m/1"), raw.as_bytes()).unwrap()
    }

    fn me() -> Vec<Mailbox> {
        vec![
            Mailbox::new("Me", "me@y.example"),
            Mailbox::new("Me Work", "me@work.example"),
        ]
    }

    const MAIL: &str = "From: Ann Lee <ann@x.example>\nTo: me@work.example, Bo <bo@z.example>\nCc: cy@z.example, me@y.example\nSubject: Q4 plan\nDate: Thu, 5 Nov 2026 14:00:00 +0000\nMessage-ID: <m1@x>\nReferences: <r0@x>\n\nHello,\nlet's meet.\n\n-- \nAnn\n";

    #[test]
    fn replying_quotes_and_threads() {
        let d = reply(&opened(MAIL), &me(), false, 72);
        assert_eq!(d.to, "Ann Lee <ann@x.example>");
        assert_eq!(d.cc, "");
        assert_eq!(d.from, "Me Work <me@work.example>");
        assert_eq!(d.subject, "Re: Q4 plan");
        assert!(
            d.body.contains(" wrote:\n> Hello,\n> let's meet.\n\n"),
            "{}",
            d.body
        );
        assert!(!d.body.contains("Ann\n"), "signature is not quoted");
        assert_eq!(d.in_reply_to.as_deref(), Some("m1@x"));
        assert_eq!(d.references, ["r0@x", "m1@x"]);

        let all = reply(&opened(MAIL), &me(), true, 72);
        assert_eq!(all.to, "Ann Lee <ann@x.example>");
        assert_eq!(all.cc, "Bo <bo@z.example>, cy@z.example");
        assert_eq!(prefixed("Re:", "Re: x"), "Re: x");
    }

    #[test]
    fn wrapping_keeps_quotes_and_long_words() {
        let t = "> > a long quoted line that goes on and on and on and on and on and on\nhttps://example.com/a/very/long/link/that/does/not/break/at/all/anywhere/in/it/ok\nshort";
        let w = wrap_long_lines(t, 40);
        for l in w.lines().filter(|l| !l.starts_with("https")) {
            assert!(l.chars().count() <= 40, "{l:?}");
        }
        assert!(
            w.contains(">> a long quoted line that goes on and\n>> on and on"),
            "{w}"
        );
        assert!(w.contains(
            "\nhttps://example.com/a/very/long/link/that/does/not/break/at/all/anywhere/in/it/ok\n"
        ));
    }

    #[test]
    fn reflow_joins_paragraphs_but_not_lists_or_signature() {
        let t = "This is a paragraph\nwritten in short\nlines.\n\n- one item\n- another item\n\n> quoted\n> text here\n\n-- \nAnn\nCEO";
        let r = reflow(t, 30);
        assert_eq!(
            r,
            "This is a paragraph written in\nshort lines.\n\n- one item\n- another item\n\n> quoted text here\n\n-- \nAnn\nCEO"
        );
    }

    #[test]
    fn builds_a_message_that_reads_back() {
        let d = Draft {
            from: "Me <me@y.example>".into(),
            to: "Zoë <zoe@x.example>, bo@z.example".into(),
            subject: "Café ☕".into(),
            body: "Héllo wörld\n".into(),
            in_reply_to: Some("m1@x".into()),
            references: vec!["r0@x".into(), "m1@x".into()],
            ..Draft::default()
        };
        let bytes = d
            .to_bytes(Local.timestamp_opt(1793887200, 0).unwrap())
            .unwrap();
        assert!(bytes.is_ascii());
        let o = read::parse(std::path::Path::new("x"), &bytes).unwrap();
        assert_eq!(o.subject, "Café ☕");
        assert_eq!(o.to[0].name, "Zoë");
        assert_eq!(o.body.text, "Héllo wörld");
        assert_eq!(o.in_reply_to, ["m1@x"]);
        assert!(o.message_id.is_some());
    }

    #[test]
    fn quoting_rewraps_only_what_grew_too_long() {
        let q = quote(
            "Could we meet on Thursday to go through the Q4 plan? I've put the draft\nbudget here (rough).\n\nThanks,\nAnn",
        );
        let w = rewrap_overlong(&q, 72);
        assert_eq!(
            w,
            "> Could we meet on Thursday to go through the Q4 plan? I've put the\n> draft budget here (rough).\n>\n> Thanks,\n> Ann"
        );
        // short lines stay as they were
        assert_eq!(rewrap_overlong("> Thanks,\n> Ann", 72), "> Thanks,\n> Ann");
    }

    #[test]
    fn markdown_goes_out_as_text_and_html_with_the_quote_as_a_blockquote() {
        let d = Draft {
            from: "me@y.example".into(),
            to: "bo@z.example".into(),
            body: "Some **bold** news.\n\nOn Thu, Ann wrote:\n> Earlier\n".into(),
            markdown: true,
            ..Draft::default()
        };
        let bytes = d.to_bytes(Local::now()).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains("multipart/alternative") && text.contains("X-Rical-Markdown: yes"));
        let m = mail_parser::MessageParser::new().parse(&bytes).unwrap();
        let root = m.root();
        assert!(root.body_text(0).unwrap().contains("Some **bold** news."));
        let html = root.body_html(0).unwrap();
        assert!(
            html.contains("<strong>bold</strong>") && html.contains("<blockquote>"),
            "{html}"
        );
        let o = read::parse(std::path::Path::new("x"), &bytes).unwrap();
        assert!(Draft::from_saved(&o).markdown);
    }

    #[test]
    fn attachments_and_mistakes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a b.pdf");
        std::fs::write(&f, b"%PDF-data").unwrap();
        let d = Draft {
            from: "me@y.example".into(),
            to: "bo@z.example".into(),
            body: "See attached".into(),
            attachments: vec![f],
            ..Draft::default()
        };
        let bytes = d.to_bytes(Local::now()).unwrap();
        let o = read::parse(std::path::Path::new("x"), &bytes).unwrap();
        assert_eq!(o.attachments[0].name, "a b.pdf");
        assert_eq!(o.attachments[0].size, 9);
        assert_eq!(o.body.text, "See attached");

        let bad = Draft {
            from: "me@y.example".into(),
            to: "nobody".into(),
            ..Draft::default()
        };
        assert!(bad.to_bytes(Local::now()).is_err());
        let none = Draft {
            from: "me@y.example".into(),
            ..Draft::default()
        };
        assert!(none.to_bytes(Local::now()).is_err());
    }
}

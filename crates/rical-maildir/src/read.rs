//! Opening a message to read it. Private by default: a message is only
//! ever turned into text. Images, styles and anything else a message would
//! have fetched are counted and reported, never loaded.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use mail_parser::{MessageParser, MessagePart};

use crate::{
    addr::{self, Mailbox},
    html::{self, Link},
    invite::{self, Invite},
    text,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    Plain,
    /// Made from HTML.
    Html,
    Empty,
}

#[derive(Debug, Clone)]
pub struct Body {
    pub text: String,
    pub links: Vec<Link>,
    pub kind: BodyKind,
}

#[derive(Debug, Clone)]
pub struct Attachment {
    /// The part's number in the message.
    pub part: u32,
    pub name: String,
    pub mime: String,
    pub size: usize,
    pub calendar: bool,
    pub inline: bool,
}

/// What the message tried that this reader did not allow.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Privacy {
    pub remote_images: usize,
    /// Of those, ones too small to see: they report that you opened it.
    pub tracking_pixels: usize,
    /// Who asked to be told when it was read (never answered).
    pub read_receipt: Option<String>,
    /// Links whose words name one site and which go to another.
    pub misleading_links: usize,
}

impl Privacy {
    pub fn is_clean(&self) -> bool {
        self.remote_images == 0
            && self.tracking_pixels == 0
            && self.read_receipt.is_none()
            && self.misleading_links == 0
    }
}

#[derive(Debug, Clone)]
pub struct Opened {
    pub path: PathBuf,
    pub from: Vec<Mailbox>,
    pub reply_to: Vec<Mailbox>,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub bcc: Vec<Mailbox>,
    pub subject: String,
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub in_reply_to: Vec<String>,
    pub references: Vec<String>,
    pub list_id: Option<String>,
    /// Where List-Post says to write to the list.
    pub list_post: Option<String>,
    /// Where List-Unsubscribe says to go: `mailto:` or `https:` addresses.
    pub unsubscribe: Vec<String>,
    /// Every header, decoded, as it came.
    pub headers: Vec<(String, String)>,
    pub body: Body,
    pub attachments: Vec<Attachment>,
    pub invites: Vec<Invite>,
    pub privacy: Privacy,
    pub size: usize,
    /// It has an HTML part (that the HTML view can show).
    pub has_html: bool,
}

fn mailboxes(list: Option<mail_parser::AddressList<'_>>) -> Vec<Mailbox> {
    list.map(|l| {
        l.mailboxes()
            .filter_map(|m| {
                let a = m.address()?;
                Some(Mailbox::new(m.name().unwrap_or(""), a))
            })
            .collect()
    })
    .unwrap_or_default()
}

fn is_calendar(p: &MessagePart) -> bool {
    p.is_content_type("text", "calendar")
        || p.is_content_type("application", "ics")
        || p.attachment_name()
            .is_some_and(|n| n.to_lowercase().ends_with(".ics"))
}

fn flowed_text(p: &MessagePart) -> Option<String> {
    let t = p.text()?;
    let ct = p.content_type();
    let flowed = ct
        .as_ref()
        .and_then(|c| c.attribute("format"))
        .is_some_and(|f| f.eq_ignore_ascii_case("flowed"));
    let delsp = ct
        .as_ref()
        .and_then(|c| c.attribute("delsp"))
        .is_some_and(|f| f.eq_ignore_ascii_case("yes"));
    let t = t.replace("\r\n", "\n");
    Some(if flowed { text::unflow(&t, delsp) } else { t })
}

pub fn open(path: &Path) -> Result<Opened> {
    let bytes = std::fs::read(path).with_context(|| format!("{}", path.display()))?;
    parse(path, &bytes)
}

pub fn parse(path: &Path, bytes: &[u8]) -> Result<Opened> {
    let msg = MessageParser::new()
        .parse(bytes)
        .context("not a mail message")?;
    let root = msg.root();

    let headers: Vec<(String, String)> =
        root.headers()
            .iter()
            .map(|h| {
                let v =
                    h.value().as_text().map(str::to_owned).unwrap_or_else(|| {
                        String::from_utf8_lossy(h.raw_value()).trim().to_owned()
                    });
                (h.raw_name().to_owned(), v)
            })
            .collect();

    // the words: plain text if there is some, else the HTML as text
    let mut plain: Option<String> = None;
    let mut html_text: Option<html::Converted> = None;
    for p in root.text_body() {
        if p.is_content_type("text", "html") {
            if html_text.is_none()
                && let Some(h) = p.text()
            {
                html_text = Some(html::convert(&h));
            }
        } else if plain.is_none()
            && let Some(t) = flowed_text(&p)
        {
            plain = Some(t);
        }
    }
    // when the sender gave both, the HTML is only read for what it tried
    let mut stats = html::Converted::default();
    for p in root.html_body() {
        if let Some(h) = p.text() {
            let c = html::convert(&h);
            stats.remote_images += c.remote_images;
            stats.tracking_pixels += c.tracking_pixels;
            if html_text.is_none() {
                html_text = Some(c);
            } else if let Some(cur) = &mut html_text
                && cur.text == c.text
            {
                // same part seen twice: nothing to add
                let _ = cur;
            }
        }
    }
    let plain_trivial = plain.as_deref().is_some_and(|p| p.trim().len() < 24);
    let body = match (plain, html_text) {
        (Some(p), h) if !(plain_trivial && h.as_ref().is_some_and(|h| h.text.len() > 80)) => {
            let mut links = text::links(&p);
            text::check(&mut links);
            Body {
                text: p.trim_end().to_owned(),
                links,
                kind: BodyKind::Plain,
            }
        }
        (_, Some(h)) => Body {
            text: h.text,
            links: h.links,
            kind: BodyKind::Html,
        },
        (Some(p), None) => Body {
            text: p.trim_end().to_owned(),
            links: Vec::new(),
            kind: BodyKind::Plain,
        },
        (None, None) => Body {
            text: String::new(),
            links: Vec::new(),
            kind: BodyKind::Empty,
        },
    };

    let mut attachments = Vec::new();
    let mut invites = Vec::new();
    for p in msg.parts() {
        if p.is_multipart() || p.is_message() && p.nested().is_none() {
            continue;
        }
        if is_calendar(&p)
            && let Some(t) = p.text()
            && let Some(i) = invite::parse(&t)
        {
            invites.push(i);
        }
    }
    for p in root.attachments() {
        let ct = p.content_type();
        let mime = ct
            .as_ref()
            .map_or("application/octet-stream".to_owned(), |c| {
                format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")).to_lowercase()
            });
        let calendar = is_calendar(&p);
        attachments.push(Attachment {
            part: p.id(),
            name: p.attachment_name().map(str::to_owned).unwrap_or_else(|| {
                if calendar {
                    "invite.ics".to_owned()
                } else {
                    format!("attachment-{}", p.id())
                }
            }),
            mime,
            size: p.decoded_len(),
            calendar,
            inline: p.is_inline(),
        });
    }

    let receipt = [
        "Disposition-Notification-To",
        "Return-Receipt-To",
        "X-Confirm-Reading-To",
    ]
    .iter()
    .find_map(|n| headers.iter().find(|(h, _)| h.eq_ignore_ascii_case(n)))
    .map(|(_, v)| v.clone());
    let privacy = Privacy {
        remote_images: stats.remote_images,
        tracking_pixels: stats.tracking_pixels,
        read_receipt: receipt,
        misleading_links: body.links.iter().filter(|l| l.misleading).count(),
    };

    Ok(Opened {
        path: path.to_owned(),
        from: mailboxes(root.from()),
        reply_to: mailboxes(root.reply_to()),
        to: mailboxes(root.to()),
        cc: mailboxes(root.cc()),
        bcc: mailboxes(root.bcc()),
        subject: root.subject().unwrap_or_default().trim().to_owned(),
        date: root.date().map(|d| d.to_timestamp()),
        message_id: root.message_id().map(str::to_owned),
        in_reply_to: root
            .in_reply_to()
            .map(|l| l.iter().map(str::to_owned).collect())
            .unwrap_or_default(),
        references: root
            .references()
            .map(|l| l.iter().map(str::to_owned).collect())
            .unwrap_or_default(),
        unsubscribe: headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("List-Unsubscribe"))
            .map(|(_, v)| unsubscribe_targets(v))
            .unwrap_or_default(),
        list_post: headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("List-Post"))
            .and_then(|(_, v)| {
                unsubscribe_targets(v)
                    .into_iter()
                    .find(|u| u.to_lowercase().starts_with("mailto:"))
            })
            .map(|u| u[7..].split('?').next().unwrap_or("").to_owned()),
        list_id: root
            .list_id()
            .and_then(|l| l.first())
            .and_then(|m| m.address().map(str::to_owned)),
        headers,
        body,
        attachments,
        invites,
        privacy,
        size: bytes.len(),
        has_html: root.html_body().any(|p| p.is_content_type("text", "html")),
    })
}

/// The bytes of one attachment (or of an invitation part), by its number.
pub fn part_bytes(path: &Path, part: u32) -> Result<Vec<u8>> {
    let bytes = std::fs::read(path)?;
    let msg = MessageParser::new()
        .parse(&bytes)
        .context("not a mail message")?;
    let p = msg.part(part).context("no such part")?;
    Ok(p.decoded().into_owned())
}

/// The first calendar part of a message, as text.
pub fn calendar_text(path: &Path) -> Result<String> {
    calendar_text_of(&std::fs::read(path)?)
}

/// The same, from the message's bytes.
pub fn calendar_text_of(bytes: &[u8]) -> Result<String> {
    let msg = MessageParser::new()
        .parse(bytes)
        .context("not a mail message")?;
    for p in msg.parts() {
        if is_calendar(&p)
            && let Some(t) = p.text()
        {
            return Ok(t.into_owned());
        }
    }
    anyhow::bail!("no calendar in this message")
}

/// A filename safe to write: no folders, no odd characters.
pub fn safe_name(name: &str) -> String {
    let n: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let n = n.trim().trim_start_matches('.').to_owned();
    if n.is_empty() {
        "attachment".to_owned()
    } else {
        n
    }
}

#[allow(unused_imports)]
use addr as _;

#[cfg(test)]
mod tests {
    use super::*;

    fn open_text(raw: &str) -> Opened {
        parse(Path::new("x"), raw.as_bytes()).unwrap()
    }

    #[test]
    fn plain_message() {
        let o = open_text(
            "From: Ann Lee <ann@x.example>\nTo: me@y.example, \"Bo, B\" <bo@z.example>\nSubject: =?UTF-8?B?Q2Fmw6k=?=\nDate: Thu, 5 Nov 2026 14:00:00 +0000\nMessage-ID: <m1@x>\nDisposition-Notification-To: ann@x.example\n\nSee https://example.com/a.\n",
        );
        assert_eq!(o.subject, "Café");
        assert_eq!(o.from[0].display(), "Ann Lee <ann@x.example>");
        assert_eq!(o.to.len(), 2);
        assert_eq!(o.to[1].name, "Bo, B");
        assert_eq!(o.body.kind, BodyKind::Plain);
        assert_eq!(o.body.links[0].url, "https://example.com/a");
        assert_eq!(o.privacy.read_receipt.as_deref(), Some("ann@x.example"));
    }

    #[test]
    fn alternative_prefers_text_but_counts_what_the_html_tried() {
        let o = open_text(
            "From: a@x\nSubject: s\nMIME-Version: 1.0\nContent-Type: multipart/alternative; boundary=B\n\n--B\nContent-Type: text/plain; charset=utf-8\n\nThis is the plain version of a newsletter with enough words.\n--B\nContent-Type: text/html; charset=utf-8\n\n<p>Hello</p><img src=\"https://t.example/p.gif\" width=1 height=1><img src=\"https://t.example/a.png\">\n--B--\n",
        );
        assert_eq!(o.body.kind, BodyKind::Plain);
        assert!(o.body.text.contains("plain version"));
        assert_eq!((o.privacy.remote_images, o.privacy.tracking_pixels), (2, 1));
    }

    #[test]
    fn html_only_becomes_text_with_links() {
        let o = open_text(
            "From: a@x\nSubject: s\nContent-Type: text/html; charset=utf-8\n\n<p>Go to <a href=\"https://example.com/z\">example.com</a>!</p>\n",
        );
        assert_eq!(o.body.kind, BodyKind::Html);
        assert_eq!(o.body.text, "Go to example.com!");
        assert_eq!(o.body.links[0].url, "https://example.com/z");
    }

    #[test]
    fn attachments_and_invitations() {
        let o = open_text(
            "From: a@x\nSubject: s\nMIME-Version: 1.0\nContent-Type: multipart/mixed; boundary=B\n\n--B\nContent-Type: text/plain\n\nSee attached.\n--B\nContent-Type: text/calendar; method=REQUEST\nContent-Disposition: attachment; filename=invite.ics\n\nBEGIN:VCALENDAR\nMETHOD:REQUEST\nBEGIN:VEVENT\nSUMMARY:Lunch\nDTSTART:20261105T120000Z\nEND:VEVENT\nEND:VCALENDAR\n--B\nContent-Type: application/pdf; name=\"a b.pdf\"\nContent-Disposition: attachment; filename=\"a b.pdf\"\nContent-Transfer-Encoding: base64\n\nJVBERi0=\n--B--\n",
        );
        assert_eq!(o.invites[0].summary, "Lunch");
        let names: Vec<_> = o
            .attachments
            .iter()
            .map(|a| (a.name.as_str(), a.calendar))
            .collect();
        assert_eq!(names, [("invite.ics", true), ("a b.pdf", false)]);
        assert_eq!(o.attachments[1].size, 5);
        assert!(o.body.text.contains("See attached"));
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(safe_name(".bashrc"), "bashrc");
        assert_eq!(safe_name(""), "attachment");
    }
}

/// The `<…>` addresses in a List-Unsubscribe header, web and mail only.
pub fn unsubscribe_targets(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(a) = rest.find('<') {
        let Some(b) = rest[a..].find('>') else { break };
        let url = rest[a + 1..a + b].trim();
        let l = url.to_lowercase();
        if l.starts_with("mailto:") || l.starts_with("https://") || l.starts_with("http://") {
            out.push(url.to_owned());
        }
        rest = &rest[a + b + 1..];
    }
    out
}

#[cfg(test)]
mod unsubscribe_tests {
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(
            unsubscribe_targets(
                "<mailto:leave@x.example?subject=unsub>, <https://x.example/u?id=1>, <javascript:x>"
            ),
            [
                "mailto:leave@x.example?subject=unsub",
                "https://x.example/u?id=1"
            ]
        );
        assert!(unsubscribe_targets("garbage").is_empty());
    }
}

/// What the HTML view is allowed to load: nothing from anywhere, only what
/// the page itself holds (inline styles, and images embedded in the message).
pub const HTML_POLICY: &str = "default-src 'none'; img-src data:; style-src 'unsafe-inline' data:; font-src data:; media-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'";

/// The message's HTML, ready to show with nothing fetched: its own images
/// (`cid:`) put inline, and a Content-Security-Policy in front that blocks
/// any other load (remote images, styles, fonts, frames, prefetches).
/// None if it has no HTML.
pub fn html_document(path: &Path) -> Result<Option<String>> {
    use base64::Engine as _;
    let bytes = std::fs::read(path)?;
    let msg = MessageParser::new()
        .parse(&bytes)
        .context("not a mail message")?;
    let root = msg.root();
    let Some(mut html) = root.html_body().find_map(|p| {
        p.is_content_type("text", "html")
            .then(|| p.text().map(|t| t.into_owned()))
            .flatten()
    }) else {
        return Ok(None);
    };
    // the message's own images, by Content-ID, as data: URIs (up to 8 MB in all)
    let mut budget: usize = 8 * 1024 * 1024;
    for p in msg.parts() {
        let Some(id) = p.content_id() else { continue };
        let id = id.trim_matches(['<', '>']);
        let needle = format!("cid:{id}");
        if !html.contains(&needle) {
            continue;
        }
        let data = p.decoded();
        if data.len() > budget {
            continue;
        }
        budget -= data.len();
        let mime = p
            .content_type()
            .map_or("application/octet-stream".to_owned(), |c| {
                format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream"))
            });
        let uri = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&data)
        );
        html = html.replace(&needle, &uri);
    }
    Ok(Some(format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{HTML_POLICY}\">\n\
         <meta http-equiv=\"x-dns-prefetch-control\" content=\"off\">\n\
         <meta name=\"referrer\" content=\"no-referrer\">\n{html}"
    )))
}

#[cfg(test)]
mod html_tests {
    use super::*;

    #[test]
    fn the_document_blocks_loads_and_keeps_its_own_images() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("m");
        std::fs::write(
            &f,
            "From: a@x\nSubject: s\nMIME-Version: 1.0\nContent-Type: multipart/related; boundary=B\n\n--B\nContent-Type: text/html\n\n<p>Hi <img src=\"cid:logo@x\"> <img src=\"https://t.example/p.gif\"></p>\n--B\nContent-Type: image/png\nContent-ID: <logo@x>\nContent-Transfer-Encoding: base64\n\niVBORw0KGgo=\n--B--\n",
        )
        .unwrap();
        let doc = html_document(&f).unwrap().unwrap();
        assert!(doc.starts_with("<meta http-equiv=\"Content-Security-Policy\""));
        assert!(doc.contains("default-src 'none'"));
        assert!(
            doc.contains("src=\"data:image/png;base64,iVBORw0KGgo=\""),
            "{doc}"
        );
        // the remote one is still in the page, and the policy keeps it out
        assert!(doc.contains("https://t.example/p.gif"));
        let plain = dir.path().join("p");
        std::fs::write(&plain, "From: a@x\nSubject: s\n\nplain\n").unwrap();
        assert!(html_document(&plain).unwrap().is_none());
    }
}

//! Markdown for mail. The text is sent as written (Markdown reads well as
//! plain text) with an HTML alternative for readers that show it. The HTML
//! is plain: no images, no raw HTML passed through, and only web, mail and
//! phone links. A reply's quoted lines become block quotes.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};

use crate::compose::split_signature;

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn safe_link(url: &str) -> bool {
    let u = url.trim().to_lowercase();
    u.starts_with("http://")
        || u.starts_with("https://")
        || u.starts_with("mailto:")
        || u.starts_with("tel:")
}

/// The HTML for a Markdown message body.
pub fn to_html(text: &str) -> String {
    let (body, signature) = split_signature(text);
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS;
    // images become links (nothing is embedded or fetched); raw HTML is
    // text; links to anything but the web, mail and phones are text
    let mut drop_end = 0usize;
    let events = Parser::new_ext(body, options).filter_map(|e| match e {
        Event::Html(h) | Event::InlineHtml(h) => Some(Event::Text(h)),
        Event::Start(Tag::Image {
            dest_url,
            title,
            id,
            ..
        }) => Some(Event::Start(Tag::Link {
            link_type: pulldown_cmark::LinkType::Inline,
            dest_url,
            title,
            id,
        })),
        Event::End(TagEnd::Image) => Some(Event::End(TagEnd::Link)),
        Event::Start(Tag::Link { dest_url, .. }) if !safe_link(&dest_url) => {
            drop_end += 1;
            None
        }
        Event::End(TagEnd::Link) if drop_end > 0 => {
            drop_end -= 1;
            None
        }
        other => Some(other),
    });
    let mut out = String::from("<html><body>\n");
    html::push_html(&mut out, events);
    if !signature.trim().is_empty() {
        let lines: Vec<String> = signature.trim_end().lines().map(escape).collect();
        out.push_str(&format!("<p>{}</p>\n", lines.join("<br>\n")));
    }
    out.push_str("</body></html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_text_lists_quotes_and_the_signature() {
        let h = to_html(
            "Hello **there**, see [docs](https://example.com).\n\n- one\n- two\n\nOn Thu, Ann wrote:\n> Earlier\n>\n> > deeper\n\n-- \nAndrew\nhttps://jeffas.net",
        );
        assert!(h.contains("<strong>there</strong>"), "{h}");
        assert!(h.contains("<a href=\"https://example.com\">docs</a>"));
        assert!(h.contains("<li>one</li>"));
        assert!(h.contains("<blockquote>"));
        assert_eq!(h.matches("<blockquote>").count(), 2, "{h}");
        assert!(h.contains("-- <br>\nAndrew<br>\nhttps://jeffas.net"));
    }

    #[test]
    fn nothing_is_loaded_or_run() {
        let h = to_html(
            "![logo](https://t.example/p.png) <script>alert(1)</script> [x](javascript:alert(1)) <img src=\"https://t.example/i\">",
        );
        assert!(!h.contains("<img"), "{h}");
        assert!(!h.contains("<script>"), "{h}");
        assert!(!h.contains("javascript:"), "{h}");
        assert!(h.contains("&lt;script&gt;"));
        assert!(h.contains("href=\"https://t.example/p.png\""));
    }
}

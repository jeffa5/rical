//! HTML mail as text. Nothing is ever fetched or run: the HTML is read for
//! its words and its links, and what it would have loaded from elsewhere is
//! only counted, so the reader can say what was kept out.

use std::ops::Range;

use scraper::{ElementRef, Html};

/// A link in a body: where it goes, and the words that show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub url: String,
    /// What the message shows for it.
    pub text: String,
    /// Where those words are in the body text.
    pub range: Range<usize>,
    /// The words look like an address, and it is not where the link goes.
    pub misleading: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Converted {
    pub text: String,
    pub links: Vec<Link>,
    /// Images it would have fetched from the web.
    pub remote_images: usize,
    /// Of those, ones too small to be seen: there to say you opened it.
    pub tracking_pixels: usize,
}

const START: char = '\u{E000}';
const END: char = '\u{E001}';

struct Out {
    buf: String,
    /// At the start of a line (nothing but its quote prefix yet).
    bol: bool,
    /// A space is owed before the next word.
    space: bool,
    /// Blank lines owed before the next word, at most 1.
    blank: bool,
    quote: usize,
    /// The quote depth of the last line written.
    last_quote: usize,
    /// A link starts at the next word.
    link_start: bool,
    pre: bool,
    list: Vec<Option<usize>>,
    links: Vec<(String, bool)>,
    in_link: bool,
    images: usize,
    pixels: usize,
}

impl Out {
    fn newline(&mut self) {
        if !self.bol {
            self.buf.push('\n');
            self.bol = true;
        }
        self.space = false;
    }

    fn block(&mut self) {
        self.newline();
        self.blank = true;
    }

    fn word(&mut self, w: &str) {
        if w.is_empty() {
            return;
        }
        if self.bol {
            if self.blank && !self.buf.is_empty() {
                // a blank line, quoted as deep as both its neighbours are
                let depth = self.quote.min(self.last_quote);
                self.buf.push_str(&">".repeat(depth));
                self.buf.push('\n');
            }
            self.blank = false;
            if self.quote > 0 {
                self.buf.push_str(&">".repeat(self.quote));
                self.buf.push(' ');
            }
            self.last_quote = self.quote;
            self.bol = false;
        } else if self.space {
            self.buf.push(' ');
        }
        self.space = false;
        if self.link_start {
            self.buf.push(START);
            self.link_start = false;
        }
        self.buf.push_str(w);
    }

    /// Text, with its white space collapsed as a browser would.
    fn text(&mut self, t: &str) {
        if self.pre {
            for (i, line) in t.split('\n').enumerate() {
                if i > 0 {
                    self.newline();
                    // keep blank lines in pre
                    self.bol = true;
                }
                if !line.is_empty() {
                    self.word(&line.replace('\t', "    "));
                }
            }
            return;
        }
        let mut first = true;
        for w in t.split(|c: char| c.is_whitespace() || c == '\u{a0}') {
            if w.is_empty() {
                // white space: a space is owed (if anything has been said)
                if first && !self.bol {
                    self.space = true;
                }
                first = false;
                continue;
            }
            if first && t.starts_with(|c: char| c.is_whitespace() || c == '\u{a0}') && !self.bol {
                self.space = true;
            }
            first = false;
            self.word(w);
            self.space = true;
        }
        if !t.ends_with(|c: char| c.is_whitespace() || c == '\u{a0}') {
            self.space = false;
        }
    }
}

fn hidden(el: &ElementRef) -> bool {
    let v = el.value();
    if v.attr("hidden").is_some() || v.attr("aria-hidden") == Some("true") {
        return true;
    }
    let Some(style) = v.attr("style") else {
        return false;
    };
    let s: String = style
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase();
    s.contains("display:none")
        || s.contains("visibility:hidden")
        || s.contains("font-size:0")
        || s.contains("max-height:0")
        || s.contains("mso-hide:all")
        || s.contains("opacity:0;")
        || s.ends_with("opacity:0")
}

fn dimension(v: Option<&str>) -> Option<u32> {
    let v = v?.trim().trim_end_matches("px");
    v.parse().ok()
}

fn is_remote(src: &str) -> bool {
    let s = src.trim().to_lowercase();
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("//")
}

fn walk(el: ElementRef, o: &mut Out) {
    let name = el.value().name();
    if matches!(
        name,
        "script"
            | "style"
            | "head"
            | "title"
            | "template"
            | "iframe"
            | "object"
            | "embed"
            | "svg"
            | "noscript"
            | "meta"
            | "link"
            | "base"
            | "form"
            | "input"
            | "button"
            | "select"
            | "textarea"
    ) && name != "form"
    {
        return;
    }
    if hidden(&el) {
        return;
    }
    match name {
        "br" => {
            o.newline();
            o.bol = true;
            // br is a line break, not an end of paragraph; the next word
            // starts a fresh line without a blank one
            return;
        }
        "hr" => {
            o.block();
            o.word("────────────────");
            o.block();
            return;
        }
        "img" => {
            let v = el.value();
            let remote = v.attr("src").is_some_and(is_remote);
            if remote {
                o.images += 1;
                let tiny = dimension(v.attr("width")).is_some_and(|w| w <= 2)
                    || dimension(v.attr("height")).is_some_and(|h| h <= 2);
                if tiny {
                    o.pixels += 1;
                    return;
                }
            }
            match v.attr("alt").map(str::trim).filter(|a| !a.is_empty()) {
                Some(alt) => o.text(&format!("[{alt}]")),
                None if o.in_link => o.text("[image]"),
                None => {}
            }
            return;
        }
        _ => {}
    }
    let block = matches!(
        name,
        "p" | "div"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "nav"
            | "aside"
            | "main"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "ul"
            | "ol"
            | "table"
            | "blockquote"
            | "pre"
            | "address"
            | "figure"
            | "dl"
            | "center"
            | "form"
    );
    let line = matches!(
        name,
        "li" | "tr" | "dt" | "dd" | "caption" | "thead" | "tbody" | "tfoot"
    );
    if block {
        o.block();
    } else if line {
        o.newline();
    }
    match name {
        "blockquote" => o.quote += 1,
        "pre" => o.pre = true,
        "ul" => o.list.push(None),
        "ol" => o.list.push(Some(0)),
        "li" => {
            let marker = match o.list.last_mut() {
                Some(Some(n)) => {
                    *n += 1;
                    format!("{n}.")
                }
                _ => "•".to_owned(),
            };
            o.word(&marker);
            o.space = true;
        }
        _ => {}
    }
    let link = (name == "a" && !o.in_link)
        .then(|| el.value().attr("href"))
        .flatten()
        .map(str::trim)
        .filter(|h| {
            let l = h.to_lowercase();
            l.starts_with("http://")
                || l.starts_with("https://")
                || l.starts_with("mailto:")
                || l.starts_with("tel:")
        })
        .map(str::to_owned);
    if let Some(url) = &link {
        if o.bol {
            // make sure the marker is not before the quote prefix
            o.word("");
        }
        o.links.push((url.clone(), false));
        o.in_link = true;
        // the marker goes after any pending space, before the words
        o.link_start = true;
    }
    let heading = matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
    let mut cells = 0;
    for child in el.children() {
        if let Some(t) = child.value().as_text() {
            o.text(t);
        } else if let Some(c) = ElementRef::wrap(child) {
            if matches!(c.value().name(), "td" | "th") {
                if cells > 0 {
                    o.text("  ");
                    o.word("│");
                    o.space = true;
                }
                cells += 1;
            }
            walk(c, o);
        }
    }
    if link.is_some() {
        if o.link_start {
            o.buf.push(START);
            o.link_start = false;
        }
        o.buf.push(END);
        o.in_link = false;
    }
    match name {
        "blockquote" => o.quote -= 1,
        "pre" => o.pre = false,
        "ul" | "ol" => {
            o.list.pop();
        }
        _ => {}
    }
    if block || heading {
        o.block();
    } else if line {
        o.newline();
    }
}

fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let h = rest.split(['/', '?', '#']).next()?;
    let h = h.rsplit('@').next()?;
    Some(
        h.split(':')
            .next()?
            .trim_start_matches("www.")
            .to_lowercase(),
    )
}

/// Whether the words shown for a link read as an address that is not
/// the one it goes to.
pub fn misleading(text: &str, url: &str) -> bool {
    let t = text.trim();
    let looks_like_url = t.starts_with("http://")
        || t.starts_with("https://")
        || t.starts_with("www.")
        || (t.contains('.')
            && !t.contains(char::is_whitespace)
            && t.split('.').next_back().is_some_and(|tld| {
                tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic() || c == '/')
            })
            && !t.contains('@'));
    if !looks_like_url {
        return false;
    }
    let shown = host(&if t.contains("://") {
        t.to_owned()
    } else {
        format!("https://{t}")
    });
    let real = host(url);
    match (shown, real) {
        (Some(s), Some(r)) => {
            let same = |a: &str, b: &str| {
                a == b || a.ends_with(&format!(".{b}")) || b.ends_with(&format!(".{a}"))
            };
            !same(&s, &r)
        }
        _ => false,
    }
}

/// Turn HTML into text with its links and what it would have loaded.
pub fn convert(html: &str) -> Converted {
    let doc = Html::parse_document(html);
    let mut o = Out {
        buf: String::new(),
        bol: true,
        space: false,
        blank: false,
        quote: 0,
        last_quote: 0,
        link_start: false,
        pre: false,
        list: Vec::new(),
        links: Vec::new(),
        in_link: false,
        images: 0,
        pixels: 0,
    };
    walk(doc.root_element(), &mut o);

    // the invisible: zero-width padding that fills a preview line
    let raw: String = o
        .buf
        .chars()
        .filter(|c| {
            !matches!(
                c,
                '\u{200b}'
                    | '\u{200c}'
                    | '\u{200d}'
                    | '\u{2060}'
                    | '\u{feff}'
                    | '\u{ad}'
                    | '\u{34f}'
            )
        })
        .collect();

    // trim line ends, squeeze blank lines, and place the link markers
    let mut text = String::new();
    let mut links: Vec<Link> = Vec::new();
    let mut open: Option<usize> = None;
    let mut blanks = 0;
    for line in raw.split('\n') {
        let line = line.trim_end();
        if line.trim_matches([START, END]).trim().is_empty()
            && !line.contains(START)
            && !line.contains(END)
        {
            blanks += 1;
            continue;
        }
        if !text.is_empty() {
            text.push('\n');
            if blanks > 0 {
                text.push('\n');
            }
        }
        blanks = 0;
        for c in line.chars() {
            match c {
                START => {
                    open = Some(links.len());
                    let (url, _) = o.links.get(links.len()).cloned().unwrap_or_default();
                    links.push(Link {
                        url,
                        text: String::new(),
                        range: text.len()..text.len(),
                        misleading: false,
                    });
                }
                END => {
                    if let Some(i) = open.take() {
                        let l = &mut links[i];
                        l.range.end = text.len();
                        l.text = text[l.range.clone()].trim().to_owned();
                    }
                }
                c => text.push(c),
            }
        }
        // a link that carried over a line break ends where the line does
        if let Some(i) = open {
            let l = &mut links[i];
            if l.range.end < text.len() {
                l.range.end = text.len();
            }
        }
    }
    // links with no words get their address as the words
    let mut fixed = String::new();
    let mut cursor = 0usize;
    let mut out_links = Vec::new();
    for mut l in links {
        let (start, end) = (l.range.start, l.range.end);
        if text[start..end].trim().is_empty() {
            fixed.push_str(&text[cursor..start]);
            let shown = l.url.trim_start_matches("mailto:").to_owned();
            let at = fixed.len();
            fixed.push_str(&shown);
            l.text = shown.clone();
            l.range = at..fixed.len();
            cursor = end;
        } else {
            fixed.push_str(&text[cursor..end]);
            cursor = end;
            let new_start = fixed.len() - (end - start);
            l.range = new_start..fixed.len();
        }
        l.misleading = misleading(&l.text, &l.url);
        out_links.push(l);
    }
    fixed.push_str(&text[cursor..]);
    Converted {
        text: fixed.trim().to_owned(),
        links: out_links,
        remote_images: o.images,
        tracking_pixels: o.pixels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_lists_and_whitespace() {
        let c = convert(
            "<html><head><title>x</title><style>p{}</style></head><body>\
             <p>Hello   <b>there</b>,\n world</p><p>Second</p>\
             <ul><li>one</li><li>two</li></ul><ol><li>a</li><li>b</li></ol>\
             <div>line<br>break</div></body></html>",
        );
        assert_eq!(
            c.text,
            "Hello there, world\n\nSecond\n\n• one\n• two\n\n1. a\n2. b\n\nline\nbreak"
        );
    }

    #[test]
    fn links_are_found_with_their_words_and_lies_flagged() {
        let c = convert(
            r#"<p>See <a href="https://example.com/a">the docs</a> or
               <a href="https://evil.example/x">https://bank.example/login</a>
               or <a href="https://t.example/p"><img src="https://t.example/i.png"></a>.</p>"#,
        );
        assert_eq!(c.links.len(), 3);
        assert_eq!(&c.text[c.links[0].range.clone()], "the docs");
        assert_eq!(c.links[0].url, "https://example.com/a");
        assert!(!c.links[0].misleading);
        assert!(c.links[1].misleading);
        assert_eq!(
            &c.text[c.links[1].range.clone()],
            "https://bank.example/login"
        );
        assert_eq!(&c.text[c.links[2].range.clone()], "[image]");
        assert_eq!(c.remote_images, 1);
    }

    #[test]
    fn tracking_pixels_and_hidden_text_are_dropped_and_counted() {
        let c = convert(
            r#"<div style="display:none">preheader&zwnj;&zwnj;</div><p>Hi</p>
               <img src="https://t.example/p.gif" width="1" height="1">
               <img src="https://t.example/logo.png" alt="Logo" width="100">
               <img src="cid:abc">"#,
        );
        assert_eq!(c.text, "Hi\n\n[Logo]");
        assert_eq!((c.remote_images, c.tracking_pixels), (2, 1));
    }

    #[test]
    fn quotes_get_arrows() {
        let c = convert(
            "<p>Ann wrote:</p><blockquote><p>Earlier</p><p>More</p></blockquote><p>Reply</p>",
        );
        assert_eq!(c.text, "Ann wrote:\n\n> Earlier\n>\n> More\n\nReply");
    }

    #[test]
    fn tables_read_across() {
        let c = convert(
            "<table><tr><td>Name</td><td>Qty</td></tr><tr><td>Tea</td><td>2</td></tr></table>",
        );
        assert_eq!(c.text, "Name │ Qty\nTea │ 2");
    }
}

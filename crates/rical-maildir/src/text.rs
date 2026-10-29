//! Plain text mail: format=flowed (RFC 3676) put back into paragraphs,
//! and the links in it.

use std::ops::Range;

use crate::html::{Link, misleading};

/// Text sent as `format=flowed`: a line ending in a space continues on the
/// next, at the same quote depth. `delsp` says that space is not part of
/// the text.
pub fn unflow(text: &str, delsp: bool) -> String {
    let mut out: Vec<String> = Vec::new();
    // the line being built, and its quote depth, while it is continued
    let mut cur: Option<(usize, String)> = None;
    for raw in text.lines() {
        let mut depth = 0;
        let mut rest = raw;
        while let Some(r) = rest.strip_prefix('>') {
            depth += 1;
            rest = r;
        }
        // space-stuffing
        let body = rest.strip_prefix(' ').unwrap_or(rest);
        let signature = body == "-- ";
        let flowed = body.ends_with(' ') && !signature;
        let piece = if flowed && delsp {
            &body[..body.len() - 1]
        } else {
            body
        };
        match cur.take() {
            Some((d, mut line)) if d == depth => {
                line.push_str(piece);
                if flowed {
                    cur = Some((d, line));
                } else {
                    out.push(quoted(d, &line));
                }
            }
            prev => {
                if let Some((d, line)) = prev {
                    out.push(quoted(d, &line));
                }
                if flowed {
                    cur = Some((depth, piece.to_owned()));
                } else {
                    out.push(quoted(depth, piece));
                }
            }
        }
    }
    if let Some((d, line)) = cur {
        out.push(quoted(d, &line));
    }
    out.join("\n")
}

fn quoted(depth: usize, line: &str) -> String {
    if depth == 0 {
        line.to_owned()
    } else if line.is_empty() {
        ">".repeat(depth)
    } else {
        format!("{} {line}", ">".repeat(depth))
    }
}

/// The web addresses in plain text, with where they are.
pub fn links(text: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let starts = ["https://", "http://", "mailto:"].iter().any(|p| {
            rest.len() >= p.len() && rest.as_bytes()[..p.len()].eq_ignore_ascii_case(p.as_bytes())
        });
        let boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if starts && boundary {
            let mut end = i;
            for (j, c) in rest.char_indices() {
                if c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\u{a0}') {
                    break;
                }
                end = i + j + c.len_utf8();
            }
            // trailing punctuation belongs to the sentence
            let mut e = end;
            while e > i
                && matches!(
                    text[..e].chars().last(),
                    Some('.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '\'')
                )
            {
                // a closing bracket that has its opener inside stays
                let last = text[..e].chars().last().unwrap();
                if last == ')' && text[i..e].contains('(') {
                    break;
                }
                e -= last.len_utf8();
            }
            if e > i + 7 {
                let url = text[i..e].to_owned();
                out.push(Link {
                    text: url.clone(),
                    url,
                    range: i..e,
                    misleading: false,
                });
                i = e;
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
    out
}

/// Whether a link's words and address disagree.
pub fn check(links: &mut [Link]) {
    for l in links {
        l.misleading = misleading(&l.text, &l.url);
    }
}

/// The line range of the lines of `text` that are quoted (start with `>`).
pub fn quoted_ranges(text: &str) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.starts_with('>') {
            match out.last_mut() {
                Some(r) if r.end == at => r.end = at + line.len(),
                _ => out.push(at..at + line.len()),
            }
        }
        at += line.len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flowed_lines_rejoin() {
        let t = "This is a long \nparagraph that was \nwrapped.\nNext.\n> quoted and \n> wrapped\n>\n-- \nSig ";
        assert_eq!(
            unflow(t, false),
            "This is a long paragraph that was wrapped.\nNext.\n> quoted and wrapped\n>\n-- \nSig ",
        );
        assert_eq!(unflow("ab \ncd", true), "abcd");
    }

    #[test]
    fn finds_links_without_the_sentence_around_them() {
        let t =
            "See https://example.com/a?b=1, or (https://example.org/x) and http://x.example/(y).";
        let l = links(t);
        let urls: Vec<&str> = l.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://example.com/a?b=1",
                "https://example.org/x",
                "http://x.example/(y)"
            ]
        );
        assert_eq!(&t[l[0].range.clone()], "https://example.com/a?b=1");
    }
}

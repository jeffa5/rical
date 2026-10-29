//! Plain-text editing rules for writing mail, as functions of text so they
//! can be tested: what Enter continues, where a line breaks as it is typed.

/// What a line starts with: quote marks, indentation, a list marker.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Lead {
    /// ">> " or nothing.
    pub quote: String,
    pub indent: String,
    /// "- ", "* ", "• " or "3. ".
    pub marker: Option<String>,
    /// What follows.
    pub rest: String,
}

pub fn lead(line: &str) -> Lead {
    // the run of > and spaces that makes the quote marks
    let mut depth = 0;
    let mut end = 0;
    for (i, c) in line.char_indices() {
        match c {
            '>' => {
                depth += 1;
                end = i + 1;
            }
            ' ' if depth > 0 => {}
            _ => break,
        }
    }
    let (quote, after) = if depth > 0 {
        let rest = &line[end..];
        (
            format!("{} ", ">".repeat(depth)),
            rest.strip_prefix(' ').unwrap_or(rest),
        )
    } else {
        (String::new(), line)
    };
    let indent: String = after.chars().take_while(|c| *c == ' ').collect();
    let body = &after[indent.len()..];
    let marker = ["- ", "* ", "• "]
        .iter()
        .find(|m| body.starts_with(**m))
        .map(|m| (*m).to_owned())
        .or_else(|| {
            let digits: String = body.chars().take_while(char::is_ascii_digit).collect();
            let tail = &body[digits.len()..];
            (!digits.is_empty()
                && digits.len() < 4
                && (tail.starts_with(". ") || tail.starts_with(") ")))
            .then(|| format!("{digits}{}", &tail[..2]))
        });
    let rest = match &marker {
        Some(m) => body[m.len()..].to_owned(),
        None => body.to_owned(),
    };
    Lead {
        quote,
        indent,
        marker,
        rest,
    }
}

/// What a break in the middle of this line starts the next with: its quote
/// marks and indentation, and room where a list marker was.
pub fn wrap_prefix(line: &str) -> String {
    let l = lead(line);
    let pad = l.marker.as_ref().map_or(0, |m| m.chars().count());
    format!("{}{}{}", l.quote, l.indent, " ".repeat(pad))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Enter {
    /// A plain new line.
    Plain,
    /// A new line starting with this.
    Continue(String),
    /// The line was only a marker or quote marks: empty it (this many
    /// characters go), ending the list or the quote.
    Clear(usize),
}

/// What Enter does after `before`, the line up to the cursor.
pub fn enter(before: &str) -> Enter {
    let l = lead(before);
    let nothing_said = l.rest.trim().is_empty();
    if nothing_said && (l.marker.is_some() || !l.quote.is_empty()) {
        return Enter::Clear(before.chars().count());
    }
    let next_marker =
        l.marker.as_ref().map(
            |m| match m.trim_end_matches([' ', '.', ')']).parse::<u32>() {
                Ok(n) => format!("{}{}", n + 1, &m[m.len() - 2..]),
                Err(_) => m.clone(),
            },
        );
    let prefix = format!("{}{}{}", l.quote, l.indent, next_marker.unwrap_or_default());
    if prefix.is_empty() {
        Enter::Plain
    } else {
        Enter::Continue(prefix)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Wrap {
    No,
    /// Typing a space at the margin: end the line instead.
    Newline,
    /// Break at the space at this character index, and carry on below.
    BreakAt(usize),
}

/// Whether typing `c` after `before` (the line so far) crosses `width`,
/// and where to break if so. A word with `://` in it (a link) is not split.
pub fn wrap(before: &str, c: char, width: usize) -> Wrap {
    let col = before.chars().count();
    if col < width || c == '\n' {
        return Wrap::No;
    }
    if c == ' ' {
        return if before.ends_with(' ') || before.trim().is_empty() {
            Wrap::No
        } else {
            Wrap::Newline
        };
    }
    let chars: Vec<char> = before.chars().collect();
    let start = wrap_prefix(before).chars().count().min(chars.len());
    let Some(sp) = (start..chars.len()).rev().find(|&i| chars[i] == ' ') else {
        return Wrap::No;
    };
    // nothing before the break but the prefix
    if chars[start..sp].iter().all(|c| *c == ' ') {
        return Wrap::No;
    }
    let word: String = chars[sp + 1..].iter().collect();
    if word.contains("://") {
        return Wrap::No;
    }
    Wrap::BreakAt(sp)
}

/// The words, lines and longest line of a text, and how many lines are
/// past `width` (links and quoted lines not counted).
pub fn measure(text: &str, width: usize) -> (usize, usize, usize, usize) {
    let words = text.split_whitespace().count();
    let lines = text.lines().count();
    let longest = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    let over = text
        .lines()
        .filter(|l| l.chars().count() > width && !l.contains("://") && !l.starts_with('>'))
        .count();
    (words, lines, longest, over)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leads() {
        let l = lead("> > - item text");
        assert_eq!(
            (l.quote.as_str(), l.marker.as_deref(), l.rest.as_str()),
            (">> ", Some("- "), "item text")
        );
        let l = lead("  3. third");
        assert_eq!(
            (l.indent.as_str(), l.marker.as_deref(), l.rest.as_str()),
            ("  ", Some("3. "), "third")
        );
        assert_eq!(lead("plain").marker, None);
        assert_eq!(lead("2024 was good").marker, None);
    }

    #[test]
    fn enter_continues_lists_and_quotes_and_ends_them() {
        assert_eq!(enter("hello"), Enter::Plain);
        assert_eq!(enter("> quoted"), Enter::Continue("> ".into()));
        assert_eq!(enter("  - item"), Enter::Continue("  - ".into()));
        assert_eq!(enter("9. nine"), Enter::Continue("10. ".into()));
        assert_eq!(enter("- "), Enter::Clear(2));
        assert_eq!(enter(">> "), Enter::Clear(3));
        assert_eq!(enter("    code"), Enter::Continue("    ".into()));
    }

    #[test]
    fn wrapping_as_you_type() {
        let line = "this line is exactly at the limit of the width";
        assert_eq!(wrap(line, 'x', 100), Wrap::No);
        let at = line.chars().count();
        assert_eq!(wrap(line, ' ', at), Wrap::Newline);
        let w = wrap(line, 'x', at);
        assert_eq!(w, Wrap::BreakAt(line.rfind(' ').unwrap()));
        assert_eq!(
            wrap("see https://example.com/a/very/long/link", 'x', 10),
            Wrap::No
        );
        assert_eq!(
            wrap("averyveryverylongwordwithoutspaces", 'x', 10),
            Wrap::No
        );
        assert_eq!(wrap_prefix("> - a list item"), ">   ");
    }

    #[test]
    fn measuring() {
        let (w, l, longest, over) = measure(
            "one two\nthree https://example.com/aaaaaaaaaaaa\n> a quoted very long line indeed",
            12,
        );
        assert_eq!((w, l), (11, 3));
        assert!(longest > 12);
        assert_eq!(over, 0);
    }
}

/// What a stretch of a Markdown line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Md {
    Heading,
    Bold,
    Italic,
    Code,
    Link,
}

/// The Markdown in one line, as ranges of characters: headings, `code`,
/// **bold**, *italic* or _italic_, and [links](url). Not nested.
pub fn md_spans(line: &str) -> Vec<(usize, usize, Md)> {
    let c: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let lead = c.iter().take_while(|x| **x == '#').count();
    if (1..=6).contains(&lead) && c.get(lead) == Some(&' ') {
        return vec![(0, c.len(), Md::Heading)];
    }
    let find = |from: usize, pat: &[char]| -> Option<usize> {
        (from..c.len().saturating_sub(pat.len() - 1)).find(|&i| c[i..i + pat.len()] == *pat)
    };
    let mut i = 0;
    while i < c.len() {
        let rest_ok = |at: usize| c.get(at).is_some_and(|x| !x.is_whitespace());
        match c[i] {
            '`' => {
                if let Some(e) = find(i + 1, &['`']) {
                    out.push((i, e + 1, Md::Code));
                    i = e + 1;
                    continue;
                }
            }
            '*' if c.get(i + 1) == Some(&'*') && rest_ok(i + 2) => {
                if let Some(e) = find(i + 2, &['*', '*']) {
                    out.push((i, e + 2, Md::Bold));
                    i = e + 2;
                    continue;
                }
            }
            ch @ ('*' | '_')
                if rest_ok(i + 1) && (ch == '*' || i == 0 || !c[i - 1].is_alphanumeric()) =>
            {
                if let Some(e) =
                    find(i + 1, &[ch]).filter(|&e| e > i + 1 && !c[e - 1].is_whitespace())
                {
                    out.push((i, e + 1, Md::Italic));
                    i = e + 1;
                    continue;
                }
            }
            '[' => {
                if let Some(close) = find(i + 1, &[']', '('])
                    && let Some(end) = find(close + 2, &[')'])
                {
                    out.push((i, end + 1, Md::Link));
                    i = end + 1;
                    continue;
                }
            }
            _ => {}
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod md_tests {
    use super::*;

    #[test]
    fn markdown_spans() {
        let l = "Some **bold**, *it* and `code` and [a link](https://x.example) done";
        let kinds: Vec<Md> = md_spans(l).into_iter().map(|s| s.2).collect();
        assert_eq!(kinds, [Md::Bold, Md::Italic, Md::Code, Md::Link]);
        let s = md_spans(l);
        let text: Vec<char> = l.chars().collect();
        assert_eq!(text[s[0].0..s[0].1].iter().collect::<String>(), "**bold**");
        assert_eq!(md_spans("## A heading"), [(0, 12, Md::Heading)]);
        assert!(md_spans("2 * 3 * 4 and snake_case_name").is_empty());
        assert!(md_spans("> quoted line").is_empty());
    }
}

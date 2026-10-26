//! Search over every field of every card, quick at tens of thousands of
//! cards.
//!
//! Each card's words (names, nicknames, emails and their parts, the
//! organisation and title, addresses, notes, categories, links) are folded
//! (lower case, accents off) and kept in one sorted list, their text in a
//! single buffer. A query's words each find their prefix in the list by
//! binary search, and a card must match them all. Phone numbers are matched
//! by digits, anywhere in the number, however they are written. A match in
//! a name counts for more than one in a note.

use crate::{
    book::{Books, CardId},
    model::Card,
    text::{fold, phone_digits, words},
};

/// Where a word came from, best first: matches rank by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Field {
    Name = 0,
    Org = 1,
    Email = 2,
    Phone = 3,
    Place = 4,
    Other = 5,
}

/// One card in the index: what lists show and sort by.
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: CardId,
    pub name: String,
    pub sort: String,
    /// What else to show under the name: an email, else a phone, else the
    /// organisation.
    pub detail: String,
    pub book: usize,
}

#[derive(Clone, Copy)]
struct Token {
    start: u32,
    len: u16,
    entry: u32,
    field: Field,
}

/// The search index of a set of address books, at one generation.
pub struct Index {
    pub generation: u64,
    pub entries: Vec<Entry>,
    text: String,
    tokens: Vec<Token>,
    /// Each phone number's digits, by entry.
    phones: Vec<(String, u32)>,
}

impl Index {
    pub fn build(books: &Books) -> Self {
        let mut entries = Vec::with_capacity(books.len());
        let mut text = String::new();
        let mut tokens = Vec::new();
        let mut phones = Vec::new();
        for (id, _, card) in books.cards() {
            let n = entries.len() as u32;
            push_words(&mut text, &mut tokens, &card.display_name(), n, Field::Name);
            if let Some(name) = card.name() {
                for part in [
                    &name.given,
                    &name.family,
                    &name.additional,
                    &name.prefixes,
                    &name.suffixes,
                ] {
                    push_words(&mut text, &mut tokens, part, n, Field::Name);
                }
            }
            for nick in card.nicknames() {
                push_words(&mut text, &mut tokens, &nick, n, Field::Name);
            }
            for o in card.org() {
                push_words(&mut text, &mut tokens, &o, n, Field::Org);
            }
            if let Some(t) = card.title() {
                push_words(&mut text, &mut tokens, &t, n, Field::Org);
            }
            if let Some(r) = card.role() {
                push_words(&mut text, &mut tokens, &r, n, Field::Org);
            }
            for e in card.emails() {
                // the address whole, and its parts by the word splitting
                push_words(&mut text, &mut tokens, &e.value, n, Field::Email);
                push_token(&mut text, &mut tokens, &fold(&e.value), n, Field::Email);
            }
            for p in card.phones() {
                let digits = phone_digits(&p.value);
                if !digits.is_empty() {
                    phones.push((digits.trim_start_matches('+').to_owned(), n));
                }
            }
            for a in card.addresses() {
                push_words(&mut text, &mut tokens, &a.one_line(), n, Field::Place);
            }
            for c in card.categories() {
                push_words(&mut text, &mut tokens, &c, n, Field::Other);
            }
            for note in card.notes() {
                push_words(&mut text, &mut tokens, &note, n, Field::Other);
            }
            for u in card.urls().into_iter().chain(card.chats()) {
                push_words(&mut text, &mut tokens, &u.value, n, Field::Other);
            }
            entries.push(Entry {
                detail: detail(&card),
                name: card.display_name(),
                sort: card.sort_key(),
                book: books.book_of(&id.path).unwrap_or(0),
                id,
            });
        }
        tokens.sort_unstable_by(|a, b| {
            let ta = &text[a.start as usize..(a.start + a.len as u32) as usize];
            let tb = &text[b.start as usize..(b.start + b.len as u32) as usize];
            ta.cmp(tb).then(a.entry.cmp(&b.entry))
        });
        Index {
            generation: books.generation(),
            entries,
            text,
            tokens,
            phones,
        }
    }

    fn token(&self, t: &Token) -> &str {
        &self.text[t.start as usize..(t.start + t.len as u32) as usize]
    }

    /// Entries matching `query`, best first; all of them, by name, for an
    /// empty query.
    pub fn search(&self, query: &str) -> Vec<usize> {
        let query = query.trim();
        if query.is_empty() {
            let mut all: Vec<usize> = (0..self.entries.len()).collect();
            all.sort_by(|a, b| self.entries[*a].sort.cmp(&self.entries[*b].sort));
            return all;
        }
        // scores by entry: lower is better, summed over the words
        let mut scores: Option<std::collections::HashMap<u32, u32>> = None;
        for word in query_words(query) {
            let mut found: std::collections::HashMap<u32, u32> = Default::default();
            // a number: its digits anywhere in a phone number
            let digits: String = word.chars().filter(char::is_ascii_digit).collect();
            if digits.len() >= 3
                && digits.len() == word.chars().filter(|c| c.is_alphanumeric()).count()
            {
                for (p, e) in &self.phones {
                    if p.contains(&digits) {
                        let s = Field::Phone as u32 * 10 + u32::from(!p.ends_with(&digits));
                        found
                            .entry(*e)
                            .and_modify(|b| *b = (*b).min(s))
                            .or_insert(s);
                    }
                }
            }
            // words starting with it, by binary search to the first
            let first = self
                .tokens
                .partition_point(|t| self.token(t) < word.as_str());
            for t in &self.tokens[first..] {
                let tok = self.token(t);
                if !tok.starts_with(word.as_str()) {
                    break;
                }
                // an exact word beats a prefix of one
                let s = t.field as u32 * 10 + u32::from(tok.len() != word.len());
                found
                    .entry(t.entry)
                    .and_modify(|b| *b = (*b).min(s))
                    .or_insert(s);
            }
            scores = Some(match scores {
                None => found,
                Some(prev) => prev
                    .into_iter()
                    .filter_map(|(e, s)| found.get(&e).map(|f| (e, s + f)))
                    .collect(),
            });
        }
        let mut out: Vec<(u32, u32)> = scores.unwrap_or_default().into_iter().collect();
        out.sort_by(|(ea, sa), (eb, sb)| {
            sa.cmp(sb).then_with(|| {
                self.entries[*ea as usize]
                    .sort
                    .cmp(&self.entries[*eb as usize].sort)
            })
        });
        out.into_iter().map(|(e, _)| e as usize).collect()
    }

    /// The entry for a card, if it is indexed.
    pub fn position(&self, id: &CardId) -> Option<usize> {
        self.entries.iter().position(|e| &e.id == id)
    }

    /// How much memory the index holds, roughly, in bytes.
    pub fn size(&self) -> usize {
        self.text.capacity()
            + self.tokens.capacity() * std::mem::size_of::<Token>()
            + self
                .entries
                .iter()
                .map(|e| {
                    e.name.len() + e.sort.len() + e.detail.len() + e.id.path.as_os_str().len() + 96
                })
                .sum::<usize>()
            + self.phones.iter().map(|(p, _)| p.len() + 28).sum::<usize>()
    }
}

/// A query's words, folded as the index's are. An email address or a
/// number is kept whole as well as split.
fn query_words(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in q.split_whitespace() {
        let folded = fold(raw);
        if folded.contains('@') {
            out.push(folded);
            continue;
        }
        let parts = words(raw);
        if parts.is_empty() {
            continue;
        }
        // "+44" or "7700-900": digits for phone matching, whole
        if folded.chars().any(|c| c.is_ascii_digit())
            && folded
                .chars()
                .all(|c| c.is_ascii_digit() || "+-() .".contains(c))
        {
            out.push(folded.chars().filter(char::is_ascii_digit).collect());
            continue;
        }
        // all the word's parts must match: "o'brien" is "o" and "brien",
        // or "obrien"; take the joined form when there is one
        out.push(parts.last().cloned().unwrap());
    }
    out
}

/// What to show under a card's name.
fn detail(card: &Card) -> String {
    if let Some(e) = card.emails().into_iter().next() {
        return e.value;
    }
    if let Some(p) = card.phones().into_iter().next() {
        return p.value;
    }
    card.org().join(", ")
}

fn push_token(text: &mut String, tokens: &mut Vec<Token>, w: &str, entry: u32, field: Field) {
    if w.is_empty() || w.len() > u16::MAX as usize {
        return;
    }
    tokens.push(Token {
        start: text.len() as u32,
        len: w.len() as u16,
        entry,
        field,
    });
    text.push_str(w);
}

fn push_words(text: &mut String, tokens: &mut Vec<Token>, s: &str, entry: u32, field: Field) {
    for w in words(s) {
        push_token(text, tokens, &w, entry, field);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn books(cards: &[&str]) -> (tempfile::TempDir, Books) {
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path().join("contacts");
        std::fs::create_dir(&book).unwrap();
        for (i, c) in cards.iter().enumerate() {
            std::fs::write(
                book.join(format!("{i}.vcf")),
                format!("BEGIN:VCARD\r\nVERSION:3.0\r\nUID:{i}\r\n{c}END:VCARD\r\n"),
            )
            .unwrap();
        }
        let b = Books::load(dir.path()).unwrap();
        (dir, b)
    }

    fn names(index: &Index, q: &str) -> Vec<String> {
        index
            .search(q)
            .into_iter()
            .map(|i| index.entries[i].name.clone())
            .collect()
    }

    #[test]
    fn finds_by_any_field() {
        let (_d, b) = books(&[
            "FN:Priya Shah\r\nN:Shah;Priya\r\nEMAIL:priya@northwind.example\r\nORG:Northwind\r\n",
            "FN:Sam Lee\r\nTEL:+44 7700 900123\r\nNOTE:Plays piano on Tuesdays\r\n",
            "FN:Zoë Ørsted\r\nADR:;;1 Park Row;Leeds;;LS1;UK\r\nNICKNAME:Zed\r\n",
            "FN:Samantha O'Brien\r\nCATEGORIES:Climbing\r\n",
        ]);
        let index = Index::build(&b);
        // name prefix, and name before other fields
        assert_eq!(names(&index, "sam"), ["Sam Lee", "Samantha O'Brien"]);
        // accents folded either way
        assert_eq!(names(&index, "zoe"), ["Zoë Ørsted"]);
        assert_eq!(names(&index, "Ørst"), ["Zoë Ørsted"]);
        // nickname, email, organisation, place, note, category
        assert_eq!(names(&index, "zed"), ["Zoë Ørsted"]);
        assert_eq!(names(&index, "priya@north"), ["Priya Shah"]);
        assert_eq!(names(&index, "northwind"), ["Priya Shah"]);
        assert_eq!(names(&index, "leeds"), ["Zoë Ørsted"]);
        assert_eq!(names(&index, "piano"), ["Sam Lee"]);
        assert_eq!(names(&index, "climb"), ["Samantha O'Brien"]);
        // phone digits anywhere, however typed
        assert_eq!(names(&index, "900123"), ["Sam Lee"]);
        assert_eq!(names(&index, "7700 900"), ["Sam Lee"]);
        // every word must match
        assert_eq!(names(&index, "sam piano"), ["Sam Lee"]);
        assert!(names(&index, "sam leeds").is_empty());
        // apostrophes either way
        assert_eq!(names(&index, "obrien"), ["Samantha O'Brien"]);
        assert_eq!(names(&index, "o'brien"), ["Samantha O'Brien"]);
        // everyone, by family name then given, for nothing typed
        assert_eq!(
            names(&index, ""),
            ["Sam Lee", "Samantha O'Brien", "Priya Shah", "Zoë Ørsted"]
        );
    }
}

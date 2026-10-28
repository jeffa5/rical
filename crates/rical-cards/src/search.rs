//! Search over every field of every card, quick at tens of thousands of
//! cards.
//!
//! Each card's words (names, nicknames, emails and their parts, the
//! organisation and title, addresses, notes, categories, links) are folded
//! (lower case, accents off) and kept in one sorted list, their text in a
//! single buffer. A query's words each find their prefix in the list by
//! binary search, and a card must match them all. Phone numbers are matched
//! by digits, anywhere in the number, however either is written: spaces and
//! punctuation aside, and with a trunk 0 or a country code. A match in a
//! name counts for more than one in a note.

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
    /// The same card (by UID) in other address books.
    pub copies: Vec<CardId>,
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
    /// Each entry's place in name order, to sort by without comparing
    /// names each time.
    rank: Vec<u32>,
    text: String,
    tokens: Vec<Token>,
    /// Each phone number's digits, by entry.
    phones: Vec<(String, u32)>,
}

impl Index {
    pub fn build(books: &Books) -> Self {
        // one entry per person: a card with the same UID in another address
        // book (two synced copies of one account, say) is a copy, noted
        let mut cards: Vec<(CardId, Card, Vec<CardId>)> = Vec::with_capacity(books.len());
        let mut seen: std::collections::HashMap<&str, usize> = Default::default();
        for (id, _, card) in books.cards() {
            if let Some(uid) = card.uid().filter(|u| !u.is_empty())
                && let Some(&i) = seen.get(uid)
                && books.book_of(&cards[i].0.path) != books.book_of(&id.path)
            {
                cards[i].2.push(id);
                continue;
            }
            if let Some(uid) = card.uid().filter(|u| !u.is_empty()) {
                seen.entry(uid).or_insert(cards.len());
            }
            cards.push((id, card, Vec::new()));
        }

        // the words of each card, on every core, in chunks joined after
        let threads = std::thread::available_parallelism()
            .map_or(1, |n| n.get())
            .min(16);
        let chunk = cards.len().div_ceil(threads).max(256);
        let parts: Vec<Part> = std::thread::scope(|s| {
            let handles: Vec<_> = cards
                .chunks(chunk)
                .enumerate()
                .map(|(i, slice)| {
                    let first = (i * chunk) as u32;
                    s.spawn(move || index_cards(books, slice, first))
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let mut entries = Vec::with_capacity(cards.len());
        let mut text = String::new();
        let mut tokens = Vec::new();
        let mut phones = Vec::new();
        for p in parts {
            let offset = text.len() as u32;
            text.push_str(&p.text);
            tokens.extend(p.tokens.into_iter().map(|mut t| {
                t.start += offset;
                t
            }));
            entries.extend(p.entries);
            phones.extend(p.phones);
        }
        tokens.sort_unstable_by(|a, b| {
            let ta = &text[a.start as usize..(a.start + a.len as u32) as usize];
            let tb = &text[b.start as usize..(b.start + b.len as u32) as usize];
            ta.cmp(tb).then(a.entry.cmp(&b.entry))
        });
        text.shrink_to_fit();
        let rank = ranks(&entries, SortBy::Shown);
        tokens.shrink_to_fit();
        Index {
            generation: books.generation(),
            entries,
            rank,
            text,
            tokens,
            phones,
        }
    }

    /// Put everyone in this order, for lists and ties between matches.
    pub fn sort_by(&mut self, by: SortBy) {
        self.rank = ranks(&self.entries, by);
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
            all.sort_by_key(|&e| self.rank[e]);
            return all;
        }
        // scores by entry: lower is better, summed over the words
        let mut scores: Option<std::collections::HashMap<u32, u32>> = None;
        for QueryWord { word, phone } in query_words(query) {
            let mut found: std::collections::HashMap<u32, u32> = Default::default();
            // a number: in a phone number, however either is written
            if !phone.is_empty() {
                for (p, e) in &self.phones {
                    if let Some(c) = phone.iter().find(|c| p.contains(c.as_str())) {
                        let s = Field::Phone as u32 * 10 + u32::from(!p.ends_with(c.as_str()));
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
        out.sort_unstable_by_key(|(e, score)| (*score, self.rank[*e as usize]));
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

/// How the list of everyone is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortBy {
    /// By the name as shown ("Sam Lee" under S), as most contact apps do.
    #[default]
    Shown,
    /// By family name, then given ("Sam Lee" under L).
    Family,
}

/// Each entry's place in `by` order.
fn ranks(entries: &[Entry], by: SortBy) -> Vec<u32> {
    let keys: Vec<String> = entries
        .iter()
        .map(|e| match by {
            SortBy::Shown => fold(e.name.trim()),
            SortBy::Family => e.sort.clone(),
        })
        .collect();
    let mut order: Vec<u32> = (0..entries.len() as u32).collect();
    order.sort_by(|a, b| keys[*a as usize].cmp(&keys[*b as usize]));
    let mut rank = vec![0u32; entries.len()];
    for (place, e) in order.iter().enumerate() {
        rank[*e as usize] = place as u32;
    }
    rank
}

/// A word of a query: as the index's words are folded, and, for a
/// number, the digit runs to look for in phone numbers.
struct QueryWord {
    word: String,
    phone: Vec<String>,
}

/// Whether a query word is (part of) a number as people write them:
/// "+44", "(0113)", "496-0000".
fn is_number(w: &str) -> bool {
    w.chars().any(|c| c.is_ascii_digit())
        && w.chars().all(|c| c.is_ascii_digit() || "+-().".contains(c))
}

/// What to look for in phone numbers' digits for a number typed as
/// `typed`, however each is written. A number is kept in the index as its
/// digits, so "+44 7700 900123" is "447700900123" and "07700 900123" is
/// "07700900123"; the part they share is the number without its trunk 0,
/// or without the international prefix ("+", "00") and country code. A
/// country code is one to three digits, so each is tried.
fn phone_needles(typed: &str) -> Vec<String> {
    let digits: String = typed.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 3 {
        return Vec::new();
    }
    let international = typed.trim_start().starts_with('+') || digits.starts_with("00");
    let national = digits.trim_start_matches('0');
    let mut out = vec![national.to_owned()];
    if international {
        for cc in 1..=3 {
            if national.len() >= cc + 4 {
                out.push(national[cc..].to_owned());
            }
        }
    }
    out.retain(|n| n.len() >= 2);
    out
}

/// A query's words, folded as the index's are. An email address is kept
/// whole; numbers typed with spaces ("07700 900 123") are one number.
fn query_words(q: &str) -> Vec<QueryWord> {
    let mut out = Vec::new();
    let raw: Vec<&str> = q.split_whitespace().collect();
    let mut i = 0;
    while i < raw.len() {
        let w = raw[i];
        i += 1;
        let folded = fold(w);
        if folded.contains('@') {
            out.push(QueryWord {
                word: folded,
                phone: Vec::new(),
            });
            continue;
        }
        if is_number(w) {
            let mut typed = w.to_owned();
            while i < raw.len() && is_number(raw[i]) {
                typed.push_str(raw[i]);
                i += 1;
            }
            out.push(QueryWord {
                word: typed.chars().filter(char::is_ascii_digit).collect(),
                phone: phone_needles(&typed),
            });
            continue;
        }
        // all the word's parts must match: "o'brien" is "o" and "brien",
        // or "obrien"; take the joined form when there is one
        if let Some(last) = words(w).pop() {
            out.push(QueryWord {
                word: last,
                phone: Vec::new(),
            });
        }
    }
    out
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
/// Part of an index, made on one thread.
struct Part {
    entries: Vec<Entry>,
    text: String,
    tokens: Vec<Token>,
    phones: Vec<(String, u32)>,
}

/// Index `cards`, numbering entries from `first`.
fn index_cards(books: &Books, cards: &[(CardId, Card, Vec<CardId>)], first: u32) -> Part {
    let mut part = Part {
        entries: Vec::with_capacity(cards.len()),
        text: String::new(),
        tokens: Vec::new(),
        phones: Vec::new(),
    };
    for (i, (id, card, copies)) in cards.iter().enumerate() {
        let n = first + i as u32;
        let (text, tokens) = (&mut part.text, &mut part.tokens);
        let display = card.display_name();
        push_words(text, tokens, &display, n, Field::Name);
        if let Some(name) = card.name() {
            for p in [
                &name.given,
                &name.family,
                &name.additional,
                &name.prefixes,
                &name.suffixes,
            ] {
                push_words(text, tokens, p, n, Field::Name);
            }
        }
        for nick in card.nicknames() {
            push_words(text, tokens, &nick, n, Field::Name);
        }
        for o in card.org() {
            push_words(text, tokens, &o, n, Field::Org);
        }
        for t in card.title().into_iter().chain(card.role()) {
            push_words(text, tokens, &t, n, Field::Org);
        }
        let emails = card.emails();
        for e in &emails {
            // the address whole, and its parts by the word splitting
            push_words(text, tokens, &e.value, n, Field::Email);
            push_token(text, tokens, &fold(&e.value), n, Field::Email);
        }
        let phones = card.phones();
        for p in &phones {
            let digits = phone_digits(&p.value);
            if !digits.is_empty() {
                part.phones
                    .push((digits.trim_start_matches('+').to_owned(), n));
            }
        }
        for a in card.addresses() {
            push_words(text, tokens, &a.one_line(), n, Field::Place);
        }
        for c in card.categories() {
            push_words(text, tokens, &c, n, Field::Other);
        }
        for note in card.notes() {
            push_words(text, tokens, &note, n, Field::Other);
        }
        for u in card.urls().into_iter().chain(card.chats()) {
            push_words(text, tokens, &u.value, n, Field::Other);
        }
        let detail = emails
            .first()
            .map(|e| e.value.clone())
            .or_else(|| phones.first().map(|p| p.value.clone()))
            .unwrap_or_else(|| card.org().join(", "));
        part.entries.push(Entry {
            detail,
            sort: card.sort_key(),
            name: display,
            book: books.book_of(&id.path).unwrap_or(0),
            id: id.clone(),
            copies: copies.clone(),
        });
    }
    part
}
#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn books(cards: &[&str]) -> (tempfile::TempDir, Books) {
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

    pub(super) fn names(index: &Index, q: &str) -> Vec<String> {
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
        // everyone, by name as shown, for nothing typed
        assert_eq!(
            names(&index, ""),
            ["Priya Shah", "Sam Lee", "Samantha O'Brien", "Zoë Ørsted"]
        );
        // or by family name, then given
        let mut index = index;
        index.sort_by(SortBy::Family);
        assert_eq!(
            names(&index, ""),
            ["Sam Lee", "Samantha O'Brien", "Priya Shah", "Zoë Ørsted"]
        );
    }
}

#[cfg(test)]
mod phone_tests {
    use super::tests::{books, names};

    #[test]
    fn numbers_however_written() {
        let (_d, b) = books(&[
            "FN:Sam Lee\r\nTEL:+44 7700 900123\r\n",
            "FN:Jo Park\r\nTEL:0113 496 0000\r\n",
            "FN:Ana Silva\r\nTEL:(415) 555-0134\r\n",
            "FN:Wei Chen\r\nTEL:+1 415 555 0199\r\n",
        ]);
        let index = super::Index::build(&b);
        // spaced, run together, dashed or dotted
        for q in ["7700 900123", "7700900123", "7700-900-123", "77 00 900 1"] {
            assert_eq!(names(&index, q), ["Sam Lee"], "{q}");
        }
        // the national form finds an international number
        for q in ["07700 900123", "07700900123", "077009"] {
            assert_eq!(names(&index, q), ["Sam Lee"], "{q}");
        }
        // and the international form a national one, with + or 00
        for q in [
            "+44 113 496 0000",
            "0044 113 496 0000",
            "+441134960000",
            "+44 113 4",
        ] {
            assert_eq!(names(&index, q), ["Jo Park"], "{q}");
        }
        // brackets, and a one-digit country code
        assert_eq!(names(&index, "(415) 555"), ["Ana Silva", "Wei Chen"]);
        assert_eq!(names(&index, "+1 415 555 0134"), ["Ana Silva"]);
        assert_eq!(names(&index, "555-0199"), ["Wei Chen"]);
        // a run of digits must be in one number, in order
        assert!(names(&index, "900123 7700").is_empty());
        // with a name
        assert_eq!(names(&index, "sam 07700"), ["Sam Lee"]);
    }
}

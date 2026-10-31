//! Search over every field of every card, quick at tens of thousands of
//! cards, from the address books' index (see [`crate::book`]).
//!
//! Each card's words (names, nicknames, emails and their parts, the
//! organisation and title, addresses, notes, categories, links) are folded
//! (lower case, accents off) and kept in the index. A query's words each
//! find the words they start, through its index of words, and a card must
//! match them all. Phone numbers are matched by digits, anywhere in the
//! number, however either is written: spaces and punctuation aside, and
//! with a trunk 0 or a country code. A match in a name counts for more
//! than one in a note.

use std::{collections::HashMap, sync::Arc};

use rical_index::rusqlite::params;

use crate::{
    book::{Books, CardId},
    model::Card,
    text::{fold, phone_digits, words},
};

/// Where a word came from, best first: matches rank by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

/// Everyone in a set of address books, at one generation, to list and
/// search.
pub struct Index {
    pub generation: u64,
    pub entries: Vec<Entry>,
    /// Each entry's place in name order, to sort by without comparing
    /// names each time.
    rank: Vec<u32>,
    db: Arc<rical_index::Index>,
    /// Each card (by its file's id in the index, and place in it) to its
    /// entry (a copy's, to the entry it is a copy in).
    by_card: HashMap<(i64, i64), u32>,
}

impl Index {
    /// Everyone, from the index: no card is read.
    pub fn build(books: &Books) -> Self {
        // one entry per person: a card with the same UID in another address
        // book (two synced copies of one account, say) is a copy, noted
        let listed = books.listed();
        let mut entries: Vec<Entry> = Vec::with_capacity(listed.len());
        let mut by_card = HashMap::with_capacity(listed.len());
        let mut seen: HashMap<String, usize> = HashMap::new();
        for l in listed {
            let book = books.book_of(&l.id.path).unwrap_or(0);
            if let Some(uid) = l.uid.as_ref().filter(|u| !u.is_empty()) {
                if let Some(&i) = seen.get(uid)
                    && entries[i].book != book
                {
                    by_card.insert((l.file, l.id.index as i64), i as u32);
                    entries[i].copies.push(l.id);
                    continue;
                }
                seen.entry(uid.clone()).or_insert(entries.len());
            }
            by_card.insert((l.file, l.id.index as i64), entries.len() as u32);
            entries.push(Entry {
                id: l.id,
                name: l.name,
                sort: l.sort,
                detail: l.detail,
                book,
                copies: Vec::new(),
            });
        }
        let rank = ranks(&entries, SortBy::Shown);
        Index {
            generation: books.generation(),
            entries,
            rank,
            db: books.index(),
            by_card,
        }
    }

    /// Put everyone in this order, for lists and ties between matches.
    pub fn sort_by(&mut self, by: SortBy) {
        self.rank = ranks(&self.entries, by);
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
        let conn = self.db.conn();
        // scores by entry: lower is better, summed over the words
        let mut scores: Option<HashMap<u32, u32>> = None;
        for QueryWord { word, phone } in query_words(query) {
            let mut found: HashMap<u32, u32> = HashMap::new();
            let mut note = |card: (i64, i64), s: u32| {
                if let Some(&e) = self.by_card.get(&card) {
                    found.entry(e).and_modify(|b| *b = (*b).min(s)).or_insert(s);
                }
            };
            // a number: in a phone number, however either is written
            if !phone.is_empty()
                && let Ok(mut st) = conn.prepare_cached("SELECT file, idx, digits FROM phones")
                && let Ok(rows) = st.query_map([], |r| {
                    Ok((
                        (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?),
                        r.get::<_, String>(2)?,
                    ))
                })
            {
                for (card, p) in rows.flatten() {
                    if let Some(c) = phone.iter().find(|c| p.contains(c.as_str())) {
                        note(
                            card,
                            Field::Phone as u32 * 10 + u32::from(!p.ends_with(c.as_str())),
                        );
                    }
                }
            }
            // words starting with it, through the index of words
            let mut upper = word.clone();
            upper.push(char::MAX);
            if let Ok(mut st) = conn.prepare_cached(
                "SELECT file, idx, field, word = ?1 FROM words WHERE word >= ?1 AND word < ?2",
            ) && let Ok(rows) = st.query_map(params![word, upper], |r| {
                Ok((
                    (r.get::<_, i64>(0)?, r.get::<_, i64>(1)?),
                    r.get::<_, i64>(2)?,
                    r.get::<_, bool>(3)?,
                ))
            }) {
                for (card, field, exact) in rows.flatten() {
                    // an exact word beats a prefix of one
                    note(card, field as u32 * 10 + u32::from(!exact));
                }
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

    /// How much memory the list holds, roughly, in bytes (the words are
    /// in the index, on disk).
    pub fn size(&self) -> usize {
        self.entries
            .iter()
            .map(|e| {
                e.name.len() + e.sort.len() + e.detail.len() + e.id.path.as_os_str().len() + 96
            })
            .sum::<usize>()
            + self.by_card.len() * 24
            + self.rank.len() * 4
    }
}

/// A card's words, each with its field; its numbers' digits; and the line
/// to show under its name.
pub(crate) fn words_of(card: Card) -> (Vec<(String, Field)>, Vec<String>, String) {
    let mut out = Vec::new();
    let mut push = |s: &str, field: Field| {
        for w in words(s) {
            out.push((w, field));
        }
    };
    let display = card.display_name();
    push(&display, Field::Name);
    if let Some(name) = card.name() {
        for p in [
            &name.given,
            &name.family,
            &name.additional,
            &name.prefixes,
            &name.suffixes,
        ] {
            push(p, Field::Name);
        }
    }
    for nick in card.nicknames() {
        push(&nick, Field::Name);
    }
    for o in card.org() {
        push(&o, Field::Org);
    }
    for t in card.title().into_iter().chain(card.role()) {
        push(&t, Field::Org);
    }
    let emails = card.emails();
    for e in &emails {
        // the address's parts by the word splitting, and it whole
        push(&e.value, Field::Email);
    }
    for a in card.addresses() {
        push(&a.one_line(), Field::Place);
    }
    for c in card.categories() {
        push(&c, Field::Other);
    }
    for note in card.notes() {
        push(&note, Field::Other);
    }
    for u in card.urls().into_iter().chain(card.chats()) {
        push(&u.value, Field::Other);
    }
    for e in &emails {
        out.push((fold(&e.value), Field::Email));
    }
    out.retain(|(w, _)| !w.is_empty());
    let phones = card.phones();
    let digits = phones
        .iter()
        .map(|p| phone_digits(&p.value).trim_start_matches('+').to_owned())
        .filter(|d| !d.is_empty())
        .collect();
    let detail = emails
        .first()
        .map(|e| e.value.clone())
        .or_else(|| phones.first().map(|p| p.value.clone()))
        .unwrap_or_else(|| card.org().join(", "));
    (out, digits, detail)
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
mod shared_tests {
    use super::*;

    /// Another app reading a file again (and so writing its cards anew)
    /// leaves a search built before it working.
    #[test]
    fn search_survives_a_file_read_again_elsewhere() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("contacts");
        std::fs::create_dir(&root).unwrap();
        let card = |name: &str| {
            format!("BEGIN:VCARD\r\nVERSION:4.0\r\nUID:{name}\r\nFN:{name}\r\nEND:VCARD\r\n")
        };
        std::fs::write(root.join("a.vcf"), card("Ada")).unwrap();
        std::fs::write(root.join("b.vcf"), card("Bo")).unwrap();
        let cache = dir.path().join("index.sqlite");
        let books = Books::load_cached(&root, Some(&cache)).unwrap();
        let index = Index::build(&books);
        // elsewhere: the file is touched and indexed again
        let mut other = Books::load_cached(&root, Some(&cache)).unwrap();
        std::fs::write(root.join("a.vcf"), card("Ada")).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(root.join("a.vcf"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert!(other.refresh().unwrap());
        assert_eq!(tests::names(&index, "ada"), ["Ada"]);
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

//! Cards from elsewhere: a `.vcf` another app opened with us, or an
//! export. Each card is matched to the address books by its UID first, so
//! opening a contact you already have shows yours rather than adding a
//! copy; without a UID match, an email address or phone number in common
//! says it is probably someone you have.

use std::path::PathBuf;

use anyhow::Context as _;
use compact_ics::Component;
use rical_core::write::Change;

use crate::{
    book::{Books, CardId},
    model::Card,
    text::{email_key, phone_tail},
};

/// What an incoming card is to the address books.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// The same card (by UID) as one you have, saying the same.
    Same(CardId),
    /// The same card, changed since; `newer` if its REV is later (or
    /// yours has none), so updating yours is the likely wish.
    Changed { id: CardId, newer: bool },
    /// No UID in common, but an email address or phone number: probably
    /// someone you have.
    Likely(CardId),
    /// Someone new.
    New,
}

/// One card from the file, as its own text, and what it matches.
#[derive(Debug, Clone)]
pub struct Incoming {
    pub text: String,
    pub card: Component,
    pub matched: Match,
}

impl Incoming {
    pub fn card(&self) -> Card<'_> {
        Card(&self.card)
    }
}

/// The top-level VCARDs of `text`, each as written.
fn split_cards(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut depth = 0usize;
    for line in text.split_inclusive('\n') {
        let content = line.trim_end_matches(['\r', '\n']);
        let upper = content.to_ascii_uppercase();
        if upper.starts_with("BEGIN:") {
            depth += 1;
            if depth == 1 && upper.trim() == "BEGIN:VCARD" {
                current = Some(String::new());
            }
        }
        if let Some(c) = current.as_mut() {
            c.push_str(line);
        }
        if upper.starts_with("END:") {
            depth = depth.saturating_sub(1);
            if depth == 0
                && let Some(c) = current.take()
            {
                let mut c = c;
                if !c.ends_with('\n') {
                    c.push_str("\r\n");
                }
                out.push(c);
            }
        }
    }
    out
}

/// What a card says, for telling whether two copies differ: its lines in
/// order, without REV and PRODID (which every save changes).
fn substance(c: &Component) -> Vec<(String, String)> {
    c.all_props()
        .iter()
        .filter(|p| !matches!(&*p.name, "REV" | "PRODID" | "X-ABLABEL"))
        .map(|p| (p.name.to_string(), p.value().trim().to_owned()))
        .collect()
}

/// Read the cards in `text` and match each to `books`.
pub fn plan(books: &Books, text: &str) -> anyhow::Result<Vec<Incoming>> {
    let mut out = Vec::new();
    for card_text in split_cards(text) {
        let (mut parsed, _) = compact_ics::parse(&card_text)?;
        let Some(card) = parsed.drain(..).next() else {
            continue;
        };
        let matched = match_card(books, &Card(&card));
        out.push(Incoming {
            text: card_text,
            card,
            matched,
        });
    }
    anyhow::ensure!(!out.is_empty(), "no contacts in the file");
    Ok(out)
}

fn match_card(books: &Books, incoming: &Card) -> Match {
    if let Some(uid) = incoming.uid()
        && let Some(id) = books.by_uid(uid).first()
        && let Some(mine) = books.get(id)
    {
        let mine = mine.card();
        if substance(mine.0) == substance(incoming.0) {
            return Match::Same(id.clone());
        }
        // REVs as written sort by time (both are ISO 8601 basic or both
        // extended in practice)
        let newer = match (incoming.revision(), mine.revision()) {
            (Some(theirs), Some(ours)) => theirs > ours,
            (Some(_), None) => true,
            _ => false,
        };
        return Match::Changed {
            id: id.clone(),
            newer,
        };
    }
    let emails: Vec<String> = incoming
        .emails()
        .iter()
        .map(|e| email_key(&e.value))
        .collect();
    let phones: Vec<String> = incoming
        .phones()
        .iter()
        .filter_map(|p| phone_tail(&p.value))
        .collect();
    if emails.is_empty() && phones.is_empty() {
        return Match::New;
    }
    // by the index: no card is read
    match books.with_keys(&emails, &phones) {
        Some(id) => Match::Likely(id),
        None => Match::New,
    }
}

impl Books {
    /// Add an incoming card to `book` as it is (its version kept), in a
    /// file named after its UID; one without a UID is given one.
    pub fn add_incoming(
        &mut self,
        book: usize,
        incoming: &Incoming,
    ) -> anyhow::Result<(Change, CardId)> {
        let dir = self
            .books
            .get(book)
            .context("no such address book")?
            .path
            .clone();
        let (uid, text) = match incoming.card().uid().filter(|u| !u.is_empty()) {
            Some(u) => (u.to_owned(), incoming.text.clone()),
            None => {
                let u = uuid::Uuid::new_v4().to_string();
                let text =
                    incoming
                        .text
                        .replacen("\nEND:VCARD", &format!("\nUID:{u}\r\nEND:VCARD"), 1);
                (u, text)
            }
        };
        // a file name from the UID, safe and unique
        let safe: String = uid
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let mut path: PathBuf = dir.join(format!("{safe}.vcf"));
        if path.exists() {
            path = dir.join(format!("{}.vcf", uuid::Uuid::new_v4()));
        }
        let change = Change {
            path: path.clone(),
            before: None,
            after: Some(text),
        };
        self.apply(&change)?;
        Ok((change, CardId { path, index: 0 }))
    }

    /// Replace a card you have with the incoming version, as it is.
    pub fn replace_with(&mut self, id: &CardId, incoming: &Incoming) -> anyhow::Result<Change> {
        anyhow::ensure!(id.index == 0, "{:?} holds several cards", id.path);
        let before =
            std::fs::read_to_string(&id.path).with_context(|| format!("reading {:?}", id.path))?;
        let change = Change {
            path: id.path.clone(),
            before: Some(before),
            after: Some(incoming.text.clone()),
        };
        self.apply(&change)?;
        Ok(change)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAM: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:sam-1\r\nFN:Sam Lee\r\n\
                       EMAIL:sam@example.com\r\nREV:20250101T000000Z\r\nEND:VCARD\r\n";

    fn setup() -> (tempfile::TempDir, Books) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("contacts")).unwrap();
        std::fs::write(dir.path().join("contacts/sam-1.vcf"), SAM).unwrap();
        let b = Books::load(dir.path()).unwrap();
        (dir, b)
    }

    #[test]
    fn matches_by_uid_then_by_address() {
        let (_d, mut b) = setup();
        // the same card, opened from another app: the one you have
        let same = plan(
            &b,
            &SAM.replace("REV:20250101T000000Z", "REV:20260101T000000Z"),
        )
        .unwrap();
        assert!(
            matches!(same[0].matched, Match::Same(_)),
            "{:?}",
            same[0].matched
        );
        // changed since, later: an update
        let newer = plan(
            &b,
            &SAM.replace("FN:Sam Lee", "FN:Samuel Lee")
                .replace("REV:20250101T000000Z", "REV:20260101T000000Z"),
        )
        .unwrap();
        assert!(matches!(
            newer[0].matched,
            Match::Changed { newer: true, .. }
        ));
        // no UID in common, but the address: probably Sam
        let likely = plan(
            &b,
            "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:other\r\nFN:S Lee\r\nEMAIL:SAM@example.com\r\nEND:VCARD\r\n",
        )
        .unwrap();
        assert!(matches!(likely[0].matched, Match::Likely(_)));
        // two in one file: each matched; someone new is new
        let two = format!(
            "{SAM}BEGIN:VCARD\r\nVERSION:2.1\r\nN:Park;Jo\r\nTEL;CELL:0113 496 0000\r\nEND:VCARD\r\n"
        );
        let both = plan(&b, &two).unwrap();
        assert_eq!(both.len(), 2);
        assert_eq!(both[1].matched, Match::New);
        assert_eq!(both[1].card().display_name(), "Jo Park");

        // adding the new one keeps it as it was, given a UID
        let (change, id) = b.add_incoming(0, &both[1]).unwrap();
        let text = change.after.unwrap();
        assert!(text.contains("VERSION:2.1\r\n"), "{text}");
        assert!(text.contains("\r\nUID:"), "{text}");
        assert_eq!(b.get(&id).unwrap().card().display_name(), "Jo Park");
        // and opened again, it is now the one you have
        let again = plan(&b, &text).unwrap();
        assert!(matches!(again[0].matched, Match::Same(_)));
    }
}

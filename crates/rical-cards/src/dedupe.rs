//! Finding duplicate cards and merging them, against contact sprawl.
//!
//! Cards are the same person when they share a UID, an email address or a
//! phone number (by its last nine digits, so `+44 7700 900123` and
//! `07700 900123` agree); sharing only a name of two words or more is a
//! weaker sign, shown as such. A merge keeps the card that says the most,
//! adds what the others have that it lacks, and lists the single values
//! that disagree for a person to choose between.

use std::collections::{BTreeMap, HashMap};

use crate::{
    book::{Books, CardId},
    edit::Draft,
    model::{Address, Card, Field},
    text::{email_key, fold, name_key, phone_tail},
};

/// Why cards look like the same person.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Reason {
    SameUid(String),
    SameEmail(String),
    SamePhone(String),
    /// Only the name: worth a look, not to merge blind.
    SameName(String),
}

impl Reason {
    /// Whether this alone makes them very likely the same.
    pub fn is_strong(&self) -> bool {
        !matches!(self, Reason::SameName(_))
    }

    pub fn describe(&self) -> String {
        match self {
            Reason::SameUid(_) => "the same card ID (UID)".to_owned(),
            Reason::SameEmail(e) => format!("the same email address, {e}"),
            Reason::SamePhone(p) => format!("the same number, ending {p}"),
            Reason::SameName(n) => format!("the same name, {n}"),
        }
    }
}

/// Cards that look like one person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    pub cards: Vec<CardId>,
    pub reasons: Vec<Reason>,
}

impl Group {
    /// Whether anything besides the name ties them.
    pub fn is_strong(&self) -> bool {
        self.reasons.iter().any(Reason::is_strong)
    }
}

/// Every group of likely duplicates: strong ones first, then by size.
///
/// A card with the same UID as one in another address book is a copy (two
/// synced copies of one account, say), not a duplicate, and is left out.
/// Cards sharing only a name make groups of their own: a common name never
/// joins otherwise unrelated cards together.
pub fn find(books: &Books) -> Vec<Group> {
    let mut cards: Vec<(CardId, Card)> = Vec::new();
    let mut first_book: HashMap<&str, Option<usize>> = HashMap::new();
    for (id, _, c) in books.cards() {
        if c.kind() != "individual" {
            continue;
        }
        if let Some(uid) = c.uid().filter(|u| !u.is_empty()) {
            let book = books.book_of(&id.path);
            match first_book.get(uid) {
                // a copy in another book
                Some(b) if *b != book => continue,
                Some(_) => {}
                None => {
                    first_book.insert(uid, book);
                }
            }
        }
        cards.push((id, c));
    }
    let mut strong: HashMap<Reason, Vec<usize>> = HashMap::new();
    let mut by_name: HashMap<String, (String, Vec<usize>)> = HashMap::new();
    for (i, (_, c)) in cards.iter().enumerate() {
        let mut add = |r: Reason| {
            let v = strong.entry(r).or_default();
            if v.last() != Some(&i) {
                v.push(i);
            }
        };
        if let Some(uid) = c.uid().filter(|u| !u.is_empty()) {
            add(Reason::SameUid(uid.to_owned()));
        }
        for e in c.emails() {
            let k = email_key(&e.value);
            if k.contains('@') {
                add(Reason::SameEmail(k));
            }
        }
        for p in c.phones() {
            if let Some(t) = phone_tail(&p.value) {
                add(Reason::SamePhone(t));
            }
        }
        let display = c.display_name();
        let key = name_key(&display);
        if key.split(' ').count() >= 2 {
            let entry = by_name.entry(key).or_insert_with(|| (display, Vec::new()));
            entry.1.push(i);
        }
    }

    // strong ties join cards into groups
    let mut parent: Vec<usize> = (0..cards.len()).collect();
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for members in strong.values().filter(|m| m.len() > 1) {
        for w in members.windows(2) {
            let (a, b) = (root(&mut parent, w[0]), root(&mut parent, w[1]));
            if a != b {
                parent[b] = a;
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..cards.len() {
        let r = root(&mut parent, i);
        groups.entry(r).or_default().push(i);
    }
    // each shared key is a reason for the group its cards are in: filed by
    // the group's root in one pass
    let mut reasons_of: HashMap<usize, Vec<Reason>> = HashMap::new();
    for (r, members) in &strong {
        if members.len() > 1 {
            let g = root(&mut parent, members[0]);
            reasons_of.entry(g).or_default().push(r.clone());
        }
    }
    let mut out: Vec<Group> = Vec::new();
    let mut grouped = vec![false; cards.len()];
    for (g, members) in groups.into_iter().filter(|(_, m)| m.len() > 1) {
        let mut reasons = reasons_of.remove(&g).unwrap_or_default();
        reasons.sort();
        for &i in &members {
            grouped[i] = true;
        }
        out.push(Group {
            cards: members.iter().map(|&i| cards[i].0.clone()).collect(),
            reasons,
        });
    }
    // the same name, among cards nothing else ties: weaker groups
    for (shown, members) in by_name.into_values() {
        let loose: Vec<usize> = members.into_iter().filter(|&i| !grouped[i]).collect();
        if loose.len() > 1 {
            out.push(Group {
                cards: loose.iter().map(|&i| cards[i].0.clone()).collect(),
                reasons: vec![Reason::SameName(shown)],
            });
        }
    }
    out.sort_by(|a, b| {
        (!a.is_strong(), std::cmp::Reverse(a.cards.len()))
            .cmp(&(!b.is_strong(), std::cmp::Reverse(b.cards.len())))
            .then_with(|| a.cards.cmp(&b.cards))
    });
    out
}

/// A single value the cards disagree on, and which one to keep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    pub field: &'static str,
    pub options: Vec<String>,
    pub chosen: usize,
}

/// How to merge some cards: which to keep, the merged card, and the values
/// to choose between.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merge {
    pub keep: CardId,
    pub others: Vec<CardId>,
    /// The kept card as it will be, with the conflicts' chosen values.
    pub draft: Draft,
    pub conflicts: Vec<Conflict>,
}

impl Merge {
    /// Choose option `i` for conflict `c`, updating the draft.
    pub fn choose(&mut self, c: usize, i: usize) {
        let Some(conflict) = self.conflicts.get_mut(c) else {
            return;
        };
        if i >= conflict.options.len() {
            return;
        }
        conflict.chosen = i;
        let v = conflict.options[i].clone();
        set_single(&mut self.draft, conflict.field, v);
    }
}

fn get_single(d: &Draft, field: &str) -> String {
    match field {
        "Name" => d.formatted_name.clone(),
        "Birthday" => d.birthday.clone(),
        "Anniversary" => d.anniversary.clone(),
        "Organisation" => d.org.join(", "),
        "Title" => d.title.clone(),
        "Role" => d.role.clone(),
        _ => String::new(),
    }
}

fn set_single(d: &mut Draft, field: &str, v: String) {
    match field {
        "Name" => d.formatted_name = v,
        "Birthday" => d.birthday = v,
        "Anniversary" => d.anniversary = v,
        "Organisation" => d.org = v.split(", ").map(str::to_owned).collect(),
        "Title" => d.title = v,
        "Role" => d.role = v,
        _ => {}
    }
}

const SINGLES: &[&str] = &[
    "Name",
    "Birthday",
    "Anniversary",
    "Organisation",
    "Title",
    "Role",
];

/// Add `more` to `list`, leaving out what it already has (by `key`).
fn union<T: Clone>(list: &mut Vec<T>, more: &[T], key: impl Fn(&T) -> String) {
    for m in more {
        let k = key(m);
        if !k.is_empty() && !list.iter().any(|x| key(x) == k) {
            list.push(m.clone());
        }
    }
}

/// Plan merging `ids` (two or more): keep the one that says the most, take
/// in what the others add, and list where they disagree.
pub fn plan(books: &Books, ids: &[CardId]) -> Option<Merge> {
    let cards: Vec<(CardId, Card)> = ids
        .iter()
        .filter_map(|id| books.get(id).map(|c| (id.clone(), c)))
        .collect();
    if cards.len() < 2 {
        return None;
    }
    let keep_at = (0..cards.len())
        .max_by_key(|&i| (cards[i].1.richness(), std::cmp::Reverse(i)))
        .unwrap();
    let mut order: Vec<usize> = vec![keep_at];
    order.extend((0..cards.len()).filter(|&i| i != keep_at));
    let drafts: Vec<Draft> = order.iter().map(|&i| Draft::of(&cards[i].1)).collect();

    let mut draft = drafts[0].clone();
    let mut conflicts = Vec::new();
    for field in SINGLES {
        let mut options: Vec<String> = Vec::new();
        for d in &drafts {
            let v = get_single(d, field).trim().to_owned();
            if !v.is_empty() && !options.iter().any(|o| fold(o) == fold(&v)) {
                options.push(v);
            }
        }
        match options.len() {
            0 => {}
            1 => set_single(&mut draft, field, options[0].clone()),
            _ => {
                // the kept card's, if it has one
                let kept = get_single(&drafts[0], field);
                let chosen = options
                    .iter()
                    .position(|o| fold(o) == fold(kept.trim()))
                    .unwrap_or(0);
                set_single(&mut draft, field, options[chosen].clone());
                conflicts.push(Conflict {
                    field,
                    options,
                    chosen,
                });
            }
        }
    }
    // the structured name: the kept card's, else the first that has one
    if draft.name == Default::default()
        && let Some(d) = drafts.iter().find(|d| d.name != Default::default())
    {
        draft.name = d.name.clone();
    }
    let email = |f: &Field| email_key(&f.value);
    let phone = |f: &Field| phone_tail(&f.value).unwrap_or_else(|| f.value.clone());
    let plain = |f: &Field| fold(f.value.trim());
    let place = |a: &Address| fold(&a.one_line());
    let word = |s: &String| fold(s.trim());
    let mut notes: Vec<String> = Vec::new();
    for d in &drafts {
        union(&mut draft.emails, &d.emails, email);
        union(&mut draft.phones, &d.phones, phone);
        union(&mut draft.addresses, &d.addresses, place);
        union(&mut draft.urls, &d.urls, plain);
        union(&mut draft.chats, &d.chats, plain);
        union(&mut draft.nicknames, &d.nicknames, word);
        union(&mut draft.categories, &d.categories, word);
        if !d.notes.trim().is_empty() && !notes.iter().any(|n| fold(n) == fold(d.notes.trim())) {
            notes.push(d.notes.trim().to_owned());
        }
    }
    draft.notes = notes.join("\n\n");
    Some(Merge {
        keep: cards[keep_at].0.clone(),
        others: order[1..].iter().map(|&i| cards[i].0.clone()).collect(),
        draft,
        conflicts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn books(cards: &[(&str, &str)]) -> (tempfile::TempDir, Books) {
        let dir = tempfile::tempdir().unwrap();
        let book = dir.path().join("contacts");
        std::fs::create_dir(&book).unwrap();
        for (name, body) in cards {
            std::fs::write(
                book.join(format!("{name}.vcf")),
                format!("BEGIN:VCARD\r\nVERSION:3.0\r\n{body}END:VCARD\r\n"),
            )
            .unwrap();
        }
        let b = Books::load(dir.path()).unwrap();
        (dir, b)
    }

    fn names(b: &Books, g: &Group) -> Vec<String> {
        let mut n: Vec<String> = g
            .cards
            .iter()
            .map(|id| id.path.file_stem().unwrap().to_string_lossy().into_owned())
            .collect();
        n.sort();
        let _ = b;
        n
    }

    #[test]
    fn finds_duplicates_by_email_phone_uid_and_name() {
        let (_d, b) = books(&[
            ("a", "UID:1\r\nFN:Priya Shah\r\nEMAIL:Priya@Example.com\r\n"),
            (
                "b",
                "UID:2\r\nFN:P. Shah\r\nEMAIL:priya@example.com\r\nTEL:+44 7700 900123\r\n",
            ),
            ("c", "UID:3\r\nFN:Priya (work)\r\nTEL:07700 900123\r\n"),
            ("d", "UID:4\r\nFN:Sam Lee\r\n"),
            ("e", "UID:5\r\nFN:SAM  LEE\r\n"),
            ("f", "UID:6\r\nFN:Mum\r\nEMAIL:mum@example.com\r\n"),
            ("g", "UID:7\r\nFN:Mum\r\n"),
            ("h", "UID:1\r\nFN:Copy of Priya\r\n"),
        ]);
        let groups = find(&b);
        // Priya: one email, then a phone written another way, then the
        // same UID in another file, all one group
        assert_eq!(names(&b, &groups[0]), ["a", "b", "c", "h"]);
        assert!(groups[0].is_strong());
        assert!(
            groups[0]
                .reasons
                .contains(&Reason::SameEmail("priya@example.com".into()))
        );
        assert!(
            groups[0]
                .reasons
                .contains(&Reason::SamePhone("700900123".into()))
        );
        assert!(groups[0].reasons.contains(&Reason::SameUid("1".into())));
        // Sam: only the name, so weaker and after
        assert_eq!(names(&b, &groups[1]), ["d", "e"]);
        assert!(!groups[1].is_strong());
        // two cards called "Mum" are not taken for the same person
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn merges_keep_everything_and_ask_about_conflicts() {
        let (dir, mut b) = books(&[
            (
                "a",
                "UID:1\r\nFN:Priya Shah\r\nN:Shah;Priya;;;\r\nEMAIL;TYPE=work:priya@example.com\r\n\
                 TEL:+44 7700 900123\r\nBDAY:1990-01-31\r\nNOTE:Met at the conference\r\n\
                 ORG:Northwind\r\n",
            ),
            (
                "b",
                "UID:2\r\nFN:Priya S\r\nEMAIL:PRIYA@example.com\r\nEMAIL:pri@home.example\r\n\
                 TEL:07700 900123\r\nBDAY:1990-01-30\r\nNOTE:Likes climbing\r\nNICKNAME:Pri\r\n",
            ),
        ]);
        let ids: Vec<CardId> = ["a", "b"]
            .iter()
            .map(|n| CardId {
                path: dir.path().join(format!("contacts/{n}.vcf")),
                index: 0,
            })
            .collect();
        let mut m = plan(&b, &ids).unwrap();
        // the richer card is kept
        assert_eq!(m.keep, ids[0]);
        assert_eq!(m.others, [ids[1].clone()]);
        // lists combined without repeats (case and formatting aside)
        let emails: Vec<&str> = m.draft.emails.iter().map(|e| e.value.as_str()).collect();
        assert_eq!(emails, ["priya@example.com", "pri@home.example"]);
        assert_eq!(m.draft.phones.len(), 1);
        assert_eq!(m.draft.nicknames, ["Pri"]);
        assert_eq!(m.draft.notes, "Met at the conference\n\nLikes climbing");
        // the organisation only one has is simply taken
        assert_eq!(m.draft.org, ["Northwind"]);
        // name and birthday disagree: listed, the kept card's chosen
        let fields: Vec<&str> = m.conflicts.iter().map(|c| c.field).collect();
        assert_eq!(fields, ["Name", "Birthday"]);
        assert_eq!(m.draft.birthday, "1990-01-31");
        m.choose(1, 1);
        assert_eq!(m.draft.birthday, "1990-01-30");

        // and it can be written, then the other deleted
        b.update(&m.keep, &m.draft).unwrap();
        b.delete(&m.others[0]).unwrap();
        let kept = b.get(&m.keep).unwrap();
        assert_eq!(kept.emails().len(), 2);
        assert_eq!(kept.birthday().as_deref(), Some("1990-01-30"));
        assert_eq!(b.len(), 1);
        assert!(find(&b).is_empty());
    }

    #[test]
    fn copies_in_other_books_and_common_names_are_not_sprawl() {
        let dir = tempfile::tempdir().unwrap();
        for book in ["account-a", "account-b"] {
            let p = dir.path().join(book);
            std::fs::create_dir(&p).unwrap();
            // the same card synced into two books
            std::fs::write(
                p.join("sam.vcf"),
                "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:sam\r\nFN:Sam Lee\r\nEMAIL:sam@example.com\r\nEND:VCARD\r\n",
            )
            .unwrap();
        }
        let a = dir.path().join("account-a");
        // two different people called Jo Park, one sharing a number with
        // a third card: the name mustn't pull all three together
        std::fs::write(a.join("jo1.vcf"), "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:j1\r\nFN:Jo Park\r\nTEL:0113 496 0001\r\nEND:VCARD\r\n").unwrap();
        std::fs::write(
            a.join("jo2.vcf"),
            "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:j2\r\nFN:Jo Park\r\nEND:VCARD\r\n",
        )
        .unwrap();
        std::fs::write(a.join("j3.vcf"), "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:j3\r\nFN:J. Park (work)\r\nTEL:+44 113 496 0001\r\nEND:VCARD\r\n").unwrap();
        let b = Books::load(dir.path()).unwrap();
        let groups = find(&b);
        let files = |g: &Group| {
            let mut f: Vec<String> = g
                .cards
                .iter()
                .map(|c| c.path.file_stem().unwrap().to_string_lossy().into_owned())
                .collect();
            f.sort();
            f
        };
        // the synced copies aren't offered; the shared number is
        assert_eq!(groups.len(), 1, "{groups:?}");
        assert_eq!(files(&groups[0]), ["j3", "jo1"]);
        assert!(groups[0].is_strong());
    }
}

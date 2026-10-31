//! Changing cards: from a [`Draft`] (the card as an editor shows it), only
//! what changed is rewritten. A value left alone keeps its line exactly,
//! parameters, Apple's label group and all; a changed one is written in
//! the card's own version (3.0's `TYPE=work,pref`, 4.0's `TYPE=work;PREF=1`).
//! New cards are 4.0. Every change is a [`Change`] that undo reverses.

use std::path::Path;

use anyhow::{Context as _, bail};
use rical_core::{
    doc::{Component, Document, Item, Line},
    write::{Change, write_atomic},
};

use crate::{
    book::{Books, CardId},
    model::{Address, Card, Field, Name, base_name},
    text::{escape, join_structured},
};

/// A card as an editor shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Draft {
    pub formatted_name: String,
    pub name: Name,
    pub nicknames: Vec<String>,
    /// The organisation, then units.
    pub org: Vec<String>,
    pub title: String,
    pub role: String,
    pub emails: Vec<Field>,
    pub phones: Vec<Field>,
    pub addresses: Vec<Address>,
    pub urls: Vec<Field>,
    pub chats: Vec<Field>,
    /// As written: "1990-01-31", or "--01-31" with no year.
    pub birthday: String,
    pub anniversary: String,
    /// Every NOTE, separated by blank lines.
    pub notes: String,
    pub categories: Vec<String>,
}

impl Draft {
    pub fn of(card: &Card) -> Self {
        Self {
            formatted_name: card.formatted_name().unwrap_or_default(),
            name: card.name().unwrap_or_default(),
            nicknames: card.nicknames(),
            org: card.org(),
            title: card.title().unwrap_or_default(),
            role: card.role().unwrap_or_default(),
            emails: card.emails(),
            phones: card.phones(),
            addresses: card.addresses(),
            urls: card.urls(),
            chats: card.chats(),
            birthday: card.birthday().unwrap_or_default(),
            anniversary: card.anniversary().unwrap_or_default(),
            notes: card.notes().join("\n\n"),
            categories: card.categories(),
        }
    }

    /// The formatted name to write: as given, else made from the name's
    /// parts, the organisation or an address (FN is required).
    fn formatted(&self) -> String {
        if !self.formatted_name.trim().is_empty() {
            return self.formatted_name.trim().to_owned();
        }
        let n = &self.name;
        let parts = [&n.prefixes, &n.given, &n.additional, &n.family, &n.suffixes]
            .into_iter()
            .map(|p| p.trim())
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if !parts.is_empty() {
            return parts;
        }
        self.org
            .first()
            .cloned()
            .or_else(|| self.emails.first().map(|e| e.value.clone()))
            .or_else(|| self.phones.first().map(|p| p.value.clone()))
            .unwrap_or_default()
    }
}

/// `YYYYMMDDTHHMMSSZ` now, for REV.
fn now_stamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

/// The lines of `c` with this base name (`item1.EMAIL` is EMAIL), by index
/// into its items.
fn lines_named(c: &Component, name: &str) -> Vec<usize> {
    c.items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| match item {
            Item::Line(l) if base_name(&l.name()) == name => Some(i),
            _ => None,
        })
        .collect()
}

/// `line` with a new value, keeping its name (and group) and parameters.
fn with_value(line: &Line, value: &str) -> Line {
    let content = line.content();
    let head = &content[..content.len() - line.value().len()];
    Line::new(format!("{head}{value}"))
}

/// Set a single-valued property: change the first's value (keeping its
/// parameters), add it, or with None remove every one.
fn set_single(c: &mut Component, name: &str, value: Option<String>) {
    let at = lines_named(c, name);
    match (value, at.first()) {
        (None, _) => {
            for i in at.into_iter().rev() {
                c.items.remove(i);
            }
        }
        (Some(v), Some(&i)) => {
            if let Item::Line(l) = &c.items[i]
                && l.value() != v
            {
                c.items[i] = Item::Line(with_value(l, &v));
            }
        }
        (Some(v), None) => c.insert(Line::new(format!("{name}:{v}"))),
    }
}

/// Labels and preference as parameters, in the card's version's style.
fn kind_params(version: &str, labels: &[String], pref: bool) -> Vec<(String, String)> {
    let mut types: Vec<String> = labels
        .iter()
        .map(|l| {
            if l == "mobile" {
                "cell".to_owned()
            } else {
                l.clone()
            }
        })
        .collect();
    let mut out = Vec::new();
    if version.starts_with('4') {
        if !types.is_empty() {
            out.push(("TYPE".to_owned(), types.join(",")));
        }
        if pref {
            out.push(("PREF".to_owned(), "1".to_owned()));
        }
    } else {
        if pref {
            types.push("pref".to_owned());
        }
        if !types.is_empty() {
            out.push(("TYPE".to_owned(), types.join(",")));
        }
    }
    out
}

fn new_line(name: &str, version: &str, value: &str, labels: &[String], pref: bool) -> Line {
    let params = kind_params(version, labels, pref);
    let refs = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect::<Vec<_>>();
    Line::property(name, &refs, value)
}

/// Bring a list property in line with `wanted`: entries as they were keep
/// their lines; changed ones are rewritten in place; new ones are added
/// after the others; removed ones go, with their Apple label line.
fn sync_list(
    c: &mut Component,
    name: &str,
    version: &str,
    before: &[Field],
    wanted: &[Field],
    encode: impl Fn(&str) -> String,
) {
    // the lines the `before` entries came from, in order (the model skips
    // empty values, so the same here)
    let lines: Vec<usize> = lines_named(c, name)
        .into_iter()
        .filter(|&i| matches!(&c.items[i], Item::Line(l) if !l.value().trim().is_empty()))
        .collect();
    let mut keep: Vec<Option<Line>> = Vec::new();
    let mut used = vec![false; before.len()];
    for w in wanted {
        if let Some(j) = (0..before.len()).find(|&j| !used[j] && before[j] == *w) {
            used[j] = true;
            if let Some(Item::Line(l)) = lines.get(j).map(|&i| &c.items[i]) {
                keep.push(Some(l.clone()));
                continue;
            }
        }
        keep.push(Some(new_line(
            name,
            version,
            &encode(&w.value),
            &w.labels,
            w.pref,
        )));
    }
    // groups of removed lines lose their label lines too
    let mut dropped_groups = Vec::new();
    for (j, &i) in lines.iter().enumerate() {
        if !used.get(j).copied().unwrap_or(false)
            && let Item::Line(l) = &c.items[i]
            && let Some((g, _)) = l.name().rsplit_once('.')
        {
            dropped_groups.push(g.to_owned());
        }
    }
    let first = lines.first().copied();
    for &i in lines.iter().rev() {
        c.items.remove(i);
    }
    c.items.retain(|item| match item {
        Item::Line(l) => {
            let n = l.name();
            !(base_name(&n) == "X-ABLABEL"
                && n.rsplit_once('.')
                    .is_some_and(|(g, _)| dropped_groups.iter().any(|d| d == g)))
        }
        _ => true,
    });
    // where they were (the first's place), else after the others
    let at = first.map(|f| f.min(c.items.len())).unwrap_or_else(|| {
        c.items
            .iter()
            .position(|i| matches!(i, Item::Component(_)))
            .unwrap_or(c.items.len())
    });
    c.items
        .splice(at..at, keep.into_iter().flatten().map(Item::Line));
}

/// Write `d` into the card `c` (a parsed VCARD), changing only what
/// differs from `before`.
fn apply_draft(c: &mut Component, before: &Draft, d: &Draft) {
    let version = c
        .property("VERSION")
        .map(|l| l.value().trim().to_owned())
        .unwrap_or_else(|| "4.0".to_owned());
    let text = |s: &str| Some(escape(s.trim())).filter(|v| !v.is_empty());

    if d.formatted() != before.formatted() || c.property("FN").is_none() {
        set_single(c, "FN", text(&d.formatted()));
    }
    if d.name != before.name || (version.starts_with('3') && c.property("N").is_none()) {
        let n = &d.name;
        let parts = [&n.family, &n.given, &n.additional, &n.prefixes, &n.suffixes]
            .map(|p| p.trim().to_owned());
        let any = parts.iter().any(|p| !p.is_empty());
        // 3.0 requires N; 4.0 doesn't
        let needed = version.starts_with('3') || version.starts_with('2');
        set_single(c, "N", (any || needed).then(|| join_structured(&parts)));
    }
    if d.nicknames != before.nicknames {
        let joined = d
            .nicknames
            .iter()
            .map(|n| escape(n.trim()))
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        set_single(c, "NICKNAME", Some(joined).filter(|v| !v.is_empty()));
    }
    if d.org != before.org {
        let org: Vec<String> = d.org.iter().map(|o| o.trim().to_owned()).collect();
        let any = org.iter().any(|o| !o.is_empty());
        set_single(c, "ORG", any.then(|| join_structured(&org)));
    }
    if d.title != before.title {
        set_single(c, "TITLE", text(&d.title));
    }
    if d.role != before.role {
        set_single(c, "ROLE", text(&d.role));
    }
    if d.birthday != before.birthday {
        set_single(
            c,
            "BDAY",
            Some(d.birthday.trim().to_owned()).filter(|v| !v.is_empty()),
        );
    }
    if d.anniversary != before.anniversary {
        set_single(
            c,
            "ANNIVERSARY",
            Some(d.anniversary.trim().to_owned()).filter(|v| !v.is_empty()),
        );
    }
    if d.notes != before.notes {
        // the notes as one
        for i in lines_named(c, "NOTE").into_iter().rev() {
            c.items.remove(i);
        }
        if let Some(n) = text(&d.notes) {
            c.insert(Line::new(format!("NOTE:{n}")));
        }
    }
    if d.categories != before.categories {
        let joined = d
            .categories
            .iter()
            .map(|n| escape(n.trim()))
            .filter(|n| !n.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        set_single(c, "CATEGORIES", Some(joined).filter(|v| !v.is_empty()));
    }
    let plain = |v: &str| v.trim().to_owned();
    if d.emails != before.emails {
        sync_list(c, "EMAIL", &version, &before.emails, &d.emails, plain);
    }
    if d.phones != before.phones {
        sync_list(c, "TEL", &version, &before.phones, &d.phones, plain);
    }
    if d.urls != before.urls {
        sync_list(c, "URL", &version, &before.urls, &d.urls, plain);
    }
    if d.chats != before.chats {
        sync_list(c, "IMPP", &version, &before.chats, &d.chats, plain);
    }
    if d.addresses != before.addresses {
        // as fields: the structured value, with the address's labels
        let as_field = |a: &Address| Field {
            value: join_structured(&[
                a.po_box.clone(),
                a.extended.clone(),
                a.street.clone(),
                a.locality.clone(),
                a.region.clone(),
                a.code.clone(),
                a.country.clone(),
            ]),
            labels: a.labels.clone(),
            pref: a.pref,
        };
        let b: Vec<Field> = before.addresses.iter().map(as_field).collect();
        let w: Vec<Field> = d.addresses.iter().map(as_field).collect();
        sync_list(c, "ADR", &version, &b, &w, |v| v.to_owned());
    }
    set_single(c, "REV", Some(now_stamp()));
}

fn read(path: &Path) -> anyhow::Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("reading {path:?}"))
}

impl Books {
    /// Save `draft` over the card `id`, changing only what differs.
    pub fn update(&mut self, id: &CardId, draft: &Draft) -> anyhow::Result<Change> {
        let card = self.get(id).context("that card is no longer there")?;
        if self.is_stale(&id.path) {
            bail!("{:?} changed on disk; reload and try again", id.path);
        }
        let before = Draft::of(&card.card());
        if id.index > 0 {
            bail!("{:?} holds several cards; edit it in another app", id.path);
        }
        let text = read(&id.path)?;
        let mut doc = Document::parse_card(&text)?;
        apply_draft(&mut doc.root, &before, draft);
        let change = Change {
            path: id.path.clone(),
            before: Some(text),
            after: Some(doc.write()),
        };
        self.apply(&change)?;
        Ok(change)
    }

    /// A new card in `book` (vCard 4.0), from `draft`.
    pub fn create(&mut self, book: usize, draft: &Draft) -> anyhow::Result<(Change, CardId)> {
        let dir = self
            .books
            .get(book)
            .context("no such address book")?
            .path
            .clone();
        let uid = uuid::Uuid::new_v4().to_string();
        let mut doc = Document::new_card(&uid);
        apply_draft(&mut doc.root, &Draft::default(), draft);
        let path = dir.join(format!("{uid}.vcf"));
        let change = Change {
            path: path.clone(),
            before: None,
            after: Some(doc.write()),
        };
        self.apply(&change)?;
        Ok((change, CardId { path, index: 0 }))
    }

    /// A copy of card `id` with a new UID, in a file of its own beside it
    /// (as it is, its version kept), to start another from.
    pub fn duplicate(&mut self, id: &CardId) -> anyhow::Result<(Change, CardId)> {
        if id.index > 0 {
            bail!("{:?} holds several cards; copy it in another app", id.path);
        }
        let mut doc = Document::parse_card(&read(&id.path)?)?;
        let uid = uuid::Uuid::new_v4().to_string();
        doc.root.set(Line::property("UID", &[], &uid));
        // Apple's own id would make the copy the same card to it
        doc.root.remove("X-ABUID");
        set_single(&mut doc.root, "REV", Some(now_stamp()));
        let dir = id.path.parent().context("the card has no folder")?;
        let path = dir.join(format!("{uid}.vcf"));
        let change = Change {
            path: path.clone(),
            before: None,
            after: Some(doc.write()),
        };
        self.apply(&change)?;
        Ok((change, CardId { path, index: 0 }))
    }

    /// Delete a card (its file, when it is the only card in it).
    pub fn delete(&mut self, id: &CardId) -> anyhow::Result<Change> {
        let text = read(&id.path)?;
        let (cards, _) = compact_ics::parse(&text)?;
        if cards.len() > 1 {
            bail!(
                "{:?} holds several cards; delete it in another app",
                id.path
            );
        }
        let change = Change {
            path: id.path.clone(),
            before: Some(text),
            after: None,
        };
        self.apply(&change)?;
        Ok(change)
    }

    /// Write a change, refusing if the file isn't as the change expects
    /// (vdirsyncer changed it, say), and checking what is written reads.
    pub fn apply(&mut self, change: &Change) -> anyhow::Result<()> {
        let current = std::fs::read_to_string(&change.path).ok();
        if current != change.before {
            bail!("{:?} changed on disk; reload and try again", change.path);
        }
        if let Some(after) = &change.after {
            compact_ics::parse(after)
                .with_context(|| format!("refusing to write invalid {:?}", change.path))?;
        }
        write_atomic(&change.path, change.after.as_deref())?;
        self.reload(&change.path);
        Ok(())
    }

    /// Undo a change made here, returning the change that redoes it.
    pub fn undo(&mut self, change: &Change) -> anyhow::Result<Change> {
        let inverse = change.inverse();
        self.apply(&inverse)?;
        Ok(inverse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const APPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nPRODID:-//Apple Inc.//iOS//EN\r\n\
        N:Shah;Priya;;;\r\nFN:Priya Shah\r\nORG:Northwind;\r\n\
        item1.EMAIL;type=INTERNET;type=pref:priya@example.com\r\n\
        item1.X-ABLabel:_$!<Work>!$_\r\n\
        item2.EMAIL;type=INTERNET:old@example.com\r\nitem2.X-ABLabel:Old\r\n\
        TEL;type=CELL;type=VOICE;type=pref:+44 7700 900123\r\n\
        X-SOCIALPROFILE;type=twitter:x-apple:priya\r\nUID:apple-1\r\n\
        END:VCARD\r\n";

    fn setup(files: &[(&str, &str)]) -> (tempfile::TempDir, Books) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("contacts")).unwrap();
        for (name, text) in files {
            std::fs::write(dir.path().join("contacts").join(name), text).unwrap();
        }
        let b = Books::load(dir.path()).unwrap();
        (dir, b)
    }

    fn id(dir: &tempfile::TempDir, name: &str) -> CardId {
        CardId {
            path: dir.path().join("contacts").join(name),
            index: 0,
        }
    }

    #[test]
    fn a_copy_keeps_the_card_with_a_new_uid() {
        let (dir, mut books) = setup(&[("p.vcf", APPLE)]);
        let original = id(&dir, "p.vcf");
        let uid = books
            .get(&original)
            .unwrap()
            .card()
            .uid()
            .unwrap()
            .to_owned();
        let (change, copy) = books.duplicate(&original).unwrap();
        let c = books.get(&copy).unwrap();
        let c = c.card();
        assert_eq!(c.display_name(), "Priya Shah");
        assert_eq!(c.emails().len(), 2);
        assert_eq!(c.version(), Some("3.0"));
        assert_ne!(c.uid(), Some(uid.as_str()));
        assert_eq!(books.by_uid(&uid), [original]);
        assert_eq!(books.len(), 2);
        books.undo(&change).unwrap();
        assert_eq!(books.len(), 1);
    }

    #[test]
    fn edits_change_only_what_changed() {
        let (dir, mut books) = setup(&[("p.vcf", APPLE)]);
        let id = id(&dir, "p.vcf");
        let mut d = Draft::of(&books.get(&id).unwrap().card());
        // a new title, a phone added, the old email gone
        d.title = "Lead, Platform".into();
        d.phones.push(Field {
            value: "0113 496 0000".into(),
            labels: vec!["home".into()],
            pref: false,
        });
        d.emails.retain(|e| e.value != "old@example.com");
        let change = books.update(&id, &d).unwrap();
        let after = change.after.as_deref().unwrap();
        // untouched lines are byte for byte as they were
        assert!(after.contains("item1.EMAIL;type=INTERNET;type=pref:priya@example.com\r\n"));
        assert!(after.contains("item1.X-ABLabel:_$!<Work>!$_\r\n"));
        assert!(after.contains("X-SOCIALPROFILE;type=twitter:x-apple:priya\r\n"));
        assert!(after.contains("PRODID:-//Apple Inc.//iOS//EN\r\n"));
        // the removed email and its label are gone
        assert!(!after.contains("old@example.com"), "{after}");
        assert!(!after.contains("item2.X-ABLabel"), "{after}");
        // new values in 3.0's style, escaped
        assert!(after.contains("TITLE:Lead\\, Platform\r\n"), "{after}");
        assert!(after.contains("TEL;TYPE=home:0113 496 0000\r\n"), "{after}");
        assert!(after.contains("\r\nREV:"));
        // and it reads back as edited
        let c = books.get(&id).unwrap();
        let c = c.card();
        assert_eq!(c.title().as_deref(), Some("Lead, Platform"));
        assert_eq!(c.phones().len(), 2);
        assert_eq!(c.emails().len(), 1);
        assert_eq!(c.emails()[0].labels, ["work"]);

        // undo puts the file back exactly
        books.undo(&change).unwrap();
        assert_eq!(std::fs::read_to_string(&id.path).unwrap(), APPLE);
    }

    #[test]
    fn new_cards_are_4_0() {
        let (_dir, mut books) = setup(&[]);
        let d = Draft {
            name: Name {
                family: "Lee".into(),
                given: "Sam".into(),
                ..Name::default()
            },
            emails: vec![Field {
                value: "sam@example.com".into(),
                labels: vec!["work".into()],
                pref: true,
            }],
            phones: vec![Field {
                value: "+44 7700 900456".into(),
                labels: vec!["mobile".into()],
                pref: false,
            }],
            ..Draft::default()
        };
        let (change, id) = books.create(0, &d).unwrap();
        let text = change.after.unwrap();
        assert!(text.starts_with("BEGIN:VCARD\r\nVERSION:4.0\r\n"), "{text}");
        assert!(text.contains("FN:Sam Lee\r\n"), "{text}");
        assert!(text.contains("N:Lee;Sam;;;\r\n"), "{text}");
        assert!(
            text.contains("EMAIL;TYPE=work;PREF=1:sam@example.com\r\n"),
            "{text}"
        );
        assert!(text.contains("TEL;TYPE=cell:+44 7700 900456\r\n"), "{text}");
        let c = books.get(&id).unwrap();
        let c = c.card();
        assert_eq!(c.display_name(), "Sam Lee");
        assert_eq!(books.by_uid(c.uid().unwrap()), [id]);
    }

    #[test]
    fn refuses_to_overwrite_a_change_made_elsewhere() {
        let (dir, mut books) = setup(&[("p.vcf", APPLE)]);
        let id = id(&dir, "p.vcf");
        let mut d = Draft::of(&books.get(&id).unwrap().card());
        std::fs::write(&id.path, APPLE.replace("Northwind", "Southwind")).unwrap();
        // a later modification time, as another program's write would have
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(&id.path)
            .unwrap()
            .set_modified(later)
            .unwrap();
        d.title = "x".into();
        let e = books.update(&id, &d).unwrap_err();
        assert!(e.to_string().contains("changed on disk"), "{e}");
        let del = books.delete(&id);
        // delete reads the file afresh, so it goes ahead
        assert!(del.is_ok());
    }
}

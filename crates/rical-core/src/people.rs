//! People to invite: everyone seen on events (as organiser or guest),
//! and contacts from a folder of vCards if there is one.

use std::{collections::BTreeMap, path::Path};

use crate::{Vdir, occurrence, write::Guest};

/// Someone known, and how often they have been on events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub guest: Guest,
    pub count: usize,
}

impl Vdir {
    /// Everyone on events in the vdir, most frequent first, leaving out
    /// `me`. Each address once, with the name it is most often given.
    pub fn people(&self, me: &[String]) -> Vec<Known> {
        let mut seen: BTreeMap<String, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
        for (_, component) in self.components() {
            for name in ["ORGANIZER", "ATTENDEE"] {
                for p in occurrence::people(component, name) {
                    let Some(email) = p.email.filter(|e| e.contains('@')) else {
                        continue;
                    };
                    let key = email.to_lowercase();
                    if me.iter().any(|m| m.eq_ignore_ascii_case(&key)) {
                        continue;
                    }
                    let (count, names) = seen.entry(key).or_default();
                    *count += 1;
                    if let Some(n) = p.name {
                        *names.entry(n).or_default() += 1;
                    }
                }
            }
        }
        let mut out = seen
            .into_iter()
            .map(|(email, (count, names))| Known {
                guest: Guest {
                    name: names.into_iter().max_by_key(|(_, n)| *n).map(|(n, _)| n),
                    email,
                },
                count,
            })
            .collect::<Vec<_>>();
        out.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then(a.guest.email.cmp(&b.guest.email))
        });
        out
    }
}

/// Contacts in a folder of `.vcf` files (searched recursively), e.g. a
/// vdirsyncer address book: one per address, named by `FN`.
pub fn contacts(dir: &Path) -> Vec<Guest> {
    let mut out = guests(&cards(dir));
    out.sort_by(|a, b| a.email.cmp(&b.email));
    out.dedup_by(|a, b| a.email == b.email);
    out
}

/// What the `.vcf` files under `dir` look like from outside: how many,
/// and the newest change. Cheap next to reading them, so a caller can
/// tell when to read them again.
pub fn contacts_fingerprint(dir: &Path) -> (usize, Option<std::time::SystemTime>) {
    let (mut count, mut newest) = (0, None);
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        // a folder's own time changes when files come or go
        if let Ok(t) = std::fs::metadata(&d).and_then(|m| m.modified()) {
            newest = newest.max(Some(t));
        }
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vcf")) {
                count += 1;
                if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
                    newest = newest.max(Some(t));
                }
            }
        }
    }
    (count, newest)
}

/// One contact from a vCard: what the person card shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Card {
    /// The `.vcf` file it is in.
    pub path: std::path::PathBuf,
    pub name: Option<String>,
    /// Addresses in lower case, each with its labels ("work").
    pub emails: Vec<(String, Vec<String>)>,
    pub phones: Vec<(String, Vec<String>)>,
    /// Postal addresses as one line each ("1 Park Row, Leeds, LS1 5HN").
    pub addresses: Vec<(String, Vec<String>)>,
    pub org: Option<String>,
    pub title: Option<String>,
    /// As written: "1990-01-31", "--01-31" (no year)…
    pub birthday: Option<String>,
    pub urls: Vec<String>,
    pub note: Option<String>,
}

/// Every contact in a folder of `.vcf` files (searched recursively).
pub fn cards(dir: &Path) -> Vec<Card> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("vcf"))
                && let Ok(text) = std::fs::read_to_string(&p)
            {
                out.extend(parse_cards(&text, &p));
            }
        }
    }
    out
}

/// The addresses in vCard text, each with its card's `FN`.
#[cfg(test)]
fn parse_vcards(text: &str) -> Vec<Guest> {
    guests(&parse_cards(text, Path::new("")))
}

/// One guest per address on `cards`, named by their card.
pub fn guests(cards: &[Card]) -> Vec<Guest> {
    cards
        .iter()
        .flat_map(|c| {
            c.emails.iter().map(|(email, _)| Guest {
                email: email.clone(),
                name: c.name.clone(),
            })
        })
        .collect()
}

/// The contacts in vCard text (vCard 2.1, 3.0 or 4.0).
fn parse_cards(text: &str, path: &Path) -> Vec<Card> {
    // unfold continuation lines first
    let unfolded = text
        .replace("\r\n ", "")
        .replace("\n ", "")
        .replace("\r\n\t", "");
    let mut out = Vec::new();
    let mut card = Card::default();
    for line in unfolded.lines() {
        let Some((head, value)) = line.split_once(':') else {
            continue;
        };
        // "item1.EMAIL;TYPE=work" → "EMAIL", with its labels
        let mut parts = head.split(';');
        let prop = parts.next().unwrap_or("");
        let prop = prop.rsplit('.').next().unwrap_or(prop).to_ascii_uppercase();
        let labels = labels(parts);
        let value = value.trim();
        let text = || Some(unescape(value)).filter(|v| !v.is_empty());
        match prop.as_str() {
            "BEGIN" => {
                card = Card {
                    path: path.to_owned(),
                    ..Card::default()
                };
            }
            "FN" => card.name = text(),
            "EMAIL" => {
                let e = value.to_lowercase();
                if e.contains('@') {
                    card.emails.push((e, labels));
                }
            }
            "TEL" => {
                // vCard 4 may write it as a URI
                let number = value.strip_prefix("tel:").unwrap_or(value).trim();
                if !number.is_empty() {
                    card.phones.push((number.to_owned(), labels));
                }
            }
            "ADR" => {
                // PO box; extended; street; locality; region; code; country
                let address = split_unescaped(value)
                    .into_iter()
                    .map(|p| p.trim().replace('\n', ", "))
                    .filter(|p| !p.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                if !address.is_empty() {
                    card.addresses.push((address, labels));
                }
            }
            "ORG" => {
                card.org = Some(
                    split_unescaped(value)
                        .into_iter()
                        .filter(|p| !p.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(", "),
                )
                .filter(|o| !o.is_empty())
            }
            "TITLE" => card.title = text(),
            "BDAY" => card.birthday = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "URL" => card.urls.push(value.to_owned()),
            "NOTE" => card.note = text(),
            "END" => out.push(std::mem::take(&mut card)),
            _ => {}
        }
    }
    out
}

/// The kinds a property is labelled with ("TYPE=work,voice" or 2.1's bare
/// "CELL"), leaving out ones that say nothing to a reader.
fn labels<'a>(params: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out = Vec::new();
    for p in params {
        let values = match p.split_once('=') {
            Some((k, v)) if k.eq_ignore_ascii_case("TYPE") => v,
            Some(_) => continue,
            None => p,
        };
        for v in values.trim_matches('"').split(',') {
            let v = v.trim().to_lowercase();
            // "cell" is the vCard word; people say mobile
            let v = if v == "cell" { "mobile".to_owned() } else { v };
            if !v.is_empty()
                && !["pref", "voice", "internet", "x400", "other"].contains(&v.as_str())
                && !out.contains(&v)
            {
                out.push(v);
            }
        }
    }
    out
}

/// A structured value split at `;` that is not escaped, each part
/// unescaped.
fn split_unescaped(value: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                parts.last_mut().unwrap().push('\\');
                if let Some(next) = chars.next() {
                    parts.last_mut().unwrap().push(next);
                }
            }
            ';' => parts.push(String::new()),
            c => parts.last_mut().unwrap().push(c),
        }
    }
    parts.iter().map(|p| unescape(p)).collect()
}

/// The best matches for what was typed: by name or address, words that
/// start with it first, then the most frequent. `known` is in frequency
/// order.
pub fn suggest<'a>(
    known: &'a [Known],
    typed: &str,
    skip: &[String],
    limit: usize,
) -> Vec<&'a Known> {
    let q = typed.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits = known
        .iter()
        .filter(|k| !skip.iter().any(|s| s.eq_ignore_ascii_case(&k.guest.email)))
        .filter_map(|k| {
            let name = k.guest.name.as_deref().unwrap_or("").to_lowercase();
            let email = k.guest.email.to_lowercase();
            let starts =
                name.split_whitespace().any(|w| w.starts_with(&q)) || email.starts_with(&q);
            (starts || name.contains(&q) || email.contains(&q)).then_some((!starts, k))
        })
        .collect::<Vec<_>>();
    // stable: frequency order is kept within each group
    hits.sort_by_key(|(later, _)| *later);
    hits.into_iter().take(limit).map(|(_, k)| k).collect()
}

/// A vCard text value as written: `\,` `\;` `\\` and `\n` undone.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(name: &str, email: &str, count: usize) -> Known {
        Known {
            guest: Guest {
                email: email.into(),
                name: (!name.is_empty()).then(|| name.into()),
            },
            count,
        }
    }

    #[test]
    fn suggestions_prefer_word_starts() {
        let all = [
            known("Sam Jones", "sam@example.com", 9),
            known("Charlene Tang", "char@example.com", 5),
            known("Rosamund Pike", "rp@example.com", 3),
        ];
        let names = |q: &str| {
            suggest(&all, q, &[], 5)
                .iter()
                .map(|k| k.guest.email.clone())
                .collect::<Vec<_>>()
        };
        // "sam" starts Sam's name, and is inside Rosamund's
        assert_eq!(names("sam"), ["sam@example.com", "rp@example.com"]);
        assert_eq!(names("tang"), ["char@example.com"]);
        assert_eq!(names("CHAR@"), ["char@example.com"]);
        assert!(names("").is_empty());
        // already added
        assert!(
            suggest(&all, "sam", &["SAM@example.com".into()], 5)
                .iter()
                .all(|k| k.guest.email != "sam@example.com")
        );
    }

    #[test]
    fn reads_vcards() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Sam Jones\r\nEMAIL;TYPE=work:Sam@Example.com\r\n\
                    item1.EMAIL:sam@home.example\r\nEND:VCARD\r\n\
                    BEGIN:VCARD\r\nFN:No Mail\r\nEND:VCARD\r\n";
        assert_eq!(
            parse_vcards(text),
            [
                Guest {
                    email: "sam@example.com".into(),
                    name: Some("Sam Jones".into())
                },
                Guest {
                    email: "sam@home.example".into(),
                    name: Some("Sam Jones".into())
                },
            ]
        );
    }

    #[test]
    fn vcard_names_are_unescaped() {
        let text =
            "BEGIN:VCARD\r\nFN:Lee\\, Sam\\; Jr\\\\\r\nEMAIL:sam@example.com\r\nEND:VCARD\r\n";
        assert_eq!(
            parse_vcards(text)[0].name.as_deref(),
            Some("Lee, Sam; Jr\\")
        );
    }

    #[test]
    fn the_fingerprint_changes_with_the_files() {
        let dir = tempfile::tempdir().unwrap();
        let card = |name: &str| format!("BEGIN:VCARD\r\nFN:{name}\r\nEMAIL:a@b.c\r\nEND:VCARD\r\n");
        std::fs::write(dir.path().join("a.vcf"), card("A")).unwrap();
        let before = contacts_fingerprint(dir.path());
        assert_eq!(before.0, 1);
        std::fs::create_dir(dir.path().join("book")).unwrap();
        std::fs::write(dir.path().join("book/b.vcf"), card("B")).unwrap();
        let after = contacts_fingerprint(dir.path());
        assert_eq!(after.0, 2);
        assert_ne!(before, after);
    }

    #[test]
    fn reads_a_whole_card() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Priya Shah\r\nORG:Acme\\, Inc;Platform\r\n\
                    TITLE:Engineering lead\r\nEMAIL;TYPE=INTERNET,WORK:Priya@Example.com\r\n\
                    TEL;TYPE=CELL,VOICE:+44 7700 900123\r\nTEL;HOME:0113 496 0000\r\n\
                    item1.ADR;TYPE=home:;;1 Park Row;Leeds;;LS1 5HN;UK\r\nBDAY:--0131\r\n\
                    URL:https://priya.example\r\nEND:VCARD\r\n";
        let card = &parse_cards(text, Path::new("p.vcf"))[0];
        assert_eq!(card.name.as_deref(), Some("Priya Shah"));
        assert_eq!(card.org.as_deref(), Some("Acme, Inc, Platform"));
        assert_eq!(card.title.as_deref(), Some("Engineering lead"));
        assert_eq!(
            card.emails,
            [("priya@example.com".into(), vec!["work".into()])]
        );
        assert_eq!(
            card.phones,
            [
                ("+44 7700 900123".into(), vec!["mobile".into()]),
                ("0113 496 0000".into(), vec!["home".into()])
            ]
        );
        assert_eq!(
            card.addresses,
            [("1 Park Row, Leeds, LS1 5HN, UK".into(), vec!["home".into()])]
        );
        assert_eq!(card.birthday.as_deref(), Some("--0131"));
        assert_eq!(card.urls, ["https://priya.example"]);
        assert_eq!(card.path, Path::new("p.vcf"));
    }
}

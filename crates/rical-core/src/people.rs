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
                out.extend(parse_vcards(&text));
            }
        }
    }
    out.sort_by(|a, b| a.email.cmp(&b.email));
    out.dedup_by(|a, b| a.email == b.email);
    out
}

/// The addresses in vCard text, each with its card's `FN`.
fn parse_vcards(text: &str) -> Vec<Guest> {
    // unfold continuation lines first
    let unfolded = text
        .replace("\r\n ", "")
        .replace("\n ", "")
        .replace("\r\n\t", "");
    let mut out = Vec::new();
    let mut name = None;
    let mut emails = Vec::new();
    for line in unfolded.lines() {
        let Some((head, value)) = line.split_once(':') else {
            continue;
        };
        // "item1.EMAIL;TYPE=work" → "EMAIL"
        let prop = head.split(';').next().unwrap_or("");
        let prop = prop.rsplit('.').next().unwrap_or(prop).to_ascii_uppercase();
        match prop.as_str() {
            "BEGIN" => {
                name = None;
                emails.clear();
            }
            "FN" => name = Some(value.trim().replace("\\,", ",")).filter(|n| !n.is_empty()),
            "EMAIL" => {
                let e = value.trim().to_lowercase();
                if e.contains('@') {
                    emails.push(e);
                }
            }
            "END" => {
                for email in emails.drain(..) {
                    out.push(Guest {
                        email,
                        name: name.clone(),
                    });
                }
            }
            _ => {}
        }
    }
    out
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
}

//! People to invite: everyone seen on events (as organiser or guest).
//! Contacts themselves are read by `rical-cards`.

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
}

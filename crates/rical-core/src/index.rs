//! What the index of a vdir keeps of each item (see [`rical_index`]): when
//! it can show, what kinds it holds, its UIDs, the people on it, the time
//! zones it defines, and the words a search looks in. Never the item
//! itself, which is read from its file when it is shown.

use std::path::Path;

use rical_index::rusqlite::{self, Transaction, params};

use crate::{
    component::AnyComponent,
    occurrence::{self, Kind, unescape_text},
};

/// The bit of each kind in `items.kinds`.
pub fn kind_bit(kind: Kind) -> i64 {
    match kind {
        Kind::Event => 1,
        Kind::Todo => 2,
        Kind::Journal => 4,
    }
}

/// Someone on an item: as organiser (`role` 0) or attendee (1) of its
/// `part`th component.
#[derive(Debug, Clone)]
pub struct PersonRow {
    pub part: i64,
    pub role: i64,
    pub email: Option<String>,
    pub name: Option<String>,
    /// Whether the component was scheduled by the server (see
    /// [`occurrence::server_scheduled`]).
    pub scheduled: bool,
}

pub struct Row {
    span: Option<(i64, i64)>,
    kinds: i64,
    uids: Vec<String>,
    people: Vec<PersonRow>,
    tzids: Vec<String>,
    search: String,
}

/// The schema of a vdir's index.
pub struct Items;

impl Items {
    /// What is kept of an item already parsed.
    pub fn row(components: &[AnyComponent], tzids: Vec<String>) -> Row {
        let mut kinds = 0;
        let mut uids: Vec<String> = Vec::new();
        let mut people = Vec::new();
        let mut search = String::new();
        for (part, c) in components.iter().enumerate() {
            kinds |= match c.component_kind() {
                "VEVENT" => kind_bit(Kind::Event),
                "VTODO" => kind_bit(Kind::Todo),
                "VJOURNAL" => kind_bit(Kind::Journal),
                _ => 0,
            };
            if let Some(uid) = c.get_uid()
                && !uids.iter().any(|u| u == uid)
            {
                uids.push(uid.to_owned());
            }
            // as a search matches them: unescaped, in lower case
            for name in ["SUMMARY", "LOCATION", "DESCRIPTION"] {
                for p in c.props(name) {
                    search.push_str(&unescape_text(p.value()).to_lowercase());
                    search.push('\n');
                }
            }
            let scheduled = occurrence::server_scheduled(c);
            for (role, name) in [(0, "ORGANIZER"), (1, "ATTENDEE")] {
                for p in occurrence::people(c, name) {
                    for t in p.name.iter().chain(&p.email) {
                        search.push_str(&t.to_lowercase());
                        search.push('\n');
                    }
                    people.push(PersonRow {
                        part: part as i64,
                        role,
                        email: p.email,
                        name: p.name,
                        scheduled,
                    });
                }
            }
        }
        Row {
            span: occurrence::span(components),
            kinds,
            uids,
            people,
            tzids,
            search,
        }
    }
}

impl rical_index::Schema for Items {
    type Row = Row;
    const VERSION: i64 = 3;
    const TABLES: &'static str = "
        CREATE TABLE items (
            file INTEGER PRIMARY KEY,
            kinds INTEGER NOT NULL,
            -- 0: it can show at any time (see `spans` otherwise)
            bounded INTEGER NOT NULL,
            search TEXT NOT NULL,
            -- who is on it, a line each (see `PersonRow::line`)
            people TEXT NOT NULL,
            -- the TZIDs it has a VTIMEZONE for, each on a line of its own
            tzids TEXT NOT NULL
        );
        -- those that can show at any time, which every query reads
        CREATE INDEX items_open ON items (file) WHERE bounded = 0;
        -- when each bounded item can show, as Unix times
        CREATE VIRTUAL TABLE spans USING rtree (id, lo, hi);
        CREATE TABLE uids (file INTEGER NOT NULL, uid TEXT NOT NULL);
        CREATE INDEX uids_uid ON uids (uid);
        CREATE INDEX uids_file ON uids (file);
    ";
    const FORGET: &'static [&'static str] = &[
        "DELETE FROM items WHERE file = ?1",
        "DELETE FROM spans WHERE id = ?1",
        "DELETE FROM uids WHERE file = ?1",
    ];

    fn extract(&self, path: &Path) -> Result<Row, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let (components, tzids) = crate::compact::parse(&text).map_err(|e| e.to_string())?;
        let shared: std::sync::Arc<Path> = path.into();
        let components: Vec<AnyComponent> = components
            .into_iter()
            .map(|c| AnyComponent::new(shared.clone(), c))
            .collect();
        Ok(Self::row(&components, tzids))
    }

    fn store(&self, tx: &Transaction, file: i64, row: &Row) -> rusqlite::Result<()> {
        let people: String = row.people.iter().map(PersonRow::line).collect();
        let tzids: String = row.tzids.iter().map(|t| format!("\n{t}\n")).collect();
        tx.prepare_cached(
            "INSERT INTO items (file, kinds, bounded, search, people, tzids)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?
        .execute(params![
            file,
            row.kinds,
            row.span.is_some(),
            row.search,
            people,
            tzids
        ])?;
        // an item with no dates (a task without any, say) never shows in a
        // range: it has no span at all
        if let Some((lo, hi)) = row.span.filter(|(lo, hi)| lo <= hi) {
            tx.prepare_cached("INSERT INTO spans (id, lo, hi) VALUES (?1, ?2, ?3)")?
                .execute(params![file, lo as f64, hi as f64])?;
        }
        let mut uid = tx.prepare_cached("INSERT INTO uids (file, uid) VALUES (?1, ?2)")?;
        for u in &row.uids {
            uid.execute(params![file, u])?;
        }
        Ok(())
    }
}

impl PersonRow {
    /// One line: part, role, scheduled, email and name, split by tabs.
    fn line(&self) -> String {
        let clean = |s: &Option<String>| s.as_deref().unwrap_or("").replace(['\t', '\n'], " ");
        format!(
            "{}\t{}\t{}\t{}\t{}\n",
            self.part,
            self.role,
            u8::from(self.scheduled),
            clean(&self.email),
            clean(&self.name)
        )
    }

    /// The people of [`PersonRow::line`]s.
    pub fn parse(lines: &str) -> impl Iterator<Item = PersonRow> + '_ {
        lines.lines().filter_map(|l| {
            let mut f = l.split('\t');
            let some = |s: &str| (!s.is_empty()).then(|| s.to_owned());
            Some(PersonRow {
                part: f.next()?.parse().ok()?,
                role: f.next()?.parse().ok()?,
                scheduled: f.next()? == "1",
                email: some(f.next()?),
                name: some(f.next()?),
            })
        })
    }
}

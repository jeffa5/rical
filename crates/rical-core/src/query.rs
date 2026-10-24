use std::{collections::BTreeMap, path::PathBuf, str::FromStr};

use chrono::{DateTime, NaiveDateTime, Utc};
use icalendar::Component as _;

use crate::{
    component::AnyComponent, files::paths_to_files, ics::read_ics_to_calendar, short_id,
    time::date_perhaps_time_to_datetime_utc,
};

#[derive(Debug, Clone)]
pub enum ListKind {
    Event,
    Todo,
    Journal,
    All,
}

impl ListKind {
    pub fn matches(&self, other: &str) -> bool {
        match self {
            ListKind::Event => other == "VEVENT",
            ListKind::Todo => other == "VTODO",
            ListKind::Journal => other == "VJOURNAL",
            ListKind::All => true,
        }
    }
}

impl FromStr for ListKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "all" => Ok(Self::All),
            "event" => Ok(Self::Event),
            "todo" => Ok(Self::Todo),
            "journal" => Ok(Self::Journal),
            _ => Err(format!("Unknown list kind {s:?}")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ListQuery {
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    pub limit: usize,
    pub from: Option<NaiveDateTime>,
    pub to: Option<NaiveDateTime>,
    pub kinds: Vec<ListKind>,
    pub short_id: Option<String>,
}

pub struct Listing {
    /// Matching components, limited and sorted by start time.
    pub components: Vec<AnyComponent>,
    /// UIDs of every matching component, before the limit was applied.
    all_ids: Vec<String>,
}

impl Listing {
    /// Shortest unique UID prefixes, computed over every matching component.
    pub fn short_ids(&self) -> BTreeMap<String, String> {
        short_id::build_prefixes_map(&mut self.all_ids.clone())
    }
}

pub fn list(query: &ListQuery) -> Listing {
    let paths = paths_to_files(&query.paths, query.recursive);

    let components = paths
        .into_iter()
        .filter_map(|p| read_ics_to_calendar(&p).ok().map(|c| (p, c)))
        .flat_map(|(p, cal)| {
            cal.components
                .into_iter()
                .map(|c| AnyComponent::new(p.clone(), c))
                .filter(|c| {
                    query
                        .short_id
                        .as_ref()
                        .is_none_or(|sid| c.get_uid().is_some_and(|uid| uid.starts_with(sid)))
                })
                .filter(|c| query.kinds.iter().any(|k| k.matches(&c.component_kind())))
                .filter(|c| {
                    query.from.is_none_or(|from| {
                        c.get_start().is_some_and(|ndt| {
                            date_perhaps_time_to_datetime_utc(ndt) >= from.and_utc()
                        })
                    })
                })
                .filter(|c| {
                    query.to.is_none_or(|to| {
                        c.get_end().is_some_and(|ndt| {
                            date_perhaps_time_to_datetime_utc(ndt) <= to.and_utc()
                        })
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let all_ids = components
        .iter()
        .filter_map(|c| c.get_uid())
        .map(|u| u.to_owned())
        .collect::<Vec<_>>();

    let mut components = components.into_iter().take(query.limit).collect::<Vec<_>>();
    components.sort_by_key(|e| {
        e.get_start()
            .map(date_perhaps_time_to_datetime_utc)
            .unwrap_or_else(|| {
                e.get_created()
                    .unwrap_or_else(|| e.get_timestamp().unwrap_or(DateTime::<Utc>::MIN_UTC))
            })
    });

    Listing {
        components,
        all_ids,
    }
}

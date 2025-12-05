use std::{collections::BTreeMap, fs::read_dir, path::PathBuf, str::FromStr};

use chrono::{DateTime, NaiveDateTime, NaiveTime, Utc};
use icalendar::{Component as _, DatePerhapsTime};
use log::warn;

use crate::{
    calendar::read_ics_to_calendar,
    component::{AnyComponent, PrettyComponent},
    format::OutputFormat,
};

#[derive(Debug, Clone)]
pub enum ListKind {
    Event,
    Todo,
    Journal,
    All,
}

impl ListKind {
    fn matches(&self, other: &str) -> bool {
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

pub struct ListArgs {
    pub paths: Vec<PathBuf>,
    pub format: OutputFormat,
    pub raw: bool,
    pub limit: usize,
    pub from: Option<NaiveDateTime>,
    pub to: Option<NaiveDateTime>,
    pub kinds: Vec<ListKind>,
    pub recursive: bool,
    pub short_id: Option<String>,
}

pub fn list(args: &ListArgs) -> anyhow::Result<()> {
    let paths = paths_to_files(&args.paths, args.recursive);

    let components = paths
        .into_iter()
        .filter_map(|p| read_ics_to_calendar(&p).ok().map(|c| (p, c)))
        .flat_map(|(p, cal)| {
            cal.components
                .into_iter()
                .map(|c| AnyComponent::new(p.clone(), c))
                .filter(|c| {
                    args.short_id
                        .as_ref()
                        .is_none_or(|sid| c.get_uid().is_some_and(|uid| uid.starts_with(sid)))
                })
                .filter(|c| args.kinds.iter().any(|k| k.matches(&c.component_kind())))
                .filter(|c| {
                    args.from.is_none_or(|from| {
                        c.get_start().is_some_and(|ndt| {
                            date_perhaps_time_to_datetime_utc(ndt) >= from.and_utc()
                        })
                    })
                })
                .filter(|c| {
                    args.to.is_none_or(|to| {
                        c.get_end().is_some_and(|ndt| {
                            date_perhaps_time_to_datetime_utc(ndt) <= to.and_utc()
                        })
                    })
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let mut all_ids = components
        .iter()
        .filter_map(|c| c.get_uid())
        .map(|u| u.to_owned())
        .collect::<Vec<_>>();

    let mut components = components.into_iter().take(args.limit).collect::<Vec<_>>();
    components.sort_by_key(|e| {
        e.get_start()
            .map(date_perhaps_time_to_datetime_utc)
            .unwrap_or_else(|| {
                e.get_created()
                    .unwrap_or_else(|| e.get_timestamp().unwrap_or(DateTime::<Utc>::MIN_UTC))
            })
    });

    let s = if args.raw {
        args.format.render(&components)?
    } else {
        let short_ids = build_prefixes_map(&mut all_ids);

        args.format.render(
            &components
                .into_iter()
                .map(|c| {
                    let short_id = c.get_uid().and_then(|u| short_ids.get(u).cloned());
                    PrettyComponent::new(c, short_id)
                })
                .collect::<Vec<_>>(),
        )?
    };
    println!("{s}");

    Ok(())
}

fn build_prefixes_map(all_ids: &mut [String]) -> BTreeMap<String, String> {
    let prefixes = build_prefixes(all_ids);
    all_ids.iter().cloned().zip(prefixes).collect()
}

fn build_prefixes(all_ids: &mut [String]) -> Vec<String> {
    if all_ids.is_empty() {
        return Vec::new();
    }

    all_ids.sort_unstable();

    let mut prefixes = Vec::new();

    prefixes.push(find_prefix(
        "",
        &all_ids[0],
        all_ids.get(1).map_or("", |s| s.as_str()),
    ));

    for window in all_ids.windows(3) {
        let [u1, u2, u3] = window else {
            panic!("window length not what was expected");
        };
        prefixes.push(find_prefix(u1, u2, u3));
    }

    let len = all_ids.len();
    prefixes.push(find_prefix(&all_ids[len - 2], &all_ids[len - 1], ""));

    assert_eq!(
        all_ids.len(),
        prefixes.len(),
        "should have the same number of prefixes as original ids"
    );

    prefixes
}

fn find_prefix(u1: &str, u2: &str, u3: &str) -> String {
    let mut before_finished = false;
    let mut after_finished = false;
    let mut matched = false;
    let prefix = u1
        .chars()
        .map(Some)
        .chain(std::iter::repeat(None))
        .zip(u2.chars().map(Some))
        .zip(u3.chars().map(Some).chain(std::iter::repeat(None)))
        .take_while(|((c1, c2), c3)| {
            before_finished |= c1 != c2;
            after_finished |= c3 != c2;
            if !before_finished || !after_finished {
                true
            } else {
                // get one more past what matches
                matched = !matched;
                matched
            }
        })
        .map(|((_, c2), _)| c2.unwrap())
        .collect::<String>();
    assert!(!prefix.is_empty());
    prefix
}

pub(crate) fn paths_to_files(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    paths
        .iter()
        .flat_map(|path| {
            if path.is_file() {
                vec![path.clone()]
            } else if path.is_dir() {
                read_dir(path)
                    .map(|rd| {
                        rd.into_iter()
                            .filter_map(|e| e.ok())
                            .flat_map(|e| {
                                e.metadata()
                                    .map(|m| {
                                        if m.is_file() {
                                            vec![e.path()]
                                        } else if m.is_dir() && recursive {
                                            paths_to_files(&[e.path()], recursive)
                                        } else {
                                            Vec::new()
                                        }
                                    })
                                    .unwrap_or_default()
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default()
            } else {
                warn!("Path was neither file or directory: {:?}", path.metadata());
                Vec::new()
            }
        })
        .collect()
}

pub(crate) fn date_perhaps_time_to_datetime_utc(dpt: DatePerhapsTime) -> chrono::DateTime<Utc> {
    match dpt {
        icalendar::DatePerhapsTime::DateTime(calendar_date_time) => calendar_date_time
            .try_into_utc()
            .unwrap_or_else(|| match calendar_date_time {
                icalendar::CalendarDateTime::Floating(naive_date_time) => naive_date_time.and_utc(),
                icalendar::CalendarDateTime::Utc(..) => todo!(),
                icalendar::CalendarDateTime::WithTimezone { .. } => todo!(),
            }),
        icalendar::DatePerhapsTime::Date(naive_date) => naive_date
            .and_time(NaiveTime::from_hms_opt(0, 0, 0).unwrap())
            .and_utc(),
    }
}

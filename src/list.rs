use std::{fs::read_dir, path::PathBuf, str::FromStr};

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
}

pub fn list(args: &ListArgs) -> anyhow::Result<()> {
    let paths = paths_to_files(&args.paths, args.recursive);

    let mut components = paths
        .into_iter()
        .filter_map(|p| read_ics_to_calendar(&p).ok().map(|c| (p, c)))
        .flat_map(|(p, cal)| {
            cal.components
                .into_iter()
                .map(|c| AnyComponent::new(p.clone(), c))
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
        .take(args.limit)
        .collect::<Vec<_>>();

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
        args.format.render(
            &components
                .into_iter()
                .map(PrettyComponent::from)
                .collect::<Vec<_>>(),
        )?
    };
    println!("{s}");

    Ok(())
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

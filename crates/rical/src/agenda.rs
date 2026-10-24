use std::path::PathBuf;

use chrono::{Days, NaiveDate, NaiveTime};
use rical_core::{
    Kind, Occurrence, Vdir, When,
    when::{local_timezone, resolve_local},
};
use serde::Serialize;

use crate::format::{Formatable, OutputFormat};

pub struct AgendaArgs {
    pub vdir: PathBuf,
    pub from: Option<NaiveDate>,
    pub days: u64,
    pub kinds: Vec<Kind>,
    pub format: OutputFormat,
}

#[derive(Debug, Serialize)]
struct AgendaRow {
    calendar: String,
    kind: Kind,
    start: String,
    end: String,
    summary: String,
    location: String,
    recurring: bool,
}

impl Formatable for AgendaRow {
    fn keys(&self) -> Vec<String> {
        ["start", "end", "calendar", "summary", "location"]
            .map(String::from)
            .to_vec()
    }
}

pub fn agenda(args: &AgendaArgs) -> anyhow::Result<()> {
    let local = local_timezone();
    let vdir = Vdir::load(&args.vdir)?;

    let from = args
        .from
        .unwrap_or_else(|| chrono::Utc::now().with_timezone(&local).date_naive());
    let to = from + Days::new(args.days);
    let at_midnight = |d: NaiveDate| resolve_local(local, d.and_time(NaiveTime::MIN)).to_utc();

    let occurrences = vdir.query(&(at_midnight(from)..at_midnight(to)), &args.kinds, local);

    let fmt = |w: &When| match w {
        When::Date(d) => d.format("%Y-%m-%d").to_string(),
        _ => w.to_tz(local).format("%Y-%m-%d %H:%M").to_string(),
    };
    let rows = occurrences
        .iter()
        .map(|o: &Occurrence| AgendaRow {
            calendar: vdir
                .calendar(&o.calendar)
                .map_or(o.calendar.clone(), |c| c.name.clone()),
            kind: o.kind,
            start: fmt(&o.start),
            end: fmt(&o.end),
            summary: o.summary.clone(),
            location: o.location.clone().unwrap_or_default(),
            recurring: o.recurring,
        })
        .collect::<Vec<_>>();

    println!("{}", args.format.render(&rows)?);
    Ok(())
}

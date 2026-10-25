//! Changing a vdir from the command line: new, done, delete and edit.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context as _, bail};
use chrono::{NaiveTime, TimeDelta, Timelike, Utc};
use rical_core::{
    Kind, Occurrence, Vdir, When,
    input::{parse_at, parse_date, parse_shift},
    when::{local_timezone, resolve_local},
    write::Scope,
};

pub struct NewArgs {
    pub vdir: PathBuf,
    pub calendar: Option<String>,
    pub at: Option<String>,
    pub length: Option<String>,
    pub location: Option<String>,
    pub todo: bool,
    pub due: Option<String>,
    pub title: String,
}

/// A calendar by folder name, display name or number (1-based); default
/// the one with the most items, birthday calendars aside.
fn find_calendar(vdir: &Vdir, name: Option<&str>) -> anyhow::Result<String> {
    let cals = &vdir.calendars;
    if let Some(name) = name {
        return name
            .parse::<usize>()
            .ok()
            .and_then(|n| cals.get(n.wrapping_sub(1)))
            .or_else(|| cals.iter().find(|c| c.id == name))
            .or_else(|| cals.iter().find(|c| c.name.eq_ignore_ascii_case(name)))
            .map(|c| c.id.clone())
            .with_context(|| format!("no calendar {name:?}"));
    }
    let count = |id: &str| vdir.components().filter(|(c, _)| c.id == id).count();
    cals.iter()
        .filter(|c| !c.id.contains("birthday"))
        .max_by_key(|c| count(&c.id))
        .or(cals.first())
        .map(|c| c.id.clone())
        .context("the vdir has no calendars")
}

pub fn new(args: &NewArgs) -> anyhow::Result<()> {
    let mut vdir = Vdir::load(&args.vdir)?;
    let local = local_timezone();
    let today = Utc::now().with_timezone(&local).date_naive();
    // the title is read in plain language ("Lunch tomorrow 1pm @ Bistro
    // 21", see rical_core::quick); flags win over what it says
    let q = rical_core::quick::parse(&args.title, today, today, local, args.todo);
    let calendar = find_calendar(&vdir, args.calendar.as_deref().or(q.calendar.as_deref()))?;
    // the next full hour
    let now = Utc::now().with_timezone(&local);
    let hour = NaiveTime::from_hms_opt((now.hour() + 1).min(23), 0, 0).unwrap();
    let slot = When::Zoned(resolve_local(local, today.and_time(hour)));
    let mut draft = q.draft(args.todo, if args.todo { When::Date(today) } else { slot });
    anyhow::ensure!(!draft.summary.is_empty(), "what is it called?");

    if let Some(at) = args.at.as_deref() {
        let start =
            parse_at(at, today, today, local).with_context(|| format!("not a time: {at:?}"))?;
        let length = draft.start.until(&draft.end);
        draft.start = start;
        draft.end = start.add(if start.is_date() {
            TimeDelta::days(1)
        } else {
            length
        });
    }
    if let Some(text) = args.length.as_deref() {
        let length = parse_shift(text)
            .filter(|d| *d >= TimeDelta::zero())
            .with_context(|| format!("not a duration: {text:?}"))?;
        draft.end = draft.start.add(length);
    }
    if let Some(location) = &args.location {
        draft.location = location.clone();
    }
    if let Some(text) = args.due.as_deref() {
        let due = parse_date(text, today).with_context(|| format!("not a date: {text:?}"))?;
        draft.has_due = true;
        draft.end = When::Date(due).add(TimeDelta::days(1));
        if !draft.has_start {
            draft.start = When::Date(due);
        }
    }
    let change = vdir.create(&calendar, &draft)?;
    println!("{}", change.path.display());
    Ok(())
}

/// The (file, UID) of the item whose UID starts with `id`.
fn find_item(vdir: &Vdir, id: &str) -> anyhow::Result<(PathBuf, String, Kind)> {
    let mut found = vdir
        .components()
        .filter_map(|(_, c)| {
            let uid = c.get_uid()?;
            let kind = Kind::from_component_kind(c.component_kind())?;
            uid.starts_with(id)
                .then(|| (c.path().to_owned(), uid.to_owned(), kind))
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
    found.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
    match found.len() {
        0 => bail!("no item with an id starting {id:?}"),
        1 => Ok(found.pop().unwrap()),
        n => bail!("{n} items have ids starting {id:?}; use more of the id"),
    }
}

pub fn done(vdir: &Path, id: &str, undo: bool) -> anyhow::Result<()> {
    let mut vdir = Vdir::load(vdir)?;
    let (path, uid, _) = find_item(&vdir, id)?;
    let task = vdir.task(&path, &uid).context("that item is not a task")?;
    vdir.set_done(&task, !undo)?;
    println!(
        "{} {:?}",
        if undo { "not done:" } else { "done:" },
        task.summary
    );
    Ok(())
}

pub fn delete(vdir: &Path, id: &str) -> anyhow::Result<()> {
    let mut vdir = Vdir::load(vdir)?;
    let (path, uid, kind) = find_item(&vdir, id)?;
    // the whole item: every component with this UID
    let occ = Occurrence {
        calendar: String::new(),
        path: path.clone(),
        uid: uid.clone(),
        recurrence_id: None,
        kind,
        summary: String::new(),
        location: None,
        description: None,
        status: None,
        start: When::Date(chrono::NaiveDate::MIN),
        end: When::Date(chrono::NaiveDate::MIN),
        all_day: true,
        recurring: false,
        due: None,
        alarms: Vec::new(),
        organizer: None,
        attendees: Vec::new(),
        conferences: Vec::new(),
        parent: None,
    };
    vdir.delete(&occ, Scope::All)?;
    println!("deleted {uid} from {}", path.display());
    Ok(())
}

/// Open the item's file in $VISUAL or $EDITOR, then check it still parses.
pub fn edit(vdir: &Path, id: &str) -> anyhow::Result<()> {
    let vdir = Vdir::load(vdir)?;
    let (path, _, _) = find_item(&vdir, id)?;
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_owned());
    let mut words = editor.split_whitespace();
    let program = words.next().context("empty $EDITOR")?;
    let status = Command::new(program)
        .args(words)
        .arg(&path)
        .status()
        .with_context(|| format!("running {editor:?}"))?;
    if !status.success() {
        bail!("{editor:?} exited with {status}");
    }
    let text = std::fs::read_to_string(&path)?;
    rical_core::ics::parse_calendar(&text)
        .and_then(|_| rical_core::doc::Document::parse(&text))
        .with_context(|| format!("{} no longer parses; fix it before syncing", path.display()))?;
    Ok(())
}

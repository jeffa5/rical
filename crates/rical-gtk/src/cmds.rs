//! rical's commands: the specs the command line, key bindings and help are
//! built from. Execution lives in `window/run.rs`.

use chrono::{Datelike, Days, Months, NaiveDate, TimeDelta, Weekday};

use crate::{
    command::{Arg, Context, Flag, Kind, Spec, Values},
    layout::ViewKind,
    state::State,
};

pub const GRID: Context = Context(1);
pub const MONTH: Context = Context(2);
pub const AGENDA: Context = Context(4);
pub const TASKS: Context = Context(8);
pub const ALL: Context = GRID.or(MONTH).or(AGENDA).or(TASKS);

pub fn context(view: ViewKind) -> Context {
    match view {
        ViewKind::Day | ViewKind::Week => GRID,
        ViewKind::Month => MONTH,
        ViewKind::Agenda => AGENDA,
        ViewKind::Tasks => TASKS,
    }
}

/// The binds.conf section for a view.
pub fn section(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Day | ViewKind::Week => "grid",
        ViewKind::Month => "month",
        ViewKind::Agenda => "agenda",
        ViewKind::Tasks => "tasks",
    }
}

const fn arg(name: &'static str, kind: Kind) -> Arg {
    Arg {
        name,
        kind,
        optional: false,
        rest: false,
    }
}

const fn optional(name: &'static str, kind: Kind) -> Arg {
    Arg {
        name,
        kind,
        optional: true,
        rest: false,
    }
}

const fn spec(name: &'static str, help: &'static str) -> Spec {
    Spec {
        name,
        aliases: &[],
        help,
        contexts: ALL,
        args: &[],
        flags: &[],
    }
}

const VIEWS: Kind = Kind::Choice(&["day", "week", "month", "agenda", "tasks"]);
const DATE: Kind = Kind::Text("date");
const SHIFT: Kind = Kind::Text("shift");
const SCOPE: Kind = Kind::Choice(&["this", "all"]);

pub static SPECS: &[Spec] = &[
    spec("today", "Go to today"),
    Spec {
        aliases: &["n"],
        ..spec("next", "Next day, week or month")
    },
    Spec {
        aliases: &["previous"],
        ..spec("prev", "Previous day, week or month")
    },
    Spec {
        args: &[arg("view", VIEWS)],
        ..spec("view", "Switch to the day, week, month or agenda view")
    },
    Spec {
        aliases: &["go"],
        args: &[arg("date", DATE)],
        ..spec(
            "goto",
            "Go to a date: 2025-03-04, 03-04, 4, today, tomorrow, fri, +2w",
        )
    },
    Spec {
        args: &[arg("days", Kind::Text("days"))],
        ..spec("focus", "Move the focused day, e.g. +1 or -7")
    },
    Spec {
        args: &[arg(
            "which",
            Kind::Choice(&["next", "prev", "first", "none"]),
        )],
        ..spec("select", "Select the next or previous event, or none")
    },
    spec("open", "Show the selected event, or open the focused day"),
    spec("details", "Show the selected event's details"),
    Spec {
        aliases: &["add"],
        args: &[Arg {
            name: "title",
            kind: Kind::Text("title"),
            optional: true,
            rest: true,
        }],
        flags: &[
            Flag {
                name: "calendar",
                kind: Kind::Dynamic("calendar"),
                help: "calendar to add to",
            },
            Flag {
                name: "at",
                kind: Kind::Text("at"),
                help: "start: [date] [HH:MM]",
            },
            Flag {
                name: "for",
                kind: Kind::Text("for"),
                help: "duration, e.g. 30m, 1h30m, 2d",
            },
            Flag {
                name: "kind",
                kind: Kind::Choice(&["event", "todo"]),
                help: "event or todo",
            },
        ],
        ..spec(
            "new",
            "New event; with a title it is created at once, else the editor opens",
        )
    },
    spec("edit", "Edit the selected event"),
    Spec {
        aliases: &["rm"],
        args: &[optional("scope", SCOPE)],
        ..spec(
            "delete",
            "Delete the selected event (this occurrence or all)",
        )
    },
    Spec {
        args: &[arg("by", SHIFT), optional("scope", SCOPE)],
        ..spec("move", "Move the selected event, e.g. +15m, -1h, +1d")
    },
    Spec {
        args: &[arg("by", SHIFT), optional("scope", SCOPE)],
        ..spec("resize", "Change the selected event's end, e.g. +15m")
    },
    Spec {
        aliases: &["complete"],
        ..spec("done", "Mark the selected task done, or not done again")
    },
    Spec {
        aliases: &["todo"],
        args: &[Arg {
            name: "title",
            kind: Kind::Text("title"),
            optional: false,
            rest: true,
        }],
        flags: &[
            Flag {
                name: "due",
                kind: DATE,
                help: "due date",
            },
            Flag {
                name: "calendar",
                kind: Kind::Dynamic("calendar"),
                help: "calendar to add to",
            },
        ],
        ..spec("task", "Add a task, e.g. :task Buy milk --due fri")
    },
    Spec {
        args: &[arg("date", DATE)],
        ..spec("due", "Set the selected task's due date, or \"none\"")
    },
    spec("undo", "Undo the last change"),
    spec("redo", "Redo the last undone change"),
    Spec {
        aliases: &["cal"],
        args: &[
            arg(
                "action",
                Kind::Choice(&["toggle", "show", "hide", "only", "all"]),
            ),
            Arg {
                name: "calendar",
                kind: Kind::Dynamic("calendar"),
                optional: true,
                rest: true,
            },
        ],
        ..spec(
            "calendar",
            "Show, hide or toggle a calendar (by name or number), or show only one",
        )
    },
    Spec {
        args: &[optional("state", Kind::Choice(&["toggle", "show", "hide"]))],
        ..spec("sidebar", "Show or hide the sidebar")
    },
    spec("reload", "Reload the vdir from disk"),
    Spec {
        aliases: &["shortcuts"],
        ..spec("help", "Show key bindings")
    },
    Spec {
        aliases: &["q"],
        ..spec("quit", "Quit")
    },
];

/// Completion values from the current state.
pub struct StateValues<'a>(pub &'a State);

impl Values for StateValues<'_> {
    fn values(&self, key: &str) -> Vec<String> {
        match key {
            "calendar" => self
                .0
                .vdir
                .calendars
                .iter()
                .map(|c| c.name.clone())
                .collect(),
            _ => Vec::new(),
        }
    }

    fn suggestions(&self, key: &str) -> Vec<String> {
        let list: &[&str] = match key {
            "date" => &[
                "none",
                "today",
                "tomorrow",
                "yesterday",
                "mon",
                "tue",
                "wed",
                "thu",
                "fri",
                "sat",
                "sun",
                "+1w",
                "-1w",
                "+1m",
                "-1m",
            ],
            "days" => &["+1", "-1", "+7", "-7"],
            "shift" => &["+15m", "-15m", "+1h", "-1h", "+1d", "-1d", "+1w", "-1w"],
            "at" => &["09:00", "today", "tomorrow", "tomorrow 09:00"],
            "for" => &["15m", "30m", "1h", "2h", "1d"],
            _ => &[],
        };
        list.iter().map(|s| s.to_string()).collect()
    }
}

/// Parse a date relative to `today`: "2025-03-04", "03-04", "4" (day of
/// this month), "today", "tomorrow", "yesterday", a weekday ("fri": the
/// next one, today included) or an offset ("+3d", "-1w", "+2m", "+1y").
pub fn parse_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let t = text.trim().to_lowercase();
    match t.as_str() {
        "today" | "now" => return Some(today),
        "tomorrow" => return today.succ_opt(),
        "yesterday" => return today.pred_opt(),
        _ => {}
    }
    if let Ok(wd) = t.parse::<Weekday>() {
        let ahead = (7 + wd.num_days_from_monday() - today.weekday().num_days_from_monday()) % 7;
        return Some(today + Days::new(ahead as u64));
    }
    if let Some(sign) = t.chars().next().filter(|c| matches!(c, '+' | '-')) {
        let body = &t[1..];
        let (n, unit) = body.split_at(
            body.find(|c: char| !c.is_ascii_digit())
                .unwrap_or(body.len()),
        );
        let n: u32 = n.parse().ok()?;
        let forward = sign == '+';
        return match unit {
            "" | "d" => shift_days(today, n as u64, forward),
            "w" => shift_days(today, n as u64 * 7, forward),
            "m" => shift_months(today, n, forward),
            "y" => shift_months(today, n * 12, forward),
            _ => None,
        };
    }
    NaiveDate::parse_from_str(&t, "%Y-%m-%d")
        .ok()
        .or_else(|| NaiveDate::parse_from_str(&format!("{}-{t}", today.year()), "%Y-%m-%d").ok())
        .or_else(|| t.parse::<u32>().ok().and_then(|d| today.with_day(d)))
}

fn shift_days(d: NaiveDate, n: u64, forward: bool) -> Option<NaiveDate> {
    if forward {
        d.checked_add_days(Days::new(n))
    } else {
        d.checked_sub_days(Days::new(n))
    }
}

fn shift_months(d: NaiveDate, n: u32, forward: bool) -> Option<NaiveDate> {
    if forward {
        d.checked_add_months(Months::new(n))
    } else {
        d.checked_sub_months(Months::new(n))
    }
}

/// Parse a signed duration such as "+15m", "-1h30m", "2d" or "1w".
pub fn parse_shift(text: &str) -> Option<TimeDelta> {
    let t = text.trim();
    let (negative, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    if body.is_empty() {
        return None;
    }
    let mut total = TimeDelta::zero();
    let mut num = String::new();
    for c in body.chars() {
        match c {
            '0'..='9' => num.push(c),
            'm' | 'h' | 'd' | 'w' => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += match c {
                    'm' => TimeDelta::minutes(n),
                    'h' => TimeDelta::hours(n),
                    'd' => TimeDelta::days(n),
                    _ => TimeDelta::weeks(n),
                };
            }
            _ => return None,
        }
    }
    if !num.is_empty() {
        return None;
    }
    Some(if negative { -total } else { total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{binds, command::Registry};

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn dates() {
        let today = d("2025-10-23"); // Thursday
        for (text, want) in [
            ("today", "2025-10-23"),
            ("tomorrow", "2025-10-24"),
            ("thu", "2025-10-23"),
            ("mon", "2025-10-27"),
            ("Friday", "2025-10-24"),
            ("+3", "2025-10-26"),
            ("+1w", "2025-10-30"),
            ("-2d", "2025-10-21"),
            ("+1m", "2025-11-23"),
            ("2026-01-02", "2026-01-02"),
            ("12-25", "2025-12-25"),
            ("5", "2025-10-05"),
        ] {
            assert_eq!(parse_date(text, today), Some(d(want)), "{text}");
        }
        assert_eq!(parse_date("soon", today), None);
        assert_eq!(parse_date("32", today), None);
    }

    #[test]
    fn shifts() {
        assert_eq!(parse_shift("+15m"), Some(TimeDelta::minutes(15)));
        assert_eq!(parse_shift("-1h30m"), Some(-TimeDelta::minutes(90)));
        assert_eq!(parse_shift("2d"), Some(TimeDelta::days(2)));
        assert_eq!(parse_shift("+"), None);
        assert_eq!(parse_shift("5"), None);
    }

    /// Every default binding runs a command line that parses.
    #[test]
    fn default_bindings_are_valid() {
        let reg = Registry { specs: SPECS };
        let binds = binds::Binds::parse(binds::DEFAULTS).unwrap();
        for (section, map) in &binds.sections {
            for (keys, action) in map {
                let Some(action) = action else { continue };
                for line in &action.run {
                    reg.parse(line, ALL)
                        .unwrap_or_else(|e| panic!("[{section}] {keys:?}: {line:?}: {e}"));
                }
                if let Some(pre) = &action.prefill {
                    let name = pre.split_whitespace().next().unwrap_or("");
                    if !name.is_empty() {
                        reg.resolve(name, ALL).unwrap();
                    }
                }
            }
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names = SPECS.iter().flat_map(|s| s.names()).collect::<Vec<_>>();
        let len = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), len);
    }
}

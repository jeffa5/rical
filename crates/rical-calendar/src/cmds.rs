//! rical's commands: the specs the command line, key bindings and help are
//! built from. Execution lives in `window/run.rs`.

pub use rical_core::input::{parse_date, parse_shift};

use crate::{
    command::{Arg, Context, Flag, Kind, Spec, Values},
    layout::ViewKind,
    state::State,
};

pub const GRID: Context = Context(1);
pub const MONTH: Context = Context(2);
pub const AGENDA: Context = Context(4);
pub const TASKS: Context = Context(8);
pub const INVITATIONS: Context = Context(16);
pub const JOURNAL: Context = Context(32);
pub const YEAR: Context = Context(64);
pub const ALL: Context = GRID
    .or(MONTH)
    .or(AGENDA)
    .or(TASKS)
    .or(INVITATIONS)
    .or(JOURNAL)
    .or(YEAR);

pub fn context(view: ViewKind) -> Context {
    match view {
        ViewKind::Day | ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_) => GRID,
        ViewKind::Month => MONTH,
        ViewKind::Year => YEAR,
        ViewKind::Agenda => AGENDA,
        ViewKind::Tasks => TASKS,
        ViewKind::Invitations => INVITATIONS,
        ViewKind::Journal => JOURNAL,
    }
}

/// The binds.conf section for a view.
pub fn section(view: ViewKind) -> &'static str {
    match view {
        ViewKind::Day | ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_) => "grid",
        ViewKind::Month => "month",
        ViewKind::Year => "year",
        ViewKind::Agenda => "agenda",
        ViewKind::Tasks => "tasks",
        ViewKind::Invitations => "invitations",
        ViewKind::Journal => "journal",
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

const VIEWS: Kind = Kind::Text("view");
const DATE: Kind = Kind::Text("date");
const SHIFT: Kind = Kind::Text("shift");
const SCOPE: Kind = Kind::Choice(&["this", "all"]);

/// What the help dialog lists: the commands, and the views' binding sections.
pub const HELP: rical_ui::help::Commands = rical_ui::help::Commands {
    specs: SPECS,
    all: ALL,
    views: &[
        ("grid", "Day and week"),
        ("month", "Month"),
        ("year", "Year"),
        ("agenda", "Agenda"),
        ("tasks", "Tasks"),
        ("invitations", "Invitations"),
        ("journal", "Journal"),
    ],
};

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
        ..spec(
            "view",
            "Switch view: day, week, workweek, fullweek, Ndays (2-14), month, year, agenda, tasks, invitations, journal",
        )
    },
    Spec {
        args: &[arg("style", Kind::Text("week"))],
        ..spec(
            "week",
            "Choose what the week view shows: fullweek, workweek or Ndays",
        )
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
    spec(
        "reveal",
        "Show the selected task, journal entry or agenda item in the calendar, on its day",
    ),
    Spec {
        aliases: &["join"],
        ..spec(
            "link",
            "Open the selected event's link (video call) from its location or notes",
        )
    },
    spec("map", "Show the selected event's location on a map"),
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
                name: "location",
                kind: Kind::Text("location"),
                help: "where",
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
        args: &[Arg {
            name: "who",
            kind: Kind::Dynamic("guest"),
            optional: false,
            rest: true,
        }],
        ..spec(
            "invite",
            "Invite people to the selected event (addresses; names complete)",
        )
    },
    Spec {
        args: &[Arg {
            name: "who",
            kind: Kind::Dynamic("invited"),
            optional: false,
            rest: true,
        }],
        ..spec("uninvite", "Take people off the selected event's guests")
    },
    spec(
        "accept",
        "Accept the selected invitation (every time it repeats)",
    ),
    Spec {
        aliases: &["tentative"],
        ..spec("maybe", "Answer maybe to the selected invitation")
    },
    spec("decline", "Decline the selected invitation"),
    spec(
        "raw",
        "Show the selected event's .ics file as it is on disk",
    ),
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
        aliases: &["preferences", "prefs"],
        ..spec("settings", "Change the settings (saved to the config file)")
    },
    spec("about", "The version, licence and details for a bug report"),
    Spec {
        aliases: &["who"],
        args: &[Arg {
            name: "who",
            kind: Kind::Dynamic("invited"),
            optional: true,
            rest: true,
        }],
        ..spec(
            "person",
            "Show someone on the selected event (name or address) from your contacts",
        )
    },
    Spec {
        aliases: &["to-task", "to-event"],
        ..spec(
            "convert",
            "Turn the selected event into a task, or task into an event",
        )
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
        aliases: &["jot"],
        args: &[Arg {
            name: "title",
            kind: Kind::Text("title"),
            optional: true,
            rest: true,
        }],
        flags: &[
            Flag {
                name: "date",
                kind: DATE,
                help: "the day it is for, or \"none\" for an undated note",
            },
            Flag {
                name: "calendar",
                kind: Kind::Dynamic("calendar"),
                help: "calendar to add to",
            },
        ],
        ..spec(
            "note",
            "Add a journal entry, e.g. :note Ran 5k today; without text the editor opens",
        )
    },
    Spec {
        args: &[Arg {
            name: "title",
            kind: Kind::Text("title"),
            optional: false,
            rest: true,
        }],
        flags: &[Flag {
            name: "due",
            kind: DATE,
            help: "due date",
        }],
        ..spec(
            "subtask",
            "Add a subtask to the selected task, e.g. :subtask Book taxi --due fri",
        )
    },
    Spec {
        aliases: &["parent"],
        ..spec("up", "Go from the selected subtask to its parent task")
    },
    Spec {
        args: &[Arg {
            name: "task",
            kind: Kind::Phrase("task"),
            optional: false,
            rest: true,
        }],
        ..spec(
            "subtask-of",
            "Make the selected task a subtask of another (by title), or \"none\"",
        )
    },
    Spec {
        args: &[Arg {
            rest: true,
            ..arg("when", DATE)
        }],
        ..spec(
            "due",
            "Set the selected task's due date, e.g. fri or fri 17:00, or \"none\"",
        )
    },
    Spec {
        args: &[Arg {
            rest: true,
            ..arg("when", DATE)
        }],
        ..spec(
            "start",
            "Set when the selected task starts, e.g. mon or mon 09:00, or \"none\"",
        )
    },
    Spec {
        aliases: &["block"],
        args: &[Arg {
            name: "at",
            kind: Kind::Text("at"),
            optional: false,
            rest: true,
        }],
        flags: &[Flag {
            name: "for",
            kind: Kind::Text("for"),
            help: "how long, e.g. 30m, 1h30m",
        }],
        ..spec(
            "schedule",
            "Time-block the selected task, e.g. :schedule tomorrow 14:00 --for 2h",
        )
    },
    Spec {
        args: &[Arg {
            name: "rule",
            kind: Kind::Text("repeat"),
            optional: false,
            rest: true,
        }],
        flags: &[
            Flag {
                name: "every",
                kind: Kind::Text("every"),
                help: "every N days/weeks/...",
            },
            Flag {
                name: "on",
                kind: Kind::Text("on"),
                help: "weekdays, e.g. mo,we,fr",
            },
            Flag {
                name: "count",
                kind: Kind::Text("count"),
                help: "end after this many times",
            },
            Flag {
                name: "until",
                kind: DATE,
                help: "end on this date",
            },
        ],
        ..spec(
            "repeat",
            "Make the selected event repeat: none, daily, weekly, monthly, yearly or an RRULE, e.g. :repeat weekly --on mo,we --count 10",
        )
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
    Spec {
        args: &[
            arg("section", Kind::Choice(&["month", "calendars", "tasks"])),
            optional("state", Kind::Choice(&["toggle", "show", "hide"])),
        ],
        ..spec(
            "section",
            "Show or hide a part of the sidebar: month, calendars or tasks",
        )
    },
    Spec {
        aliases: &["find"],
        args: &[Arg {
            name: "text",
            kind: Kind::Text("search"),
            optional: true,
            rest: true,
        }],
        ..spec("search", "Search events and tasks; Enter jumps to one")
    },
    Spec {
        args: &[optional("state", Kind::Choice(&["on", "off", "toggle"]))],
        ..spec("notifications", "Turn reminders on or off")
    },
    Spec {
        args: &[arg("before", Kind::Text("remind"))],
        ..spec(
            "remind",
            "Remind this long before events without alarms of their own, or \"none\"",
        )
    },
    Spec {
        args: &[Arg {
            name: "zones",
            kind: Kind::Dynamic("zone"),
            optional: true,
            rest: true,
        }],
        ..spec(
            "zones",
            "Extra time zones beside the hours: nothing to show or hide them; a list \
             (NYC=America/New_York, Asia/Taipei) to set them; add, remove, none",
        )
    },
    Spec {
        args: &[Arg {
            name: "how",
            kind: Kind::Choice(&["toggle", "show", "hide"]),
            optional: true,
            rest: false,
        }],
        ..spec(
            "all-day",
            "Show every all-day item in the day and week views, or fold those past three \
             rows behind \"+N more\"",
        )
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
            "guest" => self.described(key).into_iter().map(|(v, _)| v).collect(),
            "zone" => ["toggle", "show", "hide", "add", "remove", "none"]
                .into_iter()
                .map(str::to_owned)
                .chain(crate::zones::names())
                .collect(),
            _ => Vec::new(),
        }
    }

    fn described(&self, key: &str) -> Vec<(String, String)> {
        match key {
            // people to invite: from past events and contacts
            "guest" => self
                .0
                .people()
                .iter()
                .map(|k| {
                    (
                        k.guest.email.clone(),
                        k.guest.name.clone().unwrap_or_default(),
                    )
                })
                .collect(),
            // open tasks by title, for :subtask-of
            // open tasks in the selected task's calendar (subtasks never
            // cross calendars), but not itself
            "task" => {
                let me = self.0.selected_task_id();
                let calendar = me
                    .as_ref()
                    .and_then(|(path, uid)| self.0.vdir.task(path, uid))
                    .map(|t| t.calendar);
                std::iter::once(("none".to_owned(), "a task of its own".to_owned()))
                    .chain(
                        self.0
                            .vdir
                            .tasks()
                            .into_iter()
                            .filter(|t| !t.is_done() && !t.summary.is_empty())
                            .filter(|t| calendar.as_ref().is_none_or(|c| *c == t.calendar))
                            .filter(|t| me.as_ref().is_none_or(|(_, uid)| *uid != t.uid))
                            .map(|t| {
                                let cal = self
                                    .0
                                    .calendar(&t.calendar)
                                    .map_or(t.calendar, |c| c.name.clone());
                                (t.summary, cal)
                            }),
                    )
                    .collect()
            }
            // the guests of the selected event
            "invited" => self
                .0
                .selected_occurrence()
                .map(|o| {
                    o.attendees
                        .iter()
                        .filter_map(|a| {
                            Some((a.email.clone()?, a.name.clone().unwrap_or_default()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            _ => self
                .values(key)
                .into_iter()
                .map(|v| (v, String::new()))
                .collect(),
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
            "view" => &[
                "day",
                "week",
                "workweek",
                "fullweek",
                "3days",
                "4days",
                "month",
                "year",
                "agenda",
                "tasks",
                "invitations",
                "journal",
            ],
            "week" => &["fullweek", "workweek", "3days", "4days", "5days"],
            "remind" => &["none", "5m", "10m", "15m", "30m", "1h"],
            "repeat" => &["none", "daily", "weekly", "monthly", "yearly"],
            "every" => &["2", "3", "4"],
            "on" => &["mo,we,fr", "tu,th", "sa,su", "mo,tu,we,th,fr"],
            "count" => &["5", "10", "20"],
            _ => &[],
        };
        list.iter().map(|s| s.to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{binds, command::Registry};

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

/// The reference docs keep up with the code: every command, setting and
/// default key binding is in them.
#[cfg(test)]
mod docs {
    const COMMANDS: &str = include_str!("../../../docs/commands.md");
    const CONFIGURATION: &str = include_str!("../../../docs/configuration.md");
    const KEYS: &str = include_str!("../../../docs/keys.md");

    #[test]
    fn every_command_is_documented() {
        for s in super::SPECS {
            for name in s.names() {
                assert!(
                    COMMANDS.contains(&format!("`:{name}")),
                    ":{name} is missing from docs/commands.md"
                );
            }
        }
    }

    #[test]
    fn every_setting_is_documented() {
        for (name, _) in crate::config::SETTINGS {
            assert!(
                CONFIGURATION.contains(&format!("| `{name}` |"))
                    && CONFIGURATION.contains(&format!("### `{name}`")),
                "{name} is missing from docs/configuration.md"
            );
        }
    }

    #[test]
    fn every_default_key_is_documented() {
        let mut section = "";
        for line in crate::binds::DEFAULTS.lines() {
            let line = line.trim();
            if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
                section = line;
                assert!(
                    KEYS.contains(&format!("### `{section}`")),
                    "{section} is missing from docs/keys.md"
                );
                continue;
            }
            let Some((key, value)) = line.split_once(" =") else {
                continue;
            };
            if line.starts_with('#') {
                continue;
            }
            let shown = value
                .trim()
                .trim_end_matches("<Enter>")
                .trim_end_matches("<space>");
            let row = if shown.is_empty() {
                format!("| `{key}` |")
            } else {
                format!("| `{key}` | `{}", shown.replace('|', "\\|"))
            };
            assert!(
                KEYS.contains(&row),
                "{section} {key} is missing from docs/keys.md"
            );
        }
    }
}

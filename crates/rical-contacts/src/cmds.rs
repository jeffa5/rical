//! Contacts' commands: what `:` completes and keys run.

use rical_ui::command::{Arg, Context, Flag, Kind, Spec};

pub const PEOPLE: Context = Context(1);
pub const DUPLICATES: Context = Context(2);
pub const ALL: Context = PEOPLE.or(DUPLICATES);

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

const fn arg(name: &'static str, kind: Kind) -> Arg {
    Arg {
        name,
        kind,
        optional: false,
        rest: false,
    }
}

pub static SPECS: &[Spec] = &[
    Spec {
        args: &[arg("view", Kind::Choice(&["people", "duplicates"]))],
        ..spec("view", "Show everyone, or the duplicates to tidy up")
    },
    Spec {
        aliases: &["find"],
        args: &[Arg {
            name: "text",
            kind: Kind::Text("text"),
            optional: true,
            rest: true,
        }],
        ..spec(
            "search",
            "Search every field: names, email, phone digits, places, notes…",
        )
    },
    Spec {
        args: &[arg(
            "which",
            Kind::Choice(&["next", "prev", "first", "last"]),
        )],
        ..spec("select", "Select the next, previous, first or last contact")
    },
    spec("back", "Clear the search, or go back to the list"),
    Spec {
        aliases: &["add"],
        args: &[Arg {
            name: "who",
            kind: Kind::Text("who"),
            optional: true,
            rest: true,
        }],
        flags: &[Flag {
            name: "book",
            kind: Kind::Dynamic("book"),
            help: "address book to add to",
        }],
        ..spec(
            "new",
            "New contact; with text (a name, emails, numbers) it is made at once, else the editor opens",
        )
    },
    spec("edit", "Edit the selected contact"),
    Spec {
        aliases: &["rm"],
        ..spec("delete", "Delete the selected contact")
    },
    Spec {
        aliases: &["email"],
        ..spec(
            "mail",
            "Write to the selected contact (their preferred address)",
        )
    },
    Spec {
        aliases: &["phone"],
        ..spec("call", "Call the selected contact (their preferred number)")
    },
    spec("map", "Show the selected contact's address on a map"),
    Spec {
        args: &[arg(
            "what",
            Kind::Choice(&["email", "phone", "address", "name", "card"]),
        )],
        ..spec(
            "copy",
            "Copy the selected contact's email, phone, address, name or whole card",
        )
    },
    spec(
        "open-file",
        "Open the selected contact's file in the app that opens vCards",
    ),
    Spec {
        contexts: DUPLICATES,
        ..spec("merge", "Merge the selected duplicates into one")
    },
    Spec {
        contexts: DUPLICATES,
        aliases: &["ignore"],
        ..spec(
            "not-duplicates",
            "These aren't the same person: stop suggesting them",
        )
    },
    spec("undo", "Undo the last change"),
    spec("redo", "Redo the last undone change"),
    spec("reload", "Read the address books again"),
    Spec {
        aliases: &["preferences", "prefs"],
        ..spec("settings", "Change the settings (saved to the config file)")
    },
    spec("about", "The version, licence and details for a bug report"),
    Spec {
        aliases: &["shortcuts"],
        ..spec("help", "Show key bindings")
    },
    Spec {
        aliases: &["q"],
        ..spec("quit", "Quit")
    },
];

/// What the help dialog lists.
pub const HELP: rical_ui::help::Commands = rical_ui::help::Commands {
    specs: SPECS,
    all: ALL,
    views: &[("people", "People"), ("duplicates", "Duplicates")],
};

#[cfg(test)]
mod tests {
    use super::*;
    use rical_ui::binds::Binds;

    #[test]
    fn defaults_parse_and_run_real_commands() {
        let binds = Binds::parse(include_str!("binds.conf")).unwrap();
        let registry = rical_ui::command::Registry { specs: SPECS };
        for section in binds.sections.values() {
            for action in section.values().flatten() {
                for line in action.run.iter().chain(action.prefill.iter()) {
                    let name = line.split_whitespace().next().unwrap_or("");
                    if name.is_empty() {
                        continue;
                    }
                    assert!(
                        registry.resolve(name, ALL).is_ok(),
                        "binds.conf runs unknown command {name:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = SPECS.iter().flat_map(|s| s.names()).collect();
        let n = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), n);
    }
}

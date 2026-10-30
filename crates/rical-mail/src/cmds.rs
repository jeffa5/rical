//! Mail's commands: what `:` completes and keys run.

use rical_ui::command::{Arg, Context, Kind, Spec};

pub const LIST: Context = Context(1);
pub const WRITE: Context = Context(2);
pub const ALL: Context = LIST.or(WRITE);

const fn spec(name: &'static str, help: &'static str) -> Spec {
    Spec {
        name,
        aliases: &[],
        help,
        contexts: LIST,
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

const fn text(name: &'static str, kind: &'static str) -> Arg {
    Arg {
        name,
        kind: Kind::Text(kind),
        optional: true,
        rest: true,
    }
}

pub static SPECS: &[Spec] = &[
    // moving around
    Spec {
        args: &[arg(
            "which",
            Kind::Choice(&[
                "next",
                "prev",
                "first",
                "last",
                "next-unread",
                "prev-unread",
            ]),
        )],
        ..spec(
            "select",
            "Select the next, previous, first or last message, or the next unread",
        )
    },
    spec("open", "Read the selected message (and mark it read)"),
    spec("back", "Clear the search, or go back to the list"),
    Spec {
        aliases: &["cd", "go"],
        args: &[arg("folder", Kind::Dynamic("folder"))],
        ..spec("folder", "Show a folder")
    },
    spec("outbox", "Show what is waiting to be sent"),
    spec("folders", "Show or hide the list of folders"),
    Spec {
        aliases: &["find"],
        args: &[text("text", "text")],
        ..spec(
            "filter",
            "Filter this folder as you type: words, from:, to:, subject:, body:, date:, unread, flagged",
        )
    },
    Spec {
        aliases: &["s"],
        args: &[text("query", "query")],
        ..spec(
            "search",
            "Search all your mail with your search tool (mu…), or every folder; without words, the filters helper",
        )
    },
    Spec {
        aliases: &["filters"],
        ..spec(
            "search-helper",
            "Build a search from a form: from, to, subject, dates, folders, flags",
        )
    },
    spec(
        "index",
        "Update the search index (runs your `index` command, or the tool's own)",
    ),
    Spec {
        args: &[Arg {
            name: "on",
            kind: Kind::Choice(&["on", "off", "toggle"]),
            optional: true,
            rest: false,
        }],
        ..spec("threads", "Show messages in conversations, or one by one")
    },
    // writing
    Spec {
        aliases: &["new", "write"],
        args: &[text("to", "to")],
        ..spec("compose", "Write a new message")
    },
    spec("reply", "Reply to the sender"),
    Spec {
        aliases: &["reply-all"],
        ..spec("replyall", "Reply to everyone")
    },
    spec("forward", "Forward the message"),
    spec("edit", "Carry on writing the selected draft"),
    // dealing with it
    spec("archive", "Move the message to the archive"),
    Spec {
        aliases: &["rm"],
        ..spec(
            "delete",
            "Move the message to the trash (delete it for good from the trash)",
        )
    },
    spec("junk", "Move the message to junk"),
    Spec {
        aliases: &["mv"],
        args: &[arg("folder", Kind::Dynamic("folder"))],
        ..spec("move", "Move the message to a folder")
    },
    Spec {
        args: &[Arg {
            name: "as",
            kind: Kind::Choice(&["read", "unread", "toggle"]),
            optional: true,
            rest: false,
        }],
        ..spec("mark", "Mark the message read or unread")
    },
    spec("flag", "Flag or unflag the message (or the picked ones)"),
    Spec {
        args: &[arg("which", Kind::Choice(&["toggle", "all", "none"]))],
        ..spec(
            "pick",
            "Pick messages to act on together: toggle this one and move on, all, or none",
        )
    },
    spec("undo", "Undo the last move or mark"),
    // what is in it
    spec("links", "List the links in the message, and open one"),
    Spec {
        args: &[
            arg("what", Kind::Choice(&["open", "save", "all"])),
            Arg {
                name: "which",
                kind: Kind::Text("attachment"),
                optional: true,
                rest: true,
            },
        ],
        ..spec(
            "attachment",
            "Open or save an attachment (by number or name)",
        )
    },
    Spec {
        args: &[arg("number", Kind::Text("link"))],
        ..spec(
            "open-link",
            "Open link N of the message, after showing where it goes",
        )
    },
    spec("invite", "Open the invitation in the calendar"),
    spec(
        "unsubscribe",
        "Leave the mailing list this message came from (asks first)",
    ),
    Spec {
        args: &[arg("answer", Kind::Choice(&["accept", "maybe", "decline"]))],
        ..spec(
            "rsvp",
            "Answer an invitation by mail to its organiser (when the calendar does not do it)",
        )
    },
    Spec {
        aliases: &["person"],
        ..spec(
            "contact",
            "Open the sender in the contacts (add them if new)",
        )
    },
    spec("headers", "Show every header of the message"),
    spec("source", "Show the message as it is on disk"),
    // the outside
    spec("sync", "Fetch new mail (runs your `sync` command)"),
    spec("reload", "Read the folders again"),
    Spec {
        args: &[text("name", "folder")],
        ..spec("mkdir", "Make a new folder (for example work/Projects)")
    },
    spec("export", "Save the message to a file (.eml)"),
    spec("wake", "Send what is queued now (runs your `send` command)"),
    spec(
        "unsend",
        "Take back the message just sent, if it is still held back, and reopen it",
    ),
    spec("retry", "Try the messages that failed to send again"),
    Spec {
        aliases: &["preferences", "prefs"],
        contexts: ALL,
        ..spec("settings", "Change the settings (saved to the config file)")
    },
    Spec {
        contexts: ALL,
        ..spec("about", "The version, licence and details for a bug report")
    },
    Spec {
        aliases: &["shortcuts"],
        contexts: ALL,
        ..spec("help", "Show key bindings")
    },
    Spec {
        aliases: &["q"],
        contexts: ALL,
        ..spec("quit", "Quit")
    },
    // writing, in the editor window
    Spec {
        contexts: WRITE,
        ..spec("send", "Queue the message to be sent")
    },
    Spec {
        contexts: WRITE,
        aliases: &["draft"],
        ..spec("save", "Keep the message as a draft")
    },
    Spec {
        contexts: WRITE,
        aliases: &["close"],
        ..spec(
            "discard",
            "Close without sending (asks if there is something to lose)",
        )
    },
    Spec {
        contexts: WRITE,
        args: &[text("file", "file")],
        ..spec("attach", "Attach a file (without one, choose)")
    },
    Spec {
        contexts: WRITE,
        args: &[text("file", "attached")],
        ..spec("detach", "Remove an attachment (the last without a name)")
    },
    Spec {
        contexts: WRITE,
        args: &[arg(
            "field",
            Kind::Choice(&["cc", "bcc", "to", "subject", "body"]),
        )],
        ..spec("field", "Go to a field, showing Cc or Bcc if needed")
    },
    Spec {
        contexts: WRITE,
        args: &[arg("identity", Kind::Dynamic("identity"))],
        ..spec("from", "Send from another of your addresses")
    },
    Spec {
        contexts: WRITE,
        aliases: &["external"],
        ..spec("edit-external", "Edit the message in your own editor")
    },
    Spec {
        contexts: WRITE,
        args: &[Arg {
            name: "what",
            kind: Kind::Choice(&["paragraph", "all"]),
            optional: true,
            rest: false,
        }],
        ..spec(
            "reflow",
            "Wrap the paragraph at the cursor (or everything) at the wrap column",
        )
    },
    Spec {
        contexts: WRITE,
        ..spec("quote", "Quote the selected lines (or this one) with >")
    },
    Spec {
        contexts: WRITE,
        ..spec(
            "unquote",
            "Remove one level of > from the selected lines (or this one)",
        )
    },
    Spec {
        contexts: WRITE,
        ..spec("signature", "Add your signature below")
    },
    Spec {
        contexts: WRITE,
        args: &[Arg {
            name: "on",
            kind: Kind::Choice(&["on", "off", "toggle"]),
            optional: true,
            rest: false,
        }],
        ..spec(
            "markdown",
            "Write this message in Markdown: sent as written, with an HTML alternative",
        )
    },
    Spec {
        contexts: WRITE,
        args: &[arg(
            "what",
            Kind::Choice(&["bold", "italic", "code", "link", "heading", "list"]),
        )],
        ..spec(
            "format",
            "Format the selection (or what you type next) in Markdown",
        )
    },
    Spec {
        contexts: WRITE,
        ..spec("count", "Words, lines and the longest line")
    },
];

/// What the help dialog lists.
pub const HELP: rical_ui::help::Commands = rical_ui::help::Commands {
    specs: SPECS,
    all: ALL,
    views: &[("list", "Mail"), ("write", "Writing")],
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

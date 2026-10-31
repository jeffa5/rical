# Files and sync

## The vdir

rical works on a *vdir*: a folder of calendar folders, each holding one
`.ics` file per item (event, task or journal entry, with any changed
occurrences of a repeating one). It is what
[vdirsyncer](https://vdirsyncer.pimutils.org) keeps in sync with a CalDAV
server:

```text
~/calendar/nextcloud/
├── personal/
│   ├── displayname          “Personal”
│   ├── color                #3584e4
│   ├── 1a2b3c….ics
│   └── …
└── work/
    └── …
```

- Any folder (at any depth, hidden ones aside) holding `.ics` files or a
  `displayname` or `color` file is a calendar. A calendar's name is its
  `displayname`, else the folder's name; its colour is `color`, else one
  from rical's palette.
- A file rical can't read is skipped, with a warning in the
  [log](troubleshooting.md#the-log).

## Sync

rical reads and writes the files and nothing else: vdirsyncer (or any tool
that keeps a vdir) does the syncing. rical watches the folders and shows
changes made to them while it runs, a new invitation arriving or an event
changed on your phone, without reloading everything. `Ctrl+R` (`:reload`)
reads everything again.

## What rical writes

- **Only what changed.** An edit rewrites just the lines it changes; the
  rest of the file (other apps' properties, its line endings, its folding)
  stays as it was.
- **Whole files or nothing.** A file is written beside the old one and
  renamed over it, so vdirsyncer never reads half a file.
- **Nothing it can't read back.** A change is checked by two iCalendar
  parsers before it is written.
- **Not over someone else's change.** If a file changed on disk since rical
  read it, rical refuses to overwrite it and says so.
- **Time zones defined.** An event in a time zone gets that zone's
  definition (VTIMEZONE) in its file, copied from another file that has it,
  or made from the time zone database.
- **Answers, not edits, for invitations.** Answering sets only your reply
  (and the time stamps), so the organiser's server takes it as a reply.

## Opening `.ics` files

Open an `.ics` file with rical (from a mail client, a file manager, or
`rical-calendar invitation.ics`) and, before changing anything, it shows what
each item in it would do:

| The file is | rical |
|---|---|
| An invitation | Adds it, or replaces yours with a newer version |
| A reply to your invitation | Records the guest's answer |
| A cancellation | Marks the event cancelled |
| An export or a plain event | Adds what's new |

Choose the calendar for new items and import; `u` undoes the lot. An item
already there and not older is left alone (“Already in your calendar”):
rical matches items by their identity (UID), not by file name, so opening
the same invitation twice doesn't add it twice.

If rical is already open, opening a file hands it to the open window.

## Other files rical uses

| File | What |
|---|---|
| `~/.config/rical-calendar/config` | [Settings](configuration.md) |
| `~/.config/rical-calendar/local.conf` | Settings saved by Preferences when the config is read-only |
| `~/.config/rical-calendar/binds.conf` | [Key bindings](keys.md) |
| `~/.local/state/rical-calendar/rical.log` | The [log](troubleshooting.md#the-log) |
| `~/.local/state/rical-calendar/state` | What the window remembers, e.g. folded time zones |
| `~/.local/share/rical-calendar/history/` | The [history](history.md), to undo changes later |
| `~/.cache/rical/calendar-*.sqlite`, `contacts-*.sqlite` | An index of the `.ics` and `.vcf` files (SQLite), so they open quickly |

The index keeps only what the apps need to find things, not the items
themselves: for an event, when it can show (repeats included), its kinds,
UIDs, the people on it, and its title, place and notes in lower case for
search; for a contact, the name and the line under it in the list, the
words a search looks for, numbers, addresses and UID. What is on screen is
read from its files when shown (a week of a big calendar is a few hundred
files), and the last few thousand read are kept in memory.

It is only a copy. A file whose size or time of change differs from when
it was indexed is read again, and an index from another version of rical
is emptied and filled again, so it can be deleted at any time. The
calendar, mail and `rical remind` share the calendar's index.

The windows open with what the index had last time, without looking at
any file first, and check the files against it in the background
afterwards; anything that changed meanwhile (a sync while rical was
closed) appears a moment later. What can be found quickly comes first: new
and deleted files (seen from the folders' listings alone) and changes to
what is on screen, then changes to everything else. Changes while rical is open are read in
the background too, as are the contacts the calendar shows names from, the
duplicates Contacts finds, and the calendar an invitation in Mail is
compared with. Only what you do yourself (saving an event, say) writes
straight away.

(`~/.config` is `$XDG_CONFIG_HOME`, `~/.local/state` is `$XDG_STATE_HOME`
and `~/.local/share` is `$XDG_DATA_HOME`, if you set them.) Contacts keeps
its own in `~/.config/rical-contacts/`, `~/.local/state/rical-contacts/`
and `~/.local/share/rical-contacts/`.

These folders used to be shared: settings in `~/.config/rical` (and
`~/.config/rical/contacts`), state in `~/.local/state/rical`. Each app moves
its old files to its own folders when it starts. A folder holding links,
as Home Manager makes, is left as it is and read from where it is, until a
`home-manager switch` with the new module puts them in the new place. Then
anything left, such as a `local.conf` saved by Preferences, moves too.

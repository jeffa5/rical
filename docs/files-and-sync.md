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
| `~/.config/rical/config` | [Settings](configuration.md) |
| `~/.config/rical/local.conf` | Settings saved by Preferences when the config is read-only |
| `~/.config/rical/binds.conf` | [Key bindings](keys.md) |
| `~/.local/state/rical/rical.log` | The [log](troubleshooting.md#the-log) |
| `~/.local/state/rical/state` | What the window remembers, e.g. folded time zones |

(`~/.config` is `$XDG_CONFIG_HOME` and `~/.local/state` is
`$XDG_STATE_HOME`, if you set them.)

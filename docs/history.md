# History and undo

Both apps keep a lasting history of what they change. Anything can be
undone: the last change with `u`, or anything from the last 30 days in the
History dialog (`Ctrl+H` or `:history`). That holds after a restart too.
Deleting asks first, Delete being the default, and stays undoable.

## Undo, redo and the History dialog

- `u` (`Ctrl+Z`) undoes the latest change made in this app; `U`
  (`Ctrl+Shift+Z`) redoes what was last undone, in order.
- `Ctrl+H` (`:history`) lists everything this app did, newest first, with
  when and how many files: "Delete the task · Renew passport", "Merge the
  contacts · Priya Shah". Enter (or the button) undoes the selected entry,
  or redoes it if it was undone. It needn't be the latest: any entry can
  be undone, as long as its files are as it left them.
- One action is one entry. A merge (the merged card and the deleted
  duplicates) is undone in one go. So is an import of many events.

If a file changed since (vdirsyncer brought down a newer version, or you
changed the same event again), undoing that entry is refused, and nothing
is changed. Undoing the later change first, or reloading, usually lets it
through. An entry is all or nothing: if one of its files can't be put
back, the others aren't either.

## What is kept, and where

Each app keeps its history in its own folder: the calendar in
`~/.local/share/rical-calendar/history`, contacts in
`~/.local/share/rical-contacts/history`.

```text
history/
  log.jsonl                 one line per action, undo and redo
  files/<id>/0.before.ics   each file as it was before the action
  files/<id>/0.after.ics    … and as the action left it (absent if deleted)
```

The log is JSON lines: `{"op":"do","id":…,"time":…,"app":"calendar",
"what":"Delete the event · Lunch","files":[{"path":…,"before":…,"after":…}]}`,
then `{"op":"undo","id":…}` and `{"op":"redo","id":…}` as entries are
undone and redone. The files are plain `.ics` and `.vcf`, so something
deleted can also be fished out by hand, or opened in another app.

Entries older than `history-days` (30 by default) are removed, with their
files, when an app starts.

| Setting | Environment | Default |
|---|---|---|
| `history` | `RICAL_HISTORY`, `RICAL_CONTACTS_HISTORY` | `~/.local/share/rical-calendar/history`, `~/.local/share/rical-contacts/history` |
| `history-days` | `RICAL_HISTORY_DAYS`, `RICAL_CONTACTS_HISTORY_DAYS` | `30` |

They are set in each app's config (`~/.config/rical-calendar/config`,
`~/.config/rical-contacts/config`) or in Preferences, and take effect when
the app next starts. Both apps can be pointed at the same folder: each
still lists, undoes and prunes only its own entries.

## Why not a synced "deleted" calendar?

There is no standard for a rubbish bin in iCalendar or vCard, nor in
CalDAV or CardDAV. Nextcloud keeps its own server-side trash bin, where
objects deleted on the server (including deletions vdirsyncer syncs up)
stay for 30 days; local apps can't see it.

The history could live in a calendar folder that vdirsyncer syncs, but
every other client would then show it as an ordinary calendar or address
book, full of deleted events and old copies. So it is kept locally by
default. To keep it with your synced files anyway, set `history` to a
folder of your choosing, ideally one your sync leaves alone.

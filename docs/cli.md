# The `rical` command

`rical` works on the same files as the app, from a terminal or a script.
Each command takes the vdir (or files and folders) as an argument;
`-v` (repeated: `-vv`, `-vvv`) logs more. `rical COMMAND --help` lists a
command's options.

## Seeing what's on

```sh
rical agenda ~/calendar/nextcloud                 # the next 7 days
rical agenda ~/calendar/nextcloud --from 2026-11-01 --days 30
rical agenda ~/calendar/nextcloud --kinds todo -o json
```

`agenda` lists occurrences in a range, with repeats expanded: `--from`
(default today), `--days` (default 7), `--kinds` (`event`, `todo`, or
both), `-o table|json`.

```sh
rical list ~/calendar/nextcloud --recursive --limit 20
rical show ~/calendar/nextcloud --recursive --id 1a2b
```

`list` lists items with a short ID each (the start of their UID): `--from`,
`--to`, `--kinds` (`todo,event,journal`), `--limit`, `--raw`,
`-o table|json`. `show --id` prints one in full.

## Adding

```sh
rical new ~/calendar/nextcloud Lunch with Sam tomorrow 1pm @ Bistro 21
rical new ~/calendar/nextcloud --todo Submit expenses by fri !
```

`new` reads the title in plain language, as the app's
[quick add](adding.md#quick-add-in-plain-language) does, and prints the new
file. Flags win over the text: `--calendar`, `--at "[date] [HH:MM]"`,
`--for 30m`, `--location`, `--todo`, `--due`.

## Changing

```sh
rical done ~/calendar/nextcloud --id 1a2b           # --undo: not done
rical delete ~/calendar/nextcloud --id 1a2b         # every occurrence
rical edit ~/calendar/nextcloud --id 1a2b           # in $VISUAL or $EDITOR
```

`edit` opens the item's file in your editor, then checks it still reads as
iCalendar and says so if not, so you can fix it before it syncs.

## Reminders

```sh
rical remind ~/calendar/nextcloud --remind 10m
```

Shows reminders as desktop notifications, checking every 30 seconds (or
once, with `--once`). See [Reminders](reminders.md).

## Looking inside files

- `rical stats PATH [--recursive]`: how many components of each kind
  (events, tasks, journal entries…) a calendar holds.
- `rical unfold PATH`: a file with its folded lines joined, for reading.

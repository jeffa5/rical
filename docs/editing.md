# Changing events

## The details popup

`Enter`, or a click, shows the selected item's details: when (and in its
own time zone, if that differs), where, its calendar, who is invited and how
they answered, its video call and dial-in links, and its notes. Along the
bottom:

| Button | Key | Does |
|---|---|---|
| Join | `O` | Opens the video call (or a link in the location or notes) |
| Edit | `e` | Opens the editor |
| Map | `M` | Opens the location on a map |
| Done / Not done | `D` | For tasks |
| + | `:subtask` | Adds a subtask to a task |
| Locate | `v` | Shows it in the calendar (from lists) |
| Make it a task / event | `Ctrl+T` | See [Converting](#converting-between-events-and-tasks) |
| Raw | `R` | Shows the `.ics` file as it is on disk |
| Delete | `x` | Deletes it |

For an invitation it also has Accept, Maybe and Decline. Click a person to
see them [from your contacts](invitations-and-people.md#contacts).

The popup isn't modal: keys keep working (`j`/`k` move on, `e` edits), and
`Escape` closes it.

## Moving and resizing

| Key | Command | Does |
|---|---|---|
| `J` / `K` | `:move +15m` / `-15m` | Later / earlier by 15 minutes |
| `H` / `L` | `:move -1d` / `+1d` | A day earlier / later |
| `]` / `[` | `:resize +15m` / `-15m` | End later / earlier |
| `}` / `{` | `:resize +1d` / `-1d` | End a day later / earlier |

`:move` and `:resize` take any length (`+1h30m`, `-1w`). In the day and
week views you can also drag an event to move it, or its bottom edge to
resize it.

## Repeating events

Changing one occurrence of a repeating event asks whether to change **only
this one** or **all of them**; commands take it as a last word instead:
`:move +1d this`, `:delete all`. Moving the whole series moves every
occurrence by the same amount. Changing the rule (in the editor, or with
`:repeat`) applies to the whole series.

```text
:repeat weekly --on mo,we --count 10
:repeat monthly --until 2026-12-31
:repeat FREQ=MONTHLY;BYDAY=1TU      # any RRULE
:repeat none                        # stop repeating
```

## Deleting

`x` (`:delete`) deletes the selected item, asking “this one or all” for a
repeating event. `u` brings it back.

## Undo

`u` (`Ctrl+Z`) undoes the last change, `U` (`Ctrl+Shift+Z`, `Ctrl+Y`)
redoes it, through every change made in this session: creating, editing,
moving, deleting, completing, answering, converting. Undo puts the file
back exactly as it was.

If a file changed on disk since rical read it (vdirsyncer brought down a
new version, say), rical refuses to overwrite it and says so: reload with
`Ctrl+R` and try again.

## Converting between events and tasks

`Ctrl+T` (`:convert`) turns the selected event into a task, or task into
an event:

- An event becomes a task starting when the event starts and due when it
  ends; a one-day all-day event becomes a task due that day.
- A task becomes an event over its dates: start to due; a task with only a
  due time becomes an hour from then; one with no dates, all day today.

Repeats, reminders, notes, location and anything rical doesn't know are
kept. The converted item gets a new file and identity (servers may refuse a
change of type in place). rical won't convert an event with guests (removing
the original would tell them it was cancelled or declined) or a task with
subtasks.

## Guests

`:invite sam@example.com` (or a name you've invited before, or from your
contacts) adds guests; `:uninvite` removes them. See
[Invitations and people](invitations-and-people.md).

## Copying out

`R` (`:raw`) shows the item's file as it is on disk, to read or copy.

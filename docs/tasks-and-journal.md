# Tasks and the journal

Tasks (VTODOs) and journal entries (VJOURNALs) live in the same calendars
as events, so they sync the same way and other apps (Nextcloud Tasks,
Thunderbird, jtx Board) see them too.

## Tasks

The Tasks view (`T`) groups every task: **overdue**, **today**,
**upcoming**, **no date** and **done**. A task due on a day also shows on
that day in the calendar, in the all-day row, and one overdue is carried to
today, in red, until it's done.

| Key | Command | Does |
|---|---|---|
| `A`, or `c` in Tasks | `:task TITLE` | Add a task, e.g. `:task Pay rent by fri !` |
| `C` in Tasks | `:subtask TITLE` | Add a subtask to the selected task |
| `Space` in Tasks, `D` | `:done` | Mark done, or not done again |
| `Enter` | `:open` | Details |
| `e` | `:edit` | Edit |
| `S`, or `s` in Tasks | `:schedule WHEN` | Time-block it (below) |
| `^` | `:up` | From a subtask to its parent |
| `v` | `:reveal` | Show it in the calendar, on its day |
| | `:due DATE` | Set (or `none`: clear) the due date, e.g. `fri 17:00` |
| | `:start DATE` | Set (or `none`: clear) when it starts |
| | `:subtask-of TITLE` | Make it a subtask of another task (or `none`) |

[Quick add](adding.md) reads tasks too: `by fri 5pm`, `due tomorrow`,
`starts mon`, `!` for high priority, `#work` for the calendar.

### Due and start dates

A task can have a due date, a start date, both or neither, each a date or
a date and time. The sidebar lists what is on now: tasks due by today,
already started, or with no dates; the Tasks view lists them all.

### Subtasks

A subtask is linked to its parent (RELATED-TO), and sits under it in the
Tasks view, which shows the parent's progress (“1 of 2 subtasks done”).
Subtasks stay in their parent's calendar.

### Repeating tasks

Marking a repeating task done moves it on to its next time instead (and
counts down a repeat count); only the last one stays done.

### Time-blocking

`:schedule tomorrow 14:00 --for 2h` gives the task a start and due time, so
it sits in the calendar as a block of work. Dragging a task from the
sidebar onto the hours does the same for an hour; onto the all-day row, it
makes it due that day.

### Tasks and events

`Ctrl+T` turns a task into an event or back. See
[Changing events](editing.md#converting-between-events-and-tasks).

## The journal

Journal entries are notes, either for a day (a diary entry, meeting notes)
or undated. Dated entries show in the calendar on their day, marked ✎,
and in the Journal view (`E`) newest first, grouped by month; undated
notes come after them.

| Key | Command | Does |
|---|---|---|
| `c` in Journal | `:note TEXT` | Add an entry, e.g. `:note Ran 5k yesterday` |
| `C` in Journal | `:note` | Add one in the editor |
| `Enter` | `:open` | Details, with the full text |
| `e` | `:edit` | Edit |
| `v` | `:reveal` | Show it in the calendar, on its day |

`:note` dates the entry by the day the text names, else the focused day
(today, in the Journal view); `--date none` makes an undated note. In the
editor, an empty date makes it undated. A new entry goes in the same
calendar as your latest one, unless you name another (`#personal`).

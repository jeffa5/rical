# Views

The switcher in the header has four parts: the calendar (named after the
calendar view you last used, e.g. “Week ▾”), **Tasks**, **Journal** and
**Invites**. From another view, the calendar part goes back to the calendar;
once there, it opens a menu of calendar views. On a narrow window the
switcher moves to a bar at the bottom.

`:view NAME` switches from the keyboard, as do these keys:

| View | Key | Shows |
|---|---|---|
| Day | `d`, `1` | One day by the hour |
| Week | `w` | What the [`week`](configuration.md#week) setting says |
| Work week | `W` | Monday to Friday |
| Full week | `7` | Monday to Sunday |
| N days | `2`–`6` | That many days from the focused one (`:view 10days` up to 14) |
| Month | `m` | Six weeks around the month |
| Year | `z` | Twelve small months, shaded by how busy each day is |
| Agenda | `a` | A list of the next 30 days |
| Tasks | `T` | Every task, by when it is due |
| Journal | `E` | Journal entries, newest first, then undated notes |
| Invitations | `I` | Invitations waiting for your answer |

`n` and `p` (or Page Down and Page Up) move a day, week, month or year at a
time; `t` goes to today; `g` (`:goto`) to a date such as `fri`, `14`,
`2026-11-14` or `+2w`. `h` and `l` move the focused day.

## Day and week

Events sit at their times, side by side when they start together; one that
starts later than another is drawn over it, set in, leaving the earlier
one's title showing. All-day events, tasks with a due date, and dated
journal entries (marked ✎) sit in the row above the hours. Tasks overdue
from earlier days are carried to today, in red. When there are more than
three rows of them, the row folds: two rows show, then “+N more” under
each day with more. Click a “+N more”, or “▾ more” under “all day”, or
press `X` (`:all-day`), to show them all; “▴ less” (or `X` again) folds
them back. Selecting something folded away shows it, and the choice is
remembered.

- **Select** with `j`/`k` or a click; `Enter` (or a click) shows the
  details; a double click edits. `j`/`k` go through the days as drawn.
  Each day goes through its all-day row top to bottom, then by time.
  Something spanning days, even one that began before the week shown, is
  reached on each of its days, so the focused day stays where you are.
  Moving to a day (`h`/`l`) selects its first item.
- **Move** an event by dragging it, or with `J`/`K` (15 minutes) and
  `H`/`L` (a day). **Resize** by dragging its bottom edge, or with `]`/`[`.
- **Create** by clicking empty hours (an hour from the half hour clicked)
  or dragging over them; on the all-day row, click a day or drag across
  days for an all-day event. The editor opens with those times.
- **Schedule a task** by dragging it from the sidebar's task list onto the
  hours (an hour then) or the all-day row (due that day).

An invitation you haven't answered is hollow with a dashed outline; one
you answered maybe (or a tentative event) is dashed; declined and
cancelled ones are faded and struck through.

### Extra time zones

With [`zones`](configuration.md#zones) set, each zone gets a column of
hours beside yours, headed with its name, the time there now, and a +1/−1
when it's another day there. Hover a column for more.

The chevron in the corner above your hours folds the zones away; folded,
the corner says how many there are, and a click brings them back. `Z` and
`:zones` do the same, and the app remembers your choice. `:zones add
Asia/Tokyo`, `:zones remove Tokyo` and `:zones none` change the list for
the session; change it for good in Preferences.

On a window too narrow for them, the zone columns step aside on their own.

## Month

Each day lists its events, with `+N more` when they don't fit. `j`/`k`
select within the month, the arrow keys move the focused day, and
`Enter` on a day with nothing selected opens it as a day, as does a click
on its number. A click on the rest of a day adds an event on it, like `c`.

## Year

Twelve small months; the more of a day is taken up, the deeper its shade.
Arrows (and `j`/`k` by week) move the focused day, `n`/`p` the year, and
`Enter` or a double click opens the day.

## Agenda

The next 30 days as a list, grouped by day: the time, and under it when it
ends; the title, and under it where it is. `j`/`k` (or the arrows) move,
`Enter` shows details.

## Tasks

Every task, grouped: overdue, today, upcoming, no date, and done. Subtasks
sit under their parent with its progress. `Space` marks the selected one
done, `c` adds a task, `C` a subtask, `s` time-blocks it. See
[Tasks and the journal](tasks-and-journal.md).

## Journal

Dated entries newest first, by month, then undated notes. `c` adds one in
plain language, `C` opens the editor. See
[Tasks and the journal](tasks-and-journal.md#the-journal).

## Invitations

Invitations from others that you haven't answered (or answered maybe), with
Accept, Maybe and Decline on each. See
[Invitations and people](invitations-and-people.md).

## Seeing an item in the calendar

From the Tasks, Journal or Agenda view, `v` (`:reveal`, or the locate
button in the details) shows the selected item in the calendar, on its day.

## The sidebar

`F9` (`:sidebar`) shows or hides it. Its parts, each foldable
(`:section month|calendars|tasks`):

- **Month**: a small month; a click goes to that day. Dots mark busy days.
- **Calendars**: tick to show or hide one (`Alt+1`…`Alt+9`, or
  `:calendar toggle NAME`); `:calendar only NAME` shows just one. The ⋮
  on hover shows its folder, item count and colour.
- **Tasks**: open tasks, overdue ones in red; drag one onto the calendar to
  schedule it.

On a narrow window the sidebar slides over the calendar instead.

## Search

`/` (`:search`) finds events (a year either side of today) and tasks by
title, place or notes; `Enter` jumps to the one chosen.

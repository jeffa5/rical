# Adding things

## Quick add, in plain language

Type what it is and when, and rical works out the rest:

```text
:new Lunch with Sam at Bistro 21 tomorrow 1pm for 90m
:new Meet Priya 3 jun 10am London time @ King's Cross
:new Standup every weekday 9:30 for 15m #work
:new Offsite 3-5 dec
:task Submit expenses by fri 5pm !
:note Ran 5k along the canal yesterday
```

As you type, the command line shows what it will make, e.g.
“→ Event “Lunch with Sam” · Thu 22 Oct 13:00–14:30 · @ Bistro 21 · in Work”,
so it is never a guess. `Enter` creates it, selected, and `u` undoes it.
`C` opens the command line with `:new ` typed; in the Tasks view `c` starts
`:task `, in the Journal view `:note `. Whatever isn't understood is the
title.

| Part | Forms |
|---|---|
| Dates | `today`, `tomorrow`, a weekday (`fri`; `next fri` is the one in next week), `14 nov`, `nov 14th`, `14/11` (day first), `2026-11-14`, `in 3 days`, `next week`. A date already past this year is next year's. |
| Date ranges | `3-5 dec`, `mon to wed` (all day, over those days) |
| Times | `9am`, `9:30pm`, `21:30`, `noon`, `midnight`, `at 5` (1 to 6 without am/pm are the afternoon) |
| Time ranges | `9-10`, `2-4pm`, `from 9 to 11:30`, `until 5pm` |
| Length | `for 45m`, `for 1h30`, `for 2 hours`, `for 3 days` (default: an hour, or the day) |
| Time zone | `London time`, `UK time`, `Europe/London`, `SGT`, `BST`, `CET`, `EST`… The event is kept in that zone, and the preview shows the time here too. |
| Place | `@ The Mill`, or `at The Mill` (at something that isn't a time) |
| Calendar | `#work` (its folder or display name) |
| Repeats | `daily`, `every weekday`, `every mon and thu`, `every 2 weeks`, `monthly`, `every year` |
| Priority (tasks) | `!` or `urgent` (high), `!5`, `low priority` |
| Due (tasks) | `due fri`, `by fri 5pm`; any date given is the due date |
| Start (tasks) | `starts mon` |
| Literal text | `"Meeting at 3"` stays in the title as written |

An event with no date is on the focused day; with no time, an hour from
the next full hour today, or at 09:00 on other days.

### Flags

Flags override what the text says:

| Command | Flags |
|---|---|
| `:new` | `--at "[date] [HH:MM]"` (a date alone is all day), `--for 30m`, `--calendar NAME`, `--location PLACE`, `--kind event\|todo` |
| `:task` | `--due DATE` (or `none`), `--calendar NAME` |
| `:note` | `--date DATE` (or `none` for an undated note), `--calendar NAME` |
| `:subtask` | `--due DATE` |

### Notes

`:note` makes a journal entry on the day the text names, or else on the
focused day (today, in the Journal view). Without text it opens the editor.
It goes in the calendar your latest entry is in, unless you name one.

## The editor

`c` (or the **+** button, or `:new` with no text) opens the editor for a new
event; `e` edits the selected one. In the Tasks view **+** makes a task, in
the Journal view a journal entry. `Ctrl+Enter` saves, `Escape` cancels.

| Field | |
|---|---|
| Title, Calendar | |
| Type | Event or task (new items only) |
| All day | Dates without times |
| Starts, Ends | `YYYY-MM-DD HH:MM`, `YYYY-MM-DD`, or `HH:MM` on the same day; Ends also takes a length such as `+1h30m` or `45m`. For a task both are optional (empty for none) and Ends is the due date. For a journal entry there is only the date, empty for an undated note. |
| Time zone, Ends in | The zone the times are typed in (yours by default); a flight can end in another zone. The other zone's time is shown beside the field. |
| Repeat | Never, daily, weekly, monthly or yearly, or a custom rule: every N days, weeks, months or years, on chosen weekdays, ending never, after N times or on a date. Rules beyond these are kept as they are; `:repeat` sets any rule. |
| Location, Video call link | The link is the event's conference link (Join opens it). |
| Guests | Add by name or address, with suggestions from past events and your contacts. With guests, you are the organiser and your server sends the invitations when this syncs. |
| Priority, Done, Subtask of | Tasks |
| Notes | |

## Dates, times and lengths elsewhere

Commands that take a date (`:goto`, `:due`, `:start`, `:schedule`) accept:
`2025-03-04`, `03-04`, `4` (the 4th of this month), `today`, `tomorrow`,
`yesterday`, a weekday (`fri`: the next one, today included), or an offset:
`+3d`, `-1w`, `+2m`, `+1y`. Where a time is wanted too, add `HH:MM`:
`fri 14:00`.

Lengths and moves (`:move`, `:resize`, `--for`, `remind`) are like `15m`,
`1h30m`, `+2d`, `-1w`.

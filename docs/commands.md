# Commands

Every action is a command. Press `:` for the command line: commands, their
arguments and calendar, people and zone names complete as you type (`Tab`
to take one), and `Enter` runs it. Keys are bound to commands (see
[Keys](keys.md)), and `?` lists both, searchable.

Arguments in `<angle brackets>` are needed, `[square brackets]` are
optional, and `...` takes the rest of the line. A `[scope]` is `this`,
`following` (this and later ones; earlier ones stay) or
`all`, for one occurrence of a repeating event, from there on, or the whole series; left
out, rical asks.

## Moving around

| Command | Does |
|---|---|
| `:today` | Go to today |
| `:next` (also `:n`) | Next day, week or month |
| `:prev` (also `:previous`) | Previous day, week or month |
| `:goto <date>` (also `:go`) | Go to a date: 2025-03-04, 03-04, 4, today, tomorrow, fri, +2w |
| `:focus <days>` | Move the focused day, e.g. +1 or -7 |
| `:view <view>` | Switch view: day, week, workweek, fullweek, Ndays (2-14), month, year, agenda, tasks, invitations, journal |
| `:week <style>` | Choose what the week view shows: fullweek, workweek or Ndays |
| `:select <which>` | Select the next or previous event, or none |
| `:search [text...]` (also `:find`) | Search events and tasks; Enter jumps to one |

## Looking at things

| Command | Does |
|---|---|
| `:open` | Show the selected event, or open the focused day |
| `:details` | Show the selected event's details |
| `:reveal` | Show the selected task, journal entry or agenda item in the calendar, on its day |
| `:link` (also `:join`) | Open the selected event's link (video call) from its location or notes |
| `:map` | Show the selected event's location on a map |
| `:raw` | Show the selected event's .ics file as it is on disk |
| `:person [who...]` (also `:who`) | Show someone on the selected event (name or address) from your contacts |

## Adding

| Command | Does |
|---|---|
| `:new [--calendar CALENDAR] [--at AT] [--for FOR] [--location LOCATION] [--kind KIND] [title...]` (also `:add`) | New event; with a title it is created at once, else the editor opens. Flags: --calendar (calendar to add to); --at (start: [date] [HH:MM]); --for (duration, e.g. 30m, 1h30m, 2d); --location (where); --kind (event or todo) |
| `:task [--due DUE] [--calendar CALENDAR] <title...>` (also `:todo`) | Add a task, e.g. :task Buy milk --due fri. Flags: --due (due date); --calendar (calendar to add to) |
| `:subtask [--due DUE] <title...>` | Add a subtask to the selected task, e.g. :subtask Book taxi --due fri. Flags: --due (due date) |
| `:note [--date DATE] [--calendar CALENDAR] [title...]` (also `:jot`) | Add a journal entry, e.g. :note Ran 5k today; without text the editor opens. Flags: --date (the day it is for, or "none" for an undated note); --calendar (calendar to add to) |

## Changing

| Command | Does |
|---|---|
| `:edit` | Edit the selected event |
| `:move <by> [scope]` | Move the selected event, e.g. +15m, -1h, +1d |
| `:resize <by> [scope]` | Change the selected event's end, e.g. +15m |
| `:repeat [--every EVERY] [--on ON] [--count COUNT] [--until UNTIL] <rule...>` | Make the selected event repeat: none, daily, weekly, monthly, yearly or an RRULE, e.g. :repeat weekly --on mo,we --count 10. Flags: --every (every N days/weeks/...); --on (weekdays, e.g. mo,we,fr); --count (end after this many times); --until (end on this date) |
| `:delete [scope]` (also `:rm`) | Delete the selected event (this occurrence, from here on, or all) |
| `:convert` (also `:to-task`, `:to-event`) | Turn the selected event into a task, or task into an event |
| `:duplicate` (also `:dup`) | Copy the selected event, task or journal entry (a whole series), without its guests |
| `:undo` | Undo the last change |
| `:redo` | Redo the last undone change |
| `:history` | Everything changed and deleted, to undo or redo any of it (kept after restarts) |

## Tasks

| Command | Does |
|---|---|
| `:done` (also `:complete`) | Mark the selected task done, or not done again |
| `:due <when...>` | Set the selected task's due date, e.g. fri or fri 17:00, or "none" |
| `:start <when...>` | Set when the selected task starts, e.g. mon or mon 09:00, or "none" |
| `:schedule [--for FOR] <at...>` (also `:block`) | Time-block the selected task, e.g. :schedule tomorrow 14:00 --for 2h. Flags: --for (how long, e.g. 30m, 1h30m) |
| `:up` (also `:parent`) | Go from the selected subtask to its parent task |
| `:subtask-of <task...>` | Make the selected task a subtask of another (by title), or "none" |

## Invitations and guests

| Command | Does |
|---|---|
| `:accept` | Accept the selected invitation (every time it repeats) |
| `:maybe` (also `:tentative`) | Answer maybe to the selected invitation |
| `:decline` | Decline the selected invitation |
| `:invite <who...>` | Invite people to the selected event (addresses; names complete) |
| `:uninvite <who...>` | Take people off the selected event's guests |

## The window

| Command | Does |
|---|---|
| `:calendar <action> [calendar...]` (also `:cal`) | Show, hide or toggle a calendar (by name or number), or show only one |
| `:sidebar [state]` | Show or hide the sidebar |
| `:section <section> [state]` | Show or hide a part of the sidebar: month, calendars or tasks |
| `:zones [zones...]` | Extra time zones beside the hours: nothing to show or hide them; a list (NYC=America/New_York, Asia/Taipei) to set them; add, remove, none |
| `:all-day [toggle\|show\|hide]` | Show every all-day item in the day and week views, or fold those past three rows behind "+N more" |
| `:notifications [state]` | Turn reminders on or off |
| `:remind <before>` | Remind this long before events without alarms of their own, or "none" |
| `:settings` (also `:preferences`, `:prefs`) | Change the settings (saved to the config file) |
| `:reload` | Reload the vdir from disk |
| `:help` (also `:shortcuts`) | Show key bindings |
| `:about` | The version, licence and details for a bug report |
| `:quit` (also `:q`) | Quit |

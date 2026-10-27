# rical

Keyboard-driven apps for your calendars and contacts, kept as plain files:

- **Calendar:** events, tasks and a journal, from a
  [vdir](https://vdirsyncer.pimutils.org/en/stable/vdir.html) of `.ics` files.
- **Contacts:** address books, from folders of `.vcf` files.

These are the folders vdirsyncer keeps in sync with a CalDAV or CardDAV
server such as Nextcloud. The files are the source of truth: rical reads
them, writes plain iCalendar and vCard back, and picks up changes made on
disk while it runs.

- `rical-calendar` (“Calendar (rical)” in your app launcher): a GTK4/libadwaita
  app with day, week, month, year, agenda, task and journal views,
  recurring events, time zones, subtasks, invitations and reminders.
- `rical-contacts` (“Contacts (rical)”): an address book in the same style.
  It searches every field, numbers however they're written. Contacts
  opened from other apps are matched by UID, and it merges duplicates. See
  [Contacts](docs/contacts.md).
- `rical`: a CLI for listing, adding, completing and editing items, plus
  `rical remind` for desktop notifications.

The two apps work together. The calendar names guests from your contacts,
and clicking someone on an event shows their contact card. From there,
**Open in Contacts** opens it in the Contacts app, or **Add to Contacts**
adds them.

![Week view, with Singapore time beside the hours](docs/week.png)

| Month | Tasks |
|---|---|
| ![Month view](docs/month.png) | ![Tasks with subtasks](docs/tasks.png) |

| Contacts | Tidying up duplicates |
|---|---|
| ![A contact, with every way to reach them](docs/contacts.png) | ![Two cards for one person, side by side, to merge](docs/duplicates.png) |

To try them without your own data, run `scripts/demo-vdir.sh /tmp/demo` and
`scripts/demo-contacts.sh /tmp/contacts`. They write the made-up calendars
and contacts shown here. Then run `RICAL_VDIR=/tmp/demo rical-calendar` and
`RICAL_CONTACTS_CONTACTS=/tmp/contacts rical-contacts`.

## Documentation

The [documentation](docs/README.md) covers everything; start with
[Getting started](docs/getting-started.md). Also:
[Configuration](docs/configuration.md) (every setting) ·
[Views](docs/views.md) · [Adding things](docs/adding.md) ·
[Commands](docs/commands.md) · [Keys](docs/keys.md) ·
[Files and sync](docs/files-and-sync.md) · [Contacts](docs/contacts.md) ·
[Troubleshooting](docs/troubleshooting.md).

## Install

```sh
nix profile install github:jeffa5/rical   # both apps and the CLI; or: nix run github:jeffa5/rical
cargo install --path crates/rical-calendar  # needs GTK 4 and libadwaita
cargo install --path crates/rical-contacts
```

The flake also has a [Home Manager module](docs/configuration.md#home-manager)
(`programs.rical`). Its `openCalendarFiles` and `openContactFiles` make the
apps open `.ics` and `.vcf` files from other apps.

## Configure

Calendar's Preferences (`Ctrl+,` or `:settings`) edit
`~/.config/rical/config`, a file of `name = value` lines:

```ini
vdir = ~/calendars
week = workweek
remind = 10m
contacts = ~/contacts   # vCards: guests suggested and shown by these names
zones = UK=Europe/London
```

Each setting can also come from the environment (`RICAL_VDIR=...`). If the
config is read-only (say Home Manager made it), Preferences save to
`local.conf` beside it, which is read after it. See
[Configuration](docs/configuration.md) for every setting.

Contacts has its own, smaller config in `~/.config/rical/contacts/config`
(`RICAL_CONTACTS_...` in the environment). Without one, it opens
`~/contacts`:

```ini
contacts = ~/contacts   # a folder of address books
book = personal         # where new contacts go
```

## Use

Both apps work the same way. Every action is a `:command` with
completion, and keys are bound to commands (aerc style): `?` for help and
`:` for the command line.

In Calendar, `c` makes a new event, `e` edits, and `D` marks a task done.
`I` lists invitations waiting for your reply; `y`, `Y` and `N` accept,
answer maybe, or decline (your server, e.g. Nextcloud, tells the
organiser). `E` shows the journal and `:note` adds to it. From a task,
journal entry or agenda item, `v` shows it in the calendar; `Ctrl+T` turns
an event into a task or back.

Add things in plain language; the command line shows what it understood
before you press Enter, and `rical new` reads the same:

```text
:new Lunch with Sam at Bistro 21 tomorrow 1pm for 90m
:new Meet Priya 3 jun 10am London time @ King's Cross
:new Standup every weekday 9:30 for 15m #work
:new Offsite 3-5 dec
:task Submit expenses by fri 5pm !
:note Ran 5k along the canal yesterday
```

In Contacts:

- `/` searches every field, words and numbers alike (`07700 9` finds
  `+44 7700 900123`).
- `c` adds someone, and `C` adds in one line (`:new Jo Park jo@example.com
  07700 900123`).
- `e` edits, and `M`, `P` and `A` write to, call or map the person.
- `2` lists likely duplicates, and `m` merges them: you choose where they
  disagree, and the leftovers are deleted after asking.
- `u` undoes anything, a merge included.

## License

MIT

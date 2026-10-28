# Configuration

Settings come from, in order (a later one wins):

1. The defaults below.
2. The config file, `~/.config/rical/config` (or the file `RICAL_CONFIG`
   names). Within it, later lines win, and an `include` takes effect where
   it is written.
3. `local.conf` beside the config file, if it exists, read after it
   (unless the config includes it itself, when it applies at that point).
   Preferences save here when the config file is read-only.
4. The environment: `RICAL_VDIR`, `RICAL_WEEK` and so on, which no file can
   override.

## Preferences

**Preferences** in the main menu (`Ctrl+,`, or `:settings`) edits the
settings, saving each change to the config file as you make it: the rest of
the file, its comments and includes, is kept as it was. Most changes apply
at once; those marked *at start* below wait until the app starts again,
and it says so.

If the config file can't be written (Home Manager makes it a link into the
read-only Nix store), Preferences leave it alone and save to `local.conf`
beside it instead, which is read after it, so your changes still apply.
Move them into your Nix config when you like and delete `local.conf`.

A setting given in the environment shows greyed out, “Set by RICAL_…”:
change it where that is set.

## The config file

`name = value`, one per line:

```ini
# ~/.config/rical/config
vdir = ~/calendar/nextcloud
week = workweek        # a comment
remind = 10m
me = me@example.com, me@work.example
zones = UK=Europe/London, America/New_York
include = local-extra.conf           # relative to this file
include-if-exists = ~/.config/rical/secret.conf
```

- `#` starts a comment at the start of a line or after a space; `#ff0000`
  is a value, not a comment. Quote a value that has ` #` in it:
  `me = "a # b"`.
- `~/` and `$HOME/` mean your home folder; relative paths are from the
  file's own folder.
- `include = FILE` reads another file there and then (it must exist);
  `include-if-exists = FILE` skips one that doesn't.
- An unknown setting is an error, so a typo doesn't go unnoticed.

`rical-calendar --help` lists the settings too.

## Settings

| Setting | Environment | Values | Default | Applies |
|---|---|---|---|---|
| `vdir` | `RICAL_VDIR` | A folder of calendar folders | Required | at start |
| `calendar` | `RICAL_CALENDAR` | A calendar's folder name | The one with most items | at once |
| `contacts` | `RICAL_CONTACTS` | A folder of `.vcf` files (searched inside) | None | at once |
| `me` | `RICAL_ME` | Your addresses, comma-separated | Guessed | at once |
| `view` | `RICAL_VIEW` | The view to open on (see below) | `week` | at start |
| `week` | `RICAL_WEEK` | `fullweek`, `workweek` or `Ndays` (2 to 14) | `fullweek` | at once |
| `zones` | `RICAL_ZONES` | Extra time zones (see below) | None | at once |
| `remind` | `RICAL_REMIND` | A length, e.g. `10m`, `1h` | None | at once |
| `no-notifications` | `RICAL_NO_NOTIFICATIONS` | `true` or `false` | `false` | at once |
| `binds` | `RICAL_BINDS` | A key bindings file | `~/.config/rical/binds.conf` if it exists | at start |
| `renderer` | `RICAL_RENDERER` | `cairo`, `gl`, `ngl`, `vulkan` or `auto` | `cairo` | at start |
| `history` | `RICAL_HISTORY` | A folder for the history (see [History and undo](history.md)) | `~/.local/share/rical/history` | at start |
| `history-days` | `RICAL_HISTORY_DAYS` | Days of history to keep | `30` | at start |
| `date` | `RICAL_DATE` | `YYYY-MM-DD` | Today | at start |

### `vdir`

The root of the vdir: a folder whose folders are calendars (each holding
`.ics` files, and optionally `displayname` and `color` files). Nested
folders work too, e.g. one per account. See [Files and sync](files-and-sync.md).

### `calendar`

Where new events, tasks and notes go when you don't name one (`#work` in
quick add, or `--calendar`). By its folder name. Unset, it is the calendar
with the most items; once you pick one in the editor, that one is used for
the rest of the session.

### `contacts`

A folder of vCards, e.g. vdirsyncer's address book. People on events are
then shown by the names you have for them, guests are suggested from them,
and clicking someone shows their card. See
[Invitations and people](invitations-and-people.md#contacts).

### `me`

Your email addresses, to tell invitations to you apart (and which guest
is you). Unset, rical guesses: the addresses on the most invitations, plus
those your server marks as sent by you.

### `view`

The view to open on: `day`, `week` (whatever `week` says), `workweek`,
`fullweek`, `Ndays` (e.g. `3days`), `month`, `year`, `agenda`, `tasks`,
`journal` or `invitations`.

### `week`

What the week view (`w`) shows: the full week, the working week (Monday to
Friday), or a number of days starting from the focused one.

### `zones`

Extra time zones beside the hours in the day and week views, each an IANA
name, optionally labelled: `NYC=America/New_York, Asia/Taipei`. They can
be folded away from the corner above the hours (or `Z`), and the app
remembers that. See [Views](views.md#extra-time-zones).

### `remind`

A reminder this long before every timed event that has no alarm of its
own. Events' own alarms always apply. See [Reminders](reminders.md).

### `no-notifications`

`true` to leave reminders to `rical remind` running as a service, so you
don't get each one twice. Preferences show it as **Show reminders** (the
other way round).

### `binds`

A file of key bindings layered over the defaults. See [Keys](keys.md).

### `renderer`

How GTK draws. `cairo` (on the CPU) uses the least memory, 15–40 MB less
than the GPU renderers, and draws this app just as quickly. `auto` leaves
the choice to GTK. A `GSK_RENDERER` in the environment wins over this.

### `history`

The folder where changes are kept so they can be undone later, after a
restart too: a log, and each changed or deleted file as it was. Shared
with the contacts app by default. See [History and undo](history.md).

### `history-days`

How many days of history (and deleted files) to keep; older entries are
removed when the app starts.

### `date`

The day to open on, for testing or screenshots.

## Home Manager

The flake has a Home Manager module:

```nix
{
  imports = [ rical.homeManagerModules.default ];

  programs.rical = {
    enable = true;
    vdir = "~/calendar/nextcloud";
    calendar = "personal";
    week = "workweek";
    remind = "10m";
    reminders = true;              # run `rical remind` as a user service
    openCalendarFiles = true;      # open .ics files (invitations) in rical
    openContactFiles = true;       # open .vcf files (contacts) in rical-contacts
    settings = {
      me = [ "me@example.com" ];
      contacts = "~/contacts/nextcloud";
      zones = [ "UK=Europe/London" ];
    };
    binds = ''
      [global]
      <C-t> = :today<Enter>
    '';
    extraConfig = "include-if-exists = ~/.config/rical/private.conf";
  };
}
```

| Option | Does |
|---|---|
| `enable` | Install rical. |
| `package` | The package to use. |
| `vdir`, `calendar`, `week`, `remind` | Those settings. |
| `reminders` | Run `rical remind` as a systemd user service, and set `no-notifications = true` so the app doesn't remind as well. Needs `vdir`. |
| `settings` | Any other settings, by name. Lists are joined with commas; booleans are `true`/`false`. |
| `binds` | Key bindings, written to `~/.config/rical/binds.conf`. |
| `extraConfig` | Lines added to the end of the config file. |
| `openCalendarFiles` | Make rical the default for `.ics` files, so mail clients open invitations in it. |
| `openContactFiles` | Make rical-contacts the default for `.vcf` files, so contacts opened elsewhere are shown or added there ([Contacts](contacts.md)). |

The module writes these into `~/.config/rical/config`, which is then
read-only: changes made in Preferences go to `local.conf` and override it.

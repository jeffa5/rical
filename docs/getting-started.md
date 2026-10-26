# Getting started

## Install

With Nix:

```sh
nix run github:jeffa5/rical            # try it
nix profile install github:jeffa5/rical
```

Or with Home Manager, using the flake's module (see
[Configuration](configuration.md#home-manager)):

```nix
programs.rical = {
  enable = true;
  vdir = "~/calendar/nextcloud";
};
```

With Cargo (needs GTK 4 and libadwaita, and pkg-config to find them):

```sh
cargo install --path crates/rical-gtk   # the app
cargo install --path crates/rical       # the command-line tool
```

## Point it at your calendars

rical reads a *vdir*: a folder with one folder per calendar, each holding
one `.ics` file per event, task or journal entry. vdirsyncer makes one from
a CalDAV account (Nextcloud, Fastmail, iCloud and others):

```ini
# ~/.config/vdirsyncer/config (the calendar part)
[storage calendar_local]
type = "filesystem"
path = "~/calendar/nextcloud"
fileext = ".ics"
```

Tell rical where that is, in `~/.config/rical/config`:

```ini
vdir = ~/calendar/nextcloud
```

or for one run, `RICAL_VDIR=~/calendar/nextcloud rical-gtk`. Then start
**Calendar (rical)** from your launcher, or `rical-gtk`.

To look around without your own calendars, make a demo one:

```sh
scripts/demo-vdir.sh /tmp/demo
RICAL_VDIR=/tmp/demo rical-gtk
```

## Sync

rical never talks to a server itself: it reads and writes the files, and
picks up changes made to them while it runs. Run `vdirsyncer sync` (by
hand, on a timer, or as a service) to send your changes up and bring
others down. See [Files and sync](files-and-sync.md).

## The first few keys

| Key | Does |
|---|---|
| `?` | Every key and command, searchable |
| `:` | The command line, with completion |
| `t` | Today |
| `n` / `p` | Next / previous day, week or month |
| `d` `w` `m` `z` `a` | Day, week, month, year, agenda |
| `T` `E` `I` | Tasks, journal, invitations |
| `j` / `k` | Select the next / previous item |
| `Enter` | Details of the selected item |
| `c` | New event (opens the editor) |
| `C` | New event in plain language (`:new …`) |
| `e` | Edit the selected item |
| `x` | Delete it |
| `u` / `U` | Undo / redo |
| `Ctrl+,` | Preferences |

Next: [Adding things](adding.md) for quick add, and [Views](views.md).

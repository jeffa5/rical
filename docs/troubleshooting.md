# Troubleshooting

## The log

The app logs to `~/.local/state/rical/rical.log` (in `$XDG_STATE_HOME` if
you set it): what it opened, files it couldn't read, changes it couldn't
make. Warnings and errors also go to the terminal. Past 1 MB the log is
moved to `rical.log.old` when the app starts.

For more detail, set `RUST_LOG`: `RUST_LOG=debug rical-gtk` logs
everything, to the file and the terminal; `RUST_LOG=rical_core=debug`
narrows it to one part.

## Details for a bug report

**About Calendar** (in the main menu) → **Troubleshooting** →
**Debugging Information** has the version, GTK and libadwaita versions, the
renderer, the config file and calendar folder in use, your time zone, the
log's location and its last lines. **Copy Text** or **Save As** it into a
report at <https://github.com/jeffa5/rical/issues>.

## Common problems

**“which vdir?” at start.** Set [`vdir`](configuration.md#vdir) in the
config file, or `RICAL_VDIR`.

**A setting doesn't take.** The environment wins over files (`env | grep
RICAL_`), and `local.conf` wins over the config: see the
[order](configuration.md). Preferences mark settings the environment sets.

**Invitations to me aren't recognised.** Set [`me`](configuration.md#me)
to your addresses.

**“changed on disk; reload and try again”.** vdirsyncer (or another app)
changed the file since rical read it, and rical won't overwrite that.
`Ctrl+R`, then make the change again.

**An event is missing.** A file rical couldn't read is skipped and named
in the log. Check its calendar isn't hidden in the sidebar, too.

**Icons are missing or broken.** Your desktop provides no icon theme (some
window managers don't). The Nix package brings Adwaita's; otherwise install
`adwaita-icon-theme`.

**High memory use.** The default [`renderer`](configuration.md#renderer),
`cairo`, uses the least; check `GSK_RENDERER` isn't set to something else.

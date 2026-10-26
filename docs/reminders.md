# Reminders

rical shows a desktop notification (and a toast in the window) when an
event's or task's alarm goes off. Alarms come from the items themselves
(VALARMs, as other apps and servers set them), so phones and other
apps remind you at the same times.

## A default reminder

The [`remind`](configuration.md#remind) setting adds a reminder before
every timed event that has no alarm of its own:

```ini
remind = 10m
```

`:remind 15m` changes it for the session (`:remind none` turns it off).

## Turning reminders off

`:notifications off` (or `on`) for the session; the
[`no-notifications`](configuration.md#no-notifications) setting (**Show
reminders** in Preferences) for good.

## Reminders without the app open

`rical remind VDIR` checks every 30 seconds and shows reminders whether or
not the app is open:

```sh
rical remind ~/calendar/nextcloud --remind 10m
```

Run it as a service and set `no-notifications = true` so you aren't
reminded twice. Home Manager's `programs.rical.reminders = true` does both,
as a systemd user service.

Alarms that went off in the five minutes before rical started still show,
so a reminder isn't lost to a restart.

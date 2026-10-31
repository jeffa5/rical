# Mail: the send queue

`rical-mail-queue` is the sending half of a files-based mail setup. Messages to
send are files in a folder (the queue). A worker takes them one at a time
and hands each to **a command you choose** (msmtp, a script, anything that
can send a message), then files a copy under Sent. It speaks no SMTP
itself, and stores no passwords.

It is a plain command-line tool for now; a mail client can send by writing
a message into the queue (`rical-mail-queue enqueue`, or a file renamed into
`new/`).

## Try it without sending anything

```sh
cat > /tmp/rical-mail-queue.conf <<'CONF'
queue = /tmp/rical-outbox
sent = /tmp/rical-sent
send = cat >> /tmp/rical-wire.log
CONF
export RICAL_MAIL_QUEUE_CONFIG=/tmp/rical-mail-queue.conf
printf 'From: me@x.example\nTo: you@y.example\nSubject: Hi\n\nHello\n' | rical-mail-queue enqueue
rical-mail-queue list
rical-mail-queue -v run --once      # "sends" it: appends it to /tmp/rical-wire.log
```

`rical-mail-queue run` (without `--once`) keeps going: it watches the queue
and sends each message as it lands, so nothing needs to tell it to. Run it as
a service ([the Home Manager module](#as-a-service) does);
`--once` is for sending from a script or timer instead.

There is no default `send` command: until one is configured, `run`
refuses to start and nothing can leave the machine.

## Settings

`~/.config/rical-mail-queue/config` (or the file `RICAL_MAIL_QUEUE_CONFIG` or `--config`
names), `name = value` per line:

| Setting | Default | |
| --- | --- | --- |
| `queue` | `~/.local/share/rical-mail-queue/outbox` | Keep it out of any folder a mail sync (mbsync and the like) touches. |
| `sent` | `sent/` in the queue | A Maildir that gets a copy of each message sent. |
| `send` | none | The command that sends. |
| `send-from PATTERN` | none | A command for senders matching the pattern (`*` is any run of characters, case ignored). The first match wins; `send` is the fallback. A message from an address with no command fails rather than going out another way. |
| `timeout` | `60s` | How long the command has. It is killed, with anything it started, after that. |
| `max-attempts` | `8` | Then the message moves to `failed/`. |
| `retry-delay` | `30s` | Doubles after each failure, up to an hour. |
| `poll` | `1m` | The longest `run` waits before looking at the queue again. It watches the queue (inotify), so a message is sent as it arrives, and a held-back or retrying one when it comes due; this only catches what a watch cannot see, such as a queue on a network file system. |

```ini
send = msmtp --read-envelope-from -- $RICAL_MAIL_QUEUE_RECIPIENTS
send-from *@work.example = msmtp -a work -- $RICAL_MAIL_QUEUE_RECIPIENTS
```


## As a service

The Home Manager module runs the worker as a systemd user service, and
writes the settings above:

```nix
programs.rical.mailQueue = {
  enable = true;
  send = "msmtp --read-envelope-from -- $RICAL_MAIL_QUEUE_RECIPIENTS";
  sendFrom = [
    { from = "*@work.example"; command = "msmtp -a work -- $RICAL_MAIL_QUEUE_RECIPIENTS"; }
  ];
  sent = "~/mail/Sent";
  packages = [ pkgs.msmtp ];   # on the service's PATH for the send command
};
```

| Option | Setting |
| --- | --- |
| `send`, `queue`, `sent`, `timeout`, `poll` | The same names. |
| `sendFrom` | `send-from` lines, in order: a list of `{ from; command; }`. |
| `maxAttempts`, `retryDelay` | `max-attempts`, `retry-delay`. |
| `extraConfig` | Lines added at the end. |
| `packages` | Put on the service's `PATH`, ahead of your profile's, for the send command and what it runs (e.g. a password command). |
| `verbose` | Log each message's progress to the journal (default on). |

If none of the settings are given, the module leaves
`~/.config/rical-mail-queue/config` alone, so a file you keep yourself still
works. `mailQueue = true` is short for `mailQueue.enable = true`.

The service restarts when the settings change, and after a failure. With
no send command it stops instead (exit 78) rather than retrying every half
minute; `journalctl --user -u rical-mail-queue` says why. If you change
`queue`, set the mail app's `queue` to the same folder.

## The send command

It runs under `sh -c` with the message on stdin and the envelope in the
environment: `RICAL_MAIL_QUEUE_ID`, `RICAL_MAIL_QUEUE_FROM` and `RICAL_MAIL_QUEUE_RECIPIENTS`
(To, Cc and Bcc, separated by spaces). The `Bcc` header is removed from
what it reads, along with `X-Rical-*`, so it can't leak; use
`$RICAL_MAIL_QUEUE_RECIPIENTS` to reach those on Bcc.

- Exit **0**: sent.
- Exit **64–68, 73, 77, 78** (sysexits: bad usage, bad data, no such
  user or host, no permission, bad configuration): will never work, so it
  goes straight to `failed/`.
- Anything else, a signal or a timeout: tried again later.

## What you can see

`rical-mail-queue list` shows each message and why it is waiting. The same is on
the files, so a mail reader pointed at the queue folders shows it too:
`X-Rical-Attempts`, `X-Rical-Last-Error`, `X-Rical-Next-Attempt`.

```
new/       queued, or retrying: waiting for its next time
sending/   a worker has it
failed/    given up on (`rical-mail-queue retry [ID]` puts it back)
```

## What is guaranteed

- A message is never lost: every step writes the new file before removing
  the old one.
- Any number of workers can run; they claim a message with a file lock
  that the kernel drops if the worker dies, and a message a dead worker
  left behind is put back.
- It is **at least once**. SMTP has nothing to tell a sender whether a
  message it lost the answer for was accepted, so a worker dying in the
  moment between the server accepting it and the queue noting that can
  send it twice (the same `Message-ID`, which some receivers collapse). A
  command that times out may have sent it too.

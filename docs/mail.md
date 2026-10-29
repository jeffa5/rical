# Mail: the send queue

`rical-mail` is the sending half of a files-based mail setup. Messages to
send are files in a folder (the queue). A worker takes them one at a time
and hands each to **a command you choose** (msmtp, a script, anything that
can send a message), then files a copy under Sent. It speaks no SMTP
itself, and stores no passwords.

It is a plain command-line tool for now; a mail client can send by writing
a message into the queue (`rical-mail enqueue`, or a file renamed into
`new/`).

## Try it without sending anything

```sh
cat > /tmp/rical-mail.conf <<'CONF'
queue = /tmp/rical-outbox
sent = /tmp/rical-sent
send = cat >> /tmp/rical-wire.log
CONF
export RICAL_MAIL_CONFIG=/tmp/rical-mail.conf
printf 'From: me@x.example\nTo: you@y.example\nSubject: Hi\n\nHello\n' | rical-mail enqueue
rical-mail list
rical-mail -v run --once      # "sends" it: appends it to /tmp/rical-wire.log
```

There is no default `send` command: until one is configured, `run`
refuses to start and nothing can leave the machine.

## Settings

`~/.config/rical-mail/config` (or the file `RICAL_MAIL_CONFIG` or `--config`
names), `name = value` per line:

| Setting | Default | |
| --- | --- | --- |
| `queue` | `~/.local/share/rical-mail/outbox` | Keep it out of any folder a mail sync (mbsync and the like) touches. |
| `sent` | `sent/` in the queue | A Maildir that gets a copy of each message sent. |
| `send` | none | The command that sends. |
| `send-from PATTERN` | none | A command for senders matching the pattern (`*` is any run of characters, case ignored). The first match wins; `send` is the fallback. A message from an address with no command fails rather than going out another way. |
| `timeout` | `60s` | How long the command has. It is killed, with anything it started, after that. |
| `max-attempts` | `8` | Then the message moves to `failed/`. |
| `retry-delay` | `30s` | Doubles after each failure, up to an hour. |
| `poll` | `5s` | How often `run` looks at the queue. |

```ini
send = msmtp --read-envelope-from -- $RICAL_MAIL_RECIPIENTS
send-from *@work.example = msmtp -a work -- $RICAL_MAIL_RECIPIENTS
```

## The send command

It runs under `sh -c` with the message on stdin and the envelope in the
environment: `RICAL_MAIL_ID`, `RICAL_MAIL_FROM` and `RICAL_MAIL_RECIPIENTS`
(To, Cc and Bcc, separated by spaces). The `Bcc` header is removed from
what it reads, along with `X-Rical-*`, so it can't leak; use
`$RICAL_MAIL_RECIPIENTS` to reach those on Bcc.

- Exit **0**: sent.
- Exit **64–68, 73, 77, 78** (sysexits: bad usage, bad data, no such
  user or host, no permission, bad configuration): will never work, so it
  goes straight to `failed/`.
- Anything else, a signal or a timeout: tried again later.

## What you can see

`rical-mail list` shows each message and why it is waiting. The same is on
the files, so a mail reader pointed at the queue folders shows it too:
`X-Rical-Attempts`, `X-Rical-Last-Error`, `X-Rical-Next-Attempt`.

```
new/       queued, or retrying: waiting for its next time
sending/   a worker has it
failed/    given up on (`rical-mail retry [ID]` puts it back)
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

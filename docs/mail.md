# Mail

`rical-mail` is a keyboard-first mail client for Maildir folders: what
mbsync or offlineimap keep in sync with your server. It reads and writes
the files; fetching is your sync tool's job and sending is
[`rical-mail-queue`](mail-queue.md)'s, so mail stays files you can see and back up.
Like the calendar and contacts, every action is a `:command`, and keys bind
to commands (`?` lists them).

## Try it without mail

```sh
scripts/demo-mail.sh /tmp/demo-mail && scripts/demo-contacts.sh /tmp/demo-contacts
RICAL_MAIL_MAIL=/tmp/demo-mail RICAL_MAIL_CONTACTS=/tmp/demo-contacts \
RICAL_MAIL_QUEUE=/tmp/demo-queue \
RICAL_MAIL_ME='You <you@example.com>' rical-mail
```

## Settings

`~/.config/rical-mail/config`, or Preferences (`Ctrl+,`). The important
ones: `mail` (the Maildir root), `me` (your addresses), `contacts` (for
completing addresses), `sync` (e.g. `mbsync -a`, run by `Ctrl+R`), `send`
(e.g. `rical-mail-queue run --once`, run when you send; leave it empty if a
service sends), `editor`, `wrap`, `signature`, `markdown`. `--help` lists
them all.

## Private by default

- Nothing in a message is loaded: no images, no styles, no fonts, no
  frames. Mail with only an HTML part is shown as text (its words, lists
  and links), and a banner says what it would have fetched, including
  tracking pixels.
- A link says where it goes before it opens, and warns when its words
  name one site and it goes to another.
- A read receipt request is shown and never answered.
- The app itself makes no network connections: fetching and sending are
  commands you configure.
- Attachments are only opened when you ask.

## Reading

`j`/`k` move, `Enter` reads (and marks read after a second; `read-after`),
`n`/`N` find unread, `/` searches (`from:ann subject:"q4 plan" unread
flagged attachment body:budget`), `t` switches conversations, `F` folders,
`gi gs gd ga gt go` go to Inbox, Sent, Drafts, Archive, Trash and the
Outbox (what rical-mail-queue has queued, with why it is waiting).

`a` archive, `d` delete (to Trash; from Trash it deletes for good), `!`
junk, `m` move, `s` flag, `U` read or unread, `u` undo.

In a message: `o` lists its links (open one with a number), `w` saves the
attachments, `i` opens an invitation in the calendar app, `@` opens the
sender in the contacts app (adding them if new), `H` headers, `V` source.

## Writing

`c` writes, `r` replies, `R` replies to all, `f` forwards. Replies are from
the address the message was sent to. Messages are plain text, wrapped at
`wrap` (72) as you type, with a guide at that column:

- `Enter` carries quote marks, indentation and list markers on, and ends a
  list or quote on an empty one.
- `Alt+Q` wraps the paragraph, `Alt+Shift+Q` everything; `:quote` and
  `:unquote` work on the selection.
- Addresses complete from your contacts and the people your mail is from.
- `Ctrl+E` edits it in your own editor (`editor = foot nvim`).
- `Ctrl+S` keeps a draft (the Drafts folder), `Ctrl+Enter` sends. It asks
  first when there is no subject or you mention an attachment and have
  none.
- Sending puts the message in rical-mail-queue's queue; the Outbox shows it.

### Markdown

`Alt+M` (or `markdown = true`) writes the message in Markdown. It is sent
**as written** as the plain-text part, so people reading plain text see
ordinary readable text, with a plain HTML alternative for those who show
HTML. It works with replies: the quoted lines become block quotes, and your
signature stays a signature. The editor highlights bold, italic, code,
headings and links; `Ctrl+B`, `Ctrl+I` and `Ctrl+K` format the selection.
The HTML never contains images or scripts, raw HTML you type is shown as
text, and only web, mail and phone links are kept.

## Opening from other apps

The app registers for `mailto:` links, so a contact's "mail" in
rical-contacts opens a new message here, in the window that is already
running. Invitations and people go the other way: see `calendar` and
`contacts-app` in the settings.

## Not here (yet)

Spell checking, searching all folders at once,
are read when you open it), signing and encryption, and answering
invitations from the mail (the calendar app does that).

## What an invitation changes

Set `vdir` to your calendar folder (the one rical-calendar uses) and an
invitation or update is compared with the event already in it, found by its
UID. The card says how the message stands: **new** to your calendar, an
**update** (with the number of changes), the **same** version again, **older**
than the calendar's, a **cancellation**, or a guest's **reply**. For an
update each change is a row, the old value struck through and the new one in
bold:

- the time, compared as instants (the same hour written in another zone is
  no change), with how far it moved and whether the length changed ("1 hour
  later, same length", "2 hours 30 minutes instead of 1 hour");
- the title, place and repeat ("Weekly, 4 times" → "Every 2 weeks");
- the details, as the lines added and removed;
- who was invited or removed, and who answered.

A changed occurrence of a repeating event is compared with that occurrence.
Only an update shows differences: against a version the calendar already has,
what differs is your own copy's edits, so none are shown. "Open in Calendar"
applies it; the mail app never writes to the calendar.

## More

- **Undo send**: a sent message is held in the queue for `send-delay`
  (10 seconds) and the toast has **Undo**, which takes it back and reopens
  it (`:unsend`). Until then the Outbox shows when it goes, and deleting it
  there cancels it.
- **Answer an invitation by mail** (`:rsvp accept|maybe|decline`, or the
  buttons) for an invitation that is not in your calendar: an iMIP reply
  (`text/calendar; method=REPLY`) to the organiser, opened to add a note.
  For events your calendar already has, answer in the calendar, whose server
  does the scheduling.
- **Pick several** with `Space` (`Ctrl+A` for all, `Esc` for none) and
  archive, delete, move, flag or mark them together; undo takes them back
  together.
- **Unsubscribe** (`:unsubscribe`, or the button on a mailing-list message)
  writes the mail the list asks for, or opens its page after saying where it
  goes.
- Long quoted text in a message is folded behind a button.
- `:export` saves a message as `.eml`; `:mkdir` makes a folder.
- New mail while the window is not in front gives a notice with the sender
  and subject (`notify`).
- A draft is kept every minute while you write, and files dropped on the
  window attach.
- Big folders open from a header cache: 20,000 messages in about a tenth of
  a second.

## Searching

`/` filters the folder you are in as you type (`from:ann unread budget
date:1w..`). `:search QUERY` (`g/`) searches everything, and `S` opens the
**search helper**: a form for words, from, to, subject, the text, a folder,
a date range (with presets) and unread, flagged and attachments, with the
query it writes shown above as you fill it in, in the words of whatever
does the searching. Either button uses it: *Search* everywhere, or *Filter
This Folder*.

Searching is configurable with `search`:

- `builtin` (the default) reads every folder's headers (from the cache);
  `body:` looks inside messages.
- `mu` uses [mu](https://www.djcbw.org/mu/) and its index, so a search over
  a very large mailbox is instant and uses mu's query language (`flag:`,
  `maildir:`, `date:1w..`, `mime:`…). The app asks `mu find` for **links**:
  mu fills a Maildir with symlinks to the matches (`--format=links`) and the
  app shows that as a list of its own. Acting on a result (read, archive,
  delete, reply) acts on the real message. Keep the index current with
  `:index`, which also runs after `:sync` (`mu index`; set `index` for
  something else).
- `notmuch` asks `notmuch search --output=files` and makes the links itself.
- Or your own command: with `{query}` and `{dir}` it should leave a Maildir of
  links in `{dir}`; with only `{query}` it should print file names.

`Esc` leaves the results and goes back to where you were; `F5` runs the
search again.

# Mail

`rical-mailbox` is a keyboard-first mail client for Maildir folders: what
mbsync or offlineimap keep in sync with your server. It reads and writes
the files; fetching is your sync tool's job and sending is
[`rical-mail`](mail.md)'s, so mail stays files you can see and back up.
Like the calendar and contacts, every action is a `:command`, and keys bind
to commands (`?` lists them).

## Try it without mail

```sh
scripts/demo-mail.sh /tmp/demo-mail && scripts/demo-contacts.sh /tmp/demo-contacts
RICAL_MAILBOX_MAIL=/tmp/demo-mail RICAL_MAILBOX_CONTACTS=/tmp/demo-contacts \
RICAL_MAILBOX_QUEUE=/tmp/demo-queue \
RICAL_MAILBOX_ME='You <you@example.com>' rical-mailbox
```

## Settings

`~/.config/rical-mailbox/config`, or Preferences (`Ctrl+,`). The important
ones: `mail` (the Maildir root), `me` (your addresses), `contacts` (for
completing addresses), `sync` (e.g. `mbsync -a`, run by `Ctrl+R`), `send`
(e.g. `rical-mail run --once`, run when you send; leave it empty if a
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
Outbox (what rical-mail has queued, with why it is waiting).

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
- Sending puts the message in rical-mail's queue; the Outbox shows it.

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

Spell checking, a search index for very large mailboxes (a folder's headers
are read when you open it), signing and encryption, and answering
invitations from the mail (the calendar app does that).

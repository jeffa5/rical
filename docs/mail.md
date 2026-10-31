# Mail

`rical-mail` is a keyboard-first mail client for Maildir folders: what
mbsync or offlineimap keep in sync with your server. It reads and writes
the files; fetching is your sync tool's job (run it however you like: the
app watches the folders and shows new mail as it lands) and sending is
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

The first time it runs without a `mail` folder it asks for one, your
address and your contacts folder, and saves them.

`~/.config/rical-mail/config`, or Preferences (`Ctrl+,`). The important
ones: `mail` (the Maildir root), `me` (your addresses), `contacts` (for
completing addresses), `account NAME = Name <addr>` (your address in each
account), `send`
(leave it empty when `rical-mail-queue run` is running, as it sends each
message as it arrives; otherwise e.g. `rical-mail-queue run --once`), `editor`, `wrap`, `signature`, `markdown`, `spell` (e.g.
`aspell list`; misspelt words are underlined). `--help` lists
them all.

With Home Manager, `programs.rical.mail` writes the file. It fills in
`vdir`, `queue` and `contacts` from the rest of `programs.rical` (the
calendar's `vdir`, `mailQueue.queue`, `settings.contacts`):

```nix
programs.rical.mail = {
  enable = true;
  mail = "~/mail";
  me = [ "Me <me@example.com>" ];
  accounts.work = [ "Me <me@work.example>" ];
  settings = { markdown = true; read-after = "2s"; };
  binds = "";                  # ~/.config/rical-mail/binds.conf
  extraConfig = "include-if-exists = private.conf";
  openMailto = true;           # rical-mail opens mailto: links
};
```

The file is then read-only; Preferences saves to `local.conf` beside it,
which overrides it.

## Private by default

- Nothing in a message is fetched. HTML mail is drawn as HTML (`h`, or
  `:view html|text`, switches to its text) in a view with scripts off, no
  storage and a content policy that allows only the images the message
  carries itself; remote images, styles, fonts, frames, prefetching and
  referrers are all blocked, and a banner says what it would have fetched,
  including tracking pixels. Clicking a link never navigates the view: it
  asks first, like any other link.
- A link says where it goes before it opens, and warns when its words
  name one site and it goes to another.
- A read receipt request is shown and never answered.
- The app itself makes no network connections: fetching and sending are
  commands you configure.
- Attachments are only opened when you ask.

## Reading

`j`/`k` move, `Enter` reads. Reading does not mark a message read: `U` does
(or the button in the message's header); set `read-after` to `2s` to mark
after a while on screen. `Ctrl+R` reads the folders again. Loading and
searching run in the background, with a spinner. `n`/`N` find unread, `/`
filters (`from:ann subject:"q4 plan" unread
flagged attachment body:budget`), `t` switches conversations, `F` folders,
`gi gs gd ga gt go` go to Inbox, Sent, Drafts, Archive, Trash and the
Outbox (what rical-mail-queue has queued, with why it is waiting).

`a` archive, `d` delete (to Trash; from Trash it deletes for good), `!`
junk, `m` move, `s` flag, `U` read or unread, `u` undo.

**With the mouse**: every row has a menu on right-click (reply, forward,
archive, move to…, mark, flag, delete), messages drag onto a folder to move
there, and the message header has buttons for flag, read and a menu for the
rest. Rows show the first words of a message under its subject, and image
attachments show as thumbnails.

In a message: `o` lists its links (open one with a number), `w` saves the
attachments, `i` opens an invitation in the calendar app, `@` opens the
sender in the contacts app (adding them if new), `H` headers, `V` source.

## Writing

`c` writes, `r` replies, `R` replies to all, `f` forwards. Replies are from
the address the message was sent to, new mail from the account you are
looking at; the **From** list (`:from`) changes it, and the draft and sent
copy go to that account's Drafts and Sent. Messages are plain text, wrapped at
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

Signing and encryption.

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
- Big folders open from a header index (`headers.sqlite` in
  `~/.local/state/rical-mail/cache/`): 20,000 messages in about a tenth of
  a second. It keeps headers, the preview line and where each message is
  (with its read and flagged marks); a message's body is read from its
  file when you open it. A folder you open shows at once as the index had
  it, and is then listed in the background: mail that arrived, went or
  was marked meanwhile appears a moment later.

## Searching

`/` filters the folder you are in as you type (`from:ann unread budget
date:1w..`); `Shift+Enter` in the box, or the button beside it, searches
everywhere instead. `:search QUERY` (`g/`) searches everything, and `S` opens the
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
  `:index` (`mu index`; set `index` for something else), or after your
  sync tool runs.
- `notmuch` asks `notmuch search --output=files` and makes the links itself.
- Or your own command: with `{query}` and `{dir}` it should leave a Maildir of
  links in `{dir}`; with only `{query}` it should print file names.

`Esc` leaves the results and goes back to where you were; `F5` runs the
search again.

## Accounts and tabs

**Accounts.** When your mail is in accounts (`work/INBOX`, `home/INBOX`,
`gmail/[Gmail]/Drafts`: a folder per account), the folder list is grouped:
**All accounts** first, with **All inboxes**, **All drafts**, **All sent**…
(`:unified inbox`, `gI`) that put the same kind of folder from every account
in one list, each message labelled with its account; then each account as a
fold-away section with its folders, open when it has something unread (click
a name to fold or open it; it is remembered). Archive, delete and junk send
each message to the folder of that kind in *its own* account.

**Tabs.** A message you are writing opens in a tab, so you can leave it to
read, search or look something up and come back: the mail view is the first
tab and the tab bar appears when there is more than one. `gt` / `gT`,
`Ctrl+PageDown` / `Ctrl+PageUp` move between tabs, `Alt+1` is the mail and
`Alt+2`… the others (`:tab next|prev|mail|N`), and `:tab-close` (or `Ctrl+W` while writing)
closes one, asking if it holds something unsaved; so does closing the window.
A dot marks a tab with unsaved changes.

## Conversations and order

Replies are shown under what they answer (`t` turns that on or off). They
are found by `In-Reply-To` and `References`, and a reply that lost those is
put under the latest earlier message with the same subject (without the
`Re:` and `Fwd:`) within six weeks.

- **Fold** a conversation to its first message with `za` (or `zc` / `zo`;
  `zM` and `zR` fold and open all). A folded one shows how many it holds, is
  bold if any of it is unread, and **acting on it acts on all of it**: archive,
  delete, move, flag and mark read take the whole conversation.
- **Pick** a whole conversation with `T` (`:pick thread`), to act on it
  together with other things you have picked.
- **Order** the list with `:sort date|from|subject|size`, adding `reverse`
  for the other way (`os` types the command). Date is newest first (a
  conversation by its newest message), from and subject A to Z, size biggest
  first. The `sort` setting is the default; what you choose with `:sort` is
  remembered.

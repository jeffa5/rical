# Invitations and people

## Answering invitations

An invitation is an event someone else organised with you as a guest. Ones
you haven't answered are drawn hollow with a dashed outline; the
Invitations view (`I`) lists those waiting for an answer (and ones you
answered maybe), soonest first.

| Key | Command | Answer |
|---|---|---|
| `y` | `:accept` | Going |
| `Y` | `:maybe` | Maybe |
| `N` | `:decline` | Not going |

The Invitations view has the three buttons on each row, and so does the
details popup of any invitation, with your answer shown pressed. An answer
applies to every time a repeating invitation happens.

rical writes your answer into the event; when it syncs, your server
(Nextcloud, for one, with CalDAV scheduling) sends the reply to the
organiser. rical doesn't send mail itself.

### Which guest is you

rical tells invitations to you apart by your addresses: the
[`me`](configuration.md#me) setting, or a guess from the addresses on most
invitations. Set it if invitations to you aren't recognised.

### Invitations by email

Open an `.ics` attachment with rical (set it as the handler for calendar
files, or Home Manager's `openCalendarFiles`) and it shows what the file
would do before doing it: add an invitation, update one you have,
record replies to yours, or mark a cancelled event cancelled. Choose the
calendar for new ones, and import; `u` undoes it. See
[Files and sync](files-and-sync.md#opening-ics-files).

## Inviting people

Add guests in the editor's Guests section (names and addresses are
suggested as you type), or with `:invite sam@example.com Priya` and
`:uninvite`. With guests, you are the organiser; your server sends the
invitations when the event syncs.

## Contacts

Point the [`contacts`](configuration.md#contacts) setting at a folder of
vCards (vdirsyncer can keep your address book there too) and rical uses it
to:

- **Name people** by the names you have for them, wherever they appear:
  the details popup, the Invitations view, the editor's guest list. Without
  a contact, it is the name the event gives, else a name the same address
  has on other events, else the address.
- **Suggest guests** from everyone in your contacts, as well as people from
  past events.
- **Show a person's card**: click someone in the details popup, or
  `:person NAME` (`:who`). The card shows what they do and where, their
  email addresses (click to write), phone numbers (to call), addresses (to
  map), web links, birthday and notes, and **Open in …** opens their
  contact file in your contacts app. Someone not in your contacts shows
  their address.

The contacts are read again when a file in the folder changes. rical
doesn't edit contacts: that's for your contacts app.

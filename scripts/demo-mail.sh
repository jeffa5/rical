#!/usr/bin/env bash
# Write a demo Maildir tree, with the kinds of mail a mail client has to
# cope with, for screenshots and trying rical-mail without real mail:
#   scripts/demo-mail.sh /tmp/demo-mail [/tmp/demo-calendar]
# Pairs with scripts/demo-contacts.sh (the same people).
set -euo pipefail
out=${1:?usage: demo-mail.sh DIR [CALENDAR-DIR]}
rm -rf "$out"
for f in INBOX Sent Drafts Archive Trash Junk; do mkdir -p "$out/$f"/{cur,new,tmp}; done

now=$(date +%s)
n=0
# msg FOLDER new|cur FLAGS HOURS_AGO  < message (headers+body, without Date)
msg() {
  local folder=$1 sub=$2 flags=$3 hours=$4
  local ts=$((now - hours * 3600)) name
  n=$((n + 1))
  name="$ts.M${n}P1.demo"
  [[ $sub == cur ]] && name="$name:2,$flags"
  { printf 'Date: %s\n' "$(date -R -d "@$ts")"; cat; } > "$out/$folder/$sub/$name"
}

msg INBOX new "" 2 <<'EOF'
From: Ann Lee <ann@example.com>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: Q4 planning
Message-ID: <q4-1@example.com>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Hi Andrew,

Could we meet on Thursday to go through the Q4 plan? I've put the draft
budget here: https://example.com/q4/budget (the numbers are rough).

Three things I'd like to settle:

  1. who owns the migration
  2. the hiring plan
  3. what we drop

Thanks,
Ann

-- 
Ann Lee
Operations, Example Co
EOF

msg INBOX cur "S" 20 <<'EOF'
From: Priya Shah <priya@example.com>
To: Andrew Jeffery <andrew@jeffas.net>, Ann Lee <ann@example.com>
Cc: Sam Lee <sam@example.org>
Subject: Re: Q4 planning
Message-ID: <q4-2@example.com>
In-Reply-To: <q4-1@example.com>
References: <q4-1@example.com>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8; format=flowed; delsp=no

I can take the migration if we agree to drop the reporting rewrite. It 
was always going to be the first thing to slip, and the migration has 
a hard date.

Ann wrote:
> Could we meet on Thursday to go through the Q4 plan? I've put the
> draft budget here.

Thursday works for me.
EOF

msg Sent cur "S" 18 <<'EOF'
From: Andrew Jeffery <andrew@jeffas.net>
To: Ann Lee <ann@example.com>, Priya Shah <priya@example.com>
Subject: Re: Q4 planning
Message-ID: <q4-3@jeffas.net>
In-Reply-To: <q4-2@example.com>
References: <q4-1@example.com> <q4-2@example.com>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Thursday at 14:00 is fine. I'd rather keep the hiring plan for a second
meeting.
EOF

msg INBOX new "" 5 <<'EOF'
From: Northwind Weekly <news@northwind.example>
To: andrew@jeffas.net
Subject: Your week at Northwind: 3 things you missed
Message-ID: <nw-441@northwind.example>
MIME-Version: 1.0
Content-Type: text/html; charset=utf-8

<html><head><style>p{color:#333}</style></head><body>
<div style="display:none;font-size:0">Preview: three stories&zwnj;&zwnj;&zwnj;&zwnj;&zwnj;</div>
<h1>Northwind Weekly</h1>
<p>Hello Andrew, here is what happened this week.</p>
<ul><li><a href="https://northwind.example/stories/1?utm_source=mail&amp;uid=8812">The new Leeds office opens</a></li>
<li><a href="https://northwind.example/stories/2?utm_source=mail&amp;uid=8812">Product update 4.2</a></li></ul>
<p><a href="https://track.badsite.example/r/abc">https://northwind.example/account</a></p>
<p><img src="https://northwind.example/img/banner.png" alt="Summer sale"></p>
<img src="https://track.northwind.example/open.gif?u=8812" width="1" height="1">
<p>You are receiving this because you signed up. <a href="https://northwind.example/unsubscribe?u=8812">Unsubscribe</a></p>
</body></html>
EOF

msg INBOX cur "" 30 <<'EOF'
From: Sam Lee <sam@example.org>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: Climbing Saturday?
Message-ID: <sam-inv-1@example.org>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="BND"

--BND
Content-Type: text/plain; charset=utf-8

Sunday's weather looks better. Here's an invite so it lands in your calendar.
--BND
Content-Type: text/calendar; charset=utf-8; method=REQUEST
Content-Disposition: attachment; filename="invite.ics"

BEGIN:VCALENDAR
VERSION:2.0
METHOD:REQUEST
BEGIN:VEVENT
UID:climb-1@example.org
DTSTAMP:20261029T100000Z
DTSTART;TZID=Europe/London:20261101T093000
DTEND;TZID=Europe/London:20261101T150000
SUMMARY:Climbing at Almscliff
LOCATION:Almscliff Crag
ORGANIZER;CN=Sam Lee:mailto:sam@example.org
ATTENDEE;CN=Andrew Jeffery;RSVP=TRUE:mailto:andrew@jeffas.net
END:VEVENT
END:VCALENDAR
--BND--
EOF

msg INBOX cur "FS" 50 <<'EOF'
From: Jo Park <jo.park@example.net>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: Invoice and the signed form
Message-ID: <jo-1@example.net>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="B2"

--B2
Content-Type: text/plain; charset=utf-8

Both attached. The invoice is due on the 14th.
--B2
Content-Type: application/pdf; name="invoice-0042.pdf"
Content-Disposition: attachment; filename="invoice-0042.pdf"
Content-Transfer-Encoding: base64

JVBERi0xLjQKJSBkZW1vIGludm9pY2UKJSVFT0YK
--B2
Content-Type: text/plain; name="notes.txt"
Content-Disposition: attachment; filename="notes.txt"

Remember to sign page two.
--B2--
EOF

msg INBOX cur "S" 70 <<'EOF'
From: =?UTF-8?B?QW1hcmEgT2thZm9y?= <amara@example.com>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: =?UTF-8?Q?Caf=C3=A9_on_Friday_=E2=98=95?=
Message-ID: <amara-1@example.com>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8
Content-Transfer-Encoding: quoted-printable

Shall we try the new caf=C3=A9 near the station? They do a good fl=
at white, and the pastries are =E2=82=AC3.

See you there?
Amara
EOF

msg INBOX cur "S" 90 <<'EOF'
From: The Gazette <hello@gazette.example>
To: andrew@jeffas.net
Subject: Weekend reading
Message-ID: <gz-9@gazette.example>
Disposition-Notification-To: hello@gazette.example
MIME-Version: 1.0
Content-Type: multipart/alternative; boundary="ALT"

--ALT
Content-Type: text/plain; charset=utf-8

Weekend reading: a long piece on the history of the canal network, and
why it still matters. Read it at https://gazette.example/canals
--ALT
Content-Type: text/html; charset=utf-8

<p>Weekend reading: a long piece on the <b>history of the canal network</b>.</p>
<img src="https://pixel.gazette.example/o.gif?id=77" width="1" height="1">
<img src="https://gazette.example/hero.jpg">
--ALT--
EOF

msg INBOX cur "S" 120 <<'EOF'
From: Tom Becker <tom@example.de>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: Berlin trip
Message-ID: <tom-1@example.de>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Flights are booked for the 12th. Hotel is near the Hauptbahnhof; I'll send
the address once I have it.
EOF


msg INBOX cur "S" 3 <<'EOF'
From: Tom Becker <tom@example.de>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: Re: Berlin trip
Message-ID: <tom-2@example.de>
In-Reply-To: <tom-1@example.de>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8
List-Unsubscribe: <mailto:leave@example.de?subject=unsubscribe>
List-Id: <berlin.example.de>

The hotel is Hotel Am Zoo, near the Hauptbahnhof. Address below.

On Mon, 26 Oct 2026, Andrew wrote:
> Thanks for booking the flights. A few questions:
>
> 1. Which terminal do we arrive at?
> 2. Is the hotel walkable from the station?
> 3. Do we need to book the museum in advance?
> 4. What is the plan for dinner on the first night?
> 5. Should I bring the second laptop?
>
> Also: I might leave a day early.
> Tell me what you think.
EOF
msg Drafts cur "DS" 3 <<'EOF'
From: Andrew Jeffery <andrew@jeffas.net>
To: Sam Lee <sam@example.org>
Subject: Re: Climbing Saturday?
Message-ID: <draft-1@jeffas.net>
In-Reply-To: <sam-inv-1@example.org>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Sunday is good for me. I'll bring the second rope and
EOF

msg Archive cur "S" 400 <<'EOF'
From: Ann Lee <ann@example.com>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: Q3 retrospective notes
Message-ID: <q3-1@example.com>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Notes from the Q3 retro are in the usual place.
EOF

msg Trash cur "S" 200 <<'EOF'
From: Deals <deals@spam.example>
To: andrew@jeffas.net
Subject: You have won!
Message-ID: <spam-1@spam.example>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Click http://prizes.spam.example/claim now.
EOF


# An update to that invitation (a later time and place, a new guest), for
# showing what changed against the calendar made below.
msg INBOX new "" 1 <<'EOF'
From: Sam Lee <sam@example.org>
To: Andrew Jeffery <andrew@jeffas.net>
Subject: Updated invitation: Climbing at Almscliff
Message-ID: <sam-inv-2@example.org>
MIME-Version: 1.0
Content-Type: multipart/mixed; boundary="BND2"

--BND2
Content-Type: text/plain; charset=utf-8

Moved it back an hour, and Priya is coming now.
--BND2
Content-Type: text/calendar; charset=utf-8; method=REQUEST
Content-Disposition: attachment; filename="invite.ics"

BEGIN:VCALENDAR
VERSION:2.0
METHOD:REQUEST
BEGIN:VEVENT
UID:climb-1@example.org
SEQUENCE:1
DTSTAMP:20261029T180000Z
DTSTART;TZID=Europe/London:20261101T103000
DTEND;TZID=Europe/London:20261101T160000
SUMMARY:Climbing at Almscliff
LOCATION:Caley Crag
DESCRIPTION:Meet at the car park\nBring helmets\nBring the second rope
ORGANIZER;CN=Sam Lee:mailto:sam@example.org
ATTENDEE;CN=Andrew Jeffery;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:andrew@jeffas.net
ATTENDEE;CN=Priya Shah;PARTSTAT=ACCEPTED:mailto:priya@example.com
END:VEVENT
END:VCALENDAR
--BND2--
EOF

# the calendar that already has the first version (optional second argument)
if [[ -n ${2:-} ]]; then
  rm -rf "$2"
  mkdir -p "$2/personal"
  printf 'Personal\n' > "$2/personal/displayname"
  cat > "$2/personal/climb-1.ics" <<'EOF'
BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:climb-1@example.org
SEQUENCE:0
DTSTAMP:20261029T100000Z
DTSTART;TZID=Europe/London:20261101T093000
DTEND;TZID=Europe/London:20261101T150000
SUMMARY:Climbing at Almscliff
LOCATION:Almscliff Crag
DESCRIPTION:Meet at the car park\nBring helmets
ORGANIZER;CN=Sam Lee:mailto:sam@example.org
ATTENDEE;CN=Andrew Jeffery;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:andrew@jeffas.net
END:VEVENT
END:VCALENDAR
EOF
  echo "calendar with that event: $2"
fi
cat > "$out/README" <<EOF
Demo mail from scripts/demo-mail.sh. Point rical-mail at it:
  RICAL_MAIL_MAIL=$out RICAL_MAIL_ME='Andrew Jeffery <andrew@jeffas.net>' rical-mail
EOF
echo "$out: $(find "$out" -type f -path '*/cur/*' -o -type f -path '*/new/*' | wc -l) messages"

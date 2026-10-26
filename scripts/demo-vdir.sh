#!/usr/bin/env bash
# Write a demo vdir around the current week, for screenshots and trying
# rical-calendar without real data:  scripts/demo-vdir.sh /tmp/demo
set -euo pipefail
out=${1:?usage: demo-vdir.sh DIR}
rm -rf "$out"
mkdir -p "$out"/{work,personal,birthdays}
printf 'Work\n' > "$out/work/displayname";          printf '#e66100\n' > "$out/work/color"
printf 'Personal\n' > "$out/personal/displayname";  printf '#3584e4\n' > "$out/personal/color"
printf 'Birthdays\n' > "$out/birthdays/displayname"; printf '#9141ac\n' > "$out/birthdays/color"

tz=${DEMO_TZ:-Europe/London}
monday=$(date -d "$(date +%F) -$(( $(date +%u) - 1 )) days" +%F)
day() { date -d "$monday +$1 days" +%Y%m%d; }   # day N of this week (0 = Monday)
n=0
item() { # calendar body...
  local cal=$1; shift
  n=$((n + 1))
  {
    printf 'BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//rical//demo//EN\r\n'
    for line in "$@"; do printf '%s\r\n' "$line"; done
    printf 'END:VCALENDAR\r\n'
  } > "$out/$cal/demo-$n.ics"
}
event() { # calendar uid day start end summary [extra lines...]
  local cal=$1 uid=$2 d=$3 s=$4 e=$5 summary=$6; shift 6
  item "$cal" BEGIN:VEVENT "UID:$uid" DTSTAMP:20250101T000000Z \
    "DTSTART;TZID=$tz:$(day "$d")T${s}00" "DTEND;TZID=$tz:$(day "$d")T${e}00" \
    "SUMMARY:$summary" "$@" END:VEVENT
}
task() { # calendar uid summary [extra lines...]
  local cal=$1 uid=$2 summary=$3; shift 3
  item "$cal" BEGIN:VTODO "UID:$uid" DTSTAMP:20250101T000000Z "SUMMARY:$summary" "$@" END:VTODO
}

event work standup 0 0930 0945 "Standup" "RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR" "LOCATION:https://meet.example.com/abc-defg-hij"
event work planning 0 1000 1130 "Sprint planning" "LOCATION:Room 4"
event work oneone 1 1400 1430 "1:1 with Sam" \
  "CONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Video call:https://meet.example.com/sam-alex" \
  "CONFERENCE;VALUE=URI;FEATURE=PHONE;LABEL=Dial-in:tel:+44-20-7946-0123"
event work review 2 1100 1230 "Design review" "STATUS:TENTATIVE" "DESCRIPTION:Walk through the new calendar views."
event work interview 2 1130 1215 "Interview: backend engineer"
event work focus 3 1300 1600 "Focus time" "DESCRIPTION:No meetings."
event work retro 4 1500 1600 "Retro"
event work lunch 4 1230 1330 "Team lunch" "LOCATION:Noodle bar"
event work cancelled 1 1600 1700 "Vendor call" "STATUS:CANCELLED"
event personal gym 0 1800 1900 "Gym" "RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR"
event personal dentist 1 0815 0900 "Dentist"
event personal dinner 5 1930 2200 "Dinner with Alex" "LOCATION:Bistro 21\, 4 Park Row\, Leeds"
event personal piano 6 1100 1200 "Piano lesson" "RRULE:FREQ=WEEKLY"
item personal BEGIN:VEVENT UID:trip DTSTAMP:20250101T000000Z \
  "DTSTART;VALUE=DATE:$(day 5)" "DTEND;VALUE=DATE:$(day 8)" "SUMMARY:Weekend in the Lakes" END:VEVENT
item birthdays BEGIN:VEVENT UID:bday-jo DTSTAMP:20250101T000000Z \
  "DTSTART;VALUE=DATE:$(date -d "$monday +3 days" +1990%m%d)" "RRULE:FREQ=YEARLY" "SUMMARY:🎂 Jo" END:VEVENT
item birthdays BEGIN:VEVENT UID:bday-kim DTSTAMP:20250101T000000Z \
  "DTSTART;VALUE=DATE:$(date -d "$monday +17 days" +1985%m%d)" "RRULE:FREQ=YEARLY" "SUMMARY:🎂 Kim" END:VEVENT

today=$(date +%Y%m%d)
ago() { date -d "-$1 days" +%Y%m%d; }
ahead() { date -d "+$1 days" +%Y%m%d; }
task personal renew "Renew passport" "DUE;VALUE=DATE:$(ago 3)" STATUS:NEEDS-ACTION PRIORITY:1
task work expenses "Submit expenses" "DUE;VALUE=DATE:$(ago 1)" STATUS:NEEDS-ACTION
task work slides "Draft slides for review" "DUE;VALUE=DATE:$today" STATUS:NEEDS-ACTION
task personal plants "Water plants" "DUE;VALUE=DATE:$(ahead 2)" STATUS:NEEDS-ACTION
task work okrs "Write OKRs" "DUE;VALUE=DATE:$(ahead 9)" STATUS:NEEDS-ACTION PRIORITY:5
task personal books "Return library books" STATUS:NEEDS-ACTION
task personal gift "Buy a present for Jo" STATUS:NEEDS-ACTION
task work laptop "Set up new laptop" "DUE;VALUE=DATE:$(ago 2)" STATUS:COMPLETED \
  "COMPLETED:$(ago 2)T160000Z" PERCENT-COMPLETE:100
task work okrs-input "Collect team input" RELATED-TO\;RELTYPE=PARENT:okrs STATUS:COMPLETED \
  "COMPLETED:$(ago 1)T120000Z"
task work okrs-draft "Draft goals" RELATED-TO\;RELTYPE=PARENT:okrs "DUE;VALUE=DATE:$(ahead 5)" \
  STATUS:NEEDS-ACTION

# in another time zone, and an invitation waiting for a reply
item work BEGIN:VEVENT UID:sg-call DTSTAMP:20250101T000000Z \
  "DTSTART;TZID=Asia/Singapore:$(day 3)T170000" "DTEND;TZID=Asia/Singapore:$(day 3)T180000" \
  "SUMMARY:Call with Singapore office" END:VEVENT
item work BEGIN:VEVENT UID:offsite DTSTAMP:20250101T000000Z \
  "DTSTART;TZID=$tz:$(day 4)T090000" "DTEND;TZID=$tz:$(day 4)T113000" "SUMMARY:Roadmap offsite" \
  "ORGANIZER;CN=Priya Shah:mailto:priya@example.com" \
  "ATTENDEE;CN=Demo User;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.com" \
  "ATTENDEE;CN=Sam Lee;PARTSTAT=ACCEPTED:mailto:sam@example.com" END:VEVENT
item work BEGIN:VEVENT UID:guild DTSTAMP:20250101T000000Z \
  "DTSTART;TZID=$tz:$(day 8)T160000" "DTEND;TZID=$tz:$(day 8)T170000" "SUMMARY:Platform guild" \
  "RRULE:FREQ=WEEKLY;INTERVAL=2" \
  "ORGANIZER;CN=Sam Lee:mailto:sam@example.com" \
  "ATTENDEE;CN=Demo User;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:me@example.com" END:VEVENT
item personal BEGIN:VEVENT UID:housewarming DTSTAMP:20250101T000000Z \
  "DTSTART;TZID=$tz:$(day 12)T190000" "DTEND;TZID=$tz:$(day 12)T230000" "SUMMARY:Housewarming" \
  "LOCATION:12 Mill Lane" \
  "ORGANIZER;CN=Jo Park:mailto:jo@example.com" \
  "ATTENDEE;CN=Demo User;PARTSTAT=TENTATIVE:mailto:me@example.com" END:VEVENT

# journal entries (as jtx Board writes them): dated, and an undated note
item personal BEGIN:VJOURNAL UID:journal-run DTSTAMP:20250101T000000Z CREATED:20250101T000000Z \
  "DTSTART;VALUE=DATE:$(day 1)" "SUMMARY:First 10k" STATUS:FINAL \
  "DESCRIPTION:First 10k\nRan the river loop in 58 minutes. Knees fine.\nNext: try the hill route." END:VJOURNAL
item work BEGIN:VJOURNAL UID:journal-review DTSTAMP:20250101T000000Z CREATED:20250101T000000Z \
  "DTSTART;VALUE=DATE:$(day 2)" "SUMMARY:Design review notes" STATUS:FINAL \
  "DESCRIPTION:Agreed to ship the sync engine behind a flag.\nPriya to write the migration plan." END:VJOURNAL
item personal BEGIN:VJOURNAL UID:note-gifts DTSTAMP:20250101T000000Z "CREATED:$(day 0)T090000Z" \
  STATUS:FINAL "DESCRIPTION:Gift ideas\nJo: climbing chalk bag\nKim: a good bread knife" END:VJOURNAL
echo "$out"

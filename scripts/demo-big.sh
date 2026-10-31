#!/usr/bin/env bash
# Write big fake collections to measure rical with: a calendar, address
# books and Maildirs, of sizes chosen on the command line.
#   scripts/demo-big.sh DIR EVENTS CARDS MESSAGES
# e.g. scripts/demo-big.sh /var/tmp/rical-big 50000 20000 50000 (about 470 MB).
# One event in ten repeats weekly for 20 weeks; mail is in conversations of
# four, half of it in home/INBOX. See scripts/perf.sh to time the apps on it.
set -euo pipefail
usage="usage: demo-big.sh DIR EVENTS CARDS MESSAGES"
d=${1:?$usage} ev=${2:?$usage} cards=${3:?$usage} msgs=${4:?$usage}
rm -rf "$d"; mkdir -p "$d"/cal/{a,b,c} "$d"/cards/{p,w}
for c in a b c; do printf '%s\n' "$c" > "$d/cal/$c/displayname"; done
for a in work home; do for f in INBOX Sent Archive Drafts Trash; do mkdir -p "$d/mail/$a/$f"/{cur,new,tmp}; done; done
awk -v d="$d" -v ev="$ev" -v cards="$cards" -v msgs="$msgs" 'BEGIN{
 for(i=0;i<ev;i++){c=substr("abc",i%3+1,1); f=d"/cal/"c"/e"i".ics";
  y=2025+int(i/(ev/2+1)); day=sprintf("%d%02d%02d",y,(i%12)+1,(i%28)+1);
  rr=(i%10==0)?"RRULE:FREQ=WEEKLY;COUNT=20\r\n":"";
  printf "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//x//EN\r\nBEGIN:VTIMEZONE\r\nTZID:Europe/London\r\nBEGIN:STANDARD\r\nDTSTART:19701025T020000\r\nTZOFFSETFROM:+0100\r\nTZOFFSETTO:+0000\r\nRRULE:FREQ=YEARLY;BYMONTH=10;BYDAY=-1SU\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e%d\r\nDTSTAMP:20250101T000000Z\r\nDTSTART;TZID=Europe/London:%sT0900%02d\r\nDTEND;TZID=Europe/London:%sT1000%02d\r\nSUMMARY:Event number %d with a longer title\r\nDESCRIPTION:Some description text that goes on for a while\\, with details\\nand lines.\r\nLOCATION:Room %d\r\nORGANIZER;CN=Ann:mailto:ann@example.com\r\nATTENDEE;CN=Bob;PARTSTAT=ACCEPTED:mailto:bob@example.com\r\n%sBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n", i, day, i%60, day, i%60, i, i%40, rr > f; close(f)}
 for(i=0;i<cards;i++){b=(i%2)?"p":"w"; f=d"/cards/"b"/c"i".vcf";
  printf "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:c%d\r\nN:Person%d;First%d;;;\r\nFN:First%d Person%d\r\nEMAIL;TYPE=work:p%d@example.com\r\nTEL;TYPE=cell:+44 7700 9%05d\r\nORG:Company %d;\r\nADR;TYPE=home:;;%d High St;Town;;AB1 2CD;UK\r\nNOTE:Met at the conference.\r\nBDAY:1980-%02d-%02d\r\nEND:VCARD\r\n", i,i,i,i,i,i,i,i%50,i,(i%12)+1,(i%28)+1 > f; close(f)}
 base=1767225600;
 for(i=0;i<msgs;i++){
  # most in work INBOX and Archive, some at home
  r=i%10; fo=(r<5)?"home/INBOX":(r<8)?"home/Archive":(r<9)?"work/INBOX":"work/Sent";
  t=base-i*600; fl=(i%7==0)?"":"S";
  f=d"/mail/"fo"/cur/"t"."i".host:2,"fl;
  th=int(i/4);
  printf "From: Person %d <p%d@example.com>\nTo: me@example.com\nSubject: %sThread %d about the plan\nDate: %s\nMessage-ID: <m%d@example.com>\n%sContent-Type: text/plain; charset=utf-8\n\nHello,\n\nThis is message %d. It has a few lines of text\nso that the preview has something to show, and\nthe body is a realistic size for a short mail.\n\nThanks,\nPerson %d\n", i%300, i%300, (i%4)?"Re: ":"", th, strftime("%a, %d %b %Y %H:%M:%S +0000", t, 1), i, (i%4)?"In-Reply-To: <m" (i-1) "@example.com>\n":"", i, i%300 > f; close(f)}
}'
echo "$d: $(find "$d/cal" -name '*.ics' | wc -l) events, $(find "$d/cards" -name '*.vcf' | wc -l) cards, $(find "$d/mail" -type f | wc -l) messages"

#!/usr/bin/env bash
# Write demo address books, with a few duplicates to tidy, for screenshots
# and trying rical-contacts without real data:  scripts/demo-contacts.sh /tmp/c
set -euo pipefail
out=${1:?usage: demo-contacts.sh DIR}
rm -rf "$out"
mkdir -p "$out"/{personal,work}
printf 'Personal\n' > "$out/personal/displayname"
printf 'Work\n' > "$out/work/displayname"

card() { # book uid version lines...
  local book=$1 uid=$2 version=$3; shift 3
  {
    printf 'BEGIN:VCARD\r\nVERSION:%s\r\nUID:%s\r\n' "$version" "$uid"
    for line in "$@"; do printf '%s\r\n' "$line"; done
    printf 'END:VCARD\r\n'
  } > "$out/$book/$uid.vcf"
}

card personal priya 3.0 'N:Shah;Priya;;;' 'FN:Priya Shah' 'ORG:Northwind;' 'TITLE:Product lead' \
  'item1.EMAIL;type=INTERNET;type=pref:priya@example.com' 'item1.X-ABLabel:_$!<Work>!$_' \
  'TEL;type=CELL;type=VOICE;type=pref:+44 7700 900123' \
  'ADR;type=HOME:;;14 Park Row;Leeds;;LS1 5HN;United Kingdom' \
  'BDAY:1990-01-31' 'NOTE:Met at the Leeds meetup. Prefers mornings.' 'CATEGORIES:friends,climbing'
card personal sam 4.0 'N:Lee;Sam;;;' 'FN:Sam Lee' 'EMAIL;TYPE=home:sam@example.org' \
  'TEL;TYPE=cell:07700 900456' 'URL:https://samlee.example.org' 'BDAY:--0612'
card personal jo 4.0 'N:Park;Jo;;;' 'FN:Jo Park' 'EMAIL:jo.park@example.net' 'TEL;TYPE=home:0113 496 0000'
card personal amara 4.0 'N:Okafor;Amara;;;' 'FN:Amara Okafor' 'EMAIL:amara@example.com' \
  'TEL;TYPE=cell:+44 7700 900789' 'ADR;TYPE=home:;;2 Mill Lane;York;;YO1 7HH;UK' 'NICKNAME:Mara'
card personal tom 2.1 'N:Becker;Tom' 'FN:Tom Becker' 'TEL;CELL:+49 30 901820' 'EMAIL;INTERNET:tom@example.de'
card personal lena 4.0 'N:Fischer;Lena;;;' 'FN:Lena Fischer' 'EMAIL:lena@example.de' 'ORG:Fischer Design'
card personal hiro 4.0 'N:Tanaka;Hiroshi;;;' 'FN:Hiroshi Tanaka' 'NICKNAME:Hiro' 'EMAIL:hiro@example.jp' \
  'IMPP:xmpp:hiro@example.jp'
card personal grace 4.0 'N:Hopper;Grace;;;' 'FN:Grace Hopper' 'EMAIL:grace@example.com' 'BDAY:1906-12-09'
card personal nadia 4.0 'N:Rahman;Nadia;;;' 'FN:Nadia Rahman' 'TEL;TYPE=cell:07700 900321'
card personal oli 4.0 'N:Evans;Oliver;;;' 'FN:Oliver Evans' 'EMAIL:oli@example.co.uk' 'CATEGORIES:family'
card personal mum 4.0 'N:Jeffery;Mum;;;' 'FN:Mum' 'TEL;TYPE=home:01632 960001' 'CATEGORIES:family'
card personal plumber 4.0 'FN:Ace Plumbing' 'ORG:Ace Plumbing Ltd' 'TEL;TYPE=work:0113 496 0999' 'KIND:org'
# duplicates: Priya again, from work (the same email, more detail)
card work priya-work 3.0 'N:Shah;Priya;;;' 'FN:Priya Shah' 'ORG:Northwind;Platform' \
  'EMAIL;type=INTERNET;type=WORK:Priya@Example.com' 'TEL;type=WORK:+44 113 496 0123' \
  'BDAY:1990-01-30'
# Sam again by number, an older card with a different spelling
card work samuel 3.0 'N:Lee;Samuel;;;' 'FN:Samuel Lee' 'TEL;type=CELL:+44 7700 900456' 'ORG:Acme'
# two Alex Morgans who aren't the same person (name only)
card work alex-1 4.0 'N:Morgan;Alex;;;' 'FN:Alex Morgan' 'EMAIL:alex.m@acme.example' 'ORG:Acme'
card personal alex-2 4.0 'N:Morgan;Alex;;;' 'FN:Alex Morgan' 'EMAIL:alexmorgan@example.org'
card work dev 4.0 'N:Patel;Dev;;;' 'FN:Dev Patel' 'EMAIL:dev@acme.example' 'TITLE:Engineer' 'ORG:Acme'
card work wei 4.0 'N:Chen;Wei;;;' 'FN:Wei Chen' 'EMAIL:wei@acme.example' 'TEL;TYPE=work:+1 415 555 0134' \
  'ADR;TYPE=work:;;1 Market St;San Francisco;CA;94105;USA'
card work zoe 4.0 'N:Clarke;Zoë;;;' 'FN:Zoë Clarke' 'EMAIL:zoe@acme.example'
echo "$out"

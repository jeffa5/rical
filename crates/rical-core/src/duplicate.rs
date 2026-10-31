//! Duplicating an event, task or journal entry: a copy with a new UID in a
//! file of its own, beside the original.
//!
//! A series is copied whole, its moved and changed instances with it. The
//! copy has no guests: written with them, a scheduling server would send
//! each an invitation the moment it syncs. A task's copy is not done yet.

use anyhow::{Context, bail};

use crate::{
    Occurrence, Vdir,
    doc::{Document, Line},
    write::{Change, read, stamp},
};

impl Vdir {
    /// Copy `occ`'s item (the whole series) as a new one in the same
    /// calendar. Returns the change (to undo it) and the copy's UID.
    pub fn duplicate(&mut self, occ: &Occurrence) -> anyhow::Result<(Change, String)> {
        let doc = Document::parse(&read(&occ.path)?)?;
        let parts = doc
            .items()
            .filter(|c| c.uid() == Some(occ.uid.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if parts.is_empty() {
            bail!("{:?} no longer has it; reload and try again", occ.path);
        }
        let uid = uuid::Uuid::new_v4().to_string();
        let now = stamp();
        let mut new = Document::new_calendar();
        for mut c in parts {
            c.set(Line::property("UID", &[], &uid));
            c.set(Line::property("DTSTAMP", &[], &now));
            c.set(Line::property("CREATED", &[], &now));
            c.set(Line::property("LAST-MODIFIED", &[], &now));
            c.set(Line::new("SEQUENCE:0"));
            c.remove("ORGANIZER");
            c.remove("ATTENDEE");
            if c.name == "VTODO" {
                c.set(Line::new("STATUS:NEEDS-ACTION"));
                c.remove("COMPLETED");
                c.remove("PERCENT-COMPLETE");
            }
            new.push(c);
        }
        self.add_timezones(&mut new);
        let dir = occ.path.parent().context("item has no folder")?;
        let change = Change {
            path: dir.join(format!("{uid}.ics")),
            before: None,
            after: Some(new.write()),
        };
        self.apply_checked(&change)?;
        Ok((change, uid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Kind;
    use chrono_tz::Tz;

    #[test]
    fn a_copy_of_the_whole_series_without_guests() {
        let d = tempfile::tempdir().unwrap();
        let cal = d.path().join("home");
        std::fs::create_dir(&cal).unwrap();
        std::fs::write(
            cal.join("s.ics"),
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s\r\nDTSTART:20250310T090000Z\r\n\
             DURATION:PT1H\r\nRRULE:FREQ=DAILY;COUNT=3\r\nSUMMARY:Standup\r\n\
             ORGANIZER:mailto:me@example.com\r\nATTENDEE:mailto:sam@example.com\r\n\
             BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT5M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nUID:s\r\nRECURRENCE-ID:20250311T090000Z\r\n\
             DTSTART:20250311T100000Z\r\nDURATION:PT1H\r\nSUMMARY:Standup, later\r\n\
             END:VEVENT\r\nEND:VCALENDAR\r\n\
             ",
        )
        .unwrap();
        std::fs::write(
            cal.join("t.ics"),
            "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:t\r\nSUMMARY:Pay rent\r\n\
             STATUS:COMPLETED\r\nCOMPLETED:20250301T000000Z\r\nEND:VTODO\r\nEND:VCALENDAR\r\n",
        )
        .unwrap();
        let mut v = Vdir::load(d.path()).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2025, 3, 10)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let range = day..day + chrono::TimeDelta::days(5);
        let occ = v.query(&range, &[Kind::Event], Tz::UTC)[0].clone();
        let (change, uid) = v.duplicate(&occ).unwrap();
        let text = change.after.as_deref().unwrap();
        assert!(!text.contains("ATTENDEE") && !text.contains("ORGANIZER"));
        assert!(text.contains("BEGIN:VALARM"));

        // every instance twice, the moved one moved in both
        let all = v.query(&range, &[Kind::Event], Tz::UTC);
        assert_eq!(all.len(), 6);
        let copies: Vec<_> = all.iter().filter(|o| o.uid == uid).collect();
        assert_eq!(copies.len(), 3);
        assert!(copies.iter().any(|o| o.summary == "Standup, later"));

        // a task's copy is still to do
        let task = v.tasks().pop().unwrap();
        let occ = v
            .task(&task.path, &task.uid)
            .unwrap()
            .on_day(day.date_naive());
        let (_, uid) = v.duplicate(&occ).unwrap();
        let copy = v.tasks().into_iter().find(|t| t.uid == uid).unwrap();
        assert!(!copy.is_done());
        assert_eq!(copy.summary, "Pay rent");

        // and it undoes
        v.undo(&change).unwrap();
        assert_eq!(v.query(&range, &[Kind::Event], Tz::UTC).len(), 3);
    }
}

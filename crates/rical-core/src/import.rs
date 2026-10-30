//! Importing an `.ics` file: an export, a booking confirmation, or an
//! email invitation (iTIP: METHOD REQUEST, REPLY or CANCEL). First a plan
//! says what would happen to each item, for a preview; then it is applied
//! as one undoable set of changes.

use anyhow::Context as _;

use crate::{
    Vdir,
    doc::{Component, Document, Line},
    when::When,
    write::{Change, make_override, read},
};

/// What importing one item (a UID, with any instance overrides) would do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// New: added to the chosen calendar.
    Add,
    /// A newer version of an item already in this file: replaces it.
    Update(std::path::PathBuf),
    /// Already there and not older: nothing to do.
    UpToDate(std::path::PathBuf),
    /// A guest's answer to my invitation: their replies are set on my copy.
    Reply {
        path: std::path::PathBuf,
        /// (who, PARTSTAT) for each guest in the reply.
        replies: Vec<(String, String)>,
    },
    /// The organiser cancelled it (or one instance of it).
    Cancel(std::path::PathBuf),
    /// Cannot be done: why.
    Skip(String),
}

#[derive(Debug, Clone)]
pub struct Planned {
    pub uid: String,
    pub summary: String,
    pub start: Option<When>,
    /// ORGANIZER, as a name or address.
    pub organizer: Option<String>,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// The iTIP METHOD, upper-cased, if any.
    pub method: Option<String>,
    pub items: Vec<Planned>,
    incoming: Document,
}

impl Plan {
    /// Whether anything would change.
    pub fn has_changes(&self) -> bool {
        self.items
            .iter()
            .any(|p| !matches!(p.action, Action::UpToDate(_) | Action::Skip(_)))
    }

    /// Whether anything would be added (and so needs a calendar).
    pub fn adds(&self) -> bool {
        self.items.iter().any(|p| p.action == Action::Add)
    }

    /// Addresses of the guests in the file, to pick the calendar of the
    /// account that was invited.
    pub fn attendee_emails(&self) -> Vec<String> {
        self.incoming
            .items()
            .flat_map(|c| c.properties("ATTENDEE"))
            .map(email_of)
            .collect()
    }
}

fn email_of(l: &Line) -> String {
    let v = l.value().trim();
    v.get(..7)
        .filter(|p| p.eq_ignore_ascii_case("mailto:"))
        .map_or(v, |_| &v[7..])
        .to_lowercase()
}

fn sequence(c: &Component) -> u32 {
    c.property("SEQUENCE")
        .and_then(|l| l.value().trim().parse().ok())
        .unwrap_or(0)
}

/// The components of each UID in `doc`, in file order.
fn groups(doc: &Document) -> Vec<(String, Vec<&Component>)> {
    let mut out: Vec<(String, Vec<&Component>)> = Vec::new();
    for c in doc.items() {
        let uid = c.uid().unwrap_or_default().to_owned();
        match out.iter_mut().find(|(u, _)| *u == uid) {
            Some((_, v)) => v.push(c),
            None => out.push((uid, vec![c])),
        }
    }
    out
}

/// A file name for `uid` that is safe everywhere.
fn file_name(uid: &str) -> String {
    let safe = uid
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "@._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{safe}.ics")
}

impl Vdir {
    /// The file holding `uid`, in any calendar.
    pub(crate) fn path_of(&self, uid: &str) -> Option<std::path::PathBuf> {
        self.components()
            .find(|(_, c)| c.get_uid() == Some(uid))
            .map(|(_, c)| c.path().to_owned())
    }

    /// What importing `text` would do.
    pub fn plan_import(&self, text: &str) -> anyhow::Result<Plan> {
        let incoming = Document::parse(text).context("not an iCalendar file")?;
        let method = incoming
            .root
            .property("METHOD")
            .map(|l| l.value().trim().to_ascii_uppercase());
        let mut items = Vec::new();
        for (uid, comps) in groups(&incoming) {
            let master = comps
                .iter()
                .find(|c| c.recurrence_id().is_none())
                .unwrap_or(&comps[0]);
            let text = |name| {
                master
                    .property(name)
                    .map(|l| crate::occurrence::unescape_text(l.value()))
            };
            let organizer = master.property("ORGANIZER").map(|l| {
                l.param("CN")
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| email_of(l))
            });
            let existing = (!uid.is_empty()).then(|| self.path_of(&uid)).flatten();
            let action = match (method.as_deref(), existing) {
                (_, _) if uid.is_empty() => Action::Skip("it has no UID".into()),
                (Some("REPLY"), Some(path)) => Action::Reply {
                    path,
                    replies: comps
                        .iter()
                        .flat_map(|c| c.properties("ATTENDEE"))
                        .map(|l| {
                            (
                                l.param("CN")
                                    .filter(|n| !n.is_empty())
                                    .unwrap_or_else(|| email_of(l)),
                                l.param("PARTSTAT").unwrap_or_else(|| "NEEDS-ACTION".into()),
                            )
                        })
                        .collect(),
                },
                (Some("REPLY"), None) => {
                    Action::Skip("a reply to an event not in your calendars".into())
                }
                (Some("CANCEL"), Some(path)) => Action::Cancel(path),
                (Some("CANCEL"), None) => {
                    Action::Skip("cancels an event not in your calendars".into())
                }
                (Some(m), _) if !matches!(m, "PUBLISH" | "REQUEST" | "ADD") => {
                    Action::Skip(format!("{m} messages are not handled"))
                }
                (_, None) => Action::Add,
                (_, Some(path)) => {
                    let newer = self
                        .get(&path)
                        .and_then(|cs| cs.iter().find(|c| c.get_uid() == Some(uid.as_str())))
                        .map(|c| {
                            let seq = c
                                .property_value("SEQUENCE")
                                .and_then(|s| s.trim().parse::<u32>().ok())
                                .unwrap_or(0);
                            let stamp = c.property_value("DTSTAMP").unwrap_or("").to_owned();
                            let in_stamp = master.property("DTSTAMP").map(|l| l.value().to_owned());
                            sequence(master) > seq
                                || (sequence(master) == seq && in_stamp.is_some_and(|s| s > stamp))
                        })
                        .unwrap_or(true);
                    if newer {
                        Action::Update(path)
                    } else {
                        Action::UpToDate(path)
                    }
                }
            };
            items.push(Planned {
                summary: text("SUMMARY").unwrap_or_default(),
                start: master.property("DTSTART").and_then(Line::when),
                organizer,
                uid,
                action,
            });
        }
        Ok(Plan {
            method,
            items,
            incoming,
        })
    }

    /// Apply `plan`, adding new items to `calendar`.
    pub fn import(&mut self, plan: &Plan, calendar: &str) -> anyhow::Result<Vec<Change>> {
        let groups = groups(&plan.incoming);
        let comps_of = |uid: &str| {
            groups
                .iter()
                .find(|(u, _)| u == uid)
                .map(|(_, c)| c.clone())
                .unwrap_or_default()
        };
        let mut changes = Vec::new();
        for item in &plan.items {
            let comps = comps_of(&item.uid);
            let change = match &item.action {
                Action::Add => {
                    let dir = self
                        .calendar(calendar)
                        .with_context(|| format!("no calendar {calendar:?}"))?
                        .path
                        .clone();
                    let mut path = dir.join(file_name(&item.uid));
                    let mut n = 1;
                    while path.exists() {
                        n += 1;
                        path = dir.join(file_name(&format!("{}-{n}", item.uid)));
                    }
                    let mut doc = Document::new_calendar();
                    self.fill(&mut doc, plan, &comps);
                    Change {
                        path,
                        before: None,
                        after: Some(doc.write()),
                    }
                }
                Action::Update(path) => {
                    let before = read(path)?;
                    let mut doc = Document::parse(&before)?;
                    let uid = item.uid.clone();
                    doc.remove_where(|c| c.uid() == Some(uid.as_str()));
                    self.fill(&mut doc, plan, &comps);
                    Change {
                        path: path.clone(),
                        before: Some(before),
                        after: Some(doc.write()),
                    }
                }
                Action::Reply { path, .. } => {
                    let before = read(path)?;
                    let mut doc = Document::parse(&before)?;
                    for c in &comps {
                        let rid = c.recurrence_id();
                        // the instance replied about, else the series
                        let has_rid = rid.is_some() && doc.find(&item.uid, rid.as_ref()).is_some();
                        let target = doc
                            .find_mut(&item.uid, if has_rid { rid.as_ref() } else { None })
                            .context("item not found in file")?;
                        for reply in c.properties("ATTENDEE") {
                            let who = email_of(reply);
                            let status = reply
                                .param("PARTSTAT")
                                .unwrap_or_else(|| "NEEDS-ACTION".into());
                            for i in target.items.iter_mut() {
                                if let crate::doc::Item::Line(l) = i
                                    && l.name() == "ATTENDEE"
                                    && email_of(l) == who
                                {
                                    *l = l.with_param("PARTSTAT", &status);
                                }
                            }
                        }
                    }
                    Change {
                        path: path.clone(),
                        before: Some(before),
                        after: Some(doc.write()),
                    }
                }
                Action::Cancel(path) => {
                    let before = read(path)?;
                    let mut doc = Document::parse(&before)?;
                    for c in &comps {
                        let rid = c.recurrence_id();
                        if let Some(rid) = &rid
                            && doc.find(&item.uid, Some(rid)).is_none()
                        {
                            // one instance of a series: give it an override
                            let master = doc
                                .find(&item.uid, None)
                                .context("series has no master component")?;
                            let o = make_override(master, rid);
                            doc.push(o);
                        }
                        let target = doc
                            .find_mut(&item.uid, rid.as_ref())
                            .context("item not found in file")?;
                        target.set(Line::new("STATUS:CANCELLED"));
                    }
                    Change {
                        path: path.clone(),
                        before: Some(before),
                        after: Some(doc.write()),
                    }
                }
                Action::UpToDate(_) | Action::Skip(_) => continue,
            };
            self.apply_checked(&change)?;
            changes.push(change);
        }
        Ok(changes)
    }

    /// Put `comps` (and the time zones they use) into `doc`. A stored
    /// calendar object has no METHOD, so the file's own VCALENDAR lines are
    /// not copied.
    fn fill(&self, doc: &mut Document, plan: &Plan, comps: &[&Component]) {
        for c in comps {
            let mut c = (*c).clone();
            c.detach();
            doc.push(c);
        }
        let tzids = comps
            .iter()
            .flat_map(|c| c.lines())
            .filter_map(|l| l.param("TZID"))
            .collect::<Vec<_>>();
        for tzid in tzids {
            if !doc.has_timezone(&tzid)
                && let Some(tz) = plan.incoming.timezone(&tzid)
            {
                doc.add_timezone(tz.clone());
            }
        }
        self.add_timezones(doc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVITE: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nPRODID:x\r\nVERSION:2.0\r\n\
        BEGIN:VEVENT\r\nUID:inv-1@example.com\r\nSEQUENCE:0\r\nDTSTAMP:20261001T100000Z\r\n\
        DTSTART:20261101T100000Z\r\nDTEND:20261101T110000Z\r\nSUMMARY:Lunch\r\n\
        ORGANIZER;CN=Sam:mailto:sam@example.com\r\n\
        ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@example.com\r\n\
        ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.com\r\n\
        END:VEVENT\r\nEND:VCALENDAR\r\n";

    fn vdir() -> (tempfile::TempDir, Vdir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("cal")).unwrap();
        std::fs::write(dir.path().join("cal/displayname"), "Cal").unwrap();
        let v = Vdir::load(dir.path()).unwrap();
        (dir, v)
    }

    #[test]
    fn invitation_added_then_updated_then_cancelled() {
        let (dir, mut v) = vdir();
        let plan = v.plan_import(INVITE).unwrap();
        assert_eq!(plan.method.as_deref(), Some("REQUEST"));
        assert_eq!(plan.items[0].action, Action::Add);
        assert_eq!(plan.items[0].organizer.as_deref(), Some("Sam"));
        assert!(
            plan.attendee_emails()
                .contains(&"me@example.com".to_owned())
        );
        v.import(&plan, "cal").unwrap();
        let path = dir.path().join("cal/inv-1@example.com.ics");
        let stored = std::fs::read_to_string(&path).unwrap();
        // stored objects carry no METHOD
        assert!(!stored.contains("METHOD"));
        assert!(stored.contains("SUMMARY:Lunch"));

        // the same again: nothing to do
        let again = v.plan_import(INVITE).unwrap();
        assert!(matches!(again.items[0].action, Action::UpToDate(_)));
        assert!(!again.has_changes());

        // a newer version replaces it
        let moved = INVITE
            .replace("SEQUENCE:0", "SEQUENCE:1")
            .replace("T100000Z\r\nDTEND", "T120000Z\r\nDTEND");
        let plan = v.plan_import(&moved).unwrap();
        assert_eq!(plan.items[0].action, Action::Update(path.clone()));
        v.import(&plan, "cal").unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("DTSTART:20261101T120000Z")
        );

        // and a cancellation marks it cancelled
        let cancel = moved.replace("METHOD:REQUEST", "METHOD:CANCEL");
        let plan = v.plan_import(&cancel).unwrap();
        assert_eq!(plan.items[0].action, Action::Cancel(path.clone()));
        v.import(&plan, "cal").unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("STATUS:CANCELLED")
        );
    }

    #[test]
    fn replies_set_the_guests_answer() {
        let (dir, mut v) = vdir();
        let mine = INVITE
            .replace("METHOD:REQUEST\r\n", "")
            .replace("CN=Sam:mailto:sam", "CN=Me:mailto:me")
            .replace(
                "ATTENDEE;CN=Sam;PARTSTAT=ACCEPTED:mailto:sam@example.com\r\n",
                "",
            )
            .replace(
                "ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:me@example.com",
                "ATTENDEE;CN=Jo;PARTSTAT=NEEDS-ACTION:mailto:jo@example.com",
            );
        let path = dir.path().join("cal/inv-1@example.com.ics");
        std::fs::write(&path, &mine).unwrap();
        let mut v2 = Vdir::load(dir.path()).unwrap();
        std::mem::swap(&mut v, &mut v2);

        let reply = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\n\
            UID:inv-1@example.com\r\nDTSTAMP:20261002T100000Z\r\n\
            ATTENDEE;CN=Jo;PARTSTAT=ACCEPTED:mailto:JO@example.com\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let plan = v.plan_import(reply).unwrap();
        let Action::Reply { replies, .. } = &plan.items[0].action else {
            panic!("{:?}", plan.items[0].action)
        };
        assert_eq!(replies, &[("Jo".to_owned(), "ACCEPTED".to_owned())]);
        v.import(&plan, "cal").unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("ATTENDEE;CN=Jo;PARTSTAT=ACCEPTED:mailto:jo@example.com"));
    }

    #[test]
    fn replies_for_unknown_events_are_skipped() {
        let (_dir, v) = vdir();
        let plan = v
            .plan_import(&INVITE.replace("METHOD:REQUEST", "METHOD:REPLY"))
            .unwrap();
        assert!(matches!(plan.items[0].action, Action::Skip(_)));
        assert!(v.plan_import("not a calendar").is_err());
    }
}

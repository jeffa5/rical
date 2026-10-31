//! Importing `.ics` files opened with rical (say, an invitation from a mail
//! client): a preview of what each item would do, a calendar for new ones,
//! then one undoable import.

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};
use rical_core::import::{Action, Plan};

use super::Window;
use crate::format;

/// What a file is, from its iTIP METHOD.
fn describe(plan: &Plan, name: &str) -> String {
    let from = plan
        .items
        .iter()
        .find_map(|p| p.organizer.clone())
        .unwrap_or_else(|| "someone".into());
    match plan.method.as_deref() {
        Some("REQUEST") => format!("Invitation from {from}"),
        Some("REPLY") => "A reply to your invitation".into(),
        Some("CANCEL") => format!("Cancelled by {from}"),
        _ => format!("Events in {name}"),
    }
}

fn action_text(a: &Action) -> String {
    match a {
        Action::Add => "New: will be added".into(),
        Action::Update(_) => "A newer version: replaces yours".into(),
        Action::UpToDate(_) => "Already in your calendar".into(),
        Action::Reply { replies, .. } => replies
            .iter()
            .map(|(who, status)| format!("{who}: {}", format::reply(status)))
            .collect::<Vec<_>>()
            .join(", "),
        Action::Cancel(_) => "Will be marked cancelled".into(),
        Action::Skip(why) => format!("Skipped: {why}"),
    }
}

impl Window {
    /// Preview and import `files`.
    pub fn import_files(self: &Rc<Self>, files: &[gio::File]) {
        // what is imported is compared with what is there: wait for it
        if !self.state.borrow().loaded {
            self.waiting_files
                .borrow_mut()
                .extend(files.iter().cloned());
            return;
        }
        let mut plans = Vec::new();
        for file in files {
            let name = file
                .basename()
                .map_or_else(|| file.uri().to_string(), |b| b.display().to_string());
            // read now: mail clients delete their temporary copies
            let text = file
                .load_contents(None::<&gio::Cancellable>)
                .map_err(|e| e.to_string())
                .and_then(|(bytes, _)| {
                    String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
                });
            let plan = text.and_then(|t| {
                self.state
                    .borrow()
                    .vdir
                    .plan_import(&t)
                    .map_err(|e| format!("{e:#}"))
            });
            match plan {
                Ok(p) => plans.push((name, p)),
                Err(e) => self.toast(&format!("Could not import {name}: {e}")),
            }
        }
        if !plans.is_empty() {
            self.import_dialog(plans);
        }
    }

    fn import_dialog(self: &Rc<Self>, plans: Vec<(String, Plan)>) {
        let state = self.state.borrow();
        let local = state.local;
        let page = adw::PreferencesPage::new();

        for (name, plan) in &plans {
            let group = adw::PreferencesGroup::builder()
                .title(glib::markup_escape_text(&describe(plan, name)))
                .description(glib::markup_escape_text(name))
                .build();
            for item in &plan.items {
                let when = item
                    .start
                    .map(|w| format::input(&w, local))
                    .unwrap_or_default();
                let title = if item.summary.is_empty() {
                    "(untitled)"
                } else {
                    &item.summary
                };
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(title))
                    .subtitle(glib::markup_escape_text(&format!(
                        "{when}{}{}",
                        if when.is_empty() { "" } else { " · " },
                        action_text(&item.action)
                    )))
                    .build();
                let icon = match item.action {
                    Action::Add => "list-add-symbolic",
                    Action::Update(_) => "view-refresh-symbolic",
                    Action::Reply { .. } => "mail-reply-sender-symbolic",
                    Action::Cancel(_) => "window-close-symbolic",
                    Action::UpToDate(_) => "object-select-symbolic",
                    Action::Skip(_) => "dialog-warning-symbolic",
                };
                row.add_prefix(&gtk::Image::from_icon_name(icon));
                group.add(&row);
            }
            page.add(&group);
        }

        // new items go in a calendar: the invited account's if it can be
        // told, else the usual one
        let calendars = state
            .vdir
            .calendars
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect::<Vec<_>>();
        let invited = plans
            .iter()
            .flat_map(|(_, p)| p.attendee_emails())
            .collect::<Vec<_>>();
        let senders = state.vdir.senders();
        let owners = state.vdir.owners();
        let invited_here = |id: &String| {
            [senders.get(id), owners.get(id)]
                .into_iter()
                .flatten()
                .any(|e| invited.contains(e))
        };
        let usual = state.default_calendar();
        let preferred = match usual.as_ref().filter(|u| invited_here(u)) {
            Some(u) => Some(u.clone()),
            None => calendars
                .iter()
                .map(|(id, _)| id)
                .find(|id| invited_here(id))
                .cloned()
                .or(usual),
        };
        let adds = plans.iter().any(|(_, p)| p.adds());
        let names = calendars
            .iter()
            .map(|(_, n)| n.as_str())
            .collect::<Vec<_>>();
        let calendar = adw::ComboRow::builder()
            .title("Add new items to")
            .model(&gtk::StringList::new(&names))
            .selected(
                calendars
                    .iter()
                    .position(|(id, _)| Some(id) == preferred.as_ref())
                    .unwrap_or(0) as u32,
            )
            .build();
        if adds {
            let group = adw::PreferencesGroup::new();
            group.add(&calendar);
            page.add(&group);
        }
        drop(state);

        let changes = plans.iter().any(|(_, p)| p.has_changes());
        let cancel = gtk::Button::with_label("Cancel");
        let import = gtk::Button::builder()
            .label(if changes {
                "Import"
            } else {
                "Nothing to import"
            })
            .css_classes(["suggested-action"])
            .sensitive(changes)
            .build();
        let header = adw::HeaderBar::builder()
            .show_end_title_buttons(false)
            .show_start_title_buttons(false)
            .build();
        header.pack_start(&cancel);
        header.pack_end(&import);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&page));
        let dialog = adw::Dialog::builder()
            .title("Import")
            .content_width(520)
            .content_height(480)
            .child(&view)
            .build();

        let d = dialog.clone();
        cancel.connect_clicked(move |_| {
            d.close();
        });
        let (w, d) = (Rc::downgrade(self), dialog.clone());
        import.connect_clicked(move |_| {
            let Some(w) = w.upgrade() else { return };
            d.close();
            let calendar = calendars
                .get(calendar.selected() as usize)
                .map(|(id, _)| id.clone())
                .unwrap_or_default();
            let done = w.perform("import", |v| {
                let mut all = Vec::new();
                for (_, plan) in &plans {
                    all.extend(v.import(plan, &calendar)?);
                }
                Ok(all)
            });
            // show the first thing imported
            let first = plans
                .iter()
                .flat_map(|(_, p)| &p.items)
                .find(|i| !matches!(i.action, Action::Skip(_)) && i.start.is_some());
            if done && let Some(item) = first {
                let start = item.start.unwrap();
                let local = w.state.borrow().local;
                w.state.borrow_mut().date = start.date(local);
                w.reselect(&item.uid, &start);
            }
        });
        dialog.set_focus(Some(&import));
        dialog.present(Some(&self.window));
    }
}

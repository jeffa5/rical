//! A person's card: who someone on an event is, from your contacts, with
//! ways to reach them. It is the contacts app's card (`rical_ui::card`);
//! editing is left to that app, which the card opens.

use std::path::Path;

use adw::prelude::*;
use gtk::gio;
use rical_core::Person;

use crate::{details::open_uri, state::State};

/// The contacts app, when it is installed.
const CONTACTS_APP: &str = "net.jeffas.Rical.Contacts.desktop";

/// Show `person` (from an event) in a dialog over `parent`.
pub fn show(parent: &impl IsA<gtk::Widget>, state: &State, person: &Person) {
    let email = person.email.clone().unwrap_or_default();
    let name = state.name_of(person);
    let window = parent.as_ref().root().and_downcast::<gtk::Window>();
    let today = chrono::Local::now().date_naive();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(6)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    let known = state.with_card(&email, |card, id| {
        content.append(&rical_ui::card::heading(card, 64));
        for g in rical_ui::card::groups(card, today) {
            content.append(&g);
        }
        id.path.clone()
    });
    match &known {
        Some(path) => {
            let open = gtk::Button::builder()
                .label(format!("Open in {}", contacts_app_name()))
                .halign(gtk::Align::Center)
                .tooltip_text(path.display().to_string())
                .css_classes(["pill"])
                .build();
            let (path, w) = (path.clone(), window.clone());
            open.connect_clicked(move |_| open_in_contacts(w.as_ref(), &path));
            content.append(&open);
        }
        None => {
            // not a contact: who the event says, and a way to add them
            let who = gtk::Box::builder()
                .orientation(gtk::Orientation::Vertical)
                .spacing(6)
                .build();
            who.append(&adw::Avatar::new(64, Some(&name), true));
            who.append(
                &gtk::Label::builder()
                    .label(&name)
                    .css_classes(["title-2"])
                    .wrap(true)
                    .justify(gtk::Justification::Center)
                    .selectable(true)
                    .can_focus(false)
                    .build(),
            );
            who.append(
                &gtk::Label::builder()
                    .label("Not in your contacts")
                    .css_classes(["dim-label"])
                    .build(),
            );
            content.append(&who);
            if !email.is_empty() {
                let group = adw::PreferencesGroup::new();
                let address = email.to_lowercase();
                group.add(&rical_ui::card::link_row(
                    &address,
                    "",
                    "mail-send-symbolic",
                    "Write an email",
                    Some(format!("mailto:{address}")),
                    &address,
                ));
                content.append(&group);
                let add = gtk::Button::builder()
                    .label(format!("Add to {}", contacts_app_name()))
                    .halign(gtk::Align::Center)
                    .css_classes(["pill"])
                    .build();
                let given = person.name.clone();
                let w = window.clone();
                add.connect_clicked(move |_| {
                    add_to_contacts(w.as_ref(), given.as_deref(), &address);
                });
                content.append(&add);
            }
        }
    }

    let view = adw::ToolbarView::new();
    // the name is the heading below
    view.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .build(),
    ));
    adw::Dialog::builder()
        .title(&name)
        .content_width(420)
        .child(&view)
        .build()
        .present(Some(parent.as_ref()));
}

/// The app contacts open in: rical's own if installed, else the desktop's
/// app for vCards.
fn contacts_app() -> Option<gio::AppInfo> {
    gio::AppInfo::all()
        .into_iter()
        .find(|a| a.id().as_deref() == Some(CONTACTS_APP))
        .or_else(|| gio::AppInfo::default_for_type("text/vcard", false))
}

fn contacts_app_name() -> String {
    contacts_app().map_or_else(|| "Contacts".to_owned(), |a| a.display_name().to_string())
}

/// Open a card file in the contacts app, which shows the card it has (by
/// UID) rather than adding a copy.
fn open_in_contacts(parent: Option<&gtk::Window>, path: &Path) {
    let file = gio::File::for_path(path);
    match contacts_app() {
        Some(app) => {
            let ctx = parent.map(|w| gtk::prelude::WidgetExt::display(w).app_launch_context());
            if let Err(e) = app.launch(&[file], ctx.as_ref()) {
                log::warn!("could not open the contact: {e}");
            }
        }
        None => open_uri(parent, &file.uri()),
    }
}

/// A vCard for someone seen on an event: their name and address.
fn new_card(name: Option<&str>, email: &str) -> String {
    let escape = |s: &str| {
        s.replace('\\', "\\\\")
            .replace(',', "\\,")
            .replace(';', "\\;")
            .replace('\n', "\\n")
    };
    let name = name.filter(|n| !n.trim().is_empty()).unwrap_or(email);
    format!(
        "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:{}\r\nFN:{}\r\nEMAIL:{}\r\nEND:VCARD\r\n",
        gtk::glib::uuid_string_random(),
        escape(name.trim()),
        escape(email),
    )
}

/// Hand a new card to the contacts app, which asks where to add it (or
/// offers to merge it with someone it already has).
fn add_to_contacts(parent: Option<&gtk::Window>, name: Option<&str>, email: &str) {
    let dir = gtk::glib::user_runtime_dir().join("rical");
    let path = dir.join(format!("contact-{}.vcf", gtk::glib::uuid_string_random()));
    let written =
        std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, new_card(name, email)));
    match written {
        Ok(()) => open_in_contacts(parent, &path),
        Err(e) => log::warn!("could not write {path:?}: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_for_guests_parse() {
        let text = new_card(Some("Lee, Sam"), "sam@example.com");
        let (cards, _) = compact_ics::parse(&text).unwrap();
        let card = rical_cards::Card(&cards[0]);
        assert_eq!(card.display_name(), "Lee, Sam");
        assert_eq!(card.emails()[0].value, "sam@example.com");
        assert!(card.uid().is_some_and(|u| u.starts_with("urn:uuid:")));
        // no name: the address
        let text = new_card(None, "jo@example.com");
        assert!(text.contains("\r\nFN:jo@example.com\r\n"));
    }
}

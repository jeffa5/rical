//! A person's card: who someone on an event is, from your contacts, with
//! ways to reach them. Read only; the contacts app edits.

use adw::prelude::*;
use chrono::NaiveDate;
use gtk::gio;
use rical_core::Person;

use crate::{details::open_uri, format, state::State};

/// Show `person` (from an event) in a dialog over `parent`.
pub fn show(parent: &impl IsA<gtk::Widget>, state: &State, person: &Person) {
    let email = person.email.clone().unwrap_or_default();
    let card = state.card_for(&email);
    let name = state.name_of(person);
    let window = parent.as_ref().root().and_downcast::<gtk::Window>();

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .margin_top(6)
        .margin_bottom(18)
        .margin_start(18)
        .margin_end(18)
        .build();

    // who: avatar, name, and what they do
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
    let role = card.as_ref().map(|c| {
        [c.title.as_deref(), c.org.as_deref()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    });
    let role = role
        .filter(|r| !r.is_empty())
        .or_else(|| card.is_none().then(|| "Not in your contacts".to_owned()));
    if let Some(role) = role {
        who.append(
            &gtk::Label::builder()
                .label(&role)
                .css_classes(["dim-label"])
                .wrap(true)
                .justify(gtk::Justification::Center)
                .build(),
        );
    }
    content.append(&who);

    // ways to reach them, each opening its link
    let group = adw::PreferencesGroup::new();
    let link_row = |title: &str, subtitle: &str, icon: &str, tip: &str, uri: String| {
        let row = adw::ActionRow::builder()
            .title(glib_escape(title))
            .subtitle(glib_escape(subtitle))
            .activatable(true)
            .tooltip_text(tip)
            .build();
        row.add_suffix(&gtk::Image::from_icon_name(icon));
        let w = window.clone();
        row.connect_activated(move |_| open_uri(w.as_ref(), &uri));
        row
    };
    let emails = match &card {
        Some(c) => c.emails.clone(),
        None if !email.is_empty() => vec![(email.to_lowercase(), Vec::new())],
        None => Vec::new(),
    };
    for (address, labels) in &emails {
        group.add(&link_row(
            address,
            &labels.join(", "),
            "mail-send-symbolic",
            "Write an email",
            format!("mailto:{address}"),
        ));
    }
    if let Some(c) = &card {
        for (number, labels) in &c.phones {
            let tel = number
                .chars()
                .filter(|ch| ch.is_ascii_digit() || *ch == '+')
                .collect::<String>();
            group.add(&link_row(
                number,
                &labels.join(", "),
                "call-start-symbolic",
                "Call",
                format!("tel:{tel}"),
            ));
        }
        for (address, labels) in &c.addresses {
            group.add(&link_row(
                address,
                &labels.join(", "),
                "mark-location-symbolic",
                "Show on a map",
                format::map_url(address),
            ));
        }
        for url in &c.urls {
            group.add(&link_row(
                url,
                "",
                "web-browser-symbolic",
                "Open",
                url.clone(),
            ));
        }
        if let Some(day) = c.birthday.as_deref().and_then(birthday) {
            group.add(
                &adw::ActionRow::builder()
                    .title(glib_escape(&day))
                    .subtitle("birthday")
                    .build(),
            );
        }
        if let Some(note) = &c.note {
            group.add(
                &adw::ActionRow::builder()
                    .title(glib_escape(note))
                    .subtitle("note")
                    .build(),
            );
        }
    }
    content.append(&group);

    // the whole contact, in the app that opens vCards
    if let Some(c) = &card {
        let app = gio::AppInfo::default_for_type("text/vcard", false);
        let label = app.as_ref().map_or_else(
            || "Open contact".to_owned(),
            |a| format!("Open in {}", a.display_name()),
        );
        let open = gtk::Button::builder()
            .label(&label)
            .halign(gtk::Align::Center)
            .tooltip_text(c.path.display().to_string())
            .css_classes(["pill"])
            .build();
        let path = c.path.clone();
        let w = window.clone();
        open.connect_clicked(move |_| {
            let file = gio::File::for_path(&path);
            gtk::FileLauncher::new(Some(&file)).launch(
                w.as_ref(),
                None::<&gio::Cancellable>,
                |r| {
                    if let Err(e) = r {
                        log::warn!("could not open the contact: {e}");
                    }
                },
            );
        });
        content.append(&open);
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
        .content_width(400)
        .child(&view)
        .build()
        .present(Some(parent.as_ref()));
}

fn glib_escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

/// A vCard birthday in words: "31 January 1990", or "31 January" when
/// the year is left out ("--0131").
fn birthday(value: &str) -> Option<String> {
    let date = value.split('T').next()?.replace('-', "");
    match date.len() {
        // --MMDD, the dashes gone
        4 => {
            let d = NaiveDate::parse_from_str(&format!("2000{date}"), "%Y%m%d").ok()?;
            Some(d.format("%-d %B").to_string())
        }
        8 => {
            let d = NaiveDate::parse_from_str(&date, "%Y%m%d").ok()?;
            Some(d.format("%-d %B %Y").to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn birthdays_in_words() {
        assert_eq!(birthday("1990-01-31").as_deref(), Some("31 January 1990"));
        assert_eq!(birthday("19900131").as_deref(), Some("31 January 1990"));
        assert_eq!(birthday("--0131").as_deref(), Some("31 January"));
        assert_eq!(birthday("--01-31").as_deref(), Some("31 January"));
        assert_eq!(
            birthday("1990-01-31T00:00:00Z").as_deref(),
            Some("31 January 1990")
        );
        assert_eq!(birthday("--0229").as_deref(), Some("29 February"));
        assert_eq!(birthday("soon"), None);
    }
}

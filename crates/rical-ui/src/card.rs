//! A person's card, as both apps show it: who they are, and every way to
//! reach them, each a link (write, call, map, open) with a copy button.

use adw::prelude::*;
use chrono::{Datelike, NaiveDate};
use rical_cards::{Card, Field};

use crate::util::{map_url, open_uri};

fn escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

/// The top of a card: picture-free avatar, name, and what they do.
pub fn heading(card: &Card, size: i32) -> gtk::Box {
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .build();
    let name = card.display_name();
    b.append(&adw::Avatar::new(size, Some(&name), true));
    b.append(
        &gtk::Label::builder()
            .label(&name)
            .css_classes([if size >= 80 { "title-1" } else { "title-2" }])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .selectable(true)
            .can_focus(false)
            .build(),
    );
    let role = [card.title(), card.org().first().cloned()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    if !role.is_empty() {
        b.append(
            &gtk::Label::builder()
                .label(&role)
                .css_classes(["dim-label"])
                .wrap(true)
                .justify(gtk::Justification::Center)
                .build(),
        );
    }
    b
}

/// A row that opens `uri` and can copy `copy`.
pub fn link_row(
    title: &str,
    subtitle: &str,
    icon: &str,
    tip: &str,
    uri: Option<String>,
    copy: &str,
) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(escape(title))
        .subtitle(escape(subtitle))
        .title_selectable(true)
        .build();
    let copy_button = gtk::Button::builder()
        .icon_name("edit-copy-symbolic")
        .tooltip_text("Copy")
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .build();
    let text = copy.to_owned();
    copy_button.connect_clicked(move |b| b.clipboard().set_text(&text));
    row.add_suffix(&copy_button);
    if let Some(uri) = uri {
        row.set_activatable(true);
        row.set_tooltip_text(Some(tip));
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        row.connect_activated(move |r| {
            open_uri(r.root().and_downcast_ref::<gtk::Window>(), &uri);
        });
    } else {
        row.add_prefix(&gtk::Image::from_icon_name(icon));
    }
    row
}

fn labels(f: &Field) -> String {
    let mut l = f.labels.join(", ");
    if f.pref {
        l = if l.is_empty() {
            "preferred".to_owned()
        } else {
            format!("{l} · preferred")
        };
    }
    l
}

/// A birthday in words, and how old they turn this year when the year is
/// known: "31 January 1990 · 36".
pub fn date_words(value: &str, today: NaiveDate) -> Option<String> {
    let date = value.split('T').next()?.replace('-', "");
    match date.len() {
        4 => {
            let d = NaiveDate::parse_from_str(&format!("2000{date}"), "%Y%m%d").ok()?;
            Some(d.format("%-d %B").to_string())
        }
        8 => {
            let d = NaiveDate::parse_from_str(&date, "%Y%m%d").ok()?;
            let age = today.year() - d.year();
            Some(format!("{} · {age} this year", d.format("%-d %B %Y")))
        }
        _ => None,
    }
}

/// Every way to reach someone, and what else the card says, as groups.
pub fn groups(card: &Card, today: NaiveDate) -> Vec<adw::PreferencesGroup> {
    let mut out = Vec::new();
    let reach = adw::PreferencesGroup::new();
    let mut any = false;
    for e in card.emails() {
        any = true;
        reach.add(&link_row(
            &e.value,
            &labels(&e),
            "mail-send-symbolic",
            "Write an email",
            Some(format!("mailto:{}", e.value)),
            &e.value,
        ));
    }
    for p in card.phones() {
        any = true;
        let tel: String = p
            .value
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect();
        reach.add(&link_row(
            &p.value,
            &labels(&p),
            "call-start-symbolic",
            "Call",
            Some(format!("tel:{tel}")),
            &p.value,
        ));
    }
    for c in card.chats() {
        any = true;
        let shown = c.value.split_once(':').map_or(c.value.as_str(), |(_, v)| v);
        reach.add(&link_row(
            shown,
            c.value.split(':').next().unwrap_or(""),
            "chat-message-new-symbolic",
            "Open the chat",
            Some(c.value.clone()),
            shown,
        ));
    }
    if any {
        out.push(reach);
    }
    let places = adw::PreferencesGroup::new();
    let mut any = false;
    for a in card.addresses() {
        any = true;
        let line = a.one_line();
        let sub = labels(&Field {
            value: String::new(),
            labels: a.labels.clone(),
            pref: a.pref,
        });
        places.add(&link_row(
            &line,
            &sub,
            "mark-location-symbolic",
            "Show on a map",
            Some(map_url(&line)),
            &line,
        ));
    }
    for u in card.urls() {
        any = true;
        places.add(&link_row(
            &u.value,
            &labels(&u),
            "web-browser-symbolic",
            "Open",
            Some(u.value.clone()),
            &u.value,
        ));
    }
    if any {
        out.push(places);
    }
    let more = adw::PreferencesGroup::new();
    let mut any = false;
    for (what, value) in [
        ("birthday", card.birthday()),
        ("anniversary", card.anniversary()),
    ] {
        if let Some(v) = value {
            any = true;
            let words = date_words(&v, today).unwrap_or(v.clone());
            more.add(&link_row(
                &words,
                what,
                "x-office-calendar-symbolic",
                "",
                None,
                &v,
            ));
        }
    }
    let nick = card.nicknames().join(", ");
    if !nick.is_empty() {
        any = true;
        more.add(&link_row(
            &nick,
            "nickname",
            "avatar-default-symbolic",
            "",
            None,
            &nick,
        ));
    }
    let cats = card.categories().join(", ");
    if !cats.is_empty() {
        any = true;
        more.add(&link_row(
            &cats,
            "groups",
            "system-users-symbolic",
            "",
            None,
            &cats,
        ));
    }
    for note in card.notes() {
        any = true;
        let row = adw::ActionRow::builder()
            .title(escape(&note))
            .subtitle("note")
            .title_selectable(true)
            .build();
        row.add_prefix(&gtk::Image::from_icon_name("document-edit-symbolic"));
        more.add(&row);
    }
    if any {
        out.push(more);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_in_words() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        assert_eq!(
            date_words("1990-01-31", today).as_deref(),
            Some("31 January 1990 · 36 this year")
        );
        assert_eq!(date_words("--0131", today).as_deref(), Some("31 January"));
        assert_eq!(date_words("--01-31", today).as_deref(), Some("31 January"));
        assert_eq!(date_words("soon", today), None);
    }
}

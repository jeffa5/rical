//! The editor: a person's card as fields to change, by keyboard. Tab moves
//! between fields, Ctrl+Enter saves, Escape cancels. Only what changes is
//! written; the card keeps its version and everything this doesn't show.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;
use rical_cards::{Address, CardId, Field, edit::Draft};

use crate::window::Window;

/// The labels offered for an email address or a number (as written in the
/// card: TYPE values).
const EMAIL_LABELS: &[(&str, &str)] = &[
    ("", "—"),
    ("home", "Home"),
    ("work", "Work"),
    ("other", "Other"),
];
const PHONE_LABELS: &[(&str, &str)] = &[
    ("", "—"),
    ("cell", "Mobile"),
    ("home", "Home"),
    ("work", "Work"),
    ("fax", "Fax"),
    ("other", "Other"),
];
const ADDRESS_LABELS: &[(&str, &str)] = &[
    ("", "—"),
    ("home", "Home"),
    ("work", "Work"),
    ("other", "Other"),
];

/// One email address, number or link being edited, and what it was.
struct FieldRow {
    row: adw::EntryRow,
    /// The label chooser and the labels it offers (as card values).
    label: Option<(gtk::DropDown, Vec<String>)>,
    pref: bool,
}

impl FieldRow {
    fn field(&self) -> Field {
        let labels = match &self.label {
            Some((d, values)) => values
                .get(d.selected() as usize)
                .filter(|v| !v.is_empty())
                .map(|v| v.split(", ").map(str::to_owned).collect())
                .unwrap_or_default(),
            None => Vec::new(),
        };
        Field {
            value: self.row.text().trim().to_owned(),
            labels,
            pref: self.pref,
        }
    }
}

/// A label chooser over `offered`, with what the field has selected (and
/// offered, if it is something else).
fn label_chooser(offered: &[(&str, &str)], current: &[String]) -> (gtk::DropDown, Vec<String>) {
    let mut values: Vec<String> = offered.iter().map(|(v, _)| (*v).to_owned()).collect();
    let mut shown: Vec<String> = offered.iter().map(|(_, s)| (*s).to_owned()).collect();
    let now = current.join(", ").to_lowercase();
    let now = if now == "mobile" {
        "cell".to_owned()
    } else {
        now
    };
    let at = match values.iter().position(|v| *v == now) {
        Some(i) => i,
        None => {
            values.push(current.join(", "));
            shown.push(current.join(", "));
            values.len() - 1
        }
    };
    let refs: Vec<&str> = shown.iter().map(String::as_str).collect();
    let d = gtk::DropDown::from_strings(&refs);
    d.set_selected(at as u32);
    d.set_valign(gtk::Align::Center);
    d.add_css_class("flat");
    d.set_tooltip_text(Some("Label"));
    (d, values)
}

/// A group of fields of one kind (emails, numbers, links) that can grow
/// and shrink.
struct FieldList {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<Rc<FieldRow>>>,
    title: &'static str,
    labels: Option<&'static [(&'static str, &'static str)]>,
    /// Where the add row sits, last.
    add: adw::ButtonRow,
}

impl FieldList {
    fn new(
        title: &'static str,
        add_label: &str,
        labels: Option<&'static [(&'static str, &'static str)]>,
        fields: &[Field],
    ) -> Rc<Self> {
        let group = adw::PreferencesGroup::builder().title(title).build();
        let add = adw::ButtonRow::builder()
            .title(add_label)
            .start_icon_name("list-add-symbolic")
            .build();
        group.add(&add);
        let list = Rc::new(Self {
            group,
            rows: Default::default(),
            title,
            labels,
            add,
        });
        for f in fields {
            list.push(f);
        }
        let weak = Rc::downgrade(&list);
        list.add.connect_activated(move |_| {
            if let Some(l) = weak.upgrade() {
                let r = l.push(&Field::default());
                r.row.grab_focus();
            }
        });
        list
    }

    fn push(self: &Rc<Self>, f: &Field) -> Rc<FieldRow> {
        let row = adw::EntryRow::builder()
            .title(self.title.trim_end_matches('s'))
            .text(&f.value)
            .build();
        let label = self.labels.map(|offered| label_chooser(offered, &f.labels));
        if let Some((d, _)) = &label {
            row.add_suffix(d);
        }
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .tooltip_text("Remove")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        row.add_suffix(&remove);
        let fr = Rc::new(FieldRow {
            row: row.clone(),
            label,
            pref: f.pref,
        });
        // added above the add row
        self.group.remove(&self.add);
        self.group.add(&row);
        self.group.add(&self.add);
        self.rows.borrow_mut().push(fr.clone());
        let weak = Rc::downgrade(self);
        let r = row.clone();
        remove.connect_clicked(move |_| {
            if let Some(l) = weak.upgrade() {
                l.group.remove(&r);
                l.rows.borrow_mut().retain(|x| x.row != r);
            }
        });
        fr
    }

    fn fields(&self) -> Vec<Field> {
        self.rows
            .borrow()
            .iter()
            .map(|r| r.field())
            .filter(|f| !f.value.is_empty())
            .collect()
    }
}

/// One postal address being edited, by part.
struct AddressRow {
    expander: adw::ExpanderRow,
    street: adw::EntryRow,
    locality: adw::EntryRow,
    region: adw::EntryRow,
    code: adw::EntryRow,
    country: adw::EntryRow,
    label: (gtk::DropDown, Vec<String>),
    was: Address,
}

impl AddressRow {
    fn address(&self) -> Address {
        let t = |r: &adw::EntryRow| r.text().trim().to_owned();
        let (d, values) = &self.label;
        Address {
            street: t(&self.street),
            locality: t(&self.locality),
            region: t(&self.region),
            code: t(&self.code),
            country: t(&self.country),
            labels: values
                .get(d.selected() as usize)
                .filter(|v| !v.is_empty())
                .map(|v| v.split(", ").map(str::to_owned).collect())
                .unwrap_or_default(),
            ..self.was.clone()
        }
    }
}

struct AddressList {
    group: adw::PreferencesGroup,
    rows: RefCell<Vec<Rc<AddressRow>>>,
    add: adw::ButtonRow,
}

impl AddressList {
    fn new(addresses: &[Address]) -> Rc<Self> {
        let group = adw::PreferencesGroup::builder().title("Addresses").build();
        let add = adw::ButtonRow::builder()
            .title("Add Address")
            .start_icon_name("list-add-symbolic")
            .build();
        group.add(&add);
        let list = Rc::new(Self {
            group,
            rows: Default::default(),
            add,
        });
        for a in addresses {
            list.push(a, false);
        }
        let weak = Rc::downgrade(&list);
        list.add.connect_activated(move |_| {
            if let Some(l) = weak.upgrade() {
                let r = l.push(&Address::default(), true);
                r.street.grab_focus();
            }
        });
        list
    }

    fn push(self: &Rc<Self>, a: &Address, expanded: bool) -> Rc<AddressRow> {
        let line = a.one_line();
        let expander = adw::ExpanderRow::builder()
            .title(if line.is_empty() {
                "New address".to_owned()
            } else {
                glib::markup_escape_text(&line).to_string()
            })
            .expanded(expanded)
            .build();
        let entry = |title: &str, text: &str| {
            let r = adw::EntryRow::builder().title(title).text(text).build();
            expander.add_row(&r);
            r
        };
        let street = entry("Street", &a.street);
        let locality = entry("Town or city", &a.locality);
        let region = entry("County, state or region", &a.region);
        let code = entry("Postcode", &a.code);
        let country = entry("Country", &a.country);
        let label = label_chooser(ADDRESS_LABELS, &a.labels);
        expander.add_suffix(&label.0);
        let remove = gtk::Button::builder()
            .icon_name("list-remove-symbolic")
            .tooltip_text("Remove")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        expander.add_suffix(&remove);
        self.group.remove(&self.add);
        self.group.add(&expander);
        self.group.add(&self.add);
        let row = Rc::new(AddressRow {
            expander: expander.clone(),
            street,
            locality,
            region,
            code,
            country,
            label,
            was: a.clone(),
        });
        self.rows.borrow_mut().push(row.clone());
        // the title follows what is typed
        let weak = Rc::downgrade(&row);
        for e in [
            &row.street,
            &row.locality,
            &row.region,
            &row.code,
            &row.country,
        ] {
            let weak = weak.clone();
            e.connect_changed(move |_| {
                if let Some(r) = weak.upgrade() {
                    let line = r.address().one_line();
                    r.expander
                        .set_title(&glib::markup_escape_text(if line.is_empty() {
                            "New address"
                        } else {
                            &line
                        }));
                }
            });
        }
        let weak = Rc::downgrade(self);
        let ex = expander.clone();
        remove.connect_clicked(move |_| {
            if let Some(l) = weak.upgrade() {
                l.group.remove(&ex);
                l.rows.borrow_mut().retain(|x| x.expander != ex);
            }
        });
        row
    }

    fn addresses(&self) -> Vec<Address> {
        self.rows
            .borrow()
            .iter()
            .map(|r| r.address())
            .filter(|a| !a.one_line().is_empty())
            .collect()
    }
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

/// A date as typed: "1990-01-31", "31/1/1990", "--01-31" or "31 Jan".
pub fn parse_date(text: &str) -> Result<String, String> {
    let t = text.trim();
    if t.is_empty() {
        return Ok(String::new());
    }
    if let Some(md) = t.strip_prefix("--") {
        let digits: String = md.chars().filter(char::is_ascii_digit).collect();
        if digits.len() == 4
            && chrono::NaiveDate::parse_from_str(&format!("2000{digits}"), "%Y%m%d").is_ok()
        {
            return Ok(format!("--{}-{}", &digits[..2], &digits[2..]));
        }
    }
    for f in [
        "%Y-%m-%d", "%Y%m%d", "%d/%m/%Y", "%d %B %Y", "%d %b %Y", "%B %d %Y", "%b %d %Y",
    ] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&t.replace(',', ""), f) {
            return Ok(d.format("%Y-%m-%d").to_string());
        }
    }
    // a day and month, no year
    for f in ["%d %B", "%d %b", "%B %d", "%b %d", "%d/%m"] {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(
            &format!("{} 2000", t.replace(',', "")),
            &format!("{f} %Y"),
        ) {
            return Ok(d.format("--%m-%d").to_string());
        }
    }
    Err(format!(
        "“{t}” isn't a date (try 1990-01-31, or 31 Jan with no year)"
    ))
}

/// Open the editor for `id`, or for someone new.
pub fn open(win: &Rc<Window>, id: Option<CardId>) {
    let (draft, book_names, book) = {
        let s = win.state.borrow();
        let draft = id
            .as_ref()
            .and_then(|id| s.books.get(id))
            .map(|c| Draft::of(&c))
            .unwrap_or_default();
        let names: Vec<String> = s.books.books.iter().map(|b| b.name.clone()).collect();
        (draft, names, s.default_book())
    };
    if id.is_none() && book_names.is_empty() {
        win.toast("There are no address books to add to");
        return;
    }
    let dialog = adw::Dialog::builder()
        .title(if id.is_some() {
            "Edit Contact"
        } else {
            "New Contact"
        })
        .content_width(560)
        .content_height(720)
        .build();
    let page = adw::PreferencesPage::new();

    let names = adw::PreferencesGroup::new();
    let entry = |title: &str, text: &str| {
        let r = adw::EntryRow::builder().title(title).text(text).build();
        names.add(&r);
        r
    };
    let given = entry("Given name", &draft.name.given);
    let family = entry("Family name", &draft.name.family);
    let full = entry("Full name, as shown", &draft.formatted_name);
    let nick = entry("Nickname", &draft.nicknames.join(", "));
    let org = entry("Organisation", &draft.org.join(", "));
    let title = entry("Job title", &draft.title);
    page.add(&names);
    // the full name follows the parts, until it is changed by hand
    let composed = |g: &str, f: &str| {
        [g.trim(), f.trim()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let last = Rc::new(RefCell::new(composed(
        &draft.name.given,
        &draft.name.family,
    )));
    for part in [&given, &family] {
        let (given, family, full, last) =
            (given.clone(), family.clone(), full.clone(), last.clone());
        part.connect_changed(move |_| {
            let now = composed(&given.text(), &family.text());
            let mut last = last.borrow_mut();
            if full.text().trim() == last.as_str() || full.text().trim().is_empty() {
                full.set_text(&now);
            }
            *last = now;
        });
    }

    let emails = FieldList::new("Emails", "Add Email", Some(EMAIL_LABELS), &draft.emails);
    let phones = FieldList::new("Phones", "Add Phone", Some(PHONE_LABELS), &draft.phones);
    let addresses = AddressList::new(&draft.addresses);
    let urls = FieldList::new("Websites", "Add Website", None, &draft.urls);
    for g in [&emails.group, &phones.group, &addresses.group, &urls.group] {
        page.add(g);
    }

    let more = adw::PreferencesGroup::builder().title("More").build();
    let birthday = adw::EntryRow::builder()
        .title("Birthday (1990-01-31, or 31 Jan)")
        .text(&draft.birthday)
        .build();
    let anniversary = adw::EntryRow::builder()
        .title("Anniversary")
        .text(&draft.anniversary)
        .build();
    let groups = adw::EntryRow::builder()
        .title("Groups (comma-separated)")
        .text(draft.categories.join(", "))
        .build();
    more.add(&birthday);
    more.add(&anniversary);
    more.add(&groups);
    let book_row = adw::ComboRow::builder()
        .title("Address book")
        .model(&gtk::StringList::new(
            &book_names.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .selected(book.unwrap_or(0) as u32)
        .build();
    if id.is_none() && book_names.len() > 1 {
        more.add(&book_row);
    }
    page.add(&more);

    let notes_group = adw::PreferencesGroup::builder().title("Notes").build();
    let notes = gtk::TextView::builder()
        .wrap_mode(gtk::WrapMode::WordChar)
        .top_margin(10)
        .bottom_margin(10)
        .left_margin(12)
        .right_margin(12)
        .accepts_tab(false)
        .build();
    notes.buffer().set_text(&draft.notes);
    notes.update_property(&[gtk::accessible::Property::Label("Notes")]);
    let frame = gtk::Frame::builder()
        .child(&notes)
        .css_classes(["view"])
        .height_request(100)
        .build();
    notes_group.add(&frame);
    page.add(&notes_group);

    let error = gtk::Label::builder()
        .css_classes(["error"])
        .wrap(true)
        .visible(false)
        .build();
    let error_group = adw::PreferencesGroup::new();
    error_group.add(&error);
    page.add(&error_group);

    let cancel = gtk::Button::with_label("Cancel");
    let save = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .tooltip_text("Save (Ctrl+Enter)")
        .build();
    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&save);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });

    let do_save: Rc<dyn Fn()> = Rc::new({
        let dialog = dialog.clone();
        let win = win.clone();
        let id = id.clone();
        let (given, family, full, nick, org, title) = (
            given.clone(),
            family.clone(),
            full.clone(),
            nick.clone(),
            org.clone(),
            title.clone(),
        );
        let (birthday, anniversary, groups, book_row) = (
            birthday.clone(),
            anniversary.clone(),
            groups.clone(),
            book_row.clone(),
        );
        let (emails, phones, addresses, urls) = (
            emails.clone(),
            phones.clone(),
            addresses.clone(),
            urls.clone(),
        );
        let (notes, error) = (notes.clone(), error.clone());
        let original = draft.clone();
        move || {
            let fail = |e: String| {
                error.set_label(&e);
                error.set_visible(true);
            };
            // a date left as it was stays as written (3.0's 19900131, say)
            let date = |row: &adw::EntryRow, was: &str| {
                if row.text() == was {
                    Ok(was.to_owned())
                } else {
                    parse_date(&row.text())
                }
            };
            let (bday, anniv) = match (
                date(&birthday, &original.birthday),
                date(&anniversary, &original.anniversary),
            ) {
                (Ok(b), Ok(a)) => (b, a),
                (Err(e), _) | (_, Err(e)) => return fail(e),
            };
            let buffer = notes.buffer();
            let mut name = original.name.clone();
            name.given = given.text().trim().to_owned();
            name.family = family.text().trim().to_owned();
            let d = Draft {
                formatted_name: full.text().trim().to_owned(),
                name,
                nicknames: split_list(&nick.text()),
                org: if org.text().trim() == original.org.join(", ") {
                    original.org.clone()
                } else {
                    split_list(&org.text())
                },
                title: title.text().trim().to_owned(),
                emails: emails.fields(),
                phones: phones.fields(),
                addresses: addresses.addresses(),
                urls: urls.fields(),
                birthday: bday,
                anniversary: anniv,
                notes: buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .trim()
                    .to_owned(),
                categories: split_list(&groups.text()),
                ..original.clone()
            };
            let empty = d.formatted_name.is_empty()
                && d.name.given.is_empty()
                && d.name.family.is_empty()
                && d.org.is_empty()
                && d.emails.is_empty()
                && d.phones.is_empty();
            if empty {
                return fail("Give them a name, an email address or a number".to_owned());
            }
            let result = match &id {
                Some(_) if d == original => Ok(()),
                Some(id) => win.update(id, &d),
                None => win.create(book_row.selected() as usize, &d),
            };
            match result {
                Ok(()) => {
                    dialog.close();
                }
                Err(e) => fail(e),
            }
        }
    });
    let s = do_save.clone();
    save.connect_clicked(move |_| s());
    for row in [
        &given,
        &family,
        &full,
        &nick,
        &org,
        &title,
        &birthday,
        &anniversary,
        &groups,
    ] {
        let s = do_save.clone();
        row.connect_entry_activated(move |_| s());
    }
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Managed);
    for trigger in ["<Control>Return", "<Control>s"] {
        let s = do_save.clone();
        shortcuts.add_shortcut(gtk::Shortcut::new(
            gtk::ShortcutTrigger::parse_string(trigger),
            Some(gtk::CallbackAction::new(move |_, _| {
                s();
                glib::Propagation::Stop
            })),
        ));
    }
    dialog.add_controller(shortcuts);
    let w = win.clone();
    dialog.connect_closed(move |_| w.focus_view());
    dialog.set_focus(Some(if id.is_some() { &full } else { &given }));
    dialog.present(Some(&win.window));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_as_typed() {
        assert_eq!(parse_date("1990-01-31").unwrap(), "1990-01-31");
        assert_eq!(parse_date("31/1/1990").unwrap(), "1990-01-31");
        assert_eq!(parse_date("31 Jan 1990").unwrap(), "1990-01-31");
        assert_eq!(parse_date("31 January").unwrap(), "--01-31");
        assert_eq!(parse_date("--01-31").unwrap(), "--01-31");
        assert_eq!(parse_date("").unwrap(), "");
        assert!(parse_date("soon").is_err());
    }
}

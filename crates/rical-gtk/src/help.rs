//! The keys and commands dialog, generated from the bindings in effect and
//! the command specs. Its search matches any part of a description, a
//! command or a key, with keys spelt out as words ("Left", "Page Down",
//! "Shift+Down", "left bracket").

use std::{collections::BTreeMap, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::{
    binds::{Action, Binds, Key},
    cmds,
    command::Registry,
    layout::ViewKind,
};

/// A binding in GTK accelerator syntax, if it is a single key.
fn accelerator(keys: &[Key]) -> Option<String> {
    let [key] = keys else { return None };
    let mut mods = gdk::ModifierType::empty();
    if key.ctrl {
        mods |= gdk::ModifierType::CONTROL_MASK;
    }
    if key.alt {
        mods |= gdk::ModifierType::ALT_MASK;
    }
    if key.shift {
        mods |= gdk::ModifierType::SHIFT_MASK;
    }
    let keyval = match key.name.as_str() {
        "enter" => gdk::Key::Return,
        "esc" => gdk::Key::Escape,
        "space" => gdk::Key::space,
        "tab" => gdk::Key::Tab,
        "backspace" => gdk::Key::BackSpace,
        "delete" => gdk::Key::Delete,
        "insert" => gdk::Key::Insert,
        "up" => gdk::Key::Up,
        "down" => gdk::Key::Down,
        "left" => gdk::Key::Left,
        "right" => gdk::Key::Right,
        "home" => gdk::Key::Home,
        "end" => gdk::Key::End,
        "pageup" => gdk::Key::Page_Up,
        "pagedown" => gdk::Key::Page_Down,
        name if name.starts_with('f') && name.len() > 1 => {
            gdk::Key::from_name(name.to_uppercase())?
        }
        name => {
            let mut c = name.chars().next()?;
            // show "T" as Shift+T, as GTK draws plain "t" as "T" too
            if c.is_ascii_uppercase() {
                mods |= gdk::ModifierType::SHIFT_MASK;
                c = c.to_ascii_lowercase();
            }
            // the plain FFI call: no GTK initialisation needed for this
            unsafe { glib::translate::from_glib(gdk::ffi::gdk_unicode_to_keyval(c as u32)) }
        }
    };
    let mut accel = String::new();
    for (m, name) in [
        (gdk::ModifierType::CONTROL_MASK, "<Control>"),
        (gdk::ModifierType::ALT_MASK, "<Alt>"),
        (gdk::ModifierType::SHIFT_MASK, "<Shift>"),
    ] {
        if mods.contains(m) {
            accel.push_str(name);
        }
    }
    accel.push_str(&keyval.name()?);
    Some(accel)
}

/// A key in words, e.g. "Ctrl+Z", "Shift+Down", "Page Up", "[".
pub fn key_words(key: &Key) -> String {
    let mut s = String::new();
    if key.ctrl {
        s += "Ctrl+";
    }
    if key.alt {
        s += "Alt+";
    }
    let mut name = match key.name.as_str() {
        "pageup" => "Page Up".to_owned(),
        "pagedown" => "Page Down".to_owned(),
        "esc" => "Escape".to_owned(),
        n if n.chars().count() > 1 => {
            let mut c = n.chars();
            let first = c.next().unwrap().to_uppercase();
            format!("{first}{}", c.as_str())
        }
        n => n.to_owned(),
    };
    if key.shift || (name.chars().count() == 1 && name.chars().all(|c| c.is_ascii_uppercase())) {
        s += "Shift+";
    }
    if name.chars().count() == 1 {
        name = name.to_uppercase();
    }
    s + &name
}

/// Extra words to search a key by: "left arrow", "left bracket"...
fn key_synonyms(key: &Key) -> &'static str {
    match key.name.as_str() {
        "up" => "up arrow",
        "down" => "down arrow",
        "left" => "left arrow",
        "right" => "right arrow",
        "enter" => "return",
        "esc" => "esc",
        "[" => "left bracket",
        "]" => "right bracket",
        "{" => "left brace",
        "}" => "right brace",
        "?" => "question mark",
        "/" => "slash",
        ":" => "colon",
        _ => "",
    }
}

fn sequence_words(keys: &[Key]) -> String {
    keys.iter()
        .map(key_words)
        .collect::<Vec<_>>()
        .join(" then ")
}

fn spec_of(action: &Action) -> Option<&'static crate::command::Spec> {
    action
        .run
        .first()
        .or(action.prefill.as_ref())
        .and_then(|l| l.split_whitespace().next())
        .and_then(|name| {
            Registry { specs: cmds::SPECS }
                .resolve(name, cmds::ALL)
                .ok()
        })
}

fn command_text(action: &Action) -> String {
    action
        .run
        .iter()
        .map(|l| format!(":{l}"))
        .chain(action.prefill.iter().map(|p| format!(":{p}…")))
        .collect::<Vec<_>>()
        .join(" then ")
}

/// One row: what it does, the command, its keys and what to search.
struct Entry {
    title: String,
    subtitle: String,
    keys: Vec<Vec<Key>>,
    haystack: String,
}

fn entry(title: String, subtitle: String, keys: Vec<Vec<Key>>, extra: &str) -> Entry {
    let mut haystack = format!("{title} {subtitle} {extra}");
    for k in &keys {
        haystack += &format!(
            " {} {} {}",
            sequence_words(k),
            k.iter().map(key_synonyms).collect::<Vec<_>>().join(" "),
            crate::binds::keys_to_string(k)
        );
    }
    Entry {
        title,
        subtitle,
        keys,
        haystack: haystack.to_lowercase(),
    }
}

/// An action and every key sequence bound to it.
type Bound = (Action, Vec<Vec<Key>>);

/// The bindings of one section, grouped by what they do, in command order.
fn binding_entries(binds: &Binds, section: &str) -> Vec<Entry> {
    let Some(map) = binds.sections.get(section) else {
        return Vec::new();
    };
    let mut groups: BTreeMap<(usize, String), Bound> = BTreeMap::new();
    for (keys, action) in map {
        let Some(action) = action else { continue };
        let order = spec_of(action)
            .and_then(|s| cmds::SPECS.iter().position(|x| x.name == s.name))
            .unwrap_or(usize::MAX);
        groups
            .entry((order, command_text(action)))
            .or_insert_with(|| (action.clone(), Vec::new()))
            .1
            .push(keys.clone());
    }
    groups
        .into_values()
        .map(|(action, keys)| {
            let title = match spec_of(&action) {
                Some(s) if action.run.is_empty() => format!("{} (type the rest)", s.help),
                Some(s) => s.help.to_owned(),
                None if action.run.is_empty() => "Open the command line".to_owned(),
                None => command_text(&action),
            };
            entry(title, command_text(&action), keys, "")
        })
        .collect()
}

/// Every command, with the keys bound to it anywhere.
fn command_entries(binds: &Binds) -> Vec<Entry> {
    cmds::SPECS
        .iter()
        .map(|spec| {
            let mut keys = Vec::new();
            for map in binds.sections.values() {
                for (k, a) in map {
                    if a.as_ref()
                        .and_then(spec_of)
                        .is_some_and(|s| s.name == spec.name)
                        && !keys.contains(k)
                    {
                        keys.push(k.clone());
                    }
                }
            }
            let mut sub = format!(":{}", spec.usage());
            if !spec.aliases.is_empty() {
                sub += &format!("   (also {})", spec.aliases.join(", "));
            }
            entry(spec.help.to_owned(), sub, keys, spec.name)
        })
        .collect()
}

fn keys_widget(keys: &[Vec<Key>]) -> gtk::Box {
    let b = gtk::Box::builder()
        .spacing(6)
        .valign(gtk::Align::Center)
        .build();
    for (i, k) in keys.iter().take(4).enumerate() {
        if i > 0 {
            b.append(
                &gtk::Label::builder()
                    .label("or")
                    .css_classes(["dim-label", "caption"])
                    .build(),
            );
        }
        match accelerator(k) {
            Some(accel) => b.append(&adw::ShortcutLabel::new(&accel)),
            None => b.append(
                &gtk::Label::builder()
                    .label(sequence_words(k))
                    .css_classes(["dim-label"])
                    .build(),
            ),
        }
    }
    b
}

/// A group of rows under a heading, and what each row matches.
type Group = (gtk::Label, gtk::ListBox, Vec<(adw::ActionRow, String)>);

pub fn dialog(binds: &Binds, view: ViewKind) -> adw::Dialog {
    let here = cmds::section(view);
    let mut sections = vec![("global", "Everywhere")];
    // the current view's section first, then the others
    let views = [
        ("grid", "Day and week views"),
        ("month", "Month view"),
        ("agenda", "Agenda view"),
        ("tasks", "Tasks view"),
    ];
    sections.extend(views.iter().filter(|(s, _)| *s == here));
    sections.extend(views.iter().filter(|(s, _)| *s != here));

    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search what keys do, commands or keys (left, ctrl z, undo)")
        .hexpand(true)
        .build();
    let page = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .margin_top(6)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let mut groups: Vec<Group> = Vec::new();
    let all = sections
        .iter()
        .map(|(s, t)| (t.to_string(), binding_entries(binds, s)))
        .chain(std::iter::once((
            "Commands (type : to run)".to_owned(),
            command_entries(binds),
        )));
    for (title, entries) in all {
        if entries.is_empty() {
            continue;
        }
        let heading = gtk::Label::builder()
            .label(&title)
            .css_classes(["heading"])
            .xalign(0.0)
            .margin_top(12)
            .build();
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["boxed-list"])
            .build();
        let mut rows = Vec::new();
        for e in entries {
            let row = adw::ActionRow::builder()
                .title(&e.title)
                .subtitle(&e.subtitle)
                .use_markup(false)
                .build();
            row.add_suffix(&keys_widget(&e.keys));
            list.append(&row);
            rows.push((row, e.haystack));
        }
        page.append(&heading);
        page.append(&list);
        groups.push((heading, list, rows));
    }
    let empty = adw::StatusPage::builder()
        .icon_name("system-search-symbolic")
        .title("Nothing matches")
        .visible(false)
        .build();
    page.append(&empty);

    let groups = Rc::new(groups);
    search.connect_search_changed(move |s| {
        let words = s
            .text()
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let mut any = false;
        for (heading, list, rows) in groups.iter() {
            let mut shown = false;
            for (row, haystack) in rows {
                // every word must appear somewhere in the row
                let m = words.iter().all(|w| haystack.contains(w.as_str()));
                row.set_visible(m);
                shown |= m;
            }
            heading.set_visible(shown);
            list.set_visible(shown);
            any |= shown;
        }
        empty.set_visible(!any);
    });

    let scroller = gtk::ScrolledWindow::builder()
        .child(&adw::Clamp::builder().maximum_size(720).child(&page).build())
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();
    let header = adw::HeaderBar::builder().title_widget(&search).build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&scroller));
    let dialog = adw::Dialog::builder()
        .title("Keys and commands")
        .content_width(760)
        .content_height(640)
        .child(&view)
        .build();
    dialog.set_focus(Some(&search));
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::binds::parse_keys;

    #[test]
    fn accelerators() {
        let a = |s| accelerator(&parse_keys(s).unwrap());
        assert_eq!(a("t").as_deref(), Some("t"));
        assert_eq!(a("T").as_deref(), Some("<Shift>t"));
        assert_eq!(a("<C-z>").as_deref(), Some("<Control>z"));
        assert_eq!(a("<S-Down>").as_deref(), Some("<Shift>Down"));
        assert_eq!(a("]").as_deref(), Some("bracketright"));
        assert_eq!(a("<F9>").as_deref(), Some("F9"));
        assert_eq!(a("gg"), None);
    }

    #[test]
    fn words() {
        let w = |s| sequence_words(&parse_keys(s).unwrap());
        assert_eq!(w("t"), "T");
        assert_eq!(w("T"), "Shift+T");
        assert_eq!(w("<S-Down>"), "Shift+Down");
        assert_eq!(w("<C-z>"), "Ctrl+Z");
        assert_eq!(w("<PageDown>"), "Page Down");
        assert_eq!(w("<Left>"), "Left");
        assert_eq!(w("gg"), "G then G");
        assert_eq!(w("["), "[");
    }

    #[test]
    fn search_finds_by_any_part() {
        let binds = Binds::parse(crate::binds::DEFAULTS).unwrap();
        let entries = binding_entries(&binds, "global")
            .into_iter()
            .chain(command_entries(&binds))
            .collect::<Vec<_>>();
        let find = |q: &str| {
            entries
                .iter()
                .filter(|e| q.split_whitespace().all(|w| e.haystack.contains(w)))
                .map(|e| e.subtitle.clone())
                .collect::<Vec<_>>()
        };
        // by key words
        assert!(find("left").iter().any(|s| s == ":focus -1"));
        assert!(find("ctrl z").iter().any(|s| s == ":undo"));
        assert!(find("page down").iter().any(|s| s == ":next"));
        assert!(find("right bracket").iter().any(|s| s == ":resize +15m"));
        // by description and command
        assert!(find("undo").iter().any(|s| s == ":undo"));
        assert!(find("schedule").iter().any(|s| s.starts_with(":schedule")));
        assert!(find("zzzz").is_empty());
    }
}

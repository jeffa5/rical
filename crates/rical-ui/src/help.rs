//! The keys and commands dialog, generated from the bindings in effect and
//! the command specs. Its search matches any part of a description, a
//! command or a key, with keys spelt out as words ("Left", "Page Down",
//! "Shift+Down", "left bracket").

use std::{collections::BTreeMap, rc::Rc};

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::{
    binds::{Action, Binds, Key},
    command::{Context, Registry, Spec},
};

/// What the help is about: an app's commands, the context that has them
/// all, and its views' binding sections with what to call each.
#[derive(Clone, Copy)]
pub struct Commands {
    pub specs: &'static [Spec],
    pub all: Context,
    pub views: &'static [(&'static str, &'static str)],
}

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

fn spec_of(cmds: &Commands, action: &Action) -> Option<&'static Spec> {
    action
        .run
        .first()
        .or(action.prefill.as_ref())
        .and_then(|l| l.split_whitespace().next())
        .and_then(|name| Registry { specs: cmds.specs }.resolve(name, cmds.all).ok())
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

/// The view sections and what to call them.
/// The command an action runs with nothing else given, if it is just
/// that: ":today", or ":goto " left for you to finish.
fn bare_command(cmds: &Commands, action: &Action) -> Option<&'static Spec> {
    let line = match (action.run.as_slice(), &action.prefill) {
        ([line], None) | ([], Some(line)) => line,
        _ => return None,
    };
    (line.split_whitespace().count() == 1)
        .then(|| spec_of(cmds, action))
        .flatten()
}

/// What an action's row says: its title and its command.
fn describe(cmds: &Commands, action: &Action) -> (String, String, &'static str) {
    if let Some(s) = bare_command(cmds, action) {
        return (s.help.to_owned(), usage(s), s.name);
    }
    let title = match spec_of(cmds, action) {
        // "Switch view (2days)" rather than the whole help each time
        Some(s) if action.run.len() == 1 && action.prefill.is_none() => {
            let short = s
                .help
                .split([':', ';'])
                .next()
                .unwrap_or(s.help)
                .split(", e.g.")
                .next()
                .unwrap_or(s.help);
            match action.run[0].split_once(char::is_whitespace) {
                Some((_, args)) => format!("{short} ({})", args.trim()),
                None => short.to_owned(),
            }
        }
        Some(s) => s.help.to_owned(),
        None if action.run.is_empty() => "Open the command line".to_owned(),
        None => command_text(action),
    };
    (title, command_text(action), "")
}

fn usage(spec: &Spec) -> String {
    let mut s = format!(":{}", spec.usage());
    if !spec.aliases.is_empty() {
        s += &format!("   (also {})", spec.aliases.join(", "));
    }
    s
}

/// Bindings grouped by what they do, in command order, with the views
/// each is bound in.
type Grouped = BTreeMap<(usize, String), (Action, Vec<Vec<Key>>, Vec<&'static str>)>;

fn group(
    cmds: &Commands,
    grouped: &mut Grouped,
    keys: &[Key],
    action: &Action,
    view: Option<&'static str>,
) {
    let order = spec_of(cmds, action)
        .and_then(|s| cmds.specs.iter().position(|x| x.name == s.name))
        .unwrap_or(usize::MAX);
    let (_, all, views) = grouped
        .entry((order, command_text(action)))
        .or_insert_with(|| (action.clone(), Vec::new(), Vec::new()));
    if !all.iter().any(|k| k == keys) {
        all.push(keys.to_vec());
    }
    if let Some(v) = view
        && !views.contains(&v)
    {
        views.push(v);
    }
}

/// The help's groups: the keys that work in `here`, keys only bound in
/// other views, then the commands no key runs as they are.
fn entries(cmds: &Commands, binds: &Binds, here: &str) -> Vec<(String, Vec<Entry>)> {
    let empty = BTreeMap::new();
    let section = |s: &str| binds.sections.get(s).unwrap_or(&empty);
    // the keys in effect here: this view's, then the global ones it
    // doesn't take over
    let mut active: BTreeMap<Vec<Key>, Option<Action>> = section(here).clone();
    for (k, a) in section("global") {
        active.entry(k.clone()).or_insert_with(|| a.clone());
    }
    let mut local = Grouped::new();
    for (k, a) in &active {
        if let Some(a) = a {
            group(cmds, &mut local, k, a, None);
        }
    }
    let mut elsewhere = Grouped::new();
    for (s, label) in cmds.views.iter().filter(|(s, _)| *s != here) {
        for (k, a) in section(s) {
            let Some(a) = a else { continue };
            if active.get(k).is_some_and(|b| b.as_ref() == Some(a)) {
                continue;
            }
            group(cmds, &mut elsewhere, k, a, Some(label));
        }
    }
    let mut covered = Vec::new();
    let mut rows = |grouped: Grouped| {
        grouped
            .into_values()
            .map(|(action, keys, views)| {
                let (title, mut sub, name) = describe(cmds, &action);
                if bare_command(cmds, &action).is_some() {
                    covered.push(name);
                }
                if !views.is_empty() {
                    sub = format!("{sub}  ·  {} view", views.join(", "));
                }
                entry(title, sub, keys, name)
            })
            .collect::<Vec<_>>()
    };
    let local = rows(local);
    let elsewhere = rows(elsewhere);
    let commands = cmds
        .specs
        .iter()
        .filter(|s| !covered.contains(&s.name))
        .map(|s| entry(s.help.to_owned(), usage(s), Vec::new(), s.name))
        .collect();
    let here_label = cmds
        .views
        .iter()
        .find(|(s, _)| *s == here)
        .map_or("this", |(_, l)| l);
    vec![
        (
            format!("Keys in the {} view", here_label.to_lowercase()),
            local,
        ),
        ("Keys in other views".to_owned(), elsewhere),
        (
            "More commands (type : then the command)".to_owned(),
            commands,
        ),
    ]
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

/// The items whose text matches `query` fzf-style (each word's letters in
/// order, smart case), best first; all of them, in order, for no query.
fn rank<'a, T>(items: &'a [(T, String)], query: &str) -> Vec<&'a T> {
    use nucleo_matcher::{
        Config, Matcher, Utf32Str,
        pattern::{CaseMatching, Normalization, Pattern},
    };
    if query.trim().is_empty() {
        return items.iter().map(|(t, _)| t).collect();
    }
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut matcher = Matcher::new(Config::DEFAULT);
    let mut buf = Vec::new();
    let mut scored = items
        .iter()
        .filter_map(|(t, text)| {
            Some((
                pattern.score(Utf32Str::new(text, &mut buf), &mut matcher)?,
                t,
            ))
        })
        .collect::<Vec<_>>();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, t)| t).collect()
}

/// A group of rows under a heading, and what each row matches.
type Group = (gtk::Label, gtk::ListBox, Vec<(adw::ActionRow, String)>);

/// The dialog for `binds`, with the keys of the `here` view first.
pub fn dialog(cmds: &Commands, binds: &Binds, here: &str) -> adw::Dialog {
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
    for (title, entries) in entries(cmds, binds, here) {
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
            // rows take markup: escape ":goto <date>"
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&e.title))
                .subtitle(glib::markup_escape_text(&e.subtitle))
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
        let query = s.text();
        let mut any = false;
        for (heading, list, rows) in groups.iter() {
            // fzf-style, best match first; all of them, in order, when empty
            let ranked = rank(rows, &query);
            list.remove_all();
            for row in &ranked {
                list.append(*row);
            }
            let shown = !ranked.is_empty();
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
    // the search takes Escape first: clear it, or close if already empty
    let d = dialog.downgrade();
    search.connect_stop_search(move |s| {
        if !s.text().is_empty() {
            s.set_text("");
        } else if let Some(d) = d.upgrade() {
            d.close();
        }
    });
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
        use crate::command::{Arg, Kind};
        const ALL: Context = Context(0b11);
        const SPECS: &[Spec] = &[
            Spec {
                name: "today",
                aliases: &[],
                help: "Go to today",
                contexts: ALL,
                args: &[],
                flags: &[],
            },
            Spec {
                name: "focus",
                aliases: &[],
                help: "Move the focused day, e.g. +1 or -7",
                contexts: ALL,
                args: &[Arg {
                    name: "days",
                    kind: Kind::Text("days"),
                    optional: false,
                    rest: false,
                }],
                flags: &[],
            },
            Spec {
                name: "undo",
                aliases: &[],
                help: "Undo the last change",
                contexts: ALL,
                args: &[],
                flags: &[],
            },
            Spec {
                name: "goto",
                aliases: &[],
                help: "Go to a date",
                contexts: ALL,
                args: &[Arg {
                    name: "date",
                    kind: Kind::Text("date"),
                    optional: false,
                    rest: false,
                }],
                flags: &[],
            },
            Spec {
                name: "zones",
                aliases: &[],
                help: "Extra time zones",
                contexts: ALL,
                args: &[],
                flags: &[],
            },
        ];
        let cmds = Commands {
            specs: SPECS,
            all: ALL,
            views: &[("grid", "Day and week"), ("month", "Month")],
        };
        let binds = Binds::parse(
            "[global]\nt = :today<Enter>\n<Left> = :focus -1<Enter>\n<C-z> = :undo<Enter>\n\
             g = :goto<space>\n\n[month]\n<Down> = :focus +7<Enter>\n",
        )
        .unwrap();
        let entries = entries(&cmds, &binds, "grid")
            .into_iter()
            .flat_map(|(_, e)| e)
            .collect::<Vec<_>>();
        let rows = entries
            .iter()
            .map(|e| (e.subtitle.clone(), e.haystack.clone()))
            .collect::<Vec<_>>();
        let find = |q: &str| rank(&rows, q).into_iter().cloned().collect::<Vec<_>>();
        // fuzzy, fzf style: letters in order, best first
        assert!(find("lft").iter().any(|s| s == ":focus -1"));
        assert!(
            find("zones")[0].starts_with(":zones"),
            "{:?}",
            find("zones")
        );
        // by key words
        assert!(
            find("left").iter().any(|s| s == ":focus -1"),
            "{:?}",
            find("left")
        );
        assert!(find("ctrl z").iter().any(|s| s.starts_with(":undo")));
        // by description and command
        assert!(find("undo").iter().any(|s| s.starts_with(":undo")));
        assert!(find("qxjqxj").is_empty());
        // a command a key runs as it is isn't listed again
        assert_eq!(
            find("today")
                .iter()
                .filter(|s| s.starts_with(":today"))
                .count(),
            1
        );
        assert_eq!(
            find("goto")
                .iter()
                .filter(|s| s.starts_with(":goto <date>"))
                .count(),
            1
        );
        // keys bound only in other views say where
        assert!(find("month view").iter().any(|s| s.contains("Month view")));
    }
}

//! The key bindings dialog, generated from the bindings in effect and the
//! command descriptions, so it is always accurate.

use std::collections::BTreeMap;

use gtk::{gdk, glib};

use crate::{
    binds::{Action, Binds, Key, keys_to_string},
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

fn title(action: &Action) -> String {
    let lines = action
        .run
        .iter()
        .map(|l| format!(":{l}"))
        .chain(action.prefill.iter().map(|p| format!(":{p}…")))
        .collect::<Vec<_>>()
        .join(" ");
    let first = action
        .run
        .first()
        .or(action.prefill.as_ref())
        .and_then(|l| l.split_whitespace().next())
        .and_then(|name| {
            Registry { specs: cmds::SPECS }
                .resolve(name, cmds::ALL)
                .ok()
        });
    match first {
        Some(spec) if action.prefill.is_some() && action.run.is_empty() => {
            format!("{} (type the rest)   {lines}", spec.help)
        }
        Some(spec) => format!("{}   {lines}", spec.help),
        None if lines == ":…" => "Open the command line".to_owned(),
        None => lines,
    }
}

/// An action with its single-key accelerators and longer key sequences.
type Group = (Action, Vec<String>, Vec<String>);

pub fn dialog(binds: &Binds, view: ViewKind) -> adw::ShortcutsDialog {
    let dialog = adw::ShortcutsDialog::new();

    let here = cmds::section(view);
    let sections = [
        ("global", "Everywhere"),
        (
            here,
            match here {
                "grid" => "Day and week view",
                "month" => "Month view",
                "agenda" => "Agenda view",
                _ => "Tasks view",
            },
        ),
    ];
    for (name, heading) in sections {
        let Some(binds) = binds.sections.get(name) else {
            continue;
        };
        // group keys by what they do; actions differing only in a trailing
        // number (":calendar toggle 1".."9") become one item
        let mut by_action: BTreeMap<(usize, String), Group> = BTreeMap::new();
        for (keys, action) in binds {
            let Some(action) = action else { continue };
            let mut action = action.clone();
            for line in &mut action.run {
                if let Some((head, n)) = line.rsplit_once(' ')
                    && n.parse::<u32>().is_ok()
                {
                    *line = format!("{head} N");
                }
            }
            let order = action
                .run
                .first()
                .or(action.prefill.as_ref())
                .and_then(|l| l.split_whitespace().next())
                .and_then(|name| {
                    cmds::SPECS
                        .iter()
                        .position(|s| s.names().any(|n| n == name))
                })
                .unwrap_or(usize::MAX);
            let entry = by_action
                .entry((order, format!("{:?}{:?}", action.run, action.prefill)))
                .or_insert_with(|| (action, Vec::new(), Vec::new()));
            match accelerator(keys) {
                Some(accel) => entry.1.push(accel),
                None => entry.2.push(keys_to_string(keys)),
            }
        }
        let section = adw::ShortcutsSection::new(Some(heading));
        for (_, (action, accels, sequences)) in by_action {
            let mut t = title(&action);
            if !sequences.is_empty() {
                t = format!("{t}   [{}]", sequences.join(", "));
            }
            section.add(adw::ShortcutsItem::new(&t, &accels.join(" ")));
        }
        dialog.add(section);
    }

    let commands = adw::ShortcutsSection::new(Some("Commands (type : to run)"));
    for spec in cmds::SPECS {
        let mut usage = spec.usage();
        if !spec.aliases.is_empty() {
            usage = format!("{usage}   (also {})", spec.aliases.join(", "));
        }
        commands.add(adw::ShortcutsItem::new(
            &format!("{}\n:{usage}", spec.help),
            "",
        ));
    }
    dialog.add(commands);
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
}

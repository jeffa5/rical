//! Keys: a GDK key press as a binding key, and which keys move around a
//! view (so they're left to whatever has focus outside it).

use gtk::gdk;

use crate::binds::Key;

/// Keys that move around or act in a view; outside the view (a sidebar,
/// a dialog) they do what they do there.
pub fn is_navigation(key: &Key) -> bool {
    !key.ctrl
        && !key.alt
        && matches!(
            key.name.as_str(),
            "up" | "down"
                | "left"
                | "right"
                | "enter"
                | "space"
                | "esc"
                | "pageup"
                | "pagedown"
                | "tab"
                | "home"
                | "end"
        )
}

/// The binding key for a GDK key press; None for bare modifiers.
pub fn key_of(keyval: gdk::Key, mods: gdk::ModifierType) -> Option<Key> {
    use gdk::Key as K;
    let named = match keyval {
        K::Return | K::KP_Enter | K::ISO_Enter => "enter",
        K::Escape => "esc",
        K::space | K::KP_Space => "space",
        K::Tab | K::ISO_Left_Tab | K::KP_Tab => "tab",
        K::BackSpace => "backspace",
        K::Delete | K::KP_Delete => "delete",
        K::Insert | K::KP_Insert => "insert",
        K::Up | K::KP_Up => "up",
        K::Down | K::KP_Down => "down",
        K::Left | K::KP_Left => "left",
        K::Right | K::KP_Right => "right",
        K::Home | K::KP_Home => "home",
        K::End | K::KP_End => "end",
        K::Page_Up | K::KP_Page_Up => "pageup",
        K::Page_Down | K::KP_Page_Down => "pagedown",
        K::F1 => "f1",
        K::F2 => "f2",
        K::F3 => "f3",
        K::F4 => "f4",
        K::F5 => "f5",
        K::F6 => "f6",
        K::F7 => "f7",
        K::F8 => "f8",
        K::F9 => "f9",
        K::F10 => "f10",
        K::F11 => "f11",
        K::F12 => "f12",
        _ => "",
    };
    let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
    let alt = mods.contains(gdk::ModifierType::ALT_MASK);
    if !named.is_empty() {
        return Some(Key {
            name: named.to_owned(),
            ctrl,
            alt,
            shift: mods.contains(gdk::ModifierType::SHIFT_MASK),
        });
    }
    let c = keyval.to_unicode().filter(|c| !c.is_control())?;
    Some(Key {
        name: c.to_string(),
        ctrl,
        alt,
        shift: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdk_keys() {
        use gdk::ModifierType as M;
        let k = |kv, m| key_of(kv, m).map(|k| k.to_string());
        assert_eq!(k(gdk::Key::t, M::empty()).as_deref(), Some("t"));
        assert_eq!(k(gdk::Key::T, M::SHIFT_MASK).as_deref(), Some("T"));
        assert_eq!(
            k(gdk::Key::Z, M::SHIFT_MASK | M::CONTROL_MASK).as_deref(),
            Some("<C-Z>")
        );
        assert_eq!(
            k(gdk::Key::Down, M::SHIFT_MASK).as_deref(),
            Some("<S-Down>")
        );
        assert_eq!(k(gdk::Key::bracketright, M::empty()).as_deref(), Some("]"));
        assert_eq!(k(gdk::Key::_1, M::ALT_MASK).as_deref(), Some("<A-1>"));
        assert_eq!(k(gdk::Key::Shift_L, M::empty()), None);
    }
}

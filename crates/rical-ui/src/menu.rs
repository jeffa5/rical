//! Context menus: a right click (or a long press) shows what can be done to
//! what it is on, each item a command run by the window's `win.run`.

use adw::prelude::*;
use gtk::{gdk, gio, glib};

/// An item running `command` (as typed after `:`).
pub fn item(menu: &gio::Menu, label: &str, command: &str) {
    let i = gio::MenuItem::new(Some(label), None);
    i.set_action_and_target_value(Some("win.run"), Some(&command.to_variant()));
    menu.append_item(&i);
}

/// A section of items, each (label, command).
pub fn section(menu: &gio::Menu, items: &[(&str, &str)]) {
    let s = gio::Menu::new();
    for (label, command) in items {
        item(&s, label, command);
    }
    menu.append_section(None, &s);
}

/// Show `menu` at (`x`, `y`) in `widget`.
pub fn popup(widget: &impl IsA<gtk::Widget>, menu: &gio::Menu, x: f64, y: f64) {
    // its parent first: the items find their action (`win.run`) through
    // it, and items added before that make the popover come out short
    let pop = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
    pop.set_parent(widget);
    pop.set_menu_model(Some(menu));
    pop.set_has_arrow(false);
    pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    pop.connect_closed(|p| {
        // after its item has run
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    pop.popup();
}

/// On a right click or a long press on `widget`, `menu` is asked for the
/// menu for that place (selecting what is there first, say), and shows it
/// if there is one.
pub fn on_right_click(
    widget: &impl IsA<gtk::Widget>,
    menu: impl Fn(f64, f64) -> Option<gio::Menu> + 'static,
) {
    let menu = std::rc::Rc::new(menu);
    let click = gtk::GestureClick::builder()
        .button(gdk::BUTTON_SECONDARY)
        .build();
    let m = menu.clone();
    click.connect_pressed(move |g, _, x, y| {
        let Some(w) = g.widget() else { return };
        if let Some(model) = m(x, y) {
            popup(&w, &model, x, y);
        }
    });
    widget.as_ref().add_controller(click);
    let press = gtk::GestureLongPress::builder().touch_only(true).build();
    press.connect_pressed(move |g, x, y| {
        let Some(w) = g.widget() else { return };
        if let Some(model) = menu(x, y) {
            popup(&w, &model, x, y);
        }
    });
    widget.as_ref().add_controller(press);
}

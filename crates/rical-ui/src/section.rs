//! Collapsible sidebar sections, remembered between runs.

use std::{collections::BTreeSet, path::PathBuf, rc::Rc};

use adw::prelude::*;

pub struct Section {
    pub name: &'static str,
    pub widget: gtk::Box,
    revealer: gtk::Revealer,
    arrow: gtk::Image,
}

impl Section {
    /// A section called `title` holding `child`; `on_toggle` hears about
    /// it opening or closing.
    pub fn new(
        name: &'static str,
        title: &str,
        child: &impl IsA<gtk::Widget>,
        expanded: bool,
        on_toggle: Rc<dyn Fn()>,
    ) -> Rc<Self> {
        let arrow = gtk::Image::from_icon_name("pan-down-symbolic");
        let header_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        header_box.append(
            &gtk::Label::builder()
                .label(title)
                .css_classes(["heading"])
                .xalign(0.0)
                .hexpand(true)
                .build(),
        );
        header_box.append(&arrow);
        let header = gtk::Button::builder()
            .child(&header_box)
            .css_classes(["flat"])
            .margin_start(6)
            .margin_end(6)
            .tooltip_text(format!(
                "Show or hide {} (:section {name})",
                title.to_lowercase()
            ))
            .build();
        let revealer = gtk::Revealer::builder()
            .child(child)
            .transition_type(gtk::RevealerTransitionType::SlideDown)
            .reveal_child(expanded)
            .build();
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&header);
        widget.append(&revealer);
        let this = Rc::new(Self {
            name,
            widget,
            revealer,
            arrow,
        });
        this.show_arrow();
        let t = Rc::downgrade(&this);
        header.connect_clicked(move |_| {
            if let Some(t) = t.upgrade() {
                t.set_expanded(!t.is_expanded());
                on_toggle();
            }
        });
        this
    }

    pub fn is_expanded(&self) -> bool {
        self.revealer.reveals_child()
    }

    pub fn set_expanded(&self, expanded: bool) {
        self.revealer.set_reveal_child(expanded);
        self.show_arrow();
    }

    fn show_arrow(&self) {
        self.arrow.set_icon_name(Some(if self.is_expanded() {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        }));
    }
}

fn state_file() -> Option<PathBuf> {
    crate::app::state_dir().map(|d| d.join("collapsed"))
}

/// Names of the sections collapsed last time.
pub fn load_collapsed() -> BTreeSet<String> {
    state_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.lines().map(str::to_owned).collect())
        .unwrap_or_default()
}

pub fn save_collapsed(sections: &[Rc<Section>]) {
    let Some(path) = state_file() else { return };
    let text = sections
        .iter()
        .filter(|s| !s.is_expanded())
        .map(|s| format!("{}\n", s.name))
        .collect::<String>();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, text) {
        log::warn!("could not save {path:?}: {e}");
    }
}

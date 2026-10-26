//! Preferences: an app's settings, edited in place in its config file. A
//! file the user can't write (one Nix made, say) is left alone and changes
//! go to `local.conf` beside it, which is read after it.
//!
//! An app opens a [`Prefs`], adds groups of rows made with its builders,
//! and presents it; each row saves as it changes and says what happened.

use std::{
    cell::RefCell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use adw::prelude::*;
use gtk::gio;

use crate::config::{self, Saved, Source};

/// What the app does after a setting is saved: read the settings again
/// and apply what it can now. An error is shown with the setting.
pub type OnSaved = Rc<dyn Fn() -> Result<(), String>>;

impl Prefs {
    /// A Preferences dialog over the app's config file, for `settings`
    /// (those in `at_start` apply when the app next starts). None if there
    /// is no config file to edit (no home folder).
    pub fn open(
        settings: &'static [(&'static str, &'static str)],
        at_start: &'static [&'static str],
        on_saved: OnSaved,
    ) -> Option<Rc<Self>> {
        let path = config::config_path()?;
        let dialog = adw::PreferencesDialog::builder()
            .title("Preferences")
            .search_enabled(true)
            .build();
        let page = adw::PreferencesPage::new();
        dialog.add(&page);
        let now = config::current(Some(&path), settings);
        let prefs = Rc::new(Self {
            path,
            dialog,
            page: page.clone(),
            settings,
            at_start,
            on_saved,
            now,
            notice: RefCell::new(None),
        });
        // where changes go
        let place = adw::PreferencesGroup::new();
        page.add(&place);
        *prefs.notice.borrow_mut() = Some(place);
        prefs.describe_place(None);
        Some(prefs)
    }

    /// A titled group of rows, added to the page.
    pub fn group(&self, title: &str) -> adw::PreferencesGroup {
        let g = adw::PreferencesGroup::builder().title(title).build();
        self.page.add(&g);
        g
    }

    /// A setting's value as the dialog opened, if set.
    pub fn value(&self, name: &str) -> Option<String> {
        self.now.get(name).map(|(v, _)| v.clone())
    }

    /// The environment variable that sets `name`, if one does (then it
    /// can't be changed here).
    pub fn env(&self, name: &str) -> Option<String> {
        match self.now.get(name) {
            Some((_, Source::Env(var))) => Some(var.clone()),
            _ => None,
        }
    }

    pub fn present(&self, parent: &impl IsA<gtk::Widget>) {
        self.dialog.present(Some(parent.as_ref()));
    }
}

pub fn owned(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

pub fn is_true(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1")
}

pub struct Prefs {
    path: PathBuf,
    dialog: adw::PreferencesDialog,
    page: adw::PreferencesPage,
    settings: &'static [(&'static str, &'static str)],
    at_start: &'static [&'static str],
    on_saved: OnSaved,
    /// The settings as the dialog opened, and where each came from.
    now: HashMap<String, (String, Source)>,
    /// The group that says where changes are saved.
    notice: RefCell<Option<adw::PreferencesGroup>>,
}

impl Prefs {
    /// Save one setting, apply it, and say what happened.
    fn save(&self, name: &str, value: Option<String>) {
        let value = value.filter(|v| !v.trim().is_empty());
        match config::save(&self.path, name, value.as_deref()) {
            Err(e) => self.toast(&format!("Could not save: {e}")),
            Ok(saved) => {
                self.describe_place(Some(&saved));
                // a file read later may still say otherwise
                let now = config::current(Some(&self.path), self.settings);
                let overridden = match (&saved, now.get(name)) {
                    (_, Some((v, Source::File(f)))) if Some(v) != value.as_ref() => {
                        Some(f.display().to_string())
                    }
                    _ => None,
                };
                if let Some(file) = overridden {
                    self.toast(&format!("Saved, but {file} sets it again later"));
                } else if let Err(e) = (self.on_saved)() {
                    self.toast(&e);
                } else if self.at_start.contains(&name) {
                    self.toast(&format!(
                        "Saved: applies when {} next starts",
                        crate::app::get().name
                    ));
                }
            }
        }
    }

    pub fn toast(&self, text: &str) {
        self.dialog.add_toast(adw::Toast::new(text));
    }

    /// Say where changes are saved: the config, or (when it is read-only)
    /// `local.conf`, with the saved settings to copy into Home Manager.
    fn describe_place(&self, saved: Option<&Saved>) {
        let Some(group) = self.notice.borrow().clone() else {
            return;
        };
        let local = config::local_path(&self.path);
        let read_only = match saved {
            Some(Saved::Local(_)) => true,
            Some(Saved::InPlace(_)) => false,
            None => !config::writable(&self.path),
        };
        if read_only {
            group.set_title("Saving to local.conf");
            group.set_description(Some(&format!(
                "{} can’t be changed (Nix or another tool made it), so changes go to {} \
                 beside it, which is read after it.",
                tilde(&self.path),
                tilde(&local),
            )));
        } else {
            group.set_title("");
            group.set_description(Some(&format!(
                "Changes are saved to {} as you make them.",
                tilde(&self.path)
            )));
        }
        // the file itself, for what this dialog doesn't cover
        if group.header_suffix().is_none() {
            let open = gtk::Button::builder()
                .label("Open file")
                .valign(gtk::Align::Center)
                .css_classes(["flat"])
                .build();
            let path = self.path.clone();
            open.connect_clicked(move |b| {
                let file = gio::File::for_path(&path);
                gtk::FileLauncher::new(Some(&file)).launch(
                    b.root().and_downcast_ref::<gtk::Window>(),
                    None::<&gio::Cancellable>,
                    |_| {},
                );
            });
            group.set_header_suffix(Some(&open));
        }
    }

    /// A row that can't be changed here as the environment sets it.
    fn from_env(row: &impl IsA<gtk::Widget>, var: &str) {
        row.set_sensitive(false);
        let why = format!("Set by {var} in the environment, which wins over the settings file");
        row.set_tooltip_text(Some(&why));
        // and in the row itself, as a greyed-out row says nothing of why
        if let Some(r) = row.dynamic_cast_ref::<adw::ActionRow>() {
            r.set_subtitle(&format!("Set by {var}"));
        } else if let Some(r) = row.dynamic_cast_ref::<adw::PreferencesRow>() {
            r.set_title(&format!("{} (set by {var})", r.title()));
        }
    }

    pub fn path_row(
        self: &Rc<Self>,
        name: &'static str,
        title: &str,
        hint: &str,
        value: Option<String>,
        env: Option<String>,
        folder: bool,
    ) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(title)
            .subtitle(value.as_deref().unwrap_or(hint))
            .subtitle_selectable(true)
            .build();
        if let Some(var) = env {
            Self::from_env(&row, &var);
            row.set_subtitle(&format!("{} (set by {var})", value.unwrap_or_default()));
            return row;
        }
        let choose = gtk::Button::builder()
            .icon_name(if folder {
                "folder-open-symbolic"
            } else {
                "document-open-symbolic"
            })
            .tooltip_text("Choose")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .build();
        let clear = gtk::Button::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Use the default")
            .valign(gtk::Align::Center)
            .css_classes(["flat"])
            .visible(value.is_some() && name != "vdir")
            .build();
        row.add_suffix(&choose);
        row.add_suffix(&clear);
        let this = self.clone();
        let (r, c, hint) = (row.clone(), clear.clone(), hint.to_owned());
        clear.connect_clicked(move |_| {
            this.save(name, None);
            r.set_subtitle(&hint);
            c.set_visible(false);
        });
        let this = self.clone();
        let (r, c) = (row.clone(), clear);
        choose.connect_clicked(move |b| {
            let dialog = gtk::FileDialog::builder().modal(true).build();
            let (this, r, c) = (this.clone(), r.clone(), c.clone());
            let done = move |file: Result<gio::File, gtk::glib::Error>| {
                let Some(p) = file.ok().and_then(|f| f.path()) else {
                    return;
                };
                let shown = tilde(&p);
                this.save(name, Some(shown.clone()));
                r.set_subtitle(&shown);
                c.set_visible(name != "vdir");
            };
            let window = b.root().and_downcast::<gtk::Window>();
            if folder {
                dialog.select_folder(window.as_ref(), None::<&gio::Cancellable>, done);
            } else {
                dialog.open(window.as_ref(), None::<&gio::Cancellable>, done);
            }
        });
        row
    }

    pub fn combo_row(
        self: &Rc<Self>,
        name: &'static str,
        title: &str,
        options: &[(String, String)],
        value: String,
        env: Option<String>,
    ) -> adw::ComboRow {
        let mut options = options.to_vec();
        // a value set by hand that isn't offered stays offered
        if !options.iter().any(|(v, _)| *v == value) {
            options.push((value.clone(), value.clone()));
        }
        let labels = options.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>();
        let row = adw::ComboRow::builder()
            .title(title)
            .model(&gtk::StringList::new(&labels))
            .selected(options.iter().position(|(v, _)| *v == value).unwrap_or(0) as u32)
            .build();
        if let Some(var) = env {
            Self::from_env(&row, &var);
            return row;
        }
        let this = self.clone();
        row.connect_selected_notify(move |r| {
            if let Some((v, _)) = options.get(r.selected() as usize) {
                this.save(name, Some(v.clone()));
            }
        });
        row
    }

    pub fn entry_row(
        self: &Rc<Self>,
        name: &'static str,
        title: &str,
        value: Option<String>,
        env: Option<String>,
        check: impl Fn(&str) -> Result<(), String> + 'static,
    ) -> adw::EntryRow {
        let row = adw::EntryRow::builder()
            .title(title)
            .text(value.unwrap_or_default())
            .show_apply_button(true)
            .build();
        if let Some(var) = env {
            Self::from_env(&row, &var);
            return row;
        }
        let this = self.clone();
        row.connect_apply(move |r| {
            let text = r.text().trim().to_owned();
            if !text.is_empty()
                && let Err(e) = check(&text)
            {
                r.add_css_class("error");
                this.toast(&e);
                return;
            }
            r.remove_css_class("error");
            this.save(name, Some(text));
        });
        row
    }

    pub fn switch_row(
        self: &Rc<Self>,
        name: &'static str,
        title: &str,
        subtitle: &str,
        on: bool,
        env: Option<String>,
        value: impl Fn(bool) -> Option<String> + 'static,
    ) -> adw::SwitchRow {
        let row = adw::SwitchRow::builder()
            .title(title)
            .subtitle(subtitle)
            .active(on)
            .build();
        if let Some(var) = env {
            Self::from_env(&row, &var);
            return row;
        }
        let this = self.clone();
        row.connect_active_notify(move |r| this.save(name, value(r.is_active())));
        row
    }
}

/// A path with the home folder as `~`, as the config would have it.
fn tilde(p: &Path) -> String {
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if p.starts_with(&home) => {
            format!("~/{}", p.strip_prefix(&home).unwrap().display())
        }
        _ => p.display().to_string(),
    }
}

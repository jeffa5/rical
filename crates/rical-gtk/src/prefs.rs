//! Preferences: the config file's settings, edited in place. A file the
//! user can't write (one Nix made, say) is left alone and changes go to
//! `local.conf` beside it, with how to include that.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
};

use adw::prelude::*;
use gtk::gio;

use crate::config::{self, Saved, Source};

/// What the window does after a setting is saved: read the settings again
/// and apply what it can now. An error is shown with the setting.
pub type OnSaved = Rc<dyn Fn() -> Result<(), String>>;

/// Settings that only take effect when rical starts.
const AT_START: &[&str] = &["vdir", "view", "binds", "renderer"];

pub fn show(parent: &impl IsA<gtk::Widget>, calendars: Vec<(String, String)>, on_saved: OnSaved) {
    let Some(path) = config::config_path() else {
        return;
    };
    let dialog = adw::PreferencesDialog::builder()
        .title("Preferences")
        .search_enabled(true)
        .build();
    let page = adw::PreferencesPage::new();
    dialog.add(&page);

    let ctx = Rc::new(Ctx {
        path: path.clone(),
        dialog: dialog.clone(),
        on_saved,
        notice: RefCell::new(None),
    });
    let now = config::current(Some(&path));
    let value = |name: &str| now.get(name).map(|(v, _)| v.clone());
    let env = |name: &str| match now.get(name) {
        Some((_, Source::Env(var))) => Some(var.clone()),
        _ => None,
    };

    // where changes go, and what to do if that file isn't read
    let place = adw::PreferencesGroup::new();
    page.add(&place);
    *ctx.notice.borrow_mut() = Some(place.clone());
    ctx.describe_place(None);

    let group = |title: &str| {
        let g = adw::PreferencesGroup::builder().title(title).build();
        page.add(&g);
        g
    };

    let calendars_group = group("Calendars");
    calendars_group.add(&ctx.path_row(
        "vdir",
        "Calendar folder",
        "A folder of calendar folders, e.g. vdirsyncer's",
        value("vdir"),
        env("vdir"),
        true,
    ));
    let mut names = vec![("".to_owned(), "Automatic".to_owned())];
    names.extend(calendars);
    calendars_group.add(&ctx.combo_row(
        "calendar",
        "New events go in",
        &names,
        value("calendar").unwrap_or_default(),
        env("calendar"),
    ));
    calendars_group.add(&ctx.path_row(
        "contacts",
        "Contacts folder",
        "vCards to suggest guests from and name people by",
        value("contacts"),
        env("contacts"),
        true,
    ));
    calendars_group.add(&ctx.entry_row(
        "me",
        "My email addresses (comma-separated; guessed if empty)",
        value("me"),
        env("me"),
        |_| Ok(()),
    ));

    let views = group("Views");
    let view_names = [
        ("", "Week (the default)"),
        ("day", "Day"),
        ("3days", "3 days"),
        ("month", "Month"),
        ("year", "Year"),
        ("agenda", "Agenda"),
        ("tasks", "Tasks"),
        ("journal", "Journal"),
    ];
    views.add(&ctx.combo_row(
        "view",
        "Open on",
        &owned(&view_names),
        value("view").unwrap_or_default(),
        env("view"),
    ));
    let week_names = [
        ("", "Full week (the default)"),
        ("workweek", "Work week"),
        ("3days", "3 days"),
        ("5days", "5 days"),
    ];
    views.add(&ctx.combo_row(
        "week",
        "The week view shows",
        &owned(&week_names),
        value("week").unwrap_or_default(),
        env("week"),
    ));
    views.add(&ctx.entry_row(
        "zones",
        "Extra time zones (e.g. UK=Europe/London, America/New_York)",
        value("zones"),
        env("zones"),
        |v| crate::zones::Zone::parse_list(v).map(|_| ()),
    ));

    let reminders = group("Reminders");
    reminders.add(&ctx.switch_row(
        "no-notifications",
        "Show reminders",
        "Off to leave them to `rical remind` running as a service",
        // the setting says the opposite
        !value("no-notifications").is_some_and(|v| is_true(&v)),
        env("no-notifications"),
        |on| (!on).then(|| "true".to_owned()),
    ));
    reminders.add(&ctx.entry_row(
        "remind",
        "Remind before events without their own (e.g. 10m)",
        value("remind"),
        env("remind"),
        |v| {
            rical_core::input::parse_shift(v)
                .filter(|d| *d > chrono::TimeDelta::zero())
                .map(|_| ())
                .ok_or_else(|| format!("not a length of time: “{v}”"))
        },
    ));

    let advanced = group("Advanced");
    advanced.add(&ctx.path_row(
        "binds",
        "Key bindings file",
        "Default ~/.config/rical/binds.conf",
        value("binds"),
        env("binds"),
        false,
    ));
    let renderers = [
        ("", "Cairo (the default: least memory)"),
        ("gl", "OpenGL"),
        ("vulkan", "Vulkan"),
        ("auto", "GTK's choice"),
    ];
    advanced.add(
        &ctx.combo_row(
            "renderer",
            "Drawing",
            &owned(&renderers),
            value("renderer")
                .filter(|v| v != "cairo")
                .unwrap_or_default(),
            env("renderer"),
        ),
    );

    dialog.present(Some(parent.as_ref()));
}

fn owned(items: &[(&str, &str)]) -> Vec<(String, String)> {
    items
        .iter()
        .map(|(a, b)| ((*a).to_owned(), (*b).to_owned()))
        .collect()
}

fn is_true(v: &str) -> bool {
    matches!(v.to_ascii_lowercase().as_str(), "true" | "yes" | "on" | "1")
}

struct Ctx {
    path: PathBuf,
    dialog: adw::PreferencesDialog,
    on_saved: OnSaved,
    /// The group that says where changes are saved.
    notice: RefCell<Option<adw::PreferencesGroup>>,
}

impl Ctx {
    /// Save one setting, apply it, and say what happened.
    fn save(&self, name: &str, value: Option<String>) {
        let value = value.filter(|v| !v.trim().is_empty());
        match config::save(&self.path, name, value.as_deref()) {
            Err(e) => self.toast(&format!("Could not save: {e}")),
            Ok(saved) => {
                self.describe_place(Some(&saved));
                // a file read later may still say otherwise
                let now = config::current(Some(&self.path));
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
                } else if AT_START.contains(&name) {
                    self.toast("Saved: applies next time rical starts");
                }
            }
        }
    }

    fn toast(&self, text: &str) {
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
                 beside it, which rical reads after it.",
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

    fn path_row(
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

    fn combo_row(
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

    fn entry_row(
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

    fn switch_row(
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

//! The first run: where the mail is and who you are, written to the
//! settings file, then the app as usual.

use std::{path::PathBuf, rc::Rc};

use adw::prelude::*;
use gtk::{gio, glib};

/// Ask, and call `done` once the settings are written.
pub fn open(app: &adw::Application, done: Rc<dyn Fn()>) {
    let chosen: Rc<std::cell::RefCell<Option<PathBuf>>> = Rc::default();
    let folder = adw::ActionRow::builder()
        .title("Mail folder")
        .subtitle("The folder of Maildir folders your mail sync keeps (mbsync, offlineimap)")
        .activatable(true)
        .build();
    let pick = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    folder.add_suffix(&pick);
    folder.set_activatable_widget(Some(&pick));
    let me = adw::EntryRow::builder()
        .title("Your name and address (Name <you@example.com>)")
        .use_markup(false)
        .build();
    let contacts = adw::ActionRow::builder()
        .title("Contacts folder (optional)")
        .subtitle("Address books of vCards, to complete addresses from")
        .build();
    let pick_contacts = gtk::Button::builder()
        .label("Choose…")
        .valign(gtk::Align::Center)
        .build();
    contacts.add_suffix(&pick_contacts);
    let contacts_chosen: Rc<std::cell::RefCell<Option<PathBuf>>> = Rc::default();
    let group = adw::PreferencesGroup::new();
    group.add(&folder);
    group.add(&me);
    group.add(&contacts);
    let error = gtk::Label::builder()
        .css_classes(["error"])
        .wrap(true)
        .visible(false)
        .build();
    let go = gtk::Button::builder()
        .label("Start")
        .css_classes(["suggested-action", "pill"])
        .halign(gtk::Align::Center)
        .sensitive(false)
        .build();
    let column = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    column.append(&group);
    column.append(&error);
    column.append(&go);
    let page = adw::StatusPage::builder()
        .icon_name(crate::APP_ID)
        .title("Welcome to Mail")
        .description("Mail reads the folders your mail sync keeps and sends through rical-mail-queue. Nothing in a message is ever loaded from the web.")
        .child(&adw::Clamp::builder().maximum_size(560).child(&column).build())
        .build();
    let view = adw::ToolbarView::new();
    view.add_top_bar(&adw::HeaderBar::new());
    view.set_content(Some(&gtk::ScrolledWindow::builder().child(&page).build()));
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title(crate::APP_NAME)
        .default_width(720)
        .default_height(640)
        .content(&view)
        .build();

    let choose = |button: &gtk::Button,
                  row: adw::ActionRow,
                  into: Rc<std::cell::RefCell<Option<PathBuf>>>,
                  then: Rc<dyn Fn()>| {
        let w = window.clone();
        button.connect_clicked(move |_| {
            let dialog = gtk::FileDialog::builder().title("Choose a folder").build();
            let (row, into, then) = (row.clone(), into.clone(), then.clone());
            dialog.select_folder(Some(&w), None::<&gio::Cancellable>, move |r| {
                if let Ok(f) = r
                    && let Some(p) = f.path()
                {
                    row.set_subtitle(&p.to_string_lossy());
                    *into.borrow_mut() = Some(p);
                    then();
                }
            });
        });
    };
    let ready: Rc<dyn Fn()> = {
        let (chosen, go) = (chosen.clone(), go.clone());
        Rc::new(move || go.set_sensitive(chosen.borrow().is_some()))
    };
    choose(&pick, folder.clone(), chosen.clone(), ready.clone());
    choose(
        &pick_contacts,
        contacts.clone(),
        contacts_chosen.clone(),
        Rc::new(|| {}),
    );

    let w = window.clone();
    go.connect_clicked(move |_| {
        let Some(path) = rical_ui::config::config_path() else {
            error.set_label("There is no home folder to keep the settings in.");
            error.set_visible(true);
            return;
        };
        let address = me.text().trim().to_owned();
        if !address.is_empty() && rical_maildir::addr::parse_list(&address).is_empty() {
            error.set_label("Write your address as Name <you@example.com>, or leave it for later.");
            error.set_visible(true);
            return;
        }
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut settings: Vec<(&str, String)> = Vec::new();
        if let Some(m) = chosen.borrow().as_ref() {
            settings.push(("mail", m.to_string_lossy().into_owned()));
        }
        if !address.is_empty() {
            settings.push(("me", address));
        }
        if let Some(c) = contacts_chosen.borrow().as_ref() {
            settings.push(("contacts", c.to_string_lossy().into_owned()));
        }
        for (name, value) in settings {
            if let Err(e) = rical_ui::config::save(&path, name, Some(&value)) {
                error.set_label(&e);
                error.set_visible(true);
                return;
            }
        }
        w.close();
        done();
    });
    window.present();
    glib::idle_add_local_once(move || {
        pick.grab_focus();
    });
}

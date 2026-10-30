//! The search helper: a form for from, to, subject, text, folder, dates and
//! flags, that writes the query in the words of whatever does the
//! searching (mu, notmuch, or this app), shown as it is built.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;
use rical_maildir::search::{Filters, Query, Syntax};

use crate::window::Window;

const PRESETS: [(&str, &str); 6] = [
    ("Any time", ""),
    ("Today", "today"),
    ("The past week", "1w"),
    ("The past month", "1m"),
    ("The past three months", "3m"),
    ("The past year", "1y"),
];

/// The form, filled from what is in the search box.
fn seed(text: &str) -> Filters {
    let q = Query::parse(text);
    let join = |v: &[String]| v.join(" ");
    Filters {
        words: join(&q.words),
        from: join(&q.from),
        to: join(&q.to),
        subject: join(&q.subject),
        body: join(&q.body),
        folder: q.folder.first().cloned().unwrap_or_default(),
        unread: q.unread,
        flagged: q.flagged,
        attachment: q.attachment,
        ..Filters::default()
    }
}

pub fn open(win: &Rc<Window>) {
    let (syntax, tool, folders) = {
        let s = win.state.borrow();
        (
            s.opts.search.syntax(),
            s.opts.search.name().to_owned(),
            s.folders.iter().map(|f| f.name.clone()).collect::<Vec<_>>(),
        )
    };
    let start = seed(&win.search.text());

    let entry = |title: &str, text: &str| adw::EntryRow::builder().title(title).text(text).build();
    let words = entry("Words (anywhere)", &start.words);
    let from = entry("From", &start.from);
    let to = entry("To", &start.to);
    let subject = entry("Subject", &start.subject);
    let body = entry("Text of the message", &start.body);
    let find = adw::PreferencesGroup::builder().title("Find").build();
    for r in [&words, &from, &to, &subject, &body] {
        find.add(r);
    }

    let mut names = vec!["Everywhere".to_owned()];
    names.extend(folders.iter().cloned());
    let folder = adw::ComboRow::builder()
        .title("Folder")
        .model(&gtk::StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        ))
        .build();
    if let Some(i) = names
        .iter()
        .position(|n| !start.folder.is_empty() && n.to_lowercase().contains(&start.folder))
    {
        folder.set_selected(i as u32);
    }
    let when = adw::ComboRow::builder()
        .title("Date")
        .model(&gtk::StringList::new(
            &PRESETS.iter().map(|(l, _)| *l).collect::<Vec<_>>(),
        ))
        .build();
    let since = entry("Since (2w, yesterday, 2026-10-01)", "");
    rical_ui::datetime::attach(&since, rical_ui::datetime::Config::date());
    let until = entry("Until (a date)", "");
    rical_ui::datetime::attach(&until, rical_ui::datetime::Config::date());
    let place = adw::PreferencesGroup::builder()
        .title("Where and when")
        .build();
    for r in [
        folder.upcast_ref::<gtk::Widget>(),
        when.upcast_ref(),
        since.upcast_ref(),
        until.upcast_ref(),
    ] {
        place.add(r);
    }
    let switch = |title: &str, on: bool| adw::SwitchRow::builder().title(title).active(on).build();
    let unread = switch("Only unread", start.unread);
    let flagged = switch("Only flagged", start.flagged);
    let attachment = switch("Only with attachments", start.attachment);
    let only = adw::PreferencesGroup::builder().title("Only").build();
    for r in [&unread, &flagged, &attachment] {
        only.add(r);
    }

    let preview = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .selectable(true)
        .css_classes(["monospace"])
        .build();
    let note = gtk::Label::builder()
        .xalign(0.0)
        .wrap(true)
        .css_classes(["dim-label", "caption"])
        .label(format!("Written for {tool}"))
        .build();
    let shown = adw::PreferencesGroup::builder().title("The search").build();
    let pv = gtk::Box::new(gtk::Orientation::Vertical, 4);
    pv.append(&preview);
    pv.append(&note);
    shown.add(&pv);

    let page = adw::PreferencesPage::new();
    for g in [&shown, &find, &place, &only] {
        page.add(g);
    }

    let collect = {
        let (words, from, to, subject, body) = (
            words.clone(),
            from.clone(),
            to.clone(),
            subject.clone(),
            body.clone(),
        );
        let (folder, since, until) = (folder.clone(), since.clone(), until.clone());
        let (unread, flagged, attachment) = (unread.clone(), flagged.clone(), attachment.clone());
        let names = names.clone();
        Rc::new(move || -> Filters {
            Filters {
                words: words.text().into(),
                from: from.text().into(),
                to: to.text().into(),
                subject: subject.text().into(),
                body: body.text().into(),
                folder: match folder.selected() {
                    0 => String::new(),
                    i => names.get(i as usize).cloned().unwrap_or_default(),
                },
                unread: unread.is_active(),
                flagged: flagged.is_active(),
                attachment: attachment.is_active(),
                since: since.text().into(),
                until: until.text().into(),
            }
        })
    };
    let update = {
        let (collect, preview) = (collect.clone(), preview.clone());
        Rc::new(move || {
            let q = collect().to_query(syntax);
            preview.set_label(if q.is_empty() { "(everything)" } else { &q });
        })
    };
    for r in [&words, &from, &to, &subject, &body, &since, &until] {
        let u = update.clone();
        r.connect_changed(move |_| u());
    }
    for r in [&unread, &flagged, &attachment] {
        let u = update.clone();
        r.connect_active_notify(move |_| u());
    }
    let u = update.clone();
    folder.connect_selected_notify(move |_| u());
    // a preset fills Since (and empties Until)
    let (s, un) = (since.clone(), until.clone());
    when.connect_selected_notify(move |r| {
        let v = PRESETS.get(r.selected() as usize).map_or("", |p| p.1);
        s.set_text(v);
        un.set_text("");
    });
    update();

    let go = gtk::Button::builder()
        .label("Search")
        .css_classes(["suggested-action"])
        .build();
    let here = gtk::Button::builder()
        .label("Filter This Folder")
        .tooltip_text("Narrow the folder you are in, as you type, with these")
        .build();
    let header = adw::HeaderBar::new();
    header.pack_end(&go);
    header.pack_start(&here);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(
        &gtk::ScrolledWindow::builder()
            .child(&page)
            .vexpand(true)
            .build(),
    ));
    let dialog = adw::Dialog::builder()
        .title("Search")
        .content_width(560)
        .content_height(720)
        .child(&view)
        .build();

    let last: Rc<RefCell<Option<Filters>>> = Rc::default();
    let (weak, d, c) = (Rc::downgrade(win), dialog.clone(), collect.clone());
    go.connect_clicked(move |_| {
        let Some(w) = weak.upgrade() else { return };
        let q = c().to_query(syntax);
        d.close();
        if q.is_empty() {
            w.toast("Say what to look for");
        } else if let Err(e) = w.execute(&format!("search {}", rical_ui::command::quote(&q))) {
            w.toast(&e);
        }
    });
    let (weak, d, c) = (Rc::downgrade(win), dialog.clone(), collect.clone());
    here.connect_clicked(move |_| {
        let Some(w) = weak.upgrade() else { return };
        // this folder is always searched by this app, in its own words
        let q = c().to_query(Syntax::Builtin);
        d.close();
        w.search.set_text(&q);
        w.search_now();
        w.focus_view();
    });
    let _ = last;
    dialog.present(Some(&win.window));
    glib::idle_add_local_once({
        let from = from.clone();
        move || {
            from.grab_focus();
        }
    });
}

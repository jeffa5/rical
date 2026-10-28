//! Tidying up: the cards that look like one person, side by side, and
//! merging them into one. The merged card keeps everything any of them
//! said; where they disagree (two birthdays), you choose. Then the others
//! are deleted, after asking (deleting is the default), all as one undo.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use rical_cards::{Card, dedupe, edit::Draft};
use rical_core::write::Change;

use crate::{state::State, window::Window};

fn escape(s: &str) -> String {
    gtk::glib::markup_escape_text(s).to_string()
}

fn row(title: &str, subtitle: &str, icon: &str) -> adw::ActionRow {
    let r = adw::ActionRow::builder()
        .title(escape(title))
        .subtitle(escape(subtitle))
        .title_selectable(true)
        .build();
    r.add_prefix(&gtk::Image::from_icon_name(icon));
    r
}

/// The lines of one card worth comparing.
fn card_group(card: &Card, book: &str, place: &str) -> adw::PreferencesGroup {
    let g = adw::PreferencesGroup::builder()
        .title(escape(&card.display_name()))
        .description(escape(&format!("in {book} · {place}")))
        .build();
    for r in draft_rows(&Draft::of(card)) {
        g.add(&r);
    }
    g
}

/// What a card (or a merged one) says, as rows.
fn draft_rows(d: &Draft) -> Vec<adw::ActionRow> {
    let mut g = Vec::new();
    let org = [d.title.clone(), d.org.join(", ")]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    if !org.is_empty() {
        g.push(row(&org, "work", "network-workgroup-symbolic"));
    }
    for e in &d.emails {
        g.push(row(&e.value, &e.labels.join(", "), "mail-send-symbolic"));
    }
    for p in &d.phones {
        g.push(row(&p.value, &p.labels.join(", "), "call-start-symbolic"));
    }
    for a in &d.addresses {
        g.push(row(
            &a.one_line(),
            &a.labels.join(", "),
            "mark-location-symbolic",
        ));
    }
    for u in &d.urls {
        g.push(row(&u.value, "", "web-browser-symbolic"));
    }
    if !d.birthday.is_empty() {
        let today = chrono::Local::now().date_naive();
        let words = rical_ui::card::date_words(&d.birthday, today).unwrap_or(d.birthday.clone());
        g.push(row(&words, "birthday", "x-office-calendar-symbolic"));
    }
    if !d.notes.is_empty() {
        let first = d.notes.lines().next().unwrap_or("");
        g.push(row(first, "note", "document-edit-symbolic"));
    }
    g
}

/// The selected duplicates side by side, with what to do about them.
pub fn preview(s: &State) -> gtk::Widget {
    let groups = s.cached_groups();
    let Some(group) = s.selected_group.and_then(|i| groups.get(i)) else {
        return adw::StatusPage::builder()
            .icon_name("edit-copy-symbolic")
            .title(if groups.is_empty() {
                "No duplicates"
            } else {
                "Choose some duplicates"
            })
            .description(if groups.is_empty() {
                "Everyone is in your contacts once"
            } else {
                "Cards that share an email address, a number or a name"
            })
            .vexpand(true)
            .build()
            .upcast();
    };
    let b = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(18)
        .build();
    let why = group
        .reasons
        .iter()
        .map(|r| r.describe())
        .collect::<Vec<_>>()
        .join("; ");
    b.append(
        &gtk::Label::builder()
            .label(format!(
                "{} cards that look like {}",
                group.cards.len(),
                if group.is_strong() {
                    "the same person"
                } else {
                    "they may be the same person"
                }
            ))
            .css_classes(["title-2"])
            .wrap(true)
            .build(),
    );
    b.append(
        &gtk::Label::builder()
            .label(format!("They have {why}"))
            .css_classes(["dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build(),
    );
    let buttons = gtk::Box::builder()
        .spacing(6)
        .halign(gtk::Align::Center)
        .build();
    buttons.append(
        &gtk::Button::builder()
            .label("Merge…")
            .tooltip_text("Merge them into one (m)")
            .css_classes(["suggested-action", "pill"])
            .action_name("win.run")
            .action_target(&"merge".to_variant())
            .build(),
    );
    buttons.append(
        &gtk::Button::builder()
            .label("Not the Same")
            .tooltip_text("They are different people: don't suggest them again (i)")
            .css_classes(["pill"])
            .action_name("win.run")
            .action_target(&"not-duplicates".to_variant())
            .build(),
    );
    b.append(&buttons);
    for id in &group.cards {
        if let (Some(card), Some(book)) = (s.books.get(id), s.books.book(id)) {
            b.append(&card_group(&card, &book.name, &s.books.place(&id.path)));
        }
    }
    b.upcast()
}

/// Merge the selected duplicates: choose where they disagree, save the
/// merged card, then delete the others (asked, defaulting to yes).
pub fn merge(win: &Rc<Window>) -> Result<(), String> {
    let group = win.selected_group().ok_or("no duplicates selected")?;
    merge_group(win, group, None)
}

/// A card just added from another app, to merge into one you have.
pub struct Imported {
    /// Adding it, to undo if the merge is cancelled.
    pub added: Change,
    /// What to do once the merge is done or cancelled (the next card).
    pub then: Rc<dyn Fn()>,
}

/// Merge some cards into one, as [`merge`]. An imported card goes into the
/// one you had (the first), without asking to delete it: it was only ever
/// a copy.
pub fn merge_group(
    win: &Rc<Window>,
    group: dedupe::Group,
    imported: Option<Imported>,
) -> Result<(), String> {
    let mut plan = dedupe::plan(&win.state.borrow().books, &group.cards)
        .ok_or("those cards are no longer there")?;
    if imported.is_some() && plan.keep != group.cards[0] {
        plan.others = vec![std::mem::replace(&mut plan.keep, group.cards[0].clone())];
    }
    let plan = Rc::new(RefCell::new(plan));
    let count = group.cards.len();

    let dialog = adw::Dialog::builder()
        .title(format!("Merge {count} Contacts"))
        .content_width(520)
        .content_height(640)
        .build();
    let page = adw::PreferencesPage::new();
    // where it goes: the kept card's file, and the others' (deleted after
    // asking; an imported copy goes without)
    let files = adw::PreferencesGroup::builder().title("Files").build();
    {
        let s = win.state.borrow();
        let p = plan.borrow();
        let row = |place: String, what: &str, icon: &str| {
            let r = adw::ActionRow::builder()
                .title(escape(&place))
                .subtitle(what)
                .title_selectable(true)
                .build();
            r.add_prefix(&gtk::Image::from_icon_name(icon));
            r
        };
        files.add(&row(
            s.books.place(&p.keep.path),
            "saved here, as the merged card",
            "document-save-symbolic",
        ));
        let gone = if imported.is_some() {
            "the opened card, not kept"
        } else {
            "deleted afterwards (you're asked first)"
        };
        for other in &p.others {
            files.add(&row(
                s.books.place(&other.path),
                gone,
                "user-trash-symbolic",
            ));
        }
    }
    page.add(&files);

    let merged_group = adw::PreferencesGroup::builder()
        .title("The merged card")
        .description("Everything each card says, once")
        .build();
    let merged = gtk::ListBox::builder()
        .css_classes(["boxed-list"])
        .selection_mode(gtk::SelectionMode::None)
        .build();
    merged_group.add(&merged);
    if !plan.borrow().conflicts.is_empty() {
        let choose = adw::PreferencesGroup::builder()
            .title("Where they differ")
            .description("Choose which to keep")
            .build();
        let conflicts = plan.borrow().conflicts.clone();
        for (c, conflict) in conflicts.iter().enumerate() {
            let options: Vec<&str> = conflict.options.iter().map(String::as_str).collect();
            let combo = adw::ComboRow::builder()
                .title(conflict.field)
                .model(&gtk::StringList::new(&options))
                .selected(conflict.chosen as u32)
                .build();
            let plan = plan.clone();
            let merged = merged.clone();
            combo.connect_selected_notify(move |r| {
                plan.borrow_mut().choose(c, r.selected() as usize);
                refill(&merged, &plan.borrow().draft);
            });
            choose.add(&combo);
        }
        page.add(&choose);
    }
    refill(&merged, &plan.borrow().draft);
    page.add(&merged_group);

    let cancel = gtk::Button::with_label("Cancel");
    let go = gtk::Button::builder()
        .label("Merge")
        .css_classes(["suggested-action"])
        .tooltip_text("Merge (Ctrl+Enter)")
        .build();
    let header = adw::HeaderBar::builder()
        .show_end_title_buttons(false)
        .show_start_title_buttons(false)
        .build();
    header.pack_start(&cancel);
    header.pack_end(&go);
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&page));
    dialog.set_child(Some(&view));
    let d = dialog.clone();
    cancel.connect_clicked(move |_| {
        d.close();
    });

    let imported = Rc::new(RefCell::new(imported));
    let do_merge: Rc<dyn Fn()> = Rc::new({
        let (win, dialog, plan, imported) =
            (win.clone(), dialog.clone(), plan.clone(), imported.clone());
        move || {
            let p = plan.borrow().clone();
            let saved = win.state.borrow_mut().books.update(&p.keep, &p.draft);
            let change = match saved {
                Ok(c) => c,
                Err(e) => return win.toast(&format!("{e:#}")),
            };
            // taken first, so closing (which looks again) finds it gone
            let taken = imported.borrow_mut().take();
            dialog.close();
            match taken {
                Some(i) => {
                    finish(&win, &p, vec![i.added, change], true);
                    (i.then)();
                }
                None => confirm_delete(&win, p, change),
            }
        }
    });
    // cancelled: an imported card goes again
    let w = win.clone();
    dialog.connect_closed(move |_| {
        let taken = imported.borrow_mut().take();
        if let Some(i) = taken {
            if let Err(e) = w.state.borrow_mut().books.undo(&i.added) {
                w.toast(&format!("{e:#}"));
            }
            w.after_change(None);
            (i.then)();
        }
    });
    let m = do_merge.clone();
    go.connect_clicked(move |_| m());
    let shortcuts = gtk::ShortcutController::new();
    shortcuts.set_scope(gtk::ShortcutScope::Managed);
    let m = do_merge.clone();
    shortcuts.add_shortcut(gtk::Shortcut::new(
        gtk::ShortcutTrigger::parse_string("<Control>Return"),
        Some(gtk::CallbackAction::new(move |_, _| {
            m();
            gtk::glib::Propagation::Stop
        })),
    ));
    dialog.add_controller(shortcuts);
    dialog.set_focus(Some(&go));
    dialog.present(Some(&win.window));
    Ok(())
}

/// Show the merged card as it now is.
fn refill(list: &gtk::ListBox, d: &Draft) {
    list.remove_all();
    list.append(&row(&d.formatted_name, "name", "avatar-default-symbolic"));
    for r in draft_rows(d) {
        list.append(&r);
    }
}

fn name_of(plan: &dedupe::Merge) -> String {
    if plan.draft.formatted_name.is_empty() {
        "them".to_owned()
    } else {
        plan.draft.formatted_name.clone()
    }
}

/// The merged card is saved: delete the others too? Yes unless told not.
fn confirm_delete(win: &Rc<Window>, plan: dedupe::Merge, saved: Change) {
    let n = plan.others.len();
    let alert = adw::AlertDialog::builder()
        .heading(if n == 1 {
            "Delete the Duplicate?".to_owned()
        } else {
            format!("Delete the {n} Duplicates?")
        })
        .body(format!(
            "Everything they said is now in {}'s card, in {}. Deleting:\n\n{}\n\nUndo brings them back.",
            name_of(&plan),
            win.state.borrow().books.place(&plan.keep.path),
            plan.others
                .iter()
                .map(|o| win.state.borrow().books.place(&o.path))
                .collect::<Vec<_>>()
                .join("\n"),
        ))
        .default_response("delete")
        .close_response("keep")
        .build();
    alert.add_response("keep", "Keep Them");
    alert.add_response("delete", "Delete");
    alert.set_response_appearance("delete", adw::ResponseAppearance::Destructive);
    let w = win.clone();
    alert.connect_response(None, move |_, response| {
        finish(&w, &plan, vec![saved.clone()], response == "delete");
    });
    alert.present(Some(&win.window));
}

/// Delete the others (if `delete`), record it all as one change to undo,
/// and show the merged card.
fn finish(win: &Rc<Window>, plan: &dedupe::Merge, mut changes: Vec<Change>, delete: bool) {
    let mut failed = Vec::new();
    let mut deleted = 0;
    if delete {
        for id in &plan.others {
            match win.state.borrow_mut().books.delete(id) {
                Ok(c) => {
                    changes.push(c);
                    deleted += 1;
                }
                Err(e) => failed.push(format!("{e:#}")),
            }
        }
    }
    win.state.borrow_mut().record(changes);
    // the next duplicates take their place, on the same row
    win.state.borrow_mut().selected = Some(plan.keep.clone());
    win.after_change(None);
    let name = name_of(plan);
    if let Some(e) = failed.first() {
        win.toast(e);
    } else if deleted > 0 {
        win.undo_toast(&format!("Merged into {name}"));
    } else {
        win.undo_toast(&format!("Merged into {name}, keeping the others"));
    }
}

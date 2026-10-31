//! The window: folders, the messages in one, and the one being read.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use adw::prelude::*;
use chrono::{Datelike as _, Local, TimeZone as _};
use gtk::{gdk, gio, glib, pango};
use rical_maildir::{read, store::Role};
use rical_ui::{
    binds::{Binds, Lookup},
    keys::{is_navigation, key_of},
    palette,
};

use crate::{
    reader::Reader,
    state::{Place, SharedState, unified_title},
};

pub struct Window {
    pub window: adw::ApplicationWindow,
    pub app: adw::Application,
    pub state: SharedState,
    pub binds: Binds,
    title: adw::WindowTitle,
    pub outer: adw::OverlaySplitView,
    pub split: adw::NavigationSplitView,
    folder_list: gtk::ListBox,
    pub search: gtk::SearchEntry,
    model: gtk::StringList,
    /// The key of each row in `model`, as last listed.
    row_keys: RefCell<Vec<String>>,
    /// The last toast [`Window::toast`] showed.
    last_toast: RefCell<Option<adw::Toast>>,
    /// The rows of `folder_list`, as last drawn.
    folder_specs: RefCell<Vec<FolderRow>>,
    /// The latest search inside the messages: an older one stops.
    body_search: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pub selection: gtk::SingleSelection,
    pub list: gtk::ListView,
    pub reader: Rc<Reader>,
    reader_title: adw::WindowTitle,
    list_stack: gtk::Stack,
    pub toasts: adw::ToastOverlay,
    /// The mail view and the messages being written, as tabs.
    pub tabs: adw::TabView,
    pub composes: RefCell<Vec<Rc<crate::compose::Compose>>>,
    pub history: RefCell<Vec<String>>,
    pending_keys: RefCell<Vec<rical_ui::binds::Key>>,
    /// Whether each fold in the folder list (an account, a folder with
    /// folders in it, everything) is open as shown.
    folds: RefCell<std::collections::HashMap<String, bool>>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    debounce: RefCell<Option<glib::SourceId>>,
    read_timer: RefCell<Option<glib::SourceId>>,
    /// Which opening of a message the reader waits for.
    reader_ticket: Cell<u64>,
    /// Changes the app makes itself raise no reload until this has passed.
    pub quiet_until: Cell<Option<Instant>>,
    pub syncing: Cell<bool>,
    me: RefCell<std::rc::Weak<Window>>,
    /// What loads is shown after the window's first frame.
    first_frame: rical_ui::app::FirstFrame,
}

/// "14:05" today, "5 Nov" this year, "5 Nov 2025" before.
pub fn short_date(ts: i64) -> String {
    let Some(d) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    let now = Local::now();
    if d.date_naive() == now.date_naive() {
        d.format("%H:%M").to_string()
    } else if d.year() == now.year() {
        d.format("%-d %b").to_string()
    } else {
        d.format("%-d %b %Y").to_string()
    }
}

impl Window {
    pub fn new(app: &adw::Application, state: SharedState, binds: Binds) -> Rc<Self> {
        let proportional = state.borrow().opts.proportional;
        let title = adw::WindowTitle::new(crate::APP_NAME, "");

        // folders
        let main_menu = gio::Menu::new();
        let doing = gio::Menu::new();
        doing.append(Some("_Command Line"), Some("win.palette"));

        doing.append(Some("_Reload"), Some("win.run::reload"));
        main_menu.append_section(None, &doing);
        let app_items = gio::Menu::new();
        app_items.append(Some("_Preferences"), Some("win.run::settings"));
        app_items.append(Some("_Keyboard Shortcuts"), Some("win.run::help"));
        app_items.append(Some("_About Mail"), Some("win.run::about"));
        main_menu.append_section(None, &app_items);
        let leaving = gio::Menu::new();
        leaving.append(Some("_Quit"), Some("win.run::quit"));
        main_menu.append_section(None, &leaving);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&main_menu)
            .tooltip_text("Main menu")
            .build();
        let folder_header = adw::HeaderBar::builder()
            .title_widget(&adw::WindowTitle::new("Folders", ""))
            .show_end_title_buttons(false)
            .build();
        folder_header.pack_end(&menu_button);
        let folder_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar"])
            .build();
        folder_list.update_property(&[gtk::accessible::Property::Label("Folders")]);
        let folder_scroller = gtk::ScrolledWindow::builder()
            .child(&folder_list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let folder_view = adw::ToolbarView::new();
        folder_view.add_top_bar(&folder_header);
        folder_view.set_content(Some(&folder_scroller));

        // the messages
        let compose = gtk::Button::builder()
            .icon_name("mail-message-new-symbolic")
            .tooltip_text("Write (c)")
            .action_name("win.run")
            .action_target(&"compose".to_variant())
            .build();
        let fold = gtk::Button::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Folders (F)")
            .action_name("win.run")
            .action_target(&"folders".to_variant())
            .build();
        let list_header = adw::HeaderBar::builder().title_widget(&title).build();
        list_header.pack_start(&fold);
        list_header.pack_start(&compose);
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Filter this folder… (/)")
            .hexpand(true)
            .build();
        let top = gtk::Box::builder()
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        top.append(&search);
        // the other kind: everything, everywhere (the search tool, or every folder)
        let everywhere = gtk::Button::builder()
            .icon_name("system-search-symbolic")
            .tooltip_text("Search all mail for this (Shift+Enter; S builds a search)")
            .css_classes(["flat"])
            .action_name("win.run")
            .action_target(&"search-box".to_variant())
            .build();
        top.set_spacing(4);
        top.append(&everywhere);
        let model = gtk::StringList::new(&[]);
        let selection = gtk::SingleSelection::builder()
            .model(&model)
            .autoselect(false)
            .can_unselect(true)
            .build();
        let factory = gtk::SignalListItemFactory::new();
        let list = gtk::ListView::builder()
            .model(&selection)
            .factory(&factory)
            .css_classes(["navigation-sidebar", "rical-list"])
            .single_click_activate(false)
            .build();
        list.update_property(&[gtk::accessible::Property::Label("Messages")]);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let nothing = adw::StatusPage::builder()
            .icon_name("mail-read-symbolic")
            .title("No mail")
            .description("Nothing here, or nothing matches the search")
            .build();
        let list_stack = gtk::Stack::new();
        list_stack.add_named(&scroller, Some("list"));
        list_stack.add_named(&nothing, Some("none"));
        let loading = adw::StatusPage::builder()
            .title("Reading the folder…")
            .child(
                &adw::Spinner::builder()
                    .width_request(32)
                    .height_request(32)
                    .build(),
            )
            .build();
        list_stack.add_named(&loading, Some("loading"));
        let list_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list_box.append(&top);
        list_box.append(&list_stack);
        let list_view = adw::ToolbarView::new();
        list_view.add_top_bar(&list_header);
        list_view.set_content(Some(&list_box));
        let list_page = adw::NavigationPage::builder()
            .title("Messages")
            .child(&list_view)
            .build();

        // the reader, with what can be done to the message in its header
        let reader_title = adw::WindowTitle::new("", "");
        let reader_header = adw::HeaderBar::builder()
            .title_widget(&reader_title)
            .build();
        for (icon, tip, line, end) in [
            ("mail-reply-sender-symbolic", "Reply (r)", "reply", false),
            (
                "mail-reply-all-symbolic",
                "Reply to all (R)",
                "replyall",
                false,
            ),
            ("mail-forward-symbolic", "Forward (f)", "forward", false),
            ("user-trash-symbolic", "Delete (d)", "delete", true),
            ("folder-symbolic", "Archive (a)", "archive", true),
        ] {
            let b = gtk::Button::builder()
                .icon_name(icon)
                .tooltip_text(tip)
                .action_name("win.run")
                .action_target(&line.to_variant())
                .build();
            if end {
                reader_header.pack_end(&b);
            } else {
                reader_header.pack_start(&b);
            }
        }
        // what else can be done to it: the same menu as a right click
        let more = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .tooltip_text("More for this message")
            .menu_model(&message_menu())
            .build();
        reader_header.pack_end(&more);
        for (icon, tip, line) in [
            ("starred-symbolic", "Flag or unflag (s)", "flag"),
            (
                "mail-unread-symbolic",
                "Mark read or unread (U)",
                "mark toggle",
            ),
        ] {
            reader_header.pack_end(
                &gtk::Button::builder()
                    .icon_name(icon)
                    .tooltip_text(tip)
                    .action_name("win.run")
                    .action_target(&line.to_variant())
                    .build(),
            );
        }
        let reader = Reader::new(proportional);
        let reader_view = adw::ToolbarView::new();
        reader_view.add_top_bar(&reader_header);
        reader_view.set_content(Some(&reader.widget));
        let reader_page = adw::NavigationPage::builder()
            .title("Message")
            .child(&reader_view)
            .build();

        let split = adw::NavigationSplitView::builder()
            .sidebar(&list_page)
            .content(&reader_page)
            .min_sidebar_width(300.0)
            .max_sidebar_width(460.0)
            .sidebar_width_fraction(0.34)
            .build();
        let outer = adw::OverlaySplitView::builder()
            .sidebar(&folder_view)
            .content(&split)
            .min_sidebar_width(180.0)
            .max_sidebar_width(260.0)
            .show_sidebar(true)
            .build();
        let toasts = adw::ToastOverlay::new();
        // the mail view is the first tab; each message being written gets its own
        let tabs = adw::TabView::new();
        let mail_page = tabs.append_pinned(&outer);
        mail_page.set_title("Mail");
        mail_page.set_icon(Some(&gio::ThemedIcon::new("mail-unread-symbolic")));
        let tab_bar = adw::TabBar::builder().view(&tabs).autohide(true).build();
        let tabbed = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tabbed.append(&tab_bar);
        tabs.set_vexpand(true);
        tabbed.append(&tabs);
        toasts.set_child(Some(&tabbed));
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(crate::APP_NAME)
            .default_width(1280)
            .default_height(780)
            .content(&toasts)
            .build();
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            760.0,
            adw::LengthUnit::Sp,
        ));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&outer, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(narrow);

        let first_frame = rical_ui::app::FirstFrame::watch(&window);
        let this = Rc::new(Self {
            window,
            app: app.clone(),
            state,
            binds,
            title,
            outer,
            split,
            folder_list,
            search,
            model,
            selection,
            list,
            reader,
            reader_title,
            list_stack,
            toasts,
            tabs,
            composes: RefCell::default(),
            history: Default::default(),
            pending_keys: Default::default(),
            monitors: Default::default(),
            folds: Default::default(),
            debounce: Default::default(),
            read_timer: Default::default(),
            reader_ticket: Cell::new(0),
            row_keys: RefCell::default(),
            last_toast: RefCell::default(),
            folder_specs: RefCell::default(),
            body_search: Default::default(),
            quiet_until: Cell::new(None),
            syncing: Cell::new(false),
            me: Default::default(),
            first_frame,
        });
        *this.me.borrow_mut() = Rc::downgrade(&this);
        let weak = Rc::downgrade(&this);
        *this.reader.on_url.borrow_mut() = Some(Rc::new(move |url| {
            if let Some(w) = weak.upgrade()
                && let Err(e) = w.confirm_url(url)
            {
                w.toast(&e);
            }
        }));
        let weak = Rc::downgrade(&this);
        *this.reader.on_link.borrow_mut() = Some(Rc::new(move |n| {
            if let Some(w) = weak.upgrade()
                && let Err(e) = w.execute(&format!("open-link {n}"))
            {
                w.toast(&e);
            }
        }));
        this.setup_rows(&factory);
        this.setup_signals();
        this.setup_tabs();
        this.setup_actions();
        this.watch();
        this.refresh();
        this.open_place();
        // the counts from the index, counted again away from the window
        this.reload_folders(|w| w.fill_folders());
        this.load_others();
        this.focus_view();
        this
    }

    /// Read the contacts (to complete addresses) on the background pool,
    /// from their index (brought up to date there).
    fn load_others(self: &Rc<Self>) {
        let Some(contacts) = self.state.borrow().opts.contacts.clone() else {
            return;
        };
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let read = gio::spawn_blocking(move || {
                rical_cards::Books::open(&contacts)
                    .map_err(|e| log::warn!("contacts {}: {e:#}", contacts.display()))
                    .ok()
            })
            .await;
            let (Some(w), Ok(books)) = (weak.upgrade(), read) else {
                return;
            };
            {
                let mut s = w.state.borrow_mut();
                s.books = books;
                s.build_people();
            }
            // whether the sender is a contact
            w.show_message();
        });
    }

    pub fn present(&self) {
        self.window.present();
    }

    /// Say something, in place of what was said last: a high-priority
    /// toast would otherwise put the last one back to be shown after it
    /// ("Deleted" again after "Moved back"). One with its own button (Undo
    /// sending) is not made here, so stays.
    pub fn toast(&self, text: &str) {
        let toast = adw::Toast::builder()
            .title(text)
            .priority(adw::ToastPriority::High)
            .build();
        if let Some(last) = self.last_toast.replace(Some(toast.clone())) {
            last.dismiss();
        }
        self.toasts.add_toast(toast);
    }

    /// Say that what the app does next to the folders is its own doing.
    pub fn quiet(&self) {
        self.quiet_until
            .set(Some(Instant::now() + Duration::from_millis(1500)));
    }

    /// Each row of the list: who, when, the subject, and what is on it.
    fn setup_rows(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        let weak = Rc::downgrade(self);
        let menu = message_menu();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let outer = gtk::Box::builder()
                .spacing(8)
                .margin_top(5)
                .margin_bottom(5)
                .build();
            let dot = gtk::Image::builder()
                .icon_name("media-record-symbolic")
                .pixel_size(10)
                .valign(gtk::Align::Center)
                .css_classes(["accent"])
                .build();
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            text.set_hexpand(true);
            let first = gtk::Box::builder().spacing(6).build();
            let from = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(pango::EllipsizeMode::End)
                .build();
            let when = gtk::Label::builder()
                .css_classes(["dim-label", "caption"])
                .build();
            first.append(&from);
            first.append(&when);
            let second = gtk::Box::builder().spacing(6).build();
            let subject = gtk::Label::builder()
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(pango::EllipsizeMode::End)
                .css_classes(["caption"])
                .build();
            let clip = gtk::Image::builder()
                .icon_name("mail-attachment-symbolic")
                .pixel_size(12)
                .css_classes(["dim-label"])
                .build();
            let flag = gtk::Image::builder()
                .icon_name("starred-symbolic")
                .pixel_size(12)
                .css_classes(["warning"])
                .build();
            second.append(&subject);
            second.append(&clip);
            second.append(&flag);
            let note = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .css_classes(["dim-label", "caption"])
                .build();
            text.append(&first);
            text.append(&second);
            text.append(&note);
            // the start of what it says, dim, one line
            let preview = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(pango::EllipsizeMode::End)
                .single_line_mode(true)
                .css_classes(["dim-label", "caption"])
                .build();
            text.append(&preview);
            outer.append(&dot);
            outer.append(&text);
            item.set_child(Some(&outer));
            // a right click: that message, and what can be done to it
            let click = gtk::GestureClick::builder().button(3).build();
            let (w, it, m) = (weak.clone(), item.clone(), menu.clone());
            click.connect_pressed(move |g, _, x, y| {
                let Some(w) = w.upgrade() else { return };
                w.select_position(it.position() as usize);
                let Some(widget) = g.widget() else { return };
                let pop = gtk::PopoverMenu::from_model(Some(&m));
                pop.set_parent(&widget);
                pop.set_has_arrow(false);
                pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
                pop.connect_closed(|p| {
                    let p = p.clone();
                    glib::idle_add_local_once(move || p.unparent());
                });
                pop.popup();
            });
            outer.add_controller(click);
            // dragged onto a folder, it moves there (with what is picked)
            let drag = gtk::DragSource::builder()
                .actions(gdk::DragAction::MOVE)
                .build();
            let (w, it) = (weak.clone(), item.clone());
            drag.connect_prepare(move |_, _, _| {
                let w = w.upgrade()?;
                let at = it.position() as usize;
                let picked = w
                    .state
                    .borrow()
                    .message_at(at)
                    .is_some_and(|m| w.state.borrow().picked.contains(&m.path));
                if !picked {
                    w.select_position(at);
                }
                Some(gdk::ContentProvider::for_value(&DRAG_MESSAGES.to_value()))
            });
            outer.add_controller(drag);
        });
        let state = self.state.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            // the item is only the row's key (see Window::fill_list): what
            // it shows is the message at its place in the list
            let n = item.position() as usize;
            let Some(outer) = item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let s = state.borrow();
            let Some(m) = s.message_at(n) else { return };
            let depth = s.depth_at(n);
            let dot = outer.first_child().unwrap();
            let text = dot.next_sibling().unwrap();
            let first = text.first_child().unwrap();
            let from = first.first_child().and_downcast::<gtk::Label>().unwrap();
            let when = from.next_sibling().and_downcast::<gtk::Label>().unwrap();
            let second = first.next_sibling().unwrap();
            let subject = second.first_child().and_downcast::<gtk::Label>().unwrap();
            let clip = subject.next_sibling().unwrap();
            let flag = clip.next_sibling().unwrap();
            let note = second.next_sibling().and_downcast::<gtk::Label>().unwrap();
            let preview = note.next_sibling().and_downcast::<gtk::Label>().unwrap();
            let outbox = s.place == Place::Outbox;
            // a folded conversation is unread if any of it is
            let unread = (m.unread()
                || s.folded.contains(&m.path)
                    && s.thread_info.get(&m.path).is_some_and(|(_, u)| *u > 0))
                && !outbox;
            let picked = s.picked.contains(&m.path);
            if let Some(img) = dot.downcast_ref::<gtk::Image>() {
                img.set_icon_name(Some(if picked {
                    "object-select-symbolic"
                } else {
                    "media-record-symbolic"
                }));
                img.set_pixel_size(if picked { 14 } else { 10 });
            }
            dot.set_opacity(if unread || picked { 1.0 } else { 0.0 });
            outer.set_margin_start(depth.min(4) as i32 * 14);
            let who = if outbox
                || (s
                    .folder()
                    .is_some_and(|f| matches!(f.role, Role::Sent | Role::Drafts))
                    || matches!(s.place, Place::Unified(Role::Sent | Role::Drafts)))
            {
                if m.to.is_empty() {
                    "(no recipient)".to_owned()
                } else {
                    format!("To: {}", m.to)
                }
            } else if m.from_shown().is_empty() {
                "(unknown sender)".to_owned()
            } else {
                m.from_shown().to_owned()
            };
            // a folded conversation says how many it holds
            let who = match s.thread_info.get(&m.path) {
                Some((n, _)) if s.folded.contains(&m.path) => format!("{who}  ({n})"),
                _ => who,
            };
            from.set_label(&who);
            if unread {
                from.add_css_class("heading");
                subject.add_css_class("heading");
            } else {
                from.remove_css_class("heading");
                subject.remove_css_class("heading");
            }
            when.set_label(&short_date(m.date));
            subject.set_label(if m.subject.is_empty() {
                "(no subject)"
            } else {
                &m.subject
            });
            clip.set_visible(m.attachments);
            flag.set_visible(m.flags.flagged);
            // the Outbox says why each waits; elsewhere where it is, by the date
            let extra = s.outbox_notes.get(&m.path).cloned();
            if outbox {
                note.set_visible(extra.is_some());
                note.set_label(&extra.unwrap_or_default());
                preview.set_visible(false);
            } else {
                if let Some(e) = extra {
                    when.set_label(&format!(
                        "{} · {}",
                        e.trim_start_matches("in "),
                        short_date(m.date)
                    ));
                }
                note.set_visible(false);
                preview.set_label(&m.preview);
                preview.set_visible(!m.preview.is_empty());
            }
        });
    }

    fn setup_signals(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            let Some(w) = weak.upgrade() else { return };
            if w.syncing.get() {
                return;
            }
            w.search_now();
        });
        // Shift+Enter in the box searches everywhere
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && mods.contains(gdk::ModifierType::SHIFT_MASK)
                && let Some(w) = weak.upgrade()
            {
                if let Err(e) = w.execute("search-box") {
                    w.toast(&e);
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.search.add_controller(keys);
        let weak = Rc::downgrade(self);
        self.search.connect_activate(move |_| {
            if let Some(w) = weak.upgrade() {
                w.search_now();
                w.select_position(0);
                w.list.grab_focus();
            }
        });
        let weak = Rc::downgrade(self);
        self.search.connect_stop_search(move |_| {
            if let Some(w) = weak.upgrade() {
                let _ = w.execute("back");
            }
        });
        let weak = Rc::downgrade(self);
        self.selection.connect_selected_item_notify(move |sel| {
            let Some(w) = weak.upgrade() else { return };
            if w.syncing.get() {
                return;
            }
            let at = sel.selected();
            let path = (at != gtk::INVALID_LIST_POSITION)
                .then(|| {
                    w.state
                        .borrow()
                        .message_at(at as usize)
                        .map(|m| m.path.clone())
                })
                .flatten();
            w.state.borrow_mut().selected = path;
            w.show_message();
        });
        let weak = Rc::downgrade(self);
        self.list.connect_activate(move |_, _| {
            if let Some(w) = weak.upgrade() {
                let _ = w.execute("open");
            }
        });
        let weak = Rc::downgrade(self);
        self.folder_list.connect_row_activated(move |_, row| {
            let Some(w) = weak.upgrade() else { return };
            let name = row.widget_name();
            if name == "outbox" {
                let _ = w.execute("outbox");
            } else if let Some(key) = name.strip_prefix("fold:") {
                let _ = w.execute(&format!("fold-folder {}", rical_ui::command::quote(key)));
            } else if let Some(role) = name.strip_prefix("unified:") {
                let _ = w.execute(&format!("unified {role}"));
            } else if let Some(f) = w
                .state
                .borrow()
                .folders
                .iter()
                .find(|f| f.path.to_string_lossy() == name.as_str())
                .map(|f| f.name.clone())
            {
                let _ = w.execute(&format!("folder {}", rical_ui::command::quote(&f)));
            }
        });
        let weak = Rc::downgrade(self);
        self.model.connect_items_changed(move |m, _, _, _| {
            if let Some(w) = weak.upgrade() {
                w.list_stack
                    .set_visible_child_name(if m.n_items() == 0 { "none" } else { "list" });
            }
        });
    }

    fn setup_actions(self: &Rc<Self>) {
        let run = gio::SimpleAction::new("run", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        run.connect_activate(move |_, param| {
            let Some(w) = weak.upgrade() else { return };
            if let Some(line) = param.and_then(|p| p.str())
                && let Err(e) = w.execute(line)
            {
                w.toast(&e);
            }
        });
        self.window.add_action(&run);
        let open = gio::SimpleAction::new("palette", None);
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, _| {
            if let Some(w) = weak.upgrade() {
                palette::open(&w, "");
            }
        });
        self.window.add_action(&open);
        // the command line with something typed in, from a menu: "Move
        // To…" asks for the folder there, with completion, rather than
        // listing hundreds of them in a menu (which GTK keeps up to date
        // for every widget added, at seconds a start)
        let prompt = gio::SimpleAction::new("prompt", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        prompt.connect_activate(move |_, param| {
            if let (Some(w), Some(text)) = (weak.upgrade(), param.and_then(|p| p.str())) {
                palette::open(&w, text);
            }
        });
        self.window.add_action(&prompt);

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| match weak.upgrade() {
            Some(w) => w.handle_key(key, mods),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);
    }

    /// Key presses go to the bindings, except while typing or in a dialog.
    fn handle_key(self: &Rc<Self>, keyval: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use glib::Propagation::{Proceed, Stop};
        // a message being written has its own keys
        if let Some(c) = self.current_compose() {
            return c.handle_key(keyval, mods);
        }
        if self.window.visible_dialog().is_some() {
            return Proceed;
        }
        let focus = gtk::prelude::GtkWindowExt::focus(&self.window);
        let typing = focus.as_ref().is_some_and(|f| {
            f.is::<gtk::Text>()
                || f.is::<gtk::Editable>()
                || f.is::<gtk::TextView>()
                    && f.is_sensitive()
                    && f.downcast_ref::<gtk::TextView>()
                        .is_some_and(|t| t.is_editable())
        });
        let Some(key) = key_of(keyval, mods) else {
            return Proceed;
        };
        if typing {
            let in_search = focus.as_ref().is_some_and(|f| f.is_ancestor(&self.search));
            if in_search && key.name == "down" && !key.ctrl {
                self.search_now();
                self.focus_view();
                return Stop;
            }
            return Proceed;
        }
        // in the message, arrows scroll it
        let in_reader = focus
            .as_ref()
            .is_some_and(|f| f.is_ancestor(&self.reader.widget));
        if in_reader && is_navigation(&key) && key.name != "esc" && key.name != "enter" {
            self.pending_keys.borrow_mut().clear();
            return Proceed;
        }
        let mut pending = self.pending_keys.borrow_mut();
        pending.push(key.clone());
        let mut found = self.binds.lookup("list", &pending);
        if matches!(found, Lookup::None) && pending.len() > 1 {
            pending.clear();
            pending.push(key);
            found = self.binds.lookup("list", &pending);
        }
        match found {
            Lookup::Found(action) => {
                pending.clear();
                drop(pending);
                for line in &action.run {
                    if let Err(e) = self.execute(line) {
                        self.toast(&e);
                        return Stop;
                    }
                }
                if let Some(text) = &action.prefill {
                    palette::open(self, text);
                }
                Stop
            }
            Lookup::Prefix => Stop,
            Lookup::None => {
                pending.clear();
                Proceed
            }
        }
    }

    pub fn search_now(self: &Rc<Self>) {
        let text = self.search.text().to_string();
        if self.state.borrow().query == text {
            return;
        }
        {
            let mut s = self.state.borrow_mut();
            s.query = text;
            s.apply_query();
        }
        self.fill_list();
        self.show_message();
        self.search_body();
    }

    /// Everything shown, from the state.
    pub fn refresh(self: &Rc<Self>) {
        self.fill_folders();
        self.fill_list();
        self.show_message();
    }

    /// The folder list. What each row shows is worked out first; when the
    /// rows are the same ones as before (only counts changed, as after a
    /// move), just those that differ are made again, not hundreds.
    pub fn fill_folders(&self) {
        let (rows, select) = self.folder_rows();
        let same_rows = {
            let old = self.folder_specs.borrow();
            old.len() == rows.len() && old.iter().zip(&rows).all(|(a, b)| a.same_row(b))
        };
        if same_rows {
            let old = self.folder_specs.borrow();
            for (i, (was, now)) in old.iter().zip(&rows).enumerate() {
                if was != now
                    && let Some(row) = self.folder_list.row_at_index(i as i32)
                {
                    let selected = row.is_selected();
                    self.folder_list.remove(&row);
                    let new = self.folder_row_of(now);
                    self.folder_list.insert(&new, i as i32);
                    if selected {
                        self.syncing.set(true);
                        self.folder_list.select_row(Some(&new));
                        self.syncing.set(false);
                    }
                }
            }
        } else {
            while let Some(c) = self.folder_list.first_child() {
                self.folder_list.remove(&c);
            }
            for spec in &rows {
                self.folder_list.append(&self.folder_row_of(spec));
            }
        }
        *self.folder_specs.borrow_mut() = rows;
        let row = select.and_then(|i| self.folder_list.row_at_index(i as i32));
        if row.as_ref() != self.folder_list.selected_row().as_ref() {
            self.syncing.set(true);
            self.folder_list.select_row(row.as_ref());
            self.syncing.set(false);
        }
    }

    /// The rows of the folder list, and which is where you are.
    fn folder_rows(&self) -> (Vec<FolderRow>, Option<usize>) {
        let s = self.state.borrow();
        let mut rows = Vec::new();
        let mut select = None;
        let remembered = rical_ui::uistate::get("folds").unwrap_or_default();
        self.folds.borrow_mut().clear();
        if s.grouped() {
            // everything together, then each account
            let open = remembered_fold(&remembered, ALL_FOLD).unwrap_or(true);
            self.folds.borrow_mut().insert(ALL_FOLD.to_owned(), open);
            rows.push(FolderRow::Fold {
                id: format!("fold:{ALL_FOLD}"),
                name: "All accounts".to_owned(),
                unread: 0,
                open,
                indent: 0,
                strong: true,
            });
            for role in [
                Role::Inbox,
                Role::Drafts,
                Role::Sent,
                Role::Archive,
                Role::Junk,
                Role::Trash,
            ] {
                if !open {
                    break;
                }
                let of = s.with_role(role);
                if of.is_empty() || (role != Role::Inbox && of.len() < 2) {
                    continue;
                }
                let unread = if role == Role::Trash || role == Role::Junk {
                    0
                } else {
                    of.iter().map(|f| f.unread).sum()
                };
                if s.place == Place::Unified(role) {
                    select = Some(rows.len());
                }
                rows.push(FolderRow::Folder {
                    id: format!("unified:{}", unified_key(role)),
                    icon: folder_icon(role),
                    name: unified_title(role).to_owned(),
                    unread,
                    indent: 1,
                    toggle: None,
                    drop: None,
                });
            }
            for a in s.accounts() {
                let entries: Vec<(String, usize)> = a
                    .folders
                    .iter()
                    .map(|&i| (s.folders[i].name[a.name.len() + 1..].to_owned(), i))
                    .collect();
                let unread = counted_unread(&s, entries.iter().map(|e| e.1));
                // open when asked to be, else when it has something unread
                // or is where you are
                let here = entries
                    .iter()
                    .any(|e| matches!(&s.place, Place::Folder(p) if *p == s.folders[e.1].path));
                let open = remembered_fold(&remembered, &a.name).unwrap_or(unread > 0 || here);
                self.folds.borrow_mut().insert(a.name.clone(), open);
                rows.push(FolderRow::Fold {
                    id: format!("fold:{}", a.name),
                    name: a.name.clone(),
                    unread,
                    open,
                    indent: 0,
                    strong: true,
                });
                if open {
                    let entries: Vec<(Vec<&str>, usize)> = entries
                        .iter()
                        .map(|(n, i)| (n.split('/').collect(), *i))
                        .collect();
                    let prefix = format!("{}/", a.name);
                    self.tree_rows(
                        &s,
                        &entries,
                        0,
                        1,
                        &prefix,
                        &remembered,
                        &mut rows,
                        &mut select,
                    );
                }
            }
            rows.push(FolderRow::Heading("Outgoing".to_owned()));
        } else {
            let entries: Vec<(Vec<&str>, usize)> = s
                .folders
                .iter()
                .enumerate()
                .map(|(i, f)| (f.name.split('/').collect(), i))
                .collect();
            self.tree_rows(&s, &entries, 0, 0, "", &remembered, &mut rows, &mut select);
        }
        if s.place == Place::Outbox {
            select = Some(rows.len());
        }
        rows.push(FolderRow::Folder {
            id: "outbox".to_owned(),
            icon: "network-transmit-symbolic",
            name: "Outbox".to_owned(),
            unread: s.outbox_count(),
            indent: 0,
            toggle: None,
            drop: None,
        });
        (rows, select)
    }

    /// Folders as a tree by the parts of their names: a folder with folders
    /// in it (`[Gmail]`, `Projects`) folds them away. `entries` are the names
    /// split at `/` with the folder's index, in the order to show them.
    #[allow(clippy::too_many_arguments)]
    fn tree_rows(
        &self,
        s: &crate::state::State,
        entries: &[(Vec<&str>, usize)],
        depth: usize,
        indent: i32,
        prefix: &str,
        remembered: &str,
        rows: &mut Vec<FolderRow>,
        select: &mut Option<usize>,
    ) {
        // the names at this level in order, each with the folder of that
        // name and those inside it; those with nothing inside first
        let mut at: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        let mut names: Vec<(&str, Option<usize>, Entries)> = Vec::new();
        for (parts, i) in entries {
            let n = *at.entry(parts[depth]).or_insert_with(|| {
                names.push((parts[depth], None, Vec::new()));
                names.len() - 1
            });
            if parts.len() == depth + 1 {
                names[n].1.get_or_insert(*i);
            } else {
                names[n].2.push((parts.clone(), *i));
            }
        }
        names.sort_by_key(|(_, _, kids)| !kids.is_empty());
        for (name, own, kids) in names {
            let key = format!("{prefix}{name}");
            let mut fold = None;
            if !kids.is_empty() {
                let here = kids
                    .iter()
                    .any(|e| matches!(&s.place, Place::Folder(p) if *p == s.folders[e.1].path));
                let unread = counted_unread(s, kids.iter().map(|e| e.1));
                let open = remembered_fold(remembered, &key).unwrap_or(unread > 0 || here);
                self.folds.borrow_mut().insert(key.clone(), open);
                fold = Some((key.clone(), open, unread));
            }
            match (own, &fold) {
                (Some(i), _) => {
                    let f = &s.folders[i];
                    if matches!(&s.place, Place::Folder(p) if *p == f.path) {
                        *select = Some(rows.len());
                    }
                    rows.push(FolderRow::Folder {
                        id: f.path.to_string_lossy().into_owned(),
                        icon: folder_icon(f.role),
                        name: name.to_owned(),
                        unread: f.unread,
                        indent,
                        toggle: fold.as_ref().map(|(k, open, _)| (k.clone(), *open)),
                        drop: Some(f.name.clone()),
                    });
                }
                (None, Some((k, open, unread))) => rows.push(FolderRow::Fold {
                    id: format!("fold:{k}"),
                    name: name.to_owned(),
                    unread: *unread,
                    open: *open,
                    indent,
                    strong: false,
                }),
                (None, None) => continue,
            }
            if let Some((_, true, _)) = fold {
                self.tree_rows(
                    s,
                    &kids,
                    depth + 1,
                    indent + 1,
                    &format!("{key}/"),
                    remembered,
                    rows,
                    select,
                );
            }
        }
    }

    /// The widget for a row of the folder list.
    fn folder_row_of(&self, spec: &FolderRow) -> gtk::ListBoxRow {
        let row = match spec {
            FolderRow::Heading(text) => return heading_row(text),
            FolderRow::Fold {
                name,
                unread,
                open,
                indent,
                strong,
                ..
            } => fold_row(name, *unread, *open, *indent, *strong),
            FolderRow::Folder {
                icon,
                name,
                unread,
                indent,
                toggle,
                drop,
                ..
            } => {
                let row = folder_row(
                    icon,
                    name,
                    *unread,
                    *indent,
                    toggle.as_ref().map(|(k, open)| (k.as_str(), *open)),
                );
                if let Some(folder) = drop {
                    self.droppable(&row, folder);
                }
                row
            }
        };
        if let FolderRow::Fold { id, .. } | FolderRow::Folder { id, .. } = spec {
            row.set_widget_name(id);
        }
        row
    }

    /// Fold or open a part of the folder list (an account, a folder with
    /// folders in it, or `*` for everything together), and remember which.
    pub fn toggle_fold(&self, key: &str) {
        let open_now = self.folds.borrow().get(key).copied().unwrap_or(false);
        let remembered = rical_ui::uistate::get("folds").unwrap_or_default();
        let mut entries: Vec<String> = remembered
            .split('|')
            .filter(|e| !e.is_empty() && e[..e.len() - 1] != *key)
            .map(str::to_owned)
            .collect();
        entries.push(format!("{key}{}", if open_now { '-' } else { '+' }));
        rical_ui::uistate::set("folds", &entries.join("|"));
        self.fill_folders();
    }

    /// Show the list for the search, keeping the selection.
    pub fn fill_list(&self) {
        self.syncing.set(true);
        let (items, selected, name, count, unread): (
            Vec<String>,
            Option<u32>,
            String,
            usize,
            usize,
        ) = {
            let s = self.state.borrow();
            let items = (0..s.shown.len()).map(|n| row_key(&s, n)).collect();
            let selected = s.position().map(|p| p as u32);
            let unread = s
                .shown
                .iter()
                .filter(|&&i| s.messages[s.rows[i].index].unread())
                .count();
            (items, selected, s.place_name(), s.shown.len(), unread)
        };
        // only the rows that differ: deleting one changes one row, not the
        // thousands after it, which keep their place in the list (and GTK
        // their widgets)
        let (at, gone, new) = {
            let old = self.row_keys.borrow();
            let same_start = old.iter().zip(&items).take_while(|(a, b)| a == b).count();
            let most = old.len().min(items.len()) - same_start;
            let same_end = old
                .iter()
                .rev()
                .zip(items.iter().rev())
                .take(most)
                .take_while(|(a, b)| a == b)
                .count();
            (
                same_start,
                old.len() - same_start - same_end,
                same_start..items.len() - same_end,
            )
        };
        if gone > 0 || !new.is_empty() {
            let refs: Vec<&str> = items[new].iter().map(String::as_str).collect();
            self.model.splice(at as u32, gone as u32, &refs);
        }
        self.list_stack
            .set_visible_child_name(if items.is_empty() { "none" } else { "list" });
        *self.row_keys.borrow_mut() = items;
        match selected {
            Some(p) => {
                self.selection.set_selected(p);
                self.list.scroll_to(p, gtk::ListScrollFlags::NONE, None);
            }
            None => self.selection.set_selected(gtk::INVALID_LIST_POSITION),
        }
        self.syncing.set(false);
        self.title.set_title(&name);
        self.update_subtitle(count, unread);
    }

    /// The counts under the folder's name.
    fn update_subtitle(&self, count: usize, unread: usize) {
        let s = self.state.borrow();
        let q = !s.query.is_empty();
        let mut text = format!("{count} message{}", if count == 1 { "" } else { "s" });
        if s.sort != rical_maildir::thread::SortKey::Date || s.sort_reverse {
            text += &format!(
                " · by {}{}",
                s.sort.name(),
                if s.sort_reverse { " (reversed)" } else { "" }
            );
        }
        if unread > 0 {
            text += &format!(" · {unread} unread");
        }
        if q {
            text += " found";
        }
        if !s.picked.is_empty() {
            text += &format!(" · {} picked", s.picked.len());
        }
        self.title.set_subtitle(&text);
    }

    /// The subtitle again, after picking.
    pub fn update_picked_title(&self) {
        let (count, unread) = {
            let s = self.state.borrow();
            (
                s.shown.len(),
                s.shown
                    .iter()
                    .filter(|&&i| s.messages[s.rows[i].index].unread())
                    .count(),
            )
        };
        self.update_subtitle(count, unread);
    }

    /// Patch one row without relisting: its message changed.
    pub fn rebind(&self, position: usize) {
        self.syncing.set(true);
        let keep = self.selection.selected();
        let n = self.model.string(position as u32).map(|s| s.to_string());
        if let Some(n) = n {
            self.model.splice(position as u32, 1, &[n.as_str()]);
        }
        self.selection.set_selected(keep);
        self.syncing.set(false);
    }

    pub fn select_position(&self, position: usize) {
        let n = self.model.n_items() as usize;
        if n == 0 {
            return;
        }
        let p = position.min(n - 1) as u32;
        self.selection.set_selected(p);
        self.list.scroll_to(p, gtk::ListScrollFlags::NONE, None);
    }

    /// Show the selected message in the reader, and count it read once it
    /// has been there a moment.
    pub fn show_message(self: &Rc<Self>) {
        if let Some(id) = self.read_timer.borrow_mut().take() {
            id.remove();
        }
        let (path, note, delay) = {
            let s = self.state.borrow();
            let Some(m) = s.selected_summary() else {
                drop(s);
                self.state.borrow_mut().opened = None;
                self.reader.clear();
                self.reader_title.set_title("");
                return;
            };
            (
                m.path.clone(),
                s.outbox_notes
                    .get(&m.path)
                    .filter(|_| s.place == Place::Outbox)
                    .cloned(),
                s.opts.read_after,
            )
        };
        // a big message is parsed away from the window, and an invitation
        // compared with the calendar there too
        self.state.borrow_mut().opened = None;
        let ticket = self.reader_ticket.get() + 1;
        self.reader_ticket.set(ticket);
        let weak = Rc::downgrade(self);
        let p = path.clone();
        let vdir = self.state.borrow().opts.vdir.clone();
        glib::spawn_future_local(async move {
            let opened = gio::spawn_blocking(move || {
                read::open(&p).map(|o| {
                    let diffs = if o.invites.is_empty() {
                        Vec::new()
                    } else {
                        crate::state::invite_diffs(vdir.as_deref(), &p)
                    };
                    (o, diffs)
                })
            })
            .await;
            let Some(w) = weak.upgrade() else { return };
            // another was chosen meanwhile
            if w.reader_ticket.get() != ticket {
                return;
            }
            match opened {
                Ok(Ok((o, diffs))) => w.shown_opened(o, diffs, note.as_deref(), delay),
                Ok(Err(e)) => {
                    w.reader.clear();
                    w.toast(&format!("{e:#}"));
                }
                Err(_) => w.reader.clear(),
            }
        });
    }

    fn shown_opened(
        self: &Rc<Self>,
        o: read::Opened,
        diffs: Vec<rical_core::diff::Diff>,
        note: Option<&str>,
        delay: Option<u32>,
    ) {
        let in_contacts = o
            .from
            .first()
            .is_some_and(|f| self.state.borrow().is_contact(&f.address));
        self.reader_title.set_title(&o.subject);
        self.reader.show(&o, in_contacts, note, &diffs);
        self.state.borrow_mut().opened = Some(Rc::new(o));
        let unread = self
            .state
            .borrow()
            .selected_summary()
            .is_some_and(|m| m.unread())
            && self.state.borrow().place != Place::Outbox;
        if unread && let Some(secs) = delay {
            let weak = Rc::downgrade(self);
            let id = glib::timeout_add_local_once(Duration::from_secs(secs.into()), move || {
                let Some(w) = weak.upgrade() else { return };
                w.read_timer.borrow_mut().take();
                let _ = w.execute("mark read");
            });
            *self.read_timer.borrow_mut() = Some(id);
        }
    }

    /// Read the messages of the place in front, away from the window, and
    /// show them when they are read. A spinner shows while the list is empty.
    pub fn load(self: &Rc<Self>) {
        self.load_with(false);
    }

    /// Open the place: [`Window::load`], showing what the header index has
    /// first (as the folders were when last listed). Not after a change,
    /// which the index has yet to hear of, so would flicker back.
    pub fn open_place(self: &Rc<Self>) {
        self.load_with(true);
    }

    fn load_with(self: &Rc<Self>, quick: bool) {
        let job = self.state.borrow_mut().load_job();
        if self.state.borrow().messages.is_empty() {
            self.list_stack.set_visible_child_name("loading");
        }
        let weak = Rc::downgrade(self);
        let started = std::time::Instant::now();
        glib::spawn_future_local(async move {
            // the folders as the header index has them, at once, if it
            // has them; then as they are, which shows only what changed
            let quick = job.quick().filter(|_| quick);
            if let Some(quick) = quick {
                let loaded = gio::spawn_blocking(move || quick.run()).await;
                let Some(w) = weak.upgrade() else { return };
                let Ok(l) = loaded else { return };
                w.show_loaded(l, started, false);
            }
            let loaded = gio::spawn_blocking(move || job.run()).await;
            let Some(w) = weak.upgrade() else { return };
            let Ok(l) = loaded else { return };
            w.show_loaded(l, started, true);
        });
    }

    /// Show messages a load read, once the window is shown (a quick read
    /// must not hold it back; what waits for it is shown in order);
    /// `caught_up`, only if they changed.
    fn show_loaded(
        self: &Rc<Self>,
        l: crate::load::Loaded,
        started: std::time::Instant,
        caught_up: bool,
    ) {
        let read = started.elapsed();
        let w = self.clone();
        let first = self.first_frame.clone();
        first.then(move || {
            let applied = if caught_up {
                w.state.borrow_mut().apply_caught_up(l)
            } else {
                w.state.borrow_mut().apply_loaded(l)
            };
            if applied {
                w.fill_list();
                w.search_body();
                w.show_message();
                log::info!(
                    target: "perf",
                    "{} messages read in {read:?}{}, shown in {:?} (ready at {:?})",
                    w.state.borrow().messages.len(),
                    if caught_up { " from the folders" } else { " from the index" },
                    started.elapsed(),
                    rical_ui::app::since_start()
                );
            }
        });
    }

    /// A search inside the messages (`body:`) reads them: away from the
    /// window, narrowing what the headers matched when it is done.
    pub fn search_body(self: &Rc<Self>) {
        let (query, candidates) = {
            let s = self.state.borrow();
            let q = rical_maildir::search::Query::parse(&s.query);
            if !q.needs_body() {
                return;
            }
            let c: Vec<rical_maildir::store::Summary> = (0..s.shown.len())
                .filter_map(|n| s.message_at(n).cloned())
                .collect();
            (s.query.clone(), c)
        };
        self.title.set_subtitle("Searching inside the messages…");
        // each message is read whole: typing `body:budget` must not start a
        // search of the whole folder for each letter, nor leave the earlier
        // ones running, so it waits for a pause and a newer one stops it
        let ticket = self.body_search.fetch_add(1, Ordering::Relaxed) + 1;
        let latest = self.body_search.clone();
        let weak = Rc::downgrade(self);
        let q = query.clone();
        glib::spawn_future_local(async move {
            glib::timeout_future(Duration::from_millis(250)).await;
            if latest.load(Ordering::Relaxed) != ticket {
                return;
            }
            let still = latest.clone();
            let found = gio::spawn_blocking(move || {
                rical_maildir::search::Query::parse(&q)
                    .body_matches(&candidates, || still.load(Ordering::Relaxed) == ticket)
            })
            .await;
            let Some(w) = weak.upgrade() else { return };
            let Ok(Some(found)) = found else { return };
            let found: std::collections::HashSet<_> = found.into_iter().collect();
            if w.state.borrow().query != query || latest.load(Ordering::Relaxed) != ticket {
                return;
            }
            w.state.borrow_mut().restrict_shown(&found);
            w.fill_list();
            w.show_message();
        });
    }

    pub fn focus_view(&self) {
        self.list.grab_focus();
        let at = self.selection.selected();
        if at != gtk::INVALID_LIST_POSITION {
            let list = self.list.clone();
            glib::idle_add_local_once(move || {
                list.scroll_to(at, gtk::ListScrollFlags::FOCUS, None);
            });
        }
    }

    /// Watch the folders, so mail that arrives (mbsync, another client)
    /// shows up.
    fn watch(self: &Rc<Self>) {
        let mut monitors = Vec::new();
        let dirs = {
            let s = self.state.borrow();
            let mut dirs: Vec<std::path::PathBuf> = Vec::new();
            for f in &s.folders {
                dirs.push(f.path.join("new"));
                dirs.push(f.path.join("cur"));
            }
            dirs.push(s.opts.queue.join("new"));
            dirs.push(s.opts.queue.join("failed"));
            dirs
        };
        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }
            let monitor = match gio::File::for_path(&dir).monitor_directory(
                gio::FileMonitorFlags::WATCH_MOVES,
                None::<&gio::Cancellable>,
            ) {
                Ok(m) => m,
                Err(e) => {
                    log::warn!("cannot watch {dir:?}: {e}");
                    continue;
                }
            };
            let weak = Rc::downgrade(self);
            monitor.connect_changed(move |_, _, _, event| {
                if matches!(
                    event,
                    gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::AttributeChanged
                ) {
                    return;
                }
                if let Some(w) = weak.upgrade() {
                    w.catch_up_soon();
                }
            });
            monitors.push(monitor);
        }
        *self.monitors.borrow_mut() = monitors;
    }

    /// Read again what changed, once changes stop for a moment.
    fn catch_up_soon(self: &Rc<Self>) {
        if let Some(id) = self.debounce.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
            let Some(w) = weak.upgrade() else { return };
            w.debounce.borrow_mut().take();
            if w.quiet_until.get().is_some_and(|t| Instant::now() < t) {
                // our own change: the folder counts, not the whole list
                w.reload_folders(|w| w.fill_folders());
                return;
            }
            let before = w.inbox_unread();
            w.reload_folders(move |w| {
                w.refresh();
                w.load();
                w.maybe_notify(before);
            });
        });
        *self.debounce.borrow_mut() = Some(id);
    }

    /// Read the folders and the current list again, keeping the place.
    pub fn reload(self: &Rc<Self>) {
        self.reload_folders(|w| {
            w.refresh();
            w.load();
        });
    }

    /// Read the folders (and how many messages each has) on the background
    /// pool, then `then`.
    pub fn reload_folders(self: &Rc<Self>, then: impl FnOnce(&Rc<Self>) + 'static) {
        let root = self.state.borrow().root.clone();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            // kept, so the next start shows them at once
            let found = gio::spawn_blocking(move || match crate::load::cache_dir() {
                Some(dir) => rical_maildir::store::folders_kept(&root, &dir),
                None => rical_maildir::store::folders(&root),
            })
            .await;
            let Some(w) = weak.upgrade() else { return };
            match found {
                Ok(Ok(folders)) => w.state.borrow_mut().folders = folders,
                Ok(Err(e)) => log::warn!("{e:#}"),
                Err(_) => {}
            }
            then(&w);
        });
    }
}

/// What row `n` of the list shows, as text: its message (whose file name
/// changes with its flags) and how it is drawn there. A row whose key is
/// the same is not drawn again.
fn row_key(s: &crate::state::State, n: usize) -> String {
    let Some(m) = s.message_at(n) else {
        return String::new();
    };
    let folded = s
        .folded
        .contains(&m.path)
        .then(|| s.thread_info.get(&m.path).copied())
        .flatten();
    format!(
        "{}|{}|{folded:?}|{}|{:?}",
        m.path.display(),
        s.depth_at(n),
        s.picked.contains(&m.path),
        s.outbox_notes.get(&m.path),
    )
}

/// Folder names split at `/`, each with the folder's index.
type Entries<'a> = Vec<(Vec<&'a str>, usize)>;

/// A row of the folder list, before it is drawn.
#[derive(Clone, PartialEq)]
enum FolderRow {
    /// A name over folders (an account, `[Gmail]`): `id` is `fold:` and
    /// its key.
    Fold {
        id: String,
        name: String,
        unread: usize,
        open: bool,
        indent: i32,
        strong: bool,
    },
    /// A folder, a folder of every account, or the Outbox. `toggle` folds
    /// those inside it; `drop` is the folder messages dragged on move to.
    Folder {
        id: String,
        icon: &'static str,
        name: String,
        unread: usize,
        indent: i32,
        toggle: Option<(String, bool)>,
        drop: Option<String>,
    },
    Heading(String),
}

impl FolderRow {
    /// Whether `other` is this row, its count aside.
    fn same_row(&self, other: &FolderRow) -> bool {
        let without_count = |r: &FolderRow| {
            let mut r = r.clone();
            if let FolderRow::Fold { unread, .. } | FolderRow::Folder { unread, .. } = &mut r {
                *unread = 0;
            }
            r
        };
        without_count(self) == without_count(other)
    }
}

/// The fold key for "All accounts".
const ALL_FOLD: &str = "*";

/// Whether a fold was left open (`key+`) or shut (`key-`).
fn remembered_fold(remembered: &str, key: &str) -> Option<bool> {
    remembered
        .split('|')
        .find_map(|e| match e.strip_suffix('+') {
            Some(n) if n == key => Some(true),
            _ => e.strip_suffix('-').filter(|n| *n == key).map(|_| false),
        })
}

/// The unread worth counting in some folders: not junk or trash.
fn counted_unread(s: &crate::state::State, folders: impl Iterator<Item = usize>) -> usize {
    folders
        .filter(|&i| !matches!(s.folders[i].role, Role::Junk | Role::Trash))
        .map(|i| s.folders[i].unread)
        .sum()
}

fn folder_icon(role: Role) -> &'static str {
    match role {
        Role::Inbox => "mail-unread-symbolic",
        Role::Drafts => "document-edit-symbolic",
        Role::Sent => "mail-send-symbolic",
        Role::Archive => "folder-symbolic",
        Role::Junk => "mail-mark-junk-symbolic",
        Role::Trash => "user-trash-symbolic",
        Role::Other => "folder-symbolic",
    }
}

fn fold_icon(open: bool) -> &'static str {
    if open {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    }
}

/// A folder in the list. `fold` is the key and state of the folders in it,
/// when it has some: a button folds or opens them.
fn folder_row(
    icon: &str,
    name: &str,
    unread: usize,
    indent: i32,
    fold: Option<(&str, bool)>,
) -> gtk::ListBoxRow {
    let b = gtk::Box::builder()
        .spacing(10)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(4 + indent * 18)
        .margin_end(4)
        .build();
    b.append(&gtk::Image::from_icon_name(icon));
    let label = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    if unread > 0 {
        label.add_css_class("heading");
    }
    b.append(&label);
    if unread > 0 {
        b.append(
            &gtk::Label::builder()
                .label(unread.to_string())
                .css_classes(["dim-label", "caption", "numeric"])
                .build(),
        );
    }
    if let Some((key, open)) = fold {
        let button = gtk::Button::builder()
            .icon_name(fold_icon(open))
            .tooltip_text(if open {
                "Fold the folders in this one"
            } else {
                "Show the folders in this one"
            })
            .css_classes(["flat", "circular"])
            .valign(gtk::Align::Center)
            .action_name("win.run")
            .action_target(&format!("fold-folder {}", rical_ui::command::quote(key)).to_variant())
            .build();
        b.append(&button);
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&b));
    row
}

impl Window {
    fn inbox_unread(&self) -> usize {
        self.state
            .borrow()
            .folder_with_role(Role::Inbox)
            .map_or(0, |f| f.unread)
    }

    /// A desktop notification for mail that arrived while the window was
    /// not in front: who it is from and the subject, nothing of the body.
    fn maybe_notify(&self, before: usize) {
        let now = self.inbox_unread();
        let (notify, in_inbox) = {
            let s = self.state.borrow();
            (
                s.opts.notify,
                s.folder().is_some_and(|f| f.role == Role::Inbox),
            )
        };
        if !notify || now <= before || self.window.is_active() {
            return;
        }
        let n = gio::Notification::new("New mail");
        if in_inbox {
            let s = self.state.borrow();
            if let Some(m) = s.messages.iter().find(|m| m.is_new) {
                n.set_title(&format!("New mail from {}", m.from_shown()));
                n.set_body(Some(if m.subject.is_empty() {
                    "(no subject)"
                } else {
                    &m.subject
                }));
            }
        } else {
            n.set_body(Some(&format!(
                "{} new message{}",
                now - before,
                if now - before == 1 { "" } else { "s" }
            )));
        }
        self.app.send_notification(Some("new-mail"), &n);
    }
}

/// The key for a unified place, as the command takes it.
pub fn unified_key(role: Role) -> &'static str {
    match role {
        Role::Inbox => "inbox",
        Role::Drafts => "drafts",
        Role::Sent => "sent",
        Role::Archive => "archive",
        Role::Junk => "junk",
        Role::Trash => "trash",
        Role::Other => "other",
    }
}

/// A label between groups of folders: not a place.
fn heading_row(text: &str) -> gtk::ListBoxRow {
    let l = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .css_classes(["dim-label", "caption-heading"])
        .margin_top(10)
        .margin_bottom(2)
        .margin_start(8)
        .build();
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&l));
    row.set_selectable(false);
    row.set_activatable(false);
    row
}

/// A name over folders, folding them away when clicked: an account, or a
/// part of a name that is not itself a folder (`[Gmail]`).
fn fold_row(name: &str, unread: usize, open: bool, indent: i32, strong: bool) -> gtk::ListBoxRow {
    let b = gtk::Box::builder()
        .spacing(8)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(4 + indent * 18)
        .margin_end(4)
        .build();
    b.append(&gtk::Image::from_icon_name(fold_icon(open)));
    let label = gtk::Label::builder()
        .label(name)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(pango::EllipsizeMode::End)
        .build();
    if strong || unread > 0 {
        label.add_css_class("heading");
    }
    b.append(&label);
    if unread > 0 {
        b.append(
            &gtk::Label::builder()
                .label(unread.to_string())
                .css_classes(["dim-label", "caption", "numeric"])
                .build(),
        );
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&b));
    row.set_selectable(false);
    row
}

impl Window {
    /// The message being written in the tab in front, if that is the one.
    pub fn current_compose(&self) -> Option<Rc<crate::compose::Compose>> {
        let page = self.tabs.selected_page()?;
        self.composes
            .borrow()
            .iter()
            .find(|c| c.page().as_ref() == Some(&page))
            .cloned()
    }

    /// Put a message being written in a tab of its own, and go to it.
    pub fn add_tab(self: &Rc<Self>, c: &Rc<crate::compose::Compose>) {
        let page = self.tabs.append(&c.root);
        page.set_title("New message");
        page.set_icon(Some(&gio::ThemedIcon::new("mail-message-new-symbolic")));
        c.set_page(page.clone());
        self.composes.borrow_mut().push(c.clone());
        self.tabs.set_selected_page(&page);
    }

    /// Tabs: the tab in front changed, a tab asked to close.
    fn setup_tabs(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.tabs.connect_selected_page_notify(move |_| {
            let Some(w) = weak.upgrade() else { return };
            match w.current_compose() {
                Some(c) => c.focus_first(),
                None => w.focus_view(),
            }
        });
        let weak = Rc::downgrade(self);
        self.tabs.connect_close_page(move |tabs, page| {
            let Some(w) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let found = w
                .composes
                .borrow()
                .iter()
                .find(|c| c.page().as_ref() == Some(page))
                .cloned();
            match found {
                // asks, if it would lose what was written
                Some(c) if c.is_unsaved() => {
                    tabs.close_page_finish(page, false);
                    c.ask_leave();
                }
                Some(c) => {
                    w.composes.borrow_mut().retain(|o| !Rc::ptr_eq(o, &c));
                    tabs.close_page_finish(page, true);
                }
                // the mail view stays
                None => tabs.close_page_finish(page, false),
            }
            glib::Propagation::Stop
        });
        // closing the window with something unsaved asks first
        let weak = Rc::downgrade(self);
        self.window
            .connect_close_request(move |_| match weak.upgrade() {
                Some(w) if w.unsaved_tab().is_some() => {
                    w.warn_unsaved();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            });
    }

    pub fn unsaved_tab(&self) -> Option<Rc<crate::compose::Compose>> {
        self.composes
            .borrow()
            .iter()
            .find(|c| c.is_unsaved())
            .cloned()
    }

    /// Go to the message with something unsaved, and ask what to do with it.
    pub fn warn_unsaved(&self) {
        if let Some(c) = self.unsaved_tab() {
            c.show_tab();
            self.toast("This message is not sent or saved");
        }
    }

    /// `:tab next|prev|mail|N`: go to a tab.
    pub fn tab_command(&self, which: &str) -> Result<(), String> {
        let n = self.tabs.n_pages();
        let at = self
            .tabs
            .selected_page()
            .map_or(0, |p| self.tabs.page_position(&p));
        let to = match which.trim() {
            "" | "next" => (at + 1) % n,
            "prev" | "previous" => (at + n - 1) % n,
            "mail" | "first" => 0,
            "last" => n - 1,
            num => num
                .parse::<i32>()
                .ok()
                .filter(|v| (1..=n).contains(v))
                .map(|v| v - 1)
                .ok_or(format!("no tab “{num}” (there are {n})"))?,
        };
        self.tabs.set_selected_page(&self.tabs.nth_page(to));
        Ok(())
    }
}

/// What can be done to a message, for its menu and a right click.
fn message_menu() -> gio::Menu {
    let menu = gio::Menu::new();
    let item = |m: &gio::Menu, label: &str, line: &str| {
        let i = gio::MenuItem::new(Some(label), None);
        i.set_action_and_target_value(Some("win.run"), Some(&line.to_variant()));
        m.append_item(&i);
    };
    let write = gio::Menu::new();
    item(&write, "_Reply", "reply");
    item(&write, "Reply to _All", "replyall");
    item(&write, "_Forward", "forward");
    menu.append_section(None, &write);
    let deal = gio::Menu::new();
    item(&deal, "Mark Read or _Unread", "mark toggle");
    item(&deal, "_Flag", "flag");
    item(&deal, "_Archive", "archive");
    item(&deal, "_Delete", "delete");
    item(&deal, "_Junk", "junk");
    let to = gio::MenuItem::new(Some("_Move To…"), None);
    to.set_action_and_target_value(Some("win.prompt"), Some(&"move ".to_variant()));
    deal.append_item(&to);
    item(&deal, "Pick the _Conversation", "pick thread");
    menu.append_section(None, &deal);
    let look = gio::Menu::new();
    item(&look, "_Links", "links");
    item(&look, "HTML or _Text", "view");
    item(&look, "_Headers", "headers");
    item(&look, "Message _Source", "source");
    item(&look, "_Save as File", "export");
    item(&look, "Sender in _Contacts", "contact");
    item(&look, "_Unsubscribe", "unsubscribe");
    menu.append_section(None, &look);
    menu
}

/// What a dragged message carries: the move takes what is selected or picked.
const DRAG_MESSAGES: &str = "rical-mail:messages";

impl Window {
    /// A folder row takes dragged messages: they move into it.
    fn droppable(&self, row: &gtk::ListBoxRow, folder: &str) {
        let drop = gtk::DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
        let (weak, folder) = (self.me.borrow().clone(), folder.to_owned());
        drop.connect_drop(move |_, value, _, _| {
            if value.get::<String>().ok().as_deref() != Some(DRAG_MESSAGES) {
                return false;
            }
            let Some(w) = weak.upgrade() else {
                return false;
            };
            if let Err(e) = w.execute(&format!("move {}", rical_ui::command::quote(&folder))) {
                w.toast(&e);
            }
            true
        });
        row.add_controller(drop);
    }
}

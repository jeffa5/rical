//! The window: folders, the messages in one, and the one being read.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
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
    state::{Place, SharedState},
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
    pub selection: gtk::SingleSelection,
    pub list: gtk::ListView,
    pub reader: Rc<Reader>,
    reader_title: adw::WindowTitle,
    list_stack: gtk::Stack,
    pub toasts: adw::ToastOverlay,
    pub history: RefCell<Vec<String>>,
    pending_keys: RefCell<Vec<rical_ui::binds::Key>>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    debounce: RefCell<Option<glib::SourceId>>,
    read_timer: RefCell<Option<glib::SourceId>>,
    /// Changes the app makes itself raise no reload until this has passed.
    pub quiet_until: Cell<Option<Instant>>,
    pub syncing: Cell<bool>,
    me: RefCell<std::rc::Weak<Window>>,
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
        doing.append(Some("_Fetch Mail"), Some("win.run::sync"));
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
        let fetch = gtk::Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Fetch mail (Ctrl+R)")
            .action_name("win.run")
            .action_target(&"sync".to_variant())
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
        list_header.pack_end(&fetch);
        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search mail: from:ann unread budget… (/)")
            .hexpand(true)
            .build();
        let top = gtk::Box::builder()
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        top.append(&search);
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
        toasts.set_child(Some(&outer));
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
            history: Default::default(),
            pending_keys: Default::default(),
            monitors: Default::default(),
            debounce: Default::default(),
            read_timer: Default::default(),
            quiet_until: Cell::new(None),
            syncing: Cell::new(false),
            me: Default::default(),
        });
        *this.me.borrow_mut() = Rc::downgrade(&this);
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
        this.setup_actions();
        this.watch();
        this.refresh();
        this.focus_view();
        this
    }

    pub fn present(&self) {
        self.window.present();
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(
            adw::Toast::builder()
                .title(text)
                .priority(adw::ToastPriority::High)
                .build(),
        );
    }

    /// Say that what the app does next to the folders is its own doing.
    pub fn quiet(&self) {
        self.quiet_until
            .set(Some(Instant::now() + Duration::from_millis(1500)));
    }

    /// Each row of the list: who, when, the subject, and what is on it.
    fn setup_rows(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        factory.connect_setup(|_, item| {
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
            outer.append(&dot);
            outer.append(&text);
            item.set_child(Some(&outer));
        });
        let state = self.state.clone();
        factory.connect_bind(move |_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let Some(n) = item
                .item()
                .and_downcast::<gtk::StringObject>()
                .and_then(|s| s.string().parse::<usize>().ok())
            else {
                return;
            };
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
            let outbox = s.place == Place::Outbox;
            let unread = m.unread() && !outbox;
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
                    .is_some_and(|f| matches!(f.role, Role::Sent | Role::Drafts)))
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
            let extra = s.outbox_notes.get(&m.path).cloned();
            note.set_visible(extra.is_some());
            note.set_label(&extra.unwrap_or_default());
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
            let path = sel
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .and_then(|s| s.string().parse::<usize>().ok())
                .and_then(|n| w.state.borrow().message_at(n).map(|m| m.path.clone()));
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
    }

    /// Everything shown, from the state.
    pub fn refresh(self: &Rc<Self>) {
        self.fill_folders();
        self.fill_list();
        self.show_message();
    }

    pub fn fill_folders(&self) {
        while let Some(c) = self.folder_list.first_child() {
            self.folder_list.remove(&c);
        }
        let s = self.state.borrow();
        let mut select: Option<gtk::ListBoxRow> = None;
        for f in &s.folders {
            let icon = match f.role {
                Role::Inbox => "mail-unread-symbolic",
                Role::Drafts => "document-edit-symbolic",
                Role::Sent => "mail-send-symbolic",
                Role::Archive => "folder-symbolic",
                Role::Junk => "mail-mark-junk-symbolic",
                Role::Trash => "user-trash-symbolic",
                Role::Other => "folder-symbolic",
            };
            let row = folder_row(icon, &f.name, f.unread);
            row.set_widget_name(&f.path.to_string_lossy());
            self.folder_list.append(&row);
            if s.place == Place::Folder(f.path.clone()) {
                select = Some(row);
            }
        }
        let n = s.outbox_count();
        let row = folder_row("network-transmit-symbolic", "Outbox", n);
        row.set_widget_name("outbox");
        self.folder_list.append(&row);
        if s.place == Place::Outbox {
            select = Some(row);
        }
        drop(s);
        if let Some(r) = select {
            self.syncing.set(true);
            self.folder_list.select_row(Some(&r));
            self.syncing.set(false);
        }
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
            let items = (0..s.shown.len()).map(|n| n.to_string()).collect();
            let selected = s.position().map(|p| p as u32);
            let unread = s
                .shown
                .iter()
                .filter(|&&i| s.messages[s.rows[i].index].unread())
                .count();
            (items, selected, s.place_name(), s.shown.len(), unread)
        };
        let refs: Vec<&str> = items.iter().map(String::as_str).collect();
        self.model.splice(0, self.model.n_items(), &refs);
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
        let (path, in_contacts, note, delay) = {
            let s = self.state.borrow();
            let Some(m) = s.selected_summary() else {
                drop(s);
                self.state.borrow_mut().opened = None;
                self.reader.clear();
                self.reader_title.set_title("");
                return;
            };
            let in_contacts = false;
            (
                m.path.clone(),
                in_contacts,
                s.outbox_notes
                    .get(&m.path)
                    .filter(|_| s.place == Place::Outbox)
                    .cloned(),
                s.opts.read_after,
            )
        };
        match read::open(&path) {
            Ok(o) => {
                let in_contacts = in_contacts
                    || o.from
                        .first()
                        .is_some_and(|f| self.state.borrow().is_contact(&f.address));
                self.reader_title.set_title(&o.subject);
                let diffs = if o.invites.is_empty() {
                    Vec::new()
                } else {
                    self.state.borrow_mut().invite_diffs(&path)
                };
                self.reader.show(&o, in_contacts, note.as_deref(), &diffs);
                self.state.borrow_mut().opened = Some(Rc::new(o));
            }
            Err(e) => {
                self.state.borrow_mut().opened = None;
                self.reader.clear();
                self.toast(&format!("{e:#}"));
            }
        }
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
                w.state.borrow_mut().reload_folders();
                w.fill_folders();
                return;
            }
            let before = w.inbox_unread();
            w.reload();
            w.maybe_notify(before);
        });
        *self.debounce.borrow_mut() = Some(id);
    }

    /// Read the folders and the current list again, keeping the place.
    pub fn reload(self: &Rc<Self>) {
        {
            let mut s = self.state.borrow_mut();
            s.reload_folders();
            s.reload_messages();
        }
        self.refresh();
    }
}

fn folder_row(icon: &str, name: &str, unread: usize) -> gtk::ListBoxRow {
    let b = gtk::Box::builder()
        .spacing(10)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(4)
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

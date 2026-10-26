//! The window: everyone in a list with search, the selected person beside
//! it, and the duplicates to tidy up.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use adw::prelude::*;
use gtk::{gdk, gio, glib};
use rical_cards::CardId;
use rical_ui::{
    binds::{Binds, Lookup},
    keys::{is_navigation, key_of},
    palette,
};

use crate::state::{SharedState, View};

pub struct Window {
    pub window: adw::ApplicationWindow,
    pub state: SharedState,
    pub binds: Binds,
    title: adw::WindowTitle,
    pub split: adw::NavigationSplitView,
    pub search: gtk::SearchEntry,
    views: adw::ViewStack,
    toggle: adw::ToggleGroup,
    /// The people list: a virtualised list of index entries, by number.
    model: gtk::StringList,
    pub selection: gtk::SingleSelection,
    pub list: gtk::ListView,
    pub dup_list: gtk::ListBox,
    detail: adw::NavigationPage,
    pub toasts: adw::ToastOverlay,
    pub history: RefCell<Vec<String>>,
    pending_keys: RefCell<Vec<rical_ui::binds::Key>>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    debounce: RefCell<Option<glib::SourceId>>,
    trim: rical_ui::util::Trimmer,
    /// Set while the window updates widgets, so their signals are ignored.
    pub syncing: Cell<bool>,
    /// The window itself, for callbacks that come later.
    me: RefCell<std::rc::Weak<Window>>,
    /// Set while working out the duplicates is due.
    dups_pending: Cell<bool>,
}

impl Window {
    pub fn new(app: &adw::Application, state: SharedState, binds: Binds) -> Rc<Self> {
        let title = adw::WindowTitle::new(crate::APP_NAME, "");
        let menu = gio::Menu::new();
        let doing = gio::Menu::new();
        doing.append(Some("_Command Line"), Some("win.palette"));
        doing.append(Some("_Reload"), Some("win.run::reload"));
        menu.append_section(None, &doing);
        let app_items = gio::Menu::new();
        app_items.append(Some("_Preferences"), Some("win.run::settings"));
        app_items.append(Some("_Keyboard Shortcuts"), Some("win.run::help"));
        app_items.append(Some("_About Contacts"), Some("win.run::about"));
        menu.append_section(None, &app_items);
        let leaving = gio::Menu::new();
        leaving.append(Some("_Quit"), Some("win.run::quit"));
        menu.append_section(None, &leaving);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Main menu")
            .build();
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("New contact (c)")
            .action_name("win.run")
            .action_target(&"new".to_variant())
            .build();
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&add);
        header.pack_end(&menu_button);

        let search = gtk::SearchEntry::builder()
            .placeholder_text("Search names, email, numbers, places… (/)")
            .hexpand(true)
            .build();
        let toggle = adw::ToggleGroup::builder()
            .css_classes(["round"])
            .homogeneous(false)
            .build();
        for (name, label) in [("people", "People"), ("duplicates", "Duplicates")] {
            toggle.add(adw::Toggle::builder().name(name).label(label).build());
        }
        let top = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .margin_start(12)
            .margin_end(12)
            .margin_top(6)
            .margin_bottom(6)
            .build();
        top.append(&search);
        top.append(&toggle);

        // the people: built as they scroll into view, so 50,000 cost no
        // more than the dozen on screen
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
        list.update_property(&[gtk::accessible::Property::Label("Contacts")]);
        let people_scroller = gtk::ScrolledWindow::builder()
            .child(&list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let no_one = adw::StatusPage::builder()
            .icon_name("avatar-default-symbolic")
            .title("No one found")
            .description("Nothing in any field matches")
            .build();
        let people = gtk::Stack::new();
        people.add_named(&people_scroller, Some("list"));
        people.add_named(&no_one, Some("none"));

        let dup_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["navigation-sidebar", "rical-list"])
            .build();
        let dup_scroller = gtk::ScrolledWindow::builder()
            .child(&dup_list)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        let tidy = adw::StatusPage::builder()
            .icon_name("object-select-symbolic")
            .title("No duplicates")
            .description("Everyone is in your contacts once")
            .build();
        let dups = gtk::Stack::new();
        dups.add_named(&dup_scroller, Some("list"));
        dups.add_named(&tidy, Some("none"));

        let views = adw::ViewStack::new();
        views.add_named(&people, Some("people"));
        views.add_named(&dups, Some("duplicates"));
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        sidebar_box.append(&top);
        sidebar_box.append(&views);
        let sidebar_view = adw::ToolbarView::new();
        sidebar_view.add_top_bar(&header);
        sidebar_view.set_content(Some(&sidebar_box));
        let sidebar = adw::NavigationPage::builder()
            .title("Contacts")
            .child(&sidebar_view)
            .build();

        let detail = adw::NavigationPage::builder().title("Contact").build();
        let split = adw::NavigationSplitView::builder()
            .sidebar(&sidebar)
            .content(&detail)
            .min_sidebar_width(280.0)
            .max_sidebar_width(400.0)
            .build();
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&split));
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(crate::APP_NAME)
            .default_width(1000)
            .default_height(720)
            .content(&toasts)
            .build();
        // narrow: one pane at a time
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            640.0,
            adw::LengthUnit::Sp,
        ));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(narrow);

        let this = Rc::new(Self {
            window,
            state,
            binds,
            title,
            split,
            search,
            views,
            toggle,
            model,
            selection,
            list,
            dup_list,
            detail,
            toasts,
            history: Default::default(),
            pending_keys: Default::default(),
            monitors: Default::default(),
            debounce: Default::default(),
            trim: Default::default(),
            syncing: Cell::new(false),
            me: Default::default(),
            dups_pending: Cell::new(false),
        });
        *this.me.borrow_mut() = Rc::downgrade(&this);
        this.setup_rows(&factory);
        this.setup_signals(&people, &dups);
        this.setup_actions();
        this.watch();
        this.refresh();
        this.show_detail();
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

    /// Each row of the people list: initials, name, and an email or number.
    fn setup_rows(self: &Rc<Self>, factory: &gtk::SignalListItemFactory) {
        factory.connect_setup(|_, item| {
            let item = item.downcast_ref::<gtk::ListItem>().unwrap();
            let b = gtk::Box::builder()
                .spacing(10)
                .margin_top(4)
                .margin_bottom(4)
                .build();
            let avatar = adw::Avatar::new(32, None, true);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
            let name = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build();
            let detail = gtk::Label::builder()
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .css_classes(["dim-label", "caption"])
                .build();
            text.append(&name);
            text.append(&detail);
            b.append(&avatar);
            b.append(&text);
            item.set_child(Some(&b));
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
            let Some(b) = item.child().and_downcast::<gtk::Box>() else {
                return;
            };
            let s = state.borrow();
            let Some(e) = s.index.entries.get(n) else {
                return;
            };
            let avatar = b.first_child().and_downcast::<adw::Avatar>().unwrap();
            let text = avatar.next_sibling().unwrap();
            let name = text.first_child().and_downcast::<gtk::Label>().unwrap();
            let detail = name.next_sibling().and_downcast::<gtk::Label>().unwrap();
            avatar.set_text(Some(&e.name));
            name.set_label(&e.name);
            detail.set_label(&e.detail);
            detail.set_visible(!e.detail.is_empty());
        });
    }

    fn setup_signals(self: &Rc<Self>, people: &gtk::Stack, dups: &gtk::Stack) {
        let weak = Rc::downgrade(self);
        self.search.connect_search_changed(move |_| {
            let Some(w) = weak.upgrade() else { return };
            if w.syncing.get() {
                return;
            }
            w.search_now();
        });
        // Down from the search goes into the list; Enter opens the first
        let weak = Rc::downgrade(self);
        self.search.connect_activate(move |_| {
            if let Some(w) = weak.upgrade() {
                w.search_now();
                w.select_index(0);
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
            let id = sel
                .selected_item()
                .and_downcast::<gtk::StringObject>()
                .and_then(|s| s.string().parse::<usize>().ok())
                .and_then(|n| w.state.borrow().index.entries.get(n).map(|e| e.id.clone()));
            w.state.borrow_mut().selected = id;
            w.show_detail();
        });
        let weak = Rc::downgrade(self);
        self.list.connect_activate(move |_, _| {
            if let Some(w) = weak.upgrade() {
                w.split.set_show_content(true);
            }
        });
        let weak = Rc::downgrade(self);
        self.toggle.connect_active_name_notify(move |t| {
            let Some(w) = weak.upgrade() else { return };
            if w.syncing.get() {
                return;
            }
            if let Some(name) = t.active_name() {
                let _ = w.execute(&format!("view {name}"));
            }
        });
        let weak = Rc::downgrade(self);
        self.dup_list.connect_row_selected(move |_, row| {
            let Some(w) = weak.upgrade() else { return };
            if w.syncing.get() {
                return;
            }
            w.state.borrow_mut().selected_group = row.map(|r| r.index() as usize);
            w.show_detail();
        });
        let weak = Rc::downgrade(self);
        self.dup_list.connect_row_activated(move |_, _| {
            if let Some(w) = weak.upgrade() {
                let _ = w.execute("merge");
            }
        });
        // empty states
        let (p, d) = (people.clone(), dups.clone());
        let weak = Rc::downgrade(self);
        self.model.connect_items_changed(move |m, _, _, _| {
            p.set_visible_child_name(if m.n_items() == 0 { "none" } else { "list" });
            if let Some(w) = weak.upgrade() {
                let n = w.state.borrow().index.entries.len();
                let shown = m.n_items();
                w.title.set_subtitle(&if shown as usize == n {
                    format!("{n} people")
                } else {
                    format!("{shown} of {n}")
                });
            }
        });
        let _ = d;
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
            f.is::<gtk::Text>() || f.is::<gtk::Editable>() || f.is::<gtk::TextView>()
        });
        let Some(key) = key_of(keyval, mods) else {
            return Proceed;
        };
        if typing {
            // from the search, Down goes to the list (others' entries, the
            // command line's, keep their keys)
            let in_search = focus.as_ref().is_some_and(|f| f.is_ancestor(&self.search));
            if in_search && key.name == "down" && !key.ctrl {
                self.search_now();
                self.focus_view();
                return Stop;
            }
            return Proceed;
        }
        // in the person pane, arrows and Enter work its rows
        let in_detail = focus.as_ref().is_some_and(|f| f.is_ancestor(&self.detail));
        if in_detail && is_navigation(&key) && key.name != "esc" {
            self.pending_keys.borrow_mut().clear();
            return Proceed;
        }
        let section = self.state.borrow().view.section();
        let mut pending = self.pending_keys.borrow_mut();
        pending.push(key.clone());
        let mut found = self.binds.lookup(section, &pending);
        if matches!(found, Lookup::None) && pending.len() > 1 {
            pending.clear();
            pending.push(key);
            found = self.binds.lookup(section, &pending);
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

    /// Search for what is typed now, without waiting for typing to pause.
    pub fn search_now(&self) {
        let text = self.search.text().to_string();
        if self.state.borrow().query == text {
            return;
        }
        self.state.borrow_mut().query = text;
        self.state.borrow_mut().search();
        self.fill_list();
        self.show_detail();
    }

    /// Show the list for what is searched, keeping the selection.
    pub fn fill_list(&self) {
        self.syncing.set(true);
        let (items, selected): (Vec<String>, Option<u32>) = {
            let mut s = self.state.borrow_mut();
            let items = s.shown.iter().map(|n| n.to_string()).collect();
            let selected = s.selected.as_ref().and_then(|id| {
                let n = s.index.position(id)?;
                s.shown.iter().position(|&m| m == n).map(|p| p as u32)
            });
            // someone the search hides gives way to the best match
            let selected = selected.or_else(|| {
                let first = *s.shown.first()?;
                s.selected = Some(s.index.entries[first].id.clone());
                Some(0)
            });
            if selected.is_none() {
                s.selected = None;
            }
            (items, selected)
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
        self.trim.soon();
    }

    /// Select the `n`th row shown, and show them.
    pub fn select_index(&self, n: usize) {
        let count = self.model.n_items() as usize;
        if count == 0 {
            return;
        }
        let n = n.min(count - 1) as u32;
        self.selection.set_selected(n);
        self.list.scroll_to(n, gtk::ListScrollFlags::FOCUS, None);
    }

    /// Select a card, searching it out of a filter that hides it.
    pub fn select_card(&self, id: &CardId) {
        {
            let mut s = self.state.borrow_mut();
            s.selected = Some(id.clone());
            let shown = s.index.position(id).is_some_and(|n| s.shown.contains(&n));
            if !shown {
                s.query.clear();
                s.search();
            }
        }
        self.syncing.set(true);
        let q = self.state.borrow().query.clone();
        if self.search.text() != q {
            self.search.set_text(&q);
        }
        self.syncing.set(false);
        self.fill_list();
        self.show_detail();
    }

    /// Everything again, from the state.
    pub fn refresh(&self) {
        self.state.borrow_mut().refresh();
        let view = self.state.borrow().view;
        self.syncing.set(true);
        self.toggle.set_active_name(Some(view.section()));
        self.syncing.set(false);
        self.views.set_visible_child_name(view.section());
        self.fill_list();
        self.duplicates_soon();
        self.show_detail();
    }

    /// The duplicates again: now when they are showing, else once the
    /// window is drawn (with 50,000 people it takes a moment).
    pub fn duplicates_soon(&self) {
        if self.state.borrow().view == View::Duplicates {
            self.fill_duplicates();
            return;
        }
        if self.dups_pending.replace(true) {
            return;
        }
        let weak = self.me.borrow().clone();
        glib::idle_add_local_once(move || {
            if let Some(w) = weak.upgrade() {
                w.dups_pending.set(false);
                w.fill_duplicates();
            }
        });
    }

    /// The duplicates list, and its count on the switch.
    pub fn fill_duplicates(&self) {
        let groups = self.state.borrow_mut().groups();
        self.syncing.set(true);
        while let Some(row) = self.dup_list.row_at_index(0) {
            self.dup_list.remove(&row);
        }
        let s = self.state.borrow();
        for g in groups.iter() {
            let names: Vec<String> = g
                .cards
                .iter()
                .filter_map(|id| s.books.get(id).map(|c| c.display_name()))
                .collect();
            let mut unique = names.clone();
            unique.dedup();
            let title = if unique.len() == 1 {
                format!("{} ×{}", unique[0], names.len())
            } else {
                unique.join(", ")
            };
            let why = g
                .reasons
                .iter()
                .map(|r| r.describe())
                .collect::<Vec<_>>()
                .join("; ");
            let row = adw::ActionRow::builder()
                .title(gtk::glib::markup_escape_text(&title))
                .subtitle(gtk::glib::markup_escape_text(&why))
                .activatable(true)
                .build();
            if !g.is_strong() {
                row.add_suffix(
                    &gtk::Label::builder()
                        .label("maybe")
                        .css_classes(["dim-label", "caption"])
                        .build(),
                );
            }
            self.dup_list.append(&row);
        }
        if let Some(i) = s.selected_group
            && let Some(row) = self.dup_list.row_at_index(i as i32)
        {
            self.dup_list.select_row(Some(&row));
        }
        if let Some(t) = self.toggle.toggle_by_name("duplicates") {
            t.set_label(Some(&if groups.is_empty() {
                "Duplicates".to_owned()
            } else {
                format!("Duplicates ({})", groups.len())
            }));
        }
        if let Some(stack) = self
            .dup_list
            .ancestor(gtk::Stack::static_type())
            .and_downcast::<gtk::Stack>()
        {
            stack.set_visible_child_name(if groups.is_empty() { "none" } else { "list" });
        }
        self.syncing.set(false);
    }

    /// The right-hand pane: the selected person, or the selected duplicates.
    pub fn show_detail(&self) {
        let s = self.state.borrow();
        let content = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(18)
            .margin_top(24)
            .margin_bottom(24)
            .margin_start(12)
            .margin_end(12)
            .build();
        let mut title = "Contact".to_owned();
        match s.view {
            View::People => match s
                .selected
                .as_ref()
                .and_then(|id| Some((id, s.books.get(id)?)))
            {
                Some((id, card)) => {
                    title = card.display_name();
                    content.append(&rical_ui::card::heading(&card, 96));
                    content.append(&self.actions(&card));
                    let today = chrono::Local::now().date_naive();
                    for g in rical_ui::card::groups(&card, today) {
                        content.append(&g);
                    }
                    content.append(&where_kept(&s, id));
                }
                None => {
                    content.append(
                        &adw::StatusPage::builder()
                            .icon_name("avatar-default-symbolic")
                            .title("No one selected")
                            .description("Choose someone, or press c for someone new")
                            .vexpand(true)
                            .build(),
                    );
                }
            },
            View::Duplicates => {
                title = "Duplicates".to_owned();
                content.append(&crate::tidy::preview(&s));
            }
        }
        drop(s);
        let clamp = adw::Clamp::builder()
            .maximum_size(640)
            .child(&content)
            .build();
        let scroller = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let view = adw::ToolbarView::new();
        view.add_top_bar(&adw::HeaderBar::builder().show_title(false).build());
        view.set_content(Some(&scroller));
        self.detail.set_title(&title);
        self.detail.set_child(Some(&view));
    }

    /// The buttons under a person's name.
    fn actions(&self, card: &rical_cards::Card) -> gtk::Box {
        let b = gtk::Box::builder()
            .spacing(6)
            .halign(gtk::Align::Center)
            .build();
        let button = |label: &str, tip: &str, cmd: &str, suggested: bool| {
            let btn = gtk::Button::builder()
                .label(label)
                .tooltip_text(tip)
                .action_name("win.run")
                .action_target(&cmd.to_variant())
                .build();
            if suggested {
                btn.add_css_class("suggested-action");
            }
            btn
        };
        if !card.emails().is_empty() {
            b.append(&button("Email", "Write to them (M)", "mail", true));
        }
        if !card.phones().is_empty() {
            b.append(&button(
                "Call",
                "Call them (P)",
                "call",
                card.emails().is_empty(),
            ));
        }
        b.append(&button("Edit", "Edit (e)", "edit", false));
        let more = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Open in another app (O)")
            .action_name("win.run")
            .action_target(&"open-file".to_variant())
            .css_classes(["flat"])
            .build();
        b.append(&more);
        let delete = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete (x)")
            .action_name("win.run")
            .action_target(&"delete".to_variant())
            .css_classes(["flat"])
            .build();
        b.append(&delete);
        b
    }

    /// Give the list keyboard focus, now (so a key pressed straight after
    /// goes to it), then on the selected row once it is drawn.
    pub fn focus_view(&self) {
        match self.state.borrow().view {
            View::People => {
                self.list.grab_focus();
                let at = self.selection.selected();
                if at != gtk::INVALID_LIST_POSITION {
                    let list = self.list.clone();
                    glib::idle_add_local_once(move || {
                        list.scroll_to(at, gtk::ListScrollFlags::FOCUS, None);
                    });
                }
            }
            View::Duplicates => match self.dup_list.selected_row() {
                Some(r) => {
                    r.grab_focus();
                }
                None => {
                    self.dup_list.grab_focus();
                }
            },
        }
    }

    /// Watch the address books, and catch up when they change (vdirsyncer
    /// syncing, another app editing).
    fn watch(self: &Rc<Self>) {
        let mut monitors = Vec::new();
        let dirs: Vec<_> = self
            .state
            .borrow()
            .books
            .books
            .iter()
            .map(|b| b.path.clone())
            .collect();
        for dir in dirs {
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

    /// Re-read what changed, once changes stop for a moment.
    fn catch_up_soon(self: &Rc<Self>) {
        if let Some(id) = self.debounce.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let Some(w) = weak.upgrade() else { return };
            w.debounce.borrow_mut().take();
            let changed = w.state.borrow_mut().books.refresh();
            match changed {
                Ok(true) => {
                    log::info!("address books changed on disk");
                    w.refresh();
                }
                Ok(false) => {}
                Err(e) => log::warn!("could not read the address books: {e:#}"),
            }
        });
        *self.debounce.borrow_mut() = Some(id);
    }
}

/// Which address books a person is in.
fn where_kept(s: &crate::state::State, id: &CardId) -> gtk::Label {
    let book = s.books.book(id).map(|b| b.name.clone()).unwrap_or_default();
    let copies: Vec<String> = s
        .index
        .position(id)
        .map(|n| {
            s.index.entries[n]
                .copies
                .iter()
                .filter_map(|c| s.books.book(c).map(|b| b.name.clone()))
                .collect()
        })
        .unwrap_or_default();
    let text = if copies.is_empty() {
        format!("In {book}")
    } else {
        format!("In {book}; also in {}", copies.join(", "))
    };
    gtk::Label::builder()
        .label(&text)
        .css_classes(["dim-label", "caption"])
        .tooltip_text(id.path.display().to_string())
        .build()
}

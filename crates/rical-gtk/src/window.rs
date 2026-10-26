//! The main window: sidebar of calendars, header bar with navigation, and
//! the stack of views. Owns keyboard handling and the vdir file watcher.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    path::PathBuf,
    rc::{Rc, Weak},
    time::Duration,
};

use adw::prelude::*;
use chrono::{Days, Utc};
use gtk::{gio, glib};
use log::{info, warn};

#[path = "importer.rs"]
mod importer;
#[path = "notify.rs"]
mod notify;
#[path = "ops.rs"]
mod ops;
#[path = "palette.rs"]
mod palette;
#[path = "run.rs"]
mod run;
#[path = "search.rs"]
mod search;

use crate::{
    agenda::AgendaView,
    binds::{Binds, Key},
    details, format,
    grid::{DragAction, TimeGrid},
    invitations::{InvitationsView, OnReply},
    journals::JournalsView,
    layout::ViewKind,
    minimonth::MiniMonth,
    month::MonthView,
    section::{self, Section},
    sidetasks::SideTasks,
    state::SharedState,
    tasks::TasksView,
    year::YearView,
};

pub struct Window {
    pub window: adw::ApplicationWindow,
    state: SharedState,
    title: adw::WindowTitle,
    split: adw::OverlaySplitView,
    views: gtk::Stack,
    /// View switchers: in the header, and in the bottom bar on narrow windows.
    toggles: [adw::ToggleGroup; 2],
    /// The calendar view each switcher's Calendar part names.
    calendar_labels: [gtk::Label; 2],
    /// The header's + button: a new event, task or journal entry by view.
    add: gtk::Button,
    mini: Rc<MiniMonth>,
    calendar_list: gtk::ListBox,
    grid: Rc<TimeGrid>,
    month: Rc<MonthView>,
    year: Rc<YearView>,
    agenda: Rc<AgendaView>,
    invitations: Rc<InvitationsView>,
    journals: Rc<JournalsView>,
    tasks: Rc<TasksView>,
    side_tasks: Rc<SideTasks>,
    /// The collapsible sidebar sections.
    sections: Rc<RefCell<Vec<Rc<Section>>>>,
    syncing: Cell<bool>,
    /// A trim of freed memory is due (see [`Window::trim_soon`]).
    trim_pending: std::rc::Rc<Cell<bool>>,
    /// First day and number of days the grid last showed.
    last_grid_days: Cell<Option<(chrono::NaiveDate, usize)>>,
    monitors: RefCell<Vec<gio::FileMonitor>>,
    pending: RefCell<BTreeSet<PathBuf>>,
    reload_all_pending: Cell<bool>,
    debounce: RefCell<Option<glib::SourceId>>,
    toasts: adw::ToastOverlay,
    binds: Binds,
    pending_keys: RefCell<Vec<Key>>,
    /// Command line history, oldest first.
    history: RefCell<Vec<String>>,
    /// Scope given by a command, so it does not ask.
    forced_scope: Cell<Option<rical_core::write::Scope>>,
    /// Alarms up to here have been checked.
    alarm_check: Cell<chrono::DateTime<Utc>>,
    alarms_fired: RefCell<notify::Fired>,
    alarm_plan: RefCell<notify::Plan>,
}

impl Window {
    pub fn new(app: &adw::Application, state: SharedState, binds: Binds) -> Rc<Self> {
        let this = Rc::new_cyclic(|weak: &Weak<Self>| {
            let on_change: Rc<dyn Fn()> = {
                let weak = weak.clone();
                Rc::new(move || {
                    if let Some(w) = weak.upgrade() {
                        w.refresh();
                    }
                })
            };
            let on_drag: Rc<dyn Fn(DragAction)> = {
                let weak = weak.clone();
                Rc::new(move |action| {
                    if let Some(w) = weak.upgrade() {
                        w.on_drag(action);
                    }
                })
            };
            let on_toggle: Rc<dyn Fn((PathBuf, String), bool)> = {
                let weak = weak.clone();
                Rc::new(move |id, done| {
                    if let Some(w) = weak.upgrade() {
                        w.set_task_done(&id, done);
                    }
                })
            };
            let on_reply: OnReply = {
                let weak = weak.clone();
                Rc::new(move |key, status| {
                    if let Some(w) = weak.upgrade() {
                        w.state.borrow_mut().selected = Some(key);
                        if let Err(e) = w.reply(status) {
                            w.toast(&e);
                        }
                    }
                })
            };
            Self::build(app, state, on_change, on_drag, on_toggle, on_reply, binds)
        });
        this.setup_signals();
        this.setup_actions();
        this.setup_keys();
        this.watch();
        this.rebuild_calendar_list();
        this.refresh();

        // reminders
        this.check_alarms();
        let weak = Rc::downgrade(&this);
        glib::timeout_add_seconds_local(15, move || match weak.upgrade() {
            Some(w) => {
                w.check_alarms();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });

        // keep the now line current, and everything else when the day
        // changes (today's highlight, tasks carried over to today)
        let weak = Rc::downgrade(&this);
        let mut today = this.state.borrow().today();
        glib::timeout_add_seconds_local(30, move || match weak.upgrade() {
            Some(w) => {
                let now = w.state.borrow().today();
                if now != today {
                    today = now;
                    w.refresh();
                } else {
                    w.grid.tick();
                }
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        this
    }

    fn build(
        app: &adw::Application,
        state: SharedState,
        on_change: Rc<dyn Fn()>,
        on_drag: Rc<dyn Fn(DragAction)>,
        on_toggle: Rc<dyn Fn((PathBuf, String), bool)>,
        on_reply: OnReply,
        binds: Binds,
    ) -> Self {
        let grid = TimeGrid::new(state.clone(), on_change.clone(), on_drag);
        let month = MonthView::new(state.clone(), on_change.clone());
        let year = YearView::new(state.clone(), on_change.clone());
        let agenda = AgendaView::new(state.clone(), on_change.clone(), on_toggle.clone());
        let invitations = InvitationsView::new(state.clone(), on_change.clone(), on_reply);
        let journals = JournalsView::new(state.clone(), on_change.clone());
        let side_tasks = SideTasks::new(state.clone(), on_toggle.clone());
        let tasks = TasksView::new(state.clone(), on_change.clone(), on_toggle);

        let views = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(100)
            .build();
        views.add_named(&grid.widget, Some("grid"));
        views.add_named(&month.area, Some("month"));
        views.add_named(&year.area, Some("year"));
        views.add_named(&agenda.widget, Some("agenda"));
        views.add_named(&tasks.widget, Some("tasks"));
        views.add_named(&invitations.widget, Some("invitations"));
        views.add_named(&journals.widget, Some("journal"));

        let title = adw::WindowTitle::new("", "");

        // the view switcher: in the header, or on narrow windows a bottom bar
        let (toggle, label_top) = view_toggles(&state);
        let (toggle_bottom, label_bottom) = view_toggles(&state);
        let bottom_bar = gtk::ActionBar::builder().revealed(false).build();
        bottom_bar.set_center_widget(Some(&toggle_bottom));

        let sidebar_button = gtk::ToggleButton::builder()
            .icon_name("sidebar-show-symbolic")
            .tooltip_text("Toggle sidebar (F9)")
            .action_name("win.run")
            .action_target(&"sidebar".to_variant())
            .build();
        let today = gtk::Button::builder()
            .label("Today")
            .tooltip_text("Go to today (t)")
            .action_name("win.run")
            .action_target(&"today".to_variant())
            .build();
        // an icon in its place on narrow windows
        let today_icon = gtk::Button::builder()
            .icon_name("x-office-calendar-symbolic")
            .tooltip_text("Go to today (t)")
            .action_name("win.run")
            .action_target(&"today".to_variant())
            .visible(false)
            .build();
        let prev = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .tooltip_text("Previous (p)")
            .action_name("win.run")
            .action_target(&"prev".to_variant())
            .build();
        let next = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .tooltip_text("Next (n)")
            .action_name("win.run")
            .action_target(&"next".to_variant())
            .build();
        let nav = gtk::Box::builder().css_classes(["linked"]).build();
        nav.append(&prev);
        nav.append(&next);

        // grouped as GNOME apps do: doing, setting up, leaving
        let menu = gio::Menu::new();
        let doing = gio::Menu::new();
        doing.append(Some("_Command Line"), Some("win.palette"));
        doing.append(Some("_Reload"), Some("win.run::reload"));
        menu.append_section(None, &doing);
        let app_items = gio::Menu::new();
        app_items.append(Some("_Preferences"), Some("win.run::settings"));
        app_items.append(Some("_Keyboard Shortcuts"), Some("win.run::help"));
        app_items.append(Some("_About Calendar"), Some("win.run::about"));
        menu.append_section(None, &app_items);
        let leaving = gio::Menu::new();
        leaving.append(Some("_Quit"), Some("win.run::quit"));
        menu.append_section(None, &leaving);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .menu_model(&menu)
            .tooltip_text("Main menu")
            .build();

        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&sidebar_button);
        header.pack_start(&today);
        header.pack_start(&today_icon);
        header.pack_start(&nav);
        let add = gtk::Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("New event (c)")
            .action_name("win.run")
            .action_target(&"new".to_variant())
            .build();
        header.pack_end(&add);
        header.pack_end(&menu_button);
        header.pack_end(&toggle);

        let content = adw::ToolbarView::new();
        content.add_top_bar(&header);
        content.add_bottom_bar(&bottom_bar);
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&views));
        content.set_content(Some(&toasts));

        let mini = MiniMonth::new(state.clone(), {
            let (state, changed) = (state.clone(), on_change.clone());
            Rc::new(move |day| {
                {
                    let mut s = state.borrow_mut();
                    s.date = day;
                    s.selected = None;
                }
                changed();
            })
        });
        let calendar_list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .css_classes(["navigation-sidebar"])
            .build();
        calendar_list.update_property(&[gtk::accessible::Property::Label("Calendars")]);
        // a small heading over each account's calendars
        calendar_list.set_header_func(|row, before| {
            let parent = row.widget_name();
            let shown = !parent.is_empty() && before.is_none_or(|b| b.widget_name() != parent);
            row.set_header(
                shown
                    .then(|| {
                        gtk::Label::builder()
                            .label(parent.as_str())
                            .css_classes(["dim-label", "caption-heading"])
                            .xalign(0.0)
                            .margin_start(12)
                            .margin_top(if before.is_some() { 8 } else { 2 })
                            .margin_bottom(2)
                            .build()
                    })
                    .as_ref(),
            );
        });
        calendar_list.connect_row_activated(|_, row| {
            if let Some(check) = calendar_check(row) {
                check.set_active(!check.is_active());
            }
        });
        // collapsible sections, remembered between runs
        let sections: Rc<RefCell<Vec<Rc<Section>>>> = Default::default();
        let save: Rc<dyn Fn()> = {
            let sections = sections.clone();
            Rc::new(move || section::save_collapsed(&sections.borrow()))
        };
        let collapsed = section::load_collapsed();
        let sidebar_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for (name, title, child) in [
            (
                "month",
                "Month",
                mini.widget.clone().upcast::<gtk::Widget>(),
            ),
            ("calendars", "Calendars", calendar_list.clone().upcast()),
            ("tasks", "Tasks", side_tasks.widget.clone().upcast()),
        ] {
            let s = Section::new(name, title, &child, !collapsed.contains(name), save.clone());
            sidebar_box.append(&s.widget);
            sections.borrow_mut().push(s);
        }
        let sidebar_scroll = gtk::ScrolledWindow::builder()
            .child(&sidebar_box)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        let sidebar = adw::ToolbarView::new();
        sidebar.add_top_bar(
            &adw::HeaderBar::builder()
                .title_widget(&adw::WindowTitle::new(crate::APP_NAME, ""))
                .show_end_title_buttons(false)
                .build(),
        );
        sidebar.set_content(Some(&sidebar_scroll));

        let split = adw::OverlaySplitView::builder()
            .sidebar(&sidebar)
            .content(&content)
            .min_sidebar_width(250.0)
            .max_sidebar_width(300.0)
            .build();
        split
            .bind_property("show-sidebar", &sidebar_button, "active")
            .bidirectional()
            .sync_create()
            .build();

        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title(crate::APP_NAME)
            .default_width(1280)
            .default_height(860)
            .content(&split)
            .build();

        // narrower: the sidebar overlays the calendar instead of taking room
        let medium = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            1100.0,
            adw::LengthUnit::Sp,
        ));
        medium.add_setter(&split, "collapsed", Some(&true.to_value()));
        window.add_breakpoint(medium);
        // narrowest: the view switcher moves to a bottom bar so the title
        // has room (the last matching breakpoint applies)
        let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
            adw::BreakpointConditionLengthType::MaxWidth,
            860.0,
            adw::LengthUnit::Sp,
        ));
        narrow.add_setter(&split, "collapsed", Some(&true.to_value()));
        narrow.add_setter(&toggle, "visible", Some(&false.to_value()));
        narrow.add_setter(&bottom_bar, "revealed", Some(&true.to_value()));
        narrow.add_setter(&today, "visible", Some(&false.to_value()));
        narrow.add_setter(&today_icon, "visible", Some(&true.to_value()));
        window.add_breakpoint(narrow);

        Self {
            window,
            state,
            title,
            split,
            views,
            toggles: [toggle, toggle_bottom],
            calendar_labels: [label_top, label_bottom],
            add: add.clone(),
            mini,
            calendar_list,
            grid,
            month,
            year,
            agenda,
            invitations,
            journals,
            tasks,
            side_tasks,
            sections,
            syncing: Cell::new(false),
            trim_pending: Default::default(),
            last_grid_days: Cell::new(None),
            monitors: Default::default(),
            pending: Default::default(),
            reload_all_pending: Cell::new(false),
            debounce: Default::default(),
            toasts,
            binds,
            pending_keys: Default::default(),
            history: Default::default(),
            forced_scope: Cell::new(None),
            alarm_check: Cell::new(Utc::now() - notify::CATCH_UP),
            alarms_fired: Default::default(),
            alarm_plan: Default::default(),
        }
    }

    fn setup_signals(self: &Rc<Self>) {
        for toggle in &self.toggles {
            let weak = Rc::downgrade(self);
            toggle.connect_active_name_notify(move |t| {
                let Some(w) = weak.upgrade() else { return };
                if w.syncing.get() {
                    return;
                }
                let view = t
                    .active_name()
                    .and_then(|n| w.state.borrow().view_named(&n));
                if let Some(view) = view {
                    w.set_view(view);
                }
            });
        }
    }

    /// Buttons and menus run commands too: `win.run` takes a command line
    /// (e.g. `win.run::today`) and `win.palette` opens the command line.
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

        // the view menu, marked with the calendar view shown
        let calview = gio::SimpleAction::new_stateful(
            "calview",
            Some(glib::VariantTy::STRING),
            &self.state.borrow().calendar_view.name().to_variant(),
        );
        let weak = Rc::downgrade(self);
        calview.connect_activate(move |_, param| {
            let Some(w) = weak.upgrade() else { return };
            if let Some(view) = param.and_then(|p| p.str())
                && let Err(e) = w.execute(&format!("view {view}"))
            {
                w.toast(&e);
            }
        });
        self.window.add_action(&calview);

        let open = gio::SimpleAction::new("palette", None);
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, _| {
            if let Some(w) = weak.upgrade() {
                palette::open(&w, "");
            }
        });
        self.window.add_action(&open);
        // the command line with something already typed, e.g. "subtask "
        let prompt = gio::SimpleAction::new("prompt", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        prompt.connect_activate(move |_, arg| {
            if let (Some(w), Some(text)) = (weak.upgrade(), arg.and_then(|a| a.str())) {
                crate::details::close();
                palette::open(&w, text);
            }
        });
        self.window.add_action(&prompt);
    }

    /// Keys go through the bindings (see `run.rs` and `binds.conf`).
    fn setup_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| match weak.upgrade() {
            Some(w) => w.handle_key(key, mods),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);
    }

    fn today(&self) {
        {
            let mut s = self.state.borrow_mut();
            s.date = Utc::now().with_timezone(&s.local).date_naive();
            s.selected = None;
        }
        self.refresh();
        if self.state.borrow().view.is_grid() {
            self.grid.scroll_to_now_or_morning();
        }
    }

    fn step(&self, forward: bool) {
        {
            let mut s = self.state.borrow_mut();
            s.step(forward);
            s.selected = None;
        }
        self.refresh();
    }

    fn set_view(&self, view: ViewKind) {
        self.state.borrow_mut().view = view;
        self.refresh();
        self.focus_view();
    }

    fn move_day(&self, days: i64) {
        {
            let mut s = self.state.borrow_mut();
            let d = if days >= 0 {
                s.date + Days::new(days as u64)
            } else {
                s.date - Days::new(days.unsigned_abs())
            };
            s.date = d;
            if s.view == ViewKind::Month {
                s.selected = None;
            } else {
                s.select_first_on_date();
            }
        }
        self.refresh();
        self.grid.reveal_selected();
    }

    fn select_relative(&self, forward: bool) {
        // the task and journal lists are not by date: step through them
        let view = self.state.borrow().view;
        match view {
            ViewKind::Tasks => return self.tasks.step(forward),
            ViewKind::Journal => return self.journals.step(forward),
            _ => {}
        }
        self.state.borrow_mut().select_relative(forward);
        self.refresh();
        self.grid.reveal_selected();
    }

    fn show_details(&self) {
        let view = self.state.borrow().view;
        match view {
            ViewKind::Day | ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_) => {
                self.grid.show_selected_details()
            }
            ViewKind::Month => self.month.show_selected_details(),
            ViewKind::Year => {}
            ViewKind::Agenda => self.agenda.show_selected_details(),
            ViewKind::Invitations => self.invitations.show_selected_details(),
            ViewKind::Tasks | ViewKind::Journal => {
                let row = if view == ViewKind::Tasks {
                    self.tasks.selected_row()
                } else {
                    self.journals.selected_row()
                };
                if let (Some(row), Some(occ)) = (row, self.current_item()) {
                    let rect = gtk::gdk::Rectangle::new(0, 0, row.width(), row.height());
                    details::show(&row, rect, &occ, &self.state.borrow());
                }
            }
        }
    }

    /// Give the current view keyboard focus. Deferred until idle, as a view
    /// that was just switched to cannot take focus yet.
    fn focus_view(&self) {
        let view = self.state.borrow().view;
        let (grid, month, agenda, tasks, invitations, journals, year) = (
            self.grid.clone(),
            self.month.clone(),
            self.agenda.clone(),
            self.tasks.clone(),
            self.invitations.clone(),
            self.journals.clone(),
            self.year.clone(),
        );
        glib::idle_add_local_once(move || match view {
            ViewKind::Day | ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_) => grid.focus(),
            ViewKind::Month => month.focus(),
            ViewKind::Year => year.focus(),
            ViewKind::Agenda => agenda.focus(),
            ViewKind::Tasks => tasks.focus(),
            ViewKind::Invitations => invitations.focus(),
            ViewKind::Journal => journals.focus(),
        });
    }

    fn rebuild_calendar_list(self: &Rc<Self>) {
        while let Some(row) = self.calendar_list.row_at_index(0) {
            self.calendar_list.remove(&row);
        }
        let state = self.state.borrow();
        // the menu of each calendar shows on hover or focus, not as a column
        // of icons; the selection in lists stands out from a hovered row
        let mut css = String::from(
            "row:not(:hover):not(:focus-within) .calendar-more > button:not(:checked) { opacity: 0; }\n\
             .rical-list row:selected { background-color: alpha(@accent_bg_color, 0.16); }\n\
             .rical-list row:selected:hover { background-color: alpha(@accent_bg_color, 0.22); }\n\
             button.person-link { padding: 1px 4px; margin: -1px -4px; min-height: 0; font-weight: normal; }\n",
        );
        for (i, cal) in state.vdir.calendars.iter().enumerate() {
            let check = gtk::CheckButton::builder()
                .active(!state.hidden.contains(&cal.id))
                .valign(gtk::Align::Center)
                .build();
            let id = cal.id.clone();
            let weak = Rc::downgrade(self);
            check.connect_toggled(move |c| {
                let Some(w) = weak.upgrade() else { return };
                if w.syncing.get() {
                    return;
                }
                {
                    let mut s = w.state.borrow_mut();
                    if c.is_active() {
                        s.hidden.remove(&id);
                    } else {
                        s.hidden.insert(id.clone());
                    }
                    s.invalidate();
                }
                w.refresh();
            });
            // the checkbox wears the calendar's colour
            let c = state.color(&cal.id);
            check.add_css_class(&format!("cal-{i}"));
            css += &format!(
                ".cal-{i} check {{ border-color: #{r:02x}{g:02x}{b:02x}; }}\n\
                 .cal-{i} check:checked {{ background-color: #{r:02x}{g:02x}{b:02x}; \
                 color: {fg}; }}\n",
                r = c.r,
                g = c.g,
                b = c.b,
                fg = if crate::draw::contrasting(c).r == 0 {
                    "black"
                } else {
                    "white"
                },
            );
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            line.append(&check);
            line.append(
                &gtk::Label::builder()
                    .label(&cal.label)
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .build(),
            );
            let info = gtk::MenuButton::builder()
                .icon_name("view-more-symbolic")
                .tooltip_text("About this calendar")
                .valign(gtk::Align::Center)
                .css_classes(["flat", "circular", "dim-label", "calendar-more"])
                .popover(&calendar_info(cal, &state))
                .build();
            line.append(&info);
            let mut tip = cal.path.display().to_string();
            if i < 9 {
                tip += &format!("\nShow or hide: Alt+{}", i + 1);
            }
            let row = gtk::ListBoxRow::builder()
                .child(&line)
                .tooltip_text(tip)
                // the folder it is in (an account, say) heads its group
                .name(cal.id.rsplit_once('/').map_or("", |(parent, _)| parent))
                .build();
            self.calendar_list.append(&row);
        }
        CALENDAR_CSS.with(|p| p.load_from_string(&css));
    }

    fn sync_calendar_list(&self) {
        self.syncing.set(true);
        let state = self.state.borrow();
        for (i, cal) in state.vdir.calendars.iter().enumerate() {
            let check = self
                .calendar_list
                .row_at_index(i as i32)
                .and_then(|r| calendar_check(&r));
            if let Some(check) = check {
                check.set_active(!state.hidden.contains(&cal.id));
            }
        }
        self.syncing.set(false);
    }

    /// Bring every part of the window in line with the state.
    /// Hand freed memory back to the system once things settle. A view
    /// change frees thousands of occurrences, and glibc keeps that memory
    /// for itself unless asked, so the app grows to its largest view and
    /// stays there.
    fn trim_soon(&self) {
        if self.trim_pending.replace(true) {
            return;
        }
        let pending = self.trim_pending.clone();
        glib::timeout_add_local_once(Duration::from_secs(2), move || {
            pending.set(false);
            // SAFETY: malloc_trim only releases free memory; no Rust
            // object is touched
            unsafe { libc::malloc_trim(0) };
        });
    }

    pub fn refresh(&self) {
        self.trim_soon();
        details::close();
        self.side_tasks.refresh();
        self.syncing.set(true);
        let state = self.state.borrow();
        let view = state.view;
        let date = state.date;

        let (title, subtitle) = format::view_title(view, date, &state.visible_days());
        self.title.set_title(&title);
        self.title.set_subtitle(&subtitle);
        for toggle in &self.toggles {
            toggle.set_active_name(Some(view.toggle_name()));
        }
        let calendar_view = if view.is_calendar() {
            view
        } else {
            state.calendar_view
        };
        let (command, tip) = match view {
            ViewKind::Journal => ("note", "New journal entry (C)"),
            ViewKind::Tasks => ("new --kind todo", "New task (c)"),
            _ => ("new", "New event (c)"),
        };
        self.add
            .set_action_target_value(Some(&command.to_variant()));
        self.add.set_tooltip_text(Some(tip));
        if let Some(action) = self
            .window
            .lookup_action("calview")
            .and_downcast::<gio::SimpleAction>()
        {
            action.set_state(&calendar_view.name().to_variant());
        }
        for label in &self.calendar_labels {
            label.set_label(&calendar_view.label());
        }
        self.mini.refresh();
        drop(state);
        self.state.borrow_mut().calendar_view = calendar_view;

        match view {
            ViewKind::Day | ViewKind::Week | ViewKind::WorkWeek | ViewKind::Days(_) => {
                self.views.set_visible_child_name("grid");
                self.grid.refresh();
            }
            ViewKind::Year => {
                self.views.set_visible_child_name("year");
                self.year.refresh();
            }
            ViewKind::Month => {
                self.views.set_visible_child_name("month");
                self.month.refresh();
            }
            ViewKind::Agenda => {
                self.views.set_visible_child_name("agenda");
                self.agenda.refresh();
            }
            ViewKind::Tasks => {
                self.views.set_visible_child_name("tasks");
                self.tasks.refresh();
            }
            ViewKind::Invitations => {
                self.views.set_visible_child_name("invitations");
                self.invitations.refresh();
            }
            ViewKind::Journal => {
                self.views.set_visible_child_name("journal");
                self.journals.refresh();
            }
        }

        // when the grid shows different days, scroll to something useful:
        // the selected event, now if today is shown, or the first event
        let grid_days = state_days(&self.state.borrow());
        if grid_days != self.last_grid_days.get() {
            self.last_grid_days.set(grid_days);
            if grid_days.is_some() {
                // after allocation, so the scroller knows its size
                let grid = self.grid.clone();
                let has_selection = self.state.borrow().selected.is_some();
                // (at startup that can take a while: wait for a size)
                let mut tries = 0;
                glib::timeout_add_local(Duration::from_millis(50), move || {
                    tries += 1;
                    if grid.page_height() <= 0.0 && tries < 40 {
                        return glib::ControlFlow::Continue;
                    }
                    grid.scroll_to_now_or_morning();
                    if has_selection {
                        grid.reveal_selected();
                    }
                    glib::ControlFlow::Break
                });
            }
        }
        self.syncing.set(false);
    }

    pub fn present(&self) {
        self.window.present();
        self.focus_view();
    }

    fn reload_all(self: &Rc<Self>) {
        let result = self.state.borrow_mut().vdir.reload_all();
        if let Err(e) = result {
            warn!("reload failed: {e:#}");
        }
        self.state.borrow_mut().invalidate();
        self.watch();
        self.rebuild_calendar_list();
        self.refresh();
    }

    /// Watch the vdir root and each calendar folder, reloading changed items
    /// after a short debounce (vdirsyncer writes many files at once).
    fn watch(self: &Rc<Self>) {
        let mut monitors = Vec::new();
        let state = self.state.borrow();
        // every folder under the root: in calendars, items reload one by one;
        // anywhere else (a new calendar or account), everything reloads
        let dirs = state
            .vdir
            .dirs
            .iter()
            .map(|d| {
                (
                    d.clone(),
                    !state.vdir.calendars.iter().any(|c| &c.path == d),
                )
            })
            .collect::<Vec<_>>();
        for (dir, not_calendar) in dirs {
            let file = gio::File::for_path(&dir);
            let monitor = match file.monitor_directory(
                gio::FileMonitorFlags::WATCH_MOVES,
                None::<&gio::Cancellable>,
            ) {
                Ok(m) => m,
                Err(e) => {
                    warn!("cannot watch {dir:?}: {e}");
                    continue;
                }
            };
            let weak = Rc::downgrade(self);
            monitor.connect_changed(move |_, file, other, event| {
                let Some(w) = weak.upgrade() else { return };
                if matches!(
                    event,
                    gio::FileMonitorEvent::ChangesDoneHint
                        | gio::FileMonitorEvent::AttributeChanged
                ) {
                    return;
                }
                let paths = [Some(file), other]
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f.path());
                for path in paths {
                    let name = path.file_name().unwrap_or_default();
                    let is_item = path.extension().is_some_and(|e| e == "ics");
                    if name.to_string_lossy().starts_with('.') {
                        // temporary files of atomic writes (ours, vdirsyncer's):
                        // the rename to the .ics name is what counts
                    } else if not_calendar || name == "displayname" || name == "color" {
                        // a calendar was added or removed, or its metadata changed
                        w.reload_all_pending.set(true);
                    } else if is_item {
                        w.pending.borrow_mut().insert(path);
                    }
                }
                w.schedule_reload();
            });
            monitors.push(monitor);
        }
        drop(state);
        *self.monitors.borrow_mut() = monitors;
    }

    fn schedule_reload(self: &Rc<Self>) {
        if let Some(id) = self.debounce.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let Some(w) = weak.upgrade() else { return };
            w.debounce.borrow_mut().take();
            if w.reload_all_pending.take() {
                info!("reloading vdir");
                w.pending.borrow_mut().clear();
                w.reload_all();
                return;
            }
            let paths = std::mem::take(&mut *w.pending.borrow_mut());
            let mut changed = false;
            {
                let mut s = w.state.borrow_mut();
                for path in &paths {
                    changed |= s.vdir.reload(path);
                }
                if changed {
                    info!("reloaded {} item(s)", paths.len());
                    s.invalidate();
                }
            }
            if changed {
                w.refresh();
            }
        });
        *self.debounce.borrow_mut() = Some(id);
    }
}

/// The days the time grid shows, if it is the current view.
fn state_days(state: &crate::state::State) -> Option<(chrono::NaiveDate, usize)> {
    state.view.is_grid().then(|| {
        let days = state.visible_days();
        (days[0], days.len())
    })
}

thread_local! {
    /// Colours each sidebar calendar's checkbox; reloaded on rebuild.
    static CALENDAR_CSS: gtk::CssProvider = {
        let provider = gtk::CssProvider::new();
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        provider
    };
}

/// The show/hide checkbox of a sidebar calendar row.
fn calendar_check(row: &gtk::ListBoxRow) -> Option<gtk::CheckButton> {
    row.child()?.first_child()?.downcast().ok()
}

/// Details of a calendar: where it is on disk and what it holds.
fn calendar_info(cal: &rical_core::Calendar, state: &crate::state::State) -> gtk::Popover {
    let items = state
        .vdir
        .components()
        .filter(|(c, _)| c.id == cal.id)
        .map(|(_, comp)| comp.path().to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let grid = gtk::Grid::builder()
        .row_spacing(6)
        .column_spacing(12)
        .margin_top(6)
        .margin_bottom(6)
        .margin_start(6)
        .margin_end(6)
        .build();
    let color = state.color(&cal.id).to_string();
    let rows = [
        ("Name", cal.name.clone()),
        ("Folder", cal.id.clone()),
        ("Path", cal.path.display().to_string()),
        ("Items", items.to_string()),
        ("Colour", color),
    ];
    for (i, (key, value)) in rows.iter().enumerate() {
        grid.attach(
            &gtk::Label::builder()
                .label(*key)
                .css_classes(["dim-label"])
                .xalign(1.0)
                .build(),
            0,
            i as i32,
            1,
            1,
        );
        grid.attach(
            &gtk::Label::builder()
                .label(value)
                .selectable(true)
                // focusing a selectable label selects all of it
                .can_focus(false)
                .wrap(true)
                .max_width_chars(48)
                .xalign(0.0)
                .build(),
            1,
            i as i32,
            1,
            1,
        );
    }
    let open = gtk::Button::builder()
        .label("Open folder")
        .halign(gtk::Align::Start)
        .build();
    let uri = gio::File::for_path(&cal.path).uri().to_string();
    open.connect_clicked(move |b| {
        let window = b.root().and_downcast::<gtk::Window>();
        crate::details::open_uri(window.as_ref(), &uri);
    });
    grid.attach(&open, 1, rows.len() as i32, 1, 1);
    gtk::Popover::builder().child(&grid).build()
}

/// The view switcher: Calendar, Tasks, Invites. The Calendar part names
/// the calendar view ("Week ▾"): from elsewhere it goes back to it, and
/// when already there it opens a menu of calendar views.
fn view_toggles(state: &SharedState) -> (adw::ToggleGroup, gtk::Label) {
    let group = adw::ToggleGroup::builder().css_classes(["round"]).build();
    let label = gtk::Label::new(Some("Week"));
    let calendar = gtk::Box::builder()
        .spacing(4)
        .margin_start(6)
        .margin_end(2)
        .build();
    calendar.append(&label);
    calendar.append(&gtk::Image::from_icon_name("pan-down-symbolic"));
    group.add(
        adw::Toggle::builder()
            .name("calendar")
            .child(&calendar)
            .tooltip("Calendar; again for day, week, month… (d, w, m, a)")
            .build(),
    );
    for (name, text, tooltip) in [
        ("tasks", "Tasks", "Tasks (T)"),
        ("journal", "Journal", "Journal entries and notes (E)"),
        (
            "invitations",
            "Invites",
            "Invitations waiting for a reply (I)",
        ),
    ] {
        group.add(
            adw::Toggle::builder()
                .name(name)
                .label(text)
                .tooltip(tooltip)
                .build(),
        );
    }

    let menu = gio::Menu::new();
    for (text, view) in [
        ("_Day", "day"),
        ("_Week", "fullweek"),
        ("Wor_k week", "workweek"),
        ("_3 days", "3days"),
        ("_Month", "month"),
        ("_Year", "year"),
        ("_Agenda", "agenda"),
    ] {
        let item = gio::MenuItem::new(Some(text), None);
        item.set_action_and_target_value(Some("win.calview"), Some(&view.to_variant()));
        menu.append_item(&item);
    }
    let popover = gtk::PopoverMenu::from_model(Some(&menu));
    popover.set_parent(&group);
    popover.set_position(gtk::PositionType::Bottom);

    // a press on Calendar while it is already chosen opens the menu
    let click = gtk::GestureClick::new();
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (state, calendar) = (state.clone(), calendar.downgrade());
    click.connect_pressed(move |gesture, _, x, y| {
        let Some(button) = calendar.upgrade().and_then(|c| c.parent()) else {
            return;
        };
        let Some(group) = gesture.widget() else {
            return;
        };
        let Some(bounds) = button.compute_bounds(&group) else {
            return;
        };
        let inside = bounds.contains_point(&gtk::graphene::Point::new(x as f32, y as f32));
        if inside && state.borrow().view.is_calendar() {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                bounds.x() as i32,
                bounds.y() as i32,
                bounds.width() as i32,
                bounds.height() as i32,
            )));
            popover.popup();
        }
    });
    group.add_controller(click);
    (group, label)
}

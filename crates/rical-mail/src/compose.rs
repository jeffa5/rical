//! The window you write in: headers, a plain-text body that is wrapped and
//! quoted the way plain-text mail expects, attachments, and sending by
//! putting the message in rical-mail-queue's queue.

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

use adw::prelude::*;
use chrono::Local;
use gtk::{gdk, gio, glib};
use rical_maildir::{
    addr,
    compose::{self as lib, Draft},
    flags::Flags,
    store,
};
use rical_ui::{binds::Lookup, keys::key_of, palette};

use crate::{
    address::AddressEntry,
    cmds, editor,
    reader::kilobytes,
    run::{REGISTRY, StateValues},
    state::SharedState,
    window::Window,
};

thread_local! {
    /// The windows open, kept alive until they close.
    static OPEN: RefCell<Vec<Rc<Compose>>> = const { RefCell::new(Vec::new()) };
}

/// A folder for files a message needs on disk for a while.
pub fn scratch_dir(name: &str) -> PathBuf {
    let dir = rical_ui::app::state_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("scratch")
        .join(format!(
            "{name}-{}",
            store::unique_name().replace(['.', ':'], "-")
        ));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// Where a message the draft answers, or is kept as, is now: its flags may
/// have renamed it since.
fn locate(path: &Path) -> Option<PathBuf> {
    if path.exists() {
        return Some(path.to_owned());
    }
    let folder = path.parent()?.parent()?;
    let unique = path
        .file_name()?
        .to_string_lossy()
        .split(':')
        .next()?
        .to_owned();
    for sub in ["cur", "new"] {
        for e in std::fs::read_dir(folder.join(sub)).ok()?.flatten() {
            if e.file_name().to_string_lossy().starts_with(&unique) {
                return Some(e.path());
            }
        }
    }
    None
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A `mailto:` link as a draft's fields.
pub fn parse_mailto(uri: &str) -> Draft {
    let rest = uri.strip_prefix("mailto:").unwrap_or(uri);
    let (to, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut d = Draft {
        to: percent_decode(to),
        ..Draft::default()
    };
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let v = percent_decode(v);
        match k.to_lowercase().as_str() {
            "subject" => d.subject = v,
            "body" => d.body = v,
            "cc" => d.cc = v,
            "bcc" => d.bcc = v,
            "to" => {
                if !d.to.is_empty() {
                    d.to.push_str(", ");
                }
                d.to.push_str(&v);
            }
            "in-reply-to" => d.in_reply_to = Some(v.trim_matches(['<', '>']).to_owned()),
            _ => {}
        }
    }
    d
}

pub fn open_mailto(win: &Rc<Window>, uri: &str) {
    let mut d = parse_mailto(uri);
    d.from = win.new_draft().from;
    open(win, d, None);
}

pub fn open(win: &Rc<Window>, d: Draft, editing: Option<PathBuf>) {
    Compose::new(win, d, editing, false);
}

/// A reply: the cursor goes below the quote, or above it.
pub fn open_reply(win: &Rc<Window>, d: Draft, top: bool) {
    Compose::new(win, d, None, top);
}

pub struct Compose {
    pub window: adw::ApplicationWindow,
    main: Rc<Window>,
    state: SharedState,
    from_choice: gtk::DropDown,
    identities: Vec<String>,
    to: Rc<AddressEntry>,
    cc: Rc<AddressEntry>,
    bcc: Rc<AddressEntry>,
    cc_row: Vec<gtk::Widget>,
    bcc_row: Vec<gtk::Widget>,
    subject: gtk::Entry,
    body: gtk::TextView,
    guide: gtk::DrawingArea,
    attachments: RefCell<Vec<PathBuf>>,
    attach_box: gtk::FlowBox,
    status: gtk::Label,
    toasts: adw::ToastOverlay,
    /// What the message answers, and where the draft is kept.
    answering: RefCell<Option<PathBuf>>,
    saved_as: RefCell<Option<PathBuf>>,
    in_reply_to: RefCell<Option<String>>,
    references: RefCell<Vec<String>>,
    /// The text as it was when last saved or opened.
    clean: RefCell<String>,
    leaving: Cell<bool>,
    markdown: Cell<bool>,
    calendar: RefCell<Option<(String, String)>>,
    busy: Cell<bool>,
    pub history: RefCell<Vec<String>>,
    pending_keys: RefCell<Vec<rical_ui::binds::Key>>,
    me: RefCell<std::rc::Weak<Compose>>,
    /// Set while the text is changed here, not typed.
    quiet: Cell<bool>,
}

fn field_row(
    grid: &gtk::Grid,
    row: i32,
    label: &str,
    widget: &impl IsA<gtk::Widget>,
) -> Vec<gtk::Widget> {
    let l = gtk::Label::builder()
        .label(label)
        .xalign(1.0)
        .css_classes(["dim-label"])
        .build();
    grid.attach(&l, 0, row, 1, 1);
    grid.attach(widget, 1, row, 1, 1);
    vec![l.upcast(), widget.clone().upcast()]
}

impl Compose {
    fn new(main: &Rc<Window>, d: Draft, editing: Option<PathBuf>, top: bool) -> Rc<Compose> {
        let state = main.state.clone();
        let markdown = d.markdown || (editing.is_none() && state.borrow().opts.markdown);
        let (wrap, signature, identities) = {
            let s = state.borrow();
            (
                s.opts.wrap,
                signature_text(&s.opts.signature),
                s.opts.me.iter().map(|m| m.display()).collect::<Vec<_>>(),
            )
        };
        let from_choice =
            gtk::DropDown::from_strings(&identities.iter().map(String::as_str).collect::<Vec<_>>());
        let to = AddressEntry::new(state.clone(), "To");
        let cc = AddressEntry::new(state.clone(), "Cc");
        let bcc = AddressEntry::new(state.clone(), "Bcc");
        let subject = gtk::Entry::builder().hexpand(true).build();
        subject.update_property(&[gtk::accessible::Property::Label("Subject")]);

        let grid = gtk::Grid::builder()
            .column_spacing(10)
            .row_spacing(6)
            .margin_start(14)
            .margin_end(14)
            .margin_top(10)
            .margin_bottom(6)
            .build();
        let mut row = 0;
        if identities.len() > 1 {
            field_row(&grid, row, "From", &from_choice);
            row += 1;
        }
        field_row(&grid, row, "To", &to.entry);
        row += 1;
        let cc_row = field_row(&grid, row, "Cc", &cc.entry);
        row += 1;
        let bcc_row = field_row(&grid, row, "Bcc", &bcc.entry);
        row += 1;
        field_row(&grid, row, "Subject", &subject);

        let body = gtk::TextView::builder()
            .monospace(true)
            .wrap_mode(gtk::WrapMode::WordChar)
            .accepts_tab(true)
            .left_margin(14)
            .right_margin(14)
            .top_margin(10)
            .bottom_margin(10)
            .vexpand(true)
            .css_classes(["rical-body"])
            .build();
        body.set_monospace(!state.borrow().opts.proportional);
        body.update_property(&[gtk::accessible::Property::Label("Message")]);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&body)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();
        // a line down the page at the wrap column
        let guide = gtk::DrawingArea::builder()
            .can_target(false)
            .hexpand(true)
            .vexpand(true)
            .build();
        let overlay = gtk::Overlay::builder().child(&scroll).build();
        overlay.add_overlay(&guide);

        let attach_box = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .max_children_per_line(6)
            .margin_start(14)
            .margin_end(14)
            .margin_bottom(4)
            .visible(false)
            .build();
        let status = gtk::Label::builder()
            .xalign(0.0)
            .margin_start(14)
            .margin_end(14)
            .margin_top(4)
            .margin_bottom(4)
            .css_classes(["dim-label", "caption"])
            .build();

        let send = gtk::Button::builder()
            .child(
                &adw::ButtonContent::builder()
                    .label("Send")
                    .icon_name("mail-send-symbolic")
                    .build(),
            )
            .css_classes(["suggested-action"])
            .tooltip_text("Queue it to be sent (Ctrl+Enter)")
            .action_name("win.run")
            .action_target(&"send".to_variant())
            .build();
        let save = gtk::Button::builder()
            .icon_name("document-save-symbolic")
            .tooltip_text("Keep as a draft (Ctrl+S)")
            .action_name("win.run")
            .action_target(&"save".to_variant())
            .build();
        let attach = gtk::Button::builder()
            .icon_name("mail-attachment-symbolic")
            .tooltip_text("Attach a file")
            .action_name("win.run")
            .action_target(&"attach".to_variant())
            .build();
        let menu = gio::Menu::new();
        let fields = gio::Menu::new();
        fields.append(Some("Add _Cc"), Some("win.run::field cc"));
        fields.append(Some("Add _Bcc"), Some("win.run::field bcc"));
        menu.append_section(None, &fields);
        let text = gio::Menu::new();
        text.append(Some("_Wrap Paragraph"), Some("win.run::reflow"));
        text.append(Some("Wrap _Everything"), Some("win.run::reflow all"));
        text.append(Some("_Quote"), Some("win.run::quote"));
        text.append(Some("_Add Signature"), Some("win.run::signature"));
        text.append(Some("Edit in My _Editor"), Some("win.run::edit-external"));
        text.append(Some("_Markdown"), Some("win.run::markdown toggle"));
        text.append(Some("_Bold"), Some("win.run::format bold"));
        text.append(Some("_Italic"), Some("win.run::format italic"));
        text.append(Some("Insert _Link"), Some("win.run::format link"));
        menu.append_section(None, &text);
        let more = gio::Menu::new();
        more.append(Some("_Command Line"), Some("win.palette"));
        more.append(Some("_Discard"), Some("win.run::discard"));
        menu.append_section(None, &more);
        let menu_button = gtk::MenuButton::builder()
            .icon_name("view-more-symbolic")
            .menu_model(&menu)
            .build();
        let title = adw::WindowTitle::new("New message", "");
        let header = adw::HeaderBar::builder().title_widget(&title).build();
        header.pack_start(&send);
        header.pack_start(&save);
        header.pack_start(&attach);
        header.pack_end(&menu_button);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&grid);
        column.append(&attach_box);
        column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        column.append(&overlay);
        column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        column.append(&status);
        let view = adw::ToolbarView::new();
        view.add_top_bar(&header);
        view.set_content(Some(&column));
        let toasts = adw::ToastOverlay::new();
        toasts.set_child(Some(&view));
        let window = adw::ApplicationWindow::builder()
            .application(&main.app)
            .title("New message")
            .default_width(820)
            .default_height(700)
            .content(&toasts)
            .build();

        let this = Rc::new(Compose {
            window,
            main: main.clone(),
            state,
            from_choice,
            identities,
            to,
            cc,
            bcc,
            cc_row,
            bcc_row,
            subject,
            body,
            guide,
            attachments: RefCell::new(d.attachments.clone()),
            attach_box,
            status,
            toasts,
            answering: RefCell::new(d.answering.clone()),
            saved_as: RefCell::new(editing),
            in_reply_to: RefCell::new(d.in_reply_to.clone()),
            references: RefCell::new(d.references.clone()),
            clean: RefCell::default(),
            leaving: Cell::new(false),
            markdown: Cell::new(markdown),
            calendar: RefCell::new(d.calendar.clone()),
            busy: Cell::new(false),
            history: Default::default(),
            pending_keys: Default::default(),
            me: Default::default(),
            quiet: Cell::new(false),
        });
        *this.me.borrow_mut() = Rc::downgrade(&this);
        this.fill(&d, &signature, top);
        this.refresh_attachments();
        this.setup(&title, wrap);
        OPEN.with(|o| o.borrow_mut().push(this.clone()));
        *this.clean.borrow_mut() = this.snapshot();
        this.window.present();
        // where to start: the first empty thing
        if d.to.trim().is_empty() {
            this.to.entry.grab_focus();
        } else if d.subject.trim().is_empty() {
            this.subject.grab_focus();
        } else {
            this.body.grab_focus();
        }
        this
    }

    /// Put a draft in the fields.
    fn fill(&self, d: &Draft, signature: &str, top: bool) {
        self.quiet.set(true);
        if let Some(i) = self.identities.iter().position(|m| {
            addr::parse_list(m)
                .first()
                .map(|a| a.address.to_lowercase())
                == addr::parse_list(&d.from)
                    .first()
                    .map(|a| a.address.to_lowercase())
        }) {
            self.from_choice.set_selected(i as u32);
        }
        self.to.set(&d.to);
        self.cc.set(&d.cc);
        self.bcc.set(&d.bcc);
        self.subject.set_text(&d.subject);
        let cc_on = !d.cc.trim().is_empty();
        let bcc_on = !d.bcc.trim().is_empty();
        for w in &self.cc_row {
            w.set_visible(cc_on);
        }
        for w in &self.bcc_row {
            w.set_visible(bcc_on);
        }
        let reply = d.in_reply_to.is_some() && !d.body.trim().is_empty();
        let has_sig = d.body.contains("\n-- \n");
        let sig = if signature.is_empty() || has_sig {
            String::new()
        } else {
            format!("\n-- \n{signature}")
        };
        let (text, cursor): (String, usize) = if reply && top {
            (format!("\n\n{}\n{sig}", d.body.trim_end()), 0)
        } else if reply {
            // the cursor on the empty line under the quote
            let quoted = format!("{}\n\n", d.body.trim_end());
            let at = quoted.chars().count();
            (format!("{quoted}{sig}"), at)
        } else if d.body.trim().is_empty() {
            (
                if sig.is_empty() {
                    String::new()
                } else {
                    format!("\n{sig}")
                },
                0,
            )
        } else if d.body.starts_with("\n\n---------- Forwarded") {
            (format!("{}{sig}", d.body), 0)
        } else {
            // a draft or a link's text: carry on where it ends, above the signature
            let body = d.body.trim_end_matches('\n');
            let at = body.find("\n-- \n").unwrap_or(body.len());
            let cursor = body[..at].chars().count();
            (format!("{body}\n{sig}"), cursor)
        };
        let buf = self.body.buffer();
        buf.set_text(&text);
        buf.place_cursor(&buf.iter_at_offset(cursor as i32));
        self.quiet.set(false);
    }

    fn setup(self: &Rc<Self>, title: &adw::WindowTitle, wrap: usize) {
        // title and status follow the text
        let weak = Rc::downgrade(self);
        let title = title.clone();
        let refresh = Rc::new(move || {
            let Some(c) = weak.upgrade() else { return };
            let s = c.subject.text();
            let t = if s.trim().is_empty() {
                "New message".to_owned()
            } else {
                s.to_string()
            };
            title.set_title(&t);
            c.window.set_title(Some(&t));
            c.update_status();
        });
        let r = refresh.clone();
        self.subject.connect_changed(move |_| r());
        let r = refresh.clone();
        self.body.buffer().connect_changed(move |_| r());
        let weak = Rc::downgrade(self);
        let pending = Rc::new(Cell::new(false));
        self.body.buffer().connect_changed(move |_| {
            if pending.replace(true) {
                return;
            }
            let (weak, pending) = (weak.clone(), pending.clone());
            glib::idle_add_local_once(move || {
                pending.set(false);
                if let Some(c) = weak.upgrade() {
                    c.tag_text();
                }
            });
        });
        let r = refresh.clone();
        self.body
            .buffer()
            .connect_notify_local(Some("cursor-position"), move |_, _| r());
        refresh();

        // the guide
        let weak = Rc::downgrade(self);
        self.guide.set_draw_func(move |_, cr, _w, h| {
            let Some(c) = weak.upgrade() else { return };
            let wrap = c.state.borrow().opts.wrap;
            let layout = c.body.create_pango_layout(Some("0"));
            let cw = layout.pixel_size().0 as f64;
            let x = c.body.left_margin() as f64 + cw * wrap as f64 + 0.5;
            let fg = c.body.color();
            cr.set_source_rgba(fg.red() as f64, fg.green() as f64, fg.blue() as f64, 0.24);
            cr.set_line_width(1.0);
            cr.move_to(x, 0.0);
            cr.line_to(x, h as f64);
            let _ = cr.stroke();
        });
        let guide = self.guide.clone();
        self.body
            .connect_notify_local(None, move |_, _| guide.queue_draw());

        self.setup_body_keys(wrap);
        self.setup_actions();
        self.autosave();
        self.accept_drops();

        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |w| {
            let Some(c) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if c.leaving.get() || !c.is_dirty() {
                c.closed();
                return glib::Propagation::Proceed;
            }
            let _ = w;
            c.ask_leave();
            glib::Propagation::Stop
        });
    }

    fn closed(&self) {
        let me = self.me.borrow().upgrade();
        OPEN.with(|o| {
            o.borrow_mut()
                .retain(|c| me.as_ref().is_none_or(|m| !Rc::ptr_eq(c, m)))
        });
    }

    /// Typing in the body: wrapping at the margin, and Enter that carries
    /// quote marks and list markers on.
    fn setup_body_keys(self: &Rc<Self>, _wrap: usize) {
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| {
            let Some(c) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if mods.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK) {
                return glib::Propagation::Proceed;
            }
            let buf = c.body.buffer();
            if buf.has_selection() {
                return glib::Propagation::Proceed;
            }
            match key {
                gdk::Key::Return | gdk::Key::KP_Enter => {
                    if c.on_enter() {
                        glib::Propagation::Stop
                    } else {
                        glib::Propagation::Proceed
                    }
                }
                gdk::Key::Tab => {
                    buf.insert_at_cursor("    ");
                    glib::Propagation::Stop
                }
                _ => match key.to_unicode() {
                    Some(ch) if !ch.is_control() && c.state.borrow().opts.auto_wrap => {
                        if c.on_type(ch) {
                            glib::Propagation::Stop
                        } else {
                            glib::Propagation::Proceed
                        }
                    }
                    _ => glib::Propagation::Proceed,
                },
            }
        });
        self.body.add_controller(keys);
    }

    /// The line the cursor is on, up to the cursor, and whether the cursor
    /// is at its end.
    fn line_before_cursor(&self) -> (String, bool) {
        let buf = self.body.buffer();
        let cur = buf.iter_at_mark(&buf.get_insert());
        let mut start = cur;
        start.set_line_offset(0);
        let before = buf.text(&start, &cur, false).to_string();
        (before, cur.ends_line())
    }

    fn on_enter(&self) -> bool {
        let (before, _) = self.line_before_cursor();
        let buf = self.body.buffer();
        buf.begin_user_action();
        let handled = match editor::enter(&before) {
            editor::Enter::Plain => false,
            editor::Enter::Continue(prefix) => {
                buf.insert_at_cursor(&format!("\n{prefix}"));
                true
            }
            editor::Enter::Clear(n) => {
                let mut end = buf.iter_at_mark(&buf.get_insert());
                let mut start = end;
                start.backward_chars(n as i32);
                buf.delete(&mut start, &mut end);
                true
            }
        };
        buf.end_user_action();
        if handled {
            self.scroll_to_cursor();
        }
        handled
    }

    fn on_type(&self, c: char) -> bool {
        let (before, at_end) = self.line_before_cursor();
        if !at_end {
            return false;
        }
        let wrap = self.state.borrow().opts.wrap;
        let buf = self.body.buffer();
        match editor::wrap(&before, c, wrap) {
            editor::Wrap::No => false,
            editor::Wrap::Newline => {
                let prefix = editor::wrap_prefix(&before);
                buf.begin_user_action();
                buf.insert_at_cursor(&format!("\n{prefix}"));
                buf.end_user_action();
                self.scroll_to_cursor();
                true
            }
            editor::Wrap::BreakAt(i) => {
                let prefix = editor::wrap_prefix(&before);
                let cur = buf.iter_at_mark(&buf.get_insert());
                let mut line = cur;
                line.set_line_offset(0);
                // the break: the space (and any before it) becomes a new line
                let mut from = line;
                from.forward_chars(i as i32);
                let mut to = from;
                to.forward_char();
                let chars: Vec<char> = before.chars().collect();
                let mut k = i;
                while k > 0 && chars[k - 1] == ' ' {
                    from.backward_char();
                    k -= 1;
                }
                buf.begin_user_action();
                buf.delete(&mut from, &mut to);
                buf.insert(&mut from, &format!("\n{prefix}"));
                buf.end_user_action();
                // the character itself is typed as usual
                false
            }
        }
    }

    fn scroll_to_cursor(&self) {
        let buf = self.body.buffer();
        self.body.scroll_mark_onscreen(&buf.get_insert());
    }

    fn collect(&self) -> Draft {
        let from = self
            .identities
            .get(self.from_choice.selected() as usize)
            .cloned()
            .unwrap_or_default();
        Draft {
            from,
            to: self.to.entry.text().to_string(),
            cc: self.cc.entry.text().to_string(),
            bcc: self.bcc.entry.text().to_string(),
            subject: self.subject.text().to_string(),
            body: self.body_text(),
            in_reply_to: self.in_reply_to.borrow().clone(),
            references: self.references.borrow().clone(),
            attachments: self.attachments.borrow().clone(),
            answering: self.answering.borrow().clone(),
            markdown: self.markdown.get(),
            calendar: self.calendar.borrow().clone(),
        }
    }

    fn body_text(&self) -> String {
        let buf = self.body.buffer();
        buf.text(&buf.start_iter(), &buf.end_iter(), false)
            .to_string()
    }

    fn snapshot(&self) -> String {
        let d = self.collect();
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}\n{:?}\n{}",
            d.from, d.to, d.cc, d.bcc, d.subject, d.body, d.attachments, d.markdown
        )
    }

    fn is_dirty(&self) -> bool {
        *self.clean.borrow() != self.snapshot()
    }

    fn update_status(&self) {
        let wrap = self.state.borrow().opts.wrap;
        let text = self.body_text();
        let (words, lines, _longest, over) = editor::measure(&text, wrap);
        let buf = self.body.buffer();
        let cur = buf.iter_at_mark(&buf.get_insert());
        let mut s = format!(
            "Line {}, column {} · {words} word{} · {lines} line{} · wrapped at {wrap}",
            cur.line() + 1,
            cur.line_offset() + 1,
            if words == 1 { "" } else { "s" },
            if lines == 1 { "" } else { "s" },
        );
        if self.markdown.get() {
            s += " · Markdown";
        }
        if over > 0 {
            s += &format!(
                " · {over} line{} longer (Alt+Q wraps)",
                if over == 1 { "" } else { "s" }
            );
            self.status.add_css_class("warning");
        } else {
            self.status.remove_css_class("warning");
        }
        self.status.set_label(&s);
    }

    pub fn toast(&self, text: &str) {
        self.toasts.add_toast(
            adw::Toast::builder()
                .title(text)
                .priority(adw::ToastPriority::High)
                .build(),
        );
    }

    fn refresh_attachments(self: &Rc<Self>) {
        while let Some(c) = self.attach_box.first_child() {
            self.attach_box.remove(&c);
        }
        let files = self.attachments.borrow().clone();
        self.attach_box.set_visible(!files.is_empty());
        for f in files {
            let name = f
                .file_name()
                .map_or(String::new(), |n| n.to_string_lossy().into_owned());
            let size = std::fs::metadata(&f).map_or(0, |m| m.len() as usize);
            let b = gtk::Box::builder().spacing(6).build();
            b.add_css_class("card");
            b.append(
                &gtk::Image::builder()
                    .icon_name("mail-attachment-symbolic")
                    .margin_start(8)
                    .build(),
            );
            b.append(
                &gtk::Label::builder()
                    .label(format!("{name} · {}", kilobytes(size)))
                    .margin_top(4)
                    .margin_bottom(4)
                    .build(),
            );
            let x = gtk::Button::builder()
                .icon_name("window-close-symbolic")
                .css_classes(["flat", "circular"])
                .tooltip_text("Remove")
                .action_name("win.run")
                .action_target(&format!("detach {}", rical_ui::command::quote(&name)).to_variant())
                .build();
            b.append(&x);
            self.attach_box.append(&b);
        }
    }

    // ---- commands ----

    fn setup_actions(self: &Rc<Self>) {
        let run = gio::SimpleAction::new("run", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(self);
        run.connect_activate(move |_, param| {
            let Some(c) = weak.upgrade() else { return };
            if let Some(line) = param.and_then(|p| p.str())
                && let Err(e) = c.execute(line)
            {
                c.toast(&e);
            }
        });
        self.window.add_action(&run);
        let open = gio::SimpleAction::new("palette", None);
        let weak = Rc::downgrade(self);
        open.connect_activate(move |_, _| {
            if let Some(c) = weak.upgrade() {
                palette::open(&c, "");
            }
        });
        self.window.add_action(&open);
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, mods| match weak.upgrade() {
            Some(c) => c.handle_key(key, mods),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);
    }

    /// Keys here are chords: plain keys are typing.
    fn handle_key(self: &Rc<Self>, keyval: gdk::Key, mods: gdk::ModifierType) -> glib::Propagation {
        use glib::Propagation::{Proceed, Stop};
        if self.window.visible_dialog().is_some() || self.busy.get() {
            return Proceed;
        }
        let Some(key) = key_of(keyval, mods) else {
            return Proceed;
        };
        if !(key.ctrl || key.alt || key.name.starts_with('f') && key.name.len() > 1) {
            self.pending_keys.borrow_mut().clear();
            return Proceed;
        }
        let mut pending = self.pending_keys.borrow_mut();
        pending.clear();
        pending.push(key);
        let found = self
            .main
            .binds
            .lookup("write", &pending)
            .or_else_global(|| self.main.binds.lookup("global", &pending));
        pending.clear();
        drop(pending);
        match found {
            Lookup::Found(action) => {
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
            _ => Proceed,
        }
    }

    pub fn execute(self: &Rc<Self>, line: &str) -> Result<(), String> {
        let p = REGISTRY
            .parse(line, cmds::WRITE)
            .map_err(|e| e.to_string())?;
        match p.spec {
            "send" => self.send(false)?,
            "save" => {
                self.save()?;
                self.toast("Kept as a draft");
            }
            "discard" => self.discard(),
            "attach" => match p.arg(0).map(str::trim).filter(|f| !f.is_empty()) {
                Some(f) => {
                    let path =
                        rical_ui::config::expand(f, &std::env::current_dir().unwrap_or_default());
                    self.add_attachment(path)?;
                }
                None => self.choose_attachment(),
            },
            "detach" => {
                let mut files = self.attachments.borrow_mut();
                let at = match p.arg(0).map(str::trim).filter(|f| !f.is_empty()) {
                    Some(n) => files.iter().position(|f| {
                        f.file_name().is_some_and(|x| {
                            x.to_string_lossy()
                                .to_lowercase()
                                .contains(&n.to_lowercase())
                        })
                    }),
                    None => files.len().checked_sub(1),
                };
                match at {
                    Some(i) => {
                        files.remove(i);
                    }
                    None => return Err("nothing to remove".into()),
                }
                drop(files);
                self.refresh_attachments();
            }
            "field" => self.go_to(p.arg(0).unwrap_or("to")),
            "from" => {
                let want = p.arg(0).unwrap_or("").to_lowercase();
                let i = self
                    .identities
                    .iter()
                    .position(|m| m.to_lowercase().contains(&want))
                    .ok_or(format!("none of your addresses matches “{want}”"))?;
                self.from_choice.set_selected(i as u32);
            }
            "edit-external" => self.edit_external()?,
            "reflow" => self.reflow(p.arg(0) == Some("all")),
            "quote" => self.quote(true),
            "unquote" => self.quote(false),
            "signature" => {
                let sig = signature_text(&self.state.borrow().opts.signature);
                if sig.is_empty() {
                    return Err("set `signature = …` in the settings first".into());
                }
                let buf = self.body.buffer();
                buf.insert(&mut buf.end_iter(), &format!("\n-- \n{sig}"));
            }
            "markdown" => {
                let on = match p.arg(0) {
                    Some("on") => true,
                    Some("off") => false,
                    _ => !self.markdown.get(),
                };
                self.markdown.set(on);
                self.tag_text();
                self.update_status();
                self.toast(if on {
                    "Markdown: sent as written, with an HTML version for those who show it"
                } else {
                    "Plain text only"
                });
            }
            "format" => self.format(p.arg(0).unwrap_or("bold"))?,
            "count" => {
                let wrap = self.state.borrow().opts.wrap;
                let (w, l, longest, over) = editor::measure(&self.body_text(), wrap);
                self.toast(&format!(
                    "{w} words, {l} lines, longest {longest}, {over} past {wrap}"
                ));
            }
            "settings" => crate::prefs::show(&self.main),
            "about" => self.main.execute("about")?,
            "help" => {
                rical_ui::help::dialog(&cmds::HELP, &self.main.binds, "write")
                    .present(Some(&self.window));
            }
            "quit" => self.main.app.quit(),
            other => return Err(format!("“{other}” is for the list of mail")),
        }
        Ok(())
    }

    fn go_to(&self, field: &str) {
        match field {
            "cc" => {
                for w in &self.cc_row {
                    w.set_visible(true);
                }
                self.cc.entry.grab_focus();
            }
            "bcc" => {
                for w in &self.bcc_row {
                    w.set_visible(true);
                }
                self.bcc.entry.grab_focus();
            }
            "subject" => {
                self.subject.grab_focus();
            }
            "body" => {
                self.body.grab_focus();
            }
            _ => {
                self.to.entry.grab_focus();
            }
        }
    }

    fn add_attachment(self: &Rc<Self>, path: PathBuf) -> Result<(), String> {
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        if meta.len() > 25 * 1024 * 1024 {
            self.toast("That is over 25 MB: many mail servers will refuse it");
        }
        self.attachments.borrow_mut().push(path);
        self.refresh_attachments();
        Ok(())
    }

    fn choose_attachment(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder().title("Attach files").build();
        let weak = Rc::downgrade(self);
        dialog.open_multiple(Some(&self.window), None::<&gio::Cancellable>, move |r| {
            let (Some(c), Ok(files)) = (weak.upgrade(), r) else {
                return;
            };
            for i in 0..files.n_items() {
                if let Some(f) = files.item(i).and_downcast::<gio::File>()
                    && let Some(p) = f.path()
                    && let Err(e) = c.add_attachment(p)
                {
                    c.toast(&e);
                }
            }
        });
    }

    // ---- editing text ----

    /// The lines the selection touches (or the cursor's line), as offsets.
    fn selected_lines(&self) -> (i32, i32) {
        let buf = self.body.buffer();
        let (a, b) = match buf.selection_bounds() {
            Some((a, b)) => (a, b),
            None => {
                let c = buf.iter_at_mark(&buf.get_insert());
                (c, c)
            }
        };
        let last = if b.starts_line() && b != a {
            b.line() - 1
        } else {
            b.line()
        };
        (a.line(), last.max(a.line()))
    }

    fn quote(&self, add: bool) {
        let buf = self.body.buffer();
        let (first, last) = self.selected_lines();
        buf.begin_user_action();
        for line in first..=last {
            let Some(mut start) = buf.iter_at_line(line) else {
                continue;
            };
            let mut end = start;
            end.forward_to_line_end();
            let text = buf.text(&start, &end, false).to_string();
            if add {
                let new = if text.is_empty() {
                    ">".to_owned()
                } else if text.starts_with('>') {
                    format!(">{text}")
                } else {
                    format!("> {text}")
                };
                buf.delete(&mut start, &mut end);
                buf.insert(&mut start, &new);
            } else {
                let cut = if text.starts_with("> ") {
                    2
                } else if text.starts_with('>') {
                    1
                } else {
                    0
                };
                if cut > 0 {
                    let mut to = start;
                    to.forward_chars(cut);
                    buf.delete(&mut start, &mut to);
                }
            }
        }
        buf.end_user_action();
    }

    /// Wrap the paragraph at the cursor, or everything.
    fn reflow(&self, all: bool) {
        let wrap = self.state.borrow().opts.wrap;
        let buf = self.body.buffer();
        if all {
            let text = self.body_text();
            let new = lib::reflow(&text, wrap);
            if new != text {
                let cursor = buf.iter_at_mark(&buf.get_insert()).line();
                buf.begin_user_action();
                buf.set_text(&new);
                buf.end_user_action();
                if let Some(it) = buf.iter_at_line(cursor.min(buf.line_count() - 1)) {
                    buf.place_cursor(&it);
                }
            }
            return;
        }
        let cur = buf.iter_at_mark(&buf.get_insert());
        let line_text = |n: i32| -> Option<String> {
            let a = buf.iter_at_line(n)?;
            let mut b = a;
            b.forward_to_line_end();
            Some(buf.text(&a, &b, false).to_string())
        };
        let here = cur.line();
        let Some(me) = line_text(here) else { return };
        if me.trim().is_empty() || me == "-- " {
            return;
        }
        let quote_of = |s: &str| editor::lead(s).quote;
        let q = quote_of(&me);
        let same = |s: &str| !s.trim().is_empty() && s != "-- " && quote_of(s) == q;
        let mut first = here;
        while first > 0 && line_text(first - 1).is_some_and(|s| same(&s)) {
            // a list item starts its own paragraph
            if editor::lead(&line_text(first).unwrap_or_default())
                .marker
                .is_some()
            {
                break;
            }
            first -= 1;
        }
        let mut last = here;
        while line_text(last + 1).is_some_and(|s| same(&s) && editor::lead(&s).marker.is_none()) {
            last += 1;
        }
        let (Some(mut start), Some(mut end)) = (buf.iter_at_line(first), buf.iter_at_line(last))
        else {
            return;
        };
        end.forward_to_line_end();
        let text = buf.text(&start, &end, false).to_string();
        let new = lib::reflow(&text, wrap);
        if new != text {
            buf.begin_user_action();
            buf.delete(&mut start, &mut end);
            buf.insert(&mut start, &new);
            buf.end_user_action();
        }
        // after the paragraph
        let at = buf.iter_at_offset(start.offset());
        buf.place_cursor(&at);
    }

    /// Edit the message in the user's own editor and read it back.
    fn edit_external(self: &Rc<Self>) -> Result<(), String> {
        let (editor_cmd, file) = {
            let s = self.state.borrow();
            let cmd = s.opts.editor.clone().or_else(|| {
                let ed = std::env::var("VISUAL")
                    .ok()
                    .or_else(|| std::env::var("EDITOR").ok())?;
                let term = std::env::var("TERMINAL").ok()?;
                Some(format!("{term} -e {ed}"))
            });
            (
                cmd.ok_or("set `editor = foot nvim` (a terminal and your editor) in the settings, or $TERMINAL and $EDITOR")?,
                scratch_dir("edit").join("message.txt"),
            )
        };
        let d = self.collect();
        let text = format!(
            "From: {}\nTo: {}\nCc: {}\nBcc: {}\nSubject: {}\n\n{}",
            d.from, d.to, d.cc, d.bcc, d.subject, d.body
        );
        std::fs::write(&file, text).map_err(|e| e.to_string())?;
        let proc = gio::Subprocess::newv(
            &[
                std::ffi::OsStr::new("sh"),
                std::ffi::OsStr::new("-c"),
                std::ffi::OsStr::new(&format!("{editor_cmd} \"$1\"")),
                std::ffi::OsStr::new("sh"),
                file.as_os_str(),
            ],
            gio::SubprocessFlags::NONE,
        )
        .map_err(|e| format!("{editor_cmd}: {e}"))?;
        self.busy.set(true);
        self.body.set_sensitive(false);
        self.toast("Editing in your editor; finish there to carry on");
        let weak = Rc::downgrade(self);
        proc.wait_async(None::<&gio::Cancellable>, move |_| {
            let Some(c) = weak.upgrade() else { return };
            c.busy.set(false);
            c.body.set_sensitive(true);
            match std::fs::read_to_string(&file) {
                Ok(t) => c.read_back(&t),
                Err(e) => c.toast(&format!("{}: {e}", file.display())),
            }
            let _ = std::fs::remove_file(&file);
            c.window.present();
            c.body.grab_focus();
        });
        Ok(())
    }

    fn read_back(&self, text: &str) {
        let (head, body) = text.split_once("\n\n").unwrap_or((text, ""));
        self.quiet.set(true);
        for line in head.lines() {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let v = v.trim();
            match k.trim().to_lowercase().as_str() {
                "to" => self.to.set(v),
                "cc" => {
                    self.cc.set(v);
                    for w in &self.cc_row {
                        w.set_visible(!v.is_empty());
                    }
                }
                "bcc" => {
                    self.bcc.set(v);
                    for w in &self.bcc_row {
                        w.set_visible(!v.is_empty());
                    }
                }
                "subject" => self.subject.set_text(v),
                "from" => {
                    if let Some(i) = self
                        .identities
                        .iter()
                        .position(|m| m.eq_ignore_ascii_case(v))
                    {
                        self.from_choice.set_selected(i as u32);
                    }
                }
                _ => {}
            }
        }
        self.body.buffer().set_text(body);
        self.quiet.set(false);
    }

    // ---- sending, keeping, leaving ----

    /// The things worth a second look before it goes.
    fn warnings(&self, d: &Draft) -> Vec<String> {
        let mut w = Vec::new();
        if d.subject.trim().is_empty() {
            w.push("It has no subject.".to_owned());
        }
        let wrote_attach = d.body.lines().filter(|l| !l.starts_with('>')).any(|l| {
            let l = l.to_lowercase();
            l.contains("attached") || l.contains("attachment") || l.contains("enclosed")
        });
        if wrote_attach && d.attachments.is_empty() {
            w.push("It mentions an attachment, and has none.".to_owned());
        }
        let n = addr::parse_list(&format!("{}, {}, {}", d.to, d.cc, d.bcc)).len();
        if n > 20 {
            w.push(format!("It goes to {n} people."));
        }
        w
    }

    fn send(self: &Rc<Self>, confirmed: bool) -> Result<(), String> {
        let d = self.collect();
        // what is wrong with it comes first, in plain words
        d.to_bytes(Local::now()).map_err(|e| format!("{e:#}"))?;
        let warnings = self.warnings(&d);
        if !confirmed && !warnings.is_empty() {
            let dialog = adw::AlertDialog::builder()
                .heading("Send anyway?")
                .body(warnings.join("\n"))
                .build();
            dialog.add_responses(&[("cancel", "_Keep Writing"), ("send", "_Send")]);
            dialog.set_default_response(Some("cancel"));
            dialog.set_close_response("cancel");
            let weak = Rc::downgrade(self);
            dialog.connect_response(None, move |_, r| {
                if r == "send"
                    && let Some(c) = weak.upgrade()
                    && let Err(e) = c.send(true)
                {
                    c.toast(&e);
                }
            });
            dialog.present(Some(&self.window));
            return Ok(());
        }
        let bytes = d.to_bytes(Local::now()).map_err(|e| format!("{e:#}"))?;
        let queue_dir = self.state.borrow().opts.queue.clone();
        let queue =
            rical_mail_queue::queue::Queue::open(&queue_dir).map_err(|e| format!("{e:#}"))?;
        let delay = self.state.borrow().opts.send_delay;
        let not_before =
            (delay > 0).then(|| chrono::Utc::now() + chrono::Duration::seconds(delay.into()));
        let id = queue
            .enqueue_at(&bytes, not_before)
            .map_err(|e| format!("{e:#}"))?;
        self.state.borrow_mut().last_sent = (delay > 0).then(|| (id, d.clone()));

        // mark what it answers; the draft it came from is done with
        if let Some(a) = self.answering.borrow().as_deref().and_then(locate) {
            let mut f = store::flags_of(&a);
            f.replied = true;
            f.seen = true;
            let _ = store::set_flags(&a, f);
        }
        if let Some(p) = self.saved_as.borrow_mut().take().and_then(|p| locate(&p)) {
            let _ = store::remove(&p);
        }
        self.leaving.set(true);
        self.window.close();
        self.main.quiet();
        self.main.reload();
        self.main.after_queued(delay);
        Ok(())
    }

    fn save(self: &Rc<Self>) -> Result<(), String> {
        let d = self.collect();
        let bytes = d
            .to_draft_bytes(Local::now())
            .map_err(|e| format!("{e:#}"))?;
        let folder = self.state.borrow().drafts_folder();
        let flags = Flags {
            draft: true,
            seen: true,
            ..Flags::default()
        };
        let new = store::deliver(&folder, &bytes, flags, false).map_err(|e| format!("{e:#}"))?;
        if let Some(old) = self.saved_as.borrow().as_deref().and_then(locate) {
            let _ = store::remove(&old);
        }
        *self.saved_as.borrow_mut() = Some(new);
        *self.clean.borrow_mut() = self.snapshot();
        self.main.quiet();
        self.main.reload();
        Ok(())
    }

    fn discard(self: &Rc<Self>) {
        if !self.is_dirty() {
            self.leaving.set(true);
            self.window.close();
            return;
        }
        self.ask_leave();
    }

    fn ask_leave(self: &Rc<Self>) {
        let dialog = adw::AlertDialog::builder()
            .heading("Keep this message?")
            .body("It has not been sent or saved.")
            .build();
        dialog.add_responses(&[
            ("cancel", "_Keep Writing"),
            ("discard", "_Discard"),
            ("save", "_Save Draft"),
        ]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("save"));
        dialog.set_close_response("cancel");
        let weak = Rc::downgrade(self);
        dialog.connect_response(None, move |_, r| {
            let Some(c) = weak.upgrade() else { return };
            match r {
                "save" => match c.save() {
                    Ok(()) => {
                        c.leaving.set(true);
                        c.window.close();
                    }
                    Err(e) => c.toast(&e),
                },
                "discard" => {
                    c.leaving.set(true);
                    c.window.close();
                }
                _ => {}
            }
        });
        dialog.present(Some(&self.window));
    }
}

fn signature_text(setting: &Option<String>) -> String {
    let Some(s) = setting else {
        return String::new();
    };
    let path = rical_ui::config::expand(s, &std::env::current_dir().unwrap_or_default());
    if !s.contains('\n') && path.is_file() {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .trim_end()
            .to_owned()
    } else {
        s.replace("\\n", "\n").trim_end().to_owned()
    }
}

impl palette::Host for Compose {
    fn anchor(&self) -> gtk::Widget {
        self.subject.clone().upcast()
    }

    fn window(&self) -> adw::ApplicationWindow {
        self.window.clone()
    }

    fn complete(&self, line: &str) -> rical_ui::command::Completion {
        let state = self.state.borrow();
        REGISTRY.complete(line, cmds::WRITE, &StateValues(&state))
    }

    fn execute(self: &Rc<Self>, line: &str) -> Result<(), String> {
        Compose::execute(self, line)
    }

    fn history(&self) -> &RefCell<Vec<String>> {
        &self.history
    }

    fn focus_view(&self) {
        self.body.grab_focus();
    }

    fn placeholder(&self) -> &'static str {
        "command, e.g. attach ~/report.pdf, reflow, from work, send"
    }
}

trait LookupExt {
    fn or_else_global(self, f: impl FnOnce() -> Lookup) -> Lookup;
}

impl LookupExt for Lookup {
    fn or_else_global(self, f: impl FnOnce() -> Lookup) -> Lookup {
        match self {
            Lookup::None => f(),
            other => other,
        }
    }
}

impl Compose {
    /// Dim what is quoted and the signature, as the reader does.
    fn tag_text(&self) {
        let buf = self.body.buffer();
        let table = buf.tag_table();
        let fg = self.body.color();
        let make = |name: &str, alpha: f32| -> gtk::TextTag {
            let rgba = gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), alpha);
            if let Some(t) = table.lookup(name) {
                t.set_foreground_rgba(Some(&rgba));
                return t;
            }
            let t = gtk::TextTag::builder()
                .name(name)
                .foreground_rgba(&rgba)
                .build();
            table.add(&t);
            t
        };
        let quotes = [
            make("quote1", 0.72),
            make("quote2", 0.58),
            make("quote3", 0.46),
        ];
        let sig = make("signature", 0.6);
        buf.remove_all_tags(&buf.start_iter(), &buf.end_iter());
        let mut in_sig = false;
        for n in 0..buf.line_count().min(3000) {
            let Some(a) = buf.iter_at_line(n) else { break };
            let mut b = a;
            b.forward_to_line_end();
            let line = buf.text(&a, &b, false);
            if line == "-- " {
                in_sig = true;
            }
            let depth = line
                .chars()
                .take_while(|c| *c == '>' || *c == ' ')
                .filter(|c| *c == '>')
                .count();
            if in_sig {
                buf.apply_tag(&sig, &a, &b);
            } else if depth > 0 {
                buf.apply_tag(&quotes[(depth - 1).min(2)], &a, &b);
            }
            if self.markdown.get() && !in_sig {
                let accent = adw::StyleManager::default().accent_color_rgba();
                let kinds: [(editor::Md, gtk::TextTag); 5] = {
                    let tag = |name: &str, build: &dyn Fn() -> gtk::TextTag| {
                        table.lookup(name).unwrap_or_else(|| {
                            let t = build();
                            table.add(&t);
                            t
                        })
                    };
                    [
                        (
                            editor::Md::Bold,
                            tag("md-bold", &|| {
                                gtk::TextTag::builder().name("md-bold").weight(700).build()
                            }),
                        ),
                        (
                            editor::Md::Italic,
                            tag("md-italic", &|| {
                                gtk::TextTag::builder()
                                    .name("md-italic")
                                    .style(gtk::pango::Style::Italic)
                                    .build()
                            }),
                        ),
                        (
                            editor::Md::Code,
                            tag("md-code", &|| {
                                gtk::TextTag::builder()
                                    .name("md-code")
                                    .background_rgba(&gdk::RGBA::new(0.5, 0.5, 0.5, 0.18))
                                    .build()
                            }),
                        ),
                        (
                            editor::Md::Heading,
                            tag("md-heading", &|| {
                                gtk::TextTag::builder()
                                    .name("md-heading")
                                    .weight(700)
                                    .scale(1.15)
                                    .build()
                            }),
                        ),
                        (
                            editor::Md::Link,
                            tag("md-link", &|| {
                                gtk::TextTag::builder().name("md-link").build()
                            }),
                        ),
                    ]
                };
                if let Some((_, t)) = kinds.iter().find(|(k, _)| *k == editor::Md::Link) {
                    t.set_foreground_rgba(Some(&accent));
                }
                let start = a.offset();
                for (from, to, kind) in editor::md_spans(&line) {
                    if let Some((_, t)) = kinds.iter().find(|(k, _)| *k == kind) {
                        buf.apply_tag(
                            t,
                            &buf.iter_at_offset(start + from as i32),
                            &buf.iter_at_offset(start + to as i32),
                        );
                    }
                }
            }
        }
    }
}

impl Compose {
    /// Put Markdown around the selection, or open it for typing in.
    fn format(&self, what: &str) -> Result<(), String> {
        if !self.markdown.get() {
            self.markdown.set(true);
            self.tag_text();
            self.update_status();
            self.toast("Markdown is on for this message");
        }
        let buf = self.body.buffer();
        match what {
            "bold" => self.surround("**", "**"),
            "italic" => self.surround("*", "*"),
            "code" => self.surround("`", "`"),
            "link" => {
                let sel = buf.selection_bounds();
                buf.begin_user_action();
                match sel {
                    Some((mut a, mut b)) => {
                        let text = buf.text(&a, &b, false).to_string();
                        buf.delete(&mut a, &mut b);
                        let at = a.offset();
                        buf.insert(&mut a, &format!("[{text}](https://)"));
                        // the address selected, ready to type over
                        let url_start = at + text.chars().count() as i32 + 3;
                        buf.select_range(
                            &buf.iter_at_offset(url_start),
                            &buf.iter_at_offset(url_start + 8),
                        );
                    }
                    None => {
                        buf.insert_at_cursor("[](https://)");
                        let cur = buf.iter_at_mark(&buf.get_insert());
                        let mut back = cur;
                        back.backward_chars(11);
                        buf.place_cursor(&back);
                    }
                }
                buf.end_user_action();
            }
            "heading" | "list" => {
                let (first, last) = self.selected_lines();
                buf.begin_user_action();
                for line in first..=last {
                    if let Some(mut at) = buf.iter_at_line(line) {
                        buf.insert(&mut at, if what == "heading" { "## " } else { "- " });
                    }
                }
                buf.end_user_action();
            }
            other => return Err(format!("don't know how to format “{other}”")),
        }
        Ok(())
    }

    fn surround(&self, open: &str, close: &str) {
        let buf = self.body.buffer();
        buf.begin_user_action();
        match buf.selection_bounds() {
            Some((mut a, mut b)) => {
                let text = buf.text(&a, &b, false).to_string();
                // on text already wrapped, take the marks off
                if text.starts_with(open)
                    && text.ends_with(close)
                    && text.chars().count() >= open.len() + close.len()
                {
                    let inner = &text[open.len()..text.len() - close.len()];
                    buf.delete(&mut a, &mut b);
                    let at = a.offset();
                    buf.insert(&mut a, inner);
                    buf.select_range(
                        &buf.iter_at_offset(at),
                        &buf.iter_at_offset(at + inner.chars().count() as i32),
                    );
                } else {
                    buf.delete(&mut a, &mut b);
                    let at = a.offset();
                    buf.insert(&mut a, &format!("{open}{text}{close}"));
                    let start = at + open.chars().count() as i32;
                    buf.select_range(
                        &buf.iter_at_offset(start),
                        &buf.iter_at_offset(start + text.chars().count() as i32),
                    );
                }
            }
            None => {
                buf.insert_at_cursor(&format!("{open}{close}"));
                let mut back = buf.iter_at_mark(&buf.get_insert());
                back.backward_chars(close.chars().count() as i32);
                buf.place_cursor(&back);
            }
        }
        buf.end_user_action();
    }
}

impl Compose {
    /// Keep a draft safe as you write: every minute, if there is anything
    /// unsaved.
    fn autosave(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::timeout_add_seconds_local(60, move || {
            let Some(c) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if c.leaving.get() {
                return glib::ControlFlow::Break;
            }
            if c.is_dirty() && !c.busy.get() && !c.body_text().trim().is_empty() && c.save().is_ok()
            {
                c.toast("Draft kept");
            }
            glib::ControlFlow::Continue
        });
    }

    /// Dropping files on the message attaches them.
    fn accept_drops(self: &Rc<Self>) {
        let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |_, value, _, _| {
            let (Some(c), Ok(list)) = (weak.upgrade(), value.get::<gdk::FileList>()) else {
                return false;
            };
            for f in list.files() {
                if let Some(p) = f.path()
                    && let Err(e) = c.add_attachment(p)
                {
                    c.toast(&e);
                }
            }
            true
        });
        self.window.add_controller(drop);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mailto_links() {
        let d = parse_mailto(
            "mailto:bo@example.com,cy@example.com?subject=Hello%20there&cc=ann%40example.com&body=Hi%0Abye",
        );
        assert_eq!(d.to, "bo@example.com,cy@example.com");
        assert_eq!(d.subject, "Hello there");
        assert_eq!(d.cc, "ann@example.com");
        assert_eq!(d.body, "Hi\nbye");
        assert_eq!(parse_mailto("mailto:a@b.example").to, "a@b.example");
    }
}

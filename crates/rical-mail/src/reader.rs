//! The reader: one message, as text. Nothing in it is fetched or run; what
//! a message tried is counted in a banner, links say where they go before
//! they are opened, and invitations and people hand over to the apps for
//! them.

use std::{cell::RefCell, ops::Range, rc::Rc};

use adw::prelude::*;
use chrono::{Local, TimeZone as _};
use gtk::{gdk, glib, pango};
use rical_core::diff::{Diff, Guest, LineOp, Standing, answer};
use rical_maildir::{
    addr::Mailbox,
    read::{BodyKind, Opened},
};

/// A link in the text, by characters in the buffer.
#[derive(Debug, Clone)]
struct Span {
    chars: Range<i32>,
    number: usize,
}

/// Told the number of a link that was clicked.
pub type LinkHandler = Rc<dyn Fn(usize)>;

/// Told the address of a link clicked in the HTML view.
pub type UrlHandler = Rc<dyn Fn(String)>;

pub struct Reader {
    pub widget: gtk::Stack,
    subject: gtk::Label,
    heads: gtk::Grid,
    banner: gtk::Box,
    invites: gtk::Box,
    pub body: gtk::TextView,
    attachments: gtk::Box,
    fold_button: gtk::Button,
    folded: std::cell::Cell<bool>,
    spans: RefCell<Vec<Span>>,
    scroller: gtk::ScrolledWindow,
    /// Told the number of a link that was clicked.
    pub on_link: RefCell<Option<LinkHandler>>,
    /// Told the address of a link clicked in the HTML view.
    pub on_url: RefCell<Option<UrlHandler>>,
    /// The HTML view, made when first wanted: starting WebKit costs a
    /// noticeable part of a second, too much to pay before the window shows.
    web: RefCell<Option<webkit::WebView>>,
    /// Itself, for the HTML view's handlers.
    me: RefCell<std::rc::Weak<Reader>>,
    body_stack: gtk::Stack,
    /// Show HTML as HTML (when there is any), else as text.
    pub prefer_html: std::cell::Cell<bool>,
}

pub fn full_date(ts: i64) -> String {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|d| d.format("%a %-d %b %Y, %H:%M").to_string())
        .unwrap_or_default()
}

pub fn kilobytes(n: usize) -> String {
    if n < 1024 {
        format!("{n} B")
    } else if n < 1024 * 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{:.1} MB", n as f64 / 1024.0 / 1024.0)
    }
}

fn dim_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(1.0)
        .yalign(0.0)
        .css_classes(["dim-label", "caption"])
        .build()
}

fn value_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(pango::WrapMode::WordChar)
        .selectable(true)
        .hexpand(true)
        .build()
}

fn people(list: &[Mailbox]) -> String {
    list.iter()
        .map(Mailbox::display)
        .collect::<Vec<_>>()
        .join(", ")
}

impl Reader {
    pub fn new(proportional: bool) -> Rc<Reader> {
        let empty = adw::StatusPage::builder()
            .icon_name("mail-unread-symbolic")
            .title("No message selected")
            .description("Pick one from the list")
            .build();

        let subject = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .selectable(true)
            .css_classes(["title-2"])
            .build();
        let heads = gtk::Grid::builder()
            .column_spacing(10)
            .row_spacing(2)
            .build();
        let banner = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let invites = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();
        let body = gtk::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .wrap_mode(gtk::WrapMode::WordChar)
            .left_margin(2)
            .right_margin(2)
            .top_margin(8)
            .bottom_margin(8)
            .css_classes(["rical-body"])
            .build();
        body.set_monospace(!proportional);
        body.update_property(&[gtk::accessible::Property::Label("Message")]);
        let attachments = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(6)
            .build();

        let column = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .spacing(10)
            .margin_start(18)
            .margin_end(18)
            .margin_top(14)
            .margin_bottom(18)
            .build();
        let clamp = adw::Clamp::builder()
            .maximum_size(900)
            .tightening_threshold(700)
            .child(&column)
            .build();
        column.append(&subject);
        column.append(&heads);
        column.append(&banner);
        column.append(&invites);
        column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        // the text, or the HTML as it was meant to look (nothing fetched)
        let body_stack = gtk::Stack::builder().vhomogeneous(false).build();
        body_stack.add_named(&body, Some("text"));
        column.append(&body_stack);
        let fold_button = gtk::Button::builder()
            .halign(gtk::Align::Start)
            .css_classes(["flat", "pill"])
            .visible(false)
            .build();
        column.append(&fold_button);
        column.append(&attachments);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&clamp)
            .vexpand(true)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .build();

        let stack = gtk::Stack::new();
        stack.add_named(&empty, Some("empty"));
        stack.add_named(&scroller, Some("message"));
        stack.set_visible_child_name("empty");

        let reader = Rc::new(Reader {
            widget: stack,
            subject,
            heads,
            banner,
            invites,
            body,
            attachments,
            fold_button,
            folded: std::cell::Cell::new(false),
            spans: RefCell::default(),
            scroller,
            on_link: RefCell::default(),
            on_url: RefCell::default(),
            web: RefCell::default(),
            me: RefCell::default(),
            body_stack,
            prefer_html: std::cell::Cell::new(true),
        });
        reader.setup_links();
        *reader.me.borrow_mut() = Rc::downgrade(&reader);
        let weak = Rc::downgrade(&reader);
        reader.fold_button.connect_clicked(move |b| {
            let Some(r) = weak.upgrade() else { return };
            let table = r.body.buffer().tag_table();
            if let Some(t) = table.lookup("folded") {
                let hide = !t.is_invisible();
                t.set_invisible(hide);
                r.folded.set(hide);
                b.set_label(if hide {
                    "Show the quoted lines"
                } else {
                    "Hide the quoted lines"
                });
            }
        });
        reader
    }

    pub fn clear(&self) {
        self.widget.set_visible_child_name("empty");
    }

    /// Which link number is under a point in the text.
    fn link_at(&self, x: f64, y: f64) -> Option<usize> {
        let (bx, by) =
            self.body
                .window_to_buffer_coords(gtk::TextWindowType::Widget, x as i32, y as i32);
        let iter = self.body.iter_at_location(bx, by)?;
        let at = iter.offset();
        self.spans
            .borrow()
            .iter()
            .find(|s| s.chars.contains(&at))
            .map(|s| s.number)
    }

    fn setup_links(self: &Rc<Self>) {
        let click = gtk::GestureClick::new();
        let reader = Rc::downgrade(self);
        click.connect_released(move |g, _, x, y| {
            let Some(r) = reader.upgrade() else { return };
            // a drag that selected text is not a click on a link
            if r.body.buffer().has_selection() {
                return;
            }
            if let Some(n) = r.link_at(x, y) {
                g.set_state(gtk::EventSequenceState::Claimed);
                let handler = r.on_link.borrow().clone();
                if let Some(h) = handler {
                    h(n + 1);
                }
            }
        });
        self.body.add_controller(click);
        let motion = gtk::EventControllerMotion::new();
        let reader = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            let Some(r) = reader.upgrade() else { return };
            let name = if r.link_at(x, y).is_some() {
                "pointer"
            } else {
                "text"
            };
            r.body.set_cursor_from_name(Some(name));
        });
        self.body.add_controller(motion);
    }

    /// Show a message.
    pub fn show(&self, o: &Opened, in_contacts: bool, note: Option<&str>, diffs: &[Diff]) {
        self.subject.set_label(if o.subject.is_empty() {
            "(no subject)"
        } else {
            &o.subject
        });
        // selectable, but not selected just because it was shown
        self.subject.select_region(0, 0);

        // who, when
        while let Some(c) = self.heads.first_child() {
            self.heads.remove(&c);
        }
        let mut row = 0;
        let mut add = |name: &str, value: &str, extra: Option<gtk::Widget>| {
            if value.is_empty() {
                return;
            }
            self.heads.attach(&dim_label(name), 0, row, 1, 1);
            let v = value_label(value);
            match extra {
                Some(w) => {
                    let b = gtk::Box::builder().spacing(6).build();
                    b.append(&v);
                    b.append(&w);
                    self.heads.attach(&b, 1, row, 1, 1);
                }
                None => self.heads.attach(&v, 1, row, 1, 1),
            }
            row += 1;
        };
        let known = (in_contacts && !o.from.is_empty()).then(|| {
            gtk::Image::builder()
                .icon_name("avatar-default-symbolic")
                .tooltip_text("In your contacts")
                .css_classes(["dim-label"])
                .build()
                .upcast::<gtk::Widget>()
        });
        add("From", &people(&o.from), known);
        if !o.reply_to.is_empty() && o.reply_to != o.from {
            add("Reply-To", &people(&o.reply_to), None);
        }
        add("To", &people(&o.to), None);
        add("Cc", &people(&o.cc), None);
        add("Bcc", &people(&o.bcc), None);
        add("Date", &o.date.map(full_date).unwrap_or_default(), None);
        if let Some(n) = note {
            add("Status", n, None);
        }
        if let Some(l) = &o.list_id {
            let unsub = (!o.unsubscribe.is_empty()).then(|| {
                gtk::Button::builder()
                    .label("Unsubscribe")
                    .css_classes(["flat", "pill"])
                    .tooltip_text("Leave this list (asks first)")
                    .action_name("win.run")
                    .action_target(&"unsubscribe".to_variant())
                    .build()
                    .upcast::<gtk::Widget>()
            });
            add("List", l, unsub);
        }

        self.show_privacy(o);
        self.show_invites(o, diffs);
        self.show_body(o);
        self.show_html(o);
        self.show_attachments(o);
        self.widget.set_visible_child_name("message");
        self.body.queue_resize();
        // measured again once the window has laid it out at its real width
        for ms in [30u64, 150] {
            let body = self.body.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
                body.queue_resize()
            });
        }
        self.scroller.vadjustment().set_value(0.0);
    }

    fn show_privacy(&self, o: &Opened) {
        while let Some(c) = self.banner.first_child() {
            self.banner.remove(&c);
        }
        let p = &o.privacy;
        let mut lines: Vec<String> = Vec::new();
        if p.remote_images > 0 {
            let mut s = format!(
                "{} image{} it would have loaded from the web {} not loaded",
                p.remote_images,
                if p.remote_images == 1 { "" } else { "s" },
                if p.remote_images == 1 { "was" } else { "were" }
            );
            if p.tracking_pixels > 0 {
                s += &format!(
                    ", including {} tracking pixel{} that tell{} the sender you opened it",
                    p.tracking_pixels,
                    if p.tracking_pixels == 1 { "" } else { "s" },
                    if p.tracking_pixels == 1 { "s" } else { "" }
                );
            }
            lines.push(s + ".");
        }
        if let Some(who) = &p.read_receipt {
            lines.push(format!(
                "{who} asked to be told when you read this. No receipt is sent."
            ));
        }
        if p.misleading_links > 0 {
            lines.push(format!(
                "{} link{} show{} one address and go{} to another. Check before opening.",
                p.misleading_links,
                if p.misleading_links == 1 { "" } else { "s" },
                if p.misleading_links == 1 { "s" } else { "" },
                if p.misleading_links == 1 { "es" } else { "" },
            ));
        }
        if o.has_html && self.prefer_html.get() {
            lines.push(
                "Shown as its HTML, with nothing loaded from the web. h shows it as text."
                    .to_owned(),
            );
        } else if o.body.kind == BodyKind::Html {
            lines.push("This message is HTML, shown as text. h shows its HTML.".to_owned());
        }
        for l in &lines {
            let warn = p.misleading_links > 0 && l.contains("link") && !l.contains("HTML");
            let b = gtk::Box::builder().spacing(8).build();
            b.add_css_class("card");
            b.set_margin_top(0);
            let icon = gtk::Image::from_icon_name(if warn {
                "dialog-warning-symbolic"
            } else {
                "security-high-symbolic"
            });
            icon.set_margin_start(10);
            icon.set_valign(gtk::Align::Start);
            icon.set_margin_top(8);
            let label = gtk::Label::builder()
                .label(l)
                .wrap(true)
                .xalign(0.0)
                .hexpand(true)
                .margin_top(7)
                .margin_bottom(7)
                .margin_end(10)
                .css_classes(["caption"])
                .build();
            b.append(&icon);
            b.append(&label);
            self.banner.append(&b);
        }
    }

    fn show_invites(&self, o: &Opened, diffs: &[Diff]) {
        while let Some(c) = self.invites.first_child() {
            self.invites.remove(&c);
        }
        if !diffs.is_empty() {
            for (i, d) in diffs.iter().enumerate() {
                self.invites.append(&diff_card(d, o.invites.get(i)));
            }
            return;
        }
        for inv in &o.invites {
            let group = adw::PreferencesGroup::builder().title(inv.kind()).build();
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&if inv.summary.is_empty() {
                    "(no title)".to_owned()
                } else {
                    inv.summary.clone()
                }))
                .subtitle(glib::markup_escape_text(&{
                    let mut s = inv.when_shown();
                    if !inv.location.is_empty() {
                        s += &format!("\n{}", inv.location);
                    }
                    if !inv.organizer.is_empty() {
                        s += &format!("\nFrom {}", inv.organizer);
                    }
                    s
                }))
                .build();
            row.set_subtitle_lines(0);
            let open = gtk::Button::builder()
                .label("Open in Calendar")
                .valign(gtk::Align::Center)
                .action_name("win.run")
                .action_target(&"invite".to_variant())
                .css_classes(["suggested-action"])
                .build();
            row.add_suffix(&open);
            group.add(&row);
            if inv.method == "REQUEST" {
                group.add(&rsvp_row());
            }
            self.invites.append(&group);
        }
    }

    fn show_body(&self, o: &Opened) {
        let buf = self.body.buffer();
        buf.set_text("");
        self.spans.borrow_mut().clear();
        let text = &o.body.text;
        if o.body.kind == BodyKind::Empty {
            buf.set_text("(no text)");
            return;
        }
        buf.insert(&mut buf.end_iter(), text);

        let fg = self.body.color();
        let tint = |alpha: f32| gdk::RGBA::new(fg.red(), fg.green(), fg.blue(), alpha);
        let table = buf.tag_table();
        let make = |name: &str, alpha: f32| -> gtk::TextTag {
            if let Some(t) = table.lookup(name) {
                t.set_foreground_rgba(Some(&tint(alpha)));
                return t;
            }
            let t = gtk::TextTag::builder()
                .name(name)
                .foreground_rgba(&tint(alpha))
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
        let accent = adw::StyleManager::default().accent_color_rgba();
        let link = match table.lookup("link") {
            Some(t) => t,
            None => {
                let t = gtk::TextTag::builder()
                    .name("link")
                    .underline(pango::Underline::Single)
                    .build();
                table.add(&t);
                t
            }
        };
        link.set_foreground_rgba(Some(&accent));
        let warn = match table.lookup("misleading") {
            Some(t) => t,
            None => {
                let t = gtk::TextTag::builder()
                    .name("misleading")
                    .underline(pango::Underline::Error)
                    .foreground("#c01c28")
                    .build();
                table.add(&t);
                t
            }
        };

        // quoted lines and the signature
        let mut in_sig = false;
        for (line_no, line) in text.split('\n').enumerate() {
            if line == "-- " {
                in_sig = true;
            }
            let depth = line
                .chars()
                .take_while(|c| *c == '>' || *c == ' ')
                .filter(|c| *c == '>')
                .count();
            let tag = if in_sig {
                Some(&sig)
            } else if depth > 0 {
                Some(&quotes[(depth - 1).min(2)])
            } else {
                None
            };
            if let Some(t) = tag
                && let (Some(a), Some(mut b)) = (
                    buf.iter_at_line(line_no as i32),
                    buf.iter_at_line(line_no as i32),
                )
            {
                b.forward_to_line_end();
                buf.apply_tag(t, &a, &b);
            }
        }

        // long quoted blocks fold: two lines stay, the rest hide until asked
        let fold = match table.lookup("folded") {
            Some(t) => t,
            None => {
                let t = gtk::TextTag::builder().name("folded").build();
                table.add(&t);
                t
            }
        };
        fold.set_invisible(true);
        let lines: Vec<&str> = text.split('\n').collect();
        let mut hidden = 0;
        let mut i = 0;
        while i < lines.len() {
            if !lines[i].starts_with('>') {
                i += 1;
                continue;
            }
            let start = i;
            while i < lines.len() && lines[i].starts_with('>') {
                i += 1;
            }
            if i - start > 6 {
                let (Some(a), Some(mut b)) = (
                    buf.iter_at_line((start + 2) as i32),
                    buf.iter_at_line((i - 1) as i32),
                ) else {
                    continue;
                };
                b.forward_to_line_end();
                // the line break before the block stays with the block's end
                buf.apply_tag(&fold, &a, &b);
                hidden += i - start - 2;
            }
        }
        self.folded.set(hidden > 0);
        self.fold_button.set_visible(hidden > 0);
        self.fold_button
            .set_label(&format!("Show {hidden} more quoted lines"));

        // links
        // the buffer counts in characters, the links in bytes: counted along
        // the text once, in order, not from its start for each link (a
        // newsletter has thousands)
        let mut spans = Vec::new();
        let mut order: Vec<usize> = (0..o.body.links.len()).collect();
        order.sort_by_key(|&n| o.body.links[n].range.start);
        let (mut byte, mut char) = (0usize, 0usize);
        for n in order {
            let l = &o.body.links[n];
            if l.range.start < byte {
                // overlapping the last: count again from the start
                (byte, char) = (0, 0);
            }
            char += text[byte..l.range.start].chars().count();
            byte = l.range.start;
            let start = char as i32;
            let end = start + text[l.range.clone()].chars().count() as i32;
            let (a, b) = (buf.iter_at_offset(start), buf.iter_at_offset(end));
            buf.apply_tag(&link, &a, &b);
            if l.misleading {
                buf.apply_tag(&warn, &a, &b);
            }
            spans.push(Span {
                chars: start..end,
                number: n,
            });
        }
        *self.spans.borrow_mut() = spans;
    }

    fn show_attachments(&self, o: &Opened) {
        while let Some(c) = self.attachments.first_child() {
            self.attachments.remove(&c);
        }
        let shown: Vec<_> = o.attachments.iter().enumerate().collect();
        if shown.is_empty() {
            return;
        }
        let group = adw::PreferencesGroup::builder()
            .title(format!(
                "{} attachment{}",
                shown.len(),
                if shown.len() == 1 { "" } else { "s" }
            ))
            .build();
        for (i, a) in shown {
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&a.name))
                .subtitle(format!("{} · {}", a.mime, kilobytes(a.size)))
                .build();
            let icon = gtk::Image::from_icon_name(if a.calendar {
                "x-office-calendar-symbolic"
            } else if a.mime.starts_with("image/") {
                "image-x-generic-symbolic"
            } else {
                "mail-attachment-symbolic"
            });
            // a picture shows itself (it is in the message: nothing is fetched)
            let thumb = (a.mime.starts_with("image/") && a.size < 8 * 1024 * 1024)
                .then(|| rical_maildir::read::part_bytes(&o.path, a.part).ok())
                .flatten()
                .and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from_owned(b)).ok());
            match thumb {
                Some(t) => {
                    let pic = gtk::Picture::for_paintable(&t);
                    pic.set_content_fit(gtk::ContentFit::Cover);
                    pic.set_size_request(56, 56);
                    pic.set_valign(gtk::Align::Center);
                    pic.add_css_class("card");
                    row.add_prefix(&pic);
                }
                None => row.add_prefix(&icon),
            }
            let open = gtk::Button::builder()
                .label("Open")
                .valign(gtk::Align::Center)
                .action_name("win.run")
                .action_target(&format!("attachment open {}", i + 1).to_variant())
                .build();
            let save = gtk::Button::builder()
                .label("Save")
                .valign(gtk::Align::Center)
                .action_name("win.run")
                .action_target(&format!("attachment save {}", i + 1).to_variant())
                .build();
            row.add_suffix(&open);
            row.add_suffix(&save);
            group.add(&row);
        }
        self.attachments.append(&group);
    }
}

/// How the message stands, as the heading of its card.
fn headline(d: &Diff) -> String {
    let n = d.changes.len() + d.guests.len();
    let mut s = match d.standing {
        Standing::New => "Invitation · not in your calendar yet".to_owned(),
        Standing::Newer if n == 0 => "Updated invitation · nothing you can see changed".to_owned(),
        Standing::Newer => format!(
            "Updated invitation · {n} change{}",
            if n == 1 { "" } else { "s" }
        ),
        Standing::Duplicate => "Invitation · already in your calendar, no changes".to_owned(),
        Standing::Older => "Older version · your calendar has a newer one".to_owned(),
        Standing::Cancel => "Cancelled".to_owned(),
        Standing::Reply => "Reply to an invitation".to_owned(),
    };
    if let Some(o) = &d.occurrence {
        s += &format!(" · for {o}");
    }
    s
}

fn esc(s: &str) -> String {
    glib::markup_escape_text(s).to_string()
}

/// One event's invitation or update, with what changes in your calendar.
fn diff_card(d: &Diff, light: Option<&rical_maildir::invite::Invite>) -> adw::PreferencesGroup {
    let group = adw::PreferencesGroup::builder()
        .title(esc(&headline(d)))
        .build();
    if let Some(c) = &d.calendar {
        group.set_description(Some(&esc(&format!("In your calendar “{c}”"))));
    }
    // the event itself: its title and, unless the rows below say it, when
    let mut subtitle = String::new();
    if let Some(l) = light {
        let when_changed = d
            .changes
            .iter()
            .any(|c| c.field == rical_core::diff::Field::When);
        if !when_changed {
            subtitle += &esc(&l.when_shown());
        }
        let place_changed = d
            .changes
            .iter()
            .any(|c| c.field == rical_core::diff::Field::Place);
        if !place_changed && !l.location.is_empty() {
            subtitle += &format!("\n{}", esc(&l.location));
        }
        if !l.organizer.is_empty() && d.standing == Standing::New {
            subtitle += &format!("\nFrom {}", esc(&l.organizer));
        }
    }
    let head = adw::ActionRow::builder()
        .title(esc(&if d.summary.is_empty() {
            "(no title)".to_owned()
        } else {
            d.summary.clone()
        }))
        .subtitle(subtitle.trim_start_matches('\n'))
        .build();
    head.set_subtitle_lines(0);
    if !matches!(d.standing, Standing::Older | Standing::Duplicate) {
        head.add_suffix(
            &gtk::Button::builder()
                .label("Open in Calendar")
                .valign(gtk::Align::Center)
                .action_name("win.run")
                .action_target(&"invite".to_variant())
                .css_classes(["suggested-action"])
                .build(),
        );
    }
    group.add(&head);
    if d.method == "REQUEST" && d.standing == Standing::New {
        group.add(&rsvp_row());
    }

    for c in &d.changes {
        let mut sub = if c.field == rical_core::diff::Field::Description {
            let mut lines = String::new();
            for (op, l) in &c.lines {
                let line = match op {
                    LineOp::Same => format!("<span alpha=\"60%\">  {}</span>", esc(l)),
                    LineOp::Added => format!("<span foreground=\"#26a269\">+ {}</span>", esc(l)),
                    LineOp::Removed => {
                        format!("<span foreground=\"#c01c28\">− <s>{}</s></span>", esc(l))
                    }
                };
                if !lines.is_empty() {
                    lines.push('\n');
                }
                lines += &line;
            }
            lines
        } else {
            let old = if c.old.is_empty() {
                "nothing".to_owned()
            } else {
                c.old.clone()
            };
            let new = if c.new.is_empty() {
                "nothing".to_owned()
            } else {
                c.new.clone()
            };
            format!(
                "<span alpha=\"65%\"><s>{}</s></span>\n<b>{}</b>",
                esc(&old),
                esc(&new)
            )
        };
        if let Some(n) = &c.note {
            sub += &format!("\n<span alpha=\"65%\">{}</span>", esc(n));
        }
        let row = adw::ActionRow::builder()
            .title(c.field.label())
            .subtitle(sub)
            .build();
        row.set_subtitle_lines(0);
        row.add_prefix(&gtk::Image::from_icon_name(match c.field {
            rical_core::diff::Field::When => "alarm-symbolic",
            rical_core::diff::Field::Place => "mark-location-symbolic",
            rical_core::diff::Field::Description => "format-justify-left-symbolic",
            rical_core::diff::Field::Repeat => "media-playlist-repeat-symbolic",
            _ => "document-edit-symbolic",
        }));
        group.add(&row);
    }
    for g in &d.guests {
        let (words, icon) = match &g.what {
            Guest::Added => ("was invited".to_owned(), "list-add-symbolic"),
            Guest::Removed => ("is no longer invited".to_owned(), "list-remove-symbolic"),
            Guest::Answered(_, to) => (answer(to).to_owned(), "avatar-default-symbolic"),
        };
        let row = adw::ActionRow::builder()
            .title(esc(&format!("{} {words}", g.who)))
            .build();
        row.add_prefix(&gtk::Image::from_icon_name(icon));
        group.add(&row);
    }
    if d.changes.is_empty()
        && d.guests.is_empty()
        && matches!(d.standing, Standing::Newer | Standing::Duplicate)
    {
        let row = adw::ActionRow::builder()
            .title("Nothing about the event differs from your calendar")
            .build();
        row.add_css_class("dim-label");
        group.add(&row);
    }
    group
}

/// Answer by mail: for an invitation the calendar does not have, where its
/// server will not do it for you.
fn rsvp_row() -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title("Answer by mail")
        .subtitle("Sends your answer to the organiser")
        .build();
    for (label, what) in [
        ("Accept", "accept"),
        ("Maybe", "maybe"),
        ("Decline", "decline"),
    ] {
        row.add_suffix(
            &gtk::Button::builder()
                .label(label)
                .valign(gtk::Align::Center)
                .action_name("win.run")
                .action_target(&format!("rsvp {what}").to_variant())
                .build(),
        );
    }
    row
}

/// A web view that shows one page and fetches nothing: no scripts in the
/// page, no storage, a session thrown away, and (from the page itself) a
/// Content-Security-Policy that blocks every load. Links are not followed.
fn html_view() -> webkit::WebView {
    use webkit::prelude::*;
    let settings = webkit::Settings::new();
    settings.set_enable_javascript_markup(false);
    settings.set_javascript_can_open_windows_automatically(false);
    settings.set_enable_page_cache(false);
    settings.set_enable_html5_local_storage(false);
    settings.set_enable_html5_database(false);
    settings.set_enable_webgl(false);
    settings.set_auto_load_images(true);
    settings.set_hardware_acceleration_policy(webkit::HardwareAccelerationPolicy::Never);
    let web = webkit::WebView::builder()
        .network_session(&webkit::NetworkSession::new_ephemeral())
        .settings(&settings)
        .build();
    web.set_height_request(240);
    web.set_hexpand(true);
    web.update_property(&[gtk::accessible::Property::Label("Message")]);
    web
}

impl Reader {
    /// The HTML view, made (and wired: links go to the app, it grows to
    /// fit its page) the first time it is wanted.
    fn web(&self) -> webkit::WebView {
        if let Some(w) = self.web.borrow().as_ref() {
            return w.clone();
        }
        let web = html_view();
        self.body_stack.add_named(&web, Some("html"));
        self.wire_html(&web);
        *self.web.borrow_mut() = Some(web.clone());
        web
    }

    fn wire_html(&self, web: &webkit::WebView) {
        use webkit::prelude::*;
        let weak = self.me.borrow().clone();
        web.connect_decide_policy(move |_, decision, kind| {
            use webkit::PolicyDecisionType as T;
            if !matches!(kind, T::NavigationAction | T::NewWindowAction) {
                return false;
            }
            let uri = decision
                .downcast_ref::<webkit::NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
                .and_then(|a| a.request())
                .and_then(|r| r.uri())
                .map(|u| u.to_string())
                .unwrap_or_default();
            // the page itself, and nothing else
            if uri == "about:blank" && kind == T::NavigationAction {
                return false;
            }
            decision.ignore();
            let clicked = decision
                .downcast_ref::<webkit::NavigationPolicyDecision>()
                .and_then(|d| d.navigation_action())
                .is_some_and(|a| a.navigation_type() == webkit::NavigationType::LinkClicked);
            if clicked
                && let Some(r) = weak.upgrade()
                && let Some(h) = r.on_url.borrow().clone()
            {
                h(uri);
            }
            true
        });
        // as tall as its page, so the reader scrolls it with the rest
        let weak = self.me.borrow().clone();
        web.connect_load_changed(move |web, ev| {
            if ev != webkit::LoadEvent::Finished {
                return;
            }
            let Some(r) = weak.upgrade() else { return };
            r.fit_html(web);
            let (w, r2) = (web.clone(), Rc::downgrade(&r));
            glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
                if let Some(r) = r2.upgrade() {
                    r.fit_html(&w);
                }
            });
        });
    }

    fn fit_html(&self, web: &webkit::WebView) {
        use webkit::prelude::*;
        let w = web.clone();
        web.evaluate_javascript(
            "Math.max(document.body ? document.body.scrollHeight : 0, document.documentElement.scrollHeight)",
            None,
            None,
            None::<&gtk::gio::Cancellable>,
            move |r| {
                if let Ok(v) = r {
                    let h = v.to_int32().clamp(60, 40_000);
                    w.set_height_request(h + 16);
                }
            },
        );
    }

    /// Show the message's HTML, if it has some and that is wanted.
    fn show_html(&self, o: &Opened) {
        use webkit::prelude::*;
        let doc = (o.has_html && self.prefer_html.get())
            .then(|| rical_maildir::read::html_document(&o.path).ok().flatten())
            .flatten();
        match doc {
            Some(doc) => {
                let web = self.web();
                web.set_height_request(240);
                web.load_html(&doc, None);
                self.body_stack.set_visible_child_name("html");
            }
            None => self.body_stack.set_visible_child_name("text"),
        }
    }

    /// Whether the message in front is shown as HTML.
    pub fn showing_html(&self) -> bool {
        self.body_stack.visible_child_name().as_deref() == Some("html")
    }
}

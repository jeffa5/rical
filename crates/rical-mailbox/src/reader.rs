//! The reader: one message, as text. Nothing in it is fetched or run; what
//! a message tried is counted in a banner, links say where they go before
//! they are opened, and invitations and people hand over to the apps for
//! them.

use std::{cell::RefCell, ops::Range, rc::Rc};

use adw::prelude::*;
use chrono::{Local, TimeZone as _};
use gtk::{gdk, glib, pango};
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

pub struct Reader {
    pub widget: gtk::Stack,
    subject: gtk::Label,
    heads: gtk::Grid,
    banner: gtk::Box,
    invites: gtk::Box,
    pub body: gtk::TextView,
    attachments: gtk::Box,
    spans: RefCell<Vec<Span>>,
    scroller: gtk::ScrolledWindow,
    /// Told the number of a link that was clicked.
    pub on_link: RefCell<Option<LinkHandler>>,
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
        column.append(&body);
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
            spans: RefCell::default(),
            scroller,
            on_link: RefCell::default(),
        });
        reader.setup_links();
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
    pub fn show(&self, o: &Opened, in_contacts: bool, note: Option<&str>) {
        self.subject.set_label(if o.subject.is_empty() {
            "(no subject)"
        } else {
            &o.subject
        });

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
            add("List", l, None);
        }

        self.show_privacy(o);
        self.show_invites(o);
        self.show_body(o);
        self.show_attachments(o);
        self.widget.set_visible_child_name("message");
        self.body.queue_resize();
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
        if o.body.kind == BodyKind::Html {
            lines.push(
                "This message is HTML. It is shown as text, with no styling or images.".to_owned(),
            );
        }
        for (i, l) in lines.iter().enumerate() {
            let warn = i < lines.len() - usize::from(o.body.kind == BodyKind::Html)
                && p.misleading_links > 0
                && l.contains("link");
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

    fn show_invites(&self, o: &Opened) {
        while let Some(c) = self.invites.first_child() {
            self.invites.remove(&c);
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

        // links
        let mut spans = Vec::new();
        for (n, l) in o.body.links.iter().enumerate() {
            let start = text[..l.range.start].chars().count() as i32;
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
            row.add_prefix(&icon);
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

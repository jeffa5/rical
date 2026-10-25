//! Cairo and Pango helpers shared by the drawn views.

use adw::prelude::*;
use gtk::{cairo, gdk, pango};
use rical_core::Rgb;

/// Colours taken from the current style, so drawing follows light and dark.
#[derive(Clone, Copy)]
pub struct Palette {
    pub fg: gdk::RGBA,
    pub accent: gdk::RGBA,
    pub dark: bool,
}

impl Palette {
    pub fn of(widget: &impl IsA<gtk::Widget>) -> Self {
        let style = adw::StyleManager::default();
        Self {
            fg: widget.as_ref().color(),
            accent: style.accent_color_rgba(),
            dark: style.is_dark(),
        }
    }

    pub fn fg(&self, cr: &cairo::Context, alpha: f64) {
        set_rgba(cr, &self.fg, alpha);
    }

    pub fn accent(&self, cr: &cairo::Context, alpha: f64) {
        set_rgba(cr, &self.accent, alpha);
    }

    /// Fill for an unselected event block of calendar colour `c`.
    pub fn event_fill(&self, cr: &cairo::Context, c: Rgb) {
        set_rgb(cr, c, if self.dark { 0.38 } else { 0.22 });
    }
}

pub fn set_rgba(cr: &cairo::Context, c: &gdk::RGBA, alpha: f64) {
    cr.set_source_rgba(
        c.red() as f64,
        c.green() as f64,
        c.blue() as f64,
        c.alpha() as f64 * alpha,
    );
}

pub fn set_rgb(cr: &cairo::Context, c: Rgb, alpha: f64) {
    cr.set_source_rgba(
        c.r as f64 / 255.0,
        c.g as f64 / 255.0,
        c.b as f64 / 255.0,
        alpha,
    );
}

/// Black or white, whichever reads better on `c`.
pub fn contrasting(c: Rgb) -> Rgb {
    let lum = 0.2126 * c.r as f64 + 0.7152 * c.g as f64 + 0.0722 * c.b as f64;
    if lum > 150.0 {
        Rgb { r: 0, g: 0, b: 0 }
    } else {
        Rgb {
            r: 255,
            g: 255,
            b: 255,
        }
    }
}

pub fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 3.0 * pi / 2.0);
    cr.close_path();
}

pub struct TextStyle {
    pub scale: f64,
    pub bold: bool,
    pub strikethrough: bool,
}

impl TextStyle {
    pub const SMALL: Self = Self {
        scale: 0.85,
        bold: false,
        strikethrough: false,
    };
}

/// A single-line (or wrapped, if `max_height` allows) layout ellipsized to
/// fit `width`.
pub fn layout(
    widget: &impl IsA<gtk::Widget>,
    text: &str,
    style: &TextStyle,
    width: f64,
    max_lines: i32,
) -> pango::Layout {
    let layout = widget.as_ref().create_pango_layout(Some(text));
    let attrs = pango::AttrList::new();
    if style.bold {
        attrs.insert(pango::AttrInt::new_weight(pango::Weight::Bold));
    }
    if style.scale != 1.0 {
        attrs.insert(pango::AttrFloat::new_scale(style.scale));
    }
    if style.strikethrough {
        attrs.insert(pango::AttrInt::new_strikethrough(true));
    }
    layout.set_attributes(Some(&attrs));
    layout.set_width((width.max(1.0) * pango::SCALE as f64) as i32);
    layout.set_ellipsize(pango::EllipsizeMode::End);
    layout.set_wrap(pango::WrapMode::WordChar);
    // negative height limits the number of lines
    layout.set_height(-max_lines.max(1));
    layout
}

pub fn show_layout(cr: &cairo::Context, layout: &pango::Layout, x: f64, y: f64) {
    cr.move_to(x, y);
    pangocairo::functions::show_layout(cr, layout);
}

pub fn layout_height(layout: &pango::Layout) -> f64 {
    layout.pixel_size().1 as f64
}

/// A clickable region recorded while drawing.
#[derive(Debug, Clone)]
pub struct Hit<T> {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub target: T,
}

impl<T> Hit<T> {
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    pub fn rect(&self) -> gdk::Rectangle {
        gdk::Rectangle::new(self.x as i32, self.y as i32, self.w as i32, self.h as i32)
    }
}

/// The last (topmost) hit containing the point.
pub fn hit_at<T: Clone>(hits: &[Hit<T>], x: f64, y: f64) -> Option<Hit<T>> {
    hits.iter().rev().find(|h| h.contains(x, y)).cloned()
}

/// A task's tick circle centred at (cx, cy): a ring, with a tick when done.
/// The source colour must already be set.
pub fn task_circle(cr: &cairo::Context, cx: f64, cy: f64, r: f64, done: bool) {
    cr.set_line_width(1.4);
    cr.new_sub_path();
    cr.arc(cx, cy, r, 0.0, std::f64::consts::TAU);
    let _ = cr.stroke();
    if done {
        cr.set_line_width(1.6);
        cr.move_to(cx - r * 0.5, cy + r * 0.05);
        cr.line_to(cx - r * 0.12, cy + r * 0.45);
        cr.line_to(cx + r * 0.55, cy - r * 0.4);
        let _ = cr.stroke();
    }
}

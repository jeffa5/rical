//! The day and week time grid: a header of days, an all-day row, and a
//! scrollable 24 hour grid with events drawn as blocks sized by duration.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{NaiveDate, Timelike, Utc};
use gtk::cairo;
use rical_core::{Kind, Occurrence};

use crate::{
    details,
    draw::{self, Hit, Palette, TextStyle, hit_at},
    format,
    layout::{self, ViewKind},
    state::{OccurrenceKey, SharedState},
};

pub const HOUR_HEIGHT: f64 = 48.0;
pub const GUTTER: f64 = 56.0;
const HEADER_HEIGHT: i32 = 52;
const LANE_HEIGHT: f64 = 24.0;
/// Short events are drawn at least this long so their title fits.
const MIN_EVENT_MINUTES: f64 = 22.0;

type Hits<T> = Rc<RefCell<Vec<Hit<T>>>>;

pub struct TimeGrid {
    pub widget: gtk::Box,
    header: gtk::DrawingArea,
    all_day: gtk::DrawingArea,
    scroller: gtk::ScrolledWindow,
    pub grid: gtk::DrawingArea,
    state: SharedState,
    grid_hits: Hits<OccurrenceKey>,
    all_day_hits: Hits<OccurrenceKey>,
}

impl TimeGrid {
    pub fn new(state: SharedState, on_change: Rc<dyn Fn()>) -> Rc<Self> {
        let header = gtk::DrawingArea::builder()
            .content_height(HEADER_HEIGHT)
            .hexpand(true)
            .build();
        let all_day = gtk::DrawingArea::builder()
            .content_height(LANE_HEIGHT as i32 + 6)
            .hexpand(true)
            .build();
        let grid = gtk::DrawingArea::builder()
            .content_height((24.0 * HOUR_HEIGHT) as i32)
            .hexpand(true)
            .focusable(true)
            .can_focus(true)
            .build();
        grid.update_property(&[gtk::accessible::Property::Label("Time grid")]);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&grid)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();

        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.append(&header);
        widget.append(&all_day);
        widget.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        widget.append(&scroller);

        let this = Rc::new(Self {
            widget,
            header,
            all_day,
            scroller,
            grid,
            state,
            grid_hits: Default::default(),
            all_day_hits: Default::default(),
        });
        this.setup_drawing();
        this.setup_clicks(on_change);
        this
    }

    fn setup_drawing(self: &Rc<Self>) {
        let this = Rc::downgrade(self);
        self.header.set_draw_func(move |area, cr, w, _h| {
            if let Some(this) = this.upgrade() {
                this.draw_header(area, cr, w as f64);
            }
        });
        let this = Rc::downgrade(self);
        self.all_day.set_draw_func(move |area, cr, w, h| {
            if let Some(this) = this.upgrade() {
                this.draw_all_day(area, cr, w as f64, h as f64);
            }
        });
        let this = Rc::downgrade(self);
        self.grid.set_draw_func(move |area, cr, w, _h| {
            if let Some(this) = this.upgrade() {
                this.draw_grid(area, cr, w as f64);
            }
        });
    }

    fn setup_clicks(self: &Rc<Self>, on_change: Rc<dyn Fn()>) {
        // header: open that day
        let click = gtk::GestureClick::new();
        let this = Rc::downgrade(self);
        let changed = on_change.clone();
        click.connect_pressed(move |_, _, x, _| {
            let Some(this) = this.upgrade() else { return };
            let day = this.day_at(x, this.header.width() as f64);
            if let Some(day) = day {
                let mut s = this.state.borrow_mut();
                s.date = day;
                s.view = ViewKind::Day;
            }
            changed();
        });
        self.header.add_controller(click);

        for (area, hits) in [
            (self.grid.clone(), self.grid_hits.clone()),
            (self.all_day.clone(), self.all_day_hits.clone()),
        ] {
            let click = gtk::GestureClick::new();
            let this = Rc::downgrade(self);
            let changed = on_change.clone();
            let target = area.clone();
            click.connect_pressed(move |_, _, x, y| {
                let Some(this) = this.upgrade() else { return };
                this.grid.grab_focus();
                let hit = hit_at(&hits.borrow(), x, y);
                {
                    let mut s = this.state.borrow_mut();
                    if let Some(day) = this.day_at(x, target.width() as f64) {
                        s.date = day;
                    }
                    s.selected = hit.as_ref().map(|h| h.target.clone());
                }
                changed();
                if let Some(hit) = hit {
                    this.show_details_at(&target, hit.rect());
                }
            });
            area.add_controller(click);
        }
    }

    fn day_at(&self, x: f64, width: f64) -> Option<NaiveDate> {
        let days = self.state.borrow().visible_days();
        let col_w = (width - GUTTER) / days.len() as f64;
        if x < GUTTER {
            return None;
        }
        days.get(((x - GUTTER) / col_w) as usize).copied()
    }

    /// Redraw after the state changed.
    pub fn refresh(&self) {
        let state = self.state.borrow();
        let days = state.visible_days();
        let occ = state.occurrences(&days);
        let lanes = layout::lanes(&layout::all_day_bars(&occ, &days, state.local)).max(1);
        self.all_day
            .set_content_height((lanes as f64 * LANE_HEIGHT) as i32 + 6);
        self.header.queue_draw();
        self.all_day.queue_draw();
        self.grid.queue_draw();
    }

    pub fn focus(&self) {
        self.grid.grab_focus();
    }

    /// Scroll so `minutes` past midnight is a third of the way down.
    pub fn scroll_to_minutes(&self, minutes: f64) {
        let adj = self.scroller.vadjustment();
        let y = minutes / 60.0 * HOUR_HEIGHT - adj.page_size() / 3.0;
        adj.set_value(y.clamp(adj.lower(), adj.upper() - adj.page_size()));
    }

    /// Show the current time if today is visible, otherwise start just
    /// before the first event (or 07:00, whichever is earlier).
    pub fn scroll_to_now_or_morning(&self) {
        let state = self.state.borrow();
        let now = Utc::now().with_timezone(&state.local);
        let days = state.visible_days();
        if days.contains(&now.date_naive()) {
            drop(state);
            self.scroll_to_minutes((now.hour() * 60 + now.minute()) as f64);
            return;
        }
        let occ = state.occurrences(&days);
        let first = days
            .iter()
            .flat_map(|d| layout::day_segments(&occ, *d, state.local, 0.0))
            .map(|s| s.start_min)
            .fold(7.0 * 60.0, f64::min);
        drop(state);
        let adj = self.scroller.vadjustment();
        let y = (first - 30.0).max(0.0) / 60.0 * HOUR_HEIGHT;
        adj.set_value(y.clamp(adj.lower(), adj.upper() - adj.page_size()));
    }

    /// Scroll the selected occurrence into view if it is in the grid.
    pub fn reveal_selected(&self) {
        let state = self.state.borrow();
        let Some(occ) = state.selected_occurrence() else {
            return;
        };
        if layout::is_all_day_row(&occ) {
            return;
        }
        let start = occ.start.to_tz(state.local);
        let y0 = (start.hour() * 60 + start.minute()) as f64 / 60.0 * HOUR_HEIGHT;
        let y1 = y0 + (occ.duration().num_minutes() as f64 / 60.0 * HOUR_HEIGHT).max(24.0);
        drop(state);
        let adj = self.scroller.vadjustment();
        if y0 < adj.value() || y1 > adj.value() + adj.page_size() {
            adj.set_value((y0 - 24.0).clamp(adj.lower(), adj.upper() - adj.page_size()));
        }
    }

    /// Open the details popover for the selected occurrence.
    pub fn show_selected_details(&self) {
        let key = self.state.borrow().selected.clone();
        let Some(key) = key else { return };
        for (area, hits) in [
            (&self.grid, &self.grid_hits),
            (&self.all_day, &self.all_day_hits),
        ] {
            let hit = hits.borrow().iter().find(|h| h.target == key).cloned();
            if let Some(hit) = hit {
                self.show_details_at(area, hit.rect());
                return;
            }
        }
    }

    fn show_details_at(&self, area: &gtk::DrawingArea, rect: gtk::gdk::Rectangle) {
        let state = self.state.borrow();
        if let Some(occ) = state.selected_occurrence() {
            details::show(area, rect, &occ, &state);
        }
    }

    fn draw_header(&self, area: &gtk::DrawingArea, cr: &cairo::Context, width: f64) {
        let state = self.state.borrow();
        let pal = Palette::of(area);
        let days = state.visible_days();
        let col_w = (width - GUTTER) / days.len() as f64;
        let today = Utc::now().with_timezone(&state.local).date_naive();

        for (i, day) in days.iter().enumerate() {
            let x = GUTTER + i as f64 * col_w;
            let is_today = *day == today;
            let is_focus = *day == state.date && days.len() > 1;

            let name = draw::layout(
                area,
                &day.format("%a").to_string().to_uppercase(),
                &TextStyle::SMALL,
                col_w,
                1,
            );
            pal.fg(cr, if is_today { 1.0 } else { 0.6 });
            let nw = name.pixel_size().0 as f64;
            draw::show_layout(cr, &name, x + (col_w - nw) / 2.0, 6.0);

            let num = draw::layout(
                area,
                &day.format("%-d").to_string(),
                &TextStyle {
                    scale: 1.4,
                    bold: is_today,
                    strikethrough: false,
                },
                col_w,
                1,
            );
            let (nw, nh) = num.pixel_size();
            let cx = x + col_w / 2.0;
            let cy = 34.0;
            if is_today {
                pal.accent(cr, 1.0);
                cr.arc(cx, cy, 15.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_source_rgb(1.0, 1.0, 1.0);
            } else {
                pal.fg(cr, 0.9);
            }
            draw::show_layout(cr, &num, cx - nw as f64 / 2.0, cy - nh as f64 / 2.0);

            if is_focus {
                pal.accent(cr, 0.8);
                cr.rectangle(x + 8.0, HEADER_HEIGHT as f64 - 3.0, col_w - 16.0, 3.0);
                let _ = cr.fill();
            }
        }
    }

    fn draw_all_day(&self, area: &gtk::DrawingArea, cr: &cairo::Context, width: f64, height: f64) {
        let state = self.state.borrow();
        let pal = Palette::of(area);
        let days = state.visible_days();
        let occ = state.occurrences(&days);
        let col_w = (width - GUTTER) / days.len() as f64;

        let label = draw::layout(area, "all day", &TextStyle::SMALL, GUTTER - 8.0, 1);
        pal.fg(cr, 0.5);
        draw::show_layout(cr, &label, 4.0, 5.0);

        pal.fg(cr, 0.1);
        for i in 0..=days.len() {
            let x = (GUTTER + i as f64 * col_w).round() + 0.5;
            cr.move_to(x, 0.0);
            cr.line_to(x, height);
        }
        let _ = cr.stroke();

        let mut hits = self.all_day_hits.borrow_mut();
        hits.clear();
        for bar in layout::all_day_bars(&occ, &days, state.local) {
            let o = &occ[bar.index];
            let x = GUTTER + bar.first as f64 * col_w + 2.0;
            let w = (bar.last - bar.first + 1) as f64 * col_w - 4.0;
            let y = 3.0 + bar.lane as f64 * LANE_HEIGHT;
            let h = LANE_HEIGHT - 3.0;
            let selected = state.selected.as_ref().is_some_and(|k| k.matches(o));
            draw_block(area, cr, &pal, o, &state, x, y, w, h, selected, true);
            hits.push(Hit {
                x,
                y,
                w,
                h,
                target: OccurrenceKey::of(o),
            });
        }
    }

    fn draw_grid(&self, area: &gtk::DrawingArea, cr: &cairo::Context, width: f64) {
        let state = self.state.borrow();
        let pal = Palette::of(area);
        let days = state.visible_days();
        let occ = state.occurrences(&days);
        let col_w = (width - GUTTER) / days.len() as f64;
        let height = 24.0 * HOUR_HEIGHT;
        let now = Utc::now().with_timezone(&state.local);
        let today = now.date_naive();

        // today and focused day backgrounds
        for (i, day) in days.iter().enumerate() {
            let x = GUTTER + i as f64 * col_w;
            if *day == today {
                pal.accent(cr, 0.06);
                cr.rectangle(x, 0.0, col_w, height);
                let _ = cr.fill();
            }
        }

        // hour lines and labels
        for hour in 0..24 {
            let y = hour as f64 * HOUR_HEIGHT;
            pal.fg(cr, 0.12);
            cr.move_to(GUTTER - 6.0, y.round() + 0.5);
            cr.line_to(width, y.round() + 0.5);
            let _ = cr.stroke();
            pal.fg(cr, 0.05);
            cr.move_to(GUTTER, (y + HOUR_HEIGHT / 2.0).round() + 0.5);
            cr.line_to(width, (y + HOUR_HEIGHT / 2.0).round() + 0.5);
            let _ = cr.stroke();
            if hour > 0 {
                let label = draw::layout(
                    area,
                    &format!("{hour:02}:00"),
                    &TextStyle::SMALL,
                    GUTTER - 10.0,
                    1,
                );
                let (lw, lh) = label.pixel_size();
                pal.fg(cr, 0.55);
                draw::show_layout(cr, &label, GUTTER - 10.0 - lw as f64, y - lh as f64 / 2.0);
            }
        }

        // day separators
        pal.fg(cr, 0.12);
        for i in 0..=days.len() {
            let x = (GUTTER + i as f64 * col_w).round() + 0.5;
            cr.move_to(x, 0.0);
            cr.line_to(x, height);
        }
        let _ = cr.stroke();

        // events
        let mut hits = self.grid_hits.borrow_mut();
        hits.clear();
        for (i, day) in days.iter().enumerate() {
            let x0 = GUTTER + i as f64 * col_w;
            let inner = col_w - 6.0;
            for seg in layout::day_segments(&occ, *day, state.local, MIN_EVENT_MINUTES) {
                let o = &occ[seg.index];
                let w = inner / seg.columns as f64;
                let x = x0 + 2.0 + seg.column as f64 * w;
                let y = seg.start_min / 60.0 * HOUR_HEIGHT + 1.0;
                let h = (seg.end_min - seg.start_min) / 60.0 * HOUR_HEIGHT - 2.0;
                let selected = state.selected.as_ref().is_some_and(|k| k.matches(o));
                draw_block(area, cr, &pal, o, &state, x, y, w - 2.0, h, selected, false);
                hits.push(Hit {
                    x,
                    y,
                    w: w - 2.0,
                    h,
                    target: OccurrenceKey::of(o),
                });
            }
        }

        // now line
        if let Some(i) = days.iter().position(|d| *d == today) {
            let y = (now.hour() * 60 + now.minute()) as f64 / 60.0 * HOUR_HEIGHT;
            let x = GUTTER + i as f64 * col_w;
            cr.set_source_rgba(0.88, 0.11, 0.14, 0.35);
            cr.set_line_width(1.0);
            cr.move_to(GUTTER, y);
            cr.line_to(width, y);
            let _ = cr.stroke();
            cr.set_source_rgb(0.88, 0.11, 0.14);
            cr.set_line_width(2.0);
            cr.move_to(x, y);
            cr.line_to(x + col_w, y);
            let _ = cr.stroke();
            cr.arc(x, y, 5.0, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
    }
}

/// Draw one event block: a tinted box with a solid stripe and its title.
#[allow(clippy::too_many_arguments)]
fn draw_block(
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    pal: &Palette,
    o: &Occurrence,
    state: &crate::state::State,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    selected: bool,
    compact: bool,
) {
    let color = state.color(&o.calendar);
    let cancelled = format::is_cancelled(o);
    let alpha = if cancelled { 0.5 } else { 1.0 };

    cr.save().ok();
    draw::rounded_rect(cr, x, y, w, h, 5.0);
    cr.clip_preserve();
    if selected {
        draw::set_rgb(cr, color, 1.0);
    } else {
        pal.event_fill(cr, color);
    }
    let _ = cr.fill();
    draw::set_rgb(cr, color, alpha);
    cr.rectangle(x, y, 3.0, h);
    let _ = cr.fill();
    if o.status.as_deref() == Some("TENTATIVE") && !selected {
        draw::set_rgb(cr, color, 0.9);
        cr.set_line_width(1.0);
        cr.set_dash(&[3.0, 3.0], 0.0);
        draw::rounded_rect(cr, x + 0.5, y + 0.5, w - 1.0, h - 1.0, 5.0);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }

    let text_color = |cr: &cairo::Context, a: f64| {
        if selected {
            draw::set_rgb(cr, draw::contrasting(color), a);
        } else {
            pal.fg(cr, a * alpha);
        }
    };

    let title = match o.kind {
        Kind::Todo => format!("{} {}", format::todo_mark(o), o.summary),
        _ => o.summary.clone(),
    };
    let tw = w - 10.0;
    let style = TextStyle {
        scale: 0.9,
        bold: true,
        strikethrough: cancelled,
    };
    if compact {
        let text = if o.all_day {
            title
        } else {
            format!("{} {}", format::time(&o.start, state.local), title)
        };
        let l = draw::layout(area, &text, &style, tw, 1);
        text_color(cr, 1.0);
        draw::show_layout(cr, &l, x + 7.0, y + (h - draw::layout_height(&l)) / 2.0);
    } else {
        let one_line = h < 34.0;
        let time = format::time(&o.start, state.local);
        if one_line {
            let l = draw::layout(area, &format!("{title}  {time}"), &style, tw, 1);
            text_color(cr, 1.0);
            draw::show_layout(cr, &l, x + 7.0, y + 2.0);
        } else {
            let line_h = 15.0;
            let max_title_lines = (((h - 4.0) / line_h) as i32 - 1).max(1);
            let l = draw::layout(area, &title, &style, tw, max_title_lines);
            text_color(cr, 1.0);
            draw::show_layout(cr, &l, x + 7.0, y + 3.0);
            let used = draw::layout_height(&l);
            let mut sub = format!("{time} – {}", format::time(&o.end, state.local));
            if let Some(loc) = o.location.as_deref().filter(|l| !l.is_empty()) {
                sub = format!("{sub} · {loc}");
            }
            let s = draw::layout(area, &sub, &TextStyle::SMALL, tw, 1);
            if used + draw::layout_height(&s) + 4.0 <= h {
                text_color(cr, 0.8);
                draw::show_layout(cr, &s, x + 7.0, y + 3.0 + used);
            }
        }
    }
    if selected {
        pal.fg(cr, 0.9);
        cr.set_line_width(2.0);
        draw::rounded_rect(cr, x + 1.0, y + 1.0, w - 2.0, h - 2.0, 4.0);
        let _ = cr.stroke();
    }
    cr.restore().ok();
}

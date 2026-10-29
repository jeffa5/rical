//! The year view: twelve small months, each day tinted by how busy it is,
//! for seeing a year at a glance and picking a day.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{Datelike, Days, NaiveDate, Utc};
use gtk::cairo;

use crate::{
    draw::{self, Hit, Palette, TextStyle, hit_at},
    layout::{self, ViewKind},
    state::SharedState,
};

/// Busy hours at which a day gets the strongest tint.
const BUSY_HOURS_FULL: f64 = 8.0;
const MONTH_TITLE: f64 = 26.0;
const WEEKDAYS: f64 = 18.0;
const GAP: f64 = 18.0;

pub struct YearView {
    pub area: gtk::DrawingArea,
    state: SharedState,
    hits: Rc<RefCell<Vec<Hit<NaiveDate>>>>,
}

impl YearView {
    pub fn new(state: SharedState, on_change: Rc<dyn Fn()>) -> Rc<Self> {
        let area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .can_focus(true)
            .build();
        area.update_property(&[gtk::accessible::Property::Label("Year")]);
        let this = Rc::new(Self {
            area,
            state,
            hits: Default::default(),
        });
        let weak = Rc::downgrade(&this);
        this.area.set_draw_func(move |area, cr, w, h| {
            if let Some(this) = weak.upgrade() {
                this.draw(area, cr, w as f64, h as f64);
            }
        });
        // a click focuses the day; a double click opens it
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&this);
        click.connect_pressed(move |_, n_press, x, y| {
            let Some(this) = weak.upgrade() else { return };
            this.area.grab_focus();
            let day = hit_at(&this.hits.borrow(), x, y).map(|h| h.target);
            let Some(day) = day else { return };
            {
                let mut s = this.state.borrow_mut();
                s.date = day;
                s.selected = None;
                if n_press == 2 {
                    s.view = ViewKind::Day;
                }
            }
            on_change();
        });
        this.area.add_controller(click);
        this
    }

    pub fn refresh(&self) {
        self.area.queue_draw();
    }

    pub fn focus(&self) {
        self.area.grab_focus();
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &cairo::Context, width: f64, height: f64) {
        let state = self.state.borrow();
        let pal = Palette::of(area);
        let days = state.visible_days();
        let occ = state.occurrences(&days);
        let today = Utc::now().with_timezone(&state.local).date_naive();
        let year = state.date.year();

        // as many months across as fit, at least two
        let cols = ((width / 230.0).floor() as usize).clamp(2, 6);
        let rows = 12usize.div_ceil(cols);
        let month_w = (width - GAP * (cols as f64 + 1.0)) / cols as f64;
        let month_h = (height - GAP * (rows as f64 + 1.0)) / rows as f64;
        let cell_w = month_w / 7.0;
        let cell_h = ((month_h - MONTH_TITLE - WEEKDAYS) / 6.0).max(1.0);

        let mut hits = self.hits.borrow_mut();
        hits.clear();
        for m in 0..12u32 {
            let (col, row) = (m as usize % cols, m as usize / cols);
            let x0 = GAP + col as f64 * (month_w + GAP);
            let y0 = GAP + row as f64 * (month_h + GAP);
            let first = NaiveDate::from_ymd_opt(year, m + 1, 1).unwrap();

            let title = draw::layout(
                area,
                &first.format("%B").to_string(),
                &TextStyle {
                    scale: 1.0,
                    bold: true,
                    strikethrough: false,
                },
                month_w,
                1,
            );
            if first.month() == state.date.month() {
                pal.accent(cr, 1.0);
            } else {
                pal.fg(cr, 0.9);
            }
            draw::show_layout(cr, &title, x0 + 4.0, y0);

            let week_start = layout::week_start(first);
            for i in 0..7 {
                let d = week_start + Days::new(i);
                let l = draw::layout(
                    area,
                    &d.format("%a").to_string()[..1],
                    &TextStyle::SMALL,
                    cell_w,
                    1,
                );
                let lw = l.pixel_size().0 as f64;
                pal.fg(cr, 0.5);
                draw::show_layout(
                    cr,
                    &l,
                    x0 + i as f64 * cell_w + (cell_w - lw) / 2.0,
                    y0 + MONTH_TITLE,
                );
            }

            let mut day = first;
            while day.month() == m + 1 {
                let i = (day - week_start).num_days() as usize;
                let x = x0 + (i % 7) as f64 * cell_w;
                let y = y0 + MONTH_TITLE + WEEKDAYS + (i / 7) as f64 * cell_h;
                hits.push(Hit {
                    x,
                    y,
                    w: cell_w,
                    h: cell_h,
                    target: day,
                });

                let busy = layout::busy_minutes(&occ, day, state.local) / 60.0;
                if busy > 0.0 {
                    pal.accent(cr, 0.10 + 0.45 * (busy / BUSY_HOURS_FULL).min(1.0));
                    draw::rounded_rect(cr, x + 1.0, y + 1.0, cell_w - 2.0, cell_h - 2.0, 4.0);
                    let _ = cr.fill();
                }
                let (cx, cy) = (x + cell_w / 2.0, y + cell_h / 2.0);
                let radius = (cell_w.min(cell_h) / 2.0 - 1.0).max(4.0);
                if day == today {
                    pal.accent(cr, 1.0);
                    cr.new_sub_path();
                    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                }
                if day == state.date {
                    pal.fg(cr, 0.8);
                    cr.set_line_width(1.5);
                    cr.new_sub_path();
                    cr.arc(cx, cy, radius, 0.0, std::f64::consts::TAU);
                    let _ = cr.stroke();
                }
                let num = draw::layout(
                    area,
                    &day.day().to_string(),
                    &TextStyle {
                        scale: 0.85,
                        bold: day == today,
                        strikethrough: false,
                    },
                    cell_w,
                    1,
                );
                let (nw, nh) = num.pixel_size();
                if day == today {
                    cr.set_source_rgb(1.0, 1.0, 1.0);
                } else {
                    pal.fg(cr, 0.9);
                }
                draw::show_layout(cr, &num, cx - nw as f64 / 2.0, cy - nh as f64 / 2.0);
                day = day.succ_opt().unwrap();
            }
        }
    }
}

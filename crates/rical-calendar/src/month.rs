//! The month view: six weeks of day cells with event chips and a busyness
//! tint per day.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{Datelike, NaiveDate, Utc};
use gtk::cairo;

use crate::{
    details,
    draw::{self, Hit, Palette, TextStyle, hit_at},
    format,
    layout::{self, ViewKind},
    state::{OccurrenceKey, SharedState},
};

const WEEKDAY_HEIGHT: f64 = 26.0;
const DAY_NUMBER_HEIGHT: f64 = 24.0;
const CHIP_HEIGHT: f64 = 19.0;
/// Busy hours at which a day gets the strongest tint.
const BUSY_HOURS_FULL: f64 = 12.0;

#[derive(Debug, Clone, PartialEq)]
enum Target {
    Occurrence(OccurrenceKey),
    Day(NaiveDate),
}

pub struct MonthView {
    pub area: gtk::DrawingArea,
    state: SharedState,
    hits: Rc<RefCell<Vec<Hit<Target>>>>,
}

impl MonthView {
    pub fn new(state: SharedState, on_change: Rc<dyn Fn()>) -> Rc<Self> {
        let area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .can_focus(true)
            .build();
        area.update_property(&[gtk::accessible::Property::Label("Month")]);
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

        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&this);
        click.connect_pressed(move |_, n_press, x, y| {
            let Some(this) = weak.upgrade() else { return };
            this.area.grab_focus();
            let hits = this.hits.borrow().clone();
            // the topmost hit is the chip if one was clicked, else the day
            let cell = hits.iter().find_map(|h| match &h.target {
                Target::Day(d) if h.contains(x, y) => Some((*d, h.x, h.y)),
                _ => None,
            });
            let day = cell.map(|(d, _, _)| d);
            let chip = hit_at(&hits, x, y).and_then(|h| match &h.target {
                Target::Occurrence(k) => Some((k.clone(), h.rect())),
                Target::Day(_) => None,
            });
            // the day's number opens the day; the rest of it adds an event
            let on_number = cell.is_some_and(|(_, cx, cy)| {
                x - cx < DAY_NUMBER_HEIGHT + 6.0 && y - cy < DAY_NUMBER_HEIGHT
            });
            {
                let mut s = this.state.borrow_mut();
                if let Some(day) = day {
                    s.date = day;
                }
                s.selected = chip.as_ref().map(|(k, _)| k.clone());
                if on_number && chip.is_none() {
                    s.view = ViewKind::Day;
                }
            }
            on_change();
            match chip {
                Some((_, rect)) => this.show_details_at(rect),
                None if n_press == 1 && day.is_some() && !on_number => {
                    let _ = this
                        .area
                        .activate_action("win.run", Some(&"new".to_variant()));
                }
                None => {}
            }
        });
        this.area.add_controller(click);
        let weak = Rc::downgrade(&this);
        details::edit_on_double_click(&this.area, move |x, y| {
            weak.upgrade().is_some_and(|t| {
                hit_at(&t.hits.borrow(), x, y)
                    .is_some_and(|h| matches!(h.target, Target::Occurrence(_)))
            })
        });
        this
    }

    pub fn refresh(&self) {
        self.area.queue_draw();
    }

    pub fn focus(&self) {
        self.area.grab_focus();
    }

    pub fn show_selected_details(&self) {
        let key = self.state.borrow().selected.clone();
        let Some(key) = key else { return };
        let rect = self
            .hits
            .borrow()
            .iter()
            .find(|h| h.target == Target::Occurrence(key.clone()))
            .map(|h| h.rect());
        if let Some(rect) = rect {
            self.show_details_at(rect);
        }
    }

    fn show_details_at(&self, rect: gtk::gdk::Rectangle) {
        let state = self.state.borrow();
        if let Some(occ) = state.selected_occurrence() {
            details::show(&self.area, rect, &occ, &state);
        }
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &cairo::Context, width: f64, height: f64) {
        let state = self.state.borrow();
        let pal = Palette::of(area);
        let days = state.visible_days();
        let occ = state.occurrences(&days);
        let today = Utc::now().with_timezone(&state.local).date_naive();
        let month = state.date.month();

        let col_w = width / 7.0;
        let row_h = (height - WEEKDAY_HEIGHT) / 6.0;

        for (i, day) in days.iter().take(7).enumerate() {
            let l = draw::layout(
                area,
                &day.format("%a").to_string().to_uppercase(),
                &TextStyle::SMALL,
                col_w,
                1,
            );
            pal.fg(cr, 0.6);
            let lw = l.pixel_size().0 as f64;
            draw::show_layout(cr, &l, i as f64 * col_w + (col_w - lw) / 2.0, 5.0);
        }

        let mut hits = self.hits.borrow_mut();
        hits.clear();

        // cells
        for (i, day) in days.iter().enumerate() {
            let x = (i % 7) as f64 * col_w;
            let y = WEEKDAY_HEIGHT + (i / 7) as f64 * row_h;
            hits.push(Hit {
                x,
                y,
                w: col_w,
                h: row_h,
                target: Target::Day(*day),
            });

            let busy = layout::busy_minutes(&occ, *day, state.local) / 60.0;
            if busy > 0.0 {
                pal.accent(cr, 0.02 + 0.10 * (busy / BUSY_HOURS_FULL).min(1.0));
                cr.rectangle(x, y, col_w, row_h);
                let _ = cr.fill();
            }

            let in_month = day.month() == month;
            let num = draw::layout(
                area,
                &day.format("%-d").to_string(),
                &TextStyle {
                    scale: 0.95,
                    bold: *day == today,
                    strikethrough: false,
                },
                col_w,
                1,
            );
            let (nw, nh) = num.pixel_size();
            if *day == today {
                pal.accent(cr, 1.0);
                cr.new_sub_path();
                cr.arc(x + 16.0, y + 13.0, 11.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_source_rgb(1.0, 1.0, 1.0);
            } else {
                pal.fg(cr, if in_month { 0.9 } else { 0.35 });
            }
            draw::show_layout(
                cr,
                &num,
                x + 16.0 - nw as f64 / 2.0,
                y + 13.0 - nh as f64 / 2.0,
            );
        }

        // grid lines
        pal.fg(cr, 0.12);
        cr.set_line_width(1.0);
        for r in 0..=6 {
            let y = (WEEKDAY_HEIGHT + r as f64 * row_h).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(width, y);
        }
        for c in 1..7 {
            let x = (c as f64 * col_w).round() + 0.5;
            cr.move_to(x, WEEKDAY_HEIGHT);
            cr.line_to(x, height);
        }
        let _ = cr.stroke();

        // chips, week by week
        let max_lanes = ((row_h - DAY_NUMBER_HEIGHT - CHIP_HEIGHT) / CHIP_HEIGHT)
            .floor()
            .max(0.0) as usize;
        for (w, week) in days.chunks(7).enumerate() {
            let y0 = WEEKDAY_HEIGHT + w as f64 * row_h + DAY_NUMBER_HEIGHT;
            let bars = layout::month_week_bars(&occ, week, state.local);
            let mut hidden = [0usize; 7];
            for bar in &bars {
                if bar.lane >= max_lanes {
                    for h in &mut hidden[bar.first..=bar.last] {
                        *h += 1;
                    }
                    continue;
                }
                let o = &occ[bar.index];
                let x = bar.first as f64 * col_w + 3.0;
                let cw = (bar.last - bar.first + 1) as f64 * col_w - 6.0;
                let y = y0 + bar.lane as f64 * CHIP_HEIGHT;
                let h = CHIP_HEIGHT - 2.0;
                let selected = state.selected.as_ref().is_some_and(|k| k.matches(o));
                let color = state.display_color(o);
                let spans = bar.first != bar.last || layout::is_all_day_row(o);

                if spans || selected {
                    draw::rounded_rect(cr, x, y, cw, h, 4.0);
                    if selected {
                        draw::set_rgb(cr, color, 1.0);
                    } else {
                        pal.event_fill(cr, color);
                    }
                    let _ = cr.fill();
                }
                let mut tx = x + 6.0;
                let mark = if selected {
                    draw::contrasting(color)
                } else {
                    color
                };
                if o.kind == rical_core::Kind::Todo {
                    // tasks: a tick circle instead of a dot
                    if spans || selected {
                        pal.fg(cr, 0.85);
                        if selected {
                            draw::set_rgb(cr, mark, 1.0);
                        }
                    } else {
                        draw::set_rgb(cr, mark, 1.0);
                    }
                    draw::task_circle(cr, x + 9.0, y + h / 2.0, 4.5, format::is_done(o));
                    tx = x + 17.0;
                } else if !spans {
                    draw::set_rgb(cr, mark, 1.0);
                    cr.new_sub_path();
                    cr.arc(x + 6.0, y + h / 2.0, 3.5, 0.0, std::f64::consts::TAU);
                    let _ = cr.fill();
                    tx = x + 13.0;
                }
                let text = if spans {
                    format::label(o, state.local)
                } else {
                    format!(
                        "{} {}",
                        format::time(&o.start, state.local),
                        format::label(o, state.local)
                    )
                };
                let l = draw::layout(
                    area,
                    &text,
                    &TextStyle {
                        scale: 0.85,
                        bold: false,
                        strikethrough: format::is_inactive(o),
                    },
                    cw - (tx - x) - 2.0,
                    1,
                );
                if selected {
                    draw::set_rgb(cr, draw::contrasting(color), 1.0);
                } else {
                    pal.fg(cr, 0.9);
                }
                draw::show_layout(cr, &l, tx, y + (h - draw::layout_height(&l)) / 2.0);
                hits.push(Hit {
                    x,
                    y,
                    w: cw,
                    h,
                    target: Target::Occurrence(OccurrenceKey::of(o)),
                });
            }
            for (d, n) in hidden.iter().enumerate() {
                if *n == 0 {
                    continue;
                }
                let l = draw::layout(
                    area,
                    &format!("+{n} more"),
                    &TextStyle::SMALL,
                    col_w - 12.0,
                    1,
                );
                pal.fg(cr, 0.6);
                draw::show_layout(
                    cr,
                    &l,
                    d as f64 * col_w + 8.0,
                    y0 + max_lanes as f64 * CHIP_HEIGHT + 1.0,
                );
            }
        }

        // focused day
        if let Some(i) = days.iter().position(|d| *d == state.date) {
            let x = (i % 7) as f64 * col_w;
            let y = WEEKDAY_HEIGHT + (i / 7) as f64 * row_h;
            pal.accent(cr, if area.has_focus() { 0.9 } else { 0.5 });
            cr.set_line_width(2.0);
            cr.rectangle(x + 1.0, y + 1.0, col_w - 2.0, row_h - 2.0);
            let _ = cr.stroke();
        }
    }
}

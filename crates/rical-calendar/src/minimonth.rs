//! The sidebar's month: a date picker with a dot under each day for every
//! calendar that has something on it.

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use adw::prelude::*;
use chrono::{Datelike, Days, Months, NaiveDate};
use gtk::cairo;

use crate::{
    draw::{self, Palette, TextStyle},
    layout,
    state::SharedState,
};

const HEADER: f64 = 22.0;
const ROW: f64 = 30.0;
/// Width of the week-number column, as a share of a day column.
const WEEK_COLUMN: f64 = 0.8;
const MAX_DOTS: usize = 3;

/// The calendars (ids) with something on each day.
type Dots = BTreeMap<NaiveDate, Vec<String>>;

pub struct MiniMonth {
    pub widget: gtk::Box,
    title: gtk::Label,
    area: gtk::DrawingArea,
    state: SharedState,
    /// First of the month shown; follows the focused day unless browsed.
    month: Cell<NaiveDate>,
    /// Calendars with something on each day, for (generation, month).
    dots: RefCell<(Option<(u64, NaiveDate)>, Dots)>,
}

fn first_of(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap()
}

fn grid_days(month: NaiveDate) -> Vec<NaiveDate> {
    let first = layout::week_start(month);
    (0..42).map(|i| first + Days::new(i)).collect()
}

impl MiniMonth {
    pub fn new(state: SharedState, on_pick: Rc<dyn Fn(NaiveDate)>) -> Rc<Self> {
        let title = gtk::Label::builder()
            .css_classes(["heading"])
            .hexpand(true)
            .build();
        let prev = gtk::Button::builder()
            .icon_name("go-previous-symbolic")
            .css_classes(["flat", "circular"])
            .tooltip_text("Previous month")
            .build();
        let next = gtk::Button::builder()
            .icon_name("go-next-symbolic")
            .css_classes(["flat", "circular"])
            .tooltip_text("Next month")
            .build();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        header.append(&prev);
        header.append(&title);
        header.append(&next);
        let area = gtk::DrawingArea::builder()
            .content_height((HEADER + 6.0 * ROW) as i32)
            .hexpand(true)
            .build();
        area.update_property(&[gtk::accessible::Property::Label("Month")]);
        let widget = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .margin_start(6)
            .margin_end(6)
            .build();
        widget.append(&header);
        widget.append(&area);

        let today = state.borrow().date;
        let this = Rc::new(Self {
            widget,
            title,
            area,
            state,
            month: Cell::new(first_of(today)),
            dots: RefCell::new((None, BTreeMap::new())),
        });

        for (button, forward) in [(prev, false), (next, true)] {
            let t = Rc::downgrade(&this);
            button.connect_clicked(move |_| {
                if let Some(t) = t.upgrade() {
                    let m = t.month.get();
                    t.month.set(if forward {
                        m + Months::new(1)
                    } else {
                        m - Months::new(1)
                    });
                    t.redraw();
                }
            });
        }

        let t = Rc::downgrade(&this);
        this.area.set_draw_func(move |area, cr, w, _| {
            if let Some(t) = t.upgrade() {
                t.draw(area, cr, w as f64);
            }
        });

        let click = gtk::GestureClick::new();
        let t = Rc::downgrade(&this);
        click.connect_pressed(move |_, _, x, y| {
            let Some(t) = t.upgrade() else { return };
            if let Some(day) = t.day_at(x, y) {
                on_pick(day);
            }
        });
        this.area.add_controller(click);
        this
    }

    /// Follow the focused day and redraw.
    pub fn refresh(&self) {
        self.month.set(first_of(self.state.borrow().date));
        self.redraw();
    }

    fn redraw(&self) {
        self.title
            .set_label(&self.month.get().format("%B %Y").to_string());
        self.area.queue_draw();
    }

    fn col_width(&self) -> f64 {
        self.area.width() as f64 / (7.0 + WEEK_COLUMN)
    }

    fn day_at(&self, x: f64, y: f64) -> Option<NaiveDate> {
        let cw = self.col_width();
        let col = ((x - cw * WEEK_COLUMN) / cw).floor();
        let row = ((y - HEADER) / ROW).floor();
        if !(0.0..7.0).contains(&col) || !(0.0..6.0).contains(&row) {
            return None;
        }
        grid_days(self.month.get())
            .get(row as usize * 7 + col as usize)
            .copied()
    }

    /// Calendars (ids, in sidebar order) with something on each shown day.
    fn dots(&self) -> BTreeMap<NaiveDate, Vec<String>> {
        let key = (self.state.borrow().generation(), self.month.get());
        if self.dots.borrow().0 == Some(key) {
            return self.dots.borrow().1.clone();
        }
        let state = self.state.borrow();
        let days = grid_days(self.month.get());
        let range = layout::days_range(&days, state.local);
        let mut by_day: BTreeMap<NaiveDate, BTreeSet<usize>> = BTreeMap::new();
        let order = |id: &str| {
            state
                .vdir
                .calendars
                .iter()
                .position(|c| c.id == id)
                .unwrap_or(usize::MAX)
        };
        for o in state.vdir.query(
            &range,
            &[rical_core::Kind::Event, rical_core::Kind::Todo],
            state.local,
        ) {
            if state.hidden.contains(&o.calendar) {
                continue;
            }
            for day in &days {
                let on = layout::day_span(&o, std::slice::from_ref(day), state.local).is_some();
                if on {
                    by_day.entry(*day).or_default().insert(order(&o.calendar));
                }
            }
        }
        let dots = by_day
            .into_iter()
            .map(|(d, cals)| {
                let ids = cals
                    .into_iter()
                    .filter_map(|i| state.vdir.calendars.get(i).map(|c| c.id.clone()))
                    .collect();
                (d, ids)
            })
            .collect::<BTreeMap<_, _>>();
        *self.dots.borrow_mut() = (Some(key), dots.clone());
        dots
    }

    fn draw(&self, area: &gtk::DrawingArea, cr: &cairo::Context, width: f64) {
        let dots = self.dots();
        let state = self.state.borrow();
        let pal = Palette::of(area);
        let month = self.month.get();
        let today = state.today();
        let cw = width / (7.0 + WEEK_COLUMN);
        let x0 = cw * WEEK_COLUMN;
        let days = grid_days(month);

        for (i, d) in days.iter().take(7).enumerate() {
            let l = draw::layout(
                area,
                &d.format("%a").to_string()[..2],
                &TextStyle::SMALL,
                cw,
                1,
            );
            let lw = l.pixel_size().0 as f64;
            pal.fg(cr, 0.55);
            draw::show_layout(cr, &l, x0 + i as f64 * cw + (cw - lw) / 2.0, 2.0);
        }

        for (i, day) in days.iter().enumerate() {
            let (row, col) = (i / 7, i % 7);
            let cx = x0 + col as f64 * cw + cw / 2.0;
            let y = HEADER + row as f64 * ROW;
            if col == 0 {
                let wk = draw::layout(
                    area,
                    &day.format("%V").to_string(),
                    &TextStyle::SMALL,
                    x0,
                    1,
                );
                let (ww, _) = wk.pixel_size();
                pal.fg(cr, 0.4);
                draw::show_layout(cr, &wk, (x0 - ww as f64) / 2.0, y + 4.0);
            }
            let is_today = *day == today;
            let num = draw::layout(
                area,
                &day.day().to_string(),
                &TextStyle {
                    scale: 0.9,
                    bold: is_today,
                    strikethrough: false,
                },
                cw,
                1,
            );
            let (nw, nh) = num.pixel_size();
            let ny = y + 11.0;
            if is_today {
                pal.accent(cr, 1.0);
                cr.new_sub_path();
                cr.arc(cx, ny, 10.0, 0.0, std::f64::consts::TAU);
                let _ = cr.fill();
                cr.set_source_rgb(1.0, 1.0, 1.0);
            } else {
                pal.fg(
                    cr,
                    if day.month() == month.month() {
                        0.9
                    } else {
                        0.35
                    },
                );
            }
            draw::show_layout(cr, &num, cx - nw as f64 / 2.0, ny - nh as f64 / 2.0);
            if *day == state.date && !is_today {
                pal.accent(cr, 0.9);
                cr.set_line_width(1.5);
                cr.new_sub_path();
                cr.arc(cx, ny, 10.0, 0.0, std::f64::consts::TAU);
                let _ = cr.stroke();
            }

            if let Some(cals) = dots.get(day) {
                let n = cals.len().min(MAX_DOTS);
                let gap = 6.0;
                let start = cx - (n as f64 - 1.0) * gap / 2.0;
                for (k, id) in cals.iter().take(MAX_DOTS).enumerate() {
                    draw::set_rgb(
                        cr,
                        state.color(id),
                        if day.month() == month.month() {
                            1.0
                        } else {
                            0.4
                        },
                    );
                    cr.new_sub_path();
                    cr.arc(
                        start + k as f64 * gap,
                        y + 25.0,
                        2.2,
                        0.0,
                        std::f64::consts::TAU,
                    );
                    let _ = cr.fill();
                }
            }
        }
    }
}

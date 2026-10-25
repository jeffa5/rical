//! The day and week time grid: a header of days, an all-day row, and a
//! scrollable 24 hour grid with events drawn as blocks sized by duration.
//! Events can be dragged to move or resize them, and dragging on empty grid
//! creates one.

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use chrono::{NaiveDate, TimeDelta, Timelike, Utc};
use gtk::cairo;
use rical_core::{Kind, Occurrence, When};

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
/// The all-day row shows this many lanes before scrolling.
const MAX_ALL_DAY_LANES: f64 = 4.5;
/// Short events are drawn at least this long so their title fits.
const MIN_EVENT_MINUTES: f64 = 22.0;
/// Drags snap to this many minutes.
const SNAP_MINUTES: f64 = 15.0;

type Hits<T> = Rc<RefCell<Vec<Hit<T>>>>;

/// What a finished drag asks for.
pub enum DragAction {
    Move {
        occ: Box<Occurrence>,
        start: When,
        end: When,
    },
    Create {
        start: When,
        end: When,
    },
    /// A task dropped from the sidebar: a time on the grid, or a date on
    /// the all-day row.
    DropTask {
        task: (std::path::PathBuf, String),
        at: When,
    },
}

enum DragKind {
    Move {
        occ: Occurrence,
        /// Minutes between the event's start and the pointer.
        grab_min: f64,
        press_day: NaiveDate,
    },
    Resize {
        occ: Occurrence,
    },
    Create {
        day: NaiveDate,
        start_min: f64,
    },
}

struct Drag {
    kind: DragKind,
    /// In the all-day row rather than the grid.
    all_day: bool,
    hit_rect: Option<gtk::gdk::Rectangle>,
    moved: bool,
    preview: Option<(When, When)>,
}

fn minute_at(y: f64) -> f64 {
    y / HOUR_HEIGHT * 60.0
}

fn snap(minutes: f64) -> f64 {
    (minutes / SNAP_MINUTES).round() * SNAP_MINUTES
}

pub struct TimeGrid {
    pub widget: gtk::Box,
    header: gtk::DrawingArea,
    all_day: gtk::DrawingArea,
    scroller: gtk::ScrolledWindow,
    pub grid: gtk::DrawingArea,
    state: SharedState,
    grid_hits: Hits<OccurrenceKey>,
    all_day_hits: Hits<OccurrenceKey>,
    drag: RefCell<Option<Drag>>,
    on_drag: Rc<dyn Fn(DragAction)>,
}

impl TimeGrid {
    pub fn new(
        state: SharedState,
        on_change: Rc<dyn Fn()>,
        on_drag: Rc<dyn Fn(DragAction)>,
    ) -> Rc<Self> {
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
        // many all-day items (or carried-over tasks) scroll rather than
        // squeezing the grid
        let all_day_scroller = gtk::ScrolledWindow::builder()
            .child(&all_day)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_height(true)
            .max_content_height((MAX_ALL_DAY_LANES * LANE_HEIGHT) as i32 + 6)
            .build();
        widget.append(&all_day_scroller);
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
            drag: RefCell::new(None),
            on_drag,
        });
        this.setup_drawing();
        this.setup_clicks(on_change);
        this.setup_cursor();
        this.setup_drop(&this.grid.clone(), false);
        this.setup_drop(&this.all_day.clone(), true);
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

        self.setup_drag(
            &self.grid.clone(),
            self.grid_hits.clone(),
            false,
            on_change.clone(),
        );
        self.setup_drag(
            &self.all_day.clone(),
            self.all_day_hits.clone(),
            true,
            on_change,
        );
    }

    /// Press selects (and opens details if released without moving); drag
    /// moves or resizes an event, or on empty grid creates one.
    fn setup_drag(
        self: &Rc<Self>,
        area: &gtk::DrawingArea,
        hits: Hits<OccurrenceKey>,
        all_day: bool,
        on_change: Rc<dyn Fn()>,
    ) {
        let gesture = gtk::GestureDrag::new();

        let this = Rc::downgrade(self);
        let target = area.clone();
        let changed = on_change.clone();
        gesture.connect_drag_begin(move |_, x, y| {
            let Some(this) = this.upgrade() else { return };
            this.grid.grab_focus();
            let hit = hit_at(&hits.borrow(), x, y);
            let width = target.width() as f64;
            let day = this.day_at(x, width);
            let kind = {
                let mut s = this.state.borrow_mut();
                if let Some(day) = day {
                    s.date = day;
                }
                s.selected = hit.as_ref().map(|h| h.target.clone());
                let occ = s.selected_occurrence();
                match (&hit, occ, day) {
                    (Some(h), Some(occ), Some(_)) if !all_day && y > h.y + h.h - 6.0 => {
                        Some(DragKind::Resize { occ })
                    }
                    (Some(_), Some(occ), Some(day)) => {
                        let into_day = (occ.start.to_utc(s.local) - layout::day_start(day, s.local))
                            .num_minutes() as f64;
                        Some(DragKind::Move {
                            occ,
                            grab_min: minute_at(y) - into_day,
                            press_day: day,
                        })
                    }
                    (None, _, Some(day)) if !all_day => Some(DragKind::Create {
                        day,
                        start_min: snap(minute_at(y) - 7.5),
                    }),
                    _ => None,
                }
            };
            *this.drag.borrow_mut() = kind.map(|kind| Drag {
                kind,
                all_day,
                hit_rect: hit.map(|h| h.rect()),
                moved: false,
                preview: None,
            });
            changed();
        });

        let this = Rc::downgrade(self);
        let target = area.clone();
        gesture.connect_drag_update(move |g, dx, dy| {
            let Some(this) = this.upgrade() else { return };
            let Some((sx, sy)) = g.start_point() else {
                return;
            };
            let mut drag = this.drag.borrow_mut();
            let Some(drag) = drag.as_mut() else { return };
            if !drag.moved && dx.abs() < 4.0 && dy.abs() < 4.0 {
                return;
            }
            drag.moved = true;
            let (x, y) = (sx + dx, sy + dy);
            let state = this.state.borrow();
            let local = state.local;
            let days = state.visible_days();
            let width = target.width() as f64;
            let day = this
                .day_at(x.clamp(GUTTER, width - 1.0), width)
                .unwrap_or(days[0]);
            let at = |day: NaiveDate, min: f64| {
                When::Zoned(
                    (layout::day_start(day, local) + TimeDelta::minutes(min as i64))
                        .with_timezone(&local),
                )
            };
            drag.preview = match &drag.kind {
                DragKind::Move { occ, press_day, .. } if all_day => {
                    let shift = TimeDelta::days((day - *press_day).num_days());
                    Some((occ.start.add(shift), occ.end.add(shift)))
                }
                DragKind::Move { occ, grab_min, .. } => {
                    let start = at(day, snap(minute_at(y) - grab_min));
                    Some((start, start.add(occ.duration())))
                }
                DragKind::Resize { occ } => {
                    let day = day.max(occ.start.date(local));
                    let end = at(day, snap(minute_at(y)));
                    let min_end = occ.start.add(TimeDelta::minutes(15));
                    let end = if end.to_utc(local) < min_end.to_utc(local) {
                        min_end
                    } else {
                        end
                    };
                    Some((occ.start, end))
                }
                DragKind::Create { day, start_min } => {
                    let here = snap(minute_at(y));
                    let (lo, hi) = (start_min.min(here), start_min.max(here));
                    Some((at(*day, lo), at(*day, hi.max(lo + 15.0))))
                }
            };
            drop(state);
            this.grid.queue_draw();
            this.all_day.queue_draw();
        });

        let this = Rc::downgrade(self);
        let target = area.clone();
        gesture.connect_drag_end(move |_, _, _| {
            let Some(this) = this.upgrade() else { return };
            let Some(drag) = this.drag.borrow_mut().take() else {
                return;
            };
            this.grid.queue_draw();
            this.all_day.queue_draw();
            match (drag.moved, drag.preview, drag.kind) {
                (false, _, DragKind::Move { .. } | DragKind::Resize { .. }) => {
                    if let Some(rect) = drag.hit_rect {
                        this.show_details_at(&target, rect);
                    }
                }
                (
                    true,
                    Some((start, end)),
                    DragKind::Move { occ, .. } | DragKind::Resize { occ },
                ) => {
                    (this.on_drag)(DragAction::Move {
                        occ: Box::new(occ),
                        start,
                        end,
                    });
                }
                (true, Some((start, end)), DragKind::Create { .. }) => {
                    (this.on_drag)(DragAction::Create { start, end });
                }
                _ => {}
            }
        });
        area.add_controller(gesture);
    }

    /// Accept tasks dragged from the sidebar.
    fn setup_drop(self: &Rc<Self>, area: &gtk::DrawingArea, all_day: bool) {
        let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::COPY);
        let this = Rc::downgrade(self);
        let widget = area.clone();
        target.connect_drop(move |_, value, x, y| {
            let Some(this) = this.upgrade() else {
                return false;
            };
            let Some(task) = value
                .get::<String>()
                .ok()
                .and_then(|s| crate::sidetasks::decode(&s))
            else {
                return false;
            };
            let Some(day) = this.day_at(x.max(GUTTER), widget.width() as f64) else {
                return false;
            };
            let at = if all_day {
                When::Date(day)
            } else {
                let local = this.state.borrow().local;
                When::Zoned(
                    (layout::day_start(day, local) + TimeDelta::minutes(snap(minute_at(y)) as i64))
                        .with_timezone(&local),
                )
            };
            (this.on_drag)(DragAction::DropTask { task, at });
            true
        });
        area.add_controller(target);
    }

    /// Hint what dragging will do: move or, near the bottom edge, resize.
    fn setup_cursor(self: &Rc<Self>) {
        let motion = gtk::EventControllerMotion::new();
        let this = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            let Some(this) = this.upgrade() else { return };
            let hit = hit_at(&this.grid_hits.borrow(), x, y);
            let cursor = hit.map(|h| {
                if y > h.y + h.h - 6.0 {
                    "ns-resize"
                } else {
                    "grab"
                }
            });
            this.grid.set_cursor_from_name(cursor);
        });
        self.grid.add_controller(motion);
    }
    /// Whether `o` is being moved or resized; the ghost stands in for it.
    fn is_dragging(&self, o: &Occurrence) -> bool {
        self.drag.borrow().as_ref().is_some_and(|d| {
            d.preview.is_some()
                && matches!(&d.kind, DragKind::Move { occ, .. } | DragKind::Resize { occ }
                    if OccurrenceKey::of(occ).matches(o))
        })
    }

    /// The occurrence being dragged with its previewed times.
    fn drag_ghost(&self, all_day: bool) -> Option<Occurrence> {
        let drag = self.drag.borrow();
        let drag = drag.as_ref().filter(|d| d.all_day == all_day)?;
        let (start, end) = drag.preview?;
        let mut ghost = match &drag.kind {
            DragKind::Move { occ, .. } | DragKind::Resize { occ } => occ.clone(),
            DragKind::Create { .. } => Occurrence {
                calendar: self.state.borrow().default_calendar().unwrap_or_default(),
                path: Default::default(),
                uid: String::new(),
                recurrence_id: None,
                kind: Kind::Event,
                summary: "New event".into(),
                location: None,
                description: None,
                status: None,
                start,
                end,
                all_day: false,
                recurring: false,
                due: None,
                alarms: Vec::new(),
            },
        };
        ghost.start = start;
        ghost.end = end;
        Some(ghost)
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
            if self.is_dragging(o) {
                continue;
            }
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

        if let Some(ghost) = self.drag_ghost(true) {
            let ghosts = [ghost];
            for bar in layout::all_day_bars(&ghosts, &days, state.local) {
                let x = GUTTER + bar.first as f64 * col_w + 2.0;
                let w = (bar.last - bar.first + 1) as f64 * col_w - 4.0;
                draw_ghost(
                    area,
                    cr,
                    &pal,
                    &ghosts[0],
                    &state,
                    x,
                    3.0,
                    w,
                    LANE_HEIGHT - 3.0,
                    true,
                );
            }
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
                    &hour_label(days[0], hour, state.local),
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
                // drawn over earlier ones: set in so theirs still shows,
                // but never narrower than half the column
                let step = (inner * 0.15).min(16.0);
                let indent = (seg.indent as f64 * step).min(inner / 2.0);
                let w = (inner - indent) / seg.columns as f64;
                let x = x0 + 2.0 + indent + seg.column as f64 * w;
                let y = seg.start_min / 60.0 * HOUR_HEIGHT + 1.0;
                let h = (seg.end_min - seg.start_min) / 60.0 * HOUR_HEIGHT - 2.0;
                if self.is_dragging(o) {
                    continue;
                }
                if seg.indent > 0 {
                    // opaque, with a gap round it, over what is below
                    pal.base(cr);
                    draw::rounded_rect(cr, x - 1.5, y - 1.5, w + 1.0, h + 3.0, 6.5);
                    let _ = cr.fill();
                }
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

        if let Some(ghost) = self.drag_ghost(false) {
            let ghosts = [ghost];
            for (i, day) in days.iter().enumerate() {
                for seg in layout::day_segments(&ghosts, *day, state.local, MIN_EVENT_MINUTES) {
                    let x = GUTTER + i as f64 * col_w + 2.0;
                    let y = seg.start_min / 60.0 * HOUR_HEIGHT + 1.0;
                    let h = (seg.end_min - seg.start_min) / 60.0 * HOUR_HEIGHT - 2.0;
                    draw_ghost(
                        area,
                        cr,
                        &pal,
                        &ghosts[0],
                        &state,
                        x,
                        y,
                        col_w - 6.0,
                        h,
                        false,
                    );
                }
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
    let color = state.display_color(o);
    let cancelled = format::is_inactive(o);
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

    let task = o.kind == Kind::Todo;
    let title = format::label(o, state.local);
    // tasks get a tick circle before their text
    let tx = if task { x + 22.0 } else { x + 7.0 };
    let circle = |cr: &cairo::Context, cy: f64| {
        if task {
            text_color(cr, 0.9);
            draw::task_circle(cr, x + 13.0, cy, 5.0, format::is_done(o));
        }
    };
    let tw = w - 3.0 - (tx - x);
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
        draw::show_layout(cr, &l, tx, y + (h - draw::layout_height(&l)) / 2.0);
        circle(cr, y + h / 2.0);
    } else {
        // short or narrow blocks: one ellipsized line rather than wrapping
        let one_line = h < 34.0;
        // narrow blocks give every line to the title; the time is where
        // the block sits anyway
        let narrow = w < 100.0;
        let time = format::time(&o.start, state.local);
        if one_line {
            let l = draw::layout(area, &format!("{title}  {time}"), &style, tw, 1);
            text_color(cr, 1.0);
            draw::show_layout(cr, &l, tx, y + 2.0);
            circle(cr, y + 2.0 + draw::layout_height(&l) / 2.0);
        } else {
            let line_h = 15.0;
            let lines = ((h - 4.0) / line_h) as i32;
            let max_title_lines = if narrow { lines } else { lines - 1 }.max(1);
            let l = draw::layout(area, &title, &style, tw, max_title_lines);
            text_color(cr, 1.0);
            draw::show_layout(cr, &l, tx, y + 3.0);
            circle(cr, y + 11.0);
            let used = draw::layout_height(&l);
            let mut sub = format!("{time} – {}", format::time(&o.end, state.local));
            if let Some(loc) = o.location.as_deref().filter(|l| !l.is_empty()) {
                sub = format!("{sub} · {loc}");
            }
            let s = draw::layout(area, &sub, &TextStyle::SMALL, tw, 1);
            if !narrow && used + draw::layout_height(&s) + 4.0 <= h {
                text_color(cr, 0.8);
                draw::show_layout(cr, &s, tx, y + 3.0 + used);
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

/// The previewed position of an event being dragged.
#[allow(clippy::too_many_arguments)]
fn draw_ghost(
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    pal: &Palette,
    o: &Occurrence,
    state: &crate::state::State,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    compact: bool,
) {
    cr.push_group();
    draw_block(area, cr, pal, o, state, x, y, w, h, true, compact);
    let _ = cr.pop_group_to_source();
    let _ = cr.paint_with_alpha(0.85);
    pal.fg(cr, 0.9);
    cr.set_line_width(1.5);
    cr.set_dash(&[4.0, 3.0], 0.0);
    draw::rounded_rect(cr, x + 0.75, y + 0.75, w - 1.5, h - 1.5, 5.0);
    let _ = cr.stroke();
    cr.set_dash(&[], 0.0);
}

/// The local time `hours` after the start of `day`; on days the clocks
/// change this differs from the plain hour number.
fn hour_label(day: NaiveDate, hours: i64, local: chrono_tz::Tz) -> String {
    (layout::day_start(day, local) + TimeDelta::hours(hours))
        .with_timezone(&local)
        .format("%H:%M")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hour_labels_follow_clock_changes() {
        let london = chrono_tz::Tz::Europe__London;
        let normal = NaiveDate::from_ymd_opt(2026, 10, 24).unwrap();
        assert_eq!(hour_label(normal, 5, london), "05:00");
        // the clocks go back at 02:00 BST on 25 October 2026
        let change = NaiveDate::from_ymd_opt(2026, 10, 25).unwrap();
        assert_eq!(hour_label(change, 1, london), "01:00");
        assert_eq!(hour_label(change, 2, london), "01:00");
        assert_eq!(hour_label(change, 5, london), "04:00");
    }
}

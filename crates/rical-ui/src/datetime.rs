//! Pickers for the entry rows that take a date and a time: a calendar
//! button opens a month to click (and Today, Tomorrow…), a clock button
//! beside it opens hours and minutes (and 9:00, 14:00, and for an end how
//! long it lasts). Two small pickers, not one crowded one. The text stays
//! what it was, so typing (and what the app accepts when it reads it) still
//! works: the pickers just write `2026-11-05 14:30` or `2026-11-05` into it.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{Datelike as _, Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike as _};
use gtk::{gdk, glib};

/// Reads what is typed into the form the pickers write.
pub type Normalise = Rc<dyn Fn(&str) -> Option<String>>;

/// How a row's pickers read and write.
pub struct Config {
    /// Whether the value has a time of day (false for dates, as an all-day
    /// event's). Asked again whenever the text changes.
    pub timed: Rc<dyn Fn() -> bool>,
    /// What was typed as `YYYY-MM-DD[ HH:MM]`, if it is something else
    /// ("fri 14:00", "2w") the app can read.
    pub normalise: Option<Normalise>,
    /// For an end: the start (the same form), so lengths can be offered.
    pub start: Option<Rc<dyn Fn() -> Option<String>>>,
    /// Whether "Yesterday" is a sensible choice.
    pub past: bool,
}

impl Config {
    pub fn date() -> Config {
        Config {
            timed: Rc::new(|| false),
            normalise: None,
            start: None,
            past: true,
        }
    }

    pub fn datetime() -> Config {
        Config {
            timed: Rc::new(|| true),
            normalise: None,
            start: None,
            past: false,
        }
    }
}

/// `YYYY-MM-DD` or `YYYY-MM-DD HH:MM` (or just `HH:MM`, on `day`).
pub fn parse_iso(text: &str, day: NaiveDate) -> Option<NaiveDateTime> {
    let t = text.trim();
    if let Ok(dt) = NaiveDateTime::parse_from_str(t, "%Y-%m-%d %H:%M") {
        return Some(dt);
    }
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        return d.and_hms_opt(0, 0, 0);
    }
    NaiveTime::parse_from_str(t, "%H:%M")
        .ok()
        .map(|time| day.and_time(time))
}

pub fn format_iso(dt: NaiveDateTime, timed: bool) -> String {
    if timed {
        dt.format("%Y-%m-%d %H:%M").to_string()
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

fn glib_date(d: NaiveDate) -> Option<glib::DateTime> {
    glib::DateTime::from_local(d.year(), d.month() as i32, d.day() as i32, 0, 0, 0.0).ok()
}

fn chip(label: &str) -> gtk::Button {
    gtk::Button::builder()
        .label(label)
        .css_classes(["flat", "caption"])
        .build()
}

/// What the row holds now: its text, else (for an end) an hour after the
/// start, else nine today.
fn current(row: &adw::EntryRow, cfg: &Config) -> NaiveDateTime {
    let today = Local::now().date_naive();
    let typed = row.text().to_string();
    let text = match (&cfg.normalise, parse_iso(&typed, today)) {
        (_, Some(_)) => typed,
        (Some(n), None) => n(&typed).unwrap_or_default(),
        (None, None) => String::new(),
    };
    parse_iso(&text, today)
        .or_else(|| {
            cfg.start
                .as_ref()
                .and_then(|s| s())
                .and_then(|t| parse_iso(&t, today))
                .map(|s| s + Duration::hours(1))
        })
        .unwrap_or_else(|| today.and_hms_opt(9, 0, 0).unwrap_or_default())
}

fn write(row: &adw::EntryRow, cfg: &Config, dt: NaiveDateTime) {
    row.set_text(&format_iso(dt, (cfg.timed)()));
    row.set_position(-1);
}

fn button(icon: &str, tip: &str) -> (gtk::MenuButton, gtk::Popover, gtk::Box) {
    let b = gtk::MenuButton::builder()
        .icon_name(icon)
        .valign(gtk::Align::Center)
        .css_classes(["flat"])
        .tooltip_text(tip)
        .build();
    b.update_property(&[gtk::accessible::Property::Label(tip)]);
    let popover = gtk::Popover::new();
    let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
    popover.set_child(Some(&holder));
    b.set_popover(Some(&popover));
    (b, popover, holder)
}

fn fill(holder: &gtk::Box, content: &impl IsA<gtk::Widget>) {
    while let Some(c) = holder.first_child() {
        holder.remove(&c);
    }
    holder.append(content);
}

/// Give `row` a date button and, if it has a time, a time button (`Alt+Down`
/// and `Alt+Shift+Down` in the row open them).
pub fn attach(row: &adw::EntryRow, config: Config) {
    let config = Rc::new(config);

    let (date_button, date_pop, date_holder) =
        button("x-office-calendar-symbolic", "Pick the date (Alt+Down)");
    let (time_button, time_pop, time_holder) = button(
        "preferences-system-time-symbolic",
        "Pick the time (Alt+Shift+Down)",
    );
    row.add_suffix(&time_button);
    row.add_suffix(&date_button);

    let (r, c, p) = (row.clone(), config.clone(), date_pop.clone());
    date_pop.connect_show(move |_| fill(&date_holder, &date_content(&r, &c, &p)));
    let (r, c, p) = (row.clone(), config.clone(), time_pop.clone());
    time_pop.connect_show(move |_| fill(&time_holder, &time_content(&r, &c, &p)));

    // the time button only while the value has a time
    let show = {
        let (b, c) = (time_button.clone(), config.clone());
        move || b.set_visible((c.timed)())
    };
    show();
    row.connect_changed(move |_| show());

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let (d, t, c) = (date_button.clone(), time_button.clone(), config.clone());
    keys.connect_key_pressed(move |_, key, _, mods| {
        if key != gdk::Key::Down || !mods.contains(gdk::ModifierType::ALT_MASK) {
            return glib::Propagation::Proceed;
        }
        if mods.contains(gdk::ModifierType::SHIFT_MASK) && (c.timed)() {
            t.popup();
        } else {
            d.popup();
        }
        glib::Propagation::Stop
    });
    row.add_controller(keys);
}

fn pad(b: &gtk::Box) {
    b.set_margin_top(10);
    b.set_margin_bottom(10);
    b.set_margin_start(10);
    b.set_margin_end(10);
}

fn done(pop: &gtk::Popover) -> gtk::Button {
    let b = gtk::Button::builder()
        .label("Done")
        .css_classes(["suggested-action"])
        .halign(gtk::Align::End)
        .build();
    let p = pop.clone();
    b.connect_clicked(move |_| p.popdown());
    b
}

/// A month to click, and the usual days.
fn date_content(row: &adw::EntryRow, cfg: &Rc<Config>, pop: &gtk::Popover) -> gtk::Box {
    let today = Local::now().date_naive();
    let calendar = gtk::Calendar::new();
    if let Some(d) = glib_date(current(row, cfg).date()) {
        calendar.select_day(&d);
    }
    // the day changes, the time of day stays
    let day = {
        let (row, cfg) = (row.clone(), cfg.clone());
        Rc::new(move |d: NaiveDate| write(&row, &cfg, d.and_time(current(&row, &cfg).time())))
    };
    let d = day.clone();
    calendar.connect_day_selected(move |c| {
        let g = c.date();
        if let Some(n) =
            NaiveDate::from_ymd_opt(g.year(), g.month() as u32, g.day_of_month() as u32)
        {
            d(n);
        }
    });

    let days = gtk::Box::builder().spacing(2).build();
    let mut quick: Vec<(&str, i64)> = vec![("Today", 0), ("Tomorrow", 1), ("Next week", 7)];
    if cfg.past {
        quick.insert(0, ("Yesterday", -1));
    }
    for (label, n) in quick {
        let b = chip(label);
        let (d, cal) = (day.clone(), calendar.clone());
        b.connect_clicked(move |_| {
            let target = today + Duration::days(n);
            if let Some(g) = glib_date(target) {
                cal.select_day(&g);
            }
            d(target);
        });
        days.append(&b);
    }

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    pad(&root);
    root.append(&days);
    root.append(&calendar);
    root.append(&done(pop));
    root
}

/// Hours and minutes, the usual times, and for an end how long it lasts.
fn time_content(row: &adw::EntryRow, cfg: &Rc<Config>, pop: &gtk::Popover) -> gtk::Box {
    let now = current(row, cfg);
    let hour = gtk::SpinButton::with_range(0.0, 23.0, 1.0);
    let minute = gtk::SpinButton::with_range(0.0, 59.0, 5.0);
    for s in [&hour, &minute] {
        s.set_wrap(true);
        s.set_width_chars(2);
        s.set_numeric(true);
        s.connect_output(|s| {
            s.set_text(&format!("{:02}", s.value_as_int()));
            glib::Propagation::Stop
        });
    }
    hour.set_value(f64::from(now.hour()));
    minute.set_value(f64::from(now.minute()));

    // the time changes, the day stays
    let set = {
        let (row, cfg) = (row.clone(), cfg.clone());
        Rc::new(move |h: u32, m: u32| {
            let day = current(&row, &cfg).date();
            let t = NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default();
            write(&row, &cfg, day.and_time(t));
        })
    };
    for spin in [&hour, &minute] {
        let (set, h, m) = (set.clone(), hour.clone(), minute.clone());
        spin.connect_value_changed(move |_| set(h.value_as_int() as u32, m.value_as_int() as u32));
    }

    let clock = gtk::Box::builder().spacing(6).build();
    clock.append(&hour);
    clock.append(&gtk::Label::new(Some(":")));
    clock.append(&minute);

    let times = gtk::Box::builder().spacing(2).build();
    for (label, h) in [("9:00", 9), ("12:00", 12), ("14:00", 14), ("17:00", 17)] {
        let b = chip(label);
        let (set, hs, ms) = (set.clone(), hour.clone(), minute.clone());
        b.connect_clicked(move |_| {
            hs.set_value(f64::from(h));
            ms.set_value(0.0);
            set(h, 0);
        });
        times.append(&b);
    }

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .build();
    pad(&root);
    root.append(&clock);
    root.append(&times);

    // an end: how long after the start
    let start = cfg
        .start
        .as_ref()
        .and_then(|s| s())
        .and_then(|t| parse_iso(&t, Local::now().date_naive()));
    if let Some(s) = start {
        let lens = gtk::Box::builder().spacing(2).build();
        lens.append(
            &gtk::Label::builder()
                .label("Lasts")
                .css_classes(["dim-label", "caption"])
                .margin_end(4)
                .build(),
        );
        for (label, mins) in [("30m", 30), ("1h", 60), ("2h", 120), ("1 day", 24 * 60)] {
            let b = chip(label);
            let (row, cfg) = (row.clone(), cfg.clone());
            b.connect_clicked(move |_| write(&row, &cfg, s + Duration::minutes(mins)));
            lens.append(&b);
        }
        root.append(&lens);
    }
    root.append(&done(pop));
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_the_fields_hold() {
        let day = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let dt = parse_iso("2026-11-05 14:30", day).unwrap();
        assert_eq!(format_iso(dt, true), "2026-11-05 14:30");
        assert_eq!(format_iso(dt, false), "2026-11-05");
        assert_eq!(parse_iso("2026-12-25", day).unwrap().date().day(), 25);
        assert_eq!(
            parse_iso("09:15", day).unwrap().to_string(),
            "2026-11-05 09:15:00"
        );
        assert!(parse_iso("tomorrow", day).is_none());
    }
}

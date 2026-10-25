//! The editor's repeat controls: presets, or a custom rule (every N days,
//! weeks, months or years; weekdays; ending never, after N times or on a
//! date). Rules rical does not model can be kept as they are.

use std::rc::Rc;

use adw::prelude::*;
use chrono::{NaiveDate, Weekday};
use rical_core::{
    rule::{End, Freq, Rule},
    write::Repeat,
};

const PRESETS: [&str; 5] = ["Never", "Daily", "Weekly", "Monthly", "Yearly"];
const CUSTOM: u32 = PRESETS.len() as u32;
const UNITS: [&str; 4] = ["days", "weeks", "months", "years"];
const WEEKDAYS: [Weekday; 7] = [
    Weekday::Mon,
    Weekday::Tue,
    Weekday::Wed,
    Weekday::Thu,
    Weekday::Fri,
    Weekday::Sat,
    Weekday::Sun,
];

pub struct RepeatEditor {
    pub choice: adw::ComboRow,
    every: adw::SpinRow,
    unit: adw::ComboRow,
    weekdays_row: adw::ActionRow,
    weekdays: Vec<gtk::ToggleButton>,
    ends: adw::ComboRow,
    count: adw::SpinRow,
    until: adw::EntryRow,
    /// A rule rical does not model, offered as the last choice.
    kept: Option<String>,
}

impl RepeatEditor {
    pub fn new(repeat: &Repeat) -> Rc<Self> {
        let kept = match repeat {
            Repeat::Custom(raw) => Some(raw.clone()),
            _ => None,
        };
        let mut labels = PRESETS.to_vec();
        labels.push("Custom…");
        let kept_label = kept.as_ref().map(|r| format!("Keep: {r}"));
        if let Some(l) = &kept_label {
            labels.push(l);
        }
        let choice = adw::ComboRow::builder()
            .title("Repeat")
            .model(&gtk::StringList::new(&labels))
            .build();
        let every = adw::SpinRow::builder()
            .title("Every")
            .adjustment(&gtk::Adjustment::new(1.0, 1.0, 99.0, 1.0, 5.0, 0.0))
            .build();
        let unit = adw::ComboRow::builder()
            .title("Unit")
            .model(&gtk::StringList::new(&UNITS))
            .build();
        let day_box = gtk::Box::builder()
            .css_classes(["linked"])
            .valign(gtk::Align::Center)
            .build();
        let weekdays = WEEKDAYS
            .iter()
            .map(|d| {
                let b = gtk::ToggleButton::builder()
                    .label(&d.to_string()[..2])
                    .tooltip_text(format!("{d}"))
                    .build();
                day_box.append(&b);
                b
            })
            .collect::<Vec<_>>();
        let weekdays_row = adw::ActionRow::builder().title("On").build();
        weekdays_row.add_suffix(&day_box);
        let ends = adw::ComboRow::builder()
            .title("Ends")
            .model(&gtk::StringList::new(&[
                "Never",
                "After a number of times",
                "On a date",
            ]))
            .build();
        let count = adw::SpinRow::builder()
            .title("Times")
            .adjustment(&gtk::Adjustment::new(10.0, 1.0, 999.0, 1.0, 10.0, 0.0))
            .build();
        let until = adw::EntryRow::builder()
            .title("Last date (YYYY-MM-DD)")
            .build();

        let this = Rc::new(Self {
            choice,
            every,
            unit,
            weekdays_row,
            weekdays,
            ends,
            count,
            until,
            kept,
        });
        this.load(repeat);

        for w in [&this.choice, &this.unit, &this.ends] {
            let t = Rc::downgrade(&this);
            w.connect_selected_notify(move |_| {
                if let Some(t) = t.upgrade() {
                    t.sync();
                }
            });
        }
        let t = Rc::downgrade(&this);
        this.every.connect_value_notify(move |_| {
            if let Some(t) = t.upgrade() {
                t.sync();
            }
        });
        let t = Rc::downgrade(&this);
        this.count.connect_value_notify(move |_| {
            if let Some(t) = t.upgrade() {
                t.sync();
            }
        });
        for b in &this.weekdays {
            let t = Rc::downgrade(&this);
            b.connect_toggled(move |_| {
                if let Some(t) = t.upgrade() {
                    t.sync();
                }
            });
        }
        let t = Rc::downgrade(&this);
        this.until.connect_changed(move |_| {
            if let Some(t) = t.upgrade() {
                t.sync();
            }
        });
        this.sync();
        this
    }

    /// Every row, to add to a preferences group in order.
    pub fn rows(&self) -> Vec<gtk::Widget> {
        vec![
            self.choice.clone().upcast(),
            self.every.clone().upcast(),
            self.unit.clone().upcast(),
            self.weekdays_row.clone().upcast(),
            self.ends.clone().upcast(),
            self.count.clone().upcast(),
            self.until.clone().upcast(),
        ]
    }

    fn load(&self, repeat: &Repeat) {
        match repeat {
            Repeat::None => self.choice.set_selected(0),
            Repeat::Custom(_) => self.choice.set_selected(CUSTOM + 1),
            Repeat::Rule(rule) => {
                let preset =
                    rule.interval == 1 && rule.weekdays.is_empty() && rule.end == End::Never;
                let freq_index = Freq::ALL.iter().position(|f| *f == rule.freq).unwrap_or(0) as u32;
                self.choice
                    .set_selected(if preset { freq_index + 1 } else { CUSTOM });
                self.every.set_value(rule.interval as f64);
                self.unit.set_selected(freq_index);
                for (b, d) in self.weekdays.iter().zip(WEEKDAYS) {
                    b.set_active(rule.weekdays.contains(&d));
                }
                match rule.end {
                    End::Never => self.ends.set_selected(0),
                    End::Count(n) => {
                        self.ends.set_selected(1);
                        self.count.set_value(n as f64);
                    }
                    End::Until(d) => {
                        self.ends.set_selected(2);
                        self.until.set_text(&d.format("%Y-%m-%d").to_string());
                    }
                }
            }
        }
    }

    /// Show only the rows that apply, and describe the rule.
    fn sync(&self) {
        let custom = self.choice.selected() == CUSTOM;
        self.every.set_visible(custom);
        self.unit.set_visible(custom);
        self.weekdays_row
            .set_visible(custom && self.unit.selected() == 1);
        self.ends.set_visible(custom);
        self.count.set_visible(custom && self.ends.selected() == 1);
        self.until.set_visible(custom && self.ends.selected() == 2);
        let summary = match self.value() {
            Ok(Repeat::None) => String::new(),
            Ok(Repeat::Rule(r)) => r.to_string(),
            Ok(Repeat::Custom(raw)) => raw,
            Err(e) => e,
        };
        // rows take markup, and an error may hold <angle brackets>
        self.choice
            .set_subtitle(&gtk::glib::markup_escape_text(&summary));
    }

    /// The rule as chosen, or what is wrong with it.
    pub fn value(&self) -> Result<Repeat, String> {
        let selected = self.choice.selected();
        if selected == 0 {
            return Ok(Repeat::None);
        }
        if selected < CUSTOM {
            return Ok(Repeat::every(Freq::ALL[selected as usize - 1]));
        }
        if selected > CUSTOM {
            return Ok(self.kept.clone().map_or(Repeat::None, Repeat::Custom));
        }
        let freq = Freq::ALL[self.unit.selected().min(3) as usize];
        let mut rule = Rule::new(freq);
        rule.interval = self.every.value().max(1.0) as u32;
        if freq == Freq::Weekly {
            rule.weekdays = self
                .weekdays
                .iter()
                .zip(WEEKDAYS)
                .filter(|(b, _)| b.is_active())
                .map(|(_, d)| d)
                .collect();
        }
        rule.end = match self.ends.selected() {
            1 => End::Count(self.count.value().max(1.0) as u32),
            2 => End::Until(
                NaiveDate::parse_from_str(self.until.text().trim(), "%Y-%m-%d")
                    .map_err(|_| "the last date should look like 2025-12-31".to_owned())?,
            ),
            _ => End::Never,
        };
        Ok(Repeat::Rule(rule))
    }
}

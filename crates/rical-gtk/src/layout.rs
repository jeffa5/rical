//! Pure layout calculations for the calendar views, kept free of GTK so they
//! can be tested.

use std::ops::Range;

use chrono::{DateTime, Datelike, Days, NaiveDate, NaiveTime, TimeDelta, Utc, Weekday};
use chrono_tz::Tz;
use rical_core::{Occurrence, when::resolve_local};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Day,
    Week,
    Month,
    Agenda,
}

impl ViewKind {
    pub const ALL: [ViewKind; 4] = [Self::Day, Self::Week, Self::Month, Self::Agenda];

    pub fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
            Self::Agenda => "agenda",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.name() == name)
    }
}

/// Number of days the agenda view shows.
pub const AGENDA_DAYS: u64 = 30;

pub fn week_start(date: NaiveDate) -> NaiveDate {
    date.week(Weekday::Mon).first_day()
}

/// The days a view shows around `date`.
pub fn visible_days(view: ViewKind, date: NaiveDate) -> Vec<NaiveDate> {
    let (first, n) = match view {
        ViewKind::Day => (date, 1),
        ViewKind::Week => (week_start(date), 7),
        ViewKind::Month => (week_start(date.with_day(1).unwrap()), 42),
        ViewKind::Agenda => (date, AGENDA_DAYS),
    };
    (0..n).map(|i| first + Days::new(i)).collect()
}

/// Move `date` one view-length forward or back.
pub fn step(view: ViewKind, date: NaiveDate, forward: bool) -> NaiveDate {
    let shift = |d: NaiveDate, days: u64| {
        if forward {
            d + Days::new(days)
        } else {
            d - Days::new(days)
        }
    };
    match view {
        ViewKind::Day => shift(date, 1),
        ViewKind::Week => shift(date, 7),
        ViewKind::Agenda => shift(date, AGENDA_DAYS),
        ViewKind::Month => {
            let months = chrono::Months::new(1);
            if forward {
                date.checked_add_months(months).unwrap_or(date)
            } else {
                date.checked_sub_months(months).unwrap_or(date)
            }
        }
    }
}

pub fn day_start(day: NaiveDate, local: Tz) -> DateTime<Utc> {
    resolve_local(local, day.and_time(NaiveTime::MIN)).to_utc()
}

/// Instants covered by `days` (which must be consecutive).
pub fn days_range(days: &[NaiveDate], local: Tz) -> Range<DateTime<Utc>> {
    let first = days[0];
    let last = *days.last().unwrap();
    day_start(first, local)..day_start(last + Days::new(1), local)
}

/// Whether an occurrence belongs in the all-day row rather than the grid.
pub fn is_all_day_row(o: &Occurrence) -> bool {
    o.all_day || o.duration() >= TimeDelta::days(1)
}

/// The days `o` covers, as indices into `days`, clipped to them.
pub fn day_span(o: &Occurrence, days: &[NaiveDate], local: Tz) -> Option<(usize, usize)> {
    let first = o.start.date(local);
    // an end exactly at midnight does not cover the next day
    let end = o.end.to_tz(local);
    let mut last = o.end.date(local);
    if last > first && (o.end.is_date() || end.time() == NaiveTime::MIN) {
        last = last.pred_opt()?;
    }
    let lo = days.iter().position(|d| *d >= first)?;
    let hi = days.iter().rposition(|d| *d <= last)?;
    (lo <= hi && days[lo] <= last && days[hi] >= first).then_some((lo, hi))
}

/// A slice of a timed occurrence within one day, in minutes from midnight.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// Index into the occurrences slice.
    pub index: usize,
    pub start_min: f64,
    pub end_min: f64,
    pub column: usize,
    pub columns: usize,
}

/// Lay out the timed occurrences on `day`: each gets a column within its
/// cluster of overlapping occurrences so they sit side by side.
pub fn day_segments(
    occurrences: &[Occurrence],
    day: NaiveDate,
    local: Tz,
    min_duration_min: f64,
) -> Vec<Segment> {
    let start = day_start(day, local);
    let end = day_start(day + Days::new(1), local);
    let day_len = (end - start).num_minutes() as f64;

    let mut segs = occurrences
        .iter()
        .enumerate()
        .filter(|(_, o)| !is_all_day_row(o))
        .filter_map(|(index, o)| {
            let s = o.start.to_utc(local);
            let e = o.end.to_utc(local);
            let touches = if s == e {
                s >= start && s < end
            } else {
                s < end && e > start
            };
            touches.then(|| {
                let start_min = ((s - start).num_minutes() as f64).max(0.0);
                let end_min = ((e - start).num_minutes() as f64)
                    .min(day_len)
                    .max(start_min + min_duration_min);
                Segment {
                    index,
                    start_min,
                    end_min,
                    column: 0,
                    columns: 1,
                }
            })
        })
        .collect::<Vec<_>>();

    segs.sort_by(|a, b| {
        a.start_min
            .total_cmp(&b.start_min)
            .then(b.end_min.total_cmp(&a.end_min))
    });

    // Assign columns cluster by cluster.
    let mut i = 0;
    while i < segs.len() {
        let mut cluster_end = segs[i].end_min;
        let mut j = i;
        let mut column_ends: Vec<f64> = Vec::new();
        while j < segs.len() && (j == i || segs[j].start_min < cluster_end) {
            let col = column_ends
                .iter()
                .position(|&e| e <= segs[j].start_min)
                .unwrap_or_else(|| {
                    column_ends.push(0.0);
                    column_ends.len() - 1
                });
            column_ends[col] = segs[j].end_min;
            segs[j].column = col;
            cluster_end = cluster_end.max(segs[j].end_min);
            j += 1;
        }
        for seg in &mut segs[i..j] {
            seg.columns = column_ends.len();
        }
        i = j;
    }
    segs
}

/// An all-day-row item spanning days `first..=last` on a lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bar {
    pub index: usize,
    pub first: usize,
    pub last: usize,
    pub lane: usize,
}

/// Stack the all-day-row occurrences on `days` into lanes.
pub fn all_day_bars(occurrences: &[Occurrence], days: &[NaiveDate], local: Tz) -> Vec<Bar> {
    let mut bars = occurrences
        .iter()
        .enumerate()
        .filter(|(_, o)| is_all_day_row(o))
        .filter_map(|(index, o)| {
            day_span(o, days, local).map(|(first, last)| Bar {
                index,
                first,
                last,
                lane: 0,
            })
        })
        .collect::<Vec<_>>();
    bars.sort_by_key(|b| (b.first, std::cmp::Reverse(b.last)));
    assign_lanes(&mut bars);
    bars
}

fn assign_lanes(bars: &mut [Bar]) {
    let mut lane_ends: Vec<usize> = Vec::new();
    for bar in bars {
        let lane = lane_ends
            .iter()
            .position(|&e| e < bar.first)
            .unwrap_or_else(|| {
                lane_ends.push(0);
                lane_ends.len() - 1
            });
        lane_ends[lane] = bar.last;
        bar.lane = lane;
    }
}

pub fn lanes(bars: &[Bar]) -> usize {
    bars.iter().map(|b| b.lane + 1).max().unwrap_or(0)
}

/// For the month view: everything on each week row as bars, all-day and
/// timed alike, so each cell can list them in lane order.
pub fn month_week_bars(occurrences: &[Occurrence], week: &[NaiveDate], local: Tz) -> Vec<Bar> {
    let mut bars = occurrences
        .iter()
        .enumerate()
        .filter_map(|(index, o)| {
            day_span(o, week, local).map(|(first, last)| Bar {
                index,
                first,
                last,
                lane: 0,
            })
        })
        .collect::<Vec<_>>();
    // multi-day first so they get the top lanes, then by start time
    bars.sort_by_key(|b| {
        (
            b.first,
            b.first == b.last,
            occurrences[b.index].start.to_utc(local),
        )
    });
    assign_lanes(&mut bars);
    bars
}

/// Busy minutes on `day` from timed occurrences, merging overlaps.
pub fn busy_minutes(occurrences: &[Occurrence], day: NaiveDate, local: Tz) -> f64 {
    let mut segs = day_segments(occurrences, day, local, 0.0)
        .into_iter()
        .map(|s| (s.start_min, s.end_min))
        .collect::<Vec<_>>();
    segs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut total = 0.0;
    let mut cur: Option<(f64, f64)> = None;
    for (s, e) in segs {
        match cur {
            Some((cs, ce)) if s <= ce => cur = Some((cs, ce.max(e))),
            Some((cs, ce)) => {
                total += ce - cs;
                cur = Some((s, e));
            }
            None => cur = Some((s, e)),
        }
    }
    if let Some((cs, ce)) = cur {
        total += ce - cs;
    }
    total
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rical_core::{Kind, When};

    use super::*;

    const UTC: Tz = Tz::UTC;

    fn occ(start: &str, end: &str) -> Occurrence {
        let start = When::parse(start, None).unwrap();
        let end = When::parse(end, None).unwrap();
        Occurrence {
            calendar: "c".into(),
            path: PathBuf::new(),
            uid: format!("{start}"),
            recurrence_id: None,
            kind: Kind::Event,
            summary: String::new(),
            location: None,
            description: None,
            status: None,
            start,
            end,
            all_day: start.is_date(),
            recurring: false,
        }
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn views_and_steps() {
        let d = date("2025-10-23"); // Thursday
        assert_eq!(visible_days(ViewKind::Day, d), [d]);
        let week = visible_days(ViewKind::Week, d);
        assert_eq!(week[0], date("2025-10-20"));
        assert_eq!(week.len(), 7);
        let month = visible_days(ViewKind::Month, d);
        assert_eq!(month[0], date("2025-09-29"));
        assert_eq!(month.len(), 42);

        assert_eq!(step(ViewKind::Week, d, true), date("2025-10-30"));
        assert_eq!(
            step(ViewKind::Month, date("2025-01-31"), true),
            date("2025-02-28")
        );
        assert_eq!(step(ViewKind::Day, d, false), date("2025-10-22"));
    }

    #[test]
    fn overlapping_events_share_columns() {
        let o = [
            occ("20251023T090000Z", "20251023T100000Z"),
            occ("20251023T093000Z", "20251023T103000Z"),
            occ("20251023T100000Z", "20251023T110000Z"),
            occ("20251023T140000Z", "20251023T150000Z"),
        ];
        let segs = day_segments(&o, date("2025-10-23"), UTC, 15.0);
        let by_index = |i| *segs.iter().find(|s| s.index == i).unwrap();
        // first three form one cluster of two columns
        assert_eq!((by_index(0).column, by_index(0).columns), (0, 2));
        assert_eq!((by_index(1).column, by_index(1).columns), (1, 2));
        assert_eq!((by_index(2).column, by_index(2).columns), (0, 2));
        // the afternoon one is alone
        assert_eq!((by_index(3).column, by_index(3).columns), (0, 1));
        assert_eq!(by_index(3).start_min, 14.0 * 60.0);
    }

    #[test]
    fn overnight_events_are_split_across_days() {
        let o = [occ("20251023T220000Z", "20251024T020000Z")];
        let a = day_segments(&o, date("2025-10-23"), UTC, 15.0);
        let b = day_segments(&o, date("2025-10-24"), UTC, 15.0);
        assert_eq!((a[0].start_min, a[0].end_min), (22.0 * 60.0, 24.0 * 60.0));
        assert_eq!((b[0].start_min, b[0].end_min), (0.0, 120.0));
    }

    #[test]
    fn short_events_get_min_height() {
        let o = [occ("20251023T090000Z", "20251023T090500Z")];
        let s = day_segments(&o, date("2025-10-23"), UTC, 15.0);
        assert_eq!(s[0].end_min - s[0].start_min, 15.0);
    }

    #[test]
    fn all_day_lanes() {
        let days = visible_days(ViewKind::Week, date("2025-10-23"));
        let o = [
            occ("20251020", "20251023"),                 // Mon-Wed
            occ("20251022", "20251023"),                 // Wed
            occ("20251023", "20251024"),                 // Thu
            occ("20251018", "20251021"),                 // Sat-Mon, clipped
            occ("20251024T090000Z", "20251026T100000Z"), // timed, 2+ days
            occ("20251024T090000Z", "20251024T100000Z"), // timed, not here
        ];
        let bars = all_day_bars(&o, &days, UTC);
        let by_index = |i| bars.iter().find(|b| b.index == i).copied();
        // longer bars take the upper lanes
        assert_eq!(
            by_index(0).map(|b| (b.first, b.last, b.lane)),
            Some((0, 2, 0))
        );
        assert_eq!(
            by_index(3).map(|b| (b.first, b.last, b.lane)),
            Some((0, 0, 1))
        );
        assert_eq!(
            by_index(1).map(|b| (b.first, b.last, b.lane)),
            Some((2, 2, 1))
        );
        assert_eq!(
            by_index(2).map(|b| (b.first, b.last, b.lane)),
            Some((3, 3, 0))
        );
        assert_eq!(by_index(4).map(|b| (b.first, b.last)), Some((4, 6)));
        assert_eq!(by_index(5), None);
        assert_eq!(lanes(&bars), 2);
    }

    #[test]
    fn busy() {
        let o = [
            occ("20251023T090000Z", "20251023T100000Z"),
            occ("20251023T093000Z", "20251023T103000Z"),
            occ("20251023", "20251024"),
        ];
        assert_eq!(busy_minutes(&o, date("2025-10-23"), UTC), 90.0);
    }
}

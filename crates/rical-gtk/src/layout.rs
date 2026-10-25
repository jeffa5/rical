//! Pure layout calculations for the calendar views, kept free of GTK so they
//! can be tested.

use std::ops::Range;

use chrono::{DateTime, Datelike, Days, NaiveDate, NaiveTime, TimeDelta, Utc, Weekday};
use chrono_tz::Tz;
use rical_core::{Occurrence, when::resolve_local};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Day,
    /// Monday to Sunday.
    Week,
    /// Monday to Friday.
    WorkWeek,
    /// This many days from the focused day.
    Days(u8),
    Month,
    Agenda,
    Tasks,
    /// Upcoming invitations waiting for my answer.
    Invitations,
}

impl ViewKind {
    /// The name `:view` takes; "week" means whichever week view is
    /// configured, so the full week is "fullweek".
    pub fn name(self) -> String {
        match self {
            Self::Day => "day".into(),
            Self::Week => "fullweek".into(),
            Self::WorkWeek => "workweek".into(),
            Self::Days(n) => format!("{n}days"),
            Self::Month => "month".into(),
            Self::Agenda => "agenda".into(),
            Self::Tasks => "tasks".into(),
            Self::Invitations => "invitations".into(),
        }
    }

    /// Parse a view name; "week" is left to the caller. Days run from 2 to 14.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "day" => Self::Day,
            "fullweek" => Self::Week,
            "workweek" => Self::WorkWeek,
            "month" => Self::Month,
            "agenda" => Self::Agenda,
            "tasks" => Self::Tasks,
            "invitations" | "invites" => Self::Invitations,
            other => {
                let n: u8 = other.strip_suffix("days")?.parse().ok()?;
                if !(2..=14).contains(&n) {
                    return None;
                }
                Self::Days(n)
            }
        })
    }

    /// Which view switcher button stands for this view.
    pub fn toggle_name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week | Self::WorkWeek | Self::Days(_) => "week",
            Self::Month => "month",
            Self::Agenda => "agenda",
            Self::Tasks => "tasks",
            Self::Invitations => "invitations",
        }
    }

    /// Shown in the time grid.
    pub fn is_grid(self) -> bool {
        matches!(
            self,
            Self::Day | Self::Week | Self::WorkWeek | Self::Days(_)
        )
    }
}

/// Number of days the agenda view shows.
pub const AGENDA_DAYS: u64 = 30;

/// How far ahead the invitations view looks.
pub const INVITATION_DAYS: u64 = 366;

/// Monday of the work week to show for `date`: the coming one at weekends.
pub fn work_week_start(date: NaiveDate) -> NaiveDate {
    match date.weekday() {
        Weekday::Sat | Weekday::Sun => week_start(date) + Days::new(7),
        _ => week_start(date),
    }
}

pub fn week_start(date: NaiveDate) -> NaiveDate {
    date.week(Weekday::Mon).first_day()
}

/// The days a view shows around `date`.
pub fn visible_days(view: ViewKind, date: NaiveDate) -> Vec<NaiveDate> {
    let (first, n) = match view {
        ViewKind::Day => (date, 1),
        ViewKind::Week => (week_start(date), 7),
        ViewKind::WorkWeek => (work_week_start(date), 5),
        ViewKind::Days(n) => (date, n as u64),
        ViewKind::Month => (week_start(date.with_day(1).unwrap()), 42),
        ViewKind::Agenda => (date, AGENDA_DAYS),
        // tasks are not by date; the focused day still matters for "today"
        ViewKind::Tasks => (date, 1),
        ViewKind::Invitations => (date, INVITATION_DAYS),
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
        ViewKind::Week | ViewKind::WorkWeek => shift(date, 7),
        ViewKind::Days(n) => shift(date, n as u64),
        ViewKind::Agenda => shift(date, AGENDA_DAYS),
        ViewKind::Tasks | ViewKind::Invitations => date,
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

/// Where instant `t` falls on `day`'s grid: wall-clock minutes since its
/// midnight in `local`, as the hour labels read, clamped to the day. On
/// the days the clocks change this is not the time elapsed: the hour
/// that repeats shares its rows, the hour that is skipped is empty.
pub fn wall_minutes(t: DateTime<Utc>, day: NaiveDate, local: Tz) -> f64 {
    let seen = t.with_timezone(&local).naive_local();
    ((seen - day.and_time(NaiveTime::MIN)).num_minutes() as f64).clamp(0.0, 24.0 * 60.0)
}

/// The instant at wall-clock `minutes` on `day`, the inverse of
/// [`wall_minutes`] (a time in a skipped hour moves on past it).
pub fn at_wall(day: NaiveDate, minutes: f64, local: Tz) -> DateTime<Tz> {
    resolve_local(
        local,
        day.and_time(NaiveTime::MIN) + chrono::TimeDelta::minutes(minutes as i64),
    )
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
    /// Column within its group of occurrences starting together.
    pub column: usize,
    pub columns: usize,
    /// How many earlier occurrences, still running, it is drawn over.
    pub indent: usize,
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
                // by the clock, so it lines up with the hour labels
                let start_min = wall_minutes(s, day, local);
                let end_min = wall_minutes(e, day, local).max(start_min + min_duration_min);
                Segment {
                    index,
                    start_min,
                    end_min,
                    column: 0,
                    columns: 1,
                    indent: 0,
                }
            })
        })
        .collect::<Vec<_>>();

    segs.sort_by(|a, b| {
        a.start_min
            .total_cmp(&b.start_min)
            .then(b.end_min.total_cmp(&a.end_min))
    });

    // Occurrences starting close together share a group, side by side;
    // one starting later goes over those still running, indented.
    struct Group {
        start: f64,
        indent: usize,
        column_ends: Vec<f64>,
        members: Vec<usize>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (j, seg) in segs.iter_mut().enumerate() {
        let start = seg.start_min;
        let running = |g: &Group| g.column_ends.iter().any(|&e| e > start);
        let joined = groups
            .iter()
            .rposition(|g| running(g) && start - g.start <= SIDE_BY_SIDE_MIN);
        let gi = joined.unwrap_or_else(|| {
            let indent = groups
                .iter()
                .filter(|g| running(g))
                .map(|g| g.indent + 1)
                .max()
                .unwrap_or(0);
            groups.push(Group {
                start,
                indent,
                column_ends: Vec::new(),
                members: Vec::new(),
            });
            groups.len() - 1
        });
        let g = &mut groups[gi];
        let col = g
            .column_ends
            .iter()
            .position(|&e| e <= start)
            .unwrap_or_else(|| {
                g.column_ends.push(0.0);
                g.column_ends.len() - 1
            });
        g.column_ends[col] = seg.end_min;
        g.members.push(j);
        seg.column = col;
        seg.indent = g.indent;
    }
    for g in groups {
        for m in g.members {
            segs[m].columns = g.column_ends.len();
        }
    }
    segs
}

/// Occurrences starting at most this far apart sit side by side; a later
/// one is drawn over them instead, leaving their titles showing.
const SIDE_BY_SIDE_MIN: f64 = 30.0;

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
            due: None,
            alarms: Vec::new(),
            organizer: None,
            attendees: Vec::new(),
            parent: None,
        }
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn work_week_and_days() {
        let d = date("2025-10-23"); // Thursday
        let ww = visible_days(ViewKind::WorkWeek, d);
        assert_eq!((ww[0], ww.len()), (date("2025-10-20"), 5));
        let three = visible_days(ViewKind::Days(3), d);
        assert_eq!(three, [d, date("2025-10-24"), date("2025-10-25")]);
        assert_eq!(step(ViewKind::Days(3), d, true), date("2025-10-26"));
        assert_eq!(step(ViewKind::WorkWeek, d, false), date("2025-10-16"));
        // at the weekend, the coming work week
        let sunday = date("2025-10-26");
        assert_eq!(
            visible_days(ViewKind::WorkWeek, sunday)[0],
            date("2025-10-27")
        );
        assert_eq!(
            visible_days(ViewKind::WorkWeek, date("2025-10-25"))[0],
            date("2025-10-27")
        );

        for v in [
            ViewKind::Day,
            ViewKind::Week,
            ViewKind::WorkWeek,
            ViewKind::Days(4),
            ViewKind::Tasks,
        ] {
            assert_eq!(ViewKind::from_name(&v.name()), Some(v));
        }
        assert_eq!(ViewKind::from_name("1days"), None);
        assert_eq!(ViewKind::from_name("15days"), None);
        assert_eq!(ViewKind::from_name("week"), None, "callers resolve week");
        assert!(ViewKind::Days(3).is_grid() && !ViewKind::Month.is_grid());
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
        let place = |i| {
            let s = by_index(i);
            (s.column, s.columns, s.indent)
        };
        // 9:00 and 9:30 start close enough to sit side by side
        assert_eq!(place(0), (0, 2, 0));
        assert_eq!(place(1), (1, 2, 0));
        // 10:00 starts later than 9:30's group: drawn over it
        assert_eq!(place(2), (0, 1, 1));
        // the afternoon one is alone
        assert_eq!(place(3), (0, 1, 0));
        assert_eq!(by_index(3).start_min, 14.0 * 60.0);
    }

    #[test]
    fn later_events_go_over_long_ones() {
        let o = [
            occ("20251023T130000Z", "20251023T210000Z"),
            occ("20251023T150000Z", "20251023T153000Z"),
            occ("20251023T170000Z", "20251024T020000Z"),
            occ("20251023T210000Z", "20251023T220000Z"),
            occ("20251023T210000Z", "20251023T220000Z"),
        ];
        let segs = day_segments(&o, date("2025-10-23"), UTC, 15.0);
        let place = |i| {
            let s = segs.iter().find(|s| s.index == i).unwrap();
            (s.column, s.columns, s.indent)
        };
        // the long one keeps the whole width
        assert_eq!(place(0), (0, 1, 0));
        // later ones go over it rather than squeezing it into a column
        assert_eq!(place(1), (0, 1, 1));
        assert_eq!(place(2), (0, 1, 1));
        // two at the same time sit side by side over what's running
        assert_eq!(place(3), (0, 2, 2));
        assert_eq!(place(4), (1, 2, 2));
    }

    #[test]
    fn days_the_clocks_change_line_up_with_the_labels() {
        let london = chrono_tz::Tz::Europe__London;
        // back an hour at 02:00 BST on 25 October 2026: 09:00 GMT is 09:00Z
        let o = [occ("20261025T090000Z", "20261025T100000Z")];
        let s = day_segments(&o, date("2026-10-25"), london, 15.0);
        assert_eq!((s[0].start_min, s[0].end_min), (540.0, 600.0));
        // forward at 01:00 GMT on 29 March 2026: 09:00 BST is 08:00Z
        let o = [occ("20260329T080000Z", "20260329T090000Z")];
        let s = day_segments(&o, date("2026-03-29"), london, 15.0);
        assert_eq!((s[0].start_min, s[0].end_min), (540.0, 600.0));
        // and back from the grid: 01:30 does not exist that morning
        let t = at_wall(date("2026-03-29"), 90.0, london);
        assert_eq!(t.format("%H:%M %Z").to_string(), "02:00 BST");
        assert_eq!(
            at_wall(date("2026-10-25"), 540.0, london)
                .format("%H:%M %Z")
                .to_string(),
            "09:00 GMT"
        );
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

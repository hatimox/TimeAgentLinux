//! Pure date maths for the Day / Week / Month logged-hour totals (the Linux
//! counterpart of Totals.swift). Weeks start on Monday; a week is the closed
//! range start..=start+6. Offsets are 0 = current period, -1 = previous, ….

use crate::models::TimeEntry;
use chrono::{Datelike, Duration, NaiveDate};

#[derive(Clone, Debug, PartialEq)]
pub struct Period {
    pub start: NaiveDate,
    pub end: NaiveDate, // inclusive
    pub label: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Totals {
    pub day: f64,
    pub week: f64,
    pub month: f64,
    pub day_target: f64,
    pub week_target: f64,
    pub month_target: f64,
    pub day_label: String,
    pub week_label: String,
    pub month_label: String,
}

pub fn day(today: NaiveDate, offset: i32) -> Period {
    let d = today + Duration::days(offset as i64);
    let label = match offset {
        0 => "Today".to_string(),
        -1 => "Yesterday".to_string(),
        _ => d.format("%a %-d %b").to_string(),
    };
    Period { start: d, end: d, label }
}

/// Long label for a day offset, e.g. "Today", "Yesterday", "Friday 6 November 2026".
pub fn long_day_label(today: NaiveDate, offset: i32) -> String {
    match offset {
        0 => "Today".to_string(),
        -1 => "Yesterday".to_string(),
        _ => (today + Duration::days(offset as i64)).format("%A %-d %B %Y").to_string(),
    }
}

pub fn week(today: NaiveDate, offset: i32) -> Period {
    let monday = today - Duration::days(today.weekday().num_days_from_monday() as i64);
    let start = monday + Duration::days(7 * offset as i64);
    let end = start + Duration::days(6);
    let label = match offset {
        0 => "This week".to_string(),
        -1 => "Last week".to_string(),
        _ => format!("{} – {}", start.format("%-d %b"), end.format("%-d %b")),
    };
    Period { start, end, label }
}

pub fn month(today: NaiveDate, offset: i32) -> Period {
    let total = today.year() * 12 + today.month0() as i32 + offset;
    let (y, m0) = (total.div_euclid(12), total.rem_euclid(12) as u32);
    let start = NaiveDate::from_ymd_opt(y, m0 + 1, 1).unwrap();
    let next = if m0 == 11 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1).unwrap()
    } else {
        NaiveDate::from_ymd_opt(y, m0 + 2, 1).unwrap()
    };
    let end = next.pred_opt().unwrap();
    Period { start, end, label: start.format("%B %Y").to_string() }
}

/// Sum of hours whose "YYYY-MM-DD" day falls in the period.
pub fn hours_in(times: &[TimeEntry], p: &Period) -> f64 {
    let (s, e) = (p.start.format("%Y-%m-%d").to_string(), p.end.format("%Y-%m-%d").to_string());
    times.iter().filter(|t| t.day >= s && t.day <= e).map(|t| t.hours).sum()
}

/// Target hours for a period: `hours_per_day` x the working days in it.
/// `is_working(day "YYYY-MM-DD", weekday 0=Sun..6=Sat)` keeps this module pure.
pub fn target_for(p: &Period, hours_per_day: f64, is_working: &impl Fn(&str, u32) -> bool) -> f64 {
    let mut working = 0u32;
    let mut d = p.start;
    while d <= p.end {
        if is_working(&d.format("%Y-%m-%d").to_string(), d.weekday().num_days_from_sunday()) {
            working += 1;
        }
        d = d + Duration::days(1);
    }
    hours_per_day * working as f64
}

pub fn compute(
    times: &[TimeEntry],
    today: NaiveDate,
    offsets: (i32, i32, i32),
    hours_per_day: f64,
    is_working: impl Fn(&str, u32) -> bool,
) -> Totals {
    let (d, w, m) = (day(today, offsets.0), week(today, offsets.1), month(today, offsets.2));
    Totals {
        day: hours_in(times, &d),
        week: hours_in(times, &w),
        month: hours_in(times, &m),
        day_target: target_for(&d, hours_per_day, &is_working),
        week_target: target_for(&w, hours_per_day, &is_working),
        month_target: target_for(&m, hours_per_day, &is_working),
        day_label: d.label,
        week_label: w.label,
        month_label: m.label,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }
    fn te(day: &str, hours: f64) -> TimeEntry {
        TimeEntry { id: 0, item_id: 1, hours, day: day.into(), description: String::new() }
    }

    #[test]
    fn day_labels() {
        let today = d(2026, 11, 9); // Monday
        assert_eq!(day(today, 0).label, "Today");
        assert_eq!(day(today, -1).label, "Yesterday");
        assert_eq!(day(today, -2).label, "Sat 7 Nov");
        assert_eq!(day(today, -8).label, "Sun 1 Nov");
        assert_eq!(long_day_label(today, -2), "Saturday 7 November 2026");
    }

    #[test]
    fn weeks_start_monday_and_are_closed_ranges() {
        let sunday = d(2026, 11, 8);
        let w = week(sunday, 0);
        assert_eq!((w.start, w.end), (d(2026, 11, 2), d(2026, 11, 8)));
        let w = week(d(2026, 11, 9), -1);
        assert_eq!((w.start, w.end), (d(2026, 11, 2), d(2026, 11, 8)));
        assert_eq!(w.label, "Last week");
        assert_eq!(week(d(2026, 11, 9), 0).label, "This week");
        assert_eq!(week(d(2026, 11, 9), -2).label, "26 Oct – 1 Nov");
        assert_eq!(week(d(2026, 11, 9), -1 - 1).end, d(2026, 11, 1));
    }

    #[test]
    fn months_cross_year_and_leap() {
        let m = month(d(2026, 11, 9), 0);
        assert_eq!((m.start, m.end, m.label.as_str()), (d(2026, 11, 1), d(2026, 11, 30), "November 2026"));
        let m = month(d(2026, 11, 9), -11);
        assert_eq!((m.start, m.end), (d(2025, 12, 1), d(2025, 12, 31)));
        assert_eq!(month(d(2026, 11, 9), -10).label, "January 2026");
        assert_eq!(month(d(2028, 3, 15), -1).end, d(2028, 2, 29));
    }

    #[test]
    fn totals_bucket_by_period() {
        let times = vec![te("2026-11-09", 2.0), te("2026-11-08", 1.5), te("2026-11-02", 1.0), te("2026-10-31", 4.0)];
        let t = compute(&times, d(2026, 11, 9), (0, 0, 0), 8.0, |_, _| true);
        assert_eq!((t.day, t.week, t.month), (2.0, 2.0, 4.5));
        let t = compute(&times, d(2026, 11, 9), (-1, -1, -1), 8.0, |_, _| true);
        assert_eq!((t.day, t.week, t.month), (1.5, 2.5, 4.0));
    }

    /// Sat/Sun off, plus the given specific days off.
    fn working(off: &'static [&'static str]) -> impl Fn(&str, u32) -> bool {
        move |day, wd| wd != 0 && wd != 6 && !off.iter().any(|o| *o == day)
    }
    const NOV_OFF: &[&str] = &["2026-11-06", "2026-11-18", "2026-11-11"];

    #[test]
    fn targets_skip_weekends_holidays_and_days_off() {
        let today = d(2026, 11, 9);
        let t = |w: i32, today: NaiveDate| compute(&[], today, (0, w, 0), 8.0, working(NOV_OFF));
        assert_eq!(t(0, today).week_target, 32.0); // 9-13 Nov, 11th off
        assert_eq!(t(0, d(2026, 11, 16)).week_target, 32.0); // 18th off
        assert_eq!(t(-2, today).week_target, 40.0); // 26-30 Oct
        assert_eq!(t(0, today).month_target, 144.0);
        let wed = compute(&[], d(2026, 11, 11), (0, 0, 0), 8.0, working(NOV_OFF));
        assert_eq!(wed.day_target, 0.0);
    }

    #[test]
    fn month_targets_without_holidays() {
        let today = d(2026, 11, 9);
        let t = compute(&[], today, (0, 0, -1), 8.0, working(&[]));
        assert_eq!(t.month_target, 176.0); // October
        let t = compute(&[], today, (0, 0, 1), 8.0, working(&[]));
        assert_eq!(t.month_target, 184.0); // December
    }
}

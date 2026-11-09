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

pub fn compute(times: &[TimeEntry], today: NaiveDate, day_off: i32, week_off: i32, month_off: i32) -> Totals {
    let (d, w, m) = (day(today, day_off), week(today, week_off), month(today, month_off));
    Totals {
        day: hours_in(times, &d),
        week: hours_in(times, &w),
        month: hours_in(times, &m),
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
        let t = compute(&times, d(2026, 11, 9), 0, 0, 0);
        assert_eq!((t.day, t.week, t.month), (2.0, 2.0, 4.5));
        let t = compute(&times, d(2026, 11, 9), -1, -1, -1);
        assert_eq!((t.day, t.week, t.month), (1.5, 2.5, 4.0));
    }
}

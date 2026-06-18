//! Morocco civil + (configured) religious holiday days, and the day-off check
//! used to skip recurring auto-logging. Ports the Electron holidays.js intent.

use crate::settings::Settings;
use chrono::{Datelike, NaiveDate, Weekday};
use std::collections::HashSet;

/// Fixed Morocco civil holidays for a given year (month, day).
fn fixed_civil(year: i32) -> Vec<(u32, u32)> {
    // Gregorian-fixed Moroccan public holidays.
    let _ = year;
    vec![
        (1, 1),   // New Year
        (1, 11),  // Proclamation of Independence
        (5, 1),   // Labour Day
        (7, 30),  // Throne Day
        (8, 14),  // Oued Ed-Dahab
        (8, 20),  // Revolution Day
        (8, 21),  // Youth Day
        (11, 6),  // Green March
        (11, 18), // Independence Day
    ]
}

/// All off-days for a year: weekly-off weekdays are handled separately; this
/// returns the set of specific YYYY-MM-DD strings (user days off + civil
/// holidays when region=morocco). Religious days come from settings.days_off if
/// the user enabled them (the Electron app flattens enabled religious slots into
/// daysOff-like state; here we rely on days_off + civil set).
pub fn days_off_for_year(s: &Settings, year: i32) -> HashSet<String> {
    let mut out: HashSet<String> = s.days_off.iter().cloned().collect();
    if s.region == "morocco" {
        for (m, d) in fixed_civil(year) {
            if let Some(date) = NaiveDate::from_ymd_opt(year, m, d) {
                out.insert(date.format("%Y-%m-%d").to_string());
            }
        }
    }
    out
}

/// Is `date` a non-working day (weekly off or a specific day off)?
pub fn is_day_off(s: &Settings, date: NaiveDate) -> bool {
    // weekly_off: 0=Sun … 6=Sat (matches the Electron weeklyOff).
    let dow = match date.weekday() {
        Weekday::Sun => 0,
        Weekday::Mon => 1,
        Weekday::Tue => 2,
        Weekday::Wed => 3,
        Weekday::Thu => 4,
        Weekday::Fri => 5,
        Weekday::Sat => 6,
    };
    if s.weekly_off.contains(&(dow as i64)) {
        return true;
    }
    let key = date.format("%Y-%m-%d").to_string();
    days_off_for_year(s, date.year()).contains(&key)
}

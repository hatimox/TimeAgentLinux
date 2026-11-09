//! Morocco civil + religious holiday days, and the day-off check
//! used to skip recurring auto-logging. Ports the Electron holidays.js intent.

use crate::settings::Settings;
use chrono::{Datelike, NaiveDate, Weekday};
use std::collections::HashSet;

/// Fixed Morocco civil holidays (month, day, name), same dates every year.
const FIXED_CIVIL: &[(u32, u32, &str)] = &[
    (1, 1, "New Year's Day"),
    (1, 11, "Proclamation of Independence"),
    (1, 14, "Amazigh New Year"),
    (5, 1, "Labour Day"),
    (7, 30, "Throne Day"),
    (8, 14, "Oued Ed-Dahab Day"),
    (8, 20, "Revolution Day"),
    (8, 21, "Youth Day"),
    (11, 6, "Green March Day"),
    (11, 18, "Independence Day"),
];

/// Fixed civil holidays of `year` as (YYYY-MM-DD, name).
pub fn fixed_holidays_named(year: i32) -> Vec<(String, &'static str)> {
    FIXED_CIVIL
        .iter()
        .filter_map(|&(m, d, name)| {
            NaiveDate::from_ymd_opt(year, m, d).map(|date| (date.format("%Y-%m-%d").to_string(), name))
        })
        .collect()
}

type Holiday = (&'static str, &'static [&'static str]);

/// Religious holiday ESTIMATES by year (moon-sighting dependent, may move +/-1
/// day). Order matters: slot keys are "year|name|index". Mirrors the Mac table.
const RELIGIOUS: &[(i32, &[Holiday])] = &[
    (
        2026,
        &[
            ("Eid al-Fitr", &["2026-03-20", "2026-03-21"]),
            ("Eid al-Adha", &["2026-05-26", "2026-05-27"]),
            ("Islamic New Year", &["2026-06-16"]),
            ("Prophet's Birthday", &["2026-08-25", "2026-08-26"]),
        ],
    ),
    (
        2027,
        &[
            ("Eid al-Fitr", &["2027-03-09", "2027-03-10", "2027-03-11"]),
            ("Eid al-Adha", &["2027-05-16", "2027-05-17"]),
            ("Islamic New Year", &["2027-06-06"]),
            ("Prophet's Birthday", &["2027-08-15"]),
        ],
    ),
    (
        2028,
        &[
            ("Eid al-Fitr", &["2028-02-26", "2028-02-27"]),
            ("Eid al-Adha", &["2028-05-05", "2028-05-06"]),
            ("Islamic New Year", &["2028-05-25"]),
            ("Prophet's Birthday", &["2028-08-03"]),
        ],
    ),
];

/// One religious-holiday day: stable settings slot key, display label and the
/// estimated date.
#[derive(Clone, Debug, PartialEq)]
pub struct ReligiousDay {
    pub key: String,
    pub label: String,
    pub estimate: String,
}

/// Every estimated religious-holiday day of a year, flattened in order.
/// Empty when there are no estimates for that year.
pub fn religious_days(year: i32) -> Vec<ReligiousDay> {
    let Some((_, list)) = RELIGIOUS.iter().find(|(y, _)| *y == year) else {
        return vec![];
    };
    let mut out = vec![];
    for (name, dates) in list.iter() {
        for (i, date) in dates.iter().enumerate() {
            let label = if dates.len() > 1 {
                format!("{} (day {})", name, i + 1)
            } else {
                (*name).to_string()
            };
            out.push(ReligiousDay { key: format!("{}|{}|{}", year, name, i), label, estimate: (*date).to_string() });
        }
    }
    out
}

/// All specific off-days for a year as YYYY-MM-DD strings (weekly-off weekdays
/// are handled separately): the user's days off, plus
/// - region "morocco": the fixed civil holidays and every religious day, using
///   its saved slot (on flag, edited date) or, with no slot, the estimate;
/// - any other region: only saved religious slots that are on with a date.
///
/// Feeds both recurring auto-log skipping and the hour targets.
pub fn days_off_for_year(s: &Settings, year: i32) -> HashSet<String> {
    let mut out: HashSet<String> = s.days_off.iter().cloned().collect();
    if s.region == "morocco" {
        out.extend(fixed_holidays_named(year).into_iter().map(|(d, _)| d));
        for day in religious_days(year) {
            match s.religious_slots.iter().find(|r| r.key == day.key) {
                Some(slot) => {
                    if slot.on {
                        out.insert(if slot.date.is_empty() { day.estimate } else { slot.date.clone() });
                    }
                }
                None => {
                    out.insert(day.estimate);
                }
            }
        }
    } else {
        for slot in &s.religious_slots {
            if slot.on && !slot.date.is_empty() {
                out.insert(slot.date.clone());
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ReligiousSlot;

    fn morocco() -> Settings {
        Settings { region: "morocco".into(), ..Settings::default() }
    }
    fn slot(key: &str, date: &str, on: bool) -> ReligiousSlot {
        ReligiousSlot { key: key.into(), date: date.into(), on }
    }

    #[test]
    fn morocco_without_slots_uses_civil_and_estimates() {
        let off = days_off_for_year(&morocco(), 2026);
        assert!(off.contains("2026-03-20"));
        assert!(off.contains("2026-11-18"));
    }

    #[test]
    fn switched_off_slot_removes_date() {
        let mut s = morocco();
        s.religious_slots.push(slot("2026|Eid al-Fitr|0", "2026-03-20", false));
        let off = days_off_for_year(&s, 2026);
        assert!(!off.contains("2026-03-20"));
        assert!(off.contains("2026-03-21"));
    }

    #[test]
    fn edited_slot_date_replaces_estimate() {
        let mut s = morocco();
        s.religious_slots.push(slot("2026|Eid al-Fitr|0", "2026-03-21", true));
        let off = days_off_for_year(&s, 2026);
        assert!(!off.contains("2026-03-20"));
        assert!(off.contains("2026-03-21"));
    }

    #[test]
    fn other_regions_only_use_enabled_slots() {
        let mut s = Settings { region: "none".into(), ..Settings::default() };
        s.religious_slots.push(slot("2026|Eid al-Fitr|0", "2026-03-20", true));
        s.religious_slots.push(slot("2026|Eid al-Fitr|1", "2026-03-21", false));
        let off = days_off_for_year(&s, 2026);
        assert!(off.contains("2026-03-20"));
        assert!(!off.contains("2026-03-21"));
        assert!(!off.contains("2026-11-18"));
    }
}

use crate::models::{DynamicMeeting, RecurringEntry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Token lives in libsecret via the same service name the Electron app used.
const KEYRING_SERVICE: &str = "net.omnevo.timeagent";
const KEYRING_ACCOUNT: &str = "tp-token";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    #[serde(rename = "tpURL")]
    pub tp_url: String,
    #[serde(rename = "myUserId")]
    pub my_user_id: i64,
    #[serde(rename = "myUserName")]
    pub my_user_name: String,
    #[serde(rename = "myUserEmail")]
    pub my_user_email: String,
    pub timezone: String,
    #[serde(rename = "dailyTaskId")]
    pub daily_task_id: i64,
    #[serde(rename = "meetingsTaskId")]
    pub meetings_task_id: i64,
    #[serde(rename = "meetingMinMinutes")]
    pub meeting_min_minutes: i64,
    #[serde(rename = "meetingStepMinutes")]
    pub meeting_step_minutes: i64,
    pub recurring: Vec<RecurringEntry>,
    #[serde(rename = "dynamicMeetings")]
    pub dynamic_meetings: Vec<DynamicMeeting>,
    #[serde(rename = "daysOff")]
    pub days_off: Vec<String>,
    #[serde(rename = "weeklyOff")]
    pub weekly_off: Vec<i64>,
    pub region: String,

    // Not persisted to JSON — the token comes from the keyring.
    #[serde(skip)]
    pub token: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            tp_url: String::new(),
            my_user_id: 0,
            my_user_name: String::new(),
            my_user_email: String::new(),
            timezone: iana_time_zone::get_timezone().unwrap_or_else(|_| "UTC".into()),
            daily_task_id: 0,
            meetings_task_id: 0,
            meeting_min_minutes: 30,
            meeting_step_minutes: 15,
            recurring: vec![],
            dynamic_meetings: vec![],
            days_off: vec![],
            weekly_off: vec![0, 6],
            region: "none".into(),
            token: String::new(),
        }
    }
}

impl Settings {
    /// ~/.config/TimeAgent (matches the Electron Linux userData fallback).
    pub fn dir() -> PathBuf {
        let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
        let d = base.join("TimeAgent");
        let _ = std::fs::create_dir_all(&d);
        d
    }
    fn file() -> PathBuf {
        Self::dir().join("settings.json")
    }

    pub fn is_configured(&self) -> bool {
        !self.token.is_empty() && self.tp_url.starts_with("http")
    }

    pub fn load() -> Self {
        let mut s: Settings = std::fs::read_to_string(Self::file())
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        s.token = read_token().unwrap_or_default();
        s
    }

    pub fn save(&self) {
        let _ = write_token(&self.token);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::file(), json);
        }
    }

    /// Minutes east of UTC for the configured timezone.
    pub fn tz_offset_minutes(&self) -> i32 {
        use chrono::Offset;
        let tz: chrono_tz::Tz = self.timezone.parse().unwrap_or(chrono_tz::UTC);
        let now = chrono::Utc::now().with_timezone(&tz);
        now.offset().fix().local_minus_utc() / 60
    }
}

fn entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_ACCOUNT)
}
pub fn read_token() -> Option<String> {
    entry().ok()?.get_password().ok().map(|s| s.trim().to_string())
}
pub fn write_token(token: &str) -> keyring::Result<()> {
    let e = entry()?;
    if token.is_empty() {
        let _ = e.delete_credential();
        Ok(())
    } else {
        e.set_password(token)
    }
}

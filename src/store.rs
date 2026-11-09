//! Shared application state + all TP operations. Held as Arc<Store>; the GTK UI
//! (main thread) reads cached data and dispatches async ops onto the tokio
//! runtime, which write results back behind the mutexes and notify the UI via a
//! glib channel.

use crate::holidays;
use crate::models::*;
use crate::settings::Settings;
use crate::tpclient::TpClient;
use anyhow::Result;
use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// UI-bound notifications sent from async ops back to the GTK main loop.
#[derive(Clone, Debug)]
pub enum Update {
    Items,
    Times,
    Status(String),
    UserInfo,
    MeetingEnded { start_ms: i64, end_ms: i64 },
    MeetingState(bool),
    /// Open the hours + description dialog (timer stop that needs a description
    /// or whose first attempt failed).
    LogPrompt { entity_id: i64, title: String, hours: f64, description: String, date: NaiveDate, error: Option<String> },
}

pub const DESCRIPTION_REQUIRED_MSG: &str = "TargetProcess requires a description for time entries.";

pub struct Store {
    pub settings: Mutex<Settings>,
    pub client: Mutex<Option<TpClient>>,
    pub items: Mutex<Vec<WorkItem>>,
    pub times: Mutex<Vec<TimeEntry>>,
    pub states: Mutex<HashMap<i64, HashMap<String, Vec<WorkflowState>>>>,
    pub scope_all: Mutex<bool>,
    pub in_meeting: Mutex<bool>,
    /// When a meeting is being tracked, the monotonic instant it started — used
    /// for the live elapsed timer in the popup.
    meeting_start: Mutex<Option<Instant>>,
    /// Manual per-task stopwatch: (item id, started-at). Only one at a time.
    active_task: Mutex<Option<(i64, Instant)>>,
    last_status: Mutex<String>,
    tx: async_channel::Sender<Update>,
    rt: tokio::runtime::Handle,
}

impl Store {
    pub fn new(tx: async_channel::Sender<Update>, rt: tokio::runtime::Handle) -> Arc<Self> {
        let s = Settings::load();
        let client = if s.is_configured() {
            Some(TpClient::new(&s.tp_url, &s.token, s.my_user_id))
        } else {
            None
        };
        Arc::new(Store {
            settings: Mutex::new(s),
            client: Mutex::new(client),
            items: Mutex::new(vec![]),
            times: Mutex::new(vec![]),
            states: Mutex::new(HashMap::new()),
            scope_all: Mutex::new(false),
            in_meeting: Mutex::new(false),
            meeting_start: Mutex::new(None),
            active_task: Mutex::new(None),
            last_status: Mutex::new(String::new()),
            tx,
            rt,
        })
    }

    fn notify(&self, u: Update) {
        // Keep the latest status line so UIs can poll it (the update channel has
        // a single consumer in main, so windows can't subscribe directly).
        if let Update::Status(s) = &u {
            *self.last_status.lock().unwrap() = s.clone();
        }
        // Unbounded channel: try_send only fails if the receiver is gone.
        let _ = self.tx.try_send(u);
    }

    /// Latest status line (e.g. last error), for windows that poll.
    pub fn last_status(&self) -> String {
        self.last_status.lock().unwrap().clone()
    }

    /// The detected user id + display name (id 0 until detection completes).
    pub fn user_identity(&self) -> (i64, String) {
        let s = self.settings.lock().unwrap();
        (s.my_user_id, s.my_user_name.clone())
    }

    /// Record meeting state: `Some(start)` while tracking, `None` when idle.
    pub fn set_meeting(&self, start: Option<Instant>) {
        *self.in_meeting.lock().unwrap() = start.is_some();
        *self.meeting_start.lock().unwrap() = start;
    }

    /// Elapsed time of the current meeting, if one is being tracked.
    pub fn meeting_elapsed(&self) -> Option<std::time::Duration> {
        self.meeting_start.lock().unwrap().map(|s| s.elapsed())
    }

    /// Today in the configured timezone (what the totals bucket against).
    pub fn today(&self) -> NaiveDate {
        let off = chrono::FixedOffset::east_opt(self.tz_off() * 60).unwrap_or_else(|| chrono::FixedOffset::east_opt(0).unwrap());
        chrono::Utc::now().with_timezone(&off).date_naive()
    }

    /// Day / week / month totals, each shifted back by its own offset.
    pub fn totals(&self, day_off: i32, week_off: i32, month_off: i32) -> crate::periods::Totals {
        // Clone first so the settings lock is not held while locking times.
        let s = self.settings.lock().unwrap().clone();
        let cache: std::cell::RefCell<HashMap<i32, std::collections::HashSet<String>>> = Default::default();
        let is_working = |day: &str, weekday: u32| {
            if s.weekly_off.contains(&(weekday as i64)) {
                return false;
            }
            let year: i32 = day.get(..4).and_then(|y| y.parse().ok()).unwrap_or(0);
            let mut c = cache.borrow_mut();
            !c.entry(year).or_insert_with(|| holidays::days_off_for_year(&s, year)).contains(day)
        };
        let times = self.times.lock().unwrap();
        crate::periods::compute(&times, self.today(), (day_off, week_off, month_off), s.hours_per_day, is_working)
    }

    /// Surface a status line to windows that poll `last_status`.
    pub fn set_status(&self, s: String) {
        self.notify(Update::Status(s));
    }

    /// The currently-tracked task and its elapsed time, if any.
    pub fn tracking_task(&self) -> Option<(i64, std::time::Duration)> {
        self.active_task.lock().unwrap().map(|(id, start)| (id, start.elapsed()))
    }

    /// Display name for a loaded work item.
    pub fn item_name(&self, id: i64) -> Option<String> {
        self.items.lock().unwrap().iter().find(|i| i.id == id).map(|i| i.name.clone())
    }

    /// Start the stopwatch on a task. Stops + logs any task already running.
    pub fn start_task(self: &Arc<Self>, item_id: i64) {
        self.stop_task();
        *self.active_task.lock().unwrap() = Some((item_id, Instant::now()));
        self.notify(Update::Status(format!("Tracking #{}…", item_id)));
        self.notify(Update::Items); // re-render rows (Start → Stop)
    }

    /// Stop the stopwatch and log the elapsed time to that task. `log_time`
    /// reports "Logged Xh to #id" on success and reloads the time entries.
    pub fn stop_task(self: &Arc<Self>) {
        let active = self.active_task.lock().unwrap().take();
        if let Some((id, start)) = active {
            let secs = start.elapsed().as_secs_f64();
            // Round to 0.01h, but log at least 0.01h for any real (>=1s) track.
            let hours = if secs >= 1.0 {
                (((secs / 3600.0) * 100.0).round() / 100.0).max(0.01)
            } else {
                0.0
            };
            if hours > 0.0 {
                let today = chrono::Local::now().date_naive();
                let name = self.item_name(id).unwrap_or_else(|| format!("#{}", id));
                let title = format!("Log {:.2}h to “{}”", hours, name);
                let prompt = move |error: Option<String>| Update::LogPrompt {
                    entity_id: id, title: title.clone(), hours, description: name.clone(), date: today, error,
                };
                // When a description is required, or the save fails, a dialog
                // (prefilled with the task name) stays up until saved/discarded.
                if self.settings.lock().unwrap().time_description_required {
                    self.notify(prompt(None));
                } else {
                    let rx = self.log_time(id, hours, String::new(), today);
                    let me = self.clone();
                    self.rt.spawn(async move {
                        if let Ok(Err(e)) = rx.recv().await {
                            me.notify(prompt(Some(e)));
                        }
                    });
                }
            }
            self.notify(Update::Items);
        }
    }

    /// A clone of the update sender, for the watcher-forwarding task.
    pub fn sender(&self) -> async_channel::Sender<Update> {
        self.tx.clone()
    }

    pub fn rebuild_client(&self) {
        let s = self.settings.lock().unwrap();
        *self.client.lock().unwrap() = if s.is_configured() {
            Some(TpClient::new(&s.tp_url, &s.token, s.my_user_id))
        } else {
            None
        };
    }

    fn client(&self) -> Option<TpClient> {
        self.client.lock().unwrap().clone()
    }
    fn tz_off(&self) -> i32 {
        self.settings.lock().unwrap().tz_offset_minutes()
    }

    /// Detect the user behind the token if not yet known, then store it.
    pub fn ensure_user(self: &Arc<Self>) {
        let me = self.clone();
        let need = {
            let s = self.settings.lock().unwrap();
            s.my_user_id == 0 || s.my_user_name.is_empty()
        };
        if !need {
            return;
        }
        let Some(client) = self.client() else { return };
        self.rt.spawn(async move {
            match client.who_am_i().await {
                Ok((id, name, email)) => {
                    {
                        let mut s = me.settings.lock().unwrap();
                        s.my_user_id = id;
                        s.my_user_name = name;
                        s.my_user_email = email;
                        s.save();
                    }
                    if let Some(c) = me.client.lock().unwrap().as_mut() {
                        c.my_user_id = id;
                    }
                    me.notify(Update::UserInfo);
                    // The startup refresh ran with user id 0 (detection not done
                    // yet) and returned nothing; re-fetch now that we know the user.
                    me.refresh();
                }
                Err(e) => me.notify(Update::Status(format!("Sign-in failed: {}", e))),
            }
        });
    }

    pub fn refresh(self: &Arc<Self>) {
        let Some(client) = self.client() else { return };
        let scope_all = *self.scope_all.lock().unwrap();
        let me = self.clone();
        self.notify(Update::Status("Loading…".into()));
        self.rt.spawn(async move {
            match tokio::try_join!(client.fetch_all_assigned(!scope_all), client.fetch_my_times()) {
                Ok((items, times)) => {
                    let n = items.len();
                    *me.items.lock().unwrap() = items;
                    *me.times.lock().unwrap() = times;
                    me.notify(Update::Items);
                    me.notify(Update::Times);
                    me.notify(Update::Status(format!("Loaded {} items{}", n, if scope_all { "" } else { " (current sprint)" })));
                }
                Err(e) => me.notify(Update::Status(format!("Load failed: {}", e))),
            }
        });
    }

    pub fn hours_for(&self, item_id: i64) -> f64 {
        self.times.lock().unwrap().iter().filter(|t| t.item_id == item_id).map(|t| t.hours).sum()
    }

    /// Log a time entry. The returned receiver yields `Ok(())` once saved, or
    /// the error text to show; callers keep their form open until it is `Ok`.
    /// Callers that don't care can drop it.
    pub fn log_time(self: &Arc<Self>, entity_id: i64, hours: f64, description: String, date: NaiveDate) -> async_channel::Receiver<Result<(), String>> {
        let (tx, rx) = async_channel::bounded(1);
        let Some(client) = self.client() else {
            let _ = tx.try_send(Err("TargetProcess is not configured".to_string()));
            return rx;
        };
        let desc = description.trim().to_string();
        if desc.is_empty() && self.settings.lock().unwrap().time_description_required {
            let _ = tx.try_send(Err(DESCRIPTION_REQUIRED_MSG.to_string()));
            return rx;
        }
        let off = self.tz_off();
        let me = self.clone();
        self.rt.spawn(async move {
            let res = match client.log_time(entity_id, hours, &desc, date, off).await {
                Ok(_) => {
                    me.notify(Update::Status(format!("Logged {:.2}h to #{}", hours, entity_id)));
                    me.reload_times().await;
                    Ok(())
                }
                Err(e) => {
                    let m = if me.note_description_requirement(&e) { DESCRIPTION_REQUIRED_MSG.to_string() } else { e.to_string() };
                    me.notify(Update::Status(format!("Log failed: {}", m)));
                    Err(m)
                }
            };
            let _ = tx.send(res).await;
        });
        rx
    }

    /// If TP rejected a time entry for a missing description, remember that (so
    /// every form asks for one from now on) and return true.
    pub fn note_description_requirement(&self, e: &anyhow::Error) -> bool {
        let Some(te) = e.downcast_ref::<crate::tpclient::TpError>() else { return false };
        if te.status != 400 || !te.message.to_lowercase().contains("description") {
            return false;
        }
        let mut s = self.settings.lock().unwrap();
        if !s.time_description_required {
            s.time_description_required = true;
            s.save();
        }
        true
    }

    pub fn update_time(self: &Arc<Self>, time_id: i64, hours: f64, description: String, day: String) {
        let Some(client) = self.client() else { return };
        let off = self.tz_off();
        let me = self.clone();
        let date = NaiveDate::parse_from_str(&day, "%Y-%m-%d").ok();
        self.rt.spawn(async move {
            match client.update_time(time_id, Some(hours), Some(&description), date, off).await {
                Ok(_) => {
                    me.notify(Update::Status("Time entry updated".into()));
                    me.reload_times().await;
                }
                Err(e) => {
                    let m = if me.note_description_requirement(&e) { DESCRIPTION_REQUIRED_MSG.to_string() } else { e.to_string() };
                    me.notify(Update::Status(format!("Save failed: {}", m)));
                }
            }
        });
    }

    pub fn delete_time(self: &Arc<Self>, time_id: i64) {
        let Some(client) = self.client() else { return };
        let me = self.clone();
        self.rt.spawn(async move {
            match client.delete_time(time_id).await {
                Ok(_) => {
                    me.notify(Update::Status("Time entry deleted".into()));
                    me.reload_times().await;
                }
                Err(e) => me.notify(Update::Status(format!("Delete failed: {}", e))),
            }
        });
    }

    async fn reload_times(self: &Arc<Self>) {
        if let Some(c) = self.client() {
            if let Ok(times) = c.fetch_my_times().await {
                *self.times.lock().unwrap() = times;
                self.notify(Update::Times);
            }
        }
    }

    /// Workflow states for an item's process+type (cached). Synchronous read of
    /// the cache; fetches in the background and notifies if missing.
    pub fn states_for(self: &Arc<Self>, item: &WorkItem) -> Vec<WorkflowState> {
        let key = item.state_key();
        if let Some(byproc) = self.states.lock().unwrap().get(&item.process_id) {
            if let Some(v) = byproc.get(key) {
                return v.clone();
            }
        }
        // fetch in background
        let Some(client) = self.client() else { return vec![] };
        let me = self.clone();
        let pid = item.process_id;
        self.rt.spawn(async move {
            if let Ok(map) = client.fetch_process_states(pid).await {
                me.states.lock().unwrap().insert(pid, map);
                me.notify(Update::Items);
            }
        });
        vec![]
    }

    pub fn change_state(self: &Arc<Self>, item: WorkItem, state: WorkflowState) {
        let Some(client) = self.client() else { return };
        let me = self.clone();
        self.rt.spawn(async move {
            match client.set_entity_state(&item.entity_type, item.id, state.id).await {
                Ok((name, is_final)) => {
                    {
                        let mut items = me.items.lock().unwrap();
                        if let Some(it) = items.iter_mut().find(|i| i.id == item.id) {
                            it.state_id = state.id;
                            it.state_name = name.clone();
                            it.is_final = is_final;
                        }
                    }
                    me.notify(Update::Items);
                    me.notify(Update::Status(format!("#{} → {}", item.id, name)));
                }
                Err(e) => me.notify(Update::Status(format!("Status change failed: {}", e))),
            }
        });
    }

    pub fn billable_hours(&self, raw: f64) -> f64 {
        let s = self.settings.lock().unwrap();
        let step = (s.meeting_step_minutes.max(1) as f64) / 60.0;
        let min_h = (s.meeting_min_minutes.max(0) as f64) / 60.0;
        min_h.max((raw / step).ceil() * step)
    }

    /// Fire today's recurring entries once, skipping days off. Idempotent via a
    /// marker file (date + entry id), matching the Electron recurring behavior.
    pub fn log_recurring_if_due(self: &Arc<Self>) {
        let (recurring, off, today) = {
            let s = self.settings.lock().unwrap();
            let today = chrono::Local::now().date_naive();
            if holidays::is_day_off(&s, today) {
                return;
            }
            (s.recurring.clone(), s.tz_offset_minutes(), today)
        };
        let marker = Settings::dir().join("recurring_logged.json");
        let mut logged: HashMap<String, bool> =
            std::fs::read_to_string(&marker).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let day_key = today.format("%Y-%m-%d").to_string();
        let Some(client) = self.client() else { return };
        for r in recurring {
            if r.task_id == 0 {
                continue;
            }
            let k = format!("{}|{}", day_key, r.id);
            if logged.get(&k).copied().unwrap_or(false) {
                continue;
            }
            logged.insert(k, true);
            // TP may require a description: label, else the task name.
            let desc = if !r.label.trim().is_empty() {
                r.label.clone()
            } else {
                self.item_name(r.task_id).unwrap_or_else(|| "Recurring time".to_string())
            };
            let c = client.clone();
            let me = self.clone();
            self.rt.spawn(async move {
                if let Err(e) = c.log_time(r.task_id, r.hours, &desc, today, off).await {
                    me.note_description_requirement(&e);
                    me.notify(Update::Status(format!("Recurring log failed: {}", e)));
                }
            });
        }
        if let Ok(j) = serde_json::to_string_pretty(&logged) {
            let _ = std::fs::write(&marker, j);
        }
        let me = self.clone();
        self.rt.spawn(async move { me.reload_times().await; });
    }

    pub fn open_in_tp(&self, id: i64) {
        let base = { self.settings.lock().unwrap().tp_url.trim_end_matches('/').to_string() };
        let url = format!("{}/entity/{}", base, id);
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
}

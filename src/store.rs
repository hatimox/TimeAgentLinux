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

/// UI-bound notifications sent from async ops back to the GTK main loop.
#[derive(Clone, Debug)]
pub enum Update {
    Items,
    Times,
    Status(String),
    UserInfo,
    MeetingEnded { start_ms: i64, end_ms: i64 },
    MeetingState(bool),
}

pub struct Store {
    pub settings: Mutex<Settings>,
    pub client: Mutex<Option<TpClient>>,
    pub items: Mutex<Vec<WorkItem>>,
    pub times: Mutex<Vec<TimeEntry>>,
    pub states: Mutex<HashMap<i64, HashMap<String, Vec<WorkflowState>>>>,
    pub scope_all: Mutex<bool>,
    pub in_meeting: Mutex<bool>,
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
            tx,
            rt,
        })
    }

    fn notify(&self, u: Update) {
        // Unbounded channel: try_send only fails if the receiver is gone.
        let _ = self.tx.try_send(u);
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
            if let Ok((id, name, email)) = client.who_am_i().await {
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

    pub fn log_time(self: &Arc<Self>, entity_id: i64, hours: f64, description: String, date: NaiveDate) {
        let Some(client) = self.client() else { return };
        let off = self.tz_off();
        let me = self.clone();
        self.rt.spawn(async move {
            match client.log_time(entity_id, hours, &description, date, off).await {
                Ok(_) => {
                    me.notify(Update::Status(format!("Logged {:.2}h to #{}", hours, entity_id)));
                    me.reload_times().await;
                }
                Err(e) => me.notify(Update::Status(format!("Log failed: {}", e))),
            }
        });
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
                Err(e) => me.notify(Update::Status(format!("Save failed: {}", e))),
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
        let key = if item.entity_type == "Bugs" { "Bug" } else { "Task" };
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
            let c = client.clone();
            self.rt.spawn(async move {
                let _ = c.log_time(r.task_id, r.hours, "", today, off).await;
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

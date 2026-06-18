use crate::models::*;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;

/// TargetProcess REST client — faithful port of the Electron tpclient.js:
/// noon-anchored writes, offset-aware day bucketing, manual %20 query encoding,
/// skip-based pagination (TP caps a page at 1000).
#[derive(Clone)]
pub struct TpClient {
    base_url: String,
    token: String,
    pub my_user_id: i64,
    http: reqwest::Client,
}

impl TpClient {
    pub fn new(base_url: &str, token: &str, my_user_id: i64) -> Self {
        let base = base_url.trim_end_matches('/').to_string();
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(25))
            .build()
            .expect("http client");
        TpClient { base_url: base, token: token.to_string(), my_user_id, http }
    }

    // %20 for spaces (TP rejects '+').
    fn enc(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                _ => out.push_str(&format!("%{:02X}", b)),
            }
        }
        out
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> String {
        let mut params: Vec<(String, String)> =
            vec![("format".into(), "json".into()), ("access_token".into(), self.token.clone())];
        for (k, v) in query {
            params.push((k.to_string(), v.to_string()));
        }
        let qs = params
            .iter()
            .map(|(k, v)| format!("{}={}", Self::enc(k), Self::enc(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}/api/v1/{}?{}", self.base_url, path, qs)
    }

    async fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<Value> {
        let resp = self.http.get(self.url(path, query)).header("Accept", "application/json").send().await?;
        Self::parse(resp).await
    }
    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self
            .http
            .post(self.url(path, &[]))
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;
        Self::parse(resp).await
    }
    async fn delete(&self, path: &str) -> Result<()> {
        let resp = self.http.delete(self.url(path, &[])).send().await?;
        if resp.status().is_success() { Ok(()) } else { Err(anyhow!("HTTP {}", resp.status())) }
    }

    async fn parse(resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(anyhow!("HTTP {}: {}", status, text.chars().take(200).collect::<String>()));
        }
        if text.is_empty() { return Ok(json!({})); }
        Ok(serde_json::from_str(&text)?)
    }

    async fn get_all(&self, path: &str, query: &[(&str, &str)]) -> Result<Vec<Value>> {
        let take = 1000;
        let mut items = vec![];
        let mut skip = 0;
        loop {
            let skip_s = skip.to_string();
            let mut q = query.to_vec();
            q.push(("take", "1000"));
            q.push(("skip", &skip_s));
            let obj = self.get(path, &q).await?;
            let batch = obj.get("Items").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let n = batch.len();
            items.extend(batch);
            if n < take || n == 0 { break; }
            skip += take;
        }
        Ok(items)
    }

    // identity
    pub async fn who_am_i(&self) -> Result<(i64, String, String)> {
        let obj = self.get("Context", &[]).await?;
        let u = obj.get("LoggedUser").ok_or_else(|| anyhow!("no LoggedUser"))?;
        let id = u.get("Id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow!("no Id"))?;
        let name = format!(
            "{} {}",
            u.get("FirstName").and_then(|v| v.as_str()).unwrap_or(""),
            u.get("LastName").and_then(|v| v.as_str()).unwrap_or("")
        )
        .trim()
        .to_string();
        let email = u.get("Email").and_then(|v| v.as_str()).unwrap_or("").to_string();
        Ok((id, name, email))
    }

    pub async fn fetch_all_assigned(&self, current_sprint_only: bool) -> Result<Vec<WorkItem>> {
        let (tasks, bugs) = tokio::try_join!(
            self.fetch_collection("Tasks", current_sprint_only),
            self.fetch_collection("Bugs", current_sprint_only)
        )?;
        let mut all = tasks;
        all.extend(bugs);
        all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        Ok(all)
    }

    async fn fetch_collection(&self, collection: &str, current_sprint_only: bool) -> Result<Vec<WorkItem>> {
        let mut where_clause = format!("AssignedUser.Id eq {}", self.my_user_id);
        if current_sprint_only {
            where_clause.push_str(" and (TeamIteration.IsCurrent eq 'true')");
        }
        let items = self
            .get_all(
                collection,
                &[
                    ("where", &where_clause),
                    ("include", "[Id,Name,EntityState[Id,Name,IsFinal],Project[Name,Process[Id]],TeamIteration[Name],UserStory[Id,Name]]"),
                ],
            )
            .await?;
        Ok(items
            .iter()
            .filter_map(|it| {
                let id = it.get("Id")?.as_i64()?;
                let es = it.get("EntityState").cloned().unwrap_or(json!({}));
                let project = it.get("Project").cloned().unwrap_or(json!({}));
                let us = it.get("UserStory").cloned().unwrap_or(json!({}));
                Some(WorkItem {
                    id,
                    name: it.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    entity_type: collection.to_string(),
                    display_type: if collection == "Bugs" { "Bug" } else { "Task" }.to_string(),
                    state_id: es.get("Id").and_then(|v| v.as_i64()).unwrap_or(0),
                    state_name: es.get("Name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
                    is_final: es.get("IsFinal").and_then(|v| v.as_bool()).unwrap_or(false),
                    project_name: project.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    process_id: project.get("Process").and_then(|p| p.get("Id")).and_then(|v| v.as_i64()).unwrap_or(0),
                    sprint: it.get("TeamIteration").and_then(|t| t.get("Name")).and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    us_id: us.get("Id").and_then(|v| v.as_i64()).unwrap_or(0),
                    us_name: us.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                })
            })
            .collect())
    }

    pub async fn fetch_process_states(&self, process_id: i64) -> Result<HashMap<String, Vec<WorkflowState>>> {
        let mut out: HashMap<String, Vec<WorkflowState>> = HashMap::new();
        out.insert("Task".into(), vec![]);
        out.insert("Bug".into(), vec![]);
        if process_id == 0 { return Ok(out); }
        for etype in ["Task", "Bug"] {
            let where_clause = format!("(Process.Id eq {}) and (EntityType.Name eq '{}')", process_id, etype);
            let obj = match self.get("EntityStates", &[("where", &where_clause), ("include", "[Id,Name,NumericPriority,IsFinal]"), ("take", "200")]).await {
                Ok(o) => o,
                Err(_) => continue,
            };
            let mut states: Vec<WorkflowState> = obj
                .get("Items")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| {
                            Some(WorkflowState {
                                id: s.get("Id")?.as_i64()?,
                                name: s.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                                is_final: s.get("IsFinal").and_then(|v| v.as_bool()).unwrap_or(false),
                                priority: s.get("NumericPriority").and_then(|v| v.as_f64()).unwrap_or(0.0),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            states.sort_by(|a, b| a.priority.partial_cmp(&b.priority).unwrap().then(a.name.cmp(&b.name)));
            out.insert(etype.to_string(), states);
        }
        Ok(out)
    }

    pub async fn set_entity_state(&self, entity_type: &str, entity_id: i64, state_id: i64) -> Result<(String, bool)> {
        let resp = self.post(&format!("{}/{}", entity_type, entity_id), json!({"EntityState": {"Id": state_id}})).await?;
        let es = resp.get("EntityState").cloned().unwrap_or(json!({}));
        Ok((
            es.get("Name").and_then(|v| v.as_str()).unwrap_or("?").to_string(),
            es.get("IsFinal").and_then(|v| v.as_bool()).unwrap_or(false),
        ))
    }

    pub async fn fetch_my_times(&self) -> Result<Vec<TimeEntry>> {
        let where_clause = format!("User.Id eq {}", self.my_user_id);
        let items = self.get_all("Times", &[("where", &where_clause), ("include", "[Id,Spent,Date,Description,Assignable[Id]]")]).await?;
        Ok(items
            .iter()
            .filter_map(|t| {
                let id = t.get("Id")?.as_i64()?;
                let item_id = t.get("Assignable").and_then(|a| a.get("Id")).and_then(|v| v.as_i64()).unwrap_or(0);
                let hours = t.get("Spent").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let day = tp_day(t.get("Date").and_then(|v| v.as_str()))?;
                Some(TimeEntry { id, item_id, hours, day, description: t.get("Description").and_then(|v| v.as_str()).unwrap_or("").to_string() })
            })
            .collect())
    }

    pub async fn log_time(&self, entity_id: i64, hours: f64, description: &str, date: chrono::NaiveDate, tz_off_min: i32) -> Result<i64> {
        let mut body = json!({
            "Spent": hours, "Remain": 0,
            "Date": date_string(date, tz_off_min),
            "Assignable": {"Id": entity_id}
        });
        let d = description.trim();
        if !d.is_empty() {
            body["Description"] = json!(d);
        }
        let resp = self.post("Times", body).await?;
        resp.get("Id").and_then(|v| v.as_i64()).ok_or_else(|| anyhow!("Time entry not created"))
    }

    pub async fn update_time(&self, time_id: i64, hours: Option<f64>, description: Option<&str>, date: Option<chrono::NaiveDate>, tz_off_min: i32) -> Result<()> {
        let mut body = serde_json::Map::new();
        if let Some(h) = hours { body.insert("Spent".into(), json!(h)); }
        if let Some(d) = description { body.insert("Description".into(), json!(d)); }
        if let Some(dt) = date { body.insert("Date".into(), json!(date_string(dt, tz_off_min))); }
        self.post(&format!("Times/{}", time_id), Value::Object(body)).await?;
        Ok(())
    }

    pub async fn delete_time(&self, time_id: i64) -> Result<()> {
        self.delete(&format!("Times/{}", time_id)).await
    }
}

/// "/Date(ms±HHMM)/" anchored to noon on the target day at the given offset.
pub fn date_string(date: chrono::NaiveDate, off_min: i32) -> String {
    use chrono::TimeZone;
    // noon at the given offset, expressed as a UTC instant
    let off = chrono::FixedOffset::east_opt(off_min * 60).unwrap();
    let noon_local = date.and_hms_opt(12, 0, 0).unwrap();
    let dt = off.from_local_datetime(&noon_local).unwrap();
    let ms = dt.timestamp_millis();
    let sign = if off_min >= 0 { "+" } else { "-" };
    let a = off_min.abs();
    format!("/Date({}{}{:02}{:02})/", ms, sign, a / 60, a % 60)
}

/// Parse "/Date(ms±HHMM)/" → "YYYY-MM-DD" using the embedded offset.
pub fn tp_day(s: Option<&str>) -> Option<String> {
    let s = s?;
    let ms_str: String = s.chars().skip_while(|c| !c.is_ascii_digit() && *c != '-').take_while(|c| c.is_ascii_digit() || *c == '-').collect();
    let ms: i64 = ms_str.parse().ok()?;
    let mut off_sec: i64 = 0;
    if let Some(pos) = s.find(['+', '-']).filter(|&p| p > 5) {
        let bytes = s.as_bytes();
        if pos + 5 <= s.len() {
            let sign = if bytes[pos] == b'-' { -1 } else { 1 };
            let h: i64 = s[pos + 1..pos + 3].parse().unwrap_or(0);
            let m: i64 = s[pos + 3..pos + 5].parse().unwrap_or(0);
            off_sec = sign * (h * 3600 + m * 60);
        }
    }
    let secs = ms / 1000 + off_sec;
    let dt = chrono::DateTime::from_timestamp(secs, 0)?;
    Some(dt.format("%Y-%m-%d").to_string())
}

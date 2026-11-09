use serde::{Deserialize, Serialize};

/// A Task, Bug or User Story assigned to me, plus its parent User Story
/// (a story is its own parent, so it groups with its tasks).
#[derive(Clone, Debug)]
pub struct WorkItem {
    pub id: i64,
    pub name: String,
    pub entity_type: String, // "Tasks" | "Bugs" | "UserStories" | "UserStories"
    pub display_type: String, // "Task" | "Bug" | "User Story"
    pub state_id: i64,
    pub state_name: String,
    pub is_final: bool,
    pub project_name: String,
    pub process_id: i64,
    pub sprint: String,
    pub us_id: i64,
    pub us_name: String,
}

impl WorkItem {
    pub fn is_story(&self) -> bool {
        self.entity_type == "UserStories"
    }
    pub fn is_bug(&self) -> bool {
        self.entity_type == "Bugs"
    }
    /// Key into the per-process workflow-state map (also the TP entity name).
    pub fn state_key(&self) -> &'static str {
        if self.is_story() { "UserStory" } else if self.is_bug() { "Bug" } else { "Task" }
    }
    pub fn display_type_for(collection: &str) -> &'static str {
        match collection {
            "Bugs" => "Bug",
            "UserStories" => "User Story",
            _ => "Task",
        }
    }
}

/// One of my TP time entries.
#[derive(Clone, Debug)]
pub struct TimeEntry {
    pub id: i64,
    pub item_id: i64,
    pub hours: f64,
    pub day: String, // "YYYY-MM-DD" (offset-aware)
    pub description: String,
}

#[derive(Clone, Debug)]
pub struct WorkflowState {
    pub id: i64,
    pub name: String,
    pub is_final: bool,
    pub priority: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DynamicMeeting {
    pub id: String,
    pub name: String,
    #[serde(rename = "taskId")]
    pub task_id: i64,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecurringEntry {
    pub id: String,
    pub label: String,
    #[serde(rename = "taskId")]
    pub task_id: i64,
    pub hours: f64,
}

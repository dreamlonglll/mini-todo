use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub priority: String,
    pub notify_at: Option<String>,
    pub notify_before: i32,
    pub notified: bool,
    pub completed: bool,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub subtasks: Vec<SubTask>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubTask {
    pub id: i64,
    pub parent_id: i64,
    pub title: String,
    pub completed: bool,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTodoRequest {
    pub title: String,
    pub description: Option<String>,
    pub priority: String,
    pub notify_at: Option<String>,
    pub notify_before: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTodoRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub notify_at: Option<String>,
    pub notify_before: Option<i32>,
    pub completed: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubTaskRequest {
    pub parent_id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSubTaskRequest {
    pub title: Option<String>,
    pub completed: Option<bool>,
    pub sort_order: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowPosition {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub is_fixed: bool,
    pub window_position: Option<WindowPosition>,
    pub window_size: Option<WindowSize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportData {
    pub version: String,
    pub exported_at: String,
    pub todos: Vec<Todo>,
    pub settings: AppSettings,
}

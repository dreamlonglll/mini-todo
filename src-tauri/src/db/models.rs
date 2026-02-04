use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    /// 颜色（HEX 格式，如 #EF4444）
    pub color: String,
    /// 四象限：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要
    pub quadrant: i32,
    pub notify_at: Option<String>,
    pub notify_before: i32,
    pub notified: bool,
    pub completed: bool,
    pub sort_order: i32,
    /// 开始时间（可为空，空则使用 created_at）
    pub start_time: Option<String>,
    /// 截止时间（可为空）
    pub end_time: Option<String>,
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
    /// 颜色（HEX 格式，如 #EF4444）
    pub color: String,
    /// 四象限：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要
    #[serde(default = "default_quadrant")]
    pub quadrant: i32,
    pub notify_at: Option<String>,
    pub notify_before: Option<i32>,
    /// 开始时间（可为空）
    pub start_time: Option<String>,
    /// 截止时间（可为空）
    pub end_time: Option<String>,
}

fn default_quadrant() -> i32 {
    4
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateTodoRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    /// 颜色（HEX 格式，如 #EF4444）
    pub color: Option<String>,
    /// 四象限：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要
    pub quadrant: Option<i32>,
    pub notify_at: Option<String>,
    pub notify_before: Option<i32>,
    pub completed: Option<bool>,
    pub sort_order: Option<i32>,
    /// 是否明确清除通知时间
    #[serde(default)]
    pub clear_notify_at: bool,
    /// 开始时间
    pub start_time: Option<String>,
    /// 截止时间
    pub end_time: Option<String>,
    /// 是否明确清除开始时间
    #[serde(default)]
    pub clear_start_time: bool,
    /// 是否明确清除截止时间
    #[serde(default)]
    pub clear_end_time: bool,
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
    /// 文本主题：light（浅色文字，适配深色背景）或 dark（深色文字，适配浅色背景）
    #[serde(default = "default_text_theme")]
    pub text_theme: String,
}

fn default_text_theme() -> String {
    "dark".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportData {
    pub version: String,
    pub exported_at: String,
    pub todos: Vec<Todo>,
    pub settings: AppSettings,
}

/// 屏幕配置记录，用于存储不同屏幕组合下的窗口状态
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenConfig {
    pub id: i64,
    /// 屏幕配置唯一标识（如 "2_2560x1440@125_1920x1080@100"）
    pub config_id: String,
    /// 显示名称（用户可编辑）
    pub display_name: Option<String>,
    /// 窗口 X 坐标
    pub window_x: i32,
    /// 窗口 Y 坐标
    pub window_y: i32,
    /// 窗口宽度
    pub window_width: i32,
    /// 窗口高度
    pub window_height: i32,
    /// 是否固定模式
    pub is_fixed: bool,
    /// 创建时间
    pub created_at: String,
    /// 更新时间
    pub updated_at: String,
}

/// 保存/更新屏幕配置的请求
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveScreenConfigRequest {
    pub config_id: String,
    pub display_name: Option<String>,
    pub window_x: i32,
    pub window_y: i32,
    pub window_width: i32,
    pub window_height: i32,
    pub is_fixed: bool,
}

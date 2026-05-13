use rusqlite::Row;
use serde::{Deserialize, Serialize};

pub const SUBTASK_COLUMNS: &str =
    "id, parent_id, title, content, completed, sort_order, created_at, updated_at";

pub const TODO_COLUMNS: &str = "id, title, description, color, quadrant, notify_at, notify_before,
     notified, completed, sort_order, start_time, end_time, created_at, updated_at,
     repeat_enabled, repeat_type, repeat_interval, repeat_weekdays, repeat_month_day";

pub fn subtask_from_row(row: &Row) -> rusqlite::Result<SubTask> {
    Ok(SubTask {
        id: row.get(0)?,
        parent_id: row.get(1)?,
        title: row.get(2)?,
        content: row.get(3)?,
        completed: row.get::<_, i32>(4)? != 0,
        sort_order: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

pub fn todo_from_row(row: &Row) -> rusqlite::Result<Todo> {
    Ok(Todo {
        id: row.get(0)?,
        title: row.get(1)?,
        description: row.get(2)?,
        color: row.get(3)?,
        quadrant: row.get(4)?,
        notify_at: row.get(5)?,
        notify_before: row.get(6)?,
        notified: row.get::<_, i32>(7)? != 0,
        completed: row.get::<_, i32>(8)? != 0,
        sort_order: row.get(9)?,
        start_time: row.get(10)?,
        end_time: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
        repeat_enabled: row.get::<_, i32>(14).unwrap_or(0) != 0,
        repeat_type: row.get(15).unwrap_or(None),
        repeat_interval: row.get(16).unwrap_or(1),
        repeat_weekdays: row.get(17).unwrap_or(None),
        repeat_month_day: row.get(18).unwrap_or(None),
        subtasks: Vec::new(),
    })
}

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
    pub repeat_enabled: bool,
    #[serde(default)]
    pub repeat_type: Option<String>,
    #[serde(default = "default_repeat_interval")]
    pub repeat_interval: i32,
    #[serde(default)]
    pub repeat_weekdays: Option<String>,
    #[serde(default)]
    pub repeat_month_day: Option<i32>,
    #[serde(default)]
    pub subtasks: Vec<SubTask>,
}

fn default_repeat_interval() -> i32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubTask {
    pub id: i64,
    pub parent_id: i64,
    pub title: String,
    pub content: Option<String>,
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
    /// 是否启用重复提醒
    pub repeat_enabled: Option<bool>,
    /// 重复类型：daily / weekly / monthly
    pub repeat_type: Option<String>,
    /// 重复间隔
    pub repeat_interval: Option<i32>,
    /// 周重复的星期几（逗号分隔，如 "1,3,5"）
    pub repeat_weekdays: Option<String>,
    /// 月重复的日期（1~31）
    pub repeat_month_day: Option<i32>,
    /// 是否明确清除重复提醒
    #[serde(default)]
    pub clear_repeat: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSubTaskRequest {
    pub parent_id: i64,
    pub title: String,
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSubTaskRequest {
    pub title: Option<String>,
    pub content: Option<String>,
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
    /// 是否启用贴边自动隐藏
    #[serde(default = "default_auto_hide_enabled")]
    pub auto_hide_enabled: bool,
    /// 文本主题：light（浅色文字，适配深色背景）或 dark（深色文字，适配浅色背景）
    #[serde(default = "default_text_theme")]
    pub text_theme: String,
    /// 是否显示日历面板
    #[serde(default)]
    pub show_calendar: bool,
    /// 视图模式：list 或 quadrant
    #[serde(default = "default_view_mode")]
    pub view_mode: String,
    /// 通知类型：system 或 app
    #[serde(default = "default_notification_type")]
    pub notification_type: String,
}

fn default_text_theme() -> String {
    "dark".to_string()
}

fn default_auto_hide_enabled() -> bool {
    true
}

fn default_view_mode() -> String {
    "list".to_string()
}

fn default_notification_type() -> String {
    "system".to_string()
}

/// 数据导出格式。
/// v4.0 起不再包含 agent_configs / workflow_steps / task_dependencies / prompt_templates /
/// agent_executions 等 AI Agent 相关字段；反序列化保持向后兼容，旧 v3.0 备份中的这些字段
/// 在解析时通过 `#[serde(default)]` 静默跳过。
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

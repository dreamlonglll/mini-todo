use rusqlite::Row;
use serde::{Deserialize, Serialize};

pub const SUBTASK_COLUMNS: &str =
    "id, parent_id, title, content, completed, sort_order, created_at, updated_at,
     schedule_status, priority_score, max_retries, retry_count, timeout_secs,
     scheduled_at, last_scheduled_run, schedule_error";

pub const TODO_COLUMNS: &str =
    "id, title, description, color, quadrant, notify_at, notify_before,
     notified, completed, sort_order, start_time, end_time, created_at, updated_at,
     agent_id, agent_project_path, schedule_strategy, cron_expression, schedule_enabled,
     last_scheduled_run, post_action, workflow_enabled, workflow_current_step";

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
        schedule_status: row.get::<_, String>(8).unwrap_or_else(|_| "none".to_string()),
        priority_score: row.get(9).unwrap_or(0),
        max_retries: row.get(10).unwrap_or(0),
        retry_count: row.get(11).unwrap_or(0),
        timeout_secs: row.get(12).unwrap_or(600),
        scheduled_at: row.get(13).unwrap_or(None),
        last_scheduled_run: row.get(14).unwrap_or(None),
        schedule_error: row.get(15).unwrap_or(None),
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
        agent_id: row.get(14)?,
        agent_project_path: row.get(15)?,
        schedule_strategy: row.get::<_, String>(16).unwrap_or_else(|_| "manual".to_string()),
        cron_expression: row.get(17).unwrap_or(None),
        schedule_enabled: row.get::<_, i32>(18).unwrap_or(0) != 0,
        last_scheduled_run: row.get(19).unwrap_or(None),
        post_action: row.get::<_, String>(20).unwrap_or_else(|_| "none".to_string()),
        workflow_enabled: row.get::<_, i32>(21).unwrap_or(0) != 0,
        workflow_current_step: row.get::<_, i32>(22).unwrap_or(-1),
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
    /// 绑定的 Agent 配置 ID（可为空）
    pub agent_id: Option<i64>,
    /// Agent 工作的项目目录（可为空）
    pub agent_project_path: Option<String>,
    #[serde(default = "default_schedule_strategy")]
    pub schedule_strategy: String,
    #[serde(default)]
    pub cron_expression: Option<String>,
    #[serde(default)]
    pub schedule_enabled: bool,
    #[serde(default)]
    pub last_scheduled_run: Option<String>,
    #[serde(default = "default_post_action")]
    pub post_action: String,
    #[serde(default)]
    pub workflow_enabled: bool,
    #[serde(default = "default_workflow_step")]
    pub workflow_current_step: i32,
    #[serde(default)]
    pub subtasks: Vec<SubTask>,
}

fn default_post_action() -> String {
    "none".to_string()
}

fn default_workflow_step() -> i32 {
    -1
}

fn default_schedule_strategy() -> String {
    "manual".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStep {
    pub id: i64,
    pub todo_id: i64,
    pub step_order: i32,
    pub step_type: String,
    pub subtask_id: Option<i64>,
    pub prompt_text: Option<String>,
    pub status: String,
    pub carry_context: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStepInput {
    pub step_type: String,
    pub subtask_id: Option<i64>,
    pub prompt_text: Option<String>,
    #[serde(default)]
    pub carry_context: bool,
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
    #[serde(default = "default_schedule_status")]
    pub schedule_status: String,
    #[serde(default)]
    pub priority_score: i64,
    #[serde(default)]
    pub max_retries: i64,
    #[serde(default)]
    pub retry_count: i64,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: i64,
    #[serde(default)]
    pub scheduled_at: Option<String>,
    #[serde(default)]
    pub last_scheduled_run: Option<String>,
    #[serde(default)]
    pub schedule_error: Option<String>,
}

fn default_schedule_status() -> String {
    "none".to_string()
}

fn default_timeout_secs() -> i64 {
    600
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
    /// 绑定的 Agent 配置 ID
    pub agent_id: Option<i64>,
    /// Agent 工作的项目目录
    pub agent_project_path: Option<String>,
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
    /// 绑定的 Agent 配置 ID
    pub agent_id: Option<i64>,
    /// Agent 工作的项目目录
    pub agent_project_path: Option<String>,
    /// 是否明确清除 Agent 绑定
    #[serde(default)]
    pub clear_agent: bool,
    /// 子任务完成后的工作流动作
    pub post_action: Option<String>,
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

// ========== 任务调度相关模型 ==========

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDependency {
    pub id: i64,
    pub subtask_id: i64,
    pub depends_on_id: i64,
    pub dependency_type: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    pub id: String,
    pub name: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub template_content: String,
    pub variables: String,
    pub recommended_agent: Option<String>,
    pub is_builtin: bool,
    pub created_at: String,
    pub updated_at: String,
}

// ========== Agent 集成相关模型 ==========

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfig {
    pub id: i64,
    pub name: String,
    pub agent_type: String,
    pub cli_path: String,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateAgentRequest {
    pub name: String,
    pub agent_type: String,
    pub cli_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    pub agent_type: Option<String>,
    pub cli_path: Option<String>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHealthStatus {
    pub agent_id: i64,
    pub status: String,
    pub cli_found: bool,
    pub detected_version: Option<String>,
    pub version_compatible: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentExecution {
    pub id: i64,
    pub task_id: String,
    pub subtask_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub agent_type: String,
    pub status: String,
    pub logs: String,
    pub result_text: String,
    pub error: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub start_time_ms: i64,
    pub duration_ms: i64,
    pub created_at: String,
    pub session_id: Option<String>,
}

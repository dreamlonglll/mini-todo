use rusqlite::Row;
use serde::{Deserialize, Deserializer, Serialize};

pub const SUBTASK_COLUMNS: &str =
    "id, parent_id, title, content, completed, sort_order, created_at, updated_at";

pub const TODO_COLUMNS: &str = "id, title, description, color, quadrant, notify_at, notify_before,
     notified, completed, sort_order, start_time, end_time, created_at, updated_at,
     repeat_enabled, repeat_type, repeat_interval, repeat_weekdays, repeat_month_day, list_id";

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
        // v1 建表时这两列可空；NULL 按 0 处理，避免整行读取失败
        notify_before: row.get::<_, Option<i32>>(6)?.unwrap_or(0),
        notified: row.get::<_, Option<i32>>(7)?.unwrap_or(0) != 0,
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
        list_id: row.get(19)?,
        subtasks: Vec::new(),
    })
}

/// 默认清单的 id（v31 迁移建；可改名、不可删除）。`listId` 缺失或指向不存在（含已删除）的清单的待办都按它显示
pub const DEFAULT_LIST_ID: i64 = 1;

/// 默认清单的初始名称（= 3.0 主窗口标题栏写死的文字）
pub const DEFAULT_LIST_NAME: &str = "待办清单";

/// 默认清单的初始 `createdAt` / `updatedAt`（PC v31 迁移与 cloud 惰性补建用同一个值）：固定的早时间，
/// 两台设备各自建出的 id 1 不会互相"抢"，任何一次改名都在 LWW 里胜出
pub const DEFAULT_LIST_CREATED_AT: &str = "2000-01-01 00:00:00";

fn default_list_id() -> i64 {
    DEFAULT_LIST_ID
}

/// `listId`：缺失（`#[serde(default)]`）或为 `null` 都是默认清单
fn de_list_id<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    Ok(Option::<i64>::deserialize(d)?.unwrap_or(DEFAULT_LIST_ID))
}

/// todo_lists 查询列，与 [`todo_list_from_row`] 的下标一一对应
pub const TODO_LIST_COLUMNS: &str = "id, name, sort_order, created_at, updated_at";

pub fn todo_list_from_row(row: &Row) -> rusqlite::Result<TodoList> {
    Ok(TodoList {
        id: row.get(0)?,
        name: row.get(1)?,
        sort_order: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

/// 待办清单（v31 `todo_lists`）：待办的分区，每个清单在桌面上是一个独立窗口。
///
/// 参与同步（同步文档顶层 `todoLists`，墓碑在 `listTombstones`）与导出（格式 4.1 起）。反序列化宽容规则同 [`Todo`]：
/// `id` / `name` 必填；`updatedAt` 缺失时为空串，同步合并按"无法识别"跳过。PC 是强类型的，清单记录里的未知字段丢弃
/// （cloud 原样保留）。`sortOrder` 用 i64：cloud 可经 `PATCH` 改它，读侧不因取值范围丢清单
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoList {
    pub id: i64,
    pub name: String,
    /// 显示顺序：列表一律按 `(sortOrder, id)` 升序；新清单排最后（现有最大值 + 1）
    #[serde(default)]
    pub sort_order: i64,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// 待办。
///
/// 反序列化（导入备份、合并 WebDAV 远端）对非关键字段宽容：缺失时取与建表默认值一致的值。
/// `id` / `title` 必填；`updatedAt` 缺失时为空串，同步合并会把它当作无法识别的记录跳过
/// （LWW 需要它），手动导入则会统一改写为导入时刻。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    /// 颜色（HEX 格式，如 #EF4444）
    #[serde(default = "default_color")]
    pub color: String,
    /// 四象限：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要
    #[serde(default = "default_quadrant")]
    pub quadrant: i32,
    pub notify_at: Option<String>,
    #[serde(default)]
    pub notify_before: i32,
    #[serde(default)]
    pub notified: bool,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub sort_order: i32,
    /// 开始时间（可为空，空则使用 created_at）
    pub start_time: Option<String>,
    /// 截止时间（可为空）
    pub end_time: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
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
    /// 所属清单（v31）。缺失或为 `null` 时是默认清单（旧版本 / 旧备份的记录）；指向不存在（含已删除）的清单时
    /// 界面按默认清单显示（读侧兜底 `lists::effective_list_id`），存储与同步的仍是原值，不因清单不存在而改写
    #[serde(default = "default_list_id", deserialize_with = "de_list_id")]
    pub list_id: i64,
    #[serde(default)]
    pub subtasks: Vec<SubTask>,
}

fn default_repeat_interval() -> i32 {
    1
}

/// 与 v4 迁移 `color` 列的默认值一致
fn default_color() -> String {
    "#F59E0B".to_string()
}

/// 子任务。反序列化宽容规则同 [`Todo`]；`parentId` 在导入 / 合并时以外层待办的 id 为准。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubTask {
    pub id: i64,
    #[serde(default)]
    pub parent_id: i64,
    pub title: String,
    pub content: Option<String>,
    #[serde(default)]
    pub completed: bool,
    #[serde(default)]
    pub sort_order: i32,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// 删除墓碑（K2）：同步时用来阻止已删除的记录被另一端"复活"。
///
/// `entityType` 为 `"todo"` 或 `"subtask"`；`deletedAt` 为 K1 规范时间。
/// 规则：存在 `deletedAt >= record.updatedAt` 的墓碑 → 记录被删除 / 压制；
/// 删除后又被编辑（`updatedAt > deletedAt`）的记录保留。保留期 30 天。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tombstone {
    pub entity_type: String,
    pub entity_id: i64,
    pub deleted_at: String,
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
    /// 所属清单（v31）；`None` = 默认清单。必须是现有的清单，否则报"清单不存在"
    #[serde(default)]
    pub list_id: Option<i64>,
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
    /// 移到另一个清单（v31）；必须是现有的清单，否则报"清单不存在"
    #[serde(default)]
    pub list_id: Option<i64>,
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
    /// 固定模式时是否嵌入桌面（窗口嵌在桌面图标之上、所有应用窗口之下，Win+D 不最小化），
    /// 仅 Windows 生效。v27 引入，`default` 保证旧备份 / 旧版远端数据反序列化不报错
    #[serde(default)]
    pub fixed_embed_desktop: bool,
    pub window_position: Option<WindowPosition>,
    pub window_size: Option<WindowSize>,
    /// 是否启用贴边自动隐藏
    #[serde(default = "default_auto_hide_enabled")]
    pub auto_hide_enabled: bool,
    /// 贴边唤起时是否临时置顶（关闭后窗口会被全屏窗口遮挡）
    #[serde(default = "default_top_on_wake")]
    pub top_on_wake: bool,
    /// 窗口底色（HEX），仅深色主题下生效
    #[serde(default = "default_window_bg_color")]
    pub window_bg_color: String,
    /// 窗口背景透明度 0~1，仅深色主题下生效
    #[serde(default = "default_window_bg_alpha")]
    pub window_bg_alpha: f64,
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

fn default_top_on_wake() -> bool {
    true
}

/// 窗口底色默认值
pub const DEFAULT_WINDOW_BG_COLOR: &str = "#000000";

/// 窗口背景透明度默认值
///
/// 抽成可配置之前，深色主题是 html/body/#app/.app-container/.main-content 多层
/// 半透明黑叠加，合成不透明度约 0.45；重构成单层后要用这个值才能维持原有观感。
pub const DEFAULT_WINDOW_BG_ALPHA: f64 = 0.45;

fn default_window_bg_color() -> String {
    DEFAULT_WINDOW_BG_COLOR.to_string()
}

fn default_window_bg_alpha() -> f64 {
    DEFAULT_WINDOW_BG_ALPHA
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

/// fences 查询列，与 [`fence_from_row`] 的下标一一对应
pub const FENCE_COLUMNS: &str = "id, title, folder_name, x, y, width, height, collapsed, locked, \
     bg_color, bg_alpha, icon_size, sort_order, created_at, updated_at, show_hidden";

pub fn fence_from_row(row: &Row) -> rusqlite::Result<Fence> {
    Ok(Fence {
        id: row.get(0)?,
        title: row.get(1)?,
        folder_name: row.get(2)?,
        x: row.get(3)?,
        y: row.get(4)?,
        width: row.get(5)?,
        height: row.get(6)?,
        collapsed: row.get::<_, i32>(7)? != 0,
        locked: row.get::<_, i32>(8)? != 0,
        bg_color: row.get(9)?,
        bg_alpha: row.get(10)?,
        icon_size: row.get(11)?,
        sort_order: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
        show_hidden: row.get::<_, i32>(15)? != 0,
    })
}

/// 收纳箱背景透明度默认值（与 v29 迁移 `bg_alpha` 列默认值一致；背景色默认 `#000000`、图标大小默认 48）
pub const DEFAULT_FENCE_BG_ALPHA: f64 = 0.45;

/// 桌面收纳箱（v29 `fences`）。
///
/// 设备相关数据，不进 `ExportData` / WebDAV 同步。几何信息是物理像素：`x` / `y` 为虚拟屏幕坐标
/// （折叠态时即标题栏位置），`width` / `height` 为展开态尺寸，只在用户拖动 / 缩放结束后落库。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Fence {
    pub id: i64,
    pub title: String,
    /// 收纳根目录下的子文件夹名
    pub folder_name: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub collapsed: bool,
    /// 锁定（「固定」）：不可移动 / 缩放，仍可拖入、折叠、改名
    pub locked: bool,
    pub bg_color: String,
    pub bg_alpha: f64,
    /// 图标大小（逻辑像素）
    pub icon_size: i32,
    pub sort_order: i32,
    pub created_at: String,
    pub updated_at: String,
    /// 「显示隐藏的图标」（v30）：开启时隐藏项（用户隐藏 + 以 `.` 开头的自动隐藏）半透明显示，关闭时不显示
    #[serde(default)]
    pub show_hidden: bool,
}

/// 收纳箱项目的附加信息（v29 `fence_items`）：只有排序、原始路径与隐藏标记，内容以收纳文件夹为准。
///
/// 每次列举都按收纳文件夹对账（`fence::items::reconcile_rows`）：文件夹有、表里没有 → 补行；
/// 表里有、文件夹没有 → 删行。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FenceItem {
    pub id: i64,
    pub fence_id: i64,
    /// 收纳文件夹内的实际文件名
    pub file_name: String,
    /// 移入前的完整路径；在 Explorer 里直接放进收纳文件夹的为 `None`
    pub original_path: Option<String>,
    pub sort_order: i32,
    /// 用户右键「隐藏」（v30）。只影响收纳箱的显示，不改文件属性；跨箱移动不携带
    #[serde(default)]
    pub hidden: bool,
}

/// `update_fence` 的入参：只更新给出的字段（标题走 `rename_fence`，几何走 `commit_fence_geometry`）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateFenceRequest {
    pub collapsed: Option<bool>,
    pub locked: Option<bool>,
    pub bg_color: Option<String>,
    pub bg_alpha: Option<f64>,
    pub icon_size: Option<i32>,
    /// 「显示隐藏的图标」（v30）
    pub show_hidden: Option<bool>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn todo_with(extra: serde_json::Value) -> serde_json::Result<Todo> {
        let mut v = json!({"id": 5, "title": "t", "description": null, "notifyAt": null,
                           "startTime": null, "endTime": null});
        v.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(v)
    }

    /// 旧版本 / 旧备份的待办没有 `listId`：落进默认清单；`null` 同缺失；显式值原样保留（不校验清单是否存在）
    #[test]
    fn todo_list_id_defaults_to_the_default_list() {
        assert_eq!(todo_with(json!({})).unwrap().list_id, DEFAULT_LIST_ID);
        assert_eq!(
            todo_with(json!({"listId": null})).unwrap().list_id,
            DEFAULT_LIST_ID
        );
        assert_eq!(
            todo_with(json!({"listId": 1_731_000_000_000_123_i64}))
                .unwrap()
                .list_id,
            1_731_000_000_000_123
        );
        assert_eq!(todo_with(json!({"listId": 999})).unwrap().list_id, 999);
        assert!(
            todo_with(json!({"listId": "工作"})).is_err(),
            "类型错误与其它字段一样按无法识别处理"
        );

        let out = serde_json::to_value(todo_with(json!({"listId": 7})).unwrap()).unwrap();
        assert_eq!(out["listId"], 7, "总是写出 listId");
    }

    /// 清单记录：驼峰字段；`id` / `name` 必填，其余缺失取默认；未知字段丢弃
    #[test]
    fn todo_list_serde_uses_the_contract_shape() {
        let list: TodoList = serde_json::from_value(json!({
            "id": 1_731_000_000_000_123_i64, "name": "工作", "sortOrder": 3,
            "createdAt": "2026-11-01 08:00:00", "updatedAt": "2026-11-02 08:00:00",
            "color": "#123456"
        }))
        .unwrap();
        assert_eq!(
            list,
            TodoList {
                id: 1_731_000_000_000_123,
                name: "工作".to_string(),
                sort_order: 3,
                created_at: "2026-11-01 08:00:00".to_string(),
                updated_at: "2026-11-02 08:00:00".to_string(),
            }
        );
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            json!({"id": 1_731_000_000_000_123_i64, "name": "工作", "sortOrder": 3,
                   "createdAt": "2026-11-01 08:00:00", "updatedAt": "2026-11-02 08:00:00"})
        );
        let minimal: TodoList = serde_json::from_value(json!({"id": 2, "name": "x"})).unwrap();
        assert_eq!((minimal.sort_order, minimal.updated_at.as_str()), (0, ""));
        assert!(serde_json::from_value::<TodoList>(json!({"id": 2})).is_err());
        assert!(serde_json::from_value::<TodoList>(json!({"name": "x"})).is_err());
    }
}

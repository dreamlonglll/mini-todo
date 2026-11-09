//! 手动导入导出（ZIP / JSON，格式 4.1，兼容 3.0 / 4.0）与应用设置块的读写（原 `commands/data.rs`）。
//!
//! 导入是"恢复备份"语义：导入前 `VACUUM INTO` 备份、单事务整体替换、保留原 id、被移除的记录写墓碑、
//! 导入的记录 `updated_at` 刷新为导入时刻。导入改写了 `top_on_wake` / `auto_hide_enabled` 等运行时偏好，
//! **调用方**在导入成功后自行刷新窗口侧的缓存（core 不持有窗口状态）。
//!
//! 格式 4.1（v31 多待办清单）：顶层 `todoLists`、每条待办带 `listId`；清单墓碑与窗口属性不导出。导入 4.1 时清单同样按
//! "恢复备份"语义处理（默认清单始终保留、只取备份里的名字）；导入 3.0 / 4.0（没有 `todoLists`）时现有清单不动，
//! 备份里的待办没有 `listId`，全部进默认清单。

use crate::db::backup;
pub use crate::db::settings_kv::settings_version;
use crate::db::settings_kv::{
    bool_str, get_bool_setting, get_setting, get_setting_or, set_bool_setting, set_setting,
    set_setting_at, SYNCED_SETTING_KEYS,
};
use crate::db::sync_store::{
    self, insert_subtask_row, insert_todo_row, normalize_list_lenient, normalize_todo_lenient,
    EntityKind,
};
use crate::db::time::{now_local, superseding};
use crate::db::{
    AppSettings, Database, ExportData, TodoList, WindowPosition, WindowSize,
    DEFAULT_LIST_CREATED_AT, DEFAULT_LIST_ID, DEFAULT_WINDOW_BG_ALPHA, DEFAULT_WINDOW_BG_COLOR,
};
use crate::window_prefs;
use chrono::Local;
use rusqlite::params;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Write as _};
use std::path::Path;

/// 导出 / 同步文档的格式版本（4.1：多待办清单）
pub const EXPORT_VERSION: &str = "4.1";

fn read_json_setting<T: serde::de::DeserializeOwned>(
    conn: &rusqlite::Connection,
    key: &str,
) -> Option<T> {
    get_setting(conn, key)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
}

/// 读取应用设置块（导出 / 同步上传的 `settings`）；缺键取默认值
pub fn read_app_settings(conn: &rusqlite::Connection) -> AppSettings {
    let window_position: Option<WindowPosition> = read_json_setting(conn, "window_position");
    let window_size: Option<WindowSize> = read_json_setting(conn, "window_size");
    let window_bg_alpha = get_setting_or(conn, "window_bg_alpha", "")
        .parse::<f64>()
        .unwrap_or(DEFAULT_WINDOW_BG_ALPHA);

    AppSettings {
        is_fixed: get_bool_setting(conn, "is_fixed", false),
        fixed_embed_desktop: get_bool_setting(conn, "fixed_embed_desktop", false),
        window_position,
        window_size,
        auto_hide_enabled: get_bool_setting(conn, "auto_hide_enabled", true),
        top_on_wake: get_bool_setting(conn, "top_on_wake", true),
        window_bg_color: get_setting_or(conn, "window_bg_color", DEFAULT_WINDOW_BG_COLOR),
        window_bg_alpha,
        text_theme: get_setting_or(conn, "text_theme", "dark"),
        show_calendar: get_bool_setting(conn, "show_calendar", false),
        view_mode: get_setting_or(conn, "view_mode", "list"),
        notification_type: get_setting_or(conn, "notification_type", "system"),
    }
}

/// 写入全部应用设置（手动导入用，含窗口位置 / 尺寸）。只有值变化的键会刷新 updated_at。
/// 事务由调用方负责。
pub fn write_app_settings(
    conn: &rusqlite::Connection,
    settings: &AppSettings,
) -> rusqlite::Result<()> {
    set_bool_setting(conn, "is_fixed", settings.is_fixed)?;
    set_bool_setting(conn, "fixed_embed_desktop", settings.fixed_embed_desktop)?;
    if let Some(pos) = &settings.window_position {
        let pos_json = serde_json::to_string(pos).unwrap_or_default();
        set_setting(conn, "window_position", &pos_json)?;
    }
    if let Some(size) = &settings.window_size {
        let size_json = serde_json::to_string(size).unwrap_or_default();
        set_setting(conn, "window_size", &size_json)?;
    }
    set_bool_setting(conn, "auto_hide_enabled", settings.auto_hide_enabled)?;
    set_bool_setting(conn, "top_on_wake", settings.top_on_wake)?;
    set_setting(conn, "window_bg_color", &settings.window_bg_color)?;
    set_setting(
        conn,
        "window_bg_alpha",
        &settings.window_bg_alpha.to_string(),
    )?;
    set_setting(conn, "text_theme", &settings.text_theme)?;
    set_bool_setting(conn, "show_calendar", settings.show_calendar)?;
    set_setting(conn, "view_mode", &settings.view_mode)?;
    set_setting(conn, "notification_type", &settings.notification_type)?;
    Ok(())
}

/// 远端 settings JSON 字段 → 本地键 + 取值校验
enum SettingKind {
    Bool,
    HexColor,
    Alpha,
    OneOf(&'static [&'static str]),
}

const REMOTE_SETTING_FIELDS: [(&str, &str, SettingKind); 10] = [
    ("isFixed", "is_fixed", SettingKind::Bool),
    (
        "fixedEmbedDesktop",
        "fixed_embed_desktop",
        SettingKind::Bool,
    ),
    ("autoHideEnabled", "auto_hide_enabled", SettingKind::Bool),
    ("topOnWake", "top_on_wake", SettingKind::Bool),
    ("windowBgColor", "window_bg_color", SettingKind::HexColor),
    ("windowBgAlpha", "window_bg_alpha", SettingKind::Alpha),
    (
        "textTheme",
        "text_theme",
        SettingKind::OneOf(&["light", "dark"]),
    ),
    ("showCalendar", "show_calendar", SettingKind::Bool),
    (
        "viewMode",
        "view_mode",
        SettingKind::OneOf(&["list", "quadrant"]),
    ),
    (
        "notificationType",
        "notification_type",
        SettingKind::OneOf(&["system", "app"]),
    ),
];

fn is_hex_color(s: &str) -> bool {
    let Some(hex) = s.strip_prefix('#') else {
        return false;
    };
    matches!(hex.len(), 3 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

impl SettingKind {
    fn to_db(&self, v: &Value) -> Option<String> {
        match self {
            Self::Bool => v.as_bool().map(|b| bool_str(b).to_string()),
            Self::HexColor => v.as_str().filter(|s| is_hex_color(s)).map(str::to_string),
            Self::Alpha => v
                .as_f64()
                .filter(|a| (0.0..=1.0).contains(a))
                .map(|a| a.to_string()),
            Self::OneOf(allowed) => v
                .as_str()
                .filter(|s| allowed.contains(s))
                .map(str::to_string),
        }
    }
}

/// 应用远端 settings（K3-5）：只处理参与同步的键，且只写远端对象里**存在且合法**的字段
/// （缺失的字段不回落默认值，避免占位对象把本地设置重置）；`windowPosition` / `windowSize`
/// 永不应用。变化的键 `updated_at` 记为 `updated_at`（远端 settingsUpdatedAt）。
/// 返回是否有设置真的发生变化。事务由调用方负责。
pub(crate) fn apply_settings_json(
    conn: &rusqlite::Connection,
    remote: &Map<String, Value>,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let mut changed = false;
    for (json_key, db_key, kind) in &REMOTE_SETTING_FIELDS {
        let Some(value) = remote.get(*json_key) else {
            continue;
        };
        match kind.to_db(value) {
            Some(v) => changed |= set_setting_at(conn, db_key, &v, updated_at)?,
            None => log::warn!("[sync] 忽略远端非法设置 {}: {}", json_key, value),
        }
    }
    Ok(changed)
}

/// 采用远端设置（K3-5 远端较新 / 本机首次同步 / 强制拉取）：应用远端字段后，把全部同步键的
/// `updated_at` 对齐到远端的 `settingsUpdatedAt`——本地设置此刻就是远端那个版本。
/// 否则没被远端覆盖的键还带着更新的本地时间戳，下次同步会把同样的设置再上传一遍。
/// 返回是否有设置值真的发生变化。事务由调用方负责。
pub(crate) fn adopt_settings_json(
    conn: &rusqlite::Connection,
    remote: &Map<String, Value>,
    updated_at: &str,
) -> rusqlite::Result<bool> {
    let changed = apply_settings_json(conn, remote, updated_at)?;
    let placeholders = vec!["?"; SYNCED_SETTING_KEYS.len()].join(", ");
    conn.execute(
        &format!(
            "UPDATE settings SET updated_at = ? WHERE key IN ({}) AND updated_at IS NOT ?",
            placeholders
        ),
        rusqlite::params_from_iter(
            std::iter::once(updated_at)
                .chain(SYNCED_SETTING_KEYS.iter().copied())
                .chain(std::iter::once(updated_at)),
        ),
    )?;
    Ok(changed)
}

/// 导出全部清单、待办（含子任务，时间规范化）与应用设置为 pretty JSON（`export_data`，格式 4.1）
pub fn export_json(db: &Database) -> Result<String, String> {
    let (lists, todos, settings) = db
        .with_connection(|conn| {
            let mut lists = sync_store::load_lists(conn)?;
            for list in &mut lists {
                normalize_list_lenient(list);
            }
            let mut todos = sync_store::load_todos_with_subtasks(conn, "sort_order ASC, id ASC")?;
            for todo in &mut todos {
                normalize_todo_lenient(todo);
            }
            Ok((lists, todos, read_app_settings(conn)))
        })
        .map_err(|e| e.to_string())?;

    let export_data = ExportData {
        version: EXPORT_VERSION.to_string(),
        exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        todo_lists: Some(lists),
        todos,
        settings,
    };
    serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())
}

/// 导入备份数据（手动导入，语义为"整体替换"）。
///
/// 兼容 v3.0 / v4.0 / v4.1：v3.0 备份内的 agent_configs / workflow_steps /
/// task_dependencies / prompt_templates / agent_executions 字段以及 todo / subtask
/// 上的 agent / 调度 / 工作流字段，会被 serde 在反序列化阶段静默忽略
/// （`ExportData` / `Todo` / `SubTask` 在 v2.0 后不再声明这些字段）。
///
/// 1. 导入前先 `VACUUM INTO` 备份当前库（`backups/`，保留最近 5 份；内存库跳过）
/// 2. 单个事务内：清空 → 按**原 id** 插入（子任务的 parentId 以外层待办为准）→ 写设置
/// 3. 导入的记录 `updated_at` 统一改为导入时刻：恢复备份是用户的明确意图，应当在同步
///    合并中胜出，而不是被远端的"较新"旧数据覆盖（A11）。导入前本地已有同 id 记录、且它的
///    时间比"现在"还新（同步自时钟偏快的设备）时，取它 + 1 秒——否则恢复的版本照样输给它
/// 4. 导入前存在、导入后不存在的记录写墓碑（时间不早于记录自己的 updated_at，理由同上），
///    删除随同步传播；导入后存在的记录清掉同键墓碑
/// 5. 清单（契约 §9）：备份有 `todoLists`（4.1）时同样按上面的规则恢复（保留 id、`updated_at` 刷新、备份里没有的
///    清单写清单墓碑并删除），默认清单始终保留、只取备份里的名字；被删掉的清单的窗口记录随之清理
/// 6. 待办的归属（契约 §12-9）：3.0 / 4.0 备份（没有 `todoLists`）的待办没有 `listId`，恢复成**具体的**默认清单 1
///    （导入旧备份 = 全部进默认清单，并随同步传播）；现有清单不动。4.1 备份里缺 `listId` 的待办导出时就是"未知"，
///    照原样恢复成未知（同步时从别处学到真实归属，不会以新版本把它们全改成 1）
///
/// 任一步失败整体回滚，原有数据不受影响。
///
/// 成功后调用方要刷新运行时偏好（`settings::runtime_prefs`）并通知各窗口重新加载（`import_data`）。
pub fn import_json(db: &Database, json_data: &str) -> Result<(), String> {
    let import: ExportData =
        serde_json::from_str(json_data).map_err(|e| format!("Invalid JSON format: {}", e))?;

    db.with_connection(|conn| Ok(backup::snapshot(conn, "import")))
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("导入前备份失败，已取消导入: {}", e))?;

    let now = now_local();
    db.with_transaction(|tx| import_records(tx, &import, &now))
        .map_err(|e| e.to_string())
}

fn import_records(
    conn: &rusqlite::Connection,
    import: &ExportData,
    now: &str,
) -> rusqlite::Result<()> {
    // 导入前的 id → updated_at
    let versions_of = |table: &str| -> rusqlite::Result<HashMap<i64, String>> {
        let mut stmt = conn.prepare(&format!("SELECT id, updated_at FROM {table}"))?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    };
    let before_todos = versions_of("todos")?;
    let before_subtasks = versions_of("subtasks")?;
    // 导入版本的时间：至少是现在，且严格晚于被它取代的本地版本
    let imported_at = |before: &HashMap<i64, String>, id: i64| match before.get(&id) {
        Some(previous) => superseding(now, previous),
        None => now.to_string(),
    };

    conn.execute("DELETE FROM subtasks", [])?;
    conn.execute("DELETE FROM todos", [])?;

    // 3.0 / 4.0 备份不认识清单：恢复它 = 全部进默认清单（具体的 1，不是"未知"）
    let legacy_backup = import.todo_lists.is_none();
    let mut after_todos = HashSet::new();
    let mut after_subtasks = HashSet::new();
    for original in &import.todos {
        let mut todo = original.clone();
        normalize_todo_lenient(&mut todo);
        if legacy_backup && todo.list_id.is_none() {
            todo.list_id = Some(DEFAULT_LIST_ID);
        }
        todo.updated_at = imported_at(&before_todos, todo.id);
        if todo.created_at.trim().is_empty() {
            todo.created_at = now.to_string();
        }
        insert_todo_row(conn, &todo)?;
        after_todos.insert(todo.id);

        for sub in &todo.subtasks {
            let mut sub = sub.clone();
            sub.parent_id = todo.id;
            sub.updated_at = imported_at(&before_subtasks, sub.id);
            if sub.created_at.trim().is_empty() {
                sub.created_at = now.to_string();
            }
            insert_subtask_row(conn, &sub)?;
            after_subtasks.insert(sub.id);
        }
    }

    for (id, updated_at) in &before_todos {
        if !after_todos.contains(id) {
            let deleted_at = sync_store::deletion_time(now, updated_at);
            sync_store::record_tombstone(conn, EntityKind::Todo, *id, &deleted_at)?;
        }
    }
    for (id, updated_at) in &before_subtasks {
        if !after_subtasks.contains(id) {
            let deleted_at = sync_store::deletion_time(now, updated_at);
            sync_store::record_tombstone(conn, EntityKind::Subtask, *id, &deleted_at)?;
        }
    }
    for id in &after_todos {
        sync_store::remove_tombstone(conn, EntityKind::Todo, *id)?;
    }
    for id in &after_subtasks {
        sync_store::remove_tombstone(conn, EntityKind::Subtask, *id)?;
    }

    if let Some(lists) = &import.todo_lists {
        import_lists(conn, lists, now)?;
    }
    window_prefs::prune_orphans(conn)?;

    write_app_settings(conn, &import.settings)?;
    Ok(())
}

/// 4.1 备份里的清单（契约 §9）：保留 id upsert，`updated_at` 刷新为导入时刻（严格晚于被它取代的本地版本）；
/// 本地有、备份里没有的清单（默认清单除外）写清单墓碑并删除；默认清单始终保留，只取备份里的名字。
/// 被导入的清单清掉同键的清单墓碑
fn import_lists(
    conn: &rusqlite::Connection,
    lists: &[TodoList],
    now: &str,
) -> rusqlite::Result<()> {
    let before: HashMap<i64, String> = {
        let mut stmt = conn.prepare("SELECT id, updated_at FROM todo_lists")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let imported_at = |id: i64| match before.get(&id) {
        Some(previous) => superseding(now, previous),
        None => now.to_string(),
    };

    let mut kept: HashSet<i64> = HashSet::from([DEFAULT_LIST_ID]);
    for original in lists {
        let mut list = original.clone();
        normalize_list_lenient(&mut list);
        list.updated_at = imported_at(list.id);
        if list.id == DEFAULT_LIST_ID {
            conn.execute(
                "INSERT INTO todo_lists (id, name, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, 0, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, updated_at = excluded.updated_at",
                params![
                    DEFAULT_LIST_ID,
                    list.name,
                    DEFAULT_LIST_CREATED_AT,
                    list.updated_at
                ],
            )?;
        } else {
            if list.created_at.trim().is_empty() {
                list.created_at = now.to_string();
            }
            conn.execute(
                "INSERT INTO todo_lists (id, name, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET name = excluded.name, sort_order = excluded.sort_order,
                     created_at = excluded.created_at, updated_at = excluded.updated_at",
                params![
                    list.id,
                    list.name,
                    list.sort_order,
                    list.created_at,
                    list.updated_at
                ],
            )?;
        }
        kept.insert(list.id);
        sync_store::remove_list_tombstone(conn, list.id)?;
    }

    for (id, updated_at) in &before {
        if !kept.contains(id) {
            sync_store::record_list_tombstone(
                conn,
                *id,
                &sync_store::deletion_time(now, updated_at),
            )?;
            conn.execute("DELETE FROM todo_lists WHERE id = ?1", [id])?;
        }
    }
    Ok(())
}

/// 导出为 ZIP（内含 `data.json`，Deflated）（`export_data_to_file`）
pub fn export_to_file(db: &Database, file_path: &Path) -> Result<(), String> {
    let json_data = export_json(db)?;

    let file = std::fs::File::create(file_path).map_err(|e| format!("创建文件失败: {}", e))?;
    let mut zip = zip::ZipWriter::new(file);

    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("data.json", options)
        .map_err(|e| format!("写入 ZIP 失败: {}", e))?;
    zip.write_all(json_data.as_bytes())
        .map_err(|e| format!("写入数据失败: {}", e))?;

    zip.finish().map_err(|e| format!("完成 ZIP 失败: {}", e))?;
    Ok(())
}

/// 从文件导入：按 `PK` 魔数区分 ZIP（取 `data.json`）与 JSON，再走 [`import_json`]（`import_data_from_file`）
pub fn import_from_file(db: &Database, file_path: &Path) -> Result<(), String> {
    let file_bytes = std::fs::read(file_path).map_err(|e| format!("读取文件失败: {}", e))?;

    // ZIP magic bytes: PK (0x50, 0x4B)
    let is_zip = file_bytes.len() >= 2 && file_bytes[0] == 0x50 && file_bytes[1] == 0x4B;

    if is_zip {
        let cursor = std::io::Cursor::new(&file_bytes);
        let mut archive =
            zip::ZipArchive::new(cursor).map_err(|e| format!("解析 ZIP 失败: {}", e))?;

        let mut json_data = String::new();
        let mut data_file = archive
            .by_name("data.json")
            .map_err(|e| format!("ZIP 中未找到 data.json: {}", e))?;
        data_file
            .read_to_string(&mut json_data)
            .map_err(|e| format!("读取 data.json 失败: {}", e))?;

        import_json(db, &json_data)
    } else {
        let json_data =
            String::from_utf8(file_bytes).map_err(|e| format!("文件编码错误: {}", e))?;
        import_json(db, &json_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{SubTask, Todo};
    use rusqlite::OptionalExtension;

    fn test_db() -> Database {
        Database::new_in_memory().expect("打开内存库失败")
    }

    fn make_todo(id: i64, title: &str) -> Todo {
        Todo {
            id,
            title: title.to_string(),
            description: None,
            color: "#F59E0B".to_string(),
            quadrant: 4,
            notify_at: None,
            notify_before: 0,
            notified: false,
            completed: false,
            sort_order: 0,
            start_time: None,
            end_time: None,
            created_at: "2026-01-01 00:00:00".to_string(),
            updated_at: "2026-01-01 00:00:00".to_string(),
            repeat_enabled: false,
            repeat_type: None,
            repeat_interval: 1,
            repeat_weekdays: None,
            repeat_month_day: None,
            // 3.0 / 4.0 备份里的待办没有 listId
            list_id: None,
            subtasks: Vec::new(),
        }
    }

    fn make_subtask(id: i64, parent_id: i64, title: &str) -> SubTask {
        SubTask {
            id,
            parent_id,
            title: title.to_string(),
            content: None,
            completed: false,
            sort_order: 0,
            created_at: "2026-01-01 00:00:00".to_string(),
            updated_at: "2026-01-01 00:00:00".to_string(),
        }
    }

    fn backup_json(todos: Vec<Todo>) -> String {
        let data = ExportData {
            version: "4.0".to_string(),
            exported_at: "2026-01-02T00:00:00+08:00".to_string(),
            todo_lists: None,
            todos,
            settings: AppSettings {
                is_fixed: false,
                fixed_embed_desktop: false,
                window_position: None,
                window_size: None,
                auto_hide_enabled: true,
                top_on_wake: true,
                window_bg_color: DEFAULT_WINDOW_BG_COLOR.to_string(),
                window_bg_alpha: DEFAULT_WINDOW_BG_ALPHA,
                text_theme: "dark".to_string(),
                show_calendar: false,
                view_mode: "list".to_string(),
                notification_type: "system".to_string(),
            },
        };
        serde_json::to_string(&data).expect("序列化导出数据失败")
    }

    /// 先塞一条已有待办 + 子任务，模拟"用户导入前的现存数据"
    fn seed_existing(db: &Database) {
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO todos (id, title, color, quadrant, created_at, updated_at)
                 VALUES (1, '已有待办', '#EF4444', 1, '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
                [],
            )?;
            conn.execute(
                "INSERT INTO subtasks (id, parent_id, title, created_at, updated_at)
                 VALUES (11, 1, '已有子任务', '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
                [],
            )?;
            Ok(())
        })
        .expect("写入初始数据失败");
    }

    fn count(db: &Database, table: &str) -> i64 {
        db.with_connection(|conn| {
            conn.query_row(&format!("SELECT COUNT(*) FROM {}", table), [], |r| r.get(0))
        })
        .expect("统计行数失败")
    }

    fn titles(db: &Database) -> Vec<String> {
        db.with_connection(|conn| {
            let mut stmt = conn.prepare("SELECT title FROM todos ORDER BY id")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.filter_map(|r| r.ok()).collect())
        })
        .expect("读取标题失败")
    }

    #[test]
    fn import_replaces_all_data_on_success() {
        let db = test_db();
        seed_existing(&db);

        let mut t = make_todo(100, "导入待办");
        t.subtasks.push(make_subtask(200, 100, "导入子任务"));
        import_json(&db, &backup_json(vec![t])).expect("导入应当成功");

        assert_eq!(titles(&db), vec!["导入待办".to_string()]);
        assert_eq!(count(&db, "subtasks"), 1);
    }

    /// 导入中途失败（此处用 TEMP TRIGGER 模拟磁盘满 / IO 错误导致的 INSERT 失败）时，
    /// 事务整体回滚，用户原有数据必须完好无损。
    #[test]
    fn failed_import_rolls_back_and_keeps_existing_data() {
        let db = test_db();
        seed_existing(&db);

        db.with_connection(|conn| {
            conn.execute_batch(
                "CREATE TEMP TRIGGER fail_import BEFORE INSERT ON todos
                 WHEN NEW.title = '__fail__'
                 BEGIN SELECT RAISE(ABORT, 'boom'); END;",
            )
        })
        .expect("创建测试触发器失败");

        let mut ok_todo = make_todo(100, "导入待办");
        ok_todo.subtasks.push(make_subtask(200, 100, "导入子任务"));
        let json = backup_json(vec![ok_todo, make_todo(101, "__fail__")]);

        let err = import_json(&db, &json).expect_err("导入应当失败");
        assert!(err.contains("boom"), "应当是触发器中断导致的失败：{}", err);

        assert_eq!(titles(&db), vec!["已有待办".to_string()]);
        assert_eq!(count(&db, "subtasks"), 1);
    }

    fn setting_value(db: &Database, key: &str) -> Option<String> {
        db.with_connection(|conn| {
            Ok(conn
                .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
                    r.get::<_, String>(0)
                })
                .ok())
        })
        .expect("读取设置失败")
    }

    /// v27 之前的备份没有 fixedEmbedDesktop 字段：导入不报错，且按 serde default 落库为 false。
    ///
    /// 先把库里预置成 true：v27 迁移本身就会把 fixed_embed_desktop 种成 false，不预置的话
    /// 断言无论 write_app_settings 有没有写这个键都会通过，测不出东西。
    #[test]
    fn import_without_fixed_embed_desktop_defaults_to_false() {
        let db = test_db();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('fixed_embed_desktop', 'true')",
                [],
            )?;
            Ok(())
        })
        .expect("预置 fixed_embed_desktop 失败");

        let json = backup_json(vec![make_todo(1, "a")]).replace("\"fixedEmbedDesktop\":false,", "");
        assert!(
            !json.contains("fixedEmbedDesktop"),
            "测试前提：JSON 中不含 fixedEmbedDesktop"
        );

        import_json(&db, &json).expect("旧版备份导入应当成功");

        assert_eq!(
            setting_value(&db, "fixed_embed_desktop").as_deref(),
            Some("false")
        );
    }

    /// 导出 → 导入往返后 fixed_embed_desktop 保留。
    #[test]
    fn fixed_embed_desktop_survives_export_import_round_trip() {
        let db = test_db();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES ('fixed_embed_desktop', 'true')",
                [],
            )?;
            Ok(())
        })
        .expect("写入 fixed_embed_desktop 失败");

        let exported = export_json(&db).expect("导出失败");
        assert!(exported.contains("\"fixedEmbedDesktop\": true"));

        let db2 = test_db();
        import_json(&db2, &exported).expect("导入失败");
        assert_eq!(
            setting_value(&db2, "fixed_embed_desktop").as_deref(),
            Some("true")
        );
    }

    fn tombstones(db: &Database) -> Vec<(String, i64)> {
        db.with_connection(|conn| {
            let mut stmt =
                conn.prepare("SELECT entity_type, entity_id FROM tombstones ORDER BY 1, 2")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
            rows.collect()
        })
        .expect("读取墓碑失败")
    }

    /// 导入保留原 id；导入前存在、导入后不存在的记录生成墓碑；被导入的 id 不留墓碑。
    #[test]
    fn import_preserves_ids_and_tombstones_removed_records() {
        let db = test_db();
        seed_existing(&db); // todo 1 + subtask 11
        db.with_transaction(|tx| {
            sync_store::record_tombstone(tx, EntityKind::Todo, 100, "2026-01-01 00:00:00")
        })
        .unwrap();

        let mut t = make_todo(100, "导入待办");
        t.subtasks.push(make_subtask(200, 999, "导入子任务"));
        import_json(&db, &backup_json(vec![t])).expect("导入应当成功");

        let (todo_id, sub_id, parent): (i64, i64, i64) = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT t.id, s.id, s.parent_id FROM todos t JOIN subtasks s ON s.parent_id = t.id",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
            })
            .unwrap();
        assert_eq!((todo_id, sub_id, parent), (100, 200, 100));

        assert_eq!(
            tombstones(&db),
            vec![("subtask".to_string(), 11), ("todo".to_string(), 1)]
        );
    }

    /// 本地记录同步自时钟偏快的设备、updated_at 比"现在"还新：恢复的版本要严格晚于它，
    /// 被移除记录的墓碑也要压得住它——否则下次同步时这些"未来"版本照样胜出，恢复被悄悄撤销
    #[test]
    fn import_beats_future_dated_local_versions() {
        let db = test_db();
        db.with_connection(|conn| {
            conn.execute_batch(
                "INSERT INTO todos (id, title, updated_at) VALUES
                 (1, '将被恢复', '2099-01-01 08:00:00'), (2, '将被移除', '2099-01-02 08:00:00');
                 INSERT INTO subtasks (id, parent_id, title, updated_at) VALUES
                 (11, 1, '将被恢复', '2099-01-01 09:00:00'), (12, 2, '将被移除', '2099-01-03 08:00:00');",
            )
        })
        .unwrap();

        let mut t = make_todo(1, "恢复的版本");
        t.subtasks.push(make_subtask(11, 1, "恢复的子任务"));
        import_json(&db, &backup_json(vec![t])).unwrap();

        let updated_at = |table: &str, id: i64| -> String {
            db.with_connection(|c| {
                c.query_row(
                    &format!("SELECT updated_at FROM {table} WHERE id = ?1"),
                    [id],
                    |r| r.get(0),
                )
            })
            .unwrap()
        };
        assert_eq!(updated_at("todos", 1), "2099-01-01 08:00:01");
        assert_eq!(updated_at("subtasks", 11), "2099-01-01 09:00:01");

        let tombs = db
            .with_connection(|c| sync_store::list_tombstones(c, "2000-01-01 00:00:00"))
            .unwrap();
        let find = |kind: &str, id: i64| {
            tombs
                .iter()
                .find(|t| t.entity_type == kind && t.entity_id == id)
                .map(|t| t.deleted_at.clone())
        };
        assert_eq!(find("todo", 2).as_deref(), Some("2099-01-02 08:00:00"));
        assert_eq!(find("subtask", 12).as_deref(), Some("2099-01-03 08:00:00"));
    }

    /// 导入的记录 updated_at 改为导入时刻（恢复备份是权威的），时间字段规范化。
    #[test]
    fn import_sets_updated_at_to_now_and_normalizes_times() {
        let db = test_db();
        let mut t = make_todo(7, "带时间");
        t.notify_at = Some("2026-05-01T09:30".to_string());
        t.end_time = Some("2026-05-02".to_string());
        t.subtasks.push(make_subtask(70, 7, "子"));
        let before = now_local();
        import_json(&db, &backup_json(vec![t])).unwrap();

        let (notify_at, end_time, updated_at, created_at): (String, String, String, String) = db
            .with_connection(|c| {
                c.query_row(
                    "SELECT notify_at, end_time, updated_at, created_at FROM todos WHERE id = 7",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
            })
            .unwrap();
        assert_eq!(notify_at, "2026-05-01 09:30:00");
        assert_eq!(end_time, "2026-05-02 23:59:00");
        assert!(updated_at >= before, "{updated_at} 应不早于 {before}");
        assert_eq!(created_at, "2026-01-01 00:00:00", "created_at 保留");
        let sub_updated: String = db
            .with_connection(|c| {
                c.query_row("SELECT updated_at FROM subtasks WHERE id = 70", [], |r| {
                    r.get(0)
                })
            })
            .unwrap();
        assert!(sub_updated >= before);
    }

    /// 导出带上规范化后的时间；导出顺序按 sort_order。
    #[test]
    fn export_normalizes_legacy_time_formats() {
        let db = test_db();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO todos (id, title, color, quadrant, notify_at, created_at, updated_at)
                 VALUES (1, 'a', '#EF4444', 1, '2026-05-01T09:30', '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let exported: ExportData = serde_json::from_str(&export_json(&db).unwrap()).unwrap();
        assert_eq!(exported.version, EXPORT_VERSION);
        assert_eq!(
            exported.todos[0].notify_at.as_deref(),
            Some("2026-05-01 09:30:00")
        );
    }

    fn setting_updated_at(db: &Database, key: &str) -> Option<String> {
        db.with_connection(|c| {
            c.query_row(
                "SELECT updated_at FROM settings WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()
        })
        .unwrap()
    }

    /// 远端设置只写存在且合法的同步键，窗口几何永不应用，变化的键记远端时间戳。
    #[test]
    fn apply_settings_json_is_partial_validated_and_skips_window_geometry() {
        let db = test_db();
        let remote = serde_json::json!({
            "isFixed": true,
            "textTheme": "light",
            "viewMode": "bogus",
            "windowBgColor": "#123456",
            "windowBgAlpha": 2.5,
            "windowPosition": {"x": 1, "y": 2},
            "windowSize": {"width": 3, "height": 4}
        });
        let changed = db
            .with_transaction(|tx| {
                apply_settings_json(tx, remote.as_object().unwrap(), "2026-05-01 08:00:00")
            })
            .unwrap();
        assert!(changed);
        assert_eq!(setting_value(&db, "is_fixed").as_deref(), Some("true"));
        assert_eq!(setting_value(&db, "text_theme").as_deref(), Some("light"));
        assert_eq!(
            setting_value(&db, "window_bg_color").as_deref(),
            Some("#123456")
        );
        assert_eq!(
            setting_value(&db, "view_mode").as_deref(),
            Some("list"),
            "非法值忽略"
        );
        assert_eq!(
            setting_value(&db, "window_bg_alpha").as_deref(),
            Some(DEFAULT_WINDOW_BG_ALPHA.to_string().as_str()),
            "越界透明度忽略"
        );
        assert_eq!(setting_value(&db, "window_position"), None);
        assert_eq!(setting_value(&db, "window_size"), None);
        assert_eq!(
            setting_updated_at(&db, "is_fixed").as_deref(),
            Some("2026-05-01 08:00:00")
        );

        // 同值再应用：无变化
        let changed = db
            .with_transaction(|tx| {
                apply_settings_json(tx, remote.as_object().unwrap(), "2026-06-01 08:00:00")
            })
            .unwrap();
        assert!(!changed);
        assert_eq!(
            setting_updated_at(&db, "is_fixed").as_deref(),
            Some("2026-05-01 08:00:00")
        );
    }

    /// 设置版本只看同步键：窗口几何变化不推进版本，同步键变化推进版本。
    #[test]
    fn settings_version_tracks_only_synced_keys() {
        let db = test_db();
        db.with_connection(|conn| {
            conn.execute("UPDATE settings SET updated_at = '2026-01-01 00:00:00'", [])?;
            Ok(())
        })
        .unwrap();
        let v0 = db.with_connection(settings_version).unwrap();
        assert_eq!(v0.as_deref(), Some("2026-01-01 00:00:00"));

        db.with_connection(|conn| {
            set_setting(conn, "window_position", r#"{"x":5,"y":5}"#)?;
            Ok(())
        })
        .unwrap();
        assert_eq!(db.with_connection(settings_version).unwrap(), v0);

        db.with_connection(|conn| {
            set_bool_setting(conn, "show_calendar", true)?;
            Ok(())
        })
        .unwrap();
        assert!(db.with_connection(settings_version).unwrap() > v0);
    }

    /// 采用远端设置后，设置版本恰好等于远端版本（不会因为未覆盖的键较新而再次上传）
    #[test]
    fn adopt_settings_aligns_version_with_remote() {
        let db = test_db();
        let remote = serde_json::json!({"textTheme": "light"});
        db.with_transaction(|tx| {
            adopt_settings_json(tx, remote.as_object().unwrap(), "2020-01-01 00:00:00")
        })
        .unwrap();
        assert_eq!(setting_value(&db, "text_theme").as_deref(), Some("light"));
        assert_eq!(
            db.with_connection(settings_version).unwrap().as_deref(),
            Some("2020-01-01 00:00:00")
        );
        assert_eq!(
            setting_updated_at(&db, "view_mode").as_deref(),
            Some("2020-01-01 00:00:00"),
            "未被远端覆盖的键也对齐到远端版本"
        );
    }

    /// 导出到 ZIP 再从文件导入（ZIP 与纯 JSON 两种文件都认）
    #[test]
    fn file_export_and_import_round_trip_through_zip_and_json() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "mini-todo-data-test-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let db = test_db();
        seed_existing(&db);
        let zip_path = dir.join("backup.zip");
        export_to_file(&db, &zip_path).unwrap();
        assert_eq!(&std::fs::read(&zip_path).unwrap()[..2], b"PK");

        let restored = test_db();
        import_from_file(&restored, &zip_path).unwrap();
        assert_eq!(titles(&restored), vec!["已有待办".to_string()]);
        assert_eq!(count(&restored, "subtasks"), 1);

        let json_path = dir.join("backup.json");
        std::fs::write(&json_path, backup_json(vec![make_todo(5, "来自 JSON")])).unwrap();
        import_from_file(&restored, &json_path).unwrap();
        assert_eq!(titles(&restored), vec!["来自 JSON".to_string()]);

        assert!(import_from_file(&restored, &dir.join("missing.zip"))
            .unwrap_err()
            .contains("读取文件失败"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- 清单（格式 4.1，契约 §9）----

    use crate::lists;
    use crate::window_prefs::{UpdateWindowPrefs, WindowKey};

    fn backup_list(id: i64, name: &str) -> TodoList {
        TodoList {
            id,
            name: name.to_string(),
            sort_order: 3,
            created_at: "2026-01-01 00:00:00".to_string(),
            updated_at: "2026-01-01 00:00:00".to_string(),
        }
    }

    fn backup_with_lists(todos: Vec<Todo>, todo_lists: Vec<TodoList>) -> String {
        let mut data: ExportData = serde_json::from_str(&backup_json(todos)).unwrap();
        data.version = "4.1".to_string();
        data.todo_lists = Some(todo_lists);
        serde_json::to_string(&data).unwrap()
    }

    /// (id, name, sort_order, updated_at)，按 id
    fn list_rows(db: &Database) -> Vec<(i64, String, i64, String)> {
        db.with_connection(|conn| {
            conn.prepare("SELECT id, name, sort_order, updated_at FROM todo_lists ORDER BY id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect()
        })
        .unwrap()
    }

    fn list_tombstones(db: &Database) -> Vec<(i64, String)> {
        db.with_connection(|conn| {
            conn.prepare("SELECT entity_id, deleted_at FROM list_tombstones ORDER BY entity_id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
        .unwrap()
    }

    /// 存储的归属；`None` = 未知
    fn list_id_of(db: &Database, todo_id: i64) -> Option<i64> {
        db.with_connection(|c| {
            c.query_row("SELECT list_id FROM todos WHERE id = ?1", [todo_id], |r| {
                r.get(0)
            })
        })
        .unwrap()
    }

    /// 导出 4.1：顶层 todoLists（含默认清单），归属已知的待办带 listId、未知的不写（契约 §12-5）；不导出清单墓碑与
    /// 窗口属性。导入到另一个库往返一致（未知仍是未知）
    #[test]
    fn export_4_1_carries_lists_and_round_trips() {
        let db = test_db();
        let work = lists::create(&db, "工作").unwrap();
        let gone = lists::create(&db, "已删").unwrap();
        lists::delete(&db, gone.id).unwrap();
        seed_existing(&db);
        db.with_connection(|c| {
            c.execute("UPDATE todos SET list_id = ?1 WHERE id = 1", [work.id])?;
            // 迁移前的存量待办：归属未知
            c.execute(
                "INSERT INTO todos (id, title, created_at, updated_at)
                 VALUES (2, '未知', '2026-01-01 00:00:00', '2026-01-01 00:00:00')",
                [],
            )
        })
        .unwrap();

        let text = export_json(&db).unwrap();
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], "4.1");
        let ids: Vec<i64> = value["todoLists"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![1, work.id], "按 (sortOrder, id)");
        assert_eq!(value["todoLists"][1]["name"], "工作");
        assert_eq!(value["todos"][0]["listId"], work.id);
        assert!(value["todos"][1].get("listId").is_none(), "未知不写 listId");
        assert!(value.get("listTombstones").is_none());

        let restored = test_db();
        import_json(&restored, &text).unwrap();
        let names: Vec<String> = lists::list(&restored)
            .unwrap()
            .into_iter()
            .map(|l| l.name)
            .collect();
        assert_eq!(names, vec!["待办清单", "工作"]);
        assert_eq!(list_id_of(&restored, 1), Some(work.id));
        assert_eq!(
            list_id_of(&restored, 2),
            None,
            "4.1 备份里的未知照原样恢复成未知，同步时再从别处学到"
        );
    }

    /// 导入 4.1 = 恢复备份：保留 id、updated_at 刷新为导入时刻；备份里没有的清单写清单墓碑并删除（窗口记录一并清理）；
    /// 默认清单始终保留、只取备份里的名字；被导入的清单清掉同键墓碑
    #[test]
    fn import_4_1_restores_lists_like_a_backup() {
        let db = test_db();
        let local_only = lists::create(&db, "本机独有").unwrap();
        let shared = lists::create(&db, "两边都有").unwrap();
        crate::window_prefs::update_prefs(
            &db,
            WindowKey::List(local_only.id),
            &UpdateWindowPrefs {
                dark: Some(true),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        // 备份里的 777 本机删过（有墓碑）；本机"两边都有"的版本来自时钟偏快的设备
        db.with_transaction(|tx| sync_store::record_list_tombstone(tx, 777, "2026-01-01 00:00:00"))
            .unwrap();
        db.with_connection(|c| {
            c.execute(
                "UPDATE todo_lists SET updated_at = '2099-01-01 08:00:00' WHERE id = ?1",
                [shared.id],
            )
        })
        .unwrap();

        let mut default_list = backup_list(DEFAULT_LIST_ID, "备份里的默认清单名");
        default_list.sort_order = 9;
        let mut todo = make_todo(1, "a");
        todo.list_id = Some(777);
        let json = backup_with_lists(
            vec![todo],
            vec![
                default_list,
                backup_list(shared.id, "备份名"),
                backup_list(777, "曾被删除"),
            ],
        );
        let before = now_local();
        import_json(&db, &json).unwrap();

        let rows = list_rows(&db);
        assert_eq!(
            rows.iter().map(|r| r.0).collect::<Vec<_>>(),
            vec![1, 777, shared.id]
        );
        let (_, name, sort_order, updated_at) = &rows[0];
        assert_eq!(
            (name.as_str(), *sort_order),
            ("备份里的默认清单名", 0),
            "默认清单只取名字"
        );
        assert!(updated_at.as_str() >= before.as_str(), "{updated_at}");
        assert_eq!(rows[1].1, "曾被删除");
        assert!(rows[1].3 >= before);
        assert_eq!((rows[2].1.as_str(), rows[2].2), ("备份名", 3));
        assert_eq!(
            rows[2].3, "2099-01-01 08:00:01",
            "严格晚于被它取代的本地版本"
        );

        let tombs = list_tombstones(&db);
        assert_eq!(tombs.len(), 1, "777 的墓碑被清掉，只剩本机独有的那个");
        assert_eq!(tombs[0].0, local_only.id);
        assert!(tombs[0].1 >= before);
        assert_eq!(
            crate::window_prefs::get_prefs(&db, WindowKey::List(local_only.id)).unwrap(),
            None
        );
        assert_eq!(list_id_of(&db, 1), Some(777));
    }

    #[test]
    fn import_4_1_without_the_default_list_still_keeps_it() {
        let db = test_db();
        import_json(&db, &backup_with_lists(vec![], vec![backup_list(55, "x")])).unwrap();
        let rows = list_rows(&db);
        assert_eq!(
            rows[0],
            (
                1,
                "待办清单".to_string(),
                0,
                "2000-01-01 00:00:00".to_string()
            ),
            "备份里没有默认清单：原样保留"
        );
        assert_eq!(rows.len(), 2);
        assert!(list_tombstones(&db).is_empty());
    }

    /// 导入 3.0 / 4.0（没有 todoLists）：现有清单不动（不改名、不刷新、不写墓碑）；备份里的待办没有 listId，
    /// 恢复成**具体的**默认清单 1（契约 §12-9：导入旧备份 = 全部进默认清单，并随同步传播），不是"未知"
    #[test]
    fn importing_a_4_0_backup_leaves_existing_lists_alone() {
        let db = test_db();
        let work = lists::create(&db, "工作").unwrap();
        lists::rename(&db, DEFAULT_LIST_ID, "收件箱").unwrap();
        let before = list_rows(&db);

        let mut kept = make_todo(6, "手写了 listId 的旧格式");
        kept.list_id = Some(work.id);
        let doc: Value =
            serde_json::from_str(&backup_json(vec![make_todo(5, "旧备份"), kept])).unwrap();
        assert!(doc.get("todoLists").is_none(), "前提：4.0 备份没有清单");
        assert!(doc["todos"][0].get("listId").is_none(), "前提：没有 listId");
        import_json(&db, &doc.to_string()).unwrap();

        assert_eq!(list_rows(&db), before);
        assert!(list_tombstones(&db).is_empty());
        assert_eq!(list_id_of(&db, 5), Some(DEFAULT_LIST_ID));
        assert_eq!(list_id_of(&db, 6), Some(work.id), "有 listId 的照原值");
        let exported: Value = serde_json::from_str(&export_json(&db).unwrap()).unwrap();
        assert_eq!(
            exported["todos"][0]["listId"], DEFAULT_LIST_ID,
            "导出 / 上传时写出具体的 1"
        );
    }

    /// 旧版本（4.0 的 ExportData，没有 todoLists 字段）读 4.1 导出：未知字段忽略，照常导入
    #[test]
    fn a_4_1_export_still_parses_with_the_4_0_shape() {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        #[allow(dead_code)]
        struct ExportDataV40 {
            version: String,
            exported_at: String,
            todos: Vec<Value>,
            settings: AppSettings,
        }
        let db = test_db();
        lists::create(&db, "工作").unwrap();
        seed_existing(&db);
        let old: ExportDataV40 = serde_json::from_str(&export_json(&db).unwrap()).unwrap();
        assert_eq!(old.todos.len(), 1);
    }
}

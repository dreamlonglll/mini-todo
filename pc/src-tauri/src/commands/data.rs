use crate::db::backup;
pub(crate) use crate::db::settings_kv::settings_version;
use crate::db::settings_kv::{
    bool_str, get_bool_setting, get_setting, get_setting_or, set_bool_setting, set_setting,
    set_setting_at, SYNCED_SETTING_KEYS,
};
use crate::db::sync_store::{
    self, insert_subtask_row, insert_todo_row, normalize_todo_lenient, EntityKind,
};
use crate::db::time::{now_local, superseding};
use crate::db::{
    AppSettings, Database, ExportData, WindowPosition, WindowSize, DEFAULT_WINDOW_BG_ALPHA,
    DEFAULT_WINDOW_BG_COLOR,
};
use chrono::Local;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::io::{Read as _, Write as _};
use tauri::{AppHandle, Manager};

/// 导出 / 同步文档的格式版本
pub const EXPORT_VERSION: &str = "4.0";

fn read_json_setting<T: serde::de::DeserializeOwned>(
    conn: &rusqlite::Connection,
    key: &str,
) -> Option<T> {
    get_setting(conn, key)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
}

pub(crate) fn read_app_settings(conn: &rusqlite::Connection) -> AppSettings {
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
pub(crate) fn write_app_settings(
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

pub fn export_data_internal(db: &Database) -> Result<String, String> {
    let (todos, settings) = db
        .with_connection(|conn| {
            let mut todos = sync_store::load_todos_with_subtasks(conn, "sort_order ASC, id ASC")?;
            for todo in &mut todos {
                normalize_todo_lenient(todo);
            }
            Ok((todos, read_app_settings(conn)))
        })
        .map_err(|e| e.to_string())?;

    let export_data = ExportData {
        version: EXPORT_VERSION.to_string(),
        exported_at: Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(),
        todos,
        settings,
    };
    serde_json::to_string_pretty(&export_data).map_err(|e| e.to_string())
}

/// 导入备份数据（手动导入，语义为"整体替换"）。
///
/// 兼容 v3.0 与 v4.0 两个版本：v3.0 备份内的 agent_configs / workflow_steps /
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
///
/// 任一步失败整体回滚，原有数据不受影响。
pub fn import_data_raw(db: &Database, json_data: &str) -> Result<(), String> {
    let import: ExportData =
        serde_json::from_str(json_data).map_err(|e| format!("Invalid JSON format: {}", e))?;

    db.with_connection(|conn| Ok(backup::snapshot(conn, "import")))
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("导入前备份失败，已取消导入: {}", e))?;

    let now = now_local();
    db.with_transaction(|tx| import_records(tx, &import, &now))
        .map_err(|e| e.to_string())?;

    // 导入改写了 top_on_wake / auto_hide_enabled 等设置：刷新窗口模块的运行时缓存
    crate::commands::window::reload_runtime_prefs(db);
    Ok(())
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

    let mut after_todos = HashSet::new();
    let mut after_subtasks = HashSet::new();
    for original in &import.todos {
        let mut todo = original.clone();
        normalize_todo_lenient(&mut todo);
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

    write_app_settings(conn, &import.settings)?;
    Ok(())
}

/// 在阻塞线程池里跑 `f(&Database)`：导入导出要读写文件、做备份，不能占住主线程（C1）
async fn run_blocking<R, F>(app: AppHandle, f: F) -> Result<R, String>
where
    F: FnOnce(&Database) -> Result<R, String> + Send + 'static,
    R: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let db = app.state::<Database>();
        f(&db)
    })
    .await
    .map_err(|e| format!("后台任务异常: {}", e))?
}

#[tauri::command]
pub async fn export_data(app: AppHandle) -> Result<String, String> {
    run_blocking(app, export_data_internal).await
}

#[tauri::command]
pub async fn import_data(app: AppHandle, json_data: String) -> Result<(), String> {
    run_blocking(app, move |db| import_data_raw(db, &json_data)).await
}

#[tauri::command]
pub async fn export_data_to_file(app: AppHandle, file_path: String) -> Result<(), String> {
    run_blocking(app, move |db| export_to_file(db, &file_path)).await
}

#[tauri::command]
pub async fn import_data_from_file(app: AppHandle, file_path: String) -> Result<(), String> {
    run_blocking(app, move |db| import_from_file(db, &file_path)).await
}

fn export_to_file(db: &Database, file_path: &str) -> Result<(), String> {
    let json_data = export_data_internal(db)?;

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

fn import_from_file(db: &Database, file_path: &str) -> Result<(), String> {
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

        import_data_raw(db, &json_data)
    } else {
        let json_data =
            String::from_utf8(file_bytes).map_err(|e| format!("文件编码错误: {}", e))?;
        import_data_raw(db, &json_data)
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

    fn export_json(todos: Vec<Todo>) -> String {
        let data = ExportData {
            version: "4.0".to_string(),
            exported_at: "2026-01-02T00:00:00+08:00".to_string(),
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
        import_data_raw(&db, &export_json(vec![t])).expect("导入应当成功");

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
        let json = export_json(vec![ok_todo, make_todo(101, "__fail__")]);

        let err = import_data_raw(&db, &json).expect_err("导入应当失败");
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

        let json = export_json(vec![make_todo(1, "a")]).replace("\"fixedEmbedDesktop\":false,", "");
        assert!(
            !json.contains("fixedEmbedDesktop"),
            "测试前提：JSON 中不含 fixedEmbedDesktop"
        );

        import_data_raw(&db, &json).expect("旧版备份导入应当成功");

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

        let exported = export_data_internal(&db).expect("导出失败");
        assert!(exported.contains("\"fixedEmbedDesktop\": true"));

        let db2 = test_db();
        import_data_raw(&db2, &exported).expect("导入失败");
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
        import_data_raw(&db, &export_json(vec![t])).expect("导入应当成功");

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
        import_data_raw(&db, &export_json(vec![t])).unwrap();

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
        import_data_raw(&db, &export_json(vec![t])).unwrap();

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
        let exported: ExportData =
            serde_json::from_str(&export_data_internal(&db).unwrap()).unwrap();
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
}

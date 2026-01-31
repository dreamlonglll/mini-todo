use rusqlite::{Connection, Result};

pub fn run_migrations(conn: &Connection) -> Result<()> {
    // 创建迁移版本表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        )",
        [],
    )?;

    let current_version: i32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if current_version < 1 {
        migration_v1(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (1)", [])?;
    }

    if current_version < 2 {
        migration_v2(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (2)", [])?;
    }

    Ok(())
}

/// 迁移 v2：新增屏幕配置表，支持多屏幕组合下保存不同的窗口位置
fn migration_v2(conn: &Connection) -> Result<()> {
    // 创建屏幕配置表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS screen_configs (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            config_id       TEXT NOT NULL UNIQUE,
            display_name    TEXT,
            window_x        INTEGER NOT NULL,
            window_y        INTEGER NOT NULL,
            window_width    INTEGER NOT NULL,
            window_height   INTEGER NOT NULL,
            is_fixed        INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            updated_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        )",
        [],
    )?;

    // 创建索引
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_screen_configs_config_id ON screen_configs(config_id)",
        [],
    )?;

    // 迁移旧的设置数据到新表（如果存在）
    // 读取旧的窗口位置和尺寸
    let old_position: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'window_position'",
            [],
            |row| row.get(0),
        )
        .ok();

    let old_size: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'window_size'",
            [],
            |row| row.get(0),
        )
        .ok();

    let old_is_fixed: bool = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'is_fixed'",
            [],
            |row| {
                let val: String = row.get(0)?;
                Ok(val == "true")
            },
        )
        .unwrap_or(false);

    // 如果有旧数据，创建一个默认的屏幕配置记录
    if let (Some(pos_json), Some(size_json)) = (old_position, old_size) {
        // 解析旧的位置和尺寸
        if let (Ok(pos), Ok(size)) = (
            serde_json::from_str::<serde_json::Value>(&pos_json),
            serde_json::from_str::<serde_json::Value>(&size_json),
        ) {
            let x = pos.get("x").and_then(|v| v.as_i64()).unwrap_or(100) as i32;
            let y = pos.get("y").and_then(|v| v.as_i64()).unwrap_or(100) as i32;
            let width = size.get("width").and_then(|v| v.as_i64()).unwrap_or(380) as i32;
            let height = size.get("height").and_then(|v| v.as_i64()).unwrap_or(600) as i32;

            // 使用 "legacy" 作为旧配置的标识，用户可以稍后删除
            conn.execute(
                "INSERT OR IGNORE INTO screen_configs 
                 (config_id, display_name, window_x, window_y, window_width, window_height, is_fixed) 
                 VALUES ('legacy', '旧版配置', ?1, ?2, ?3, ?4, ?5)",
                (x, y, width, height, if old_is_fixed { 1 } else { 0 }),
            )?;
        }
    }

    Ok(())
}

fn migration_v1(conn: &Connection) -> Result<()> {
    // 创建待办表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS todos (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            title           TEXT NOT NULL,
            description     TEXT,
            priority        TEXT NOT NULL DEFAULT 'medium' CHECK(priority IN ('high', 'medium', 'low')),
            notify_at       TEXT,
            notify_before   INTEGER DEFAULT 0,
            notified        INTEGER DEFAULT 0,
            completed       INTEGER NOT NULL DEFAULT 0,
            sort_order      INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            updated_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        )",
        [],
    )?;

    // 创建子任务表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS subtasks (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            parent_id       INTEGER NOT NULL,
            title           TEXT NOT NULL,
            completed       INTEGER NOT NULL DEFAULT 0,
            sort_order      INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            updated_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            FOREIGN KEY (parent_id) REFERENCES todos(id) ON DELETE CASCADE
        )",
        [],
    )?;

    // 创建设置表
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (
            key             TEXT PRIMARY KEY,
            value           TEXT NOT NULL,
            updated_at      TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        )",
        [],
    )?;

    // 创建索引
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_todos_completed ON todos(completed)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_todos_sort_order ON todos(sort_order)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_todos_notify_at ON todos(notify_at)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_subtasks_parent_id ON subtasks(parent_id)",
        [],
    )?;

    Ok(())
}

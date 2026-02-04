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

    if current_version < 3 {
        migration_v3(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (3)", [])?;
    }

    if current_version < 4 {
        migration_v4(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (4)", [])?;
    }

    if current_version < 5 {
        migration_v5(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (5)", [])?;
    }

    Ok(())
}

/// 迁移 v5：添加 quadrant 字段，支持四象限视图
/// quadrant 值：1=重要紧急, 2=重要不紧急, 3=紧急不重要, 4=不紧急不重要
fn migration_v5(conn: &Connection) -> Result<()> {
    // 添加 quadrant 列，默认为 4（不紧急不重要）
    conn.execute(
        "ALTER TABLE todos ADD COLUMN quadrant INTEGER NOT NULL DEFAULT 4",
        [],
    )?;

    // 创建索引以优化四象限查询
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_todos_quadrant ON todos(quadrant)",
        [],
    )?;

    // 初始化视图模式设置（默认列表模式）
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('view_mode', 'list', datetime('now', 'localtime'))",
        [],
    )?;

    Ok(())
}

/// 迁移 v4：将 priority 字段改为 color 字段，支持自定义颜色
fn migration_v4(conn: &Connection) -> Result<()> {
    // 添加 color 列，默认橙色
    conn.execute(
        "ALTER TABLE todos ADD COLUMN color TEXT NOT NULL DEFAULT '#F59E0B'",
        [],
    )?;

    // 根据 priority 迁移颜色数据
    // high -> 红色 #EF4444
    conn.execute(
        "UPDATE todos SET color = '#EF4444' WHERE priority = 'high'",
        [],
    )?;
    // medium -> 橙色 #F59E0B (已是默认值)
    conn.execute(
        "UPDATE todos SET color = '#F59E0B' WHERE priority = 'medium'",
        [],
    )?;
    // low -> 绿色 #10B981
    conn.execute(
        "UPDATE todos SET color = '#10B981' WHERE priority = 'low'",
        [],
    )?;

    Ok(())
}

/// 迁移 v3：todos 表新增 start_time 和 end_time 字段，支持日历视图
fn migration_v3(conn: &Connection) -> Result<()> {
    // 新增 start_time 字段（开始时间，可为空）
    conn.execute(
        "ALTER TABLE todos ADD COLUMN start_time TEXT",
        [],
    )?;

    // 新增 end_time 字段（截止时间，可为空）
    conn.execute(
        "ALTER TABLE todos ADD COLUMN end_time TEXT",
        [],
    )?;

    // 创建索引以优化日历查询
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_todos_start_time ON todos(start_time)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_todos_end_time ON todos(end_time)",
        [],
    )?;

    // 初始化日历显示设置（默认关闭）
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('show_calendar', 'false', datetime('now', 'localtime'))",
        [],
    )?;

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

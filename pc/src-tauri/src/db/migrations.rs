use rusqlite::{Connection, Result, Transaction, TransactionBehavior};

/// 执行单个迁移，并把"迁移体 + 版本号 INSERT"包进同一个事务。
///
/// 迁移中途失败（多语句迁移尤其容易）时整体回滚，数据库停留在上一个版本，
/// 下次启动可以安全重跑；否则会出现"前半截 DDL 已生效、版本号没记上"，
/// 重跑时在已变更的 schema 上再次报错，用户陷入"启动即失败"的死循环。
///
/// `run_migrations` 只拿得到 `&Connection`（`Database::with_connection` 的约束），
/// 用不了需要 `&mut` 的 `conn.transaction()`，因此走 `Transaction::new_unchecked`；
/// 返回的 guard 默认 drop 即 ROLLBACK。
fn apply_migration<F>(conn: &Connection, version: i32, f: F) -> Result<()>
where
    F: FnOnce(&Connection) -> Result<()>,
{
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    f(&tx)?;
    tx.execute("INSERT INTO migrations (version) VALUES (?1)", [version])?;
    tx.commit()
}

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
        apply_migration(conn, 1, migration_v1)?;
    }

    if current_version < 2 {
        apply_migration(conn, 2, migration_v2)?;
    }

    if current_version < 3 {
        apply_migration(conn, 3, migration_v3)?;
    }

    if current_version < 4 {
        apply_migration(conn, 4, migration_v4)?;
    }

    if current_version < 5 {
        apply_migration(conn, 5, migration_v5)?;
    }

    if current_version < 6 {
        apply_migration(conn, 6, migration_v6)?;
    }

    if current_version < 7 {
        apply_migration(conn, 7, migration_v7)?;
    }

    if current_version < 8 {
        apply_migration(conn, 8, migration_v8)?;
    }

    if current_version < 9 {
        apply_migration(conn, 9, migration_v9)?;
    }

    if current_version < 10 {
        apply_migration(conn, 10, migration_v10)?;
    }

    if current_version < 11 {
        apply_migration(conn, 11, migration_v11)?;
    }

    if current_version < 12 {
        apply_migration(conn, 12, migration_v12)?;
    }

    if current_version < 13 {
        apply_migration(conn, 13, migration_v13)?;
    }

    if current_version < 14 {
        apply_migration(conn, 14, migration_v14)?;
    }

    if current_version < 15 {
        apply_migration(conn, 15, migration_v15)?;
    }

    if current_version < 16 {
        apply_migration(conn, 16, migration_v16)?;
    }

    if current_version < 17 {
        apply_migration(conn, 17, migration_v17)?;
    }

    if current_version < 18 {
        apply_migration(conn, 18, migration_v18)?;
    }

    if current_version < 19 {
        apply_migration(conn, 19, migration_v19)?;
    }

    if current_version < 20 {
        apply_migration(conn, 20, migration_v20)?;
    }

    if current_version < 21 {
        apply_migration(conn, 21, migration_v21)?;
    }

    if current_version < 22 {
        apply_migration(conn, 22, migration_v22)?;
    }

    if current_version < 23 {
        apply_migration(conn, 23, migration_v23)?;
    }

    if current_version < 24 {
        apply_migration(conn, 24, migration_v24)?;
    }

    if current_version < 25 {
        apply_migration(conn, 25, migration_v25)?;
    }

    if current_version < 26 {
        apply_migration(conn, 26, migration_v26)?;
    }

    Ok(())
}

/// 迁移 v26：新增 `window_bg_color` / `window_bg_alpha` settings key。
///
/// 窗口底色与背景透明度（issue #9 第 4 点）。默认值取自 `models.rs` 的常量，
/// 等价于改为可配置之前深色主题多层半透明黑的合成观感，保证老用户升级后外观不变。
/// 仅深色主题下生效，浅色主题是不透明白底。
fn migration_v26(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('window_bg_color', ?1, datetime('now', 'localtime'))",
        [crate::db::models::DEFAULT_WINDOW_BG_COLOR],
    )?;
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('window_bg_alpha', ?1, datetime('now', 'localtime'))",
        [crate::db::models::DEFAULT_WINDOW_BG_ALPHA.to_string()],
    )?;
    Ok(())
}

/// 迁移 v25：新增 `top_on_wake` settings key。
///
/// 贴边自动隐藏唤起窗口时是否临时置顶。默认开启——不置顶时窗口只是移回锚点，
/// Z 序不变，会被最大化/无边框全屏窗口完全遮住（issue #9 第 5 点）。
/// 允许关闭是为了照顾全屏游戏等不希望被打断的场景。
fn migration_v25(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('top_on_wake', 'true', datetime('now', 'localtime'))",
        [],
    )?;
    Ok(())
}

/// 迁移 v24：新增 `webdav_last_modified` settings key。
///
/// 用于 PC 端 WebDAV 同步走条件 PUT（`If-Unmodified-Since`），避免覆盖
/// cloud 端 / 其它 PC 并发写入。值是 server 在 `Last-Modified` header 返回的
/// HTTP 日期字符串（例如 `Wed, 13 May 2026 12:34:56 GMT`），初始为空。
///
/// 使用 `INSERT OR IGNORE` 保护已存在的值不被覆盖（防止重复 migration 误擦数据）。
fn migration_v24(conn: &Connection) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('webdav_last_modified', '', datetime('now', 'localtime'))",
        [],
    )?;
    Ok(())
}

/// 迁移 v23：移除所有 AI Agent / 任务调度 / 工作流相关表和字段。
///
/// 删除的表：agent_configs / agent_executions / workflow_steps / task_dependencies / prompt_templates。
/// 从 todos 表移除：agent_id / agent_project_path / schedule_strategy / cron_expression /
///   schedule_enabled / last_scheduled_run / post_action / workflow_enabled / workflow_current_step。
/// 从 subtasks 表移除：schedule_status / priority_score / max_retries / retry_count /
///   timeout_secs / scheduled_at / last_scheduled_run / schedule_error。
///
/// 重复提醒字段（repeat_enabled / repeat_type / repeat_interval / repeat_weekdays /
/// repeat_month_day）保留，由 v22 引入。
fn migration_v23(conn: &Connection) -> Result<()> {
    // 删除 AI Agent 相关表
    conn.execute_batch(
        "DROP TABLE IF EXISTS agent_executions;
         DROP TABLE IF EXISTS workflow_steps;
         DROP TABLE IF EXISTS task_dependencies;
         DROP TABLE IF EXISTS prompt_templates;
         DROP TABLE IF EXISTS agent_configs;",
    )?;

    // 移除 todos 表的 Agent / 调度 / 工作流字段
    // SQLite 3.35+ 支持 ALTER TABLE DROP COLUMN（rusqlite 0.32 bundled SQLite >= 3.40 满足）
    conn.execute_batch(
        "ALTER TABLE todos DROP COLUMN agent_id;
         ALTER TABLE todos DROP COLUMN agent_project_path;
         ALTER TABLE todos DROP COLUMN schedule_strategy;
         ALTER TABLE todos DROP COLUMN cron_expression;
         ALTER TABLE todos DROP COLUMN schedule_enabled;
         ALTER TABLE todos DROP COLUMN last_scheduled_run;
         ALTER TABLE todos DROP COLUMN post_action;
         ALTER TABLE todos DROP COLUMN workflow_enabled;
         ALTER TABLE todos DROP COLUMN workflow_current_step;",
    )?;

    // 移除 subtasks 表的调度字段
    conn.execute_batch(
        "DROP INDEX IF EXISTS idx_subtasks_schedule_status;
         ALTER TABLE subtasks DROP COLUMN schedule_status;
         ALTER TABLE subtasks DROP COLUMN priority_score;
         ALTER TABLE subtasks DROP COLUMN max_retries;
         ALTER TABLE subtasks DROP COLUMN retry_count;
         ALTER TABLE subtasks DROP COLUMN timeout_secs;
         ALTER TABLE subtasks DROP COLUMN scheduled_at;
         ALTER TABLE subtasks DROP COLUMN last_scheduled_run;
         ALTER TABLE subtasks DROP COLUMN schedule_error;",
    )?;

    Ok(())
}

/// 迁移 v22：todos 表新增重复提醒字段
fn migration_v22(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE todos ADD COLUMN repeat_enabled INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE todos ADD COLUMN repeat_type TEXT;
         ALTER TABLE todos ADD COLUMN repeat_interval INTEGER NOT NULL DEFAULT 1;
         ALTER TABLE todos ADD COLUMN repeat_weekdays TEXT;
         ALTER TABLE todos ADD COLUMN repeat_month_day INTEGER;",
    )
}

/// 迁移 v21：移除内置提示词模板
fn migration_v21(conn: &Connection) -> Result<()> {
    conn.execute("DELETE FROM prompt_templates WHERE is_builtin = 1", [])?;
    Ok(())
}

/// 迁移 v20：工作流上下文传递支持
/// - agent_executions 新增 session_id 字段，保存 Agent 会话 ID
/// - workflow_steps 新增 carry_context 字段，标记是否带入上一步结果
fn migration_v20(conn: &Connection) -> Result<()> {
    conn.execute(
        "ALTER TABLE agent_executions ADD COLUMN session_id TEXT",
        [],
    )?;
    conn.execute(
        "ALTER TABLE workflow_steps ADD COLUMN carry_context INTEGER NOT NULL DEFAULT 0",
        [],
    )?;
    Ok(())
}

/// 迁移 v11：创建 agent_executions 表，持久化 Agent 执行记录和日志。
fn migration_v11(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_executions (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            task_id         TEXT    NOT NULL,
            subtask_id      INTEGER,
            agent_id        INTEGER,
            status          TEXT    NOT NULL DEFAULT 'running',
            logs            TEXT    NOT NULL DEFAULT '[]',
            result_text     TEXT    NOT NULL DEFAULT '',
            error           TEXT,
            input_tokens    INTEGER NOT NULL DEFAULT 0,
            output_tokens   INTEGER NOT NULL DEFAULT 0,
            start_time_ms   INTEGER NOT NULL DEFAULT 0,
            duration_ms     INTEGER NOT NULL DEFAULT 0,
            created_at      TEXT    NOT NULL DEFAULT (datetime('now', 'localtime')),
            FOREIGN KEY (subtask_id) REFERENCES subtasks(id) ON DELETE SET NULL,
            FOREIGN KEY (agent_id) REFERENCES agent_configs(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_agent_executions_subtask ON agent_executions(subtask_id);
        CREATE INDEX IF NOT EXISTS idx_agent_executions_task ON agent_executions(task_id);",
    )
}

/// 迁移 v12：agent_executions 表新增 agent_type 字段，
/// 支持前端按 Agent 类型分别处理日志显示。
fn migration_v12(conn: &Connection) -> Result<()> {
    conn.execute(
        "ALTER TABLE agent_executions ADD COLUMN agent_type TEXT NOT NULL DEFAULT ''",
        [],
    )?;
    Ok(())
}

/// 迁移 v13：扩展 subtasks 表增加调度相关字段。
fn migration_v13(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE subtasks ADD COLUMN schedule_status TEXT NOT NULL DEFAULT 'none';
         ALTER TABLE subtasks ADD COLUMN priority_score INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE subtasks ADD COLUMN max_retries INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE subtasks ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
         ALTER TABLE subtasks ADD COLUMN timeout_secs INTEGER NOT NULL DEFAULT 600;
         ALTER TABLE subtasks ADD COLUMN scheduled_at TEXT;
         ALTER TABLE subtasks ADD COLUMN last_scheduled_run TEXT;
         ALTER TABLE subtasks ADD COLUMN schedule_error TEXT;
         CREATE INDEX IF NOT EXISTS idx_subtasks_schedule_status ON subtasks(schedule_status);",
    )
}

/// 迁移 v14：扩展 todos 表增加调度策略字段。
fn migration_v14(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE todos ADD COLUMN schedule_strategy TEXT NOT NULL DEFAULT 'manual';
         ALTER TABLE todos ADD COLUMN cron_expression TEXT;
         ALTER TABLE todos ADD COLUMN schedule_enabled INTEGER NOT NULL DEFAULT 0;",
    )
}

/// 迁移 v15：创建任务依赖关系表。
fn migration_v15(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS task_dependencies (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            subtask_id      INTEGER NOT NULL,
            depends_on_id   INTEGER NOT NULL,
            dependency_type TEXT    NOT NULL DEFAULT 'finish-to-start',
            created_at      TEXT    NOT NULL DEFAULT (datetime('now', 'localtime')),
            FOREIGN KEY (subtask_id) REFERENCES subtasks(id) ON DELETE CASCADE,
            FOREIGN KEY (depends_on_id) REFERENCES subtasks(id) ON DELETE CASCADE,
            UNIQUE(subtask_id, depends_on_id)
        );
        CREATE INDEX IF NOT EXISTS idx_task_deps_subtask ON task_dependencies(subtask_id);
        CREATE INDEX IF NOT EXISTS idx_task_deps_depends ON task_dependencies(depends_on_id);",
    )
}

/// 迁移 v16：创建 Prompt 模板表（内置模板已在 v21 中移除）。
fn migration_v16(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS prompt_templates (
            id                  TEXT PRIMARY KEY,
            name                TEXT NOT NULL,
            category            TEXT,
            description         TEXT,
            template_content    TEXT NOT NULL,
            variables           TEXT NOT NULL DEFAULT '[]',
            recommended_agent   TEXT,
            is_builtin          INTEGER NOT NULL DEFAULT 0,
            created_at          TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            updated_at          TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
        );",
    )
}

/// 迁移 v17：给 todos 表添加 last_scheduled_run 字段，用于 Cron 定时任务触发时间记录。
fn migration_v17(conn: &Connection) -> Result<()> {
    conn.execute("ALTER TABLE todos ADD COLUMN last_scheduled_run TEXT", [])?;
    Ok(())
}

/// 迁移 v18：给 todos 表添加 post_action 字段（已废弃，保留兼容）。
fn migration_v18(conn: &Connection) -> Result<()> {
    let has_column: bool = conn
        .prepare("SELECT post_action FROM todos LIMIT 0")
        .is_ok();
    if !has_column {
        conn.execute(
            "ALTER TABLE todos ADD COLUMN post_action TEXT NOT NULL DEFAULT 'none'",
            [],
        )?;
    }
    Ok(())
}

/// 迁移 v19：创建 workflow_steps 表 + todos 表新增工作流字段。
fn migration_v19(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_steps (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            todo_id     INTEGER NOT NULL,
            step_order  INTEGER NOT NULL,
            step_type   TEXT NOT NULL CHECK(step_type IN ('subtask', 'prompt')),
            subtask_id  INTEGER,
            prompt_text TEXT,
            status      TEXT NOT NULL DEFAULT 'pending',
            created_at  TEXT NOT NULL DEFAULT (datetime('now', 'localtime')),
            FOREIGN KEY (todo_id) REFERENCES todos(id) ON DELETE CASCADE,
            FOREIGN KEY (subtask_id) REFERENCES subtasks(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_steps_todo ON workflow_steps(todo_id, step_order);",
    )?;

    let has_wf_enabled: bool = conn
        .prepare("SELECT workflow_enabled FROM todos LIMIT 0")
        .is_ok();
    if !has_wf_enabled {
        conn.execute_batch(
            "ALTER TABLE todos ADD COLUMN workflow_enabled INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE todos ADD COLUMN workflow_current_step INTEGER NOT NULL DEFAULT -1;",
        )?;
    }
    Ok(())
}

/// 迁移 v10：简化 agent_configs 表，移除不再需要的字段。
/// 自动检测模式下 API key、sandbox 等由 CLI 自行管理。
fn migration_v10(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE agent_configs_new (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            name            TEXT    NOT NULL,
            agent_type      TEXT    NOT NULL CHECK(agent_type IN ('claude_code', 'codex', 'custom')),
            cli_path        TEXT    NOT NULL DEFAULT '',
            enabled         INTEGER NOT NULL DEFAULT 1,
            created_at      TEXT    NOT NULL DEFAULT (datetime('now', 'localtime')),
            updated_at      TEXT    NOT NULL DEFAULT (datetime('now', 'localtime'))
        );
        INSERT INTO agent_configs_new (id, name, agent_type, cli_path, enabled, created_at, updated_at)
            SELECT id, name, agent_type, cli_path, enabled, created_at, updated_at FROM agent_configs;
        DROP TABLE agent_configs;
        ALTER TABLE agent_configs_new RENAME TO agent_configs;"
    )
}

/// 迁移 v9：todos 表新增 agent_id 和 agent_project_path 字段，
/// 用于在待办级别绑定 Agent 配置，子任务执行时使用。
fn migration_v9(conn: &Connection) -> Result<()> {
    conn.execute(
        "ALTER TABLE todos ADD COLUMN agent_id INTEGER REFERENCES agent_configs(id) ON DELETE SET NULL",
        [],
    )?;
    conn.execute("ALTER TABLE todos ADD COLUMN agent_project_path TEXT", [])?;
    Ok(())
}

/// 迁移 v8：创建 agent_configs 表，支持 AI Agent 集成
fn migration_v8(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_configs (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            name            TEXT    NOT NULL,
            agent_type      TEXT    NOT NULL CHECK(agent_type IN ('claude_code', 'codex', 'custom')),
            cli_path        TEXT    NOT NULL DEFAULT '',
            cli_version     TEXT    NOT NULL DEFAULT '',
            min_cli_version TEXT    NOT NULL DEFAULT '',
            api_key_encrypted TEXT  NOT NULL DEFAULT '',
            default_model   TEXT    NOT NULL DEFAULT '',
            max_concurrent  INTEGER NOT NULL DEFAULT 1,
            timeout_seconds INTEGER NOT NULL DEFAULT 300,
            capabilities    TEXT    NOT NULL DEFAULT '{}',
            env_vars        TEXT    NOT NULL DEFAULT '{}',
            sandbox_config  TEXT    NOT NULL DEFAULT '{}',
            enabled         INTEGER NOT NULL DEFAULT 1,
            created_at      TEXT    NOT NULL DEFAULT (datetime('now', 'localtime')),
            updated_at      TEXT    NOT NULL DEFAULT (datetime('now', 'localtime'))
        );"
    )
}

/// 迁移 v7：subtasks 表新增 content 列，支持 Markdown 内容
fn migration_v7(conn: &Connection) -> Result<()> {
    conn.execute("ALTER TABLE subtasks ADD COLUMN content TEXT", [])?;
    Ok(())
}

/// 迁移 v6：添加通知类型设置，支持系统通知和软件通知切换
fn migration_v6(conn: &Connection) -> Result<()> {
    // 初始化通知类型设置（默认系统通知）
    conn.execute(
        "INSERT OR IGNORE INTO settings (key, value, updated_at) VALUES ('notification_type', 'system', datetime('now', 'localtime'))",
        [],
    )?;

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
    conn.execute("ALTER TABLE todos ADD COLUMN start_time TEXT", [])?;

    // 新增 end_time 字段（截止时间，可为空）
    conn.execute("ALTER TABLE todos ADD COLUMN end_time TEXT", [])?;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Connection {
        let conn = Connection::open_in_memory().expect("打开内存库失败");
        // 与 Database::new 保持一致，避免测试与真实启动路径行为分叉
        conn.execute("PRAGMA foreign_keys = ON", [])
            .expect("启用外键失败");
        conn
    }

    fn conn_with_migrations_table() -> Connection {
        let conn = fresh_conn();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT (datetime('now', 'localtime'))
            )",
            [],
        )
        .expect("创建 migrations 表失败");
        conn
    }

    fn max_version(conn: &Connection) -> i32 {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM migrations",
            [],
            |row| row.get(0),
        )
        .expect("读取迁移版本失败")
    }

    fn table_exists(conn: &Connection, name: &str) -> bool {
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .expect("查询表是否存在失败")
            > 0
    }

    /// 迁移中途失败：已执行的 DDL 回滚、版本号不推进，下次启动可安全重跑。
    #[test]
    fn failed_migration_rolls_back_ddl_and_version() {
        let conn = conn_with_migrations_table();

        let result = apply_migration(&conn, 99, |c| {
            c.execute("CREATE TABLE probe (id INTEGER PRIMARY KEY)", [])?;
            // 模拟多语句迁移的后半截失败
            c.execute("INSERT INTO not_exists_table (id) VALUES (1)", [])?;
            Ok(())
        });

        assert!(result.is_err(), "迁移应当失败");
        assert!(!table_exists(&conn, "probe"), "失败迁移的 DDL 应当被回滚");
        assert_eq!(max_version(&conn), 0, "失败迁移不应写入版本号");

        // 重试：修好后重跑同一版本可以正常推进
        apply_migration(&conn, 99, |c| {
            c.execute("CREATE TABLE probe (id INTEGER PRIMARY KEY)", [])?;
            Ok(())
        })
        .expect("重试迁移应当成功");

        assert!(table_exists(&conn, "probe"));
        assert_eq!(max_version(&conn), 99);
    }

    /// 26 个迁移逐个包事务后，全新库仍能一次性迁到最新版本。
    #[test]
    fn fresh_database_migrates_to_latest_version() {
        let conn = fresh_conn();
        run_migrations(&conn).expect("全新库迁移失败");

        assert_eq!(max_version(&conn), 26);
        assert!(table_exists(&conn, "todos"));
        assert!(table_exists(&conn, "subtasks"));
        assert!(table_exists(&conn, "settings"));
        assert!(table_exists(&conn, "screen_configs"));
        // v23 已删除的 Agent 相关表不应残留
        assert!(!table_exists(&conn, "agent_configs"));
    }

    /// 迁移是幂等的：重复调用不重复执行、版本号不变。
    #[test]
    fn rerunning_migrations_is_noop() {
        let conn = fresh_conn();
        run_migrations(&conn).expect("首次迁移失败");
        run_migrations(&conn).expect("二次迁移失败");

        assert_eq!(max_version(&conn), 26);
    }
}

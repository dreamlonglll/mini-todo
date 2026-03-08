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

    if current_version < 6 {
        migration_v6(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (6)", [])?;
    }

    if current_version < 7 {
        migration_v7(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (7)", [])?;
    }

    if current_version < 8 {
        migration_v8(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (8)", [])?;
    }

    if current_version < 9 {
        migration_v9(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (9)", [])?;
    }

    if current_version < 10 {
        migration_v10(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (10)", [])?;
    }

    if current_version < 11 {
        migration_v11(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (11)", [])?;
    }

    if current_version < 12 {
        migration_v12(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (12)", [])?;
    }

    if current_version < 13 {
        migration_v13(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (13)", [])?;
    }

    if current_version < 14 {
        migration_v14(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (14)", [])?;
    }

    if current_version < 15 {
        migration_v15(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (15)", [])?;
    }

    if current_version < 16 {
        migration_v16(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (16)", [])?;
    }

    if current_version < 17 {
        migration_v17(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (17)", [])?;
    }

    if current_version < 18 {
        migration_v18(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (18)", [])?;
    }

    if current_version < 19 {
        migration_v19(conn)?;
        conn.execute("INSERT INTO migrations (version) VALUES (19)", [])?;
    }

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
        CREATE INDEX IF NOT EXISTS idx_agent_executions_task ON agent_executions(task_id);"
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
         CREATE INDEX IF NOT EXISTS idx_subtasks_schedule_status ON subtasks(schedule_status);"
    )
}

/// 迁移 v14：扩展 todos 表增加调度策略字段。
fn migration_v14(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "ALTER TABLE todos ADD COLUMN schedule_strategy TEXT NOT NULL DEFAULT 'manual';
         ALTER TABLE todos ADD COLUMN cron_expression TEXT;
         ALTER TABLE todos ADD COLUMN schedule_enabled INTEGER NOT NULL DEFAULT 0;"
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
        CREATE INDEX IF NOT EXISTS idx_task_deps_depends ON task_dependencies(depends_on_id);"
    )
}

/// 迁移 v16：创建 Prompt 模板表，内置常用模板。
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
        );

        INSERT OR IGNORE INTO prompt_templates (id, name, category, description, template_content, variables, recommended_agent, is_builtin)
        VALUES
            ('builtin_feature', '功能开发', 'development', '新增功能的开发任务',
             '请在项目中实现以下功能：

{{feature_desc}}

技术要求：
{{tech_requirements}}

注意事项：
- 遵循项目现有的代码规范
- 确保代码类型安全
- 添加必要的错误处理',
             '[{\"name\":\"feature_desc\",\"label\":\"功能描述\",\"type\":\"textarea\",\"required\":true},{\"name\":\"tech_requirements\",\"label\":\"技术要求\",\"type\":\"textarea\",\"required\":false,\"default_value\":\"无特殊要求\"}]',
             'claude_code', 1),

            ('builtin_bugfix', 'Bug 修复', 'bugfix', '修复已知问题',
             '请修复以下问题：

问题描述：{{bug_desc}}

复现步骤：
{{reproduce_steps}}

期望行为：
{{expected_behavior}}',
             '[{\"name\":\"bug_desc\",\"label\":\"问题描述\",\"type\":\"textarea\",\"required\":true},{\"name\":\"reproduce_steps\",\"label\":\"复现步骤\",\"type\":\"textarea\",\"required\":false},{\"name\":\"expected_behavior\",\"label\":\"期望行为\",\"type\":\"textarea\",\"required\":false}]',
             'codex', 1),

            ('builtin_refactor', '代码重构', 'refactor', '优化和重构代码',
             '请对以下范围的代码进行重构：

重构范围：{{refactor_scope}}

重构目标：{{refactor_goal}}

约束条件：
- 保持对外接口不变
- 确保功能不受影响
{{constraints}}',
             '[{\"name\":\"refactor_scope\",\"label\":\"重构范围\",\"type\":\"textarea\",\"required\":true},{\"name\":\"refactor_goal\",\"label\":\"重构目标\",\"type\":\"textarea\",\"required\":true},{\"name\":\"constraints\",\"label\":\"额外约束\",\"type\":\"textarea\",\"required\":false}]',
             'claude_code', 1),

            ('builtin_test', '单元测试', 'test', '编写单元测试',
             '请为以下文件/模块编写单元测试：

目标文件：{{target_file}}

测试框架：{{test_framework}}

覆盖目标：{{coverage_goal}}

要求：
- 覆盖主要逻辑分支
- 包含边界条件测试
- 使用清晰的测试命名',
             '[{\"name\":\"target_file\",\"label\":\"目标文件\",\"type\":\"text\",\"required\":true},{\"name\":\"test_framework\",\"label\":\"测试框架\",\"type\":\"text\",\"required\":false,\"default_value\":\"自动检测\"},{\"name\":\"coverage_goal\",\"label\":\"覆盖目标\",\"type\":\"text\",\"required\":false,\"default_value\":\"核心逻辑\"}]',
             'codex', 1),

            ('builtin_review', 'Code Review', 'review', '代码审查',
             '请对以下范围的代码进行审查：

审查范围：{{review_scope}}

关注点：{{review_focus}}

请从以下方面给出建议：
- 代码质量与可读性
- 潜在的 Bug 或安全问题
- 性能优化建议
- 架构设计改进',
             '[{\"name\":\"review_scope\",\"label\":\"审查范围\",\"type\":\"textarea\",\"required\":true},{\"name\":\"review_focus\",\"label\":\"关注重点\",\"type\":\"textarea\",\"required\":false}]',
             'claude_code', 1);"
    )
}

/// 迁移 v17：给 todos 表添加 last_scheduled_run 字段，用于 Cron 定时任务触发时间记录。
fn migration_v17(conn: &Connection) -> Result<()> {
    conn.execute(
        "ALTER TABLE todos ADD COLUMN last_scheduled_run TEXT",
        [],
    )?;
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
        CREATE INDEX IF NOT EXISTS idx_workflow_steps_todo ON workflow_steps(todo_id, step_order);"
    )?;

    let has_wf_enabled: bool = conn
        .prepare("SELECT workflow_enabled FROM todos LIMIT 0")
        .is_ok();
    if !has_wf_enabled {
        conn.execute_batch(
            "ALTER TABLE todos ADD COLUMN workflow_enabled INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE todos ADD COLUMN workflow_current_step INTEGER NOT NULL DEFAULT -1;"
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
    conn.execute(
        "ALTER TABLE todos ADD COLUMN agent_project_path TEXT",
        [],
    )?;
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

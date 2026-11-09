//! 待办清单（v31）：清单 CRUD 与有效清单（读侧兜底）。
//!
//! 清单 = 待办的分区（`todos.list_id`），每个清单在桌面上是一个独立窗口（窗口属性见 [`crate::window_prefs`]）。
//! 清单参与同步（同步文档 `todoLists` / `listTombstones`，合并在 `db::sync_store`）；PC 与 cloud 是同一份数据的两个
//! 写入方，规则逐条对齐 `.trellis/tasks/11-09-multi-list-windows/contract.md`。所有函数阻塞，错误是面向用户的中文。
//!
//! - 默认清单：id 固定为 [`DEFAULT_LIST_ID`]，可改名、不可删除；归属未知（`listId` 缺失，契约 §12）或指向不存在
//!   （含已删除）的清单的待办都按它显示（[`effective_list_id`]，记录本身不改写）
//! - 名称（[`normalize_name`]）：去首尾空白，不能为空、不超过 [`MAX_LIST_NAME_CHARS`] 个字（按 `char` 计）；在现有清单里
//!   去首尾空白后不区分大小写不能重名。同步造成的重名容忍，不在这里处理
//! - 新清单 id：[`crate::ids::new_id`]（与 cloud 同一算法），撞主键换 id 重试；`sortOrder` = 现有最大值 + 1。
//!   列表一律按 `(sortOrder, id)` 升序
//! - 删除（[`delete`]，单事务）：其下待办全部移到默认清单（`updated_at` 严格递增，随同步传播）→ 写清单墓碑
//!   （`deletedAt = max(now, 清单 updatedAt)`）→ 删清单 → 删它的窗口属性与布局、从日历筛选里移出

use crate::db::models::{todo_list_from_row, TodoList, DEFAULT_LIST_CREATED_AT, TODO_LIST_COLUMNS};
use crate::db::sync_store;
use crate::db::time::{now_local, SQL_SET_UPDATED_AT};
use crate::db::Database;
use crate::{ids, window_prefs};
use rusqlite::{params, Connection, OptionalExtension};

pub use crate::db::models::{DEFAULT_LIST_ID, DEFAULT_LIST_NAME};

/// 清单名称的长度上限（按 `char` 计，与 cloud 一致）
pub const MAX_LIST_NAME_CHARS: usize = 50;

/// 清单不存在（新建 / 移动待办的目标清单、改名 / 删除的清单）
pub const LIST_NOT_FOUND: &str = "清单不存在";

/// 全部清单，按 `(sortOrder, id)` 升序（契约 §8：编辑表单下拉、右键「移动到」、日历筛选都按它）；
/// 含默认清单，默认清单缺失时先补建
pub fn list(db: &Database) -> Result<Vec<TodoList>, String> {
    db.with_connection(|conn| {
        ensure_default_list(conn)?;
        sync_store::load_lists(conn)
    })
    .map_err(|e| e.to_string())
}

/// 读取单个清单；不存在报 [`LIST_NOT_FOUND`]
pub fn get(db: &Database, id: i64) -> Result<TodoList, String> {
    db.with_connection(|conn| load_list(conn, id))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| LIST_NOT_FOUND.to_string())
}

/// 规范化清单名称（契约 §6）：去首尾空白；空 → `清单名称不能为空`；超过 50 个字 → `清单名称不能超过 50 个字`。
/// 界面可以先用它校验输入（改名时空名由界面恢复原名）
pub fn normalize_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("清单名称不能为空".to_string());
    }
    if name.chars().count() > MAX_LIST_NAME_CHARS {
        return Err(format!("清单名称不能超过 {} 个字", MAX_LIST_NAME_CHARS));
    }
    Ok(name.to_string())
}

/// 两个清单名是否算重名：去首尾空白后不区分大小写相等（契约 §6，cloud 按名查找用同一规则）
pub fn names_conflict(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// 新建清单：排在最后，返回新清单。名称校验见 [`normalize_name`]；与现有清单重名报 `已有同名清单「<现有名称>」`
pub fn create(db: &Database, name: &str) -> Result<TodoList, String> {
    let name = normalize_name(name)?;
    let now = now_local();
    db.with_transaction(|tx| -> rusqlite::Result<Result<TodoList, String>> {
        if let Some(existing) = find_conflict(tx, &name, None)? {
            return Ok(Err(conflict_error(&existing)));
        }
        let max_order: Option<i64> =
            tx.query_row("SELECT MAX(sort_order) FROM todo_lists", [], |r| r.get(0))?;
        let sort_order = max_order.map_or(0, |max| max.saturating_add(1));
        let id = ids::insert_with_fresh_id(ids::new_id, |id| {
            tx.execute(
                "INSERT INTO todo_lists (id, name, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![id, name, sort_order, now],
            )
            .map(|_| ())
        })?;
        Ok(load_list(tx, id)?.ok_or_else(|| "新建清单后读取失败".to_string()))
    })
    .map_err(|e| e.to_string())?
}

/// 改名（默认清单也可以）：`updated_at` 严格递增（参与 LWW）；名称没变时什么都不改。
/// 名称校验同 [`create`]（只改大小写不算与自己重名）；清单不存在报 [`LIST_NOT_FOUND`]
pub fn rename(db: &Database, id: i64, name: &str) -> Result<TodoList, String> {
    let name = normalize_name(name)?;
    db.with_transaction(|tx| -> rusqlite::Result<Result<TodoList, String>> {
        let Some(current) = load_list(tx, id)? else {
            return Ok(Err(LIST_NOT_FOUND.to_string()));
        };
        if current.name == name {
            return Ok(Ok(current));
        }
        if let Some(existing) = find_conflict(tx, &name, Some(id))? {
            return Ok(Err(conflict_error(&existing)));
        }
        tx.execute(
            &format!("UPDATE todo_lists SET name = ?1, {SQL_SET_UPDATED_AT} WHERE id = ?2"),
            params![name, id],
        )?;
        Ok(load_list(tx, id)?.ok_or_else(|| LIST_NOT_FOUND.to_string()))
    })
    .map_err(|e| e.to_string())?
}

/// 删除清单（契约 §5，单事务）：其下待办（含已完成）全部移到默认清单并刷新 `updated_at`，写清单墓碑，删清单，
/// 再删掉它的窗口属性 / 布局并从日历筛选里移出。返回移到默认清单的待办数。
/// 默认清单报 `默认清单不能删除`；不存在报 [`LIST_NOT_FOUND`]
pub fn delete(db: &Database, id: i64) -> Result<usize, String> {
    if id == DEFAULT_LIST_ID {
        return Err("默认清单不能删除".to_string());
    }
    let now = now_local();
    db.with_transaction(|tx| -> rusqlite::Result<Result<usize, String>> {
        let updated_at: Option<String> = tx
            .query_row(
                "SELECT updated_at FROM todo_lists WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(updated_at) = updated_at else {
            return Ok(Err(LIST_NOT_FOUND.to_string()));
        };
        let moved = tx.execute(
            &format!("UPDATE todos SET list_id = ?1, {SQL_SET_UPDATED_AT} WHERE list_id = ?2"),
            params![DEFAULT_LIST_ID, id],
        )?;
        sync_store::record_list_tombstone(tx, id, &sync_store::deletion_time(&now, &updated_at))?;
        tx.execute("DELETE FROM todo_lists WHERE id = ?1", [id])?;
        window_prefs::forget_list(tx, id)?;
        Ok(Ok(moved))
    })
    .map_err(|e| e.to_string())?
}

/// 存储的 `list_id` 等于 `id` 的待办数（含已完成；归属未知的不算）：删除确认框"N 项待办将移到「待办清单」"用
pub fn todo_count(db: &Database, id: i64) -> Result<usize, String> {
    db.with_connection(|conn| {
        conn.query_row("SELECT COUNT(*) FROM todos WHERE list_id = ?1", [id], |r| {
            r.get::<_, i64>(0)
        })
    })
    .map(|n| n.max(0) as usize)
    .map_err(|e| e.to_string())
}

/// 有效清单（契约 §3 / §12-4）：`list_id` 指向一个现有（未删除）的清单时就是它；归属未知（`None`）或指向不存在
/// （含已删除）的清单时是默认清单。只用于界面归属 / 计数 / 过滤，不写回：存储与同步的仍是原值，收到清单墓碑时也不改写待办
pub fn effective_list_id(list_id: Option<i64>, lists: &[TodoList]) -> i64 {
    match list_id {
        Some(id) if id == DEFAULT_LIST_ID || lists.iter().any(|l| l.id == id) => id,
        _ => DEFAULT_LIST_ID,
    }
}

// ============================================================================
// 内部
// ============================================================================

pub(crate) fn load_list(conn: &Connection, id: i64) -> rusqlite::Result<Option<TodoList>> {
    conn.prepare_cached(&format!(
        "SELECT {TODO_LIST_COLUMNS} FROM todo_lists WHERE id = ?1"
    ))?
    .query_row([id], todo_list_from_row)
    .optional()
}

/// 待办可以放进这个清单吗：默认清单总可以，其它清单必须存在
pub(crate) fn is_valid_target(conn: &Connection, id: i64) -> rusqlite::Result<bool> {
    if id == DEFAULT_LIST_ID {
        return Ok(true);
    }
    Ok(conn
        .prepare_cached("SELECT 1 FROM todo_lists WHERE id = ?1")?
        .query_row([id], |_| Ok(()))
        .optional()?
        .is_some())
}

/// 默认清单缺失（外部工具改过库）时按迁移种子的值补建；已存在时不写（不触发 local_seq）
fn ensure_default_list(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO todo_lists (id, name, sort_order, created_at, updated_at)
         VALUES (?1, ?2, 0, ?3, ?3)",
        params![DEFAULT_LIST_ID, DEFAULT_LIST_NAME, DEFAULT_LIST_CREATED_AT],
    )?;
    Ok(())
}

/// 与 `name` 重名的现有清单（排除 `except`）的名称
fn find_conflict(
    conn: &Connection,
    name: &str,
    except: Option<i64>,
) -> rusqlite::Result<Option<String>> {
    let mut stmt =
        conn.prepare_cached("SELECT id, name FROM todo_lists ORDER BY sort_order, id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (id, existing) = row?;
        if Some(id) != except && names_conflict(&existing, name) {
            return Ok(Some(existing.trim().to_string()));
        }
    }
    Ok(None)
}

fn conflict_error(existing: &str) -> String {
    format!("已有同名清单「{}」", existing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sync_store::local_seq;
    use crate::window_prefs::{self, UpdateWindowPrefs, WindowKey, WindowLayout};

    fn db() -> Database {
        Database::new_in_memory().expect("内存库")
    }

    fn seq(db: &Database) -> i64 {
        db.with_connection(local_seq).unwrap()
    }

    fn list_tombstones(db: &Database) -> Vec<(i64, String)> {
        db.with_connection(|c| {
            c.prepare("SELECT entity_id, deleted_at FROM list_tombstones ORDER BY entity_id")?
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect()
        })
        .unwrap()
    }

    /// `list_id = None`：归属未知（迁移前的存量待办 / 旧版本写的）
    fn insert_todo(db: &Database, id: i64, list_id: Option<i64>, updated_at: &str) {
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO todos (id, title, list_id, updated_at) VALUES (?1, 't', ?2, ?3)",
                params![id, list_id, updated_at],
            )
        })
        .unwrap();
    }

    fn todo_row(db: &Database, id: i64) -> (Option<i64>, String) {
        db.with_connection(|c| {
            c.query_row(
                "SELECT list_id, updated_at FROM todos WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap()
    }

    #[test]
    fn default_list_exists_and_new_lists_go_last_with_time_based_ids() {
        let db = db();
        let lists = list(&db).unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].id, DEFAULT_LIST_ID);
        assert_eq!(lists[0].name, "待办清单");
        assert_eq!(lists[0].updated_at, "2000-01-01 00:00:00");

        let before = now_local();
        let base = seq(&db);
        let work = create(&db, "  工作 ").unwrap();
        assert_eq!(work.name, "工作", "去首尾空白");
        assert_eq!(work.sort_order, 1);
        assert!(
            (ids::FALLBACK_MIN_ID..=ids::MAX_SAFE_ID).contains(&work.id),
            "{}",
            work.id
        );
        assert!(work.created_at >= before && work.created_at == work.updated_at);
        assert!(seq(&db) > base, "新建清单是要同步的本地变更");

        let life = create(&db, "生活").unwrap();
        assert_eq!(life.sort_order, 2);
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(names, vec!["待办清单", "工作", "生活"]);

        // 顺序按 (sortOrder, id)：cloud 改过 sortOrder 后同样生效；新清单仍排在最大值之后
        db.with_connection(|c| {
            c.execute(
                "UPDATE todo_lists SET sort_order = -5 WHERE id = ?1",
                [life.id],
            )
        })
        .unwrap();
        let names: Vec<String> = list(&db).unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(names, vec!["生活", "待办清单", "工作"]);
        assert_eq!(create(&db, "其他").unwrap().sort_order, 2);
        assert_eq!(get(&db, work.id).unwrap(), work);
        assert_eq!(get(&db, 42).unwrap_err(), LIST_NOT_FOUND);
    }

    #[test]
    fn names_are_trimmed_bounded_and_unique_ignoring_case() {
        let db = db();
        for blank in ["", "   ", "\t\n"] {
            assert_eq!(create(&db, blank).unwrap_err(), "清单名称不能为空");
        }
        let fifty: String = "清".repeat(50);
        assert_eq!(create(&db, &format!(" {fifty} ")).unwrap().name, fifty);
        assert_eq!(
            create(&db, &"单".repeat(51)).unwrap_err(),
            "清单名称不能超过 50 个字",
            "按字符而不是字节计"
        );

        let work = create(&db, "Work").unwrap();
        assert_eq!(create(&db, " work ").unwrap_err(), "已有同名清单「Work」");
        assert_eq!(create(&db, "WORK").unwrap_err(), "已有同名清单「Work」");
        assert_eq!(
            create(&db, "待办清单").unwrap_err(),
            "已有同名清单「待办清单」"
        );
        assert!(names_conflict("  Ärger ", "ärger"));
        assert!(!names_conflict("工作", "工作1"));

        // 改名：只改大小写不算与自己重名；改成别的清单的名字（不区分大小写）不行
        let renamed = rename(&db, work.id, "WORK").unwrap();
        assert_eq!(renamed.name, "WORK");
        assert_eq!(
            rename(&db, work.id, "待办清单").unwrap_err(),
            "已有同名清单「待办清单」"
        );
        assert_eq!(rename(&db, work.id, " ").unwrap_err(), "清单名称不能为空");
        assert_eq!(rename(&db, 42, "x").unwrap_err(), LIST_NOT_FOUND);
        assert_eq!(list(&db).unwrap().len(), 3, "失败的操作不留下记录");
    }

    #[test]
    fn rename_moves_updated_at_strictly_forward_and_skips_no_ops() {
        let db = db();
        // 默认清单可以改名：从固定的 2000-01-01 推到现在
        let before = now_local();
        let renamed = rename(&db, DEFAULT_LIST_ID, "收件箱").unwrap();
        assert_eq!(renamed.name, "收件箱");
        assert!(renamed.updated_at >= before, "{}", renamed.updated_at);

        // 版本来自时钟偏快的设备：新版本仍严格更晚
        db.with_connection(|c| {
            c.execute(
                "UPDATE todo_lists SET updated_at = '2099-01-01 08:00:00' WHERE id = 1",
                [],
            )
        })
        .unwrap();
        assert_eq!(
            rename(&db, DEFAULT_LIST_ID, "收件箱 2").unwrap().updated_at,
            "2099-01-01 08:00:01"
        );

        // 名称没变（去首尾空白后）：不写库、不算本地变更
        let base = seq(&db);
        let same = rename(&db, DEFAULT_LIST_ID, "  收件箱 2 ").unwrap();
        assert_eq!(same.updated_at, "2099-01-01 08:00:01");
        assert_eq!(seq(&db), base);
    }

    #[test]
    fn deleting_a_list_moves_its_todos_to_the_default_list_and_writes_a_tombstone() {
        let db = db();
        let work = create(&db, "工作").unwrap();
        let other = create(&db, "其他").unwrap();
        insert_todo(&db, 1, Some(work.id), "2026-01-01 00:00:00");
        // 同步自时钟偏快的设备：移动后的版本也要严格晚于它
        insert_todo(&db, 2, Some(work.id), "2099-01-01 08:00:00");
        insert_todo(&db, 3, Some(other.id), "2026-01-01 00:00:00");
        insert_todo(&db, 4, Some(DEFAULT_LIST_ID), "2026-01-01 00:00:00");
        insert_todo(&db, 5, None, "2026-01-01 00:00:00");
        db.with_connection(|c| c.execute("UPDATE todos SET completed = 1 WHERE id = 2", []))
            .unwrap();
        assert_eq!(todo_count(&db, work.id).unwrap(), 2);

        let before = now_local();
        let base = seq(&db);
        assert_eq!(delete(&db, work.id).unwrap(), 2, "含已完成的待办");
        assert!(seq(&db) > base);

        let (list_id, updated_at) = todo_row(&db, 1);
        assert_eq!(list_id, Some(DEFAULT_LIST_ID));
        assert!(updated_at >= before, "{updated_at}");
        assert_eq!(
            todo_row(&db, 2),
            (Some(DEFAULT_LIST_ID), "2099-01-01 08:00:01".to_string())
        );
        assert_eq!(todo_row(&db, 3).0, Some(other.id), "其它清单的待办不动");
        assert_eq!(
            todo_row(&db, 4),
            (Some(DEFAULT_LIST_ID), "2026-01-01 00:00:00".to_string()),
            "本来就在默认清单的不刷新"
        );
        assert_eq!(
            todo_row(&db, 5),
            (None, "2026-01-01 00:00:00".to_string()),
            "归属未知的待办不动（仍是未知）"
        );

        let tombs = list_tombstones(&db);
        assert_eq!(tombs.len(), 1);
        assert_eq!(tombs[0].0, work.id);
        assert!(tombs[0].1 >= before);
        assert!(get(&db, work.id).is_err());
        assert_eq!(todo_count(&db, work.id).unwrap(), 0);
        assert_eq!(delete(&db, work.id).unwrap_err(), LIST_NOT_FOUND);
    }

    #[test]
    fn deletion_tombstone_beats_a_future_dated_list_version() {
        let db = db();
        let work = create(&db, "工作").unwrap();
        db.with_connection(|c| {
            c.execute(
                "UPDATE todo_lists SET updated_at = '2099-01-01 08:00:00' WHERE id = ?1",
                [work.id],
            )
        })
        .unwrap();
        delete(&db, work.id).unwrap();
        assert_eq!(
            list_tombstones(&db),
            vec![(work.id, "2099-01-01 08:00:00".to_string())],
            "deletedAt >= 被删清单的 updatedAt，删除才压得住它"
        );
    }

    #[test]
    fn the_default_list_cannot_be_deleted() {
        let db = db();
        insert_todo(&db, 1, Some(DEFAULT_LIST_ID), "2026-01-01 00:00:00");
        assert_eq!(
            delete(&db, DEFAULT_LIST_ID).unwrap_err(),
            "默认清单不能删除"
        );
        assert_eq!(list(&db).unwrap().len(), 1);
        assert!(list_tombstones(&db).is_empty());
        assert_eq!(
            todo_row(&db, 1),
            (Some(DEFAULT_LIST_ID), "2026-01-01 00:00:00".to_string())
        );
    }

    #[test]
    fn deleting_a_list_forgets_its_window_and_calendar_filter_entry() {
        let db = db();
        let work = create(&db, "工作").unwrap();
        let life = create(&db, "生活").unwrap();
        let key = WindowKey::List(work.id);
        window_prefs::update_prefs(
            &db,
            key,
            &UpdateWindowPrefs {
                dark: Some(true),
                ..Default::default()
            },
        )
        .unwrap()
        .expect("清单存在");
        assert!(window_prefs::save_layout(
            &db,
            &WindowLayout {
                key,
                screen_config_id: "1_1920x1080@100".to_string(),
                x: 0,
                y: 0,
                width: 380,
                height: 600,
                is_fixed: true,
            }
        )
        .unwrap());
        window_prefs::set_calendar_filter(&db, Some(&[work.id, life.id])).unwrap();

        delete(&db, work.id).unwrap();
        assert_eq!(window_prefs::get_prefs(&db, key).unwrap(), None);
        assert_eq!(
            window_prefs::get_layout(&db, key, "1_1920x1080@100").unwrap(),
            None
        );
        assert_eq!(
            window_prefs::get_prefs(&db, WindowKey::Calendar)
                .unwrap()
                .unwrap()
                .calendar_filter,
            Some(vec![life.id]),
            "被删除的清单从筛选里移出"
        );

        // 筛选只剩被删的清单：回到"全部"
        delete(&db, life.id).unwrap();
        assert_eq!(
            window_prefs::get_prefs(&db, WindowKey::Calendar)
                .unwrap()
                .unwrap()
                .calendar_filter,
            None
        );
    }

    #[test]
    fn effective_list_falls_back_to_the_default_list() {
        let db = db();
        let work = create(&db, "工作").unwrap();
        let lists = list(&db).unwrap();
        assert_eq!(effective_list_id(Some(work.id), &lists), work.id);
        assert_eq!(
            effective_list_id(Some(DEFAULT_LIST_ID), &lists),
            DEFAULT_LIST_ID
        );
        assert_eq!(
            effective_list_id(None, &lists),
            DEFAULT_LIST_ID,
            "归属未知按默认清单显示"
        );
        assert_eq!(
            effective_list_id(Some(12345), &lists),
            DEFAULT_LIST_ID,
            "已删除 / 不存在"
        );
        assert_eq!(effective_list_id(Some(0), &lists), DEFAULT_LIST_ID);
        assert_eq!(
            effective_list_id(Some(DEFAULT_LIST_ID), &[]),
            DEFAULT_LIST_ID
        );
    }

    #[test]
    fn a_missing_default_list_is_recreated_on_read() {
        let db = db();
        db.with_connection(|c| c.execute("DELETE FROM todo_lists", []))
            .unwrap();
        let lists = list(&db).unwrap();
        assert_eq!(lists.len(), 1);
        assert_eq!(
            (
                lists[0].id,
                lists[0].name.as_str(),
                lists[0].updated_at.as_str()
            ),
            (DEFAULT_LIST_ID, "待办清单", "2000-01-01 00:00:00")
        );
        let base = seq(&db);
        list(&db).unwrap();
        assert_eq!(seq(&db), base, "已存在时不写库");
    }
}

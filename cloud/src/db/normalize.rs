//! 存量记录的一次性 K6 归一化（见 `crate::model::normalize_stored_todo`）。
//!
//! 旧版 API 不校验写入：缓存里可能有 `priority` / `dueDate` / `notes` 这类 PC 不认识的
//! 字段（PC 往返时被丢掉，AI 的意图就丢了），也可能有 `quadrant: "urgent_important"`、
//! `color: null` 这类让 PC 整条反序列化失败的值（两端从此永久分叉）。
//!
//! 运行时机：**第一次成功 pull 之后**（由 `sync::pull` 调用），而不是启动时立即运行——
//! 语义修复要刷新 `updatedAt`，先把远端最新内容合并进来，才不会用一次修复在 LWW 里压过
//! PC 已经同步上来的编辑。做完后在 meta 记一个标记，之后不再运行。
//!
//! - 语义修复（别名转换、类型修复）：刷新 `updatedAt` 并标脏，让结果传播给 PC
//! - 仅格式修正（时间格式、冗余键）：原地改写，不刷新 `updatedAt`、不标脏——否则一次纯
//!   格式修改会在 LWW 里压过 PC 还没同步上来的真实编辑

use chrono_tz::Tz;
use rusqlite::Connection;
use serde_json::{json, Value};
use tracing::warn;

use crate::db::repo::{self, meta_keys as mk};
use crate::model;
use crate::time::{bump_updated_at, now_local_string};

/// 一次归一化的统计。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NormalizeReport {
    /// 语义修复、已刷新 updatedAt 的 todo 数。
    pub todos_repaired: usize,
    /// 只修正了格式的 todo 数。
    pub todos_reformatted: usize,
    pub subtasks_repaired: usize,
    pub subtasks_reformatted: usize,
    /// data_json 不是 JSON 对象、无法处理的行。
    pub corrupt_rows: usize,
}

impl NormalizeReport {
    pub fn any(&self) -> bool {
        self.todos_repaired
            + self.todos_reformatted
            + self.subtasks_repaired
            + self.subtasks_reformatted
            + self.corrupt_rows
            > 0
    }
}

/// 还没做过就做一次（单事务）；已经做过返回 `Ok(None)`。
pub fn normalize_cache_once(
    conn: &mut Connection,
    tz: Tz,
) -> rusqlite::Result<Option<NormalizeReport>> {
    if repo::get_meta(conn, mk::CACHE_NORMALIZED)?.is_some() {
        return Ok(None);
    }
    let tx = conn.transaction()?;
    let report = normalize_all(&tx, tz)?;
    repo::set_meta(&tx, mk::CACHE_NORMALIZED, &now_local_string(tz))?;
    tx.commit()?;
    Ok(Some(report))
}

/// 归一化全部 todo / subtask。调用方负责事务。
pub fn normalize_all(conn: &Connection, tz: Tz) -> rusqlite::Result<NormalizeReport> {
    let now = now_local_string(tz);
    let mut report = NormalizeReport::default();

    for row in repo::all_todos(conn)? {
        let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(&row.data_json) else {
            warn!(target: "minitodo_cloud::model", "todo {}: data_json 不是 JSON 对象，跳过", row.id);
            report.corrupt_rows += 1;
            continue;
        };
        let previous = model::json_updated_at(&obj).map(str::to_string);
        let out = model::normalize_stored_todo(&mut obj, tz, &row.id, &row.updated_at);
        if out.semantic {
            // 修复后的版本必须严格晚于被修复的版本（见 time::bump_updated_at）
            let updated_at = bump_updated_at(
                &now,
                &[&row.updated_at, previous.as_deref().unwrap_or("")],
                tz,
            );
            obj.insert("updatedAt".into(), json!(updated_at));
            repo::upsert_todo(conn, &row.id, &Value::Object(obj).to_string(), &updated_at)?;
            report.todos_repaired += 1;
        } else if out.format {
            repo::upsert_todo(
                conn,
                &row.id,
                &Value::Object(obj).to_string(),
                &row.updated_at,
            )?;
            report.todos_reformatted += 1;
        }
    }

    for row in repo::all_subtasks(conn)? {
        let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(&row.data_json) else {
            warn!(target: "minitodo_cloud::model", "subtask {}: data_json 不是 JSON 对象，跳过", row.id);
            report.corrupt_rows += 1;
            continue;
        };
        let previous = model::json_updated_at(&obj).map(str::to_string);
        let out =
            model::normalize_stored_subtask(&mut obj, tz, &row.id, &row.todo_id, &row.updated_at);
        if out.semantic {
            let updated_at = bump_updated_at(
                &now,
                &[&row.updated_at, previous.as_deref().unwrap_or("")],
                tz,
            );
            obj.insert("updatedAt".into(), json!(updated_at));
            repo::upsert_subtask(
                conn,
                &row.id,
                &row.todo_id,
                &Value::Object(obj).to_string(),
                &updated_at,
            )?;
            report.subtasks_repaired += 1;
        } else if out.format {
            repo::upsert_subtask(
                conn,
                &row.id,
                &row.todo_id,
                &Value::Object(obj).to_string(),
                &row.updated_at,
            )?;
            report.subtasks_reformatted += 1;
        }
    }

    if report.todos_repaired + report.subtasks_repaired > 0 {
        repo::mark_dirty(conn)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;

    fn tz() -> Tz {
        "Asia/Shanghai".parse().unwrap()
    }

    fn fresh() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        schema::init(&c).unwrap();
        c
    }

    fn data(c: &Connection, id: &str) -> (Value, String) {
        let row = repo::get_todo(c, id).unwrap().unwrap();
        (
            serde_json::from_str(&row.data_json).unwrap(),
            row.updated_at,
        )
    }

    #[test]
    fn legacy_records_are_repaired_once_and_marked_dirty() {
        let mut c = fresh();
        let old = "2026-05-13 10:00:00";
        // 旧版 API 写的记录：priority / dueDate / notes + 坏类型
        repo::upsert_todo(
            &c,
            "1",
            r##"{"id":1,"title":"ai","priority":"high","color":"#10B981","dueDate":"2026-05-20","notes":"n","quadrant":"urgent_important","updatedAt":"2026-05-13 10:00:00"}"##,
            old,
        )
        .unwrap();
        // PC 形态、只有时间格式不规范
        repo::upsert_todo(
            &c,
            "2",
            r##"{"id":2,"title":"pc","color":"#F59E0B","notifyAt":"2026-05-20T09:00","updatedAt":"2026-05-13 10:00:00"}"##,
            old,
        )
        .unwrap();
        // 完全规范
        repo::upsert_todo(
            &c,
            "3",
            r#"{"id":3,"title":"ok","updatedAt":"2026-05-13 10:00:00"}"#,
            old,
        )
        .unwrap();
        repo::upsert_todo(&c, "4", "not json", old).unwrap();
        repo::upsert_subtask(
            &c,
            "11",
            "1",
            r#"{"id":11,"parentId":1,"title":"s","completed":"yes","updatedAt":"2026-05-13 10:00:00"}"#,
            old,
        )
        .unwrap();

        let report = normalize_cache_once(&mut c, tz()).unwrap().unwrap();
        assert_eq!(
            report,
            NormalizeReport {
                todos_repaired: 1,
                todos_reformatted: 1,
                subtasks_repaired: 1,
                subtasks_reformatted: 0,
                corrupt_rows: 1,
            }
        );

        let (t1, u1) = data(&c, "1");
        assert_eq!(t1["color"], model::COLOR_HIGH);
        assert_eq!(t1["endTime"], "2026-05-20 23:59:00");
        assert_eq!(t1["description"], "n");
        assert_eq!(t1["quadrant"], 1);
        assert!(t1.get("priority").is_none() && t1.get("dueDate").is_none());
        assert!(u1.as_str() > old, "语义修复刷新 updated_at");
        assert_eq!(t1["updatedAt"], json!(u1));

        let (t2, u2) = data(&c, "2");
        assert_eq!(t2["notifyAt"], "2026-05-20 09:00:00");
        assert_eq!(u2, old, "仅格式修正不刷新 updated_at");
        assert_eq!(t2["updatedAt"], old);

        let (_, u3) = data(&c, "3");
        assert_eq!(u3, old);

        let sub = repo::get_subtask(&c, "11").unwrap().unwrap();
        let s: Value = serde_json::from_str(&sub.data_json).unwrap();
        assert_eq!(s["completed"], true);
        assert!(sub.updated_at.as_str() > old);

        assert!(repo::is_dirty(&c).unwrap(), "语义修复要推送出去");

        // 只做一次
        repo::upsert_todo(&c, "5", r#"{"id":5,"title":"x","priority":"low"}"#, old).unwrap();
        assert_eq!(normalize_cache_once(&mut c, tz()).unwrap(), None);
        assert!(data(&c, "5").0.get("priority").is_some());
    }

    #[test]
    fn format_only_changes_do_not_mark_dirty() {
        let mut c = fresh();
        repo::upsert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"pc","endTime":"2026-05-20","updatedAt":"2026-05-13 10:00:00"}"#,
            "2026-05-13 10:00:00",
        )
        .unwrap();
        let report = normalize_cache_once(&mut c, tz()).unwrap().unwrap();
        assert_eq!(report.todos_reformatted, 1);
        assert!(!repo::is_dirty(&c).unwrap());
    }
}

//! 跨端契约 K3：把远端 sync-data 文档合并进本地 SQLite 缓存。
//!
//! 规则（PC 合并远端、cloud pull、cloud push 构造上传文档三处一致）：
//! 1. 记录级 LWW：比较**规范化后**的 `updatedAt` 字符串；平局保持本地版本不动。
//! 2. 墓碑：同 `(entityType, entityId)` 且 `deletedAt >= record.updatedAt` 的墓碑
//!    删除 / 压制该记录（比较对象是 LWW 胜出的那个版本）；todo 的墓碑连带删除其全部
//!    子任务。记录 `updatedAt > deletedAt`（删除后又被编辑）→ 记录保留。
//! 3. 只在一侧存在的记录：保留（并集），除非被墓碑压制。**不做"缺席即删除"**。
//!    兼容例外：远端文档**没有 `tombstones` 键**（旧版 PC 写的）且本地不 dirty 时，
//!    沿用旧的缺席清理。
//! 4. 墓碑集合：并集，同键取较大 `deletedAt`，按 30 天保留期过滤。
//!
//! 云端不解析 PC 记录的业务字段：记录 JSON 原样存进 `data_json`（只剥掉嵌套的
//! `subtasks`、把子任务的 `parentId` 对齐到外层 todo、把可解析的非规范时间串统一成
//! K1 规范格式——仅格式，不改语义，未知字段原样保留）。缺 id 的坏记录跳过并计数，
//! 不影响其它记录。

use std::collections::{HashMap, HashSet};

use chrono::NaiveTime;
use chrono_tz::Tz;
use rusqlite::Connection;
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::db::repo;
use crate::model::canonicalize_record_times;
use crate::sync::doc::{EntityType, RemoteDoc};
use crate::time::normalize_datetime;
use crate::util::id_string;

/// 一次合并的统计。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergeStats {
    pub todos_upserted: usize,
    pub todos_deleted: usize,
    pub subtasks_upserted: usize,
    pub subtasks_deleted: usize,
    /// 新增或推后了 `deletedAt` 的墓碑数。
    pub tombstones_applied: usize,
    /// 跳过的坏记录（缺 id / 不是对象）与非法墓碑。
    pub records_skipped: usize,
    /// 是否走了旧协议的缺席清理。
    pub legacy_cleanup: bool,
}

/// 把时间字符串规范化成可比较的形态；无法解析视为空串（比任何合法时间都旧）。
pub fn canon_ts(raw: Option<&str>, tz: Tz) -> String {
    raw.and_then(|s| normalize_datetime(s, tz, NaiveTime::MIN))
        .unwrap_or_default()
}

fn record_ts(v: &Value, tz: Tz) -> String {
    canon_ts(v.get("updatedAt").and_then(Value::as_str), tz)
}

/// 合并远端文档。调用方负责事务（传进来的 `conn` 应当是一个 `Transaction`），
/// 以便把"合并 + 记录基准"原子地提交。
pub fn apply_remote_doc(
    conn: &Connection,
    doc: &RemoteDoc,
    tz: Tz,
    retention_cutoff: &str,
) -> rusqlite::Result<MergeStats> {
    let mut stats = MergeStats::default();

    // ---- 1. 墓碑：本地 ∪ 远端，同键取较大 deletedAt；保留期外的远端墓碑不导入 ----
    let mut tombs: HashMap<(EntityType, String), String> = HashMap::new();
    for (typ, id, deleted_at) in repo::list_tombstones(conn)? {
        let Some(t) = EntityType::parse(&typ) else {
            continue;
        };
        let d = canon_ts(Some(&deleted_at), tz);
        if !d.is_empty() {
            tombs.insert((t, id), d);
        }
    }
    let (remote_tombs, invalid_tombs) = doc.tombstones(tz);
    if invalid_tombs > 0 {
        warn!(
            target: "minitodo_cloud::sync",
            "skip {} invalid remote tombstone(s)", invalid_tombs
        );
        stats.records_skipped += invalid_tombs;
    }
    for t in remote_tombs {
        if t.deleted_at.as_str() < retention_cutoff {
            continue;
        }
        let key = (t.entity_type, t.entity_id.clone());
        if tombs.get(&key).is_none_or(|old| t.deleted_at > *old) {
            repo::add_tombstone(conn, t.entity_type.as_str(), &t.entity_id, &t.deleted_at)?;
            tombs.insert(key, t.deleted_at);
            stats.tombstones_applied += 1;
        }
    }
    let suppressed = |typ: EntityType, id: &str, winner_ts: &str| {
        tombs
            .get(&(typ, id.to_string()))
            .is_some_and(|deleted_at| deleted_at.as_str() >= winner_ts)
    };

    // ---- 2. 记录级 LWW ----
    let mut local_todos: HashMap<String, String> = repo::todo_timestamps(conn)?
        .into_iter()
        .map(|(id, ts)| (id, canon_ts(Some(&ts), tz)))
        .collect();
    let mut local_subs: HashMap<String, String> = repo::subtask_timestamps(conn)?
        .into_iter()
        .map(|(id, (_, ts))| (id, canon_ts(Some(&ts), tz)))
        .collect();
    let mut seen_todos: HashSet<String> = HashSet::new();
    let mut seen_subs: HashSet<String> = HashSet::new();

    for todo in doc.todos() {
        let Some(todo_id) = id_string(todo).filter(|_| todo.is_object()) else {
            stats.records_skipped += 1;
            debug!(target: "minitodo_cloud::sync", "skip remote todo without id");
            continue;
        };
        seen_todos.insert(todo_id.clone());

        let remote_ts = record_ts(todo, tz);
        let local_ts = local_todos.get(&todo_id).cloned();
        // 平局保持本地（K3-1）
        let remote_wins = local_ts.as_ref().is_none_or(|l| remote_ts > *l);
        let winner_ts = if remote_wins {
            remote_ts.clone()
        } else {
            local_ts.clone().unwrap_or_default()
        };
        if suppressed(EntityType::Todo, &todo_id, &winner_ts) {
            // 远端版本连同子任务一起丢弃；本地副本在第 3 步清扫时删除
            continue;
        }
        if remote_wins {
            let mut body = todo.clone();
            if let Some(obj) = body.as_object_mut() {
                // 子任务单独存 subtasks 表；data_json 里不留过期的嵌套副本
                obj.remove("subtasks");
                // 缓存里的时间统一成规范格式（列表过滤 / 排序按字符串比较）
                canonicalize_record_times(obj, true, tz);
            }
            repo::upsert_todo(conn, &todo_id, &body.to_string(), &remote_ts)?;
            local_todos.insert(todo_id.clone(), remote_ts);
            stats.todos_upserted += 1;
        }

        let parent_num = todo_id.parse::<i64>().ok();
        let Some(subs) = todo.get("subtasks").and_then(Value::as_array) else {
            continue;
        };
        for sub in subs {
            let Some(sub_id) = id_string(sub).filter(|_| sub.is_object()) else {
                stats.records_skipped += 1;
                debug!(target: "minitodo_cloud::sync", "skip remote subtask without id");
                continue;
            };
            seen_subs.insert(sub_id.clone());
            let remote_ts = record_ts(sub, tz);
            let local_ts = local_subs.get(&sub_id).cloned();
            let remote_wins = local_ts.as_ref().is_none_or(|l| remote_ts > *l);
            let winner_ts = if remote_wins {
                remote_ts.clone()
            } else {
                local_ts.unwrap_or_default()
            };
            if suppressed(EntityType::Subtask, &sub_id, &winner_ts) || !remote_wins {
                continue;
            }
            let mut body = sub.clone();
            if let Some(obj) = body.as_object_mut() {
                if let Some(p) = parent_num {
                    // 归属以外层 todo 为准（嵌套关系就是父子关系）
                    obj.insert("parentId".into(), json!(p));
                }
                canonicalize_record_times(obj, false, tz);
            }
            repo::upsert_subtask(conn, &sub_id, &todo_id, &body.to_string(), &remote_ts)?;
            local_subs.insert(sub_id, remote_ts);
            stats.subtasks_upserted += 1;
        }
    }

    // ---- 3. 墓碑清扫：删除本地 updatedAt <= deletedAt 的记录 ----
    let mut todo_tombs: Vec<(&String, &String)> = Vec::new();
    let mut sub_tombs: Vec<(&String, &String)> = Vec::new();
    for ((typ, id), deleted_at) in &tombs {
        match typ {
            EntityType::Todo => todo_tombs.push((id, deleted_at)),
            EntityType::Subtask => sub_tombs.push((id, deleted_at)),
        }
    }
    todo_tombs.sort();
    sub_tombs.sort();
    for (id, deleted_at) in todo_tombs {
        if local_todos.get(id).is_some_and(|l| deleted_at >= l) {
            stats.subtasks_deleted += repo::delete_todo_with_children(conn, id)?;
            local_todos.remove(id);
            stats.todos_deleted += 1;
        }
    }
    for (id, deleted_at) in sub_tombs {
        if local_subs.get(id).is_some_and(|l| deleted_at >= l) && repo::delete_subtask(conn, id)? {
            stats.subtasks_deleted += 1;
        }
    }

    // ---- 4. 旧协议兼容：远端没有 tombstones 键且本地不 dirty → 缺席即删除 ----
    if !doc.has_tombstones_key() {
        // dirty 必须在事务内读：防止事务开始后 API handler 新建 todo 置 dirty，
        // 清理却仍按旧值执行。读失败按"可能有未推送的本地新建"处理——宁可留孤儿
        // （下次 pull 会再清），不可误删本地记录。
        let dirty = match repo::is_dirty(conn) {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    target: "minitodo_cloud::sync",
                    "读 meta.dirty 失败，本轮跳过旧协议孤儿清理: {}", e
                );
                true
            }
        };
        if !dirty {
            stats.todos_deleted += repo::delete_todos_not_in(conn, &seen_todos)?;
            stats.subtasks_deleted += repo::delete_subtasks_not_in(conn, &seen_subs)?;
            stats.legacy_cleanup = true;
        }
    }

    // ---- 5. 本地墓碑按保留期清理 ----
    repo::purge_tombstones_before(conn, retention_cutoff)?;

    Ok(stats)
}

/// 合并之后，远端文档是否仍缺少本地已有的内容（与 PC 端"需要上传"的判定一致）：
///
/// - 本地有、远端没有的 todo / subtask
/// - 本地版本比远端新（规范化后的 `updatedAt` 更大；平局视为相同）
/// - 本地墓碑（`deletedAt >= recheck_cutoff`）远端没有，或远端同键的 `deletedAt` 更早。
///   远端是旧协议文档（没有 `tombstones` 键，旧版 PC 写的）时不比较墓碑——旧版 PC
///   不认识墓碑，每次都会把它们丢掉，比较会导致每轮都重推。
///
/// 典型场景：别的写入方在忽略前置条件的服务端（nginx dav / Caddy webdav）上用自己的旧
/// 快照整包覆盖了云端刚写入的记录或墓碑。调用方（pull）为真时标脏，让 push 把并集写回去，
/// 不让任何一端的数据被静默丢掉。必须在 `apply_remote_doc` 之后、同一事务内调用。
pub fn remote_lacks_local_state(
    conn: &Connection,
    doc: &RemoteDoc,
    tz: Tz,
    recheck_cutoff: &str,
) -> rusqlite::Result<bool> {
    let mut remote_todos: HashMap<String, String> = HashMap::new();
    let mut remote_subs: HashMap<String, String> = HashMap::new();
    for todo in doc.todos() {
        let Some(id) = id_string(todo).filter(|_| todo.is_object()) else {
            continue;
        };
        remote_todos.insert(id, record_ts(todo, tz));
        for sub in todo
            .get("subtasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(sid) = id_string(sub).filter(|_| sub.is_object()) {
                remote_subs.insert(sid, record_ts(sub, tz));
            }
        }
    }
    let behind = |remote: &HashMap<String, String>, id: &str, local_ts: &str| {
        remote
            .get(id)
            .is_none_or(|remote_ts| canon_ts(Some(local_ts), tz) > *remote_ts)
    };
    // 只比较会被导出的记录（坏行 / 孤儿子任务永远到不了远端，算进去会每轮重推）
    let (local_todos, local_subs) = repo::exportable_timestamps(conn)?;
    for (id, ts) in local_todos {
        if behind(&remote_todos, &id, &ts) {
            debug!(target: "minitodo_cloud::sync", "remote lacks local todo {}", id);
            return Ok(true);
        }
    }
    for (id, ts) in local_subs {
        if behind(&remote_subs, &id, &ts) {
            debug!(target: "minitodo_cloud::sync", "remote lacks local subtask {}", id);
            return Ok(true);
        }
    }

    if !doc.has_tombstones_key() {
        return Ok(false);
    }
    // 墓碑按数值 id 比较：写出的墓碑 `entityId` 一律是整数（`Tombstone::to_value`），
    // 本地 id 不是整数的墓碑根本导不出去，算进来会让每次 pull 都判定"远端缺墓碑"而无限重推
    let (remote_tombs, _) = doc.tombstones(tz);
    let mut remote_deleted: HashMap<(EntityType, i64), String> = HashMap::new();
    for t in remote_tombs {
        let Ok(id) = t.entity_id.parse::<i64>() else {
            continue;
        };
        let entry = remote_deleted.entry((t.entity_type, id)).or_default();
        if t.deleted_at > *entry {
            *entry = t.deleted_at;
        }
    }
    for (typ, id, deleted_at) in repo::list_tombstones(conn)? {
        let Some(t) = EntityType::parse(&typ) else {
            continue;
        };
        // 与 `Tombstone::to_value` 的判定完全一致
        let Ok(num_id) = id.parse::<i64>() else {
            continue;
        };
        let local = canon_ts(Some(&deleted_at), tz);
        if local.is_empty() || local.as_str() < recheck_cutoff {
            continue;
        }
        if remote_deleted
            .get(&(t, num_id))
            .is_none_or(|remote| local > *remote)
        {
            debug!(target: "minitodo_cloud::sync", "remote lacks local tombstone {}:{}", typ, id);
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use tempfile::TempDir;

    const CUTOFF: &str = "2000-01-01 00:00:00";

    fn tz() -> Tz {
        "Asia/Shanghai".parse().unwrap()
    }

    fn fresh_db() -> (Db, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let db = Db::open(&tmp.path().join("data.db")).expect("open db");
        (db, tmp)
    }

    fn todo_value(id: i64, title: &str, updated_at: &str) -> Value {
        json!({"id": id, "title": title, "updatedAt": updated_at, "subtasks": []})
    }

    fn subtask_value(id: i64, parent_id: i64, title: &str, updated_at: &str) -> Value {
        json!({"id": id, "parentId": parent_id, "title": title, "updatedAt": updated_at})
    }

    /// 旧版 PC 写的文档：没有 tombstones 键。
    fn legacy_doc(todos: Vec<Value>) -> RemoteDoc {
        RemoteDoc::from_value(json!({"version": "4.0", "todos": todos})).unwrap()
    }

    /// 新协议文档：带 tombstones 键。
    fn doc(todos: Vec<Value>, tombstones: Vec<Value>) -> RemoteDoc {
        RemoteDoc::from_value(json!({"version": "4.0", "todos": todos, "tombstones": tombstones}))
            .unwrap()
    }

    fn merge(db: &Db, d: &RemoteDoc) -> MergeStats {
        merge_with_cutoff(db, d, CUTOFF)
    }

    fn merge_with_cutoff(db: &Db, d: &RemoteDoc, cutoff: &str) -> MergeStats {
        db.with_conn(|conn| {
            let tx = conn.transaction().unwrap();
            let stats = apply_remote_doc(&tx, d, tz(), cutoff).unwrap();
            tx.commit().unwrap();
            stats
        })
    }

    fn todo_title(db: &Db, id: &str) -> Option<String> {
        db.with_conn(|conn| {
            repo::get_todo(conn, id).unwrap().map(|row| {
                serde_json::from_str::<Value>(&row.data_json).unwrap()["title"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
        })
    }

    fn subtask_title(db: &Db, id: &str) -> Option<String> {
        db.with_conn(|conn| {
            repo::get_subtask(conn, id).unwrap().map(|row| {
                serde_json::from_str::<Value>(&row.data_json).unwrap()["title"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
        })
    }

    fn tombstones(db: &Db) -> Vec<(String, String, String)> {
        let mut v = db.with_conn(|c| repo::list_tombstones(c).unwrap());
        v.sort();
        v
    }

    // ---------------------------------------------------------------- 旧协议兼容

    #[test]
    fn legacy_cleanup_removes_remote_missing_todos_and_seq() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &legacy_doc(vec![
                todo_value(1, "留", "2026-01-01 10:00:00"),
                todo_value(2, "删", "2026-01-01 10:00:00"),
            ]),
        );
        db.with_conn(|conn| repo::assign_seq(conn, "2").unwrap());

        let stats = merge(
            &db,
            &legacy_doc(vec![todo_value(1, "留", "2026-01-01 10:00:00")]),
        );
        assert!(stats.legacy_cleanup);
        assert!(todo_title(&db, "1").is_some());
        assert!(todo_title(&db, "2").is_none());
        db.with_conn(|conn| assert_eq!(repo::get_seq(conn, "2").unwrap(), None));
    }

    #[test]
    fn legacy_cleanup_removes_orphan_subtasks() {
        let (db, _tmp) = fresh_db();
        let mut t = todo_value(1, "父", "2026-01-01 10:00:00");
        t["subtasks"] = json!([
            subtask_value(11, 1, "留", "2026-01-01 10:00:00"),
            subtask_value(12, 1, "删", "2026-01-01 10:00:00"),
        ]);
        merge(&db, &legacy_doc(vec![t]));

        let mut t2 = todo_value(1, "父", "2026-01-01 10:00:00");
        t2["subtasks"] = json!([subtask_value(11, 1, "留", "2026-01-01 10:00:00")]);
        merge(&db, &legacy_doc(vec![t2]));

        assert!(subtask_title(&db, "11").is_some());
        assert!(subtask_title(&db, "12").is_none());
    }

    /// dirty=true 表示 cloud API 有本地新建还没 push 的记录，此时跳过清理。
    #[test]
    fn legacy_cleanup_skipped_when_dirty() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &legacy_doc(vec![
                todo_value(1, "A", "2026-01-01 10:00:00"),
                todo_value(2, "本地新建", "2026-01-01 10:00:00"),
            ]),
        );
        db.with_conn(|conn| repo::set_meta(conn, "dirty", "true").unwrap());

        let stats = merge(
            &db,
            &legacy_doc(vec![todo_value(1, "A", "2026-01-01 10:00:00")]),
        );
        assert!(!stats.legacy_cleanup);
        assert!(todo_title(&db, "2").is_some(), "dirty 时不得清理本地记录");
    }

    /// 读 dirty 出错（DROP TABLE meta 注入 DB 错误）时同样跳过清理。
    #[test]
    fn legacy_cleanup_skipped_when_dirty_read_fails() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &legacy_doc(vec![
                todo_value(1, "A", "2026-01-01 10:00:00"),
                todo_value(2, "本地新建", "2026-01-01 10:00:00"),
            ]),
        );
        db.with_conn(|conn| conn.execute_batch("DROP TABLE meta"))
            .unwrap();

        merge(
            &db,
            &legacy_doc(vec![todo_value(1, "A", "2026-01-01 10:00:00")]),
        );
        assert!(
            todo_title(&db, "2").is_some(),
            "读 dirty 失败时不得清理本地记录"
        );
    }

    /// 新协议（带 tombstones 键）：缺席绝不等于删除。
    #[test]
    fn absence_does_not_delete_with_new_protocol() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(
                vec![
                    todo_value(1, "A", "2026-01-01 10:00:00"),
                    todo_value(2, "B", "2026-01-01 10:00:00"),
                ],
                vec![],
            ),
        );
        let stats = merge(
            &db,
            &doc(vec![todo_value(1, "A", "2026-01-01 10:00:00")], vec![]),
        );
        assert!(!stats.legacy_cleanup);
        assert_eq!(stats.todos_deleted, 0);
        assert!(todo_title(&db, "2").is_some());
    }

    // ---------------------------------------------------------------- LWW

    #[test]
    fn lww_keeps_newer_local() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(
                vec![todo_value(1, "本地较新", "2026-01-05 10:00:00")],
                vec![],
            ),
        );
        merge(
            &db,
            &doc(
                vec![todo_value(1, "远端较旧", "2026-01-02 10:00:00")],
                vec![],
            ),
        );
        assert_eq!(todo_title(&db, "1").as_deref(), Some("本地较新"));
    }

    #[test]
    fn lww_applies_newer_remote() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(vec![todo_value(1, "旧", "2026-01-01 10:00:00")], vec![]),
        );
        let stats = merge(
            &db,
            &doc(vec![todo_value(1, "新", "2026-01-03 10:00:00")], vec![]),
        );
        assert_eq!(stats.todos_upserted, 1);
        assert_eq!(todo_title(&db, "1").as_deref(), Some("新"));
    }

    /// K3-1：平局保持本地版本不动（旧实现 `>=` 会被远端覆盖）。
    #[test]
    fn lww_tie_keeps_local() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(vec![todo_value(1, "本地", "2026-01-01 10:00:00")], vec![]),
        );
        let stats = merge(
            &db,
            &doc(vec![todo_value(1, "远端", "2026-01-01 10:00:00")], vec![]),
        );
        assert_eq!(stats.todos_upserted, 0);
        assert_eq!(todo_title(&db, "1").as_deref(), Some("本地"));
    }

    /// 比较的是规范化后的时间：`T` 与空格混用不影响胜负。
    #[test]
    fn lww_compares_normalized_timestamps() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(vec![todo_value(1, "本地", "2026-01-01 10:00:00")], vec![]),
        );
        // 字节比较 "2026-01-01T09:00:00" > "2026-01-01 10:00:00"，规范化后它更旧
        merge(
            &db,
            &doc(vec![todo_value(1, "远端旧", "2026-01-01T09:00:00")], vec![]),
        );
        assert_eq!(todo_title(&db, "1").as_deref(), Some("本地"));
        merge(
            &db,
            &doc(vec![todo_value(1, "远端新", "2026-01-01T11:00")], vec![]),
        );
        assert_eq!(todo_title(&db, "1").as_deref(), Some("远端新"));
        let row = db.with_conn(|c| repo::get_todo(c, "1").unwrap().unwrap());
        assert_eq!(
            row.updated_at, "2026-01-01 11:00:00",
            "updated_at 列存规范格式"
        );
    }

    #[test]
    fn subtasks_lww_and_union() {
        let (db, _tmp) = fresh_db();
        let mut t = todo_value(1, "p", "2026-05-13 10:00:00");
        t["subtasks"] = json!([
            subtask_value(1, 1, "old", "2026-05-13 10:00:00"),
            subtask_value(2, 1, "remote-only", "2026-05-13 10:00:00"),
        ]);
        merge(&db, &doc(vec![t], vec![]));
        // 本地改了 1，远端再来一个更旧的 1 + 一个新的 3
        db.with_conn(|c| {
            repo::upsert_subtask(
                c,
                "1",
                "1",
                r#"{"id":1,"parentId":1,"title":"new","updatedAt":"2026-05-13 11:00:00"}"#,
                "2026-05-13 11:00:00",
            )
            .unwrap()
        });
        let mut t2 = todo_value(1, "p", "2026-05-13 10:00:00");
        t2["subtasks"] = json!([
            subtask_value(1, 1, "stale", "2026-05-13 10:30:00"),
            subtask_value(3, 1, "another", "2026-05-13 10:00:00"),
        ]);
        merge(&db, &doc(vec![t2], vec![]));
        assert_eq!(subtask_title(&db, "1").as_deref(), Some("new"));
        assert_eq!(subtask_title(&db, "2").as_deref(), Some("remote-only"));
        assert_eq!(subtask_title(&db, "3").as_deref(), Some("another"));
    }

    #[test]
    fn subtask_parent_follows_outer_todo_and_data_json_has_no_nested_subtasks() {
        let (db, _tmp) = fresh_db();
        let mut t = todo_value(5, "p", "2026-05-13 10:00:00");
        t["subtasks"] = json!([subtask_value(9, 999, "s", "2026-05-13 10:00:00")]);
        merge(&db, &doc(vec![t], vec![]));
        let (todo_row, sub_row) = db.with_conn(|c| {
            (
                repo::get_todo(c, "5").unwrap().unwrap(),
                repo::get_subtask(c, "9").unwrap().unwrap(),
            )
        });
        assert_eq!(sub_row.todo_id, "5");
        let sub: Value = serde_json::from_str(&sub_row.data_json).unwrap();
        assert_eq!(sub["parentId"], json!(5));
        let todo: Value = serde_json::from_str(&todo_row.data_json).unwrap();
        assert!(todo.get("subtasks").is_none());
    }

    /// 坏记录跳过计数，不影响其它记录，也绝不当作删除。
    #[test]
    fn bad_records_are_skipped_without_affecting_others() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(vec![todo_value(1, "keep", "2026-01-01 10:00:00")], vec![]),
        );
        let mut good = todo_value(2, "good", "2026-01-01 10:00:00");
        good["subtasks"] =
            json!([{"title": "no id"}, subtask_value(21, 2, "ok", "2026-01-01 10:00:00")]);
        let stats = merge(
            &db,
            &doc(
                vec![json!({"title": "no id"}), json!("garbage"), good],
                vec![
                    json!({"entityType": "todo", "entityId": "x", "deletedAt": "2026-01-01 10:00:00"}),
                ],
            ),
        );
        assert_eq!(stats.records_skipped, 4);
        assert!(todo_title(&db, "1").is_some());
        assert!(todo_title(&db, "2").is_some());
        assert!(subtask_title(&db, "21").is_some());
    }

    // ---------------------------------------------------------------- 墓碑

    #[test]
    fn remote_tombstone_deletes_local_record_and_is_stored() {
        let (db, _tmp) = fresh_db();
        let mut t = todo_value(1, "A", "2026-05-13 10:00:00");
        t["subtasks"] = json!([subtask_value(11, 1, "child", "2026-05-13 10:00:00")]);
        merge(
            &db,
            &doc(vec![t, todo_value(2, "B", "2026-05-13 10:00:00")], vec![]),
        );
        db.with_conn(|conn| repo::assign_seq(conn, "1").unwrap());

        let stats = merge(
            &db,
            &doc(
                vec![todo_value(2, "B", "2026-05-13 10:00:00")],
                vec![
                    json!({"entityType": "todo", "entityId": 1, "deletedAt": "2026-05-13T11:00:00"}),
                ],
            ),
        );
        assert_eq!(stats.todos_deleted, 1);
        assert_eq!(stats.subtasks_deleted, 1, "todo 墓碑连带删除子任务");
        assert_eq!(stats.tombstones_applied, 1);
        assert!(todo_title(&db, "1").is_none());
        assert!(subtask_title(&db, "11").is_none());
        assert!(todo_title(&db, "2").is_some());
        db.with_conn(|conn| assert_eq!(repo::get_seq(conn, "1").unwrap(), None));
        assert_eq!(
            tombstones(&db),
            vec![("todo".into(), "1".into(), "2026-05-13 11:00:00".into())]
        );
    }

    #[test]
    fn remote_record_suppressed_by_local_tombstone() {
        let (db, _tmp) = fresh_db();
        db.with_conn(|c| repo::add_tombstone(c, "todo", "1", "2026-05-13 11:00:00").unwrap());
        let mut t = todo_value(1, "陈旧副本", "2026-05-13 10:00:00");
        t["subtasks"] = json!([subtask_value(11, 1, "child", "2026-05-13 12:00:00")]);
        merge(&db, &doc(vec![t], vec![]));
        assert!(todo_title(&db, "1").is_none(), "墓碑压制远端陈旧副本");
        assert!(
            subtask_title(&db, "11").is_none(),
            "被压制 todo 的子任务一起丢弃"
        );
    }

    /// 删除后又被编辑（updatedAt > deletedAt）→ 记录保留（墓碑不再无条件胜出）。
    #[test]
    fn edit_after_delete_survives_tombstone() {
        let (db, _tmp) = fresh_db();
        db.with_conn(|c| repo::add_tombstone(c, "todo", "1", "2026-05-13 11:00:00").unwrap());
        merge(
            &db,
            &doc(vec![todo_value(1, "复活", "2026-05-13 12:00:00")], vec![]),
        );
        assert_eq!(todo_title(&db, "1").as_deref(), Some("复活"));

        // 反方向：本地较新的编辑也不被远端更早的墓碑删掉
        let (db2, _tmp2) = fresh_db();
        merge(
            &db2,
            &doc(
                vec![todo_value(3, "本地编辑", "2026-05-13 12:00:00")],
                vec![],
            ),
        );
        merge(
            &db2,
            &doc(
                vec![],
                vec![
                    json!({"entityType": "todo", "entityId": 3, "deletedAt": "2026-05-13 11:00:00"}),
                ],
            ),
        );
        assert_eq!(todo_title(&db2, "3").as_deref(), Some("本地编辑"));
    }

    /// 墓碑与 LWW 胜出版本比较：远端更新但仍早于墓碑 → 删除。
    #[test]
    fn tombstone_compares_with_lww_winner() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(vec![todo_value(1, "v1", "2026-05-13 09:00:00")], vec![]),
        );
        merge(
            &db,
            &doc(
                vec![todo_value(1, "v2", "2026-05-13 10:00:00")],
                vec![
                    json!({"entityType": "todo", "entityId": 1, "deletedAt": "2026-05-13 10:00:00"}),
                ],
            ),
        );
        assert!(
            todo_title(&db, "1").is_none(),
            "deletedAt >= updatedAt（含相等）即删除"
        );
    }

    #[test]
    fn subtask_tombstone_deletes_only_that_subtask() {
        let (db, _tmp) = fresh_db();
        let mut t = todo_value(1, "p", "2026-05-13 10:00:00");
        t["subtasks"] = json!([
            subtask_value(11, 1, "a", "2026-05-13 10:00:00"),
            subtask_value(12, 1, "b", "2026-05-13 10:00:00"),
        ]);
        merge(&db, &doc(vec![t.clone()], vec![]));
        let stats = merge(
            &db,
            &doc(
                vec![t],
                vec![
                    json!({"entityType": "subtask", "entityId": 11, "deletedAt": "2026-05-13 10:30:00"}),
                ],
            ),
        );
        assert_eq!(stats.subtasks_deleted, 1);
        assert!(subtask_title(&db, "11").is_none());
        assert!(subtask_title(&db, "12").is_some());
        assert!(todo_title(&db, "1").is_some());
    }

    fn lacks(db: &Db, d: &RemoteDoc, recheck_cutoff: &str) -> bool {
        db.with_conn(|c| remote_lacks_local_state(c, d, tz(), recheck_cutoff).unwrap())
    }

    #[test]
    fn remote_lacks_local_state_detects_missing_records_versions_and_tombstones() {
        let (db, _tmp) = fresh_db();
        let mut t = todo_value(1, "p", "2026-05-13 10:00:00");
        t["subtasks"] = json!([subtask_value(11, 1, "s", "2026-05-13 10:00:00")]);
        let full = doc(vec![t.clone()], vec![]);
        merge(&db, &full);
        assert!(!lacks(&db, &full, CUTOFF), "完全一致");

        // 子任务缺失
        let mut no_sub = t.clone();
        no_sub["subtasks"] = json!([]);
        assert!(lacks(&db, &doc(vec![no_sub], vec![]), CUTOFF));
        // 远端版本更旧（T 格式规范化后比较）；平局不算落后
        let mut older = t.clone();
        older["updatedAt"] = json!("2026-05-13T09:00:00");
        assert!(lacks(&db, &doc(vec![older], vec![]), CUTOFF));
        let mut tie = t.clone();
        tie["updatedAt"] = json!("2026-05-13T10:00");
        assert!(!lacks(&db, &doc(vec![tie], vec![]), CUTOFF));

        // 本地墓碑：远端没有 / 远端更早 → 落后；远端相同或更晚 → 不落后
        db.with_conn(|c| repo::add_tombstone(c, "todo", "9", "2026-05-13 12:00:00").unwrap());
        assert!(lacks(&db, &doc(vec![t.clone()], vec![]), CUTOFF));
        let tomb = |d: &str| json!({"entityType": "todo", "entityId": 9, "deletedAt": d});
        assert!(lacks(
            &db,
            &doc(vec![t.clone()], vec![tomb("2026-05-13 11:00:00")]),
            CUTOFF
        ));
        assert!(!lacks(
            &db,
            &doc(vec![t.clone()], vec![tomb("2026-05-13 12:00:00")]),
            CUTOFF
        ));
        // 快过期的本地墓碑不要求远端保留（各端时钟不同步时避免每轮重推）
        assert!(!lacks(
            &db,
            &doc(vec![t.clone()], vec![]),
            "2026-05-14 00:00:00"
        ));
        // 旧协议文档不比较墓碑
        assert!(!lacks(&db, &legacy_doc(vec![t.clone()]), CUTOFF));
        // 导不出去的孤儿子任务 / 坏行不算（否则每轮 pull 都会重推）
        db.with_conn(|c| {
            repo::purge_tombstones_before(c, "9999-01-01 00:00:00").unwrap();
            repo::upsert_subtask(c, "99", "404", r#"{"id":99}"#, "2026-05-13 10:00:00").unwrap();
            repo::upsert_todo(c, "8", "corrupt", "2026-05-13 10:00:00").unwrap();
        });
        assert!(!lacks(&db, &doc(vec![t], vec![]), CUTOFF));
    }

    /// 本地 id 不是整数的墓碑导不出去（`Tombstone::to_value` 只写整数 entityId）：不能因此判定
    /// 远端落后，否则每次拿到新文档都会标脏重推，永不收敛。数值 id 照常比较（远端字符串 "7"
    /// 与本地 7 是同一条）。
    #[test]
    fn unexportable_local_tombstones_do_not_trigger_repush() {
        let (db, _tmp) = fresh_db();
        let t = todo_value(1, "p", "2026-05-13 10:00:00");
        let full = doc(vec![t.clone()], vec![]);
        merge(&db, &full);
        db.with_conn(|c| repo::add_tombstone(c, "todo", "abc", "2026-05-13 12:00:00").unwrap());
        assert!(!lacks(&db, &full, CUTOFF), "导不出去的墓碑不算远端缺失");
        let out = db.with_conn(|c| {
            crate::sync::doc::build_outgoing_doc(c, &serde_json::Map::new(), vec![], tz(), CUTOFF)
                .unwrap()
        });
        assert_eq!(out["tombstones"], json!([]), "确实不会写出");

        db.with_conn(|c| repo::add_tombstone(c, "subtask", "7", "2026-05-13 12:00:00").unwrap());
        assert!(lacks(&db, &full, CUTOFF));
        let with_tomb = doc(
            vec![t],
            vec![
                json!({"entityType": "subtask", "entityId": "7", "deletedAt": "2026-05-13T12:00:00"}),
            ],
        );
        assert!(!lacks(&db, &with_tomb, CUTOFF));
    }

    /// 墓碑集合：同键取较大 deletedAt；保留期外的远端墓碑不导入，本地过期墓碑被清理。
    #[test]
    fn tombstone_union_takes_max_and_respects_retention() {
        let (db, _tmp) = fresh_db();
        db.with_conn(|c| {
            repo::add_tombstone(c, "todo", "1", "2026-09-20 10:00:00").unwrap();
            repo::add_tombstone(c, "todo", "2", "2026-09-25 10:00:00").unwrap();
            repo::add_tombstone(c, "todo", "3", "2026-08-01 10:00:00").unwrap();
            // 本地过期
        });
        merge_with_cutoff(
            &db,
            &doc(
                vec![],
                vec![
                    json!({"entityType": "todo", "entityId": 1, "deletedAt": "2026-09-21 10:00:00"}),
                    json!({"entityType": "todo", "entityId": 2, "deletedAt": "2026-09-24 10:00:00"}),
                    json!({"entityType": "subtask", "entityId": 4, "deletedAt": "2026-08-15 10:00:00"}),
                    json!({"entityType": "subtask", "entityId": 5, "deletedAt": "2026-09-30 10:00:00"}),
                ],
            ),
            "2026-09-01 00:00:00",
        );
        assert_eq!(
            tombstones(&db),
            vec![
                ("subtask".into(), "5".into(), "2026-09-30 10:00:00".into()),
                ("todo".into(), "1".into(), "2026-09-21 10:00:00".into()),
                ("todo".into(), "2".into(), "2026-09-25 10:00:00".into()),
            ]
        );
    }
}

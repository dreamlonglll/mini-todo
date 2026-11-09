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
//! 清单（`todoLists` / `listTombstones`，契约 `.trellis/tasks/11-09-multi-list-windows/contract.md`
//! §4）与 todo 完全同构：记录级 LWW（平局保留本地）、`deletedAt >= updatedAt` 压制、并集（远端
//! 没有 `todoLists` 键时本地清单一条都不删）、墓碑并集取较大值并按同一保留期清理；默认清单
//! （id 1）的墓碑一律忽略。收到清单墓碑时**不**改写本地待办（读侧按"有效清单"兜底显示在默认
//! 清单，见 `db::repo` 的有效清单过滤）。
//!
//! 待办的 `listId`（契约 §12 修订 1）：缺失或 `null` = **未知**（写入方不认识清单，旧版本每次
//! 上传都会丢掉全部 `listId`），不等于默认清单。远端胜出但未知 → 沿用本地已知的值；远端已知、
//! 本地未知 → 不论 LWW 谁胜都采纳远端的值（不改 `updatedAt`、不标脏）。未知永远不会覆盖已知。
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
use crate::model::{
    canonicalize_record_times, is_pristine_default_list, list_id_state, ListIdState,
    DEFAULT_LIST_ID,
};
use crate::sync::doc::{list_record_id, EntityType, RemoteDoc};
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
    pub lists_upserted: usize,
    pub lists_deleted: usize,
    /// 新增或推后了 `deletedAt` 的清单墓碑数。
    pub list_tombstones_applied: usize,
    /// 远端胜出但缺 `listId`、沿用了本地已知值的待办数（契约 §12-2）。
    pub list_ids_carried: usize,
    /// 本地未知、采纳了远端 `listId` 的待办数（契约 §12-3，不改 updatedAt）。
    pub list_ids_adopted: usize,
    /// 跳过的坏记录（缺 id / 不是对象）与非法墓碑（含清单与清单墓碑）。
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
    // 本地每条 todo 的 listId 状态（契约 §12：缺失 / null = 未知）
    let local_list_ids = repo::todo_list_id_states(conn)?;

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
        let local_list = local_list_ids.get(&todo_id).copied();
        if remote_wins {
            let mut body = todo.clone();
            if let Some(obj) = body.as_object_mut() {
                // 子任务单独存 subtasks 表；data_json 里不留过期的嵌套副本
                obj.remove("subtasks");
                // 缓存里的时间统一成规范格式（列表过滤 / 排序按字符串比较）
                canonicalize_record_times(obj, true, tz);
                // §12-2：远端胜出但不认识清单（缺 listId / null）→ 沿用本地已知的 listId；
                // 本地也未知 / 本地没有这条 → 仍是未知（统一成缺失）
                if list_id_state(obj) == ListIdState::Unknown {
                    match local_list {
                        Some(ListIdState::Known(list_id)) => {
                            obj.insert("listId".into(), json!(list_id));
                            stats.list_ids_carried += 1;
                        }
                        _ => {
                            obj.remove("listId");
                        }
                    }
                }
            }
            repo::upsert_todo(conn, &todo_id, &body.to_string(), &remote_ts)?;
            local_todos.insert(todo_id.clone(), remote_ts);
            stats.todos_upserted += 1;
        } else if let (Some(ListIdState::Unknown), Some(obj)) = (local_list, todo.as_object()) {
            // §12-3：本地胜出（含平局、本地更新）但本地不认识清单，远端已知 → 采纳远端的 listId，
            // 不改 updatedAt（不是一次新的编辑，也不标脏：远端本来就有这个值）
            if let ListIdState::Known(list_id) = list_id_state(obj) {
                if repo::set_todo_list_id(conn, &todo_id, list_id)? {
                    stats.list_ids_adopted += 1;
                }
            }
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

    // ---- 6. 清单 ----
    merge_lists(conn, doc, tz, retention_cutoff, &mut stats)?;

    Ok(stats)
}

/// 合并远端清单（契约 §4，与 todo 同构；见模块注释）。
fn merge_lists(
    conn: &Connection,
    doc: &RemoteDoc,
    tz: Tz,
    retention_cutoff: &str,
    stats: &mut MergeStats,
) -> rusqlite::Result<()> {
    let default_id = DEFAULT_LIST_ID.to_string();
    // 默认清单不可删除；本地没有时按固定初始值补建（契约 §2）
    repo::ensure_default_list(conn)?;

    // ---- 清单墓碑：本地 ∪ 远端，同键取较大 deletedAt；保留期外的远端墓碑不导入 ----
    let mut tombs: HashMap<String, String> = HashMap::new();
    for (id, deleted_at) in repo::all_list_tombstones(conn)? {
        let d = canon_ts(Some(&deleted_at), tz);
        if id != default_id && !d.is_empty() {
            tombs.insert(id, d);
        }
    }
    let (remote_tombs, invalid) = doc.list_tombstones(tz);
    if invalid > 0 {
        warn!(
            target: "minitodo_cloud::sync",
            "skip {} invalid remote list tombstone(s)", invalid
        );
        stats.records_skipped += invalid;
    }
    for t in remote_tombs {
        if t.deleted_at.as_str() < retention_cutoff {
            continue;
        }
        if tombs
            .get(&t.entity_id)
            .is_none_or(|old| t.deleted_at > *old)
        {
            repo::add_list_tombstone(conn, &t.entity_id, &t.deleted_at)?;
            tombs.insert(t.entity_id, t.deleted_at);
            stats.list_tombstones_applied += 1;
        }
    }

    // ---- 记录级 LWW（平局保留本地；墓碑与 LWW 胜出的版本比较） ----
    let mut local: HashMap<String, String> = repo::list_timestamps(conn)?
        .into_iter()
        .map(|(id, ts)| (id, canon_ts(Some(&ts), tz)))
        .collect();
    for list in doc.lists() {
        let Some(id) = list_record_id(list) else {
            stats.records_skipped += 1;
            debug!(target: "minitodo_cloud::sync", "skip remote list without an integer id");
            continue;
        };
        let remote_ts = record_ts(list, tz);
        let local_ts = local.get(&id).cloned();
        let remote_wins = local_ts.as_ref().is_none_or(|l| remote_ts > *l);
        let winner_ts = if remote_wins {
            remote_ts.clone()
        } else {
            local_ts.unwrap_or_default()
        };
        let suppressed = id != default_id
            && tombs
                .get(&id)
                .is_some_and(|deleted_at| deleted_at.as_str() >= winner_ts.as_str());
        if suppressed || !remote_wins {
            // 被压制的远端版本丢弃；本地副本在下面清扫时删除
            continue;
        }
        let mut body = list.clone();
        if let Some(obj) = body.as_object_mut() {
            // 只有 createdAt / updatedAt 两个时间字段：与子任务同一套仅格式的规范化
            canonicalize_record_times(obj, false, tz);
        }
        repo::upsert_list(conn, &id, &body.to_string(), &remote_ts)?;
        local.insert(id, remote_ts);
        stats.lists_upserted += 1;
    }

    // ---- 墓碑清扫：删除本地 updatedAt <= deletedAt 的清单（待办不改写，读侧兜底） ----
    let mut sweep: Vec<(&String, &String)> = tombs.iter().collect();
    sweep.sort();
    for (id, deleted_at) in sweep {
        if local.get(id).is_some_and(|l| deleted_at >= l) && repo::delete_list(conn, id)? {
            stats.lists_deleted += 1;
        }
    }

    repo::purge_list_tombstones_before(conn, retention_cutoff)?;
    Ok(())
}

/// 合并之后，远端文档是否仍缺少本地已有的内容（与 PC 端"需要上传"的判定一致）：
///
/// - 本地有、远端没有的 todo / subtask
/// - 本地版本比远端新（规范化后的 `updatedAt` 更大；平局视为相同）
/// - 本地墓碑（`deletedAt >= recheck_cutoff`）远端没有，或远端同键的 `deletedAt` 更早。
///   远端是旧协议文档（没有 `tombstones` 键，旧版 PC 写的）时不比较墓碑——旧版 PC
///   不认识墓碑，每次都会把它们丢掉，比较会导致每轮都重推。
/// - 清单与清单墓碑按同样的规则比较（远端没有 `todoLists` 键时本地的每个清单都算缺失）；
///   还是初始值的默认清单不算（每个写入方都会按同样的值补建它）。旧协议文档同样不比较清单：
///   写它的旧版 PC 也不会原样带回不认识的顶层键，比较会导致每轮都重推。
/// - 远端待办缺 `listId`（未知）而本地同一条已知（契约 §12-8）；旧协议文档同样不比较。
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
    if remote_lacks_local_list_ids(conn, doc)? {
        return Ok(true);
    }
    if remote_lacks_local_lists(conn, doc, tz, recheck_cutoff)? {
        return Ok(true);
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

/// 远端某条待办缺 `listId`（未知）、本地同一条已知（任何值，含 1）→ 远端缺少本地内容
/// （契约 §12-8 的 cloud 对应：补传一次修复文档）。上传时本地的已知值随记录写出，之后远端就不再缺。
fn remote_lacks_local_list_ids(conn: &Connection, doc: &RemoteDoc) -> rusqlite::Result<bool> {
    let local = repo::todo_list_id_states(conn)?;
    for todo in doc.todos() {
        let (Some(id), Some(obj)) = (id_string(todo), todo.as_object()) else {
            continue;
        };
        if list_id_state(obj) == ListIdState::Unknown
            && matches!(local.get(&id), Some(ListIdState::Known(_)))
        {
            debug!(target: "minitodo_cloud::sync", "remote todo {} lacks the local listId", id);
            return Ok(true);
        }
    }
    Ok(false)
}

/// `remote_lacks_local_state` 的清单部分（规则见那里）。
fn remote_lacks_local_lists(
    conn: &Connection,
    doc: &RemoteDoc,
    tz: Tz,
    recheck_cutoff: &str,
) -> rusqlite::Result<bool> {
    let remote_lists: HashMap<String, String> = doc
        .lists()
        .iter()
        .filter_map(|l| list_record_id(l).map(|id| (id, record_ts(l, tz))))
        .collect();
    for (id, ts) in repo::exportable_list_timestamps(conn)? {
        let local = canon_ts(Some(&ts), tz);
        if is_pristine_default_list(&id, &local) {
            continue;
        }
        if remote_lists
            .get(&id)
            .is_none_or(|remote_ts| local > *remote_ts)
        {
            debug!(target: "minitodo_cloud::sync", "remote lacks local list {}", id);
            return Ok(true);
        }
    }

    let (remote_tombs, _) = doc.list_tombstones(tz);
    let mut remote_deleted: HashMap<String, String> = HashMap::new();
    for t in remote_tombs {
        let entry = remote_deleted.entry(t.entity_id).or_default();
        if t.deleted_at > *entry {
            *entry = t.deleted_at;
        }
    }
    for (id, deleted_at) in repo::all_list_tombstones(conn)? {
        // 与 `ListTombstone::to_value` 的判定一致：导不出去的墓碑不算
        if id.parse::<i64>().is_err() {
            continue;
        }
        let local = canon_ts(Some(&deleted_at), tz);
        if local.is_empty() || local.as_str() < recheck_cutoff {
            continue;
        }
        if remote_deleted.get(&id).is_none_or(|remote| local > *remote) {
            debug!(target: "minitodo_cloud::sync", "remote lacks local list tombstone {}", id);
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

    // ---------------------------------------------------------------- 清单（契约 §4）

    fn list_value(id: i64, name: &str, updated_at: &str) -> Value {
        json!({"id": id, "name": name, "sortOrder": 1, "createdAt": "2026-11-01 08:00:00",
               "updatedAt": updated_at})
    }

    fn lists_doc(lists: Vec<Value>, list_tombs: Vec<Value>) -> RemoteDoc {
        RemoteDoc::from_value(json!({
            "version": "4.0", "todos": [], "tombstones": [],
            "todoLists": lists, "listTombstones": list_tombs
        }))
        .unwrap()
    }

    fn list_tomb(id: i64, deleted_at: &str) -> Value {
        json!({"entityId": id, "deletedAt": deleted_at})
    }

    fn list_name(db: &Db, id: &str) -> Option<String> {
        db.with_conn(|conn| {
            repo::all_list_rows(conn)
                .unwrap()
                .into_iter()
                .find(|r| r.id == id)
                .map(|r| {
                    serde_json::from_str::<Value>(&r.data_json).unwrap()["name"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
        })
    }

    fn list_tombstones(db: &Db) -> Vec<(String, String)> {
        let mut v = db.with_conn(|c| repo::all_list_tombstones(c).unwrap());
        v.sort();
        v
    }

    #[test]
    fn lists_lww_union_and_tie_keeps_local() {
        let (db, _tmp) = fresh_db();
        let stats = merge(
            &db,
            &lists_doc(vec![list_value(5, "工作", "2026-11-01 10:00:00")], vec![]),
        );
        assert_eq!(stats.lists_upserted, 1);
        assert_eq!(list_name(&db, "5").as_deref(), Some("工作"));

        // 远端更旧 / 平局：保留本地
        let stats = merge(
            &db,
            &lists_doc(vec![list_value(5, "旧名", "2026-11-01T09:00:00")], vec![]),
        );
        assert_eq!(stats.lists_upserted, 0);
        merge(
            &db,
            &lists_doc(vec![list_value(5, "平局", "2026-11-01 10:00")], vec![]),
        );
        assert_eq!(list_name(&db, "5").as_deref(), Some("工作"));
        // 远端更新：采用；时间统一成规范格式
        merge(
            &db,
            &lists_doc(vec![list_value(5, "工作2", "2026-11-01T11:00:00")], vec![]),
        );
        assert_eq!(list_name(&db, "5").as_deref(), Some("工作2"));
        let row = db.with_conn(|c| {
            repo::all_list_rows(c)
                .unwrap()
                .into_iter()
                .find(|r| r.id == "5")
                .unwrap()
        });
        assert_eq!(row.updated_at, "2026-11-01 11:00:00");
        assert!(row
            .data_json
            .contains("\"updatedAt\":\"2026-11-01 11:00:00\""));

        // 只在一侧存在：并集；远端没有 todoLists 键：本地清单一条都不删
        merge(&db, &lists_doc(vec![], vec![]));
        merge(&db, &doc(vec![], vec![]));
        assert_eq!(list_name(&db, "5").as_deref(), Some("工作2"));
        assert_eq!(list_name(&db, "1").as_deref(), Some("待办清单"));
    }

    /// 墓碑压制同键且 `updatedAt <= deletedAt` 的清单；删除后又被编辑的保留；
    /// 收到清单墓碑时不改写待办（读侧兜底）。
    #[test]
    fn list_tombstones_delete_and_suppress_but_edit_after_delete_survives() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &lists_doc(
                vec![
                    list_value(5, "工作", "2026-11-01 10:00:00"),
                    list_value(6, "生活", "2026-11-01 10:00:00"),
                ],
                vec![],
            ),
        );
        db.with_conn(|c| {
            repo::upsert_todo(
                c,
                "1",
                r#"{"id":1,"title":"t","listId":5}"#,
                "2026-11-01 10:00:00",
            )
            .unwrap()
        });
        let stats = merge(
            &db,
            &lists_doc(
                vec![list_value(6, "生活-改", "2026-11-01 12:00:00")],
                vec![
                    list_tomb(5, "2026-11-01 10:00:00"),
                    list_tomb(6, "2026-11-01 11:00:00"),
                    list_tomb(7, "2026-11-01 11:00:00"),
                ],
            ),
        );
        assert_eq!(
            stats.lists_deleted, 1,
            "deletedAt >= updatedAt（含相等）即删除"
        );
        assert_eq!(stats.list_tombstones_applied, 3);
        assert!(list_name(&db, "5").is_none());
        assert_eq!(
            list_name(&db, "6").as_deref(),
            Some("生活-改"),
            "删除后又被编辑 → 保留"
        );
        let todo = db.with_conn(|c| repo::get_todo(c, "1").unwrap().unwrap());
        assert!(
            todo.data_json.contains("\"listId\":5"),
            "清单墓碑不改写待办：{}",
            todo.data_json
        );

        // 远端陈旧副本被本地墓碑压制
        let stats = merge(
            &db,
            &lists_doc(vec![list_value(7, "陈旧", "2026-11-01 09:00:00")], vec![]),
        );
        assert_eq!(stats.lists_upserted, 0);
        assert!(list_name(&db, "7").is_none());
        assert_eq!(
            list_tombstones(&db),
            vec![
                ("5".to_string(), "2026-11-01 10:00:00".to_string()),
                ("6".to_string(), "2026-11-01 11:00:00".to_string()),
                ("7".to_string(), "2026-11-01 11:00:00".to_string()),
            ]
        );
    }

    /// 默认清单：远端的 id 1 墓碑一律忽略；改名照常按 LWW 生效；本地没有时补建。
    #[test]
    fn default_list_tombstone_is_ignored_and_rename_wins() {
        let (db, _tmp) = fresh_db();
        let stats = merge(
            &db,
            &lists_doc(vec![], vec![list_tomb(1, "2999-01-01 00:00:00")]),
        );
        assert_eq!(stats.list_tombstones_applied, 0);
        assert_eq!(stats.records_skipped, 0);
        assert!(list_tombstones(&db).is_empty(), "不存");
        assert_eq!(list_name(&db, "1").as_deref(), Some("待办清单"));

        merge(
            &db,
            &lists_doc(
                vec![list_value(1, "收件箱", "2026-11-01 10:00:00")],
                vec![list_tomb(1, "2999-01-01 00:00:00")],
            ),
        );
        assert_eq!(list_name(&db, "1").as_deref(), Some("收件箱"));

        // 本地默认清单被（异常地）删掉了：合并时按固定初始值补建
        db.with_conn(|c| c.execute("DELETE FROM todo_lists WHERE id = '1'", []))
            .unwrap();
        merge(&db, &doc(vec![], vec![]));
        assert_eq!(list_name(&db, "1").as_deref(), Some("待办清单"));
    }

    #[test]
    fn bad_remote_lists_are_skipped_and_counted_never_deleted() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &lists_doc(vec![list_value(5, "工作", "2026-11-01 10:00:00")], vec![]),
        );
        let stats = merge(
            &db,
            &lists_doc(
                vec![
                    json!({"name": "no id"}),
                    json!({"id": "5", "name": "string id"}),
                    json!("garbage"),
                    list_value(8, "ok", "2026-11-01 10:00:00"),
                ],
                vec![json!({"entityId": "x", "deletedAt": "2026-11-01 10:00:00"})],
            ),
        );
        assert_eq!(stats.records_skipped, 4);
        assert_eq!(list_name(&db, "5").as_deref(), Some("工作"));
        assert_eq!(list_name(&db, "8").as_deref(), Some("ok"));
    }

    // ---------------------------------------------------------------- 待办的 listId（契约 §12）

    fn todo_in(id: i64, title: &str, updated_at: &str, list_id: Option<Value>) -> Value {
        let mut t = todo_value(id, title, updated_at);
        if let Some(l) = list_id {
            t["listId"] = l;
        }
        t
    }

    /// 缓存里的 data_json 与 updated_at 列。
    fn stored(db: &Db, id: &str) -> (Value, String) {
        db.with_conn(|c| {
            let row = repo::get_todo(c, id).unwrap().unwrap();
            (
                serde_json::from_str(&row.data_json).unwrap(),
                row.updated_at,
            )
        })
    }

    fn dirty(db: &Db) -> bool {
        db.with_conn(|c| repo::is_dirty(c).unwrap())
    }

    /// §12-2：远端胜出但不认识清单（缺 listId / null）→ 沿用本地已知的 listId；本地也未知时
    /// 仍是未知（不补成 1）。远端已知的新值照常按 LWW 覆盖。
    #[test]
    fn remote_winner_without_list_id_keeps_the_local_list_id() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(
                vec![
                    todo_in(1, "a", "2026-11-01 10:00:00", Some(json!(5))),
                    todo_in(2, "b", "2026-11-01 10:00:00", None),
                ],
                vec![],
            ),
        );
        let stats = merge(
            &db,
            &doc(
                vec![
                    todo_in(1, "a-old-pc", "2026-11-01 11:00:00", None),
                    todo_in(2, "b-old-pc", "2026-11-01 11:00:00", Some(Value::Null)),
                ],
                vec![],
            ),
        );
        assert_eq!(stats.todos_upserted, 2);
        assert_eq!(stats.list_ids_carried, 1);
        let (t1, ts1) = stored(&db, "1");
        assert_eq!(t1["title"], "a-old-pc");
        assert_eq!(t1["listId"], 5, "沿用本地已知的清单");
        assert_eq!(ts1, "2026-11-01 11:00:00", "版本是远端的");
        let (t2, _) = stored(&db, "2");
        assert_eq!(t2["title"], "b-old-pc");
        assert!(
            t2.get("listId").is_none(),
            "本地也未知：仍是未知（null 统一成缺失），不补成 1: {}",
            t2
        );

        merge(
            &db,
            &doc(
                vec![todo_in(1, "a3", "2026-11-01 12:00:00", Some(Value::Null))],
                vec![],
            ),
        );
        assert_eq!(stored(&db, "1").0["listId"], 5, "远端 null 同样算未知");
        merge(
            &db,
            &doc(
                vec![todo_in(1, "a4", "2026-11-01 13:00:00", Some(json!(7)))],
                vec![],
            ),
        );
        assert_eq!(stored(&db, "1").0["listId"], 7, "远端已知的新值照常覆盖");
        assert!(!dirty(&db), "合并从不标脏");
    }

    /// §12-3：远端已知、本地未知 → 不论 LWW 谁胜（含平局、本地更新）都采纳远端的 listId，
    /// 不改 updatedAt、不标脏；本地已知 / 远端类型不对时不采纳。
    #[test]
    fn remote_list_id_fills_unknown_local_regardless_of_lww() {
        let (db, _tmp) = fresh_db();
        merge(
            &db,
            &doc(
                vec![
                    todo_in(1, "tie", "2026-11-01 10:00:00", None),
                    todo_in(2, "local-newer", "2026-11-01 12:00:00", None),
                    todo_in(3, "known", "2026-11-01 10:00:00", Some(json!(5))),
                    todo_in(4, "bad remote", "2026-11-01 10:00:00", None),
                    todo_in(5, "remote-newer", "2026-11-01 10:00:00", None),
                ],
                vec![],
            ),
        );
        let stats = merge(
            &db,
            &doc(
                vec![
                    todo_in(1, "tie", "2026-11-01 10:00:00", Some(json!(7))),
                    todo_in(2, "remote-older", "2026-11-01 11:00:00", Some(json!(7))),
                    todo_in(3, "known", "2026-11-01 10:00:00", Some(json!(8))),
                    todo_in(4, "bad remote", "2026-11-01 10:00:00", Some(json!("7"))),
                    todo_in(5, "remote-newer", "2026-11-01 13:00:00", Some(json!(7))),
                ],
                vec![],
            ),
        );
        assert_eq!(stats.list_ids_adopted, 2);
        let (t1, ts1) = stored(&db, "1");
        assert_eq!(t1["listId"], 7, "平局：采纳");
        assert_eq!(ts1, "2026-11-01 10:00:00");
        assert_eq!(t1["updatedAt"], "2026-11-01 10:00:00", "不改 updatedAt");
        let (t2, ts2) = stored(&db, "2");
        assert_eq!(t2["listId"], 7, "本地更新：也采纳");
        assert_eq!(t2["title"], "local-newer", "本地版本保留，只补 listId");
        assert_eq!(ts2, "2026-11-01 12:00:00");
        assert_eq!(t2["updatedAt"], "2026-11-01 12:00:00");
        assert_eq!(stored(&db, "3").0["listId"], 5, "本地已知：平局保留本地");
        assert!(
            stored(&db, "4").0.get("listId").is_none(),
            "类型不对的远端值不采纳"
        );
        let (t5, ts5) = stored(&db, "5");
        assert_eq!(
            (t5["listId"].clone(), ts5.as_str()),
            (json!(7), "2026-11-01 13:00:00"),
            "远端胜出：整条采用"
        );
        assert!(!dirty(&db));

        // 只采纳 listId（平局）不会让远端显得"落后"：不为它多推一次
        let (db2, _tmp2) = fresh_db();
        merge(
            &db2,
            &doc(vec![todo_in(1, "a", "2026-11-01 10:00:00", None)], vec![]),
        );
        let new_writer = doc(
            vec![todo_in(1, "a", "2026-11-01 10:00:00", Some(json!(7)))],
            vec![],
        );
        merge(&db2, &new_writer);
        assert_eq!(stored(&db2, "1").0["listId"], 7);
        assert!(!lacks(&db2, &new_writer, CUTOFF));
        assert!(!dirty(&db2));
    }

    /// §12-8 的 cloud 对应：远端缺 listId（未知）而本地已知 → 远端落后，补传一次；
    /// 两边都未知、远端已知、旧协议文档都不算。
    #[test]
    fn remote_missing_a_known_list_id_counts_as_behind() {
        let (db, _tmp) = fresh_db();
        let t = todo_in(1, "a", "2026-11-01 10:00:00", Some(json!(5)));
        merge(&db, &doc(vec![t.clone()], vec![]));
        assert!(!lacks(&db, &doc(vec![t], vec![]), CUTOFF));
        let unknown = todo_in(1, "a", "2026-11-01 10:00:00", None);
        assert!(lacks(&db, &doc(vec![unknown.clone()], vec![]), CUTOFF));
        let null = todo_in(1, "a", "2026-11-01 10:00:00", Some(Value::Null));
        assert!(lacks(&db, &doc(vec![null], vec![]), CUTOFF));
        assert!(
            !lacks(&db, &legacy_doc(vec![unknown]), CUTOFF),
            "旧协议文档不比较"
        );

        let (db2, _tmp2) = fresh_db();
        let u = todo_in(2, "b", "2026-11-01 10:00:00", None);
        merge(&db2, &doc(vec![u.clone()], vec![]));
        assert!(!lacks(&db2, &doc(vec![u], vec![]), CUTOFF), "两边都未知");
    }

    /// 远端缺本地清单 / 清单墓碑（或版本更旧）→ 落后；初始值的默认清单不算；旧协议文档不比较。
    #[test]
    fn remote_lacks_local_lists_and_list_tombstones() {
        let (db, _tmp) = fresh_db();
        // 只有初始值的默认清单：远端没有任何清单信息也不算落后
        assert!(!lacks(&db, &doc(vec![], vec![]), CUTOFF));
        assert!(!lacks(&db, &lists_doc(vec![], vec![]), CUTOFF));

        let full = lists_doc(
            vec![
                list_value(1, "收件箱", "2026-11-01 09:00:00"),
                list_value(5, "工作", "2026-11-01 10:00:00"),
            ],
            vec![list_tomb(6, "2026-11-01 11:00:00")],
        );
        merge(&db, &full);
        assert!(!lacks(&db, &full, CUTOFF), "完全一致");
        // 缺清单 5
        assert!(lacks(
            &db,
            &lists_doc(
                vec![list_value(1, "收件箱", "2026-11-01 09:00:00")],
                vec![list_tomb(6, "2026-11-01 11:00:00")]
            ),
            CUTOFF
        ));
        // 改过名的默认清单不再是初始值：远端缺它同样算落后
        assert!(lacks(
            &db,
            &lists_doc(
                vec![list_value(5, "工作", "2026-11-01 10:00:00")],
                vec![list_tomb(6, "2026-11-01 11:00:00")]
            ),
            CUTOFF
        ));
        // 远端版本更旧；平局不算
        assert!(lacks(
            &db,
            &lists_doc(
                vec![
                    list_value(1, "收件箱", "2026-11-01 09:00:00"),
                    list_value(5, "工作", "2026-11-01T09:30:00"),
                ],
                vec![list_tomb(6, "2026-11-01 11:00:00")]
            ),
            CUTOFF
        ));
        // 缺清单墓碑 / 远端的更早
        let without_tomb = lists_doc(
            vec![
                list_value(1, "收件箱", "2026-11-01 09:00:00"),
                list_value(5, "工作", "2026-11-01 10:00"),
            ],
            vec![],
        );
        assert!(lacks(&db, &without_tomb, CUTOFF));
        assert!(
            !lacks(&db, &without_tomb, "2026-11-02 00:00:00"),
            "快过期的清单墓碑不要求远端保留"
        );
        // 远端没有 todoLists 键：本地清单都算缺失
        assert!(lacks(&db, &doc(vec![], vec![]), CUTOFF));
        // 旧协议文档（没有 tombstones 键）不比较清单
        assert!(!lacks(&db, &legacy_doc(vec![]), CUTOFF));
    }

    #[test]
    fn list_tombstone_union_takes_max_and_respects_retention() {
        let (db, _tmp) = fresh_db();
        db.with_conn(|c| {
            repo::add_list_tombstone(c, "5", "2026-09-20 10:00:00").unwrap();
            repo::add_list_tombstone(c, "6", "2026-09-25 10:00:00").unwrap();
            repo::add_list_tombstone(c, "7", "2026-08-01 10:00:00").unwrap(); // 本地过期
        });
        merge_with_cutoff(
            &db,
            &lists_doc(
                vec![],
                vec![
                    list_tomb(5, "2026-09-21 10:00:00"),
                    list_tomb(6, "2026-09-24 10:00:00"),
                    list_tomb(8, "2026-08-15 10:00:00"), // 远端过期：不导入
                    list_tomb(9, "2026-09-30 10:00:00"),
                ],
            ),
            "2026-09-01 00:00:00",
        );
        assert_eq!(
            list_tombstones(&db),
            vec![
                ("5".to_string(), "2026-09-21 10:00:00".to_string()),
                ("6".to_string(), "2026-09-25 10:00:00".to_string()),
                ("9".to_string(), "2026-09-30 10:00:00".to_string()),
            ]
        );
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

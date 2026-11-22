use super::*;
use crate::db::models::{ListTombstone, Tombstone, DEFAULT_LIST_ID};
use crate::db::Database;
use rusqlite::OptionalExtension;
use serde_json::{json, Value};

const CUTOFF: &str = "2000-01-01 00:00:00";

fn db() -> Database {
    Database::new_in_memory().expect("内存库")
}

fn todo_json(id: i64, title: &str, updated_at: &str) -> Value {
    json!({
        "id": id, "title": title, "description": null, "color": "#EF4444", "quadrant": 1,
        "notifyAt": null, "notifyBefore": 0, "notified": false, "completed": false,
        "sortOrder": 0, "startTime": null, "endTime": null,
        "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at, "subtasks": []
    })
}

fn sub_json(id: i64, parent: i64, title: &str, updated_at: &str) -> Value {
    json!({
        "id": id, "parentId": parent, "title": title, "content": null, "completed": false,
        "sortOrder": 0, "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at
    })
}

fn with_subs(mut todo: Value, subs: Vec<Value>) -> Value {
    todo["subtasks"] = Value::Array(subs);
    todo
}

fn merge(db: &Database, todos: Vec<Value>, tombs: Vec<Tombstone>) -> MergeStats {
    let parsed = parse_remote_todos(&todos);
    db.with_transaction(|tx| merge_remote(tx, &parsed, &tombs, CUTOFF))
        .expect("合并失败")
}

fn tomb(kind: &str, id: i64, at: &str) -> Tombstone {
    Tombstone {
        entity_type: kind.to_string(),
        entity_id: id,
        deleted_at: at.to_string(),
    }
}

fn title(db: &Database, table: &str, id: i64) -> Option<String> {
    db.with_connection(|c| {
        c.query_row(
            &format!("SELECT title FROM {table} WHERE id = ?1"),
            [id],
            |r| r.get(0),
        )
        .optional()
    })
    .unwrap()
}

fn count(db: &Database, table: &str) -> i64 {
    db.with_connection(|c| c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)))
        .unwrap()
}

#[test]
fn inserts_remote_records_with_their_ids() {
    let db = db();
    let stats = merge(
        &db,
        vec![with_subs(
            todo_json(1715000000000123, "远端", "2026-05-01 10:00:00"),
            vec![sub_json(77, 999, "子", "2026-05-01 10:00:00")],
        )],
        vec![],
    );
    assert_eq!(stats.todos_inserted, 1);
    assert_eq!(stats.subtasks_inserted, 1);
    assert_eq!(
        title(&db, "todos", 1715000000000123).as_deref(),
        Some("远端")
    );
    // parentId 以外层待办为准
    let parent: i64 = db
        .with_connection(|c| {
            c.query_row("SELECT parent_id FROM subtasks WHERE id = 77", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(parent, 1715000000000123);
}

#[test]
fn lww_remote_newer_wins_local_newer_and_ties_keep_local() {
    let db = db();
    merge(&db, vec![todo_json(1, "v1", "2026-05-01 10:00:00")], vec![]);

    // 远端较新 → 覆盖
    let s = merge(&db, vec![todo_json(1, "v2", "2026-05-02 10:00:00")], vec![]);
    assert_eq!(s.todos_updated, 1);
    assert_eq!(title(&db, "todos", 1).as_deref(), Some("v2"));

    // 平局 → 保留本地
    let s = merge(
        &db,
        vec![todo_json(1, "tie", "2026-05-02 10:00:00")],
        vec![],
    );
    assert_eq!(s.todos_updated, 0);
    assert_eq!(title(&db, "todos", 1).as_deref(), Some("v2"));

    // 远端较旧 → 保留本地
    let s = merge(
        &db,
        vec![todo_json(1, "old", "2026-04-01 10:00:00")],
        vec![],
    );
    assert_eq!(s, MergeStats::default());
    assert_eq!(title(&db, "todos", 1).as_deref(), Some("v2"));
}

#[test]
fn lww_compares_normalized_timestamps() {
    let db = db();
    merge(
        &db,
        vec![todo_json(1, "local", "2026-05-01 10:00:00")],
        vec![],
    );
    // 带 T 的同一时刻 = 平局，保留本地
    let s = merge(
        &db,
        vec![todo_json(1, "T 格式", "2026-05-01T10:00:00")],
        vec![],
    );
    assert_eq!(s.todos_updated, 0);
    // 带 T、晚一分钟 → 远端较新
    let s = merge(
        &db,
        vec![todo_json(1, "T 格式新", "2026-05-01T10:01")],
        vec![],
    );
    assert_eq!(s.todos_updated, 1);
    let stored: String = db
        .with_connection(|c| {
            c.query_row("SELECT updated_at FROM todos WHERE id = 1", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(stored, "2026-05-01 10:01:00", "落库前规范化");
}

#[test]
fn absence_on_remote_never_deletes_local_records() {
    let db = db();
    merge(
        &db,
        vec![with_subs(
            todo_json(1, "本地独有", "2026-05-01 10:00:00"),
            vec![sub_json(11, 1, "子", "2026-05-01 10:00:00")],
        )],
        vec![],
    );
    let s = merge(
        &db,
        vec![todo_json(2, "远端", "2026-05-01 10:00:00")],
        vec![],
    );
    assert_eq!(s.todos_deleted + s.subtasks_deleted, 0);
    assert!(title(&db, "todos", 1).is_some());
    assert!(title(&db, "subtasks", 11).is_some());
    assert!(title(&db, "todos", 2).is_some());
}

#[test]
fn remote_tombstone_deletes_older_local_record_and_its_subtasks() {
    let db = db();
    merge(
        &db,
        vec![with_subs(
            todo_json(1, "将被删", "2026-05-01 10:00:00"),
            vec![sub_json(11, 1, "随父删", "2026-05-03 10:00:00")],
        )],
        vec![],
    );
    let s = merge(&db, vec![], vec![tomb("todo", 1, "2026-05-02 10:00:00")]);
    assert_eq!(s.todos_deleted, 1);
    assert_eq!(s.subtasks_deleted, 1);
    assert_eq!(count(&db, "todos"), 0);
    assert_eq!(count(&db, "subtasks"), 0);
    // 墓碑被并入本地，供后续上传
    let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
    assert_eq!(tombs, vec![tomb("todo", 1, "2026-05-02 10:00:00")]);
}

#[test]
fn record_edited_after_deletion_survives_tombstone() {
    let db = db();
    merge(
        &db,
        vec![todo_json(1, "删除后又编辑", "2026-05-03 10:00:00")],
        vec![],
    );
    let s = merge(&db, vec![], vec![tomb("todo", 1, "2026-05-02 10:00:00")]);
    assert_eq!(s.todos_deleted, 0);
    assert!(title(&db, "todos", 1).is_some());

    // 远端带着较新的版本 + 较旧的墓碑：插入
    let db2 = self::db();
    let s = merge(
        &db2,
        vec![todo_json(5, "复活", "2026-05-03 10:00:00")],
        vec![tomb("todo", 5, "2026-05-02 10:00:00")],
    );
    assert_eq!(s.todos_inserted, 1);
}

#[test]
fn local_tombstone_suppresses_stale_remote_record() {
    let db = db();
    db.with_transaction(|tx| record_tombstone(tx, EntityKind::Todo, 9, "2026-05-02 10:00:00"))
        .unwrap();
    db.with_transaction(|tx| record_tombstone(tx, EntityKind::Subtask, 21, "2026-05-02 10:00:00"))
        .unwrap();
    let s = merge(
        &db,
        vec![
            todo_json(9, "已删除的旧版本", "2026-05-01 10:00:00"),
            with_subs(
                todo_json(2, "保留", "2026-05-01 10:00:00"),
                vec![
                    sub_json(21, 2, "已删除子任务", "2026-05-01 10:00:00"),
                    sub_json(22, 2, "保留子任务", "2026-05-01 10:00:00"),
                ],
            ),
        ],
        vec![],
    );
    assert_eq!(s.todos_inserted, 1);
    assert_eq!(s.subtasks_inserted, 1);
    assert_eq!(s.records_skipped, 0, "被墓碑压制不算跳过");
    assert!(title(&db, "todos", 9).is_none());
    assert!(title(&db, "subtasks", 21).is_none());
    assert!(title(&db, "subtasks", 22).is_some());
}

#[test]
fn bad_records_are_skipped_without_affecting_others() {
    let db = db();
    merge(
        &db,
        vec![todo_json(3, "本地已有", "2026-05-01 10:00:00")],
        vec![],
    );

    let mut bad_type = todo_json(3, "类型错误", "2026-05-09 10:00:00");
    bad_type["quadrant"] = json!("urgent_important");
    let mut bad_time = todo_json(4, "时间错误", "2026-05-09 10:00:00");
    bad_time["notifyAt"] = json!("明天早上");
    let good = with_subs(
        todo_json(5, "正常", "2026-05-09 10:00:00"),
        vec![
            sub_json(51, 5, "正常子任务", "2026-05-09 10:00:00"),
            json!({"id": 52, "title": 123, "updatedAt": "2026-05-09 10:00:00"}),
        ],
    );
    let s = merge(
        &db,
        vec![bad_type, bad_time, json!("not an object"), good],
        vec![],
    );
    assert_eq!(s.todos_inserted, 1);
    assert_eq!(s.subtasks_inserted, 1);
    assert_eq!(s.records_skipped, 4);
    assert_eq!(
        title(&db, "todos", 3).as_deref(),
        Some("本地已有"),
        "绝不当作删除"
    );
    assert!(title(&db, "todos", 4).is_none());
    assert!(title(&db, "todos", 5).is_some());
}

#[test]
fn subtask_of_unparseable_todo_merges_into_existing_parent() {
    let db = db();
    merge(
        &db,
        vec![todo_json(3, "本地父", "2026-05-01 10:00:00")],
        vec![],
    );
    let mut bad_parent = with_subs(
        todo_json(3, "父看不懂", "2026-05-09 10:00:00"),
        vec![sub_json(31, 3, "子能看懂", "2026-05-09 10:00:00")],
    );
    bad_parent["completed"] = json!("yes");
    let s = merge(&db, vec![bad_parent], vec![]);
    assert_eq!(s.records_skipped, 1);
    assert_eq!(s.subtasks_inserted, 1);
    assert_eq!(title(&db, "todos", 3).as_deref(), Some("本地父"));
}

#[test]
fn delete_helpers_write_tombstones() {
    let db = db();
    merge(
        &db,
        vec![with_subs(
            todo_json(1, "父", "2026-05-01 10:00:00"),
            vec![
                sub_json(11, 1, "a", "2026-05-01 10:00:00"),
                sub_json(12, 1, "b", "2099-01-01 00:00:00"),
            ],
        )],
        vec![],
    );
    db.with_transaction(|tx| delete_subtask_with_tombstone(tx, 11, "2026-06-01 00:00:00"))
        .unwrap();
    db.with_transaction(|tx| delete_todo_with_tombstones(tx, 1, "2026-06-02 00:00:00"))
        .unwrap();
    assert!(!db
        .with_transaction(|tx| delete_todo_with_tombstones(tx, 1, "2026-06-02 00:00:00"))
        .unwrap());
    assert_eq!(count(&db, "todos"), 0);
    assert_eq!(count(&db, "subtasks"), 0);
    let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
    assert_eq!(
        tombs,
        vec![
            tomb("subtask", 11, "2026-06-01 00:00:00"),
            // 记录时间比"现在"还新（另一端时钟偏快）：墓碑取记录时间，保证压得住
            tomb("subtask", 12, "2099-01-01 00:00:00"),
            tomb("todo", 1, "2026-06-02 00:00:00"),
        ]
    );
}

#[test]
fn force_pull_makes_local_equal_remote() {
    let db = db();
    merge(
        &db,
        vec![
            with_subs(
                todo_json(1, "本地较新", "2026-05-09 10:00:00"),
                vec![sub_json(11, 1, "本地独有子任务", "2026-05-01 10:00:00")],
            ),
            todo_json(2, "本地独有", "2026-05-01 10:00:00"),
        ],
        vec![],
    );
    db.with_transaction(|tx| record_tombstone(tx, EntityKind::Todo, 99, "2026-05-01 10:00:00"))
        .unwrap();

    let remote = parse_remote_todos(&[todo_json(1, "远端旧版", "2026-05-01 10:00:00")]);
    let s = db
        .with_transaction(|tx| {
            force_pull_replace(
                tx,
                &remote,
                &[tomb("todo", 7, "2026-05-01 10:00:00")],
                CUTOFF,
            )
        })
        .unwrap();
    assert_eq!(s.todos_updated, 1);
    assert_eq!(s.todos_deleted, 1);
    assert_eq!(s.subtasks_deleted, 1);
    assert_eq!(title(&db, "todos", 1).as_deref(), Some("远端旧版"));
    assert!(title(&db, "todos", 2).is_none());
    let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
    assert_eq!(
        tombs,
        vec![tomb("todo", 7, "2026-05-01 10:00:00")],
        "墓碑表替换为远端"
    );
}

#[test]
fn force_push_prepare_tombstones_remote_only_and_bumps_conflicts() {
    let db = db();
    merge(
        &db,
        vec![
            todo_json(1, "本地旧", "2026-05-01 10:00:00"),
            todo_json(2, "本地", "2026-05-01 10:00:00"),
        ],
        vec![],
    );
    let remote = vec![
        todo_json(1, "远端新", "2026-05-05 10:00:00"),
        with_subs(
            todo_json(3, "远端独有", "2026-05-01 10:00:00"),
            vec![sub_json(31, 3, "远端独有子", "2026-05-01 10:00:00")],
        ),
    ];
    let now = "2026-05-03 00:00:00";
    let bumped = db
        .with_transaction(|tx| {
            force_push_prepare(tx, &remote, &[tomb("todo", 2, "2026-05-02 00:00:00")], now)
        })
        .unwrap();
    assert_eq!(bumped, 2);
    let ua = |id: i64| -> String {
        db.with_connection(|c| {
            c.query_row("SELECT updated_at FROM todos WHERE id = ?1", [id], |r| {
                r.get(0)
            })
        })
        .unwrap()
    };
    assert_eq!(ua(1), "2026-05-05 10:00:01", "必须严格晚于远端版本");
    assert_eq!(ua(2), now, "必须压过远端墓碑");
    let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
    assert_eq!(tombs, vec![tomb("subtask", 31, now), tomb("todo", 3, now)]);
}

/// 回归：远端记录的 updatedAt 是乱码时，强制推送以前会把它当成"更新的版本"，
/// 用 plus_one_second（乱码原样返回）把乱码抄进本地记录的 updated_at——此后这条记录在
/// 每一次 LWW 里都赢、其它写入方又都解析不了它。乱码时间现在视为最旧：不推进、不进墓碑。
#[test]
fn force_push_prepare_ignores_unparseable_remote_timestamps() {
    let db = db();
    merge(
        &db,
        vec![todo_json(1, "本地", "2026-05-01 10:00:00")],
        vec![],
    );
    let mut garbage_shared = todo_json(1, "远端乱码时间", "2026-05-01 10:00:00");
    garbage_shared["updatedAt"] = json!("zzz-not-a-time");
    let mut garbage_only = todo_json(2, "远端独有乱码时间", "2026-05-01 10:00:00");
    garbage_only["updatedAt"] = json!("zzz-not-a-time");
    let now = "2026-05-03 00:00:00";

    let bumped = db
        .with_transaction(|tx| force_push_prepare(tx, &[garbage_shared, garbage_only], &[], now))
        .unwrap();
    assert_eq!(bumped, 0, "乱码时间不算更新的版本");
    let local: String = db
        .with_connection(|c| {
            c.query_row("SELECT updated_at FROM todos WHERE id = 1", [], |r| {
                r.get(0)
            })
        })
        .unwrap();
    assert_eq!(local, "2026-05-01 10:00:00");
    let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
    assert_eq!(tombs, vec![tomb("todo", 2, now)], "墓碑时间只取合法时间");
}

#[test]
fn deletion_time_only_trusts_parseable_record_times() {
    let now = "2026-05-03 00:00:00";
    assert_eq!(deletion_time(now, "2026-05-01 10:00:00"), now);
    assert_eq!(
        deletion_time(now, "2099-01-01T08:00"),
        "2099-01-01 08:00:00",
        "比现在还新的合法时间：取它（规范化后）"
    );
    for bad in ["zzz", "", "not a time"] {
        assert_eq!(deletion_time(now, bad), now, "{bad:?}");
    }

    // 本地记录带着乱码时间被删除：墓碑落规范时间
    let db = db();
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO todos (id, title, updated_at) VALUES (5, 'x', 'zzz')",
            [],
        )
        .map(|_| ())
    })
    .unwrap();
    db.with_transaction(|tx| delete_todo_with_tombstones(tx, 5, now))
        .unwrap();
    let tombs = db.with_connection(|c| list_tombstones(c, CUTOFF)).unwrap();
    assert_eq!(tombs, vec![tomb("todo", 5, now)]);
}

#[test]
fn value_updated_at_treats_garbage_as_oldest() {
    assert_eq!(
        value_updated_at(&json!({"updatedAt": "2026-05-01T10:00"})),
        "2026-05-01 10:00:00"
    );
    for v in [
        json!({"updatedAt": "zzz"}),
        json!({"updatedAt": 20260501}),
        json!({}),
        json!("not an object"),
    ] {
        assert_eq!(value_updated_at(&v), "", "{v}");
    }
}

#[test]
fn load_todos_groups_subtasks_without_n_plus_one() {
    let db = db();
    merge(
        &db,
        vec![
            with_subs(
                todo_json(1, "a", "2026-05-01 10:00:00"),
                vec![sub_json(11, 1, "a1", "2026-05-01 10:00:00")],
            ),
            with_subs(
                todo_json(2, "b", "2026-05-01 10:00:00"),
                vec![
                    sub_json(21, 2, "b1", "2026-05-01 10:00:00"),
                    sub_json(22, 2, "b2", "2026-05-01 10:00:00"),
                ],
            ),
        ],
        vec![],
    );
    let todos = db
        .with_connection(|c| load_todos_with_subtasks(c, "id ASC"))
        .unwrap();
    assert_eq!(todos.len(), 2);
    assert_eq!(todos[0].subtasks.len(), 1);
    assert_eq!(todos[1].subtasks.len(), 2);
}

#[test]
fn parse_tombstone_is_lenient_but_validates() {
    assert_eq!(
        parse_tombstone(
            &json!({"entityType": "todo", "entityId": 5, "deletedAt": "2026-05-01T10:00"})
        ),
        Some(tomb("todo", 5, "2026-05-01 10:00:00"))
    );
    assert_eq!(
        parse_tombstone(
            &json!({"entityType": "subtask", "entityId": "6", "deletedAt": "2026-05-01 10:00:00"})
        ),
        Some(tomb("subtask", 6, "2026-05-01 10:00:00"))
    );
    assert_eq!(
        parse_tombstone(&json!({"entityType": "note", "entityId": 5, "deletedAt": "2026-05-01"})),
        None
    );
    assert_eq!(
        parse_tombstone(&json!({"entityType": "todo", "entityId": "x", "deletedAt": "2026-05-01"})),
        None
    );
    assert_eq!(
        parse_tombstone(&json!({"entityType": "todo", "entityId": 5, "deletedAt": "yesterday"})),
        None
    );
}

// ---- 清单（v31，契约 §4 / §5）----

const WORK: i64 = 1_731_000_000_000_123;
const LIFE: i64 = 1_731_000_000_000_456;

fn list_json(id: i64, name: &str, updated_at: &str) -> Value {
    json!({"id": id, "name": name, "sortOrder": 1,
           "createdAt": "2026-01-01 00:00:00", "updatedAt": updated_at})
}

fn list_tomb(id: i64, at: &str) -> ListTombstone {
    ListTombstone {
        entity_id: id,
        deleted_at: at.to_string(),
    }
}

fn merge_lists(db: &Database, lists: Option<Vec<Value>>, tombs: Vec<ListTombstone>) -> MergeStats {
    let parsed = lists.as_deref().map(parse_remote_lists);
    db.with_transaction(|tx| merge_remote_lists(tx, parsed.as_deref(), &tombs))
        .expect("合并清单失败")
}

fn list_name(db: &Database, id: i64) -> Option<String> {
    db.with_connection(|c| {
        c.query_row("SELECT name FROM todo_lists WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .optional()
    })
    .unwrap()
}

fn list_tombs(db: &Database) -> Vec<ListTombstone> {
    db.with_connection(load_list_tombstones).unwrap()
}

#[test]
fn remote_lists_merge_by_lww_and_ties_keep_local() {
    let db = db();
    let s = merge_lists(
        &db,
        Some(vec![list_json(WORK, "工作", "2026-05-01 10:00:00")]),
        vec![],
    );
    assert_eq!(s.lists_inserted, 1);
    assert_eq!(list_name(&db, WORK).as_deref(), Some("工作"));

    // 远端较新 → 覆盖
    let s = merge_lists(
        &db,
        Some(vec![list_json(WORK, "工作 2", "2026-05-02 10:00:00")]),
        vec![],
    );
    assert_eq!(s.lists_updated, 1);
    assert_eq!(list_name(&db, WORK).as_deref(), Some("工作 2"));
    // 平局 / 远端较旧 → 保留本地
    for (name, at) in [
        ("平局", "2026-05-02 10:00:00"),
        ("旧", "2026-04-01 10:00:00"),
    ] {
        let s = merge_lists(&db, Some(vec![list_json(WORK, name, at)]), vec![]);
        assert_eq!(s, MergeStats::default());
        assert_eq!(list_name(&db, WORK).as_deref(), Some("工作 2"));
    }

    // 默认清单：迁移种子是固定的 2000-01-01，任何一次改名都胜出；本地改过名（更新）则本地胜出
    let s = merge_lists(
        &db,
        Some(vec![list_json(1, "收件箱", "2026-05-03 10:00:00")]),
        vec![],
    );
    assert_eq!(s.lists_updated, 1);
    assert_eq!(list_name(&db, 1).as_deref(), Some("收件箱"));
    let s = merge_lists(
        &db,
        Some(vec![list_json(1, "待办清单", "2000-01-01 00:00:00")]),
        vec![],
    );
    assert_eq!(s.lists_updated, 0, "另一台设备没改过名的 id 1 不会抢");
    assert_eq!(list_name(&db, 1).as_deref(), Some("收件箱"));
}

#[test]
fn local_only_lists_survive_and_a_remote_without_list_info_deletes_nothing() {
    let db = db();
    merge_lists(
        &db,
        Some(vec![list_json(WORK, "本地独有", "2026-05-01 10:00:00")]),
        vec![],
    );
    let s = merge_lists(
        &db,
        Some(vec![list_json(LIFE, "远端", "2026-05-01 10:00:00")]),
        vec![],
    );
    assert_eq!((s.lists_inserted, s.lists_deleted), (1, 0), "并集");
    // 远端文档没有 todoLists 键（旧版本写入方）：本地清单一条都不删
    let s = merge_lists(&db, None, vec![]);
    assert_eq!(s, MergeStats::default());
    assert!(list_name(&db, WORK).is_some() && list_name(&db, LIFE).is_some());
    // 有 todoLists 但是空数组：同样不删（没有"缺席即删除"）
    let s = merge_lists(&db, Some(vec![]), vec![]);
    assert_eq!(s.lists_deleted, 0);
    assert_eq!(count(&db, "todo_lists"), 3);
}

#[test]
fn list_tombstones_delete_older_lists_but_not_ones_edited_after_deletion() {
    let db = db();
    merge_lists(
        &db,
        Some(vec![
            list_json(WORK, "将被删", "2026-05-01 10:00:00"),
            list_json(LIFE, "删除后又改", "2026-05-05 10:00:00"),
        ]),
        vec![],
    );
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO todos (id, title, list_id) VALUES (1, 't', ?1)",
            [WORK],
        )
    })
    .unwrap();

    let s = merge_lists(
        &db,
        None,
        vec![
            list_tomb(WORK, "2026-05-02 10:00:00"),
            list_tomb(LIFE, "2026-05-02 10:00:00"),
        ],
    );
    assert_eq!(s.lists_deleted, 1);
    assert!(list_name(&db, WORK).is_none());
    assert!(
        list_name(&db, LIFE).is_some(),
        "updatedAt > deletedAt：保留"
    );
    let list_id: i64 = db
        .with_connection(|c| {
            c.query_row("SELECT list_id FROM todos WHERE id = 1", [], |r| r.get(0))
        })
        .unwrap();
    assert_eq!(
        list_id, WORK,
        "收到清单墓碑不改写待办（读侧按默认清单显示）"
    );
    assert_eq!(
        list_tombs(&db),
        vec![
            list_tomb(WORK, "2026-05-02 10:00:00"),
            list_tomb(LIFE, "2026-05-02 10:00:00")
        ],
        "墓碑并入本地，供后续上传"
    );

    // 本地墓碑压制远端的旧版本（另一台设备还没同步到删除）；墓碑并集取较大值
    let s = merge_lists(
        &db,
        Some(vec![list_json(WORK, "旧版本", "2026-05-01 10:00:00")]),
        vec![list_tomb(WORK, "2026-05-03 10:00:00")],
    );
    assert_eq!(s, MergeStats::default(), "被墓碑压制不算跳过");
    assert!(list_name(&db, WORK).is_none());
    assert_eq!(list_tombs(&db)[0], list_tomb(WORK, "2026-05-03 10:00:00"));

    // 远端带着较新的版本 + 较旧的墓碑：插入
    let db2 = self::db();
    let s = merge_lists(
        &db2,
        Some(vec![list_json(WORK, "复活", "2026-05-04 10:00:00")]),
        vec![list_tomb(WORK, "2026-05-03 10:00:00")],
    );
    assert_eq!(s.lists_inserted, 1);
}

#[test]
fn default_list_tombstones_are_ignored_everywhere() {
    assert_eq!(
        parse_list_tombstone(&json!({"entityId": 1, "deletedAt": "2026-05-01 10:00:00"})),
        None,
        "默认清单不可删除：它的墓碑不存、不传播"
    );
    let db = db();
    db.with_transaction(|tx| record_list_tombstone(tx, 1, "2099-01-01 00:00:00"))
        .unwrap();
    assert!(list_tombs(&db).is_empty());
    // 就算墓碑表里有（外部工具写的），默认清单也不会被删、不会被压制
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO list_tombstones (entity_id, deleted_at) VALUES (1, '2099-01-01 00:00:00')",
            [],
        )
    })
    .unwrap();
    let s = merge_lists(
        &db,
        Some(vec![list_json(1, "收件箱", "2026-05-01 10:00:00")]),
        vec![list_tomb(1, "2099-01-01 00:00:00")],
    );
    assert_eq!((s.lists_deleted, s.lists_updated), (0, 1));
    assert_eq!(list_name(&db, 1).as_deref(), Some("收件箱"));
    assert!(list_tombs(&db).is_empty(), "读出 / 上传时也不带它");
}

#[test]
fn bad_remote_lists_are_skipped_and_counted() {
    let db = db();
    db.with_transaction(|tx| record_list_tombstone(tx, 77, "2026-05-02 10:00:00"))
        .unwrap();
    let mut no_name = list_json(WORK, "x", "2026-05-01 10:00:00");
    no_name.as_object_mut().unwrap().remove("name");
    let mut bad_type = list_json(LIFE, "x", "2026-05-01 10:00:00");
    bad_type["sortOrder"] = json!("first");
    let mut bad_time = list_json(55, "x", "2026-05-01 10:00:00");
    bad_time["updatedAt"] = json!("昨天");
    let mut suppressed_bad = list_json(77, "x", "2026-05-01 10:00:00");
    suppressed_bad["name"] = json!(123);
    let s = merge_lists(
        &db,
        Some(vec![
            no_name,
            bad_type,
            bad_time,
            json!("not an object"),
            json!({"name": "没有 id", "updatedAt": "2026-05-01 10:00:00"}),
            suppressed_bad,
            list_json(66, "正常", "2026-05-01 10:00:00"),
        ]),
        vec![],
    );
    assert_eq!(s.lists_inserted, 1);
    assert_eq!(s.records_skipped, 5, "已被墓碑压制的坏记录不算跳过");
    assert_eq!(count(&db, "todo_lists"), 2);
}

/// (list_id, title, updated_at)
fn todo_row(db: &Database, id: i64) -> (Option<i64>, String, String) {
    db.with_connection(|c| {
        c.query_row(
            "SELECT list_id, title, updated_at FROM todos WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
    })
    .unwrap()
}

fn todo_in(id: i64, title: &str, updated_at: &str, list_id: i64) -> Value {
    let mut todo = todo_json(id, title, updated_at);
    todo["listId"] = json!(list_id);
    todo
}

/// 契约 §12 / §13-3：`listId` 缺失 = 未知，是最旧的归属，永远不覆盖已知的。远端胜出但没有 listId → 内容取远端、
/// 归属沿用本地；远端新增且没有 listId → 本地也是未知（不是默认清单）；上传时未知不写 listId
#[test]
fn missing_list_id_means_unknown_and_never_overrides_a_known_list() {
    let db = db();
    merge(
        &db,
        vec![todo_in(1, "新设备上的", "2026-05-01 10:00:00", WORK)],
        vec![],
    );
    assert_eq!(todo_row(&db, 1).0, Some(WORK), "listId 随记录落库");

    // 旧设备上传的同一版本（平局）缺 listId：保留本地
    let s = merge(
        &db,
        vec![todo_json(1, "旧设备", "2026-05-01 10:00:00")],
        vec![],
    );
    assert_eq!(s, MergeStats::default());
    assert_eq!(todo_row(&db, 1).1, "新设备上的");
    // 旧设备改过（远端较新）缺 listId：内容取远端，归属沿用本地清单
    let s = merge(
        &db,
        vec![todo_json(1, "旧设备改过", "2026-05-02 10:00:00")],
        vec![],
    );
    assert_eq!(s.todos_updated, 1);
    assert_eq!(
        todo_row(&db, 1),
        (
            Some(WORK),
            "旧设备改过".to_string(),
            "2026-05-02 10:00:00".to_string()
        )
    );
    // 远端新增、没有 listId：未知（显示在默认清单，但不是"已知的 1"）
    merge(
        &db,
        vec![todo_json(2, "旧设备新建", "2026-05-01 10:00:00")],
        vec![],
    );
    assert_eq!(todo_row(&db, 2).0, None);
    let local = db
        .with_connection(|c| local_records(c, CUTOFF))
        .unwrap()
        .todos;
    let written: Vec<Value> = local
        .iter()
        .map(|t| serde_json::to_value(t).unwrap())
        .collect();
    assert_eq!(written[0]["listId"], WORK);
    assert!(
        written[1].get("listId").is_none(),
        "上传时未知不写 listId 键"
    );
}

/// 契约 §13-4 / 5：远端有 listId、本地未知 → 不论记录 LWW 谁胜（平局、本地较新、远端较新）都采纳远端的归属；
/// 平局 / 本地较新时只改归属、不改 updated_at 与内容。已知归属之间的取舍见下面 §13 的测试
#[test]
fn unknown_local_todos_adopt_a_known_remote_list_whoever_wins() {
    let db = db();
    merge(
        &db,
        vec![
            todo_json(1, "本地", "2026-05-01 10:00:00"),
            todo_json(2, "本地较新", "2026-05-05 10:00:00"),
            todo_json(3, "本地较旧", "2026-05-01 10:00:00"),
            todo_json(4, "两边都未知", "2026-05-01 10:00:00"),
        ],
        vec![],
    );
    for id in 1..=4 {
        assert_eq!(todo_row(&db, id).0, None, "前提：本地归属未知");
    }

    let s = merge(
        &db,
        vec![
            todo_in(1, "远端平局", "2026-05-01 10:00:00", WORK),
            todo_in(2, "远端较旧", "2026-05-01 10:00:00", LIFE),
            todo_in(3, "远端较新", "2026-05-02 10:00:00", DEFAULT_LIST_ID),
            todo_json(4, "两边都未知", "2026-05-01 10:00:00"),
        ],
        vec![],
    );
    assert_eq!(s.todos_updated, 3, "采纳归属也是本地数据变化，界面要刷新");
    assert_eq!(
        todo_row(&db, 1),
        (
            Some(WORK),
            "本地".to_string(),
            "2026-05-01 10:00:00".to_string()
        ),
        "平局：内容保留本地，采纳远端的归属，不改 updated_at"
    );
    assert_eq!(
        todo_row(&db, 2),
        (
            Some(LIFE),
            "本地较新".to_string(),
            "2026-05-05 10:00:00".to_string()
        ),
        "本地较新：同上"
    );
    assert_eq!(
        todo_row(&db, 3),
        (
            Some(DEFAULT_LIST_ID),
            "远端较新".to_string(),
            "2026-05-02 10:00:00".to_string()
        )
    );
    assert_eq!(todo_row(&db, 4).0, None, "远端也未知：仍是未知");

    // 完全相同的归属：什么都不改
    let s = merge(
        &db,
        vec![todo_in(1, "远端平局", "2026-05-01 10:00:00", WORK)],
        vec![],
    );
    assert_eq!(s, MergeStats::default());
}

fn todo_at(id: i64, title: &str, updated_at: &str, list_id: i64, list_at: &str) -> Value {
    let mut todo = todo_in(id, title, updated_at, list_id);
    todo["listUpdatedAt"] = json!(list_at);
    todo
}

/// 本地存储的归属 (list_id, list_updated_at)
fn assignment(db: &Database, id: i64) -> (Option<i64>, Option<String>) {
    db.with_connection(|c| {
        c.query_row(
            "SELECT list_id, list_updated_at FROM todos WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
    })
    .unwrap()
}

fn at(s: &str) -> Option<String> {
    Some(s.to_string())
}

/// 契约 §13-3：未知 < 已知但没有版本（缺失 / 无法识别）< 有版本的按时间比较；一样新、listId 不同时较大的胜出
/// （两种顺序结论相同）；完全相同不算更新
#[test]
fn list_assignments_order_by_their_own_version() {
    let unknown = ListAssignment::new(None, Some("2026-05-01 10:00:00"));
    assert_eq!(unknown, ListAssignment::default(), "未知时版本没有意义");
    let bare = ListAssignment::new(Some(WORK), None);
    assert_eq!(
        ListAssignment::new(Some(WORK), Some("昨天")),
        bare,
        "无法识别的版本 = 没有版本"
    );
    let early = ListAssignment::new(Some(WORK), Some("2026-05-01T10:00"));
    assert_eq!(early.list_updated_at, at("2026-05-01 10:00:00"), "规范化");
    let late = ListAssignment::new(Some(DEFAULT_LIST_ID), Some("2026-05-02 10:00:00"));
    let ordered = [&unknown, &bare, &early, &late];
    for (i, a) in ordered.iter().enumerate() {
        for (j, b) in ordered.iter().enumerate() {
            assert_eq!(a.is_newer_than(b), i > j, "{a:?} vs {b:?}");
        }
    }

    // 一样新：较大的 listId 胜出（LIFE > WORK）
    let same_time_life = ListAssignment::new(Some(LIFE), Some("2026-05-01 10:00:00"));
    assert!(same_time_life.is_newer_than(&early));
    assert_eq!(early.clone().newest(same_time_life.clone()), same_time_life);
    assert_eq!(same_time_life.clone().newest(early.clone()), same_time_life);
    let bare_life = ListAssignment::new(Some(LIFE), None);
    assert!(bare_life.is_newer_than(&bare) && !bare.is_newer_than(&bare_life));
}

/// 契约 §13（F1）：归属按自己的版本合并，与记录的 updatedAt 无关。A 把待办移到"工作"；B 拿着旧副本改了标题
/// （记录版本更新、归属陈旧）——两种合并顺序都收敛到 B 的内容 + A 的移动
#[test]
fn a_stale_copy_cannot_undo_a_newer_move_in_either_order() {
    let moved = todo_at(
        1,
        "原标题",
        "2026-05-02 10:00:00",
        WORK,
        "2026-05-02 10:00:00",
    );
    let stale_edit = todo_at(
        1,
        "B 改的标题",
        "2026-05-03 10:00:00",
        DEFAULT_LIST_ID,
        "2026-05-01 10:00:00",
    );
    let expected = (Some(WORK), at("2026-05-02 10:00:00"));

    // 本地是 A 的移动，远端来了 B 的编辑：内容取远端，归属留本地
    let db_a = db();
    merge(&db_a, vec![moved.clone()], vec![]);
    let s = merge(&db_a, vec![stale_edit.clone()], vec![]);
    assert_eq!(s.todos_updated, 1);
    assert_eq!(
        todo_row(&db_a, 1),
        (
            Some(WORK),
            "B 改的标题".to_string(),
            "2026-05-03 10:00:00".to_string()
        )
    );
    assert_eq!(assignment(&db_a, 1), expected);

    // 本地是 B 的编辑，远端来了 A 的移动：内容留本地，只采纳归属（不改 updated_at）
    let db_b = db();
    merge(&db_b, vec![stale_edit], vec![]);
    let s = merge(&db_b, vec![moved], vec![]);
    assert_eq!(s.todos_updated, 1, "采纳归属也是本地数据变化");
    assert_eq!(
        todo_row(&db_b, 1),
        (
            Some(WORK),
            "B 改的标题".to_string(),
            "2026-05-03 10:00:00".to_string()
        )
    );
    assert_eq!(assignment(&db_b, 1), expected);

    // 旧版本写的同一记录版本（没有 listId）：未知最旧，什么都不改
    let s = merge(
        &db_b,
        vec![todo_json(1, "旧设备", "2026-05-03 10:00:00")],
        vec![],
    );
    assert_eq!(s, MergeStats::default());
    assert_eq!(assignment(&db_b, 1), expected);
}

/// 契约 §13（F3）：降级到旧版本时它只改了记录（updated_at）、留下陈旧的归属；升级回来后，远端更新的归属照样胜出，
/// 记录内容仍取本机较新的那份
#[test]
fn a_downgraded_device_learns_the_newer_list_after_upgrading() {
    let db = db();
    merge(
        &db,
        vec![todo_at(
            1,
            "原标题",
            "2026-05-01 10:00:00",
            WORK,
            "2026-05-01 10:00:00",
        )],
        vec![],
    );
    // 旧版本（不认识清单）改了标题：list_id / list_updated_at 原样留下
    db.with_connection(|c| {
        c.execute(
            "UPDATE todos SET title = '旧版本改的', updated_at = '2026-05-06 10:00:00'
             WHERE id = 1",
            [],
        )
    })
    .unwrap();
    // 期间别的设备把它移到了"生活"
    let s = merge(
        &db,
        vec![todo_at(
            1,
            "原标题",
            "2026-05-05 10:00:00",
            LIFE,
            "2026-05-05 10:00:00",
        )],
        vec![],
    );
    assert_eq!(s.todos_updated, 1);
    assert_eq!(
        todo_row(&db, 1),
        (
            Some(LIFE),
            "旧版本改的".to_string(),
            "2026-05-06 10:00:00".to_string()
        )
    );
    assert_eq!(assignment(&db, 1).1, at("2026-05-05 10:00:00"));
}

/// 契约 §13-4：墓碑不影响归属的合并。
/// F2：本地版本刚被远端墓碑删掉、又被远端更新的编辑救回来——本地合并前的归属仍是可见版本；
/// F4：远端版本被墓碑压制、本地删除后又编辑过的记录还在——远端版本的归属照样参与
#[test]
fn tombstones_do_not_hide_assignment_versions() {
    let db = db();
    merge(
        &db,
        vec![
            todo_at(
                1,
                "本地",
                "2026-05-02 10:00:00",
                WORK,
                "2026-05-02 10:00:00",
            ),
            todo_at(
                2,
                "删后又改",
                "2026-05-08 10:00:00",
                DEFAULT_LIST_ID,
                "2026-05-01 10:00:00",
            ),
        ],
        vec![],
    );
    db.with_transaction(|tx| record_tombstone(tx, EntityKind::Todo, 2, "2026-05-05 10:00:00"))
        .unwrap();

    let s = merge(
        &db,
        vec![
            // F2：拿着旧副本、在删除之后编辑过（不被墓碑压制）
            todo_at(
                1,
                "删后又改",
                "2026-05-04 10:00:00",
                DEFAULT_LIST_ID,
                "2026-05-01 10:00:00",
            ),
            // F4：被本地墓碑压制的远端版本，归属比本地新
            todo_at(
                2,
                "远端旧版",
                "2026-05-04 10:00:00",
                LIFE,
                "2026-05-04 10:00:00",
            ),
        ],
        vec![tomb("todo", 1, "2026-05-03 10:00:00")],
    );
    assert_eq!(
        (s.todos_deleted, s.todos_inserted, s.todos_updated),
        (1, 1, 1)
    );
    assert_eq!(
        todo_row(&db, 1),
        (
            Some(WORK),
            "删后又改".to_string(),
            "2026-05-04 10:00:00".to_string()
        ),
        "F2：内容取远端，归属取本地合并前的版本"
    );
    assert_eq!(assignment(&db, 1).1, at("2026-05-02 10:00:00"));
    assert_eq!(
        todo_row(&db, 2),
        (
            Some(LIFE),
            "删后又改".to_string(),
            "2026-05-08 10:00:00".to_string()
        ),
        "F4：记录不动，归属取被压制的远端版本"
    );
    assert_eq!(assignment(&db, 2).1, at("2026-05-04 10:00:00"));
}

/// 契约 §13-3：两边一样新、listId 不同时取较大的 listId——两种合并顺序结论相同；没有版本的已知归属之间同理
#[test]
fn equally_new_assignments_pick_the_larger_list_id() {
    for (local, remote) in [(WORK, LIFE), (LIFE, WORK)] {
        let db = db();
        merge(
            &db,
            vec![
                todo_at(
                    1,
                    "本地",
                    "2026-05-01 10:00:00",
                    local,
                    "2026-05-01 09:00:00",
                ),
                todo_in(2, "本地", "2026-05-01 10:00:00", local),
            ],
            vec![],
        );
        merge(
            &db,
            vec![
                todo_at(
                    1,
                    "远端",
                    "2026-05-01 10:00:00",
                    remote,
                    "2026-05-01 09:00:00",
                ),
                todo_in(2, "远端", "2026-05-01 10:00:00", remote),
            ],
            vec![],
        );
        assert_eq!(assignment(&db, 1), (Some(LIFE), at("2026-05-01 09:00:00")));
        assert_eq!(assignment(&db, 2), (Some(LIFE), None));
        assert_eq!(todo_row(&db, 1).1, "本地", "记录平局保留本地");
    }
}

/// 契约 §13-11（F6）：畸形记录不参与归属合并——`listId` 类型不对、记录的 `updatedAt` 无法识别、不是对象；
/// `listUpdatedAt` 识别不了只当作没有版本
#[test]
fn malformed_remote_records_take_no_part_in_assignments() {
    let mut wrong_type = todo_at(1, "x", "2026-05-09 10:00:00", LIFE, "2099-01-01 00:00:00");
    wrong_type["listId"] = json!("生活");
    let mut bad_time = todo_at(2, "x", "2026-05-09 10:00:00", LIFE, "2099-01-01 00:00:00");
    bad_time["updatedAt"] = json!("昨天");
    for bad in [&wrong_type, &bad_time, &json!("不是对象")] {
        assert_eq!(ListAssignment::from_value(bad), None, "{bad}");
    }
    let mut odd_version = todo_in(3, "x", "2026-05-09 10:00:00", LIFE);
    odd_version["listUpdatedAt"] = json!(20260509);
    assert_eq!(
        ListAssignment::from_value(&odd_version),
        Some(ListAssignment::new(Some(LIFE), None))
    );
    let mut null_list = todo_at(4, "x", "2026-05-09 10:00:00", LIFE, "2026-05-09 10:00:00");
    null_list["listId"] = Value::Null;
    assert_eq!(
        ListAssignment::from_value(&null_list),
        Some(ListAssignment::default())
    );

    let db = db();
    merge(
        &db,
        vec![
            todo_at(
                1,
                "本地",
                "2026-05-01 10:00:00",
                WORK,
                "2026-05-01 10:00:00",
            ),
            todo_at(
                2,
                "本地",
                "2026-05-01 10:00:00",
                WORK,
                "2026-05-01 10:00:00",
            ),
        ],
        vec![],
    );
    let s = merge(&db, vec![wrong_type, bad_time], vec![]);
    assert_eq!(s.records_skipped, 2);
    for id in [1, 2] {
        assert_eq!(assignment(&db, id), (Some(WORK), at("2026-05-01 10:00:00")));
    }
}

/// 强制推送前（契约 §13）：本机未知、远端已知的归属先采纳（只改归属，连同远端的版本）；本机已知、远端的更新 →
/// 本机归属的版本推到"至少 now、且严格晚于远端那个版本"，让本机的归属胜出；本机已经更新的不动
#[test]
fn force_push_prepare_adopts_known_remote_lists_for_unknown_local_todos() {
    let db = db();
    merge(
        &db,
        vec![
            todo_json(1, "本机未知", "2026-05-09 10:00:00"),
            todo_in(2, "本机已知", "2026-05-09 10:00:00", WORK),
            todo_at(
                3,
                "本机较新",
                "2026-05-09 10:00:00",
                WORK,
                "2026-05-09 10:00:00",
            ),
            todo_at(
                4,
                "远端偏快",
                "2026-05-09 10:00:00",
                WORK,
                "2026-05-01 10:00:00",
            ),
        ],
        vec![],
    );
    let remote = vec![
        todo_at(
            1,
            "远端",
            "2026-05-01 10:00:00",
            LIFE,
            "2026-05-01 10:00:00",
        ),
        todo_in(2, "远端", "2026-05-01 10:00:00", LIFE),
        todo_at(
            3,
            "远端",
            "2026-05-01 10:00:00",
            LIFE,
            "2026-05-01 10:00:00",
        ),
        todo_at(
            4,
            "远端",
            "2026-05-01 10:00:00",
            LIFE,
            "2099-01-01 08:00:00",
        ),
    ];
    db.with_transaction(|tx| force_push_prepare(tx, &remote, &[], "2026-05-10 00:00:00"))
        .unwrap();
    assert_eq!(
        todo_row(&db, 1),
        (
            Some(LIFE),
            "本机未知".to_string(),
            "2026-05-09 10:00:00".to_string()
        )
    );
    assert_eq!(assignment(&db, 1).1, at("2026-05-01 10:00:00"));
    assert_eq!(
        assignment(&db, 2),
        (Some(WORK), at("2026-05-10 00:00:00")),
        "一样新（都没有版本）、远端的 listId 较大：本机的推到 now"
    );
    assert_eq!(
        assignment(&db, 3),
        (Some(WORK), at("2026-05-09 10:00:00")),
        "本机较新：不动"
    );
    assert_eq!(
        assignment(&db, 4),
        (Some(WORK), at("2099-01-01 08:00:01")),
        "严格晚于远端的版本"
    );
}

/// 强制拉取同样不让"未知"覆盖本地已知的归属；远端已知时照常覆盖
#[test]
fn force_pull_keeps_a_known_list_when_the_remote_does_not_know_it() {
    let db = db();
    merge(
        &db,
        vec![
            todo_in(1, "本地", "2026-05-09 10:00:00", WORK),
            todo_in(2, "本地", "2026-05-09 10:00:00", WORK),
        ],
        vec![],
    );
    let remote = parse_remote_todos(&[
        todo_json(1, "远端不认识清单", "2026-05-01 10:00:00"),
        todo_in(2, "远端已知", "2026-05-01 10:00:00", LIFE),
    ]);
    db.with_transaction(|tx| force_pull_replace(tx, &remote, &[], CUTOFF))
        .unwrap();
    assert_eq!(
        todo_row(&db, 1),
        (
            Some(WORK),
            "远端不认识清单".to_string(),
            "2026-05-01 10:00:00".to_string()
        )
    );
    assert_eq!(todo_row(&db, 2).0, Some(LIFE));
}

#[test]
fn force_pull_lists_makes_local_lists_equal_remote() {
    let db = db();
    merge_lists(
        &db,
        Some(vec![
            list_json(WORK, "本地较新", "2026-05-09 10:00:00"),
            list_json(LIFE, "本地独有", "2026-05-01 10:00:00"),
            list_json(1, "本地改名", "2026-05-01 10:00:00"),
        ]),
        vec![],
    );
    db.with_transaction(|tx| record_list_tombstone(tx, 99, "2026-05-01 10:00:00"))
        .unwrap();

    let force_pull = |lists: Option<Vec<Value>>, tombs: Vec<ListTombstone>| {
        let parsed = lists.as_deref().map(parse_remote_lists);
        db.with_transaction(|tx| force_pull_replace_lists(tx, parsed.as_deref(), &tombs))
            .unwrap()
    };

    // 远端没有清单信息：什么都不动
    assert_eq!(force_pull(None, vec![]), MergeStats::default());
    assert_eq!(count(&db, "todo_lists"), 3);
    assert_eq!(list_tombs(&db).len(), 1);

    // 远端（不含默认清单——写入方有问题）：本地独有的删掉（不写墓碑），默认清单保留，墓碑表替换为远端
    let s = force_pull(
        Some(vec![
            list_json(WORK, "远端旧版", "2026-05-01 10:00:00"),
            list_json(55, "远端自相矛盾", "2026-05-01 10:00:00"),
        ]),
        vec![
            list_tomb(55, "2026-05-02 10:00:00"),
            list_tomb(7, "2026-05-01 10:00:00"),
        ],
    );
    assert_eq!(
        (s.lists_updated, s.lists_deleted, s.lists_inserted),
        (1, 1, 0)
    );
    assert_eq!(list_name(&db, WORK).as_deref(), Some("远端旧版"));
    assert!(list_name(&db, LIFE).is_none());
    assert!(list_name(&db, 55).is_none(), "被远端自己的墓碑压制");
    assert_eq!(
        list_name(&db, 1).as_deref(),
        Some("本地改名"),
        "默认清单永远保留"
    );
    assert_eq!(
        list_tombs(&db),
        vec![
            list_tomb(7, "2026-05-01 10:00:00"),
            list_tomb(55, "2026-05-02 10:00:00")
        ]
    );
}

#[test]
fn force_push_prepare_lists_tombstones_remote_only_and_bumps_conflicts() {
    let db = db();
    merge_lists(
        &db,
        Some(vec![
            list_json(WORK, "本地旧", "2026-05-01 10:00:00"),
            list_json(LIFE, "本地", "2026-05-01 10:00:00"),
        ]),
        vec![],
    );
    let now = "2026-05-03 00:00:00";
    let remote = vec![
        list_json(WORK, "远端新", "2026-05-05 10:00:00"),
        list_json(55, "远端独有", "2099-01-01 08:00:00"),
        list_json(1, "远端改名的默认清单", "2026-05-04 00:00:00"),
    ];
    let bumped = db
        .with_transaction(|tx| {
            force_push_prepare_lists(
                tx,
                Some(&remote),
                &[list_tomb(LIFE, "2026-05-02 00:00:00")],
                now,
            )
        })
        .unwrap();
    assert_eq!(bumped, 3);
    let ua = |id: i64| -> String {
        db.with_connection(|c| {
            c.query_row(
                "SELECT updated_at FROM todo_lists WHERE id = ?1",
                [id],
                |r| r.get(0),
            )
        })
        .unwrap()
    };
    assert_eq!(ua(WORK), "2026-05-05 10:00:01", "必须严格晚于远端版本");
    assert_eq!(ua(LIFE), now, "必须压过远端墓碑");
    assert_eq!(ua(1), "2026-05-04 00:00:01");
    assert_eq!(
        list_tombs(&db),
        vec![list_tomb(55, "2099-01-01 08:00:00")],
        "远端独有的清单写墓碑，时间压得住它的 updatedAt"
    );
}

#[test]
fn parse_list_tombstone_is_lenient_but_validates() {
    assert_eq!(
        parse_list_tombstone(&json!({"entityId": WORK, "deletedAt": "2026-05-01T10:00"})),
        Some(list_tomb(WORK, "2026-05-01 10:00:00"))
    );
    assert_eq!(
        parse_list_tombstone(&json!({"entityId": "42", "deletedAt": "2026-05-01"})),
        Some(list_tomb(42, "2026-05-01 00:00:00"))
    );
    for bad in [
        json!({"entityId": "x", "deletedAt": "2026-05-01"}),
        json!({"entityId": 5, "deletedAt": "yesterday"}),
        json!({"entityId": 5}),
        json!({"deletedAt": "2026-05-01"}),
        json!("not an object"),
    ] {
        assert_eq!(parse_list_tombstone(&bad), None, "{bad}");
    }
}

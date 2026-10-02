//! `sync-data.json.gz` 文档模型（跨端契约 K2）。
//!
//! ```jsonc
//! {
//!   "version": "4.0", "deviceId": "…", "updatedAt": "…",   // 元信息
//!   "todos": [ /* Todo（嵌套 subtasks） */ ],
//!   "settings": { /* PC AppSettings，cloud 原样透传 */ },
//!   "settingsUpdatedAt": "YYYY-MM-DD HH:MM:SS",               // 可缺省，cloud 原样透传
//!   "images": ["name.png"],
//!   "tombstones": [ { "entityType": "todo" | "subtask", "entityId": 123, "deletedAt": "…" } ]
//!   // 其它任何顶层键：原样保留（前向兼容）
//! }
//! ```
//!
//! 云端上传时**以远端文档为底**：除 `todos / images / tombstones / updatedAt /
//! version / deviceId` 外的顶层键（含 `settings`、`settingsUpdatedAt` 与未知键）
//! 一律原样保留。远端 304 时拿不到正文，所以把基准版本的这部分（"信封"）存在
//! `meta.remote_envelope`，见 `sync::pull`。

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use chrono::NaiveTime;
use chrono_tz::Tz;
use rusqlite::Connection;
use serde_json::{json, Map, Value};
use tracing::{debug, warn};

use crate::db::repo;
use crate::sync::images::is_safe_image_name;
use crate::time::{normalize_datetime, now_iso_string};

/// 云端写出的 sync-data 格式版本（远端文档带了更高版本号时保留远端的）。
pub const SYNC_FORMAT_VERSION: &str = "4.0";
/// 云端写出文档时的 `deviceId`。
pub const CLOUD_DEVICE_ID: &str = "minitodo-cloud";
/// 墓碑保留期（天）。写出时丢弃更早的墓碑，本地墓碑表同样按此清理。
pub const TOMBSTONE_RETENTION_DAYS: i64 = 30;

/// 墓碑的实体类型。todo 与 subtask 的 id 是两个独立命名空间。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EntityType {
    Todo,
    Subtask,
}

impl EntityType {
    pub fn as_str(self) -> &'static str {
        match self {
            EntityType::Todo => "todo",
            EntityType::Subtask => "subtask",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(EntityType::Todo),
            "subtask" => Some(EntityType::Subtask),
            _ => None,
        }
    }
}

/// 一条（已校验、时间已规范化的）墓碑。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tombstone {
    pub entity_type: EntityType,
    /// 十进制 i64 字符串（与本地 `tombstones.entity_id` / 记录主键一致）。
    pub entity_id: String,
    /// 规范格式 `YYYY-MM-DD HH:MM:SS`。
    pub deleted_at: String,
}

impl Tombstone {
    /// 从远端 JSON 解析。不合法（类型未知 / id 不是整数 / 时间无法解析）→ `None`。
    pub fn from_value(v: &Value, tz: Tz) -> Option<Self> {
        let entity_type = EntityType::parse(v.get("entityType")?.as_str()?)?;
        let entity_id = match v.get("entityId")? {
            Value::Number(n) => n.as_i64()?,
            Value::String(s) => s.trim().parse::<i64>().ok()?,
            _ => return None,
        };
        let deleted_at = normalize_datetime(v.get("deletedAt")?.as_str()?, tz, NaiveTime::MIN)?;
        Some(Tombstone {
            entity_type,
            entity_id: entity_id.to_string(),
            deleted_at,
        })
    }

    /// 写出到 sync-data 的形态。`entityId` 必须是整数（PC 端是 i64），否则不写出。
    pub fn to_value(&self) -> Option<Value> {
        let id = self.entity_id.parse::<i64>().ok()?;
        Some(json!({
            "entityType": self.entity_type.as_str(),
            "entityId": id,
            "deletedAt": self.deleted_at,
        }))
    }
}

/// 解析后的远端文档。
#[derive(Debug, Clone)]
pub struct RemoteDoc {
    map: Map<String, Value>,
}

impl RemoteDoc {
    /// 解析远端 JSON。读不懂的文档一律报错——调用方绝不能拿自己的数据覆盖它。
    pub fn parse(json: &str) -> anyhow::Result<Self> {
        let v: Value = serde_json::from_str(json)
            .map_err(|e| anyhow::anyhow!("解析 sync-data 失败: {}", e))?;
        Self::from_value(v)
    }

    pub fn from_value(v: Value) -> anyhow::Result<Self> {
        let Value::Object(map) = v else {
            anyhow::bail!("sync-data 顶层不是 JSON 对象");
        };
        for key in ["todos", "tombstones", "images"] {
            if let Some(val) = map.get(key) {
                if !val.is_array() && !val.is_null() {
                    anyhow::bail!("sync-data 的 `{}` 不是数组", key);
                }
            }
        }
        Ok(RemoteDoc { map })
    }

    pub fn todos(&self) -> &[Value] {
        self.map
            .get("todos")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// 是否带 `tombstones` 键（新协议写入方一定会写，哪怕是空数组）。
    /// 没有这个键 = 旧版 PC 写的文档，pull 才允许走"缺席即删除"的兼容路径。
    pub fn has_tombstones_key(&self) -> bool {
        self.map.contains_key("tombstones")
    }

    /// 合法的远端墓碑 + 被跳过的非法条目数。
    pub fn tombstones(&self, tz: Tz) -> (Vec<Tombstone>, usize) {
        let mut out = Vec::new();
        let mut invalid = 0;
        if let Some(arr) = self.map.get("tombstones").and_then(Value::as_array) {
            for v in arr {
                match Tombstone::from_value(v, tz) {
                    Some(t) => out.push(t),
                    None => invalid += 1,
                }
            }
        }
        (out, invalid)
    }

    pub fn settings(&self) -> Option<&Value> {
        self.map.get("settings")
    }

    /// 信封：除 `todos` / `tombstones` 外的全部顶层键。
    pub fn envelope(&self) -> Map<String, Value> {
        let mut env = self.map.clone();
        env.remove("todos");
        env.remove("tombstones");
        env
    }
}

/// 信封里 `images` 清单中合法（K5）的文件名。
pub fn envelope_images(envelope: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(arr) = envelope.get("images").and_then(Value::as_array) {
        for v in arr {
            match v.as_str() {
                Some(name) if is_safe_image_name(name) => out.push(name.to_string()),
                other => debug!(
                    target: "minitodo_cloud::sync",
                    "skip unsafe image name in remote list: {:?}", other
                ),
            }
        }
    }
    out
}

/// 远端缺 `settings` 时写的最小合法 PC `AppSettings`：`isFixed` / `windowPosition` /
/// `windowSize` 在 PC 端没有 `serde(default)`，缺失会导致 PC 反序列化失败。
/// 写占位时**绝不**同时写 `settingsUpdatedAt`（K3-5：PC 因而不会应用占位）。
pub fn placeholder_settings() -> Value {
    json!({
        "isFixed": false,
        "windowPosition": null,
        "windowSize": null,
    })
}

/// 按 `sortOrder` 升序、再按 id 升序（数字 id 按数值）排序，保证输出稳定。
fn cmp_records(a: &Value, b: &Value) -> Ordering {
    let order = |v: &Value| v.get("sortOrder").and_then(Value::as_i64).unwrap_or(0);
    order(a).cmp(&order(b)).then_with(|| {
        match (
            a.get("id").and_then(Value::as_i64),
            b.get("id").and_then(Value::as_i64),
        ) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => crate::util::id_string(a).cmp(&crate::util::id_string(b)),
        }
    })
}

/// 用本地 SQLite 当前状态 + 基准信封构造要上传的文档（K2）。
///
/// - `todos`：本地全部 todo（嵌套各自的 subtasks）。本地状态已经合并过基准版本，
///   所以它就是"远端 ∪ 本地"按 K3 合并后的结果。
/// - `tombstones`：本地墓碑表（已包含合并进来的远端墓碑），按保留期过滤。
/// - `images`：调用方算好的清单。
/// - 其它顶层键来自 `envelope`，原样保留；`settings` 缺失时写占位并去掉
///   `settingsUpdatedAt`。
pub fn build_outgoing_doc(
    conn: &Connection,
    envelope: &Map<String, Value>,
    images: Vec<String>,
    tz: Tz,
    retention_cutoff: &str,
) -> rusqlite::Result<Map<String, Value>> {
    let mut subs_by_todo: HashMap<String, Vec<Value>> = HashMap::new();
    for row in repo::all_subtasks(conn)? {
        match serde_json::from_str::<Value>(&row.data_json) {
            Ok(v) if v.is_object() => subs_by_todo.entry(row.todo_id).or_default().push(v),
            _ => warn!(target: "minitodo_cloud::push", "skip corrupt subtask row {}", row.id),
        }
    }

    let mut todos = Vec::new();
    for row in repo::all_todos(conn)? {
        let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(&row.data_json) else {
            warn!(target: "minitodo_cloud::push", "skip corrupt todo row {}", row.id);
            continue;
        };
        let mut subs = subs_by_todo.remove(&row.id).unwrap_or_default();
        subs.sort_by(cmp_records);
        obj.insert("subtasks".into(), Value::Array(subs));
        todos.push(Value::Object(obj));
    }
    todos.sort_by(cmp_records);
    if !subs_by_todo.is_empty() {
        debug!(
            target: "minitodo_cloud::push",
            "{} subtask group(s) without parent todo were not exported",
            subs_by_todo.len()
        );
    }

    let mut tombs: Vec<Tombstone> = Vec::new();
    for (typ, id, deleted_at) in repo::list_tombstones(conn)? {
        let Some(entity_type) = EntityType::parse(&typ) else {
            continue;
        };
        let Some(deleted_at) = normalize_datetime(&deleted_at, tz, NaiveTime::MIN) else {
            continue;
        };
        if deleted_at.as_str() < retention_cutoff {
            continue;
        }
        tombs.push(Tombstone {
            entity_type,
            entity_id: id,
            deleted_at,
        });
    }
    tombs.sort_by(|a, b| {
        a.entity_type.cmp(&b.entity_type).then_with(|| {
            let ai = a.entity_id.parse::<i64>().unwrap_or(i64::MAX);
            let bi = b.entity_id.parse::<i64>().unwrap_or(i64::MAX);
            ai.cmp(&bi)
        })
    });
    let tombstones: Vec<Value> = tombs.iter().filter_map(Tombstone::to_value).collect();

    let mut doc = envelope.clone();
    let version = doc
        .get("version")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(SYNC_FORMAT_VERSION)
        .to_string();
    doc.insert("version".into(), json!(version));
    doc.insert("deviceId".into(), json!(CLOUD_DEVICE_ID));
    doc.insert("updatedAt".into(), json!(now_iso_string(tz)));
    if !doc.get("settings").is_some_and(Value::is_object) {
        doc.insert("settings".into(), placeholder_settings());
        doc.remove("settingsUpdatedAt");
    }
    doc.insert("todos".into(), Value::Array(todos));
    doc.insert(
        "images".into(),
        Value::Array(images.into_iter().map(Value::String).collect()),
    );
    doc.insert("tombstones".into(), Value::Array(tombstones));
    Ok(doc)
}

/// 去重、过滤 K5 非法名、排序。
pub fn normalize_image_list(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let set: HashSet<String> = names
        .into_iter()
        .filter(|n| is_safe_image_name(n))
        .collect();
    let mut out: Vec<String> = set.into_iter().collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tz() -> Tz {
        "Asia/Shanghai".parse().unwrap()
    }

    #[test]
    fn tombstone_parse_accepts_numeric_and_string_ids() {
        let t = Tombstone::from_value(
            &json!({"entityType": "todo", "entityId": 12, "deletedAt": "2026-05-13T10:00:00"}),
            tz(),
        )
        .unwrap();
        assert_eq!(t.entity_type, EntityType::Todo);
        assert_eq!(t.entity_id, "12");
        assert_eq!(t.deleted_at, "2026-05-13 10:00:00");
        let t2 = Tombstone::from_value(
            &json!({"entityType": "subtask", "entityId": "7", "deletedAt": "2026-05-13 10:00:00"}),
            tz(),
        )
        .unwrap();
        assert_eq!(t2.entity_id, "7");
        assert_eq!(
            t2.to_value().unwrap(),
            json!({"entityType": "subtask", "entityId": 7, "deletedAt": "2026-05-13 10:00:00"})
        );
    }

    #[test]
    fn tombstone_parse_rejects_invalid() {
        for bad in [
            json!({"entityType": "note", "entityId": 1, "deletedAt": "2026-05-13 10:00:00"}),
            json!({"entityType": "todo", "entityId": "abc", "deletedAt": "2026-05-13 10:00:00"}),
            json!({"entityType": "todo", "entityId": 1.5, "deletedAt": "2026-05-13 10:00:00"}),
            json!({"entityType": "todo", "entityId": 1, "deletedAt": "yesterday"}),
            json!({"entityType": "todo", "entityId": 1}),
            json!("todo:1"),
        ] {
            assert!(Tombstone::from_value(&bad, tz()).is_none(), "{}", bad);
        }
    }

    #[test]
    fn remote_doc_rejects_unreadable_shapes() {
        assert!(RemoteDoc::parse("not json").is_err());
        assert!(RemoteDoc::parse("[1,2]").is_err());
        assert!(RemoteDoc::parse(r#"{"todos": {"a": 1}}"#).is_err());
        assert!(RemoteDoc::parse(r#"{"tombstones": "x"}"#).is_err());
        let ok = RemoteDoc::parse(r#"{"todos": null}"#).unwrap();
        assert!(ok.todos().is_empty());
        assert!(!ok.has_tombstones_key());
    }

    #[test]
    fn envelope_keeps_everything_but_records() {
        let doc = RemoteDoc::from_value(json!({
            "version": "4.0",
            "todos": [],
            "tombstones": [],
            "settings": {"isFixed": true},
            "settingsUpdatedAt": "2026-05-13 10:00:00",
            "futureKey": {"nested": [1, 2]},
            "images": ["a.png"]
        }))
        .unwrap();
        let env = doc.envelope();
        assert!(!env.contains_key("todos"));
        assert!(!env.contains_key("tombstones"));
        assert_eq!(env["futureKey"], json!({"nested": [1, 2]}));
        assert_eq!(env["settingsUpdatedAt"], json!("2026-05-13 10:00:00"));
        assert_eq!(env["settings"], json!({"isFixed": true}));
    }

    #[test]
    fn envelope_images_filters_unsafe_names() {
        let env = json!({"images": ["ok.png", "../etc/passwd", "a b.png", 3, ".hidden.png"]});
        assert_eq!(envelope_images(env.as_object().unwrap()), vec!["ok.png"]);
    }

    #[test]
    fn placeholder_settings_has_pc_required_fields() {
        // PC 端 AppSettings 必填字段：isFixed / windowPosition / windowSize
        let v = placeholder_settings();
        assert!(v.get("isFixed").is_some());
        assert!(v.get("windowPosition").is_some());
        assert!(v.get("windowSize").is_some());
    }

    fn mem_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::db::schema::init(&c).unwrap();
        c
    }

    #[test]
    fn outgoing_doc_preserves_envelope_and_replaces_owned_keys() {
        let c = mem_db();
        repo::upsert_todo(
            &c,
            "1",
            r#"{"id":1,"title":"a","sortOrder":2,"updatedAt":"2026-05-13 10:00:00"}"#,
            "2026-05-13 10:00:00",
        )
        .unwrap();
        repo::upsert_todo(
            &c,
            "2",
            r#"{"id":2,"title":"b","sortOrder":1,"updatedAt":"2026-05-13 10:00:00"}"#,
            "2026-05-13 10:00:00",
        )
        .unwrap();
        repo::upsert_subtask(
            &c,
            "9",
            "1",
            r#"{"id":9,"parentId":1,"title":"s","updatedAt":"2026-05-13 10:00:00"}"#,
            "2026-05-13 10:00:00",
        )
        .unwrap();
        let env = json!({
            "version": "4.1",
            "deviceId": "dev_pc",
            "updatedAt": "old",
            "settings": {"isFixed": true, "theme": "x"},
            "settingsUpdatedAt": "2026-05-01 00:00:00",
            "futureKey": [1, 2, 3]
        });
        let doc = build_outgoing_doc(
            &c,
            env.as_object().unwrap(),
            vec!["a.png".into()],
            tz(),
            "2000-01-01 00:00:00",
        )
        .unwrap();
        assert_eq!(doc["version"], json!("4.1"));
        assert_eq!(doc["deviceId"], json!(CLOUD_DEVICE_ID));
        assert_ne!(doc["updatedAt"], json!("old"));
        assert_eq!(doc["settings"], json!({"isFixed": true, "theme": "x"}));
        assert_eq!(doc["settingsUpdatedAt"], json!("2026-05-01 00:00:00"));
        assert_eq!(doc["futureKey"], json!([1, 2, 3]));
        assert_eq!(doc["images"], json!(["a.png"]));
        assert_eq!(doc["tombstones"], json!([]));
        let todos = doc["todos"].as_array().unwrap();
        // sortOrder 升序
        assert_eq!(todos[0]["id"], json!(2));
        assert_eq!(todos[1]["id"], json!(1));
        assert_eq!(todos[1]["subtasks"][0]["id"], json!(9));
        assert_eq!(todos[0]["subtasks"], json!([]));
    }

    #[test]
    fn outgoing_doc_writes_placeholder_without_settings_updated_at() {
        let c = mem_db();
        let env = json!({"settingsUpdatedAt": "2026-05-01 00:00:00"});
        let doc = build_outgoing_doc(
            &c,
            env.as_object().unwrap(),
            vec![],
            tz(),
            "2000-01-01 00:00:00",
        )
        .unwrap();
        assert_eq!(doc["settings"], placeholder_settings());
        assert!(
            !doc.contains_key("settingsUpdatedAt"),
            "占位 settings 绝不能带 settingsUpdatedAt，否则 PC 会应用占位"
        );
        assert_eq!(doc["version"], json!(SYNC_FORMAT_VERSION));
        // PC 旧版 SyncData 的必填键全部存在
        for key in [
            "version",
            "deviceId",
            "updatedAt",
            "todos",
            "settings",
            "images",
        ] {
            assert!(doc.contains_key(key), "missing {}", key);
        }
    }

    #[test]
    fn outgoing_doc_tombstones_respect_retention_and_shape() {
        let c = mem_db();
        repo::add_tombstone(&c, "todo", "5", "2026-09-30 10:00:00").unwrap();
        repo::add_tombstone(&c, "subtask", "6", "2026-09-30T11:00:00").unwrap();
        repo::add_tombstone(&c, "todo", "7", "2026-01-01 10:00:00").unwrap(); // 过期
        repo::add_tombstone(&c, "todo", "not-a-number", "2026-09-30 10:00:00").unwrap();
        let doc = build_outgoing_doc(&c, &Map::new(), vec![], tz(), "2026-09-01 00:00:00").unwrap();
        assert_eq!(
            doc["tombstones"],
            json!([
                {"entityType": "todo", "entityId": 5, "deletedAt": "2026-09-30 10:00:00"},
                {"entityType": "subtask", "entityId": 6, "deletedAt": "2026-09-30 11:00:00"}
            ])
        );
    }

    #[test]
    fn normalize_image_list_dedups_filters_and_sorts() {
        assert_eq!(
            normalize_image_list(vec![
                "b.png".to_string(),
                "a.png".to_string(),
                "b.png".to_string(),
                "../x.png".to_string()
            ]),
            vec!["a.png", "b.png"]
        );
    }
}

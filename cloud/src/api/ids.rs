//! 新建记录的 id：与 PC 端 `INTEGER PRIMARY KEY AUTOINCREMENT` 共用一个 i64 命名空间。
//!
//! 取值 = 当前 UNIX 毫秒 × 1000 + 随机 0..999：
//! - 大致按时间递增（毫秒分量），同一毫秒内的并发靠随机后缀区分；真撞上已有 id 时
//!   新建走普通 INSERT、遇主键冲突换一个 id 重试（`insert_with_fresh_id`），绝不覆盖
//!   已有记录（旧实现用 `ON CONFLICT DO UPDATE`，撞号会静默覆盖别的记录）
//! - 量级：2026 年约 1.8e15。PC 前端是 JavaScript，id 必须小于 JS 安全整数上限
//!   2^53 − 1 ≈ 9.007e15，否则 `JSON.parse` 丢精度、id 错乱；毫秒 × 1000 在 2255 年
//!   之前都满足（旧注释写"year 9999 ≈ 2.5e14"漏乘了 1000，实际约 2.5e17，早已越界）。
//!   万一越界，退回到安全范围内的随机值
//! - PC AUTOINCREMENT 从 1 起步；合并进 PC 的云端 id 会把 PC 的序列推到 ~1.8e15，之后
//!   PC 新建的 id 每条 +1，而云端的 id 每毫秒前进 1000，两者实际不会相撞

use chrono::Utc;
use rand::Rng;

use crate::db::repo::is_primary_key_conflict;

/// JavaScript 安全整数上限（`Number.MAX_SAFE_INTEGER`）。
pub const MAX_SAFE_ID: i64 = (1 << 53) - 1;
/// 新建记录时 id 冲突的最大重试次数。
pub const MAX_ID_ATTEMPTS: usize = 8;
/// 越界兜底时随机 id 的下限（保持与时间型 id 同一量级，远离 PC 的小 id）。
const FALLBACK_MIN_ID: i64 = 1_000_000_000_000_000;

/// 毫秒 × 1000 + 后缀；超出 JS 安全整数范围时返回 `None`。
pub fn compose_id(unix_millis: i64, suffix: i64) -> Option<i64> {
    unix_millis
        .checked_mul(1000)
        .and_then(|v| v.checked_add(suffix))
        .filter(|id| (1..=MAX_SAFE_ID).contains(id))
}

pub fn new_id() -> i64 {
    let mut rng = rand::thread_rng();
    compose_id(Utc::now().timestamp_millis(), rng.gen_range(0..1000))
        .unwrap_or_else(|| rng.gen_range(FALLBACK_MIN_ID..=MAX_SAFE_ID))
}

/// 用 `next_id` 生成 id 并调用 `insert`（普通 INSERT）；主键冲突时换一个 id 重试，
/// 最多 `MAX_ID_ATTEMPTS` 次。其它错误立即返回。成功返回最终使用的 id。
pub fn insert_with_fresh_id(
    mut next_id: impl FnMut() -> i64,
    mut insert: impl FnMut(i64) -> rusqlite::Result<()>,
) -> rusqlite::Result<i64> {
    let mut last_err = None;
    for _ in 0..MAX_ID_ATTEMPTS {
        let id = next_id();
        match insert(id) {
            Ok(()) => return Ok(id),
            Err(e) if is_primary_key_conflict(&e) => {
                tracing::debug!(target: "minitodo_cloud::api", "id {} already taken, retrying", id);
                last_err = Some(e);
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err.expect("MAX_ID_ATTEMPTS > 0"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::repo;
    use chrono::TimeZone;
    use rusqlite::Connection;

    fn millis(y: i32, m: u32, d: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn ids_are_js_safe_until_2255() {
        let now = new_id();
        assert!(now > FALLBACK_MIN_ID && now <= MAX_SAFE_ID, "{}", now);
        assert!(compose_id(millis(2026, 10, 2), 999).unwrap() < 2_000_000_000_000_000);
        assert!(compose_id(millis(2255, 1, 1), 999).is_some());
        assert_eq!(
            compose_id(millis(2256, 1, 1), 0),
            None,
            "越界时不生成不安全的 id"
        );
        assert_eq!(compose_id(i64::MAX, 0), None);
    }

    fn mem_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        crate::db::schema::init(&c).unwrap();
        c
    }

    #[test]
    fn conflicting_id_is_regenerated_without_overwriting() {
        let c = mem_db();
        repo::insert_todo(&c, "100", r#"{"id":100,"title":"existing"}"#, "t").unwrap();
        let mut candidates = vec![100, 100, 200].into_iter();
        let id = insert_with_fresh_id(
            || candidates.next().unwrap(),
            |id| repo::insert_todo(&c, &id.to_string(), &format!(r#"{{"id":{}}}"#, id), "t"),
        )
        .unwrap();
        assert_eq!(id, 200);
        let existing = repo::get_todo(&c, "100").unwrap().unwrap();
        assert!(
            existing.data_json.contains("existing"),
            "已有记录不能被覆盖"
        );
    }

    #[test]
    fn gives_up_after_bounded_attempts_and_propagates_other_errors() {
        let c = mem_db();
        repo::insert_todo(&c, "1", "{}", "t").unwrap();
        let mut calls = 0;
        let err = insert_with_fresh_id(
            || 1,
            |id| {
                calls += 1;
                repo::insert_todo(&c, &id.to_string(), "{}", "t")
            },
        )
        .unwrap_err();
        assert!(repo::is_primary_key_conflict(&err));
        assert_eq!(calls, MAX_ID_ATTEMPTS);

        let mut calls = 0;
        let err = insert_with_fresh_id(
            || 2,
            |_| {
                calls += 1;
                c.execute("INSERT INTO no_such_table VALUES (1)", [])
                    .map(|_| ())
            },
        )
        .unwrap_err();
        assert!(!repo::is_primary_key_conflict(&err));
        assert_eq!(calls, 1);
    }
}

//! 新建同步记录的 id（目前用于待办清单）：与 cloud `src/api/ids.rs` 同一算法、常量逐字相同。
//!
//! cloud 不依赖 core，两端各自一份；改这里的算法或常量时同步改 cloud（契约见
//! `.trellis/tasks/11-09-multi-list-windows/contract.md` §7）。
//!
//! 取值 = 当前 UNIX 毫秒 × 1000 + 随机 0..=999：
//! - 大致按时间递增，同一毫秒内的并发靠随机后缀区分；两台设备离线各建一条也几乎不会撞号。真撞上已有 id 时
//!   走普通 INSERT、遇主键冲突换一个 id 重试（[`insert_with_fresh_id`]，最多 [`MAX_ID_ATTEMPTS`] 次），绝不覆盖已有记录
//! - 必须小于 JavaScript 安全整数上限 2^53 − 1（旧前端 / 其它 JSON 消费方按 double 解析数字），毫秒 × 1000
//!   在 2255 年之前都满足；万一越界（时钟异常），退回 `[FALLBACK_MIN_ID, MAX_SAFE_ID]` 内的随机值
//!
//! 随机数只求"不撞号"、不用于安全场景：用标准库 `RandomState`（每个实例的种子不同）混入计数与纳秒，
//! 不为此引入新的依赖。

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

/// JavaScript 安全整数上限（`Number.MAX_SAFE_INTEGER`）
pub const MAX_SAFE_ID: i64 = (1 << 53) - 1;
/// 新建记录时 id 冲突的最大重试次数
pub const MAX_ID_ATTEMPTS: usize = 8;
/// 越界兜底时随机 id 的下限（保持与时间型 id 同一量级，远离 PC 待办 `AUTOINCREMENT` 的小 id）
pub const FALLBACK_MIN_ID: i64 = 1_000_000_000_000_000;

/// 毫秒 × 1000 + 后缀；超出 `1..=MAX_SAFE_ID` 时返回 `None`
pub fn compose_id(unix_millis: i64, suffix: i64) -> Option<i64> {
    unix_millis
        .checked_mul(1000)
        .and_then(|v| v.checked_add(suffix))
        .filter(|id| (1..=MAX_SAFE_ID).contains(id))
}

/// 一个新 id（见模块说明）
pub fn new_id() -> i64 {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    compose_id(millis, (random_u64() % 1000) as i64).unwrap_or_else(|| fallback_id(random_u64()))
}

/// 越界兜底：`[FALLBACK_MIN_ID, MAX_SAFE_ID]` 内的随机值
fn fallback_id(random: u64) -> i64 {
    let span = (MAX_SAFE_ID - FALLBACK_MIN_ID + 1) as u64;
    FALLBACK_MIN_ID + (random % span) as i64
}

/// 非加密的随机 u64（只求不撞号）
pub(crate) fn random_u64() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    // RandomState 每个实例的种子都不同（进程级随机 + 递增），再混入计数与纳秒
    let mut hasher = RandomState::new().build_hasher();
    hasher.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    hasher.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default(),
    );
    hasher.finish()
}

/// 主键（或唯一键）冲突：换一个 id 重试的条件（与 cloud `repo::is_primary_key_conflict` 相同）
pub fn is_primary_key_conflict(e: &rusqlite::Error) -> bool {
    matches!(
        e,
        rusqlite::Error::SqliteFailure(err, _)
            if err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                || err.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
    )
}

/// 用 `next_id` 生成 id 并调用 `insert`（普通 INSERT）；主键冲突时换一个 id 重试，最多 [`MAX_ID_ATTEMPTS`] 次。
/// 其它错误立即返回。成功返回最终使用的 id。
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
                log::debug!("[ids] id {} 已被占用，换一个重试", id);
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
    use chrono::{TimeZone, Utc};
    use rusqlite::Connection;

    fn millis(y: i32, m: u32, d: u32) -> i64 {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0)
            .unwrap()
            .timestamp_millis()
    }

    /// 与 cloud `ids_are_js_safe_until_2255` 同一组断言：两端算法一致
    #[test]
    fn ids_are_js_safe_until_2255() {
        let now = new_id();
        assert!(now > FALLBACK_MIN_ID && now <= MAX_SAFE_ID, "{now}");
        assert!(compose_id(millis(2026, 10, 2), 999).unwrap() < 2_000_000_000_000_000);
        assert!(compose_id(millis(2255, 1, 1), 999).is_some());
        assert_eq!(
            compose_id(millis(2256, 1, 1), 0),
            None,
            "越界时不生成不安全的 id"
        );
        assert_eq!(compose_id(i64::MAX, 0), None);
        assert_eq!(compose_id(0, 0), None, "0 不是合法 id");
        assert_eq!(
            compose_id(1_731_000_000_000, 123),
            Some(1_731_000_000_000_123)
        );
    }

    #[test]
    fn new_ids_use_the_current_millisecond_and_a_suffix_below_1000() {
        let before = Utc::now().timestamp_millis();
        let id = new_id();
        let after = Utc::now().timestamp_millis();
        assert!((before * 1000..=after * 1000 + 999).contains(&id), "{id}");
        // 同一毫秒内连续生成：后缀随机，不保证不同，但不应总是相同
        let ids: std::collections::HashSet<i64> = (0..50).map(|_| new_id()).collect();
        assert!(ids.len() > 1, "{ids:?}");
    }

    #[test]
    fn fallback_ids_stay_in_the_safe_range() {
        for r in [0, 1, u64::MAX, u64::MAX / 2, 7_007_199_254_740_991] {
            let id = fallback_id(r);
            assert!((FALLBACK_MIN_ID..=MAX_SAFE_ID).contains(&id), "{r} -> {id}");
        }
    }

    fn mem_db() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        c.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT NOT NULL)")
            .unwrap();
        c
    }

    fn insert(c: &Connection, id: i64, v: &str) -> rusqlite::Result<()> {
        c.execute(
            "INSERT INTO t (id, v) VALUES (?1, ?2)",
            rusqlite::params![id, v],
        )
        .map(|_| ())
    }

    #[test]
    fn conflicting_id_is_regenerated_without_overwriting() {
        let c = mem_db();
        insert(&c, 100, "existing").unwrap();
        let mut candidates = vec![100, 100, 200].into_iter();
        let id = insert_with_fresh_id(|| candidates.next().unwrap(), |id| insert(&c, id, "new"))
            .unwrap();
        assert_eq!(id, 200);
        let existing: String = c
            .query_row("SELECT v FROM t WHERE id = 100", [], |r| r.get(0))
            .unwrap();
        assert_eq!(existing, "existing", "已有记录不能被覆盖");
    }

    #[test]
    fn gives_up_after_bounded_attempts_and_propagates_other_errors() {
        let c = mem_db();
        insert(&c, 1, "x").unwrap();
        let mut calls = 0;
        let err = insert_with_fresh_id(
            || 1,
            |id| {
                calls += 1;
                insert(&c, id, "y")
            },
        )
        .unwrap_err();
        assert!(is_primary_key_conflict(&err));
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
        assert!(!is_primary_key_conflict(&err));
        assert_eq!(calls, 1);
    }
}

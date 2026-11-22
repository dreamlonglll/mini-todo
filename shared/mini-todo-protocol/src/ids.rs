//! 新建同步记录的分布式 ID（PC 端与 Cloud 端共用同一算法与常量）。
//!
//! 取值 = 当前 UNIX 毫秒 × 1000 + 随机 0..=999：
//! - 大致按时间递增，同一毫秒内的并发靠随机后缀区分；两台设备离线各建一条几乎不会撞号
//! - 必须小于 JavaScript 安全整数上限 `2^53 - 1`（2255 年前恒满足）；时钟异常越界时退回
//!   `[FALLBACK_MIN_ID, MAX_SAFE_ID]` 内的随机值

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

/// JavaScript 安全整数上限（`Number.MAX_SAFE_INTEGER`）
pub const MAX_SAFE_ID: i64 = (1 << 53) - 1;
/// 新建记录时 ID 冲突的最大重试次数
pub const MAX_ID_ATTEMPTS: usize = 8;
/// 越界兜底时随机 ID 的下限（保持与时间型 ID 同一量级，远离 PC AUTOINCREMENT 的小 ID）
pub const FALLBACK_MIN_ID: i64 = 1_000_000_000_000_000;

/// 毫秒 × 1000 + 后缀；超出 `1..=MAX_SAFE_ID` 时返回 `None`
pub fn compose_id(unix_millis: i64, suffix: i64) -> Option<i64> {
    unix_millis
        .checked_mul(1000)
        .and_then(|v| v.checked_add(suffix))
        .filter(|id| (1..=MAX_SAFE_ID).contains(id))
}

/// 生成一个新 ID
pub fn new_id() -> i64 {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    compose_id(millis, (random_u64() % 1000) as i64).unwrap_or_else(|| fallback_id(random_u64()))
}

/// 越界兜底：`[FALLBACK_MIN_ID, MAX_SAFE_ID]` 内的随机值
pub fn fallback_id(random: u64) -> i64 {
    let span = (MAX_SAFE_ID - FALLBACK_MIN_ID + 1) as u64;
    FALLBACK_MIN_ID + (random % span) as i64
}

/// 非加密的随机 `u64`（只求不撞号，不用于安全场景）
pub fn random_u64() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_and_fallback_stay_within_js_safe_range() {
        assert_eq!(
            compose_id(1_731_000_000_000, 123),
            Some(1_731_000_000_000_123)
        );
        assert_eq!(compose_id(0, 0), None);
        assert_eq!(compose_id(i64::MAX, 0), None);
        for r in [0, 1, u64::MAX, u64::MAX / 2, 7_007_199_254_740_991] {
            let id = fallback_id(r);
            assert!((FALLBACK_MIN_ID..=MAX_SAFE_ID).contains(&id), "{r} -> {id}");
        }
        let generated = new_id();
        assert!(generated > FALLBACK_MIN_ID && generated <= MAX_SAFE_ID);
    }
}

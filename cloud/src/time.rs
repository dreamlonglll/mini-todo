//! 时间格式化工具。
//!
//! **核心契约**：所有 `updated_at` 字符串必须与 PC SQLite
//! `datetime('now','localtime')` 完全一致，即 `"%Y-%m-%d %H:%M:%S"`、
//! 不带时区后缀、按"本地时区现在"取墙钟时间。云端没有 OS 级 localtime，
//! 改用 `chrono::FixedOffset` 模拟用户在 config 里指定的时区即可。

use chrono::{FixedOffset, Offset, TimeZone, Utc};
use chrono_tz::Tz;

/// 取出 `tz` 当前时刻相对 UTC 的 `FixedOffset`。
///
/// 由于 DST 切换时 offset 会变，理论上每次都应当用 `Tz` 重新算；但 mini-todo
/// 当前唯一支持时区是 Asia/Shanghai（无 DST），其他用户场景也基本都是无 DST
/// 时区，所以服务启动时取一次即可。后续如需精确支持 DST，可改用
/// `now_local_string_in_tz` 直接传 `Tz`。
pub fn offset_for_tz_now(tz: Tz) -> FixedOffset {
    let now_utc = Utc::now();
    tz.from_utc_datetime(&now_utc.naive_utc()).offset().fix()
}

/// 返回与 PC SQLite `datetime('now','localtime')` 字符串格式一致的时间戳。
///
/// 示例：`"2026-05-13 12:34:56"`。
pub fn now_local_string(offset: FixedOffset) -> String {
    Utc::now()
        .with_timezone(&offset)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 同 `now_local_string`，但直接接受 IANA 时区。仅用于希望 DST 自动跟随的
/// 场景；目前未使用，留作未来扩展。
#[allow(dead_code)]
pub fn now_local_string_in_tz(tz: Tz) -> String {
    Utc::now()
        .with_timezone(&tz)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_shape_matches_pc_sqlite() {
        let s = now_local_string(FixedOffset::east_opt(8 * 3600).unwrap());
        // 形如 "2026-05-13 12:34:56"
        assert_eq!(s.len(), 19);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
    }
}

//! 时间格式化与解析工具（跨端契约 K1）。
//!
//! **核心契约**：记录级时间（todo / subtask 的 `createdAt`/`updatedAt`、墓碑
//! `deletedAt` 等）一律是 `YYYY-MM-DD HH:MM:SS`——本地墙钟、无时区后缀，与 PC
//! SQLite `datetime('now','localtime')` 字节级一致。云端没有 OS 级 localtime，
//! 按 `config.timezone`（IANA 时区）换算。
//!
//! **每次取时间都用 `Utc::now().with_timezone(&tz)` 重新换算**，不缓存偏移：
//! 早期版本在启动时算一次 `FixedOffset`，DST 时区切换后会整整差一小时。

use chrono::{
    DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta, TimeZone, Utc,
};
use chrono_tz::Tz;

/// 规范存储格式（K1）。
pub const CANONICAL_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// 返回与 PC SQLite `datetime('now','localtime')` 字符串格式一致的时间戳。
///
/// 示例：`"2026-05-13 12:34:56"`。
pub fn now_local_string(tz: Tz) -> String {
    Utc::now()
        .with_timezone(&tz)
        .format(CANONICAL_FORMAT)
        .to_string()
}

/// sync-data 顶层 `updatedAt` 元信息：ISO 8601 带偏移，例如
/// `2026-05-13T12:34:56+08:00`（与 PC 端 `webdav_last_sync_at` 同格式；旧版 PC
/// 用它做"远端是否较新"的字符串比较，所以格式保持不变）。
pub fn now_iso_string(tz: Tz) -> String {
    Utc::now()
        .with_timezone(&tz)
        .format("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

/// `days` 天前的本地墙钟（规范格式）。墓碑保留期截止线用。
pub fn days_ago_local_string(tz: Tz, days: i64) -> String {
    (Utc::now() - TimeDelta::days(days))
        .with_timezone(&tz)
        .format(CANONICAL_FORMAT)
        .to_string()
}

/// UNIX 秒 → 本地墙钟规范字符串（`/health` 展示 `dirtySince` 等用）。
pub fn epoch_to_local_string(secs: i64, tz: Tz) -> Option<String> {
    DateTime::<Utc>::from_timestamp(secs, 0)
        .map(|d| d.with_timezone(&tz).format(CANONICAL_FORMAT).to_string())
}

/// 把本地墙钟字符串（K1 任意可接受形态）换算成 UTC 时刻。
///
/// DST 回拨造成的重复时刻取较早的一个；落在 DST 跳变空洞里的时刻返回 `None`
/// （云端自己写出的时间不可能落在空洞里，只有时区配置被改过才会发生）。
pub fn local_string_to_utc(raw: &str, tz: Tz) -> Option<DateTime<Utc>> {
    let naive = parse_datetime(raw, tz, NaiveTime::MIN)?;
    tz.from_local_datetime(&naive)
        .earliest()
        .map(|d| d.with_timezone(&Utc))
}

/// K1 宽松解析：返回 `tz` 下的本地墙钟。
///
/// 接受：
/// - `YYYY-MM-DD HH:MM:SS` / `YYYY-MM-DDTHH:MM:SS`
/// - `YYYY-MM-DD HH:MM` / `YYYY-MM-DDTHH:MM`
/// - 秒后可选小数（`.123` / `,123`，截断丢弃）
/// - 可选 `Z` / `±HH:MM` / `±HHMM` / `±HH` 后缀（允许前置一个空格），换算到 `tz` 的墙钟
/// - 仅日期 `YYYY-MM-DD` → 使用 `default_time`
///
/// 其它形态返回 `None`。
pub fn parse_datetime(raw: &str, tz: Tz, default_time: NaiveTime) -> Option<NaiveDateTime> {
    let s = raw.trim();
    if s.len() < 10 || !s.is_char_boundary(10) {
        return None;
    }
    let (date_part, rest) = s.split_at(10);
    if !date_part.bytes().enumerate().all(|(i, b)| {
        if i == 4 || i == 7 {
            b == b'-'
        } else {
            b.is_ascii_digit()
        }
    }) {
        return None;
    }
    let date = NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()?;
    if rest.is_empty() {
        return Some(date.and_time(default_time));
    }

    let rb = rest.as_bytes();
    if !matches!(rb[0], b' ' | b'T' | b't') {
        return None;
    }
    let t = &rest[1..];
    let tb = t.as_bytes();
    if tb.len() < 5 || tb[2] != b':' {
        return None;
    }
    let hour = two_digits(&tb[0..2])?;
    let minute = two_digits(&tb[3..5])?;
    let mut idx = 5;
    let mut second = 0;
    if tb.len() >= idx + 3 && tb[idx] == b':' {
        second = two_digits(&tb[idx + 1..idx + 3])?;
        idx += 3;
        if idx < tb.len() && (tb[idx] == b'.' || tb[idx] == b',') {
            idx += 1;
            let start = idx;
            while idx < tb.len() && tb[idx].is_ascii_digit() {
                idx += 1;
            }
            if idx == start {
                return None;
            }
        }
    }
    let time = NaiveTime::from_hms_opt(hour, minute, second)?;
    let naive = date.and_time(time);

    // idx 之前全是 ASCII，切片落在字符边界上
    let suffix = t[idx..].trim_start();
    if suffix.is_empty() {
        return Some(naive);
    }
    let offset = parse_offset(suffix)?;
    let with_offset = offset.from_local_datetime(&naive).single()?;
    Some(with_offset.with_timezone(&tz).naive_local())
}

/// 把 K1 任意可接受形态规范化为 `YYYY-MM-DD HH:MM:SS`；无法解析返回 `None`。
/// `default_time` 用于仅日期的输入。
pub fn normalize_datetime(raw: &str, tz: Tz, default_time: NaiveTime) -> Option<String> {
    parse_datetime(raw, tz, default_time).map(|d| d.format(CANONICAL_FORMAT).to_string())
}

fn two_digits(b: &[u8]) -> Option<u32> {
    match b {
        [h, l] if h.is_ascii_digit() && l.is_ascii_digit() => {
            Some(u32::from(h - b'0') * 10 + u32::from(l - b'0'))
        }
        _ => None,
    }
}

fn parse_offset(s: &str) -> Option<FixedOffset> {
    if s == "Z" || s == "z" {
        return FixedOffset::east_opt(0);
    }
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &s[1..];
    let rb = rest.as_bytes();
    let (hh, mm) = match rb.len() {
        2 => (two_digits(rb)?, 0),
        4 => (two_digits(&rb[0..2])?, two_digits(&rb[2..4])?),
        5 if rb[2] == b':' => (two_digits(&rb[0..2])?, two_digits(&rb[3..5])?),
        _ => return None,
    };
    if hh > 23 || mm > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (hh as i32 * 3600 + mm as i32 * 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shanghai() -> Tz {
        "Asia/Shanghai".parse().unwrap()
    }

    fn norm(s: &str) -> Option<String> {
        normalize_datetime(s, shanghai(), NaiveTime::MIN)
    }

    #[test]
    fn format_shape_matches_pc_sqlite() {
        let s = now_local_string(shanghai());
        // 形如 "2026-05-13 12:34:56"
        assert_eq!(s.len(), 19);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[7..8], "-");
        assert_eq!(&s[10..11], " ");
        assert_eq!(&s[13..14], ":");
        assert_eq!(&s[16..17], ":");
    }

    #[test]
    fn iso_metadata_has_offset() {
        let s = now_iso_string(shanghai());
        assert!(s.ends_with("+08:00"), "{}", s);
        assert_eq!(&s[10..11], "T");
    }

    #[test]
    fn normalize_accepts_all_k1_shapes() {
        assert_eq!(
            norm("2026-05-13 10:20:30").as_deref(),
            Some("2026-05-13 10:20:30")
        );
        assert_eq!(
            norm("2026-05-13T10:20:30").as_deref(),
            Some("2026-05-13 10:20:30")
        );
        assert_eq!(
            norm("2026-05-13T10:20").as_deref(),
            Some("2026-05-13 10:20:00")
        );
        assert_eq!(
            norm("2026-05-13 10:20").as_deref(),
            Some("2026-05-13 10:20:00")
        );
        assert_eq!(
            norm("2026-05-13 10:20:30.789").as_deref(),
            Some("2026-05-13 10:20:30")
        );
        assert_eq!(
            norm("  2026-05-13 10:20:30  ").as_deref(),
            Some("2026-05-13 10:20:30")
        );
        assert_eq!(norm("2026-05-13").as_deref(), Some("2026-05-13 00:00:00"));
    }

    #[test]
    fn normalize_converts_offsets_to_local_wall_clock() {
        // UTC 02:00 == 上海 10:00
        assert_eq!(
            norm("2026-05-13T02:00:00Z").as_deref(),
            Some("2026-05-13 10:00:00")
        );
        assert_eq!(
            norm("2026-05-13T02:00:00.5z").as_deref(),
            Some("2026-05-13 10:00:00")
        );
        assert_eq!(
            norm("2026-05-13T10:00:00+08:00").as_deref(),
            Some("2026-05-13 10:00:00")
        );
        assert_eq!(
            norm("2026-05-13 10:00:00 +0800").as_deref(),
            Some("2026-05-13 10:00:00")
        );
        assert_eq!(
            norm("2026-05-13T00:00:00-02").as_deref(),
            Some("2026-05-13 10:00:00")
        );
        // 跨日
        assert_eq!(
            norm("2026-05-13T20:00:00Z").as_deref(),
            Some("2026-05-14 04:00:00")
        );
    }

    #[test]
    fn date_only_uses_default_time() {
        let t = NaiveTime::from_hms_opt(23, 59, 0).unwrap();
        assert_eq!(
            normalize_datetime("2026-05-13", shanghai(), t).as_deref(),
            Some("2026-05-13 23:59:00")
        );
    }

    #[test]
    fn normalize_rejects_garbage() {
        for bad in [
            "",
            "yesterday",
            "2026/05/13 10:00:00",
            "2026-13-01 10:00:00",
            "2026-05-13X10:00:00",
            "2026-05-13 25:00:00",
            "2026-05-13 10:00:00abc",
            "2026-05-13 10:00:00.",
            "2026-05-13 10",
            "+2026-05-13",
            "2026-05-13 10:00:00+25:00",
            "２０２６-05-13",
        ] {
            assert_eq!(norm(bad), None, "should reject {:?}", bad);
        }
    }

    #[test]
    fn canonical_strings_compare_lexicographically() {
        // LWW 依赖规范化后的字符串比较；混用 T / 空格时规范化后才可比
        let a = norm("2026-05-13T09:00:00").unwrap();
        let b = norm("2026-05-13 10:00").unwrap();
        assert!(a < b);
    }

    #[test]
    fn local_to_utc_round_trip() {
        let utc = local_string_to_utc("2026-05-13 10:00:00", shanghai()).unwrap();
        assert_eq!(utc.to_rfc3339(), "2026-05-13T02:00:00+00:00");
    }

    #[test]
    fn dst_zone_uses_current_offset_per_call() {
        // America/New_York：夏令时 -04:00、冬令时 -05:00。每次调用按当时的偏移换算。
        let ny: Tz = "America/New_York".parse().unwrap();
        assert_eq!(
            normalize_datetime("2026-07-01T12:00:00Z", ny, NaiveTime::MIN).as_deref(),
            Some("2026-07-01 08:00:00")
        );
        assert_eq!(
            normalize_datetime("2026-12-01T12:00:00Z", ny, NaiveTime::MIN).as_deref(),
            Some("2026-12-01 07:00:00")
        );
        // DST 回拨的重复时刻取较早的那个（EDT）
        let utc = local_string_to_utc("2026-11-01 01:30:00", ny).unwrap();
        assert_eq!(utc.to_rfc3339(), "2026-11-01T05:30:00+00:00");
    }

    #[test]
    fn epoch_formats_in_zone() {
        assert_eq!(
            epoch_to_local_string(0, shanghai()).as_deref(),
            Some("1970-01-01 08:00:00")
        );
    }

    #[test]
    fn days_ago_is_canonical_and_earlier() {
        let tz = shanghai();
        let cutoff = days_ago_local_string(tz, 30);
        let now = now_local_string(tz);
        assert_eq!(cutoff.len(), 19);
        assert!(cutoff < now);
    }
}

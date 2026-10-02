//! K1 时间格式工具。
//!
//! 规范存储格式：`YYYY-MM-DD HH:MM:SS`（本地墙钟、无时区后缀），与 SQLite
//! `datetime('now', 'localtime')` 逐字节一致。todo 的 `notifyAt / startTime / endTime /
//! createdAt / updatedAt`、subtask 的 `createdAt / updatedAt`、墓碑 `deletedAt`、
//! `settingsUpdatedAt` 都用它，LWW 合并直接比较字符串。
//!
//! 读取方兼容：
//! - `YYYY-MM-DD HH:MM:SS` / `YYYY-MM-DDTHH:MM:SS`
//! - `YYYY-MM-DD HH:MM` / `YYYY-MM-DDTHH:MM`
//! - 可选小数秒（`.123`，截断）
//! - 可选 `Z` / `±HH:MM` / `±HHMM` / `±HH` 后缀（换算为本机本地墙钟）
//! - 仅日期 `YYYY-MM-DD`，按字段补默认时刻（见 [`DefaultTime`]）
//!
//! 写入方一律输出规范格式。通知调度、todo 命令（R2）也复用本模块。

use chrono::{Duration, FixedOffset, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};

/// 规范存储格式（chrono 格式串）
pub const CANONICAL_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// 仅日期输入时补的默认时刻（与 PC 编辑器一致）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefaultTime {
    /// 00:00:00：`startTime`，以及 `createdAt` / `updatedAt` / `deletedAt` 等时间戳
    StartOfDay,
    /// 23:59:00：`endTime`（cloud 的 `dueDate` 别名同此）
    EndOfDay,
    /// 09:00:00：`notifyAt`
    Notify,
}

impl DefaultTime {
    fn time(self) -> NaiveTime {
        let (h, m) = match self {
            Self::StartOfDay => (0, 0),
            Self::EndOfDay => (23, 59),
            Self::Notify => (9, 0),
        };
        NaiveTime::from_hms_opt(h, m, 0).expect("合法的固定时刻")
    }
}

/// 当前本地时间，规范格式
pub fn now_local() -> String {
    format_canonical(&Local::now().naive_local())
}

/// `days` 天前的本地时间，规范格式（墓碑保留期截止点等）
pub fn days_ago_local(days: i64) -> String {
    format_canonical(&(Local::now().naive_local() - Duration::days(days)))
}

/// 把 `NaiveDateTime` 格式化为规范格式（小数秒截断）
pub fn format_canonical(dt: &NaiveDateTime) -> String {
    dt.format(CANONICAL_FORMAT).to_string()
}

/// 解析任意 K1 形态为本地墙钟时间；仅日期按 00:00:00 处理。
pub fn parse_local_datetime(s: &str) -> Option<NaiveDateTime> {
    parse_local_datetime_with(s, DefaultTime::StartOfDay)
}

/// 解析任意 K1 形态为本地墙钟时间；仅日期时补 `default` 指定的时刻。
pub fn parse_local_datetime_with(s: &str, default: DefaultTime) -> Option<NaiveDateTime> {
    parse_in_tz(s, default, &Local)
}

/// 规范化为 `YYYY-MM-DD HH:MM:SS`；无法识别返回 `None`。
pub fn normalize_datetime(s: &str, default: DefaultTime) -> Option<String> {
    parse_local_datetime_with(s, default).map(|dt| format_canonical(&dt))
}

/// 规范时间加一秒（用于"比某个时间戳严格更新"的场景）；无法解析时原样返回。
pub fn plus_one_second(s: &str) -> String {
    match parse_local_datetime(s) {
        Some(dt) => format_canonical(&(dt + Duration::seconds(1))),
        None => s.to_string(),
    }
}

/// 解析核心：带时区后缀的输入换算到 `tz` 的墙钟。抽出 `tz` 参数是为了让单测不依赖机器时区。
fn parse_in_tz<Tz: TimeZone>(s: &str, default: DefaultTime, tz: &Tz) -> Option<NaiveDateTime> {
    let s = s.trim();
    let date_part = s.get(..10)?;
    let date = NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()?;
    let rest = s.get(10..)?;
    if rest.is_empty() {
        return Some(date.and_time(default.time()));
    }

    let mut chars = rest.chars();
    let sep = chars.next()?;
    if !matches!(sep, ' ' | 'T' | 't') {
        return None;
    }
    let rest = chars.as_str();

    let (time_part, offset_secs) = split_offset(rest)?;
    let time = parse_time(time_part)?;
    let naive = date.and_time(time);

    match offset_secs {
        None => Some(naive),
        Some(secs) => {
            let offset = FixedOffset::east_opt(secs)?;
            let instant = offset.from_local_datetime(&naive).single()?;
            Some(instant.with_timezone(tz).naive_local())
        }
    }
}

/// 拆出时间部分与可选的时区偏移（秒）。
fn split_offset(rest: &str) -> Option<(&str, Option<i32>)> {
    if let Some(stripped) = rest.strip_suffix(['Z', 'z']) {
        return Some((stripped, Some(0)));
    }
    // 时间部分本身只含数字、':'、'.'，出现 '+' / '-' 一定是偏移
    match rest.rfind(['+', '-']) {
        Some(pos) => {
            let offset = parse_offset(&rest[pos..])?;
            Some((&rest[..pos], Some(offset)))
        }
        None => Some((rest, None)),
    }
}

/// `+08:00` / `+0800` / `+08` / `-05:30` → 秒
fn parse_offset(s: &str) -> Option<i32> {
    let sign = match s.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let body = &s[1..];
    let (h, m) = match body.len() {
        2 => (body, "00"),
        4 => (&body[..2], &body[2..]),
        5 if body.as_bytes()[2] == b':' => (&body[..2], &body[3..]),
        _ => return None,
    };
    if !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let h: i32 = h.parse().ok()?;
    let m: i32 = m.parse().ok()?;
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60))
}

/// `HH:MM:SS`、`HH:MM`，可带小数秒（截断）。
fn parse_time(s: &str) -> Option<NaiveTime> {
    let main = match s.split_once('.') {
        Some((main, frac)) => {
            if frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            main
        }
        None => s,
    };
    let parts: Vec<&str> = main.split(':').collect();
    let nums: Option<Vec<u32>> = parts
        .iter()
        .map(|p| {
            if p.is_empty() || p.len() > 2 || !p.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                p.parse().ok()
            }
        })
        .collect();
    match nums?.as_slice() {
        [h, m] if !s.contains('.') => NaiveTime::from_hms_opt(*h, *m, 0),
        [h, m, sec] => NaiveTime::from_hms_opt(*h, *m, *sec),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cst() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    /// 在 UTC+8 下解析并规范化，结果与机器时区无关
    fn norm8(s: &str, default: DefaultTime) -> Option<String> {
        parse_in_tz(s, default, &cst()).map(|dt| format_canonical(&dt))
    }

    #[test]
    fn canonical_input_is_unchanged() {
        assert_eq!(
            norm8("2026-05-01 09:30:15", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:15")
        );
    }

    #[test]
    fn t_separator_and_missing_seconds_are_accepted() {
        assert_eq!(
            norm8("2026-05-01T09:30:15", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:15")
        );
        assert_eq!(
            norm8("2026-05-01T09:30", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
        assert_eq!(
            norm8("2026-05-01 09:30", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
        assert_eq!(
            norm8("2026-05-01t09:30", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
    }

    #[test]
    fn fractional_seconds_are_truncated() {
        assert_eq!(
            norm8("2026-05-01T09:30:15.987", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:15")
        );
        assert_eq!(
            norm8("2026-05-01 09:30:15.123456", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:15")
        );
    }

    #[test]
    fn utc_and_offset_suffixes_convert_to_local_wall_clock() {
        // UTC 01:30 = UTC+8 09:30
        assert_eq!(
            norm8("2026-05-01T01:30:00Z", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
        assert_eq!(
            norm8("2026-05-01T01:30:00.000Z", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
        // 同一时区：不变
        assert_eq!(
            norm8("2026-05-01T09:30:00+08:00", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
        assert_eq!(
            norm8("2026-05-01T09:30:00+0800", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
        assert_eq!(
            norm8("2026-05-01T09:30+08", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:30:00")
        );
        // 跨日：UTC-05:00 的 20:00 = UTC+8 次日 09:00
        assert_eq!(
            norm8("2026-04-30T20:00:00-05:00", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 09:00:00")
        );
    }

    #[test]
    fn date_only_uses_field_default_time() {
        assert_eq!(
            norm8("2026-05-01", DefaultTime::StartOfDay).as_deref(),
            Some("2026-05-01 00:00:00")
        );
        assert_eq!(
            norm8("2026-05-01", DefaultTime::EndOfDay).as_deref(),
            Some("2026-05-01 23:59:00")
        );
        assert_eq!(
            norm8("2026-05-01", DefaultTime::Notify).as_deref(),
            Some("2026-05-01 09:00:00")
        );
        assert_eq!(
            norm8("  2026-05-01  ", DefaultTime::Notify).as_deref(),
            Some("2026-05-01 09:00:00")
        );
    }

    #[test]
    fn garbage_is_rejected() {
        for bad in [
            "",
            "tomorrow",
            "2026-13-01",
            "2026-02-30 10:00:00",
            "2026-05-01X09:00",
            "2026-05-01 25:00",
            "2026-05-01 09:00:00+25:00",
            "2026-05-01 09",
            "2026-05-01T",
            "2026-05-01 09:00:00.",
            "2026/05/01 09:00",
        ] {
            assert_eq!(norm8(bad, DefaultTime::StartOfDay), None, "应拒绝 {bad:?}");
        }
    }

    #[test]
    fn local_wrappers_produce_canonical_shape() {
        let now = now_local();
        assert_eq!(now.len(), 19);
        assert_eq!(&now[10..11], " ");
        assert!(!now.contains('T'));
        assert_eq!(
            normalize_datetime(&now, DefaultTime::StartOfDay),
            Some(now.clone())
        );
        assert!(days_ago_local(30) < now);
        assert_eq!(
            parse_local_datetime("2026-05-01")
                .map(|d| format_canonical(&d))
                .as_deref(),
            Some("2026-05-01 00:00:00")
        );
    }

    #[test]
    fn plus_one_second_rolls_over() {
        assert_eq!(
            plus_one_second("2026-12-31 23:59:59"),
            "2027-01-01 00:00:00"
        );
        assert_eq!(plus_one_second("garbage"), "garbage");
    }
}

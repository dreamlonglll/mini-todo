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

/// 本机写入（编辑 / 排序 / 提醒状态 / 导入 / 强制推送）的新 `updated_at`：取 `now` 与
/// `previous + 1 秒` 中较晚者，保证新版本严格晚于它所基于、要取代的那个版本；`previous`
/// 无法识别时取 `now`（乱码时间绝不会被抄进新版本）。
///
/// 记录级 LWW 下这是必须的：同步来的版本可能带着比本机时钟还新的时间戳（另一台设备时钟偏快、
/// cloud 的 `timezone` 配错），直接写"现在"的话，这次写入在下次同步时会输给它要取代的旧版本，
/// 被静默覆盖。删除墓碑同理（见 `sync_store::deletion_time`）。
pub fn superseding(now: &str, previous: &str) -> String {
    match parse_local_datetime(previous) {
        Some(dt) => {
            let after = format_canonical(&(dt + Duration::seconds(1)));
            if after.as_str() > now {
                after
            } else {
                now.to_string()
            }
        }
        None => now.to_string(),
    }
}

/// UPDATE 语句里刷新 `updated_at` 的 SET 片段，语义同 [`superseding`]（"现在"取 SQLite 的
/// `datetime('now', 'localtime')`，与建表默认值一致）。原值无法解析时 `datetime()` 为 NULL——
/// SQLite 的多参数 `MAX` 遇 NULL 结果就是 NULL，会撞 `NOT NULL` 约束——所以先 COALESCE 成空串。
pub const SQL_SET_UPDATED_AT: &str = "updated_at = MAX(datetime('now', 'localtime'), \
     COALESCE(datetime(updated_at, '+1 second'), ''))";

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
    // 下面按字节下标切分：含多字节字符的输入（如 "+1é1"，4 字节）会切在字符中间直接 panic。
    // 时间来自远端 JSON / 旧数据，必须先拒绝非 ASCII
    if !body.is_ascii() {
        return None;
    }
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

    /// 回归：时区偏移里混进多字节字符时，按字节切分会切在字符中间 panic。
    /// 这些串可以来自远端 JSON（合并时规范化）或旧库（v28 迁移时规范化，panic 即启动失败）。
    #[test]
    fn non_ascii_offsets_are_rejected_without_panicking() {
        for bad in [
            "2026-05-01 10:00+1é1",
            "2026-05-01T10:00:00-é",
            "2026-05-01T10:00:00+0é",
            "2026-05-01T10:00:00+08:é",
            "2026-05-01T10:00:00+中文",
            "2026-05-01T10:00:00+０８:００",
            "2026-05-01 1é:00",
            "2026-05-01 10:00:0é",
        ] {
            assert_eq!(norm8(bad, DefaultTime::StartOfDay), None, "应拒绝 {bad:?}");
            assert_eq!(normalize_datetime(bad, DefaultTime::Notify), None);
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
    fn superseding_is_strictly_newer_than_the_previous_version() {
        let now = "2026-06-01 12:00:00";
        // 旧版本在过去：就是现在
        assert_eq!(superseding(now, "2026-05-01 10:00:00"), now);
        // 旧版本在"未来"（对端时钟偏快）：比它晚一秒，含跨年进位
        assert_eq!(
            superseding(now, "2026-12-31 23:59:59"),
            "2027-01-01 00:00:00"
        );
        // 恰好等于现在：仍要严格更晚
        assert_eq!(superseding(now, now), "2026-06-01 12:00:01");
        // 非规范但可识别的旧值先规范化
        assert_eq!(superseding(now, "2026-07-01T08:00"), "2026-07-01 08:00:01");
        // 乱码 / 缺失：绝不抄进新版本
        for bad in ["zzz", "", "9999-99-99"] {
            assert_eq!(superseding(now, bad), now, "{bad:?}");
        }
    }

    /// SQL 片段与 [`superseding`] 语义一致，且原值是乱码时不会得到 NULL
    #[test]
    fn sql_set_updated_at_matches_superseding() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, updated_at TEXT NOT NULL);
             INSERT INTO t VALUES (1, '2099-01-01 00:00:00'), (2, '2000-01-01 00:00:00'),
                                  (3, 'not a time'), (4, '2099-01-01T08:30');",
        )
        .unwrap();
        let before = now_local();
        conn.execute(&format!("UPDATE t SET {SQL_SET_UPDATED_AT}"), [])
            .unwrap();
        let at = |id: i64| -> String {
            conn.query_row("SELECT updated_at FROM t WHERE id = ?1", [id], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(at(1), "2099-01-01 00:00:01");
        assert!(at(2) >= before && at(2).len() == 19, "{}", at(2));
        assert!(at(3) >= before && at(3).len() == 19, "{}", at(3));
        assert_eq!(at(4), "2099-01-01 08:30:01");
    }
}

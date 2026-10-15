//! 日期时间工具（旧 `utils/datetime.ts` 的 1:1 移植，跨端契约 K1）。
//!
//! 规范存储格式：`YYYY-MM-DD HH:MM:SS`（本地墙钟、无时区后缀），与 core 的 `db::time`、cloud 一致。
//! 界面写入一律用它；界面解析 / 格式化也只用本模块（禁止 `split('T')` 之类的手写解析）。
//!
//! 读取兼容：
//! - `YYYY-MM-DD HH:MM:SS`、`YYYY-MM-DDTHH:MM:SS`（`T` / `t` / 空格）
//! - `YYYY-MM-DD HH:MM`、可选小数秒（`.123`）
//! - 可选时区后缀 `Z` / `±HH:MM` / `±HHMM` / `±HH`（换算为本机墙钟；时间与时区之间允许空白）
//! - 仅日期 `YYYY-MM-DD`：按字段类型补默认时刻（开始 00:00、截止 23:59、提醒 09:00）
//!
//! 与后端 `db::time` 的边界差异照旧（见 `ui-editor-windows.md` §9.5）：这里要求两位数的时 / 分 / 秒，
//! 接受时间与时区之间的空白。

use std::sync::LazyLock;

use chrono::{Duration, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone};
use regex::Regex;

/// 时间字段类型，决定仅日期输入时补的默认时刻
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DateTimeKind {
    Start,
    End,
    Notify,
    #[default]
    Generic,
}

impl DateTimeKind {
    /// 仅日期时的默认时刻（时, 分, 秒）
    fn default_time(self) -> (u32, u32, u32) {
        match self {
            DateTimeKind::Start | DateTimeKind::Generic => (0, 0, 0),
            DateTimeKind::End => (23, 59, 0),
            DateTimeKind::Notify => (9, 0, 0),
        }
    }
}

/// dayjs 记法的规范存储格式
pub const STORAGE_FORMAT: &str = "YYYY-MM-DD HH:mm:ss";

// 日期 + 可选时间（T / t / 空格分隔，秒与小数秒可选）+ 可选时区；时区只允许跟在时间后面。
// JS 的 `\d` 只匹配 ASCII 数字，这里写成 [0-9]
static DATE_TIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^([0-9]{4})-([0-9]{2})-([0-9]{2})(?:[Tt ]([0-9]{2}):([0-9]{2})(?::([0-9]{2})(?:\.[0-9]+)?)?\s*([Zz]|[+-][0-9]{2}(?::?[0-9]{2})?)?)?$",
    )
    .expect("DATE_TIME_RE")
});

/// 时区后缀 → 相对 UTC 的分钟数；非法时返回 `None`
fn parse_offset_minutes(offset: &str) -> Option<i64> {
    if offset == "Z" || offset == "z" {
        return Some(0);
    }
    let sign = if offset.starts_with('-') { -1 } else { 1 };
    let digits: String = offset[1..].chars().filter(|c| *c != ':').collect();
    let hours: i64 = digits.get(0..2)?.parse().ok()?;
    let minutes: i64 = match digits.get(2..4) {
        Some(m) => m.parse().ok()?,
        None => 0,
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 60 + minutes))
}

/// 解析任意 K1 形态为 `tz` 的本地墙钟时间；无法识别时返回 `None`。
/// 不带时区的输入按墙钟直接解释；带时区的先换算成绝对时间再落到 `tz` 的墙钟。
fn parse_in<Tz: TimeZone>(value: &str, kind: DateTimeKind, tz: &Tz) -> Option<NaiveDateTime> {
    let caps = DATE_TIME_RE.captures(value.trim())?;
    let num = |i: usize| -> Option<u32> { caps.get(i).and_then(|m| m.as_str().parse().ok()) };
    let year: i32 = caps.get(1)?.as_str().parse().ok()?;
    let month = num(2)?;
    let day = num(3)?;
    // 不滚动：2026-02-29、2026-04-31 都是非法日期
    let date = NaiveDate::from_ymd_opt(year, month, day)?;

    let (hour, minute, second) = match caps.get(4) {
        None => kind.default_time(),
        Some(_) => {
            let hour = num(4)?;
            let minute = num(5)?;
            let second = num(6).unwrap_or(0);
            if hour > 23 || minute > 59 || second > 59 {
                return None;
            }
            (hour, minute, second)
        }
    };
    let wall = date.and_time(NaiveTime::from_hms_opt(hour, minute, second)?);

    match caps.get(7) {
        Some(offset) => {
            let offset_minutes = parse_offset_minutes(offset.as_str())?;
            let utc = wall - Duration::minutes(offset_minutes);
            Some(tz.from_utc_datetime(&utc).naive_local())
        }
        None => Some(wall),
    }
}

/// 解析任意 K1 形态为本机墙钟时间；无法识别（含空值）时返回 `None`
pub fn parse_date_time(value: Option<&str>, kind: DateTimeKind) -> Option<NaiveDateTime> {
    parse_in(value.filter(|v| !v.is_empty())?, kind, &Local)
}

/// 墙钟时间 → 规范存储格式 `YYYY-MM-DD HH:MM:SS`
pub fn format_storage_date_time(date: &NaiveDateTime) -> String {
    date.format("%Y-%m-%d %H:%M:%S").to_string()
}

/// 任意 K1 形态 → 规范存储格式；无法识别时返回 `None`（幂等）
pub fn normalize_date_time(value: Option<&str>, kind: DateTimeKind) -> Option<String> {
    parse_date_time(value, kind).map(|d| format_storage_date_time(&d))
}

/// 把日期 / 时间选择器的值合成为规范存储格式：
/// `date` 为空 → `None`；`time` 为空（`None` / `""`）→ 按 `kind` 补默认时刻
pub fn compose_date_time(
    date: Option<&str>,
    time: Option<&str>,
    kind: DateTimeKind,
) -> Option<String> {
    let date = date.filter(|d| !d.is_empty())?;
    match time.filter(|t| !t.is_empty()) {
        Some(time) => normalize_date_time(Some(&format!("{date} {time}")), kind),
        None => normalize_date_time(Some(date), kind),
    }
}

/// 拆成日期 / 时间选择器的值
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DateTimeParts {
    /// `YYYY-MM-DD`
    pub date: String,
    /// `HH:mm`
    pub time: String,
}

/// 拆成日期 / 时间选择器的值（秒丢弃）；无法识别时返回 `None`
pub fn split_date_time(value: Option<&str>, kind: DateTimeKind) -> Option<DateTimeParts> {
    let date = parse_date_time(value, kind)?;
    Some(DateTimeParts {
        date: date.format("%Y-%m-%d").to_string(),
        time: date.format("%H:%M").to_string(),
    })
}

fn is_blank(value: Option<&str>) -> bool {
    value.is_none_or(str::is_empty)
}

/// 两个时间是否落在同一分钟（任意 K1 形态）。都为空视为相同；只有一边为空或无法识别视为不同。
///
/// 编辑器的时间选择器只精确到分钟，用它判断"用户有没有改过这个时间"
pub fn is_same_minute(a: Option<&str>, b: Option<&str>, kind: DateTimeKind) -> bool {
    is_same_minute_in(a, b, kind, &Local)
}

fn is_same_minute_in<Tz: TimeZone>(
    a: Option<&str>,
    b: Option<&str>,
    kind: DateTimeKind,
    tz: &Tz,
) -> bool {
    if is_blank(a) && is_blank(b) {
        return true;
    }
    let parse = |v: Option<&str>| {
        v.filter(|v| !v.is_empty())
            .and_then(|v| parse_in(v, kind, tz))
    };
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => {
            a.and_utc().timestamp().div_euclid(60) == b.and_utc().timestamp().div_euclid(60)
        }
        _ => false,
    }
}

/// 墙钟时间 → 本地日期键 `YYYY-MM-DD`
pub fn format_date_key(date: &NaiveDateTime) -> String {
    date.format("%Y-%m-%d").to_string()
}

/// 任意 K1 形态 → 本地日期键 `YYYY-MM-DD`；无法识别时返回 `None`
pub fn to_date_key(value: Option<&str>, kind: DateTimeKind) -> Option<String> {
    parse_date_time(value, kind).map(|d| format_date_key(&d))
}

/// 按 dayjs 记法格式化（支持 `YYYY` `MM` `DD` `HH` `mm` `ss`，其它字符原样输出）
pub fn format_pattern(date: &NaiveDateTime, pattern: &str) -> String {
    const TOKENS: [(&str, &str); 6] = [
        ("YYYY", "%Y"),
        ("MM", "%m"),
        ("DD", "%d"),
        ("HH", "%H"),
        ("mm", "%M"),
        ("ss", "%S"),
    ];
    let mut out = String::new();
    let mut rest = pattern;
    'outer: while !rest.is_empty() {
        for (token, spec) in TOKENS {
            if let Some(after) = rest.strip_prefix(token) {
                out.push_str(&date.format(spec).to_string());
                rest = after;
                continue 'outer;
            }
        }
        let mut chars = rest.chars();
        if let Some(c) = chars.next() {
            out.push(c);
        }
        rest = chars.as_str();
    }
    out
}

/// 展示用格式化（dayjs 格式串，旧默认 `YYYY-MM-DD HH:mm`）；无法识别时返回 `None`，
/// 由调用方决定是隐藏还是回落到原文
pub fn format_date_time(value: Option<&str>, pattern: &str, kind: DateTimeKind) -> Option<String> {
    parse_date_time(value, kind).map(|d| format_pattern(&d, pattern))
}

#[cfg(test)]
mod tests {
    //! 原 `datetime.test.ts`（vitest 固定 TZ=Asia/Shanghai）：带时区换算的用例用固定 +08:00 时区，
    //! 其余用例与本机时区无关。
    use chrono::{Datelike, FixedOffset, Timelike};

    use super::*;

    fn shanghai() -> FixedOffset {
        FixedOffset::east_opt(8 * 3600).unwrap()
    }

    fn parse(value: &str, kind: DateTimeKind) -> Option<NaiveDateTime> {
        parse_in(value, kind, &shanghai())
    }

    fn wall(value: &str) -> Option<[u32; 6]> {
        wall_kind(value, DateTimeKind::Generic)
    }

    fn wall_kind(value: &str, kind: DateTimeKind) -> Option<[u32; 6]> {
        parse(value, kind).map(|d| {
            [
                d.year() as u32,
                d.month(),
                d.day(),
                d.hour(),
                d.minute(),
                d.second(),
            ]
        })
    }

    fn normalize(value: &str, kind: DateTimeKind) -> Option<String> {
        parse(value, kind).map(|d| format_storage_date_time(&d))
    }

    #[test]
    fn parses_the_canonical_storage_format_as_local_wall_clock_time() {
        assert_eq!(wall("2026-10-02 09:30:15"), Some([2026, 10, 2, 9, 30, 15]));
    }

    #[test]
    fn accepts_t_separators_missing_seconds_and_fractional_seconds() {
        assert_eq!(wall("2026-10-02T09:30:15"), Some([2026, 10, 2, 9, 30, 15]));
        assert_eq!(wall("2026-10-02t09:30"), Some([2026, 10, 2, 9, 30, 0]));
        assert_eq!(wall("2026-10-02 09:30"), Some([2026, 10, 2, 9, 30, 0]));
        assert_eq!(
            wall("2026-10-02T09:30:15.123"),
            Some([2026, 10, 2, 9, 30, 15])
        );
    }

    #[test]
    fn trims_surrounding_whitespace() {
        assert_eq!(
            wall("  2026-10-02 09:30:00\n"),
            Some([2026, 10, 2, 9, 30, 0])
        );
    }

    #[test]
    fn converts_timezone_suffixes_to_local_wall_clock_time() {
        for value in [
            "2026-10-02T01:00:00Z",
            "2026-10-02T01:00:00.500z",
            "2026-10-02T09:00:00+08:00",
            "2026-10-02T09:00:00+0800",
            "2026-10-02T09:00:00+08",
        ] {
            assert_eq!(wall(value), Some([2026, 10, 2, 9, 0, 0]), "{value}");
        }
        // 跨日：纽约晚上 = 北京次日上午
        assert_eq!(
            wall("2026-10-01T21:00:00-05:00"),
            Some([2026, 10, 2, 10, 0, 0])
        );
        assert_eq!(
            wall("2026-10-02 09:00 +05:30"),
            Some([2026, 10, 2, 11, 30, 0])
        );
    }

    #[test]
    fn fills_the_default_time_of_day_for_date_only_input_by_field_kind() {
        use DateTimeKind::*;
        assert_eq!(wall_kind("2026-10-02", Start), Some([2026, 10, 2, 0, 0, 0]));
        assert_eq!(wall_kind("2026-10-02", End), Some([2026, 10, 2, 23, 59, 0]));
        assert_eq!(
            wall_kind("2026-10-02", Notify),
            Some([2026, 10, 2, 9, 0, 0])
        );
        assert_eq!(wall("2026-10-02"), Some([2026, 10, 2, 0, 0, 0]));
    }

    #[test]
    fn keeps_an_explicit_time_even_when_a_kind_is_given() {
        assert_eq!(
            wall_kind("2026-10-02 18:45:00", DateTimeKind::Notify),
            Some([2026, 10, 2, 18, 45, 0])
        );
    }

    #[test]
    fn validates_calendar_dates_without_rollover() {
        assert_eq!(wall("2026-02-29"), None);
        assert_eq!(wall("2028-02-29"), Some([2028, 2, 29, 0, 0, 0]));
        assert_eq!(wall("2026-04-31 10:00:00"), None);
        assert_eq!(wall("2026-13-01"), None);
        assert_eq!(wall("2026-00-10"), None);
        assert_eq!(wall("2026-10-00"), None);
    }

    #[test]
    fn rejects_out_of_range_or_malformed_times_and_offsets() {
        for value in [
            "2026-10-02 24:00:00",
            "2026-10-02 23:60",
            "2026-10-02 23:59:60",
            "2026-10-02 9:00",
            "2026-10-02T09:00:00+24:00",
            "2026-10-02T09:00:00+08:60",
            // 时区只能跟在时间后面
            "2026-10-02Z",
        ] {
            assert_eq!(wall(value), None, "{value}");
        }
    }

    #[test]
    fn returns_none_for_empty_or_unrecognised_values() {
        assert_eq!(parse_date_time(None, DateTimeKind::Generic), None);
        assert_eq!(parse_date_time(Some(""), DateTimeKind::Generic), None);
        for value in [
            "tomorrow",
            "2026/10/02 09:00",
            "20261002",
            "2026-10-02 09:00:00T09:00:00",
            // 非 ASCII 数字
            "２０２６-10-02",
        ] {
            assert_eq!(wall(value), None, "{value}");
        }
    }

    #[test]
    fn normalize_outputs_storage_format_byte_for_byte() {
        let out = normalize("2026-10-02T09:05", DateTimeKind::Generic).unwrap();
        assert_eq!(out, "2026-10-02 09:05:00");
        let re = Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2} [0-9]{2}:[0-9]{2}:[0-9]{2}$").unwrap();
        assert!(re.is_match(&out));
    }

    #[test]
    fn normalizes_every_accepted_input_shape() {
        use DateTimeKind::*;
        assert_eq!(
            normalize("2026-10-02 09:05:07", Generic).as_deref(),
            Some("2026-10-02 09:05:07")
        );
        assert_eq!(
            normalize("2026-10-02T09:05:07.999", Generic).as_deref(),
            Some("2026-10-02 09:05:07")
        );
        assert_eq!(
            normalize("2026-10-02T01:05:07Z", Generic).as_deref(),
            Some("2026-10-02 09:05:07")
        );
        assert_eq!(
            normalize("2026-10-02", End).as_deref(),
            Some("2026-10-02 23:59:00")
        );
        assert_eq!(
            normalize("2026-10-02", Notify).as_deref(),
            Some("2026-10-02 09:00:00")
        );
    }

    #[test]
    fn normalize_is_idempotent() {
        let once = normalize("2026-10-01T22:30:00-02:00", DateTimeKind::Generic).unwrap();
        assert_eq!(once, "2026-10-02 08:30:00");
        assert_eq!(
            normalize(&once, DateTimeKind::Generic).as_deref(),
            Some(once.as_str())
        );
        // 本机时区版本对规范格式同样幂等
        assert_eq!(
            normalize_date_time(Some(&once), DateTimeKind::Generic).as_deref(),
            Some(once.as_str())
        );
    }

    #[test]
    fn normalize_returns_none_for_invalid_input() {
        assert_eq!(
            normalize_date_time(Some("not a date"), DateTimeKind::Generic),
            None
        );
        assert_eq!(normalize_date_time(None, DateTimeKind::Generic), None);
    }

    #[test]
    fn compose_combines_picker_values_into_the_storage_format() {
        use DateTimeKind::*;
        assert_eq!(
            compose_date_time(Some("2026-10-02"), Some("14:30"), Notify).as_deref(),
            Some("2026-10-02 14:30:00")
        );
        assert_eq!(
            compose_date_time(Some("2026-10-02"), Some("14:30:15"), Start).as_deref(),
            Some("2026-10-02 14:30:15")
        );
    }

    #[test]
    fn compose_uses_the_default_time_when_only_a_date_is_picked() {
        use DateTimeKind::*;
        assert_eq!(
            compose_date_time(Some("2026-10-02"), None, Start).as_deref(),
            Some("2026-10-02 00:00:00")
        );
        assert_eq!(
            compose_date_time(Some("2026-10-02"), Some(""), End).as_deref(),
            Some("2026-10-02 23:59:00")
        );
        assert_eq!(
            compose_date_time(Some("2026-10-02"), None, Notify).as_deref(),
            Some("2026-10-02 09:00:00")
        );
    }

    #[test]
    fn compose_returns_none_without_a_date() {
        assert_eq!(
            compose_date_time(None, Some("14:30"), DateTimeKind::Notify),
            None
        );
        assert_eq!(compose_date_time(Some(""), None, DateTimeKind::Start), None);
    }

    fn parts(date: &str, time: &str) -> Option<DateTimeParts> {
        Some(DateTimeParts {
            date: date.into(),
            time: time.into(),
        })
    }

    #[test]
    fn split_splits_stored_values_into_picker_values() {
        use DateTimeKind::*;
        assert_eq!(
            split_date_time(Some("2026-10-02 14:30:59"), Generic),
            parts("2026-10-02", "14:30")
        );
        assert_eq!(
            split_date_time(Some("2026-10-02T14:30"), Generic),
            parts("2026-10-02", "14:30")
        );
        // 不会产生 "…T09:00:00" 的重复形态
        let p = split_date_time(Some("2026-10-02 09:00:00"), Notify).unwrap();
        assert_eq!(Some(p.clone()), parts("2026-10-02", "09:00"));
        assert_eq!(
            compose_date_time(Some(&p.date), Some(&p.time), Notify).as_deref(),
            Some("2026-10-02 09:00:00")
        );
        // 仅日期按字段默认时刻
        assert_eq!(
            split_date_time(Some("2026-10-02"), End),
            parts("2026-10-02", "23:59")
        );
        assert_eq!(split_date_time(Some("garbage"), Generic), None);
        assert_eq!(split_date_time(None, Generic), None);
    }

    #[test]
    fn split_converts_offsets_to_local_time_before_splitting() {
        let d = parse("2026-10-01T20:00:00Z", DateTimeKind::Generic).unwrap();
        assert_eq!(format_pattern(&d, "YYYY-MM-DD HH:mm"), "2026-10-02 04:00");
    }

    #[test]
    fn same_minute_ignores_seconds_and_spelling() {
        use DateTimeKind::*;
        let tz = shanghai();
        let same = |a: Option<&str>, b: Option<&str>, k| is_same_minute_in(a, b, k, &tz);
        assert!(same(
            Some("2026-10-02 09:00:30"),
            Some("2026-10-02 09:00:00"),
            Generic
        ));
        assert!(same(
            Some("2026-10-02T09:00"),
            Some("2026-10-02 09:00:59"),
            Generic
        ));
        assert!(same(
            Some("2026-10-02T01:00:00Z"),
            Some("2026-10-02 09:00:00"),
            Generic
        ));
        // 仅日期按字段默认时刻
        assert!(same(
            Some("2026-10-02"),
            Some("2026-10-02 09:00:00"),
            Notify
        ));
        assert!(!same(
            Some("2026-10-02"),
            Some("2026-10-02 09:00:00"),
            Start
        ));
        // 分钟 / 日期 / 有无的变化
        assert!(!same(
            Some("2026-10-02 09:01:00"),
            Some("2026-10-02 09:00:00"),
            Generic
        ));
        assert!(!same(
            Some("2026-10-03 09:00:00"),
            Some("2026-10-02 09:00:00"),
            Generic
        ));
        assert!(!same(Some("2026-10-02 09:00:00"), None, Generic));
        assert!(!same(None, Some("2026-10-02 09:00:00"), Generic));
        // 两边都空相同，无法识别不同
        assert!(same(None, None, Generic));
        assert!(same(Some(""), None, Generic));
        assert!(!same(Some("garbage"), Some("garbage"), Generic));
        // 本机时区版本的基本行为
        assert!(is_same_minute(None, Some(""), Generic));
        assert!(is_same_minute(
            Some("2026-10-02 09:00:30"),
            Some("2026-10-02 09:00:00"),
            Generic
        ));
    }

    #[test]
    fn date_keys_and_display_formatting() {
        use DateTimeKind::*;
        let d = NaiveDate::from_ymd_opt(2026, 1, 5)
            .unwrap()
            .and_hms_opt(23, 59, 0)
            .unwrap();
        assert_eq!(format_date_key(&d), "2026-01-05");
        assert_eq!(
            to_date_key(Some("2026-10-02 23:59:59"), Generic).as_deref(),
            Some("2026-10-02")
        );
        let utc = parse("2026-10-01T16:30:00Z", Generic).unwrap();
        assert_eq!(format_date_key(&utc), "2026-10-02");
        assert_eq!(
            to_date_key(Some("2026-10-02"), End).as_deref(),
            Some("2026-10-02")
        );
        assert_eq!(to_date_key(Some("nope"), Generic), None);

        assert_eq!(
            format_date_time(Some("2026-10-02 09:05:00"), "YYYY-MM-DD HH:mm", Generic).as_deref(),
            Some("2026-10-02 09:05")
        );
        assert_eq!(
            format_date_time(Some("2026-10-02T09:05:00"), "MM-DD HH:mm", Generic).as_deref(),
            Some("10-02 09:05")
        );
        assert_eq!(
            format_date_time(Some("2026-10-02"), "MM-DD HH:mm", Notify).as_deref(),
            Some("10-02 09:00")
        );
        assert_eq!(
            format_date_time(Some("invalid"), "YYYY-MM-DD HH:mm", Generic),
            None
        );
        assert_eq!(format_date_time(None, "YYYY-MM-DD HH:mm", Generic), None);
    }

    #[test]
    fn formats_a_date_in_the_storage_format() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 2)
            .unwrap()
            .and_hms_opt(7, 8, 9)
            .unwrap();
        assert_eq!(format_storage_date_time(&d), "2026-10-02 07:08:09");
        assert_eq!(format_pattern(&d, STORAGE_FORMAT), "2026-10-02 07:08:09");
    }
}

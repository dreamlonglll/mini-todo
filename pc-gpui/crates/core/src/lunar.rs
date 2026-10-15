//! 农历、节气、传统节日（日历视图单元格下方的小字）。
//!
//! 与旧版前端 `pc/src/utils/lunar.ts`（lunar-javascript）逐字段等价：底层换算（公历 → 农历年月日、
//! 闰月、年干支、节气日）用同一作者 6tail 的 Rust 库 `tyme4rs`，名称与格式按 lunar-javascript 输出：
//!
//! | 字段 | lunar-javascript | 示例 |
//! |---|---|---|
//! | `day_text` | `getDayInChinese()` | 初一、二十、廿一、三十 |
//! | `month_text` | `getMonthInChinese() + '月'` | 正月、闰六月、冬月、腊月 |
//! | `year_text` | `getYearInGanZhi() + '年'`（以正月初一换年） | 甲辰年 |
//! | `solar_term` | `getJieQi()`（当天是节气交节日才有） | 立春 |
//! | `festival` | `getFestivals()[0]`（闰月不算；腊月最后一天为除夕） | 春节、龙头节、除夕 |
//! | `is_first_day` | `getDay() === 1` | |
//! | `full_text` | `toString()` | 二〇二三年冬月二十 |
//!
//! 对照数据由 `tests/fixtures/gen_lunar.cjs` 用 lunar-javascript 生成（2024-01-01 ~ 2027-12-31 逐日），
//! 单测逐日比对。

use chrono::{Datelike, NaiveDate};
use tyme4rs::tyme::lunar::{LunarDay, LunarMonth};
use tyme4rs::tyme::solar::SolarDay;
use tyme4rs::tyme::{Culture, Tyme};

/// 某天的农历信息（字段同 `pc/src/utils/lunar.ts` 的 `LunarInfo`）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LunarInfo {
    /// 农历日（初一、初二……三十）
    pub day_text: String,
    /// 农历月（正月……冬月、腊月；闰月带「闰」）
    pub month_text: String,
    /// 干支年（如「甲辰年」），以正月初一换年
    pub year_text: String,
    /// 当天交节的节气，没有为空串
    pub solar_term: String,
    /// 传统节日（春节、元宵节、龙头节、端午节、七夕节、中秋节、重阳节、腊八节、除夕），没有为空串
    pub festival: String,
    /// 是否农历初一（日历上显示月份）
    pub is_first_day: bool,
    /// 完整农历日期（如「二〇二四年正月初一」）
    pub full_text: String,
}

/// 日历单元格显示文本的类型（对应前端的 `'festival' | 'solarTerm' | 'lunar'`，决定颜色）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LunarTextKind {
    Festival,
    SolarTerm,
    Lunar,
}

/// 支持的公历年份范围：tyme4rs 只接受公历 1~9999 年；判断「除夕」要看下一天，最后一年不算
const MIN_YEAR: i32 = 1;
const MAX_YEAR: i32 = 9998;

/// lunar-javascript `LunarUtil.MONTH`（下标 1~12）
const MONTH_NAMES: [&str; 12] = [
    "正", "二", "三", "四", "五", "六", "七", "八", "九", "十", "冬", "腊",
];

/// lunar-javascript `LunarUtil.DAY`（下标 1~30）
const DAY_NAMES: [&str; 30] = [
    "初一", "初二", "初三", "初四", "初五", "初六", "初七", "初八", "初九", "初十", "十一", "十二",
    "十三", "十四", "十五", "十六", "十七", "十八", "十九", "二十", "廿一", "廿二", "廿三", "廿四",
    "廿五", "廿六", "廿七", "廿八", "廿九", "三十",
];

/// lunar-javascript `LunarUtil.NUMBER`：年份逐位转写
const YEAR_DIGITS: [&str; 10] = ["〇", "一", "二", "三", "四", "五", "六", "七", "八", "九"];

/// lunar-javascript `LunarUtil.FESTIVAL`（键为非闰月的 月-日）
const FESTIVALS: [((usize, usize), &str); 8] = [
    ((1, 1), "春节"),
    ((1, 15), "元宵节"),
    ((2, 2), "龙头节"),
    ((5, 5), "端午节"),
    ((7, 7), "七夕节"),
    ((8, 15), "中秋节"),
    ((9, 9), "重阳节"),
    ((12, 8), "腊八节"),
];

const NEW_YEARS_EVE: &str = "除夕";

fn month_text(month: &LunarMonth) -> String {
    let leap = if month.is_leap() { "闰" } else { "" };
    format!("{leap}{}月", MONTH_NAMES[month.get_month() - 1])
}

fn year_in_chinese(year: isize) -> String {
    year.to_string()
        .chars()
        .map(|c| match c.to_digit(10) {
            Some(d) => YEAR_DIGITS[d as usize].to_string(),
            None => c.to_string(),
        })
        .collect()
}

/// lunar-javascript `getFestivals()[0]`：固定节日按非闰月的月-日匹配；腊月（含闰腊月）且第二天
/// 已是下一个农历年时为除夕
fn festival_of(day: &LunarDay, month: &LunarMonth) -> String {
    if !month.is_leap() {
        let key = (month.get_month(), day.get_day());
        if let Some((_, name)) = FESTIVALS.iter().find(|(k, _)| *k == key) {
            return name.to_string();
        }
    }
    if month.get_month() == 12 && day.get_day() >= 29 && day.next(1).get_year() != day.get_year() {
        return NEW_YEARS_EVE.to_string();
    }
    String::new()
}

/// 当天交节的节气名（lunar-javascript `getJieQi()`），不是交节日为空串
fn solar_term_of(solar: &SolarDay) -> String {
    let term_day = solar.get_term_day();
    if term_day.get_day_index() == 0 {
        term_day.get_solar_term().get_name()
    } else {
        String::new()
    }
}

/// 指定公历日期的农历信息（`getLunarInfo` 的等价物）。超出支持范围（公历 1~9998 年）时返回空信息。
pub fn lunar_info(date: NaiveDate) -> LunarInfo {
    if !(MIN_YEAR..=MAX_YEAR).contains(&date.year()) {
        return LunarInfo::default();
    }
    let solar = SolarDay::from_ymd(
        date.year() as isize,
        date.month() as usize,
        date.day() as usize,
    );
    let lunar = solar.get_lunar_day();
    let month = lunar.get_lunar_month();
    let day_text = DAY_NAMES[lunar.get_day() - 1].to_string();
    let month_text = month_text(&month);
    LunarInfo {
        year_text: format!("{}年", month.get_lunar_year().get_sixty_cycle().get_name()),
        solar_term: solar_term_of(&solar),
        festival: festival_of(&lunar, &month),
        is_first_day: lunar.get_day() == 1,
        full_text: format!(
            "{}年{}{}",
            year_in_chinese(lunar.get_year()),
            month_text,
            day_text
        ),
        day_text,
        month_text,
    }
}

/// 日历单元格应显示的文本（`getLunarDisplayText` 的等价物）：
/// 优先级 传统节日 > 节气 > 农历日（初一显示月份）
pub fn lunar_display_text(date: NaiveDate) -> (String, LunarTextKind) {
    let info = lunar_info(date);
    if !info.festival.is_empty() {
        return (info.festival, LunarTextKind::Festival);
    }
    if !info.solar_term.is_empty() {
        return (info.solar_term, LunarTextKind::SolarTerm);
    }
    let text = if info.is_first_day {
        info.month_text
    } else {
        info.day_text
    };
    (text, LunarTextKind::Lunar)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct FixtureDay {
        date: String,
        day_text: String,
        month_text: String,
        year_text: String,
        solar_term: String,
        festival: String,
        is_first_day: bool,
        full_text: String,
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// 与 lunar-javascript 逐日比对（2024-01-01 ~ 2027-12-31，含闰六月、除夕在廿九的年份）
    #[test]
    fn matches_lunar_javascript_day_by_day() {
        let fixture: Vec<FixtureDay> =
            serde_json::from_str(include_str!("../tests/fixtures/lunar_fixture.json")).unwrap();
        assert_eq!(fixture.len(), 1461, "四年逐日");
        let mut mismatches = Vec::new();
        for day in &fixture {
            let expected = LunarInfo {
                day_text: day.day_text.clone(),
                month_text: day.month_text.clone(),
                year_text: day.year_text.clone(),
                solar_term: day.solar_term.clone(),
                festival: day.festival.clone(),
                is_first_day: day.is_first_day,
                full_text: day.full_text.clone(),
            };
            let got = lunar_info(date(&day.date));
            if got != expected {
                mismatches.push(format!("{}: got {got:?}, expected {expected:?}", day.date));
            }
        }
        assert!(
            mismatches.is_empty(),
            "{} 天不一致：\n{}",
            mismatches.len(),
            mismatches
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    #[test]
    fn display_text_prefers_festival_then_solar_term_then_lunar_day() {
        // 2024-02-10 春节（正月初一）：节日优先于"正月"
        assert_eq!(
            lunar_display_text(date("2024-02-10")),
            ("春节".to_string(), LunarTextKind::Festival)
        );
        // 2024-02-04 立春（腊月廿五）
        assert_eq!(
            lunar_display_text(date("2024-02-04")),
            ("立春".to_string(), LunarTextKind::SolarTerm)
        );
        // 2024-03-10 二月初一：初一显示月份
        assert_eq!(
            lunar_display_text(date("2024-03-10")),
            ("二月".to_string(), LunarTextKind::Lunar)
        );
        // 2024-03-11 二月初二是龙头节；2024-03-12 二月初三
        assert_eq!(
            lunar_display_text(date("2024-03-12")),
            ("初三".to_string(), LunarTextKind::Lunar)
        );
        // 2025-07-25 闰六月初一
        assert_eq!(
            lunar_display_text(date("2025-07-25")),
            ("闰六月".to_string(), LunarTextKind::Lunar)
        );
    }

    #[test]
    fn out_of_range_dates_are_empty_instead_of_panicking() {
        assert_eq!(
            lunar_info(NaiveDate::from_ymd_opt(9999, 12, 31).unwrap()),
            LunarInfo::default()
        );
        assert_eq!(
            lunar_display_text(NaiveDate::from_ymd_opt(-5, 1, 1).unwrap()),
            (String::new(), LunarTextKind::Lunar)
        );
    }
}

//! 清单窗口的纯逻辑（无 GPUI）：重复提醒文案、提醒时间、象限、拖拽排序。
//! 前几项逐字移植自旧前端（`TodoItem.vue`、`utils/quadrant.ts`、`utils/color.ts`），
//! 都带单测。日历网格与跨天条在 `windows::calendar_window::logic`；版本比较（旧 `appStore.compareVersions`）在 `util::version`。

use mini_todo_core::db::Todo;
use mini_todo_core::db::time::{DefaultTime, parse_local_datetime_with};

use crate::theme::is_same_color;

// ----------------------------------------------------------------------
// 待办行
// ----------------------------------------------------------------------

/// 重复提醒 tooltip（`TodoItem.vue` 的 `repeatTooltip`）
pub fn repeat_tooltip(todo: &Todo) -> String {
    if !todo.repeat_enabled {
        return String::new();
    }
    let Some(kind) = todo.repeat_type.as_deref().filter(|t| !t.is_empty()) else {
        return String::new();
    };
    let interval = if todo.repeat_interval == 0 {
        1
    } else {
        todo.repeat_interval
    };
    const WEEKDAYS: [&str; 7] = ["一", "二", "三", "四", "五", "六", "日"];
    match kind {
        "daily" => {
            if interval == 1 {
                "每天重复".to_string()
            } else {
                format!("每 {interval} 天重复")
            }
        }
        "weekly" => {
            let days: Vec<String> = todo
                .repeat_weekdays
                .as_deref()
                .unwrap_or("")
                .split(',')
                .filter(|d| !d.is_empty())
                .map(|d| {
                    d.trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|i| WEEKDAYS.get(i))
                        .map_or_else(|| d.to_string(), |name| (*name).to_string())
                })
                .collect();
            let prefix = if interval == 1 {
                "每周".to_string()
            } else {
                format!("每 {interval} 周")
            };
            if days.is_empty() {
                prefix
            } else {
                format!("{prefix} 周{}", days.join("、"))
            }
        }
        "monthly" => {
            let day = match todo.repeat_month_day {
                Some(d) if d != 0 => d,
                _ => 1,
            };
            if interval == 1 {
                format!("每月 {day} 号")
            } else {
                format!("每 {interval} 月 {day} 号")
            }
        }
        _ => "重复提醒".to_string(),
    }
}

/// 提醒时间 `MM-DD HH:mm`（仅日期补 09:00；无法解析返回 `None`，整个元素不显示）
pub fn notify_text(todo: &Todo) -> Option<String> {
    let raw = todo.notify_at.as_deref()?;
    parse_local_datetime_with(raw, DefaultTime::Notify)
        .map(|dt| dt.format("%m-%d %H:%M").to_string())
}

/// JS `string.length`（UTF-16 码元数），子任务标题 tooltip 的阈值判断用
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

// ----------------------------------------------------------------------
// 象限
// ----------------------------------------------------------------------

/// 象限定义（`types/todo.ts QUADRANT_INFO`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadrantInfo {
    pub id: i32,
    pub name: &'static str,
    /// `#RRGGBB`
    pub color: &'static str,
    /// 象限色 RGB（底色是它的 10%）
    pub rgb: (u8, u8, u8),
}

pub const QUADRANTS: [QuadrantInfo; 4] = [
    QuadrantInfo {
        id: 1,
        name: "重要且紧急",
        color: "#EF4444",
        rgb: (239, 68, 68),
    },
    QuadrantInfo {
        id: 2,
        name: "重要不紧急",
        color: "#F59E0B",
        rgb: (245, 158, 11),
    },
    QuadrantInfo {
        id: 3,
        name: "紧急不重要",
        color: "#3B82F6",
        rgb: (59, 130, 246),
    },
    QuadrantInfo {
        id: 4,
        name: "不紧急不重要",
        color: "#10B981",
        rgb: (16, 185, 129),
    },
];

/// 新建待办的默认颜色
pub const DEFAULT_COLOR: &str = "#10B981";

/// 象限默认色（`getQuadrantColor`）
pub fn quadrant_color(quadrant: i32) -> &'static str {
    QUADRANTS
        .iter()
        .find(|q| q.id == quadrant)
        .map_or(DEFAULT_COLOR, |q| q.color)
}

/// 切换象限时的颜色（`resolveQuadrantColor`）：仍是旧象限默认色 → 跟随新象限；用户自选色保留
pub fn resolve_quadrant_color(current: &str, prev: i32, next: i32) -> String {
    if is_same_color(current, quadrant_color(prev)) {
        quadrant_color(next).to_string()
    } else {
        current.to_string()
    }
}

/// 把某个象限的新相对顺序合并回全局顺序（`mergeQuadrantOrder`）
pub fn merge_quadrant_order(global: &[i64], quadrant: &[i64]) -> Vec<i64> {
    let mut queue: std::collections::VecDeque<i64> = quadrant.iter().copied().collect();
    let mut merged: Vec<i64> = global
        .iter()
        .map(|id| {
            if quadrant.contains(id) {
                queue.pop_front().unwrap_or(*id)
            } else {
                *id
            }
        })
        .collect();
    merged.extend(queue);
    merged
}

/// 非法象限归 1，返回下标 0..4
pub fn quadrant_index(quadrant: i32) -> usize {
    match quadrant {
        q @ 1..=4 => (q - 1) as usize,
        _ => 0,
    }
}

// ----------------------------------------------------------------------
// 拖拽排序
// ----------------------------------------------------------------------

/// 拖拽中占位应在的位置：`others_mid_y` 是除被拖项之外各行（按当前显示顺序）的垂直中点，
/// 返回中点在指针之上的行数。被拖项插到这个下标后，各行位置的变化不会让结果再变（不抖动）
pub fn insertion_index(others_mid_y: &[f32], pointer_y: f32) -> usize {
    others_mid_y.iter().filter(|mid| **mid < pointer_y).count()
}

/// 把 `id` 移到 `index`（`index` 按去掉 `id` 之后的列表计）；不在列表里时插入
pub fn move_to(order: &mut Vec<i64>, id: i64, index: usize) {
    order.retain(|x| *x != id);
    let index = index.min(order.len());
    order.insert(index, id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn todo(id: i64) -> Todo {
        Todo {
            id,
            title: format!("t{id}"),
            description: None,
            color: "#EF4444".into(),
            quadrant: 1,
            notify_at: None,
            notify_before: 0,
            notified: false,
            completed: false,
            sort_order: 0,
            start_time: None,
            end_time: None,
            created_at: "2026-10-01 08:00:00".into(),
            updated_at: String::new(),
            repeat_enabled: false,
            repeat_type: None,
            repeat_interval: 1,
            repeat_weekdays: None,
            repeat_month_day: None,
            list_id: Some(1),
            list_updated_at: None,
            subtasks: vec![],
        }
    }

    fn repeat(kind: &str, interval: i32, weekdays: Option<&str>, day: Option<i32>) -> Todo {
        Todo {
            repeat_enabled: true,
            repeat_type: Some(kind.into()),
            repeat_interval: interval,
            repeat_weekdays: weekdays.map(Into::into),
            repeat_month_day: day,
            ..todo(1)
        }
    }

    #[test]
    fn repeat_tooltip_matches_frontend() {
        assert_eq!(repeat_tooltip(&repeat("daily", 1, None, None)), "每天重复");
        assert_eq!(
            repeat_tooltip(&repeat("daily", 3, None, None)),
            "每 3 天重复"
        );
        assert_eq!(repeat_tooltip(&repeat("daily", 0, None, None)), "每天重复");
        assert_eq!(
            repeat_tooltip(&repeat("weekly", 1, Some("1,3,5"), None)),
            "每周 周一、三、五"
        );
        assert_eq!(repeat_tooltip(&repeat("weekly", 1, None, None)), "每周");
        assert_eq!(
            repeat_tooltip(&repeat("weekly", 2, Some("7,x"), None)),
            "每 2 周 周日、x"
        );
        assert_eq!(
            repeat_tooltip(&repeat("weekly", 2, Some(""), None)),
            "每 2 周"
        );
        assert_eq!(
            repeat_tooltip(&repeat("monthly", 1, None, Some(25))),
            "每月 25 号"
        );
        assert_eq!(
            repeat_tooltip(&repeat("monthly", 3, None, None)),
            "每 3 月 1 号"
        );
        assert_eq!(repeat_tooltip(&repeat("yearly", 1, None, None)), "重复提醒");
        let mut off = repeat("daily", 1, None, None);
        off.repeat_enabled = false;
        assert_eq!(repeat_tooltip(&off), "");
        let mut no_type = repeat("daily", 1, None, None);
        no_type.repeat_type = None;
        assert_eq!(repeat_tooltip(&no_type), "");
    }

    #[test]
    fn notify_text_formats_and_skips_garbage() {
        let mut t = todo(1);
        t.notify_at = Some("2026-10-15 17:30:00".into());
        assert_eq!(notify_text(&t).as_deref(), Some("10-15 17:30"));
        t.notify_at = Some("2026-10-16".into());
        assert_eq!(notify_text(&t).as_deref(), Some("10-16 09:00"));
        t.notify_at = Some("2026-10-16T06:30".into());
        assert_eq!(notify_text(&t).as_deref(), Some("10-16 06:30"));
        t.notify_at = Some("garbage".into());
        assert_eq!(notify_text(&t), None);
        t.notify_at = None;
        assert_eq!(notify_text(&t), None);
        assert_eq!(utf16_len("预约评审会议室并发出日历邀请给所有相关同事"), 21);
        assert_eq!(utf16_len("a😀"), 3);
    }

    #[test]
    fn quadrant_color_resolution_and_merge() {
        assert_eq!(resolve_quadrant_color("#ef4444", 1, 3), "#3B82F6");
        assert_eq!(resolve_quadrant_color(" #EF4444 ", 1, 2), "#F59E0B");
        assert_eq!(resolve_quadrant_color("#6366F1", 2, 4), "#6366F1");
        assert_eq!(quadrant_color(9), DEFAULT_COLOR);
        assert_eq!(
            merge_quadrant_order(&[1, 2, 3, 4, 5], &[5, 3]),
            vec![1, 2, 5, 4, 3]
        );
        assert_eq!(merge_quadrant_order(&[1, 2], &[2, 9]), vec![1, 2, 9]);
        assert_eq!(quadrant_index(0), 0);
        assert_eq!(quadrant_index(4), 3);
    }

    #[test]
    fn drag_insertion_is_stable() {
        // 三行（除被拖项外），中点 20 / 60 / 100
        assert_eq!(insertion_index(&[20., 60., 100.], 10.), 0);
        assert_eq!(insertion_index(&[20., 60., 100.], 61.), 2);
        assert_eq!(insertion_index(&[20., 60., 100.], 500.), 3);
        let mut order = vec![1, 2, 3, 4];
        move_to(&mut order, 1, 2);
        assert_eq!(order, vec![2, 3, 1, 4]);
        move_to(&mut order, 4, 0);
        assert_eq!(order, vec![4, 2, 3, 1]);
        move_to(&mut order, 9, 99);
        assert_eq!(order, vec![4, 2, 3, 1, 9]);
    }
}

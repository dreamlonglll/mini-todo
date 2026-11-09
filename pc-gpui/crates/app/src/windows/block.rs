//! 桌面块：待办清单窗口（每个清单一个）与日历窗口（单例）。它们和收纳箱一样是独立的顶层窗口：各自锁定（固定模式）、
//! 各自按屏幕组合记几何，与收纳箱同一个吸附组。
//!
//! 任务 `11-09-multi-list-windows` 第 3 步只有现在的主窗口一个使用者（[`BlockKey::DEFAULT_LIST`]），界面上看不出拆分；
//! 拆成清单窗口 + 日历窗口是第 4 步。
//!
//! | 能力 | 位置 |
//! |---|---|
//! | 模式（普通 / 经典固定 / 嵌入桌面）、贴边隐藏、唤起置顶、几何保存 | `services::window_mode`（按 [`BlockKey`] 的状态表） |
//! | 几何与锁定的持久化接缝（第 4 步换成 core 的窗口属性表） | `services::window_mode::store` |
//! | 吸附（与收纳箱同一组） | `services::snap` + `windows::snap` |

use std::fmt;

/// 默认清单的 id（core 迁移 v31 建的"待办清单"，id 固定为 1、不可删除；现在的主窗口就是它的窗口）
pub const DEFAULT_LIST_ID: i64 = 1;

/// 桌面块窗口的标识：`window_mode` 的状态表、几何与锁定的持久化接缝、吸附组共用。
///
/// 排序（`Ord`）：清单在前（按 id，默认清单 id 最小排第一），日历最后——轮询、重置位置按这个顺序处理，结果是确定的
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum BlockKey {
    /// 待办清单窗口（清单 id）
    List(i64),
    /// 日历窗口（单例）
    Calendar,
}

impl BlockKey {
    /// 默认清单窗口（= 现在的主窗口）
    pub const DEFAULT_LIST: BlockKey = BlockKey::List(DEFAULT_LIST_ID);
}

impl fmt::Display for BlockKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BlockKey::List(id) => write!(f, "清单窗口 #{id}"),
            BlockKey::Calendar => f.write_str("日历窗口"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_order_the_default_list_first_and_the_calendar_last() {
        // 新清单 id 是 UNIX 毫秒 × 1000 + 随机数（与 cloud 同一算法），远大于默认清单的 1
        let mut keys = vec![
            BlockKey::Calendar,
            BlockKey::List(1_762_000_000_000_123),
            BlockKey::DEFAULT_LIST,
        ];
        keys.sort();
        assert_eq!(
            keys,
            vec![
                BlockKey::DEFAULT_LIST,
                BlockKey::List(1_762_000_000_000_123),
                BlockKey::Calendar
            ]
        );
        assert_eq!(BlockKey::DEFAULT_LIST, BlockKey::List(1));
        assert_eq!(BlockKey::DEFAULT_LIST.to_string(), "清单窗口 #1");
        assert_eq!(BlockKey::Calendar.to_string(), "日历窗口");
    }
}

//! 桌面块窗口（清单 / 日历窗口）"在当前屏幕组合下的几何与锁定"的持久化（阻塞函数，在后台线程或建窗前同步调用）。
//!
//! 存在 core 的窗口属性表（v31 `window_layouts`，经 `mini_todo_core::window_prefs`）：每个 (窗口 key, 屏幕组合) 一行，
//! 客户区（= 可见区域）的物理像素 `x / y / width / height` + 是否锁定，口径与以前的 `screen_configs` 相同（macOS 是全局点 ×
//! 统一系数）。迁移 v31 已把原主窗口在各屏幕组合下的记录复制给默认清单窗口（`list:1`），升级后位置不变。
//!
//! **屏幕组合的登记**：设置 → 屏幕配置页按 `screen_configs` 列出屏幕组合（显示名、"固定模式 / 普通模式 | 位置"）。
//! 默认清单窗口（总是存在、启动时没有记录就建一条初始记录）保存时照旧顺带 upsert 一行 `screen_configs`（同样的几何与锁定），
//! 新出现的屏幕组合因此仍会出现在设置页里；删除屏幕配置时 core 连带删除该组合下全部窗口的布局。其它窗口只写 `window_layouts`。
//!
//! 不再写旧的全局设置键（`save_window_settings` 的 `is_fixed` / `window_position` / `window_size` / `text_theme` …）：
//! 锁定、日 / 夜、视图都按窗口存了，同步键 `is_fixed` / `text_theme` / `view_mode` / `show_calendar` 只作迁移初始值；
//! `window_position` / `window_size` 在 3.x 里没有任何读取方（恢复几何一直按屏幕组合），只随导出备份，留着最后一次的值即可。

use mini_todo_core::Database;
use mini_todo_core::db::SaveScreenConfigRequest;
use mini_todo_core::settings;
use mini_todo_core::window_prefs::{self, WindowLayout};

use crate::platform::PhysRect;
use crate::windows::block::BlockKey;

/// 某个窗口在某个屏幕组合下记下的几何与锁定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockPlacement {
    /// 客户区（= 可见区域）的物理像素矩形；记录里没有有效尺寸时 `None`（调用方按默认位置建窗）
    pub rect: Option<PhysRect>,
    /// 是否锁定（经典固定与嵌入桌面都为 true）
    pub fixed: bool,
}

/// 布局记录 → 窗口要用的几何与锁定（`width / height > 0` 才算有几何，同以前的 `screen_configs`）
pub fn placement_of(layout: &WindowLayout) -> BlockPlacement {
    BlockPlacement {
        rect: (layout.width > 0 && layout.height > 0)
            .then(|| PhysRect::new(layout.x, layout.y, layout.width, layout.height)),
        fixed: layout.is_fixed,
    }
}

/// 窗口的几何与锁定 → 布局记录
pub fn layout_of(key: BlockKey, screen_id: &str, rect: PhysRect, fixed: bool) -> WindowLayout {
    WindowLayout {
        key: key.window_key(),
        screen_config_id: screen_id.to_string(),
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
        is_fixed: fixed,
    }
}

/// 读 `key` 在屏幕组合 `screen_id` 下的记录（没有记录 `Ok(None)`）
pub fn load(
    db: &Database,
    key: BlockKey,
    screen_id: &str,
) -> Result<Option<BlockPlacement>, String> {
    Ok(window_prefs::get_layout(db, key.window_key(), screen_id)?
        .as_ref()
        .map(placement_of))
}

/// 写 `key` 在屏幕组合 `screen_id` 下的几何（贴边隐藏中由调用方换成锚点）与锁定。清单已被删除（同步 / 本机删除后窗口还没关）
/// 时 core 忽略这次写入，这里也不报错。默认清单窗口另外登记屏幕组合（见模块文档）
pub fn save(
    db: &Database,
    key: BlockKey,
    screen_id: &str,
    rect: PhysRect,
    fixed: bool,
) -> Result<(), String> {
    if !window_prefs::save_layout(db, &layout_of(key, screen_id, rect, fixed))? {
        log::debug!("[window] {key} 对应的清单已不存在，跳过保存几何");
        return Ok(());
    }
    if key == BlockKey::DEFAULT_LIST {
        settings::save_screen_config(
            db,
            &SaveScreenConfigRequest {
                config_id: screen_id.to_string(),
                display_name: Some(settings::screen_config_display_name(screen_id)),
                window_x: rect.x,
                window_y: rect.y,
                window_width: rect.width,
                window_height: rect.height,
                is_fixed: fixed,
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: &str = "1_1920x1080@100";
    const OTHER_SCREEN: &str = "2_1920x1080@100_2560x1440@125";

    #[test]
    fn layouts_round_trip_per_window_and_screen_combination() {
        let db = Database::new_in_memory().unwrap();
        let work = mini_todo_core::lists::create(&db, "工作").unwrap();
        let keys = [
            BlockKey::DEFAULT_LIST,
            BlockKey::List(work.id),
            BlockKey::Calendar,
        ];
        for key in keys {
            assert_eq!(load(&db, key, SCREEN).unwrap(), None, "{key}");
        }
        for (index, key) in keys.into_iter().enumerate() {
            let rect = PhysRect::new(-1720 + index as i32 * 400, 96, 380, 600);
            save(&db, key, SCREEN, rect, index == 1).unwrap();
        }
        for (index, key) in keys.into_iter().enumerate() {
            assert_eq!(
                load(&db, key, SCREEN).unwrap(),
                Some(BlockPlacement {
                    rect: Some(PhysRect::new(-1720 + index as i32 * 400, 96, 380, 600)),
                    fixed: index == 1
                }),
                "{key}"
            );
            // 别的屏幕组合各存各的
            assert_eq!(load(&db, key, OTHER_SCREEN).unwrap(), None, "{key}");
        }
        // 存在 core 的窗口布局表里（客户区物理像素 + 锁定）
        let stored = window_prefs::get_layout(&db, BlockKey::Calendar.window_key(), SCREEN)
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                stored.x,
                stored.y,
                stored.width,
                stored.height,
                stored.is_fixed
            ),
            (-920, 96, 380, 600, false)
        );
    }

    #[test]
    fn the_default_list_also_registers_the_screen_combination() {
        let db = Database::new_in_memory().unwrap();
        save(
            &db,
            BlockKey::Calendar,
            OTHER_SCREEN,
            PhysRect::new(0, 0, 600, 600),
            false,
        )
        .unwrap();
        assert!(
            settings::list_screen_configs(&db).unwrap().is_empty(),
            "其它窗口只写窗口布局"
        );

        let rect = PhysRect::new(-1720, 96, 380, 600);
        save(&db, BlockKey::DEFAULT_LIST, SCREEN, rect, true).unwrap();
        // 设置的屏幕配置页按 screen_configs 列出组合：一行，带显示名、默认清单窗口的几何与锁定
        let row = settings::get_screen_config(&db, SCREEN).unwrap().unwrap();
        assert_eq!(
            (
                row.window_x,
                row.window_y,
                row.window_width,
                row.window_height,
                row.is_fixed
            ),
            (-1720, 96, 380, 600, true)
        );
        assert_eq!(
            row.display_name.as_deref(),
            Some(settings::screen_config_display_name(SCREEN).as_str())
        );
        // 删除屏幕配置连带删除该组合下全部窗口的布局
        settings::delete_screen_config(&db, SCREEN).unwrap();
        assert_eq!(load(&db, BlockKey::DEFAULT_LIST, SCREEN).unwrap(), None);
        assert!(
            load(&db, BlockKey::Calendar, OTHER_SCREEN)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_record_without_a_size_keeps_only_the_lock() {
        let db = Database::new_in_memory().unwrap();
        save(
            &db,
            BlockKey::DEFAULT_LIST,
            SCREEN,
            PhysRect::new(10, 20, 0, 600),
            true,
        )
        .unwrap();
        assert_eq!(
            load(&db, BlockKey::DEFAULT_LIST, SCREEN).unwrap(),
            Some(BlockPlacement {
                rect: None,
                fixed: true
            })
        );
    }

    #[test]
    fn windows_of_deleted_lists_are_not_stored() {
        let db = Database::new_in_memory().unwrap();
        let ghost = BlockKey::List(1_762_000_000_000_001);
        save(&db, ghost, SCREEN, PhysRect::new(1, 2, 300, 400), true).unwrap();
        assert_eq!(load(&db, ghost, SCREEN).unwrap(), None);

        let work = mini_todo_core::lists::create(&db, "工作").unwrap();
        let key = BlockKey::List(work.id);
        save(&db, key, SCREEN, PhysRect::new(1, 2, 300, 400), false).unwrap();
        mini_todo_core::lists::delete(&db, work.id).unwrap();
        assert_eq!(load(&db, key, SCREEN).unwrap(), None, "删清单时连带删布局");
    }
}

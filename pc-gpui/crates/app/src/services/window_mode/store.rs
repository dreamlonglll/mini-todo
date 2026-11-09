//! 桌面块窗口"在当前屏幕组合下的几何与锁定"的持久化接缝（阻塞函数，在后台线程或建窗前同步调用）。
//!
//! 现在（任务 `11-09-multi-list-windows` 第 3 步）只有默认清单窗口（= 原主窗口，[`BlockKey::DEFAULT_LIST`]）有记录，
//! 存在 `screen_configs`（按屏幕组合：客户区位置 / 尺寸（物理像素）+ `is_fixed`），与以前逐字段相同；其它窗口还没有存储，
//! 读到 `None`、写入什么都不做。第 4 步把这两个函数的实现换成 core 的窗口属性表（按 `(窗口, 屏幕组合)` 存同样的三项），
//! 调用方（`window_mode` 的启动恢复与保存）不用改。
//!
//! 只管几何与锁定：默认清单窗口保存时顺带写的旧全局设置键（`save_window_settings` 的 `is_fixed` / `window_position` /
//! `window_size` / 主题 / 贴边…，参与同步与导出）不属于这里，在 `window_mode::save_window_state_with` 里单独写。

use mini_todo_core::Database;
use mini_todo_core::db::SaveScreenConfigRequest;
use mini_todo_core::settings;

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

/// 读 `key` 在屏幕组合 `screen_id` 下的记录（没有记录 `Ok(None)`）
pub fn load(
    db: &Database,
    key: BlockKey,
    screen_id: &str,
) -> Result<Option<BlockPlacement>, String> {
    if key != BlockKey::DEFAULT_LIST {
        return Ok(None);
    }
    let saved = settings::get_screen_config(db, screen_id)?;
    Ok(saved.map(|c| BlockPlacement {
        rect: (c.window_width > 0 && c.window_height > 0)
            .then(|| PhysRect::new(c.window_x, c.window_y, c.window_width, c.window_height)),
        fixed: c.is_fixed,
    }))
}

/// 写 `key` 在屏幕组合 `screen_id` 下的几何（贴边隐藏中由调用方换成锚点）与锁定
pub fn save(
    db: &Database,
    key: BlockKey,
    screen_id: &str,
    rect: PhysRect,
    fixed: bool,
) -> Result<(), String> {
    if key != BlockKey::DEFAULT_LIST {
        log::debug!("[window] {key} 的几何还没有存储（第 4 步接入窗口属性表），跳过保存");
        return Ok(());
    }
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
    )
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: &str = "1_1920x1080@100";

    #[test]
    fn default_list_round_trips_through_screen_configs() {
        let db = Database::new_in_memory().unwrap();
        assert_eq!(load(&db, BlockKey::DEFAULT_LIST, SCREEN).unwrap(), None);

        let rect = PhysRect::new(-1720, 96, 380, 600);
        save(&db, BlockKey::DEFAULT_LIST, SCREEN, rect, true).unwrap();
        assert_eq!(
            load(&db, BlockKey::DEFAULT_LIST, SCREEN).unwrap(),
            Some(BlockPlacement {
                rect: Some(rect),
                fixed: true
            })
        );
        // 与以前的主窗口逐字段相同：screen_configs 一行，带显示名
        let row = settings::get_screen_config(&db, SCREEN).unwrap().unwrap();
        assert_eq!(
            (
                row.window_x,
                row.window_y,
                row.window_width,
                row.window_height
            ),
            (-1720, 96, 380, 600)
        );
        assert!(row.is_fixed);
        assert_eq!(
            row.display_name.as_deref(),
            Some(settings::screen_config_display_name(SCREEN).as_str())
        );
        // 别的屏幕组合各存各的
        assert_eq!(
            load(&db, BlockKey::DEFAULT_LIST, "2_1920x1080@100_2560x1440@125").unwrap(),
            None
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
    fn other_windows_are_not_stored_yet() {
        let db = Database::new_in_memory().unwrap();
        for key in [BlockKey::List(1_762_000_000_000_001), BlockKey::Calendar] {
            save(&db, key, SCREEN, PhysRect::new(1, 2, 300, 400), true).unwrap();
            assert_eq!(load(&db, key, SCREEN).unwrap(), None, "{key}");
        }
        assert!(
            settings::list_screen_configs(&db).unwrap().is_empty(),
            "不写主窗口的屏幕配置"
        );
    }
}

//! 清单窗口 / 日历窗口的设备本地属性（v31 `window_prefs` / `window_layouts`）。
//!
//! 不同步、不导出、不接 `local_seq`（性质同 `screen_configs` 与收纳箱表）。窗口 key（[`WindowKey`]）：清单窗口
//! `list:<清单 id>`、日历窗口 `calendar`。
//!
//! - 每个窗口（[`WindowPrefs`]）：是否隐藏、深色、视图模式（仅清单）、展开全部子任务（仅清单）、
//!   日历清单筛选（仅日历：`None` = 全部，含以后新建的清单；否则是一组清单 id）
//! - 每个 (窗口, 屏幕组合)（[`WindowLayout`]）：客户区物理像素 `x / y / width / height` 与是否锁定，口径同 `screen_configs`
//!   （macOS 是全局点 × 统一系数）；屏幕组合 ID 由 `settings::screen_config_id` 计算
//! - 没有记录的窗口按 [`WindowPrefs::new`] 的默认值（显示、浅色、列表视图、不展开、全部清单）；写入时按需补建记录
//! - 清单不存在（含刚被同步删除）时写入一律忽略（返回 `None` / `false`），不留孤儿记录。清单被删除时（本机删除、
//!   同步来的墓碑、强制拉取、导入备份）它的窗口记录一并删除、从日历筛选里移出，筛选空了回到"全部"（由 core 在删除
//!   清单的同一事务里完成，app 只需关掉对应的窗口）
//! - 删除屏幕配置时连带删除该组合下的全部窗口布局（`settings::delete_screen_config`）
//!
//! 迁移 v31 的种子：`list:1` 继承原主窗口（深色、视图、各屏幕组合的位置 / 尺寸 / 锁定），`calendar` 隐藏 = 原来没开日历
//! （见 `db::migrations::migration_v31`）。展开全部子任务原来存在 app 的 `ui-state.json`，由 app 首次启动时迁移。

use crate::db::models::DEFAULT_LIST_ID;
use crate::db::Database;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashSet;
use std::fmt;

/// 日历窗口的 key
pub const CALENDAR_WINDOW_KEY: &str = "calendar";
/// 清单窗口 key 的前缀（`list:<清单 id>`）
pub const LIST_WINDOW_KEY_PREFIX: &str = "list:";

/// 窗口 key：清单窗口（每个清单一个）或日历窗口（单例）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WindowKey {
    List(i64),
    Calendar,
}

impl WindowKey {
    /// 解析存储的 key；只认规范写法（`calendar`、`list:<正整数，无前导 0 / +>`），其它返回 `None`
    pub fn parse(key: &str) -> Option<Self> {
        if key == CALENDAR_WINDOW_KEY {
            return Some(Self::Calendar);
        }
        let id = key.strip_prefix(LIST_WINDOW_KEY_PREFIX)?;
        let parsed: i64 = id.parse().ok()?;
        (parsed >= 1 && parsed.to_string() == id).then_some(Self::List(parsed))
    }

    /// 存储用的 key 字符串
    pub fn as_key(&self) -> String {
        match self {
            Self::List(id) => format!("{LIST_WINDOW_KEY_PREFIX}{id}"),
            Self::Calendar => CALENDAR_WINDOW_KEY.to_string(),
        }
    }

    /// 清单窗口对应的清单 id
    pub fn list_id(&self) -> Option<i64> {
        match self {
            Self::List(id) => Some(*id),
            Self::Calendar => None,
        }
    }
}

impl fmt::Display for WindowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_key())
    }
}

/// 清单窗口的视图模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    List,
    Quadrant,
}

impl ViewMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Quadrant => "quadrant",
        }
    }

    /// `"quadrant"` → 四象限，其它（含无法识别的值）→ 列表
    pub fn parse(s: &str) -> Self {
        if s == "quadrant" {
            Self::Quadrant
        } else {
            Self::List
        }
    }
}

/// 一个窗口的设备本地属性
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowPrefs {
    pub key: WindowKey,
    /// 关闭窗口 = 隐藏；重启后保持（默认清单窗口启动时总是显示，由 app 决定）
    pub hidden: bool,
    /// 深色主题（底色 / 透明度仍是全局设置）
    pub dark: bool,
    /// 列表 / 四象限（仅清单窗口）
    pub view_mode: ViewMode,
    /// 展开全部子任务（仅清单窗口）
    pub expand_all: bool,
    /// 日历清单筛选（仅日历窗口）：`None` = 全部（含以后新建的清单）；否则是一组现有清单的 id（升序、不重复、非空）
    pub calendar_filter: Option<Vec<i64>>,
}

impl WindowPrefs {
    /// 没有记录时的默认值：显示、浅色、列表视图、不展开、全部清单（同表的列默认值）
    pub fn new(key: WindowKey) -> Self {
        Self {
            key,
            hidden: false,
            dark: false,
            view_mode: ViewMode::List,
            expand_all: false,
            calendar_filter: None,
        }
    }
}

/// [`update_prefs`] 的入参：只更新给出的字段（日历筛选走 [`set_calendar_filter`]）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UpdateWindowPrefs {
    pub hidden: Option<bool>,
    pub dark: Option<bool>,
    pub view_mode: Option<ViewMode>,
    pub expand_all: Option<bool>,
}

/// 一个窗口在某个屏幕组合下的布局（客户区物理像素）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowLayout {
    pub key: WindowKey,
    /// 屏幕组合 ID（与 `screen_configs.config_id` / `fence_layouts.screen_config_id` 同一格式）
    pub screen_config_id: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// 锁定（固定模式）：按屏幕组合记忆，同原来的 `screen_configs.is_fixed`
    pub is_fixed: bool,
}

// ============================================================================
// 读
// ============================================================================

/// 读取一个窗口的属性记录；没有记录返回 `None`（调用方按 [`WindowPrefs::new`] 的默认值处理，
/// 例如同步来的新清单"本机没有窗口记录 → 自动开窗"）
pub fn get_prefs(db: &Database, key: WindowKey) -> Result<Option<WindowPrefs>, String> {
    db.with_connection(|conn| load_prefs(conn, key))
        .map_err(|e| e.to_string())
}

/// 全部窗口的属性记录（按 key 排序；无法识别的 key 跳过）
pub fn list_prefs(db: &Database) -> Result<Vec<WindowPrefs>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT window_key, {PREFS_FIELDS} FROM window_prefs ORDER BY window_key"
        ))?;
        let rows = stmt.query_map([], |r| {
            let key: String = r.get(0)?;
            match WindowKey::parse(&key) {
                Some(key) => prefs_from_row(key, r, 1).map(Some),
                None => Ok(None),
            }
        })?;
        let mut out = Vec::new();
        for row in rows {
            if let Some(prefs) = row? {
                out.push(prefs);
            }
        }
        Ok(out)
    })
    .map_err(|e| e.to_string())
}

/// 读取一个窗口在某个屏幕组合下的布局
pub fn get_layout(
    db: &Database,
    key: WindowKey,
    screen_config_id: &str,
) -> Result<Option<WindowLayout>, String> {
    db.with_connection(|conn| {
        conn.query_row(
            &format!(
                "SELECT {LAYOUT_COLUMNS} FROM window_layouts
                 WHERE window_key = ?1 AND screen_config_id = ?2"
            ),
            params![key.as_key(), screen_config_id],
            layout_from_row,
        )
        .optional()
        .map(Option::flatten)
    })
    .map_err(|e| e.to_string())
}

/// 某个屏幕组合下全部窗口的布局（启动 / 屏幕组合变化时一次读出；无法识别的 key 跳过）
pub fn layouts_for_screen(
    db: &Database,
    screen_config_id: &str,
) -> Result<Vec<WindowLayout>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {LAYOUT_COLUMNS} FROM window_layouts
             WHERE screen_config_id = ?1 ORDER BY window_key"
        ))?;
        let rows = stmt.query_map([screen_config_id], layout_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            if let Some(layout) = row? {
                out.push(layout);
            }
        }
        Ok(out)
    })
    .map_err(|e| e.to_string())
}

// ============================================================================
// 写
// ============================================================================

/// 更新一个窗口的属性（只改给出的字段；没有记录时先按默认值补建）。返回更新后的属性；
/// 清单窗口对应的清单不存在时什么都不写，返回 `None`
pub fn update_prefs(
    db: &Database,
    key: WindowKey,
    update: &UpdateWindowPrefs,
) -> Result<Option<WindowPrefs>, String> {
    db.with_transaction(|tx| -> rusqlite::Result<Option<WindowPrefs>> {
        if !window_exists(tx, key)? {
            log::warn!("[window-prefs] 忽略不存在的清单的窗口属性写入: {key}");
            return Ok(None);
        }
        ensure_row(tx, key)?;
        if *update != UpdateWindowPrefs::default() {
            tx.execute(
                "UPDATE window_prefs SET
                    hidden = COALESCE(?1, hidden),
                    dark = COALESCE(?2, dark),
                    view_mode = COALESCE(?3, view_mode),
                    expand_all = COALESCE(?4, expand_all),
                    updated_at = datetime('now', 'localtime')
                 WHERE window_key = ?5",
                params![
                    update.hidden.map(i32::from),
                    update.dark.map(i32::from),
                    update.view_mode.map(ViewMode::as_str),
                    update.expand_all.map(i32::from),
                    key.as_key(),
                ],
            )?;
        }
        load_prefs(tx, key)
    })
    .map_err(|e| e.to_string())
}

/// 设置日历的清单筛选：`None` = 全部；`Some(ids)` 去掉不存在的清单、去重排序后保存，结果为空时回到"全部"。
/// 返回实际保存的筛选
pub fn set_calendar_filter(
    db: &Database,
    filter: Option<&[i64]>,
) -> Result<Option<Vec<i64>>, String> {
    db.with_transaction(|tx| -> rusqlite::Result<Option<Vec<i64>>> {
        ensure_row(tx, WindowKey::Calendar)?;
        let normalized = match filter {
            None => None,
            Some(ids) => normalize_filter(ids, &list_ids(tx)?),
        };
        tx.execute(
            "UPDATE window_prefs SET calendar_filter = ?1, updated_at = datetime('now', 'localtime')
             WHERE window_key = ?2",
            params![filter_text(normalized.as_deref()), CALENDAR_WINDOW_KEY],
        )?;
        Ok(normalized)
    })
    .map_err(|e| e.to_string())
}

/// 保存一个窗口在某个屏幕组合下的布局（upsert；窗口没有属性记录时按默认值补建）。
/// 清单不存在时什么都不写，返回 `false`
pub fn save_layout(db: &Database, layout: &WindowLayout) -> Result<bool, String> {
    db.with_transaction(|tx| save_layout_row(tx, layout))
        .map_err(|e: rusqlite::Error| e.to_string())
}

/// 一个事务里保存多个布局（托盘「重置位置」等）：清单已不存在的跳过，返回实际保存的条数
pub fn save_layouts(db: &Database, layouts: &[WindowLayout]) -> Result<usize, String> {
    db.with_transaction(|tx| -> rusqlite::Result<usize> {
        let mut saved = 0;
        for layout in layouts {
            if save_layout_row(tx, layout)? {
                saved += 1;
            }
        }
        Ok(saved)
    })
    .map_err(|e| e.to_string())
}

/// 删除一个窗口的属性与全部布局（之后读到的是默认值）
pub fn delete_window(db: &Database, key: WindowKey) -> Result<(), String> {
    db.with_transaction(|tx| delete_window_rows(tx, key))
        .map_err(|e: rusqlite::Error| e.to_string())
}

// ============================================================================
// core 内部：清单被删除 / 屏幕配置被删除时的连带清理（调用方提供事务）
// ============================================================================

/// 清单 `list_id` 已被删除：删掉它的窗口记录，并从日历筛选里移出
pub(crate) fn forget_list(conn: &Connection, list_id: i64) -> rusqlite::Result<()> {
    if list_id != DEFAULT_LIST_ID {
        delete_window_rows(conn, WindowKey::List(list_id))?;
    }
    prune_calendar_filter(conn, &list_ids(conn)?)
}

/// 删除某个屏幕组合下全部窗口的布局（删除屏幕配置时连带）
pub(crate) fn delete_layouts_for_screen(
    conn: &Connection,
    screen_config_id: &str,
) -> rusqlite::Result<usize> {
    conn.execute(
        "DELETE FROM window_layouts WHERE screen_config_id = ?1",
        [screen_config_id],
    )
}

// ============================================================================
// 内部
// ============================================================================

const PREFS_FIELDS: &str = "hidden, dark, view_mode, expand_all, calendar_filter";

const LAYOUT_COLUMNS: &str = "window_key, screen_config_id, x, y, width, height, is_fixed";

fn prefs_from_row(
    key: WindowKey,
    r: &rusqlite::Row,
    first: usize,
) -> rusqlite::Result<WindowPrefs> {
    Ok(WindowPrefs {
        key,
        hidden: r.get::<_, i64>(first)? != 0,
        dark: r.get::<_, i64>(first + 1)? != 0,
        view_mode: ViewMode::parse(&r.get::<_, String>(first + 2)?),
        expand_all: r.get::<_, i64>(first + 3)? != 0,
        calendar_filter: parse_filter(r.get::<_, Option<String>>(first + 4)?.as_deref()),
    })
}

fn load_prefs(conn: &Connection, key: WindowKey) -> rusqlite::Result<Option<WindowPrefs>> {
    conn.query_row(
        &format!("SELECT {PREFS_FIELDS} FROM window_prefs WHERE window_key = ?1"),
        [key.as_key()],
        |r| prefs_from_row(key, r, 0),
    )
    .optional()
}

/// 布局行；无法识别的 key 为 `None`
fn layout_from_row(r: &rusqlite::Row) -> rusqlite::Result<Option<WindowLayout>> {
    let key: String = r.get(0)?;
    let Some(key) = WindowKey::parse(&key) else {
        return Ok(None);
    };
    Ok(Some(WindowLayout {
        key,
        screen_config_id: r.get(1)?,
        x: r.get(2)?,
        y: r.get(3)?,
        width: r.get(4)?,
        height: r.get(5)?,
        is_fixed: r.get::<_, i64>(6)? != 0,
    }))
}

/// 日历是单例，总是"存在"；清单窗口要求清单存在（默认清单总存在）
fn window_exists(conn: &Connection, key: WindowKey) -> rusqlite::Result<bool> {
    match key {
        WindowKey::Calendar => Ok(true),
        WindowKey::List(id) => crate::lists::is_valid_target(conn, id),
    }
}

/// 没有记录时按列默认值补建（= [`WindowPrefs::new`]）
fn ensure_row(conn: &Connection, key: WindowKey) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO window_prefs (window_key, updated_at)
         VALUES (?1, datetime('now', 'localtime'))",
        [key.as_key()],
    )?;
    Ok(())
}

fn save_layout_row(conn: &Connection, layout: &WindowLayout) -> rusqlite::Result<bool> {
    if !window_exists(conn, layout.key)? {
        log::warn!(
            "[window-prefs] 忽略不存在的清单的窗口布局写入: {}",
            layout.key
        );
        return Ok(false);
    }
    ensure_row(conn, layout.key)?;
    conn.execute(
        "INSERT INTO window_layouts
             (window_key, screen_config_id, x, y, width, height, is_fixed, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now', 'localtime'))
         ON CONFLICT(window_key, screen_config_id) DO UPDATE SET
             x = excluded.x, y = excluded.y, width = excluded.width, height = excluded.height,
             is_fixed = excluded.is_fixed, updated_at = excluded.updated_at",
        params![
            layout.key.as_key(),
            layout.screen_config_id,
            layout.x,
            layout.y,
            layout.width,
            layout.height,
            layout.is_fixed as i32,
        ],
    )?;
    Ok(true)
}

fn delete_window_rows(conn: &Connection, key: WindowKey) -> rusqlite::Result<()> {
    let key = key.as_key();
    // 外键级联也会删布局；显式删一遍，外键关闭的连接上同样不留孤儿
    conn.execute("DELETE FROM window_layouts WHERE window_key = ?1", [&key])?;
    conn.execute("DELETE FROM window_prefs WHERE window_key = ?1", [&key])?;
    Ok(())
}

fn list_ids(conn: &Connection) -> rusqlite::Result<HashSet<i64>> {
    let mut stmt = conn.prepare_cached("SELECT id FROM todo_lists")?;
    let rows = stmt.query_map([], |r| r.get(0))?;
    rows.collect()
}

/// 筛选：只留现有清单、去重升序；空 → `None`（全部）
fn normalize_filter(ids: &[i64], existing: &HashSet<i64>) -> Option<Vec<i64>> {
    let mut kept: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|id| existing.contains(id))
        .collect();
    kept.sort_unstable();
    kept.dedup();
    (!kept.is_empty()).then_some(kept)
}

/// 存储形式：`NULL`（全部）或清单 id 的 JSON 数组；无法识别 / 空数组按"全部"
fn parse_filter(text: Option<&str>) -> Option<Vec<i64>> {
    let ids: Vec<i64> = serde_json::from_str(text?).ok()?;
    (!ids.is_empty()).then_some(ids)
}

fn filter_text(filter: Option<&[i64]>) -> Option<String> {
    filter.map(|ids| serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string()))
}

/// 日历筛选里去掉已不存在的清单；空了回到"全部"
fn prune_calendar_filter(conn: &Connection, existing: &HashSet<i64>) -> rusqlite::Result<()> {
    let stored: Option<Option<String>> = conn
        .query_row(
            "SELECT calendar_filter FROM window_prefs WHERE window_key = ?1",
            [CALENDAR_WINDOW_KEY],
            |r| r.get(0),
        )
        .optional()?;
    let Some(Some(text)) = stored else {
        return Ok(());
    };
    let next = parse_filter(Some(&text)).and_then(|ids| normalize_filter(&ids, existing));
    let next_text = filter_text(next.as_deref());
    if next_text.as_deref() != Some(text.as_str()) {
        conn.execute(
            "UPDATE window_prefs SET calendar_filter = ?1, updated_at = datetime('now', 'localtime')
             WHERE window_key = ?2",
            params![next_text, CALENDAR_WINDOW_KEY],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::sync_store::local_seq;
    use crate::lists;

    fn db() -> Database {
        Database::new_in_memory().expect("内存库")
    }

    fn layout(key: WindowKey, screen: &str, x: i32, fixed: bool) -> WindowLayout {
        WindowLayout {
            key,
            screen_config_id: screen.to_string(),
            x,
            y: 20,
            width: 380,
            height: 600,
            is_fixed: fixed,
        }
    }

    #[test]
    fn window_keys_round_trip_and_reject_non_canonical_forms() {
        assert_eq!(WindowKey::parse("calendar"), Some(WindowKey::Calendar));
        assert_eq!(WindowKey::parse("list:1"), Some(WindowKey::List(1)));
        assert_eq!(
            WindowKey::parse("list:1731000000000123"),
            Some(WindowKey::List(1_731_000_000_000_123))
        );
        for bad in [
            "",
            "list:",
            "list:0",
            "list:-3",
            "list:+3",
            "list:03",
            "list:x",
            "List:1",
            "fence-1",
            "calendar ",
        ] {
            assert_eq!(WindowKey::parse(bad), None, "{bad:?}");
        }
        for key in [WindowKey::Calendar, WindowKey::List(1), WindowKey::List(42)] {
            assert_eq!(WindowKey::parse(&key.as_key()), Some(key));
            assert_eq!(key.to_string(), key.as_key());
        }
        assert_eq!(WindowKey::List(7).list_id(), Some(7));
        assert_eq!(WindowKey::Calendar.list_id(), None);
        assert_eq!(ViewMode::parse("quadrant"), ViewMode::Quadrant);
        assert_eq!(ViewMode::parse("grid"), ViewMode::List);
    }

    /// 迁移种子：默认清单窗口与日历窗口都有记录（全新库：浅色、列表、日历隐藏）
    #[test]
    fn seeded_prefs_are_readable() {
        let db = db();
        let prefs = list_prefs(&db).unwrap();
        assert_eq!(
            prefs,
            vec![
                WindowPrefs {
                    hidden: true,
                    ..WindowPrefs::new(WindowKey::Calendar)
                },
                WindowPrefs::new(WindowKey::List(1)),
            ]
        );
    }

    #[test]
    fn update_prefs_creates_missing_records_and_changes_only_given_fields() {
        let db = db();
        let work = lists::create(&db, "工作").unwrap();
        let key = WindowKey::List(work.id);
        assert_eq!(
            get_prefs(&db, key).unwrap(),
            None,
            "同步来的新清单：本机没有窗口记录"
        );

        let created = update_prefs(&db, key, &UpdateWindowPrefs::default())
            .unwrap()
            .unwrap();
        assert_eq!(created, WindowPrefs::new(key));

        let updated = update_prefs(
            &db,
            key,
            &UpdateWindowPrefs {
                dark: Some(true),
                view_mode: Some(ViewMode::Quadrant),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            updated,
            WindowPrefs {
                dark: true,
                view_mode: ViewMode::Quadrant,
                ..WindowPrefs::new(key)
            }
        );
        let updated = update_prefs(
            &db,
            key,
            &UpdateWindowPrefs {
                hidden: Some(true),
                expand_all: Some(true),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(updated.hidden && updated.expand_all && updated.dark);
        assert_eq!(get_prefs(&db, key).unwrap(), Some(updated));

        // 每个窗口各自一份：默认清单窗口不受影响
        assert_eq!(
            get_prefs(&db, WindowKey::List(1)).unwrap(),
            Some(WindowPrefs::new(WindowKey::List(1)))
        );
    }

    #[test]
    fn writes_for_missing_lists_are_ignored() {
        let db = db();
        let ghost = WindowKey::List(1_731_000_000_000_999);
        let update = UpdateWindowPrefs {
            dark: Some(true),
            ..Default::default()
        };
        assert_eq!(update_prefs(&db, ghost, &update).unwrap(), None);
        assert!(!save_layout(&db, &layout(ghost, "1_1920x1080@100", 0, false)).unwrap());
        assert_eq!(get_prefs(&db, ghost).unwrap(), None, "不留孤儿记录");
        assert_eq!(
            save_layouts(
                &db,
                &[
                    layout(ghost, "1_1920x1080@100", 0, false),
                    layout(WindowKey::List(1), "1_1920x1080@100", 5, true),
                    layout(WindowKey::Calendar, "1_1920x1080@100", 400, true),
                ]
            )
            .unwrap(),
            2,
            "批量保存跳过不存在的清单，其余照常保存"
        );
        assert_eq!(layouts_for_screen(&db, "1_1920x1080@100").unwrap().len(), 2);
    }

    #[test]
    fn layouts_are_kept_per_window_and_screen_combination() {
        let db = db();
        let work = lists::create(&db, "工作").unwrap();
        let key = WindowKey::List(work.id);
        assert!(save_layout(&db, &layout(key, "1_1920x1080@100", 10, false)).unwrap());
        assert!(save_layout(&db, &layout(key, "2_3840x2160@150_1920x1080@100", 20, true)).unwrap());
        assert!(save_layout(
            &db,
            &layout(WindowKey::List(1), "1_1920x1080@100", 30, true)
        )
        .unwrap());
        // 再次保存同一组合：覆盖
        let moved = WindowLayout {
            x: 11,
            width: 500,
            is_fixed: true,
            ..layout(key, "1_1920x1080@100", 0, false)
        };
        assert!(save_layout(&db, &moved).unwrap());

        assert_eq!(
            get_layout(&db, key, "1_1920x1080@100").unwrap(),
            Some(moved.clone())
        );
        assert_eq!(get_layout(&db, key, "3_x").unwrap(), None);
        assert_eq!(
            layouts_for_screen(&db, "1_1920x1080@100").unwrap(),
            vec![
                layout(WindowKey::List(1), "1_1920x1080@100", 30, true),
                moved
            ]
        );
        assert_eq!(
            get_prefs(&db, key).unwrap(),
            Some(WindowPrefs::new(key)),
            "保存布局时按默认值补建属性记录"
        );

        delete_window(&db, key).unwrap();
        assert_eq!(get_prefs(&db, key).unwrap(), None);
        assert!(get_layout(&db, key, "2_3840x2160@150_1920x1080@100")
            .unwrap()
            .is_none());
        assert_eq!(layouts_for_screen(&db, "1_1920x1080@100").unwrap().len(), 1);
    }

    #[test]
    fn calendar_filter_keeps_existing_lists_and_falls_back_to_all() {
        let db = db();
        let work = lists::create(&db, "工作").unwrap();
        let life = lists::create(&db, "生活").unwrap();
        assert_eq!(set_calendar_filter(&db, None).unwrap(), None);

        let saved = set_calendar_filter(&db, Some(&[life.id, 999, work.id, life.id])).unwrap();
        let mut expected = vec![work.id, life.id];
        expected.sort_unstable();
        assert_eq!(saved, Some(expected.clone()), "去掉不存在的清单、去重排序");
        assert_eq!(
            get_prefs(&db, WindowKey::Calendar)
                .unwrap()
                .unwrap()
                .calendar_filter,
            Some(expected)
        );
        assert_eq!(
            set_calendar_filter(&db, Some(&[])).unwrap(),
            None,
            "空选 = 全部"
        );
        assert_eq!(
            set_calendar_filter(&db, Some(&[999])).unwrap(),
            None,
            "只选了不存在的清单 = 全部"
        );
        assert_eq!(
            set_calendar_filter(&db, Some(&[1])).unwrap(),
            Some(vec![1]),
            "默认清单可以单独选"
        );
    }

    #[test]
    fn window_prefs_are_device_local() {
        let db = db();
        let base = db.with_connection(local_seq).unwrap();
        update_prefs(
            &db,
            WindowKey::List(1),
            &UpdateWindowPrefs {
                dark: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        save_layout(
            &db,
            &layout(WindowKey::Calendar, "1_1920x1080@100", 0, true),
        )
        .unwrap();
        set_calendar_filter(&db, Some(&[1])).unwrap();
        delete_window(&db, WindowKey::Calendar).unwrap();
        assert_eq!(
            db.with_connection(local_seq).unwrap(),
            base,
            "窗口属性不计入同步的本地变更"
        );
    }

    #[test]
    fn unknown_keys_and_garbage_filters_are_tolerated_on_read() {
        let db = db();
        db.with_connection(|c| {
            c.execute_batch(
                "INSERT INTO window_prefs (window_key, updated_at) VALUES ('board:9', '');
                 INSERT INTO window_layouts (window_key, screen_config_id, x, y, width, height, updated_at)
                     VALUES ('board:9', '1_1920x1080@100', 0, 0, 1, 1, '');
                 UPDATE window_prefs SET calendar_filter = 'not json', view_mode = 'grid'
                     WHERE window_key = 'calendar';",
            )
        })
        .unwrap();
        let keys: Vec<WindowKey> = list_prefs(&db)
            .unwrap()
            .into_iter()
            .map(|p| p.key)
            .collect();
        assert_eq!(keys, vec![WindowKey::Calendar, WindowKey::List(1)]);
        assert!(layouts_for_screen(&db, "1_1920x1080@100")
            .unwrap()
            .is_empty());
        let calendar = get_prefs(&db, WindowKey::Calendar).unwrap().unwrap();
        assert_eq!(calendar.calendar_filter, None, "无法识别的筛选按全部");
        assert_eq!(calendar.view_mode, ViewMode::List);
    }
}

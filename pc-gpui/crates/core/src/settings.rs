//! 应用设置项与屏幕配置（原 `commands/settings_cmd.rs` + `commands/window.rs` 中只读写数据库的部分）。
//!
//! 每个设置函数只读 / 写自己那一个（或一组）键，全部经 `db::settings_kv`（值没变时不刷新 `updated_at`，
//! 不触发设置重新同步）。窗口侧的副作用（固定 / 嵌入切换、贴边隐藏状态、置顶）不在这里：调用方写完键后
//! 自己处理，见 [`crate::window_logic`]。
//!
//! 屏幕组合 ID（[`screen_config_id_of`]）与旧版前端 `appStore.generateScreenConfigId()`、收纳箱
//! `fence.rs::screen_config_id_of` 逐字一致，`screen_configs.config_id` / `fence_layouts.screen_config_id`
//! 里已有的记录可以直接匹配。

use crate::db::settings_kv::{
    self, get_bool_setting, get_setting_or, set_bool_setting, set_setting as kv_set_setting,
};
use crate::db::{
    AppSettings, Database, SaveScreenConfigRequest, ScreenConfig, DEFAULT_WINDOW_BG_ALPHA,
    DEFAULT_WINDOW_BG_COLOR,
};
use serde::Serialize;

// ============================================================================
// 白名单设置（原前端 get_setting / set_setting）
// ============================================================================

/// 可以按键名直接读写的设置（白名单 + 合法取值）。
///
/// 只开放没有副作用的界面偏好；WebDAV 密码、同步基准、窗口几何等由专用函数维护的键一律不开放。
const FRONTEND_SETTINGS: [(&str, &[&str]); 1] = [("view_mode", &["list", "quadrant"])];

fn frontend_setting_values(key: &str) -> Result<&'static [&'static str], String> {
    FRONTEND_SETTINGS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, values)| *values)
        .ok_or_else(|| format!("不允许直接读写设置项：{}", key))
}

/// 读取白名单内的单个设置项（目前只有 `view_mode`）；键不存在返回 `None`（`get_setting`）
pub fn get_setting(db: &Database, key: &str) -> Result<Option<String>, String> {
    frontend_setting_values(key)?;
    db.with_connection(|conn| settings_kv::get_setting(conn, key))
        .map_err(|e| e.to_string())
}

/// 写入白名单内的单个设置项（目前只有 `view_mode`）；取值不合法时报错（`set_setting`）
pub fn set_setting(db: &Database, key: &str, value: &str) -> Result<(), String> {
    if !frontend_setting_values(key)?.contains(&value) {
        return Err(format!("设置项 {} 的取值不合法：{}", key, value));
    }
    db.with_connection(|conn| kv_set_setting(conn, key, value).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 列表 / 四象限视图（`todoStore.loadViewMode`）：库里没有或取值不合法时为 `"list"`
pub fn view_mode(db: &Database) -> String {
    match get_setting(db, "view_mode") {
        Ok(Some(mode)) if mode == "list" || mode == "quadrant" => mode,
        _ => "list".to_string(),
    }
}

/// 保存视图模式（`todoStore.saveViewMode`），只接受 `list` / `quadrant`
pub fn set_view_mode(db: &Database, mode: &str) -> Result<(), String> {
    set_setting(db, "view_mode", mode)
}

// ============================================================================
// 字体 / 通知类型（原 settings_cmd.rs）
// ============================================================================

/// 系统字体族列表（DirectWrite 枚举，排序去重；非 Windows 为空）（`get_system_fonts`）
pub fn system_fonts() -> Result<Vec<String>, String> {
    #[cfg(target_os = "windows")]
    {
        use windows::Win32::Graphics::DirectWrite::*;

        unsafe {
            let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)
                .map_err(|e| format!("DWrite factory: {e}"))?;

            let mut collection = None;
            factory
                .GetSystemFontCollection(&mut collection, false)
                .map_err(|e| format!("Font collection: {e}"))?;
            let collection = collection.ok_or("No font collection")?;

            let count = collection.GetFontFamilyCount();
            let mut families = Vec::with_capacity(count as usize);

            for i in 0..count {
                let Ok(family) = collection.GetFontFamily(i) else {
                    continue;
                };
                let Ok(names) = family.GetFamilyNames() else {
                    continue;
                };
                let len = names.GetStringLength(0).unwrap_or(0);
                if len == 0 {
                    continue;
                }
                let mut buf = vec![0u16; (len + 1) as usize];
                if names.GetString(0, &mut buf).is_ok() {
                    if let Ok(name) = String::from_utf16(&buf[..len as usize]) {
                        families.push(name);
                    }
                }
            }

            families.sort_unstable();
            families.dedup();
            Ok(families)
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        Ok(vec![])
    }
}

/// 待办字体（空串 = 跟随默认）（`get_todo_font_family`）
pub fn font_family(db: &Database) -> Result<String, String> {
    db.with_connection(|conn| {
        Ok(settings_kv::get_setting(conn, "todo_font_family")?.unwrap_or_default())
    })
    .map_err(|e| e.to_string())
}

/// `set_todo_font_family`
pub fn set_font_family(db: &Database, font_family: &str) -> Result<(), String> {
    db.with_connection(|conn| kv_set_setting(conn, "todo_font_family", font_family).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 待办字号（默认 14）（`get_todo_font_size`）
pub fn font_size(db: &Database) -> Result<i32, String> {
    db.with_connection(|conn| {
        Ok(settings_kv::get_setting(conn, "todo_font_size")?
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(14))
    })
    .map_err(|e| e.to_string())
}

/// 字号夹到 12~20 后保存（`set_todo_font_size`）
pub fn set_font_size(db: &Database, font_size: i32) -> Result<(), String> {
    let size = font_size.clamp(12, 20);
    db.with_connection(|conn| kv_set_setting(conn, "todo_font_size", &size.to_string()).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 通知类型：`system`（系统通知，默认）/ `app`（应用内通知窗口）（`get_notification_type`）
pub fn notification_type(db: &Database) -> Result<String, String> {
    db.with_connection(|conn| Ok(get_setting_or(conn, "notification_type", "system")))
        .map_err(|e| e.to_string())
}

/// 非 `system` / `app` 一律存 `system`（`set_notification_type`）
pub fn set_notification_type(db: &Database, notification_type: &str) -> Result<(), String> {
    let valid_type = match notification_type {
        "system" | "app" => notification_type,
        _ => "system",
    };
    db.with_connection(|conn| kv_set_setting(conn, "notification_type", valid_type).map(|_| ()))
        .map_err(|e| e.to_string())
}

// ============================================================================
// 窗口相关设置（原 window.rs 中只写键的部分）
// ============================================================================

/// 窗口侧轮询要用的运行时偏好（原 `reload_runtime_prefs` 读取的两个键）。
///
/// 启动、手动导入、同步应用了远端设置（`SyncReport::settings_applied`）、[`save_window_settings`] 之后，
/// 调用方重新读取它并刷新自己的缓存（贴边隐藏状态机见 [`crate::window_logic::apply_runtime_prefs`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimePrefs {
    /// 贴边唤起时临时置顶（默认 true）
    pub top_on_wake: bool,
    /// 固定模式贴边自动隐藏（默认 true，只在经典固定模式下生效）
    pub auto_hide_enabled: bool,
}

/// 读取 [`RuntimePrefs`]；读库失败时按默认值（都为 true）
pub fn runtime_prefs(db: &Database) -> RuntimePrefs {
    let (top_on_wake, auto_hide_enabled) = db
        .with_connection(|conn| {
            Ok((
                get_bool_setting(conn, "top_on_wake", true),
                get_bool_setting(conn, "auto_hide_enabled", true),
            ))
        })
        .unwrap_or((true, true));
    RuntimePrefs {
        top_on_wake,
        auto_hide_enabled,
    }
}

/// 主窗口保存窗口状态（原 `save_settings`，旧前端 `appStore.saveWindowState`）：一个事务内写入
/// `is_fixed` / `fixed_embed_desktop` / `window_position` / `window_size` / `auto_hide_enabled` /
/// `top_on_wake` / `window_bg_color` / `window_bg_alpha` / `text_theme`，值没变的键不刷新 `updated_at`
/// （否则每次挪窗口都把参与同步的设置版本推到"现在"）。不写 `show_calendar` / `view_mode` /
/// `notification_type`。
///
/// 调用方必须先读出各键的真值再填 `settings`（否则把 `top_on_wake` / `fixed_embed_desktop` 刷成默认值），
/// 写完后刷新 [`RuntimePrefs`]。设置窗口里绝不能调用它（会把设置窗口的几何存成主窗口的）。
pub fn save_window_settings(db: &Database, settings: &AppSettings) -> Result<(), String> {
    db.with_transaction(|tx| -> rusqlite::Result<()> {
        set_bool_setting(tx, "is_fixed", settings.is_fixed)?;
        set_bool_setting(tx, "fixed_embed_desktop", settings.fixed_embed_desktop)?;
        if let Some(pos) = &settings.window_position {
            let pos_json = serde_json::to_string(pos).unwrap_or_default();
            kv_set_setting(tx, "window_position", &pos_json)?;
        }
        if let Some(size) = &settings.window_size {
            let size_json = serde_json::to_string(size).unwrap_or_default();
            kv_set_setting(tx, "window_size", &size_json)?;
        }
        set_bool_setting(tx, "auto_hide_enabled", settings.auto_hide_enabled)?;
        set_bool_setting(tx, "top_on_wake", settings.top_on_wake)?;
        kv_set_setting(tx, "window_bg_color", &settings.window_bg_color)?;
        kv_set_setting(tx, "window_bg_alpha", &settings.window_bg_alpha.to_string())?;
        kv_set_setting(tx, "text_theme", &settings.text_theme)?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 文本主题（默认 `"dark"`）。该键描述的是文字颜色：`"light"` 表示浅色文字，即「深色主题开启」
/// （`get_text_theme`）
pub fn text_theme(db: &Database) -> Result<String, String> {
    db.with_connection(|conn| Ok(get_setting_or(conn, "text_theme", "dark")))
        .map_err(|e| e.to_string())
}

/// 只写 `text_theme`（`set_text_theme`）
pub fn set_text_theme(db: &Database, theme: &str) -> Result<(), String> {
    db.with_connection(|conn| kv_set_setting(conn, "text_theme", theme).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 「固定模式时，嵌入桌面中」；缺键（旧库）按 false（`get_fixed_embed_desktop`）
pub fn fixed_embed_desktop(db: &Database) -> bool {
    db.with_connection(|conn| Ok(get_bool_setting(conn, "fixed_embed_desktop", false)))
        .unwrap_or(false)
}

/// 只写 `fixed_embed_desktop`（`set_fixed_embed_desktop` 的写键部分）。
///
/// 调用方随后：Windows 上已处于经典固定且 `enabled` → 切到嵌入桌面；处于嵌入桌面且 `!enabled` →
/// 切回经典固定；其它情况只记住偏好。非 Windows 只记住偏好。
pub fn set_fixed_embed_desktop(db: &Database, enabled: bool) -> Result<(), String> {
    db.with_connection(|conn| set_bool_setting(conn, "fixed_embed_desktop", enabled).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 贴边自动隐藏开关（默认 true）（`get_auto_hide_enabled`）
pub fn auto_hide_enabled(db: &Database) -> bool {
    db.with_connection(|conn| Ok(get_bool_setting(conn, "auto_hide_enabled", true)))
        .unwrap_or(true)
}

/// 只写 `auto_hide_enabled`（`set_auto_hide_enabled` 的写键部分；状态机见
/// [`crate::window_logic::AutoHideState::set_enabled`]）
pub fn set_auto_hide_enabled(db: &Database, enabled: bool) -> Result<(), String> {
    db.with_connection(|conn| set_bool_setting(conn, "auto_hide_enabled", enabled).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 贴边唤起时临时置顶（默认 true）（`get_top_on_wake`）
pub fn top_on_wake(db: &Database) -> bool {
    db.with_connection(|conn| Ok(get_bool_setting(conn, "top_on_wake", true)))
        .unwrap_or(true)
}

/// 只写 `top_on_wake`（`set_top_on_wake` 的写键部分；关闭时调用方立即撤销置顶）
pub fn set_top_on_wake(db: &Database, enabled: bool) -> Result<(), String> {
    db.with_connection(|conn| set_bool_setting(conn, "top_on_wake", enabled).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 窗口底色与背景透明度（仅深色主题下生效）
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowBackground {
    pub color: String,
    pub alpha: f64,
}

/// 读 `window_bg_color`（默认 `#000000`）/ `window_bg_alpha`（默认 0.45）（`get_window_background`）
pub fn window_background(db: &Database) -> Result<WindowBackground, String> {
    db.with_connection(|conn| {
        let color = get_setting_or(conn, "window_bg_color", DEFAULT_WINDOW_BG_COLOR);
        let alpha = get_setting_or(conn, "window_bg_alpha", "")
            .parse::<f64>()
            .unwrap_or(DEFAULT_WINDOW_BG_ALPHA);
        Ok(WindowBackground { color, alpha })
    })
    .map_err(|e| e.to_string())
}

/// 一个事务写两键，透明度夹到 0~1（`set_window_background`）
pub fn set_window_background(db: &Database, color: &str, alpha: f64) -> Result<(), String> {
    let alpha = alpha.clamp(0.0, 1.0);
    db.with_transaction(|tx| -> rusqlite::Result<()> {
        kv_set_setting(tx, "window_bg_color", color)?;
        kv_set_setting(tx, "window_bg_alpha", &alpha.to_string())?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

/// 是否显示日历（默认 false）（`get_show_calendar`）
pub fn show_calendar(db: &Database) -> Result<bool, String> {
    db.with_connection(|conn| Ok(get_bool_setting(conn, "show_calendar", false)))
        .map_err(|e| e.to_string())
}

/// `set_show_calendar`
pub fn set_show_calendar(db: &Database, show: bool) -> Result<(), String> {
    db.with_connection(|conn| set_bool_setting(conn, "show_calendar", show).map(|_| ()))
        .map_err(|e| e.to_string())
}

// ============================================================================
// 屏幕组合 ID 与屏幕配置（screen_configs）
// ============================================================================

/// 无显示器信息时的屏幕组合 ID
pub const UNKNOWN_SCREEN_CONFIG: &str = "unknown";

/// 屏幕组合 ID：`{显示器数量}_{宽}x{高}@{缩放%}_…`（主窗口 `screen_configs.config_id` 与收纳箱
/// `fence_layouts.screen_config_id` 同一格式，与旧版前端 `appStore.generateScreenConfigId()` 逐字一致）。
///
/// - 输入为显示器的枚举顺序：（物理像素宽, 高, 缩放比例）。旧版取的是 tao `available_monitors()`，即
///   `EnumDisplayMonitors` 的顺序
/// - 缩放% = `round(scale × 100)`：缩放比例为正数，Rust `f64::round`（逢半远离零）与 JS `Math.round`
///   （逢半向 +∞）结果相同
/// - 按像素面积降序；稳定排序，同面积保持枚举顺序（与 JS `Array.prototype.sort` 一致）
/// - 没有显示器时为 `unknown`
pub fn screen_config_id_of(monitors: &[(u32, u32, f64)]) -> String {
    if monitors.is_empty() {
        return UNKNOWN_SCREEN_CONFIG.to_string();
    }
    let mut infos: Vec<(u32, u32, i64)> = monitors
        .iter()
        .map(|&(w, h, scale)| (w, h, (scale * 100.0).round() as i64))
        .collect();
    // sort_by_key 是稳定排序
    infos.sort_by_key(|&(w, h, _)| std::cmp::Reverse(u64::from(w) * u64::from(h)));
    let parts: Vec<String> = infos
        .iter()
        .map(|(w, h, scale)| format!("{w}x{h}@{scale}"))
        .collect();
    format!("{}_{}", monitors.len(), parts.join("_"))
}

/// 当前屏幕组合 ID：[`screen_config_id_of`]，debug 构建下可被 `MINI_TODO_SCREEN_ID_FILE` 覆盖
/// （文件内容去首尾空白后非空才生效；e2e 用它模拟显示器组合变化，release 构建忽略）
pub fn screen_config_id(monitors: &[(u32, u32, f64)]) -> String {
    #[cfg(debug_assertions)]
    if let Some(id) = dev_screen_id_override() {
        return id;
    }
    screen_config_id_of(monitors)
}

/// 开发专用（**仅 debug 构建**）的屏幕组合 ID 覆盖
#[cfg(debug_assertions)]
fn dev_screen_id_override() -> Option<String> {
    const DEV_SCREEN_ID_FILE_ENV: &str = "MINI_TODO_SCREEN_ID_FILE";
    let path = std::env::var_os(DEV_SCREEN_ID_FILE_ENV).filter(|v| !v.is_empty())?;
    let text = std::fs::read_to_string(path).ok()?;
    let id = text.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// 屏幕配置的显示名（旧版前端 `appStore.generateScreenConfigDisplayName`）：
/// `unknown` → 「未知配置」、`legacy` → 「旧版配置」，否则 `"{n}屏: {W}x{H} ({scale}%) + …"`
pub fn screen_config_display_name(config_id: &str) -> String {
    match config_id {
        "unknown" => return "未知配置".to_string(),
        "legacy" => return "旧版配置".to_string(),
        _ => {}
    }
    let mut parts = config_id.split('_');
    let count = parts.next().unwrap_or("");
    let monitors: Vec<String> = parts
        .map(|p| {
            // 与 JS `const [res, scale] = p.split('@')` 一致：缺 @ 时 scale 为 undefined
            let mut it = p.split('@');
            let res = it.next().unwrap_or("");
            let scale = it.next().unwrap_or("undefined");
            format!("{res} ({scale}%)")
        })
        .collect();
    format!("{count}屏: {}", monitors.join(" + "))
}

/// screen_configs 查询列，与 `screen_config_from_row` 的下标一一对应。
///
/// 三处 SELECT 共用一份列表：新增列只改这里和映射函数，不会出现某条查询漏读新列的情况。
const SCREEN_CONFIG_COLUMNS: &str = "id, config_id, display_name, window_x, window_y, \
     window_width, window_height, is_fixed, created_at, updated_at";

fn screen_config_from_row(row: &rusqlite::Row) -> rusqlite::Result<ScreenConfig> {
    Ok(ScreenConfig {
        id: row.get(0)?,
        config_id: row.get(1)?,
        display_name: row.get(2)?,
        window_x: row.get(3)?,
        window_y: row.get(4)?,
        window_width: row.get(5)?,
        window_height: row.get(6)?,
        is_fixed: row.get::<_, i32>(7)? != 0,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

/// 根据屏幕组合 ID 读取保存的主窗口配置（`get_screen_config`）
pub fn get_screen_config(db: &Database, config_id: &str) -> Result<Option<ScreenConfig>, String> {
    db.with_connection(|conn| {
        let result = conn.query_row(
            &format!(
                "SELECT {} FROM screen_configs WHERE config_id = ?",
                SCREEN_CONFIG_COLUMNS
            ),
            [config_id],
            screen_config_from_row,
        );

        match result {
            Ok(config) => Ok(Some(config)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    })
    .map_err(|e| e.to_string())
}

/// 按 `config_id` upsert（`display_name` 为 `None` 时保留旧值），返回保存后的行（`save_screen_config`）
pub fn save_screen_config(
    db: &Database,
    config: &SaveScreenConfigRequest,
) -> Result<ScreenConfig, String> {
    db.with_connection(|conn| {
        conn.execute(
            "INSERT INTO screen_configs
             (config_id, display_name, window_x, window_y, window_width, window_height, is_fixed, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now', 'localtime'))
             ON CONFLICT(config_id) DO UPDATE SET
                display_name = COALESCE(?2, display_name),
                window_x = ?3,
                window_y = ?4,
                window_width = ?5,
                window_height = ?6,
                is_fixed = ?7,
                updated_at = datetime('now', 'localtime')",
            (
                &config.config_id,
                &config.display_name,
                config.window_x,
                config.window_y,
                config.window_width,
                config.window_height,
                if config.is_fixed { 1 } else { 0 },
            ),
        )?;

        conn.query_row(
            &format!(
                "SELECT {} FROM screen_configs WHERE config_id = ?",
                SCREEN_CONFIG_COLUMNS
            ),
            [&config.config_id],
            screen_config_from_row,
        )
    })
    .map_err(|e| e.to_string())
}

/// 全部屏幕配置，最近更新的在前（`list_screen_configs`）
pub fn list_screen_configs(db: &Database) -> Result<Vec<ScreenConfig>, String> {
    db.with_connection(|conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {} FROM screen_configs ORDER BY updated_at DESC",
            SCREEN_CONFIG_COLUMNS
        ))?;

        let configs = stmt.query_map([], screen_config_from_row)?;

        configs.collect::<Result<Vec<_>, _>>()
    })
    .map_err(|e| e.to_string())
}

/// 删除屏幕配置，连同桌面收纳箱在该屏幕组合下的位置记录（`fence_layouts`，PRD R5）
fn delete_screen_config_rows(conn: &rusqlite::Connection, config_id: &str) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM screen_configs WHERE config_id = ?",
        [config_id],
    )?;
    conn.execute(
        "DELETE FROM fence_layouts WHERE screen_config_id = ?",
        [config_id],
    )?;
    Ok(())
}

/// 删除屏幕配置（连带该组合的收纳箱位置记录）（`delete_screen_config`）
pub fn delete_screen_config(db: &Database, config_id: &str) -> Result<(), String> {
    db.with_transaction(|tx| delete_screen_config_rows(tx, config_id))
        .map_err(|e: rusqlite::Error| e.to_string())
}

/// 改屏幕配置的显示名（`update_screen_config_name`，旧前端未调用）
pub fn update_screen_config_name(
    db: &Database,
    config_id: &str,
    display_name: &str,
) -> Result<(), String> {
    db.with_connection(|conn| {
        conn.execute(
            "UPDATE screen_configs SET display_name = ?, updated_at = datetime('now', 'localtime') WHERE config_id = ?",
            [display_name, config_id],
        )?;
        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{WindowPosition, WindowSize};

    // ---- 原 settings_cmd.rs ----

    #[test]
    fn only_whitelisted_settings_are_writable_from_the_frontend() {
        let db = Database::new_in_memory().unwrap();
        set_setting(&db, "view_mode", "quadrant").unwrap();
        assert_eq!(
            db.with_connection(|c| settings_kv::get_setting(c, "view_mode"))
                .unwrap()
                .as_deref(),
            Some("quadrant")
        );
        assert!(set_setting(&db, "view_mode", "grid").is_err());
        for key in ["webdav_password", "webdav_remote_etag", "is_fixed", ""] {
            assert!(set_setting(&db, key, "list").is_err(), "{key}");
            assert!(frontend_setting_values(key).is_err(), "{key}");
            assert!(get_setting(&db, key).is_err(), "{key}");
        }
        assert_eq!(view_mode(&db), "quadrant");
    }

    // ---- 原 window.rs ----

    #[test]
    fn deleting_a_screen_config_also_drops_fence_layouts_of_that_combination() {
        let db = Database::new_in_memory().unwrap();
        db.with_connection(|conn| {
            conn.execute_batch(
                "INSERT INTO screen_configs (config_id, display_name, window_x, window_y, window_width, window_height, is_fixed, updated_at)
                     VALUES ('1_1920x1080@100', 'a', 0, 0, 380, 600, 0, ''), ('2_3840x2160@150_2560x1600@125', 'b', 0, 0, 380, 600, 0, '');
                 INSERT INTO fences (id, title, folder_name, x, y, width, height, created_at, updated_at)
                     VALUES (1, '收纳箱', '收纳箱', 0, 0, 368, 334, '', '');
                 INSERT INTO fence_layouts (fence_id, screen_config_id, x, y, width, height, updated_at)
                     VALUES (1, '1_1920x1080@100', 0, 0, 368, 334, ''), (1, '2_3840x2160@150_2560x1600@125', 9, 9, 552, 504, '');",
            )?;
            delete_screen_config_rows(conn, "1_1920x1080@100")?;
            let configs: i64 =
                conn.query_row("SELECT COUNT(*) FROM screen_configs", [], |r| r.get(0))?;
            let layouts: Vec<String> = conn
                .prepare("SELECT screen_config_id FROM fence_layouts")?
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            assert_eq!(configs, 1);
            assert_eq!(layouts, vec!["2_3840x2160@150_2560x1600@125".to_string()]);
            Ok(())
        })
        .unwrap();
    }

    // ---- settings 写入只在值变化时刷新时间 ----

    #[test]
    fn save_settings_only_touches_changed_keys() {
        let db = Database::new_in_memory().unwrap();
        let mut settings = db
            .with_connection(|c| Ok(crate::data::read_app_settings(c)))
            .unwrap();
        settings.window_position = Some(WindowPosition { x: 10, y: 20 });
        settings.window_size = Some(WindowSize {
            width: 380,
            height: 600,
        });
        save_window_settings(&db, &settings).unwrap();
        db.with_connection(|c| {
            c.execute("UPDATE settings SET updated_at = '2026-01-01 00:00:00'", [])
        })
        .unwrap();
        let version = |db: &Database| db.with_connection(crate::data::settings_version).unwrap();
        assert_eq!(version(&db).as_deref(), Some("2026-01-01 00:00:00"));

        // 只挪了窗口：参与同步的设置版本不变（以前每次挪窗口都会触发设置重传）
        settings.window_position = Some(WindowPosition { x: 11, y: 21 });
        save_window_settings(&db, &settings).unwrap();
        assert_eq!(version(&db).as_deref(), Some("2026-01-01 00:00:00"));
        let position: String = db
            .with_connection(|c| Ok(get_setting_or(c, "window_position", "")))
            .unwrap();
        assert!(position.contains("11"), "{position}");

        // 真的改了设置：版本前进
        settings.top_on_wake = !settings.top_on_wake;
        save_window_settings(&db, &settings).unwrap();
        assert!(version(&db).unwrap().as_str() > "2026-01-01 00:00:00");
    }

    // ---- 新增：只写键的设置函数 ----

    #[test]
    fn single_key_settings_round_trip_with_defaults_and_validation() {
        let db = Database::new_in_memory().unwrap();
        assert_eq!(
            runtime_prefs(&db),
            RuntimePrefs {
                top_on_wake: true,
                auto_hide_enabled: true
            }
        );
        set_top_on_wake(&db, false).unwrap();
        set_auto_hide_enabled(&db, false).unwrap();
        assert_eq!(
            runtime_prefs(&db),
            RuntimePrefs {
                top_on_wake: false,
                auto_hide_enabled: false
            }
        );
        assert!(!top_on_wake(&db) && !auto_hide_enabled(&db));

        assert!(!fixed_embed_desktop(&db));
        set_fixed_embed_desktop(&db, true).unwrap();
        assert!(fixed_embed_desktop(&db));

        assert_eq!(text_theme(&db).unwrap(), "dark");
        set_text_theme(&db, "light").unwrap();
        assert_eq!(text_theme(&db).unwrap(), "light");

        set_window_background(&db, "#123456", 1.7).unwrap();
        assert_eq!(
            window_background(&db).unwrap(),
            WindowBackground {
                color: "#123456".to_string(),
                alpha: 1.0
            }
        );

        assert!(!show_calendar(&db).unwrap());
        set_show_calendar(&db, true).unwrap();
        assert!(show_calendar(&db).unwrap());

        assert_eq!(font_size(&db).unwrap(), 14);
        set_font_size(&db, 99).unwrap();
        assert_eq!(font_size(&db).unwrap(), 20, "夹到 12~20");
        assert_eq!(font_family(&db).unwrap(), "");
        set_font_family(&db, "Microsoft YaHei").unwrap();
        assert_eq!(font_family(&db).unwrap(), "Microsoft YaHei");

        assert_eq!(notification_type(&db).unwrap(), "system");
        set_notification_type(&db, "app").unwrap();
        assert_eq!(notification_type(&db).unwrap(), "app");
        set_notification_type(&db, "bogus").unwrap();
        assert_eq!(notification_type(&db).unwrap(), "system");

        assert_eq!(view_mode(&db), "list");
        assert!(set_view_mode(&db, "grid").is_err());
    }

    #[test]
    fn screen_configs_upsert_keep_display_name_and_list_newest_first() {
        let db = Database::new_in_memory().unwrap();
        let mut req = SaveScreenConfigRequest {
            config_id: "1_1920x1080@100".to_string(),
            display_name: Some("1屏: 1920x1080 (100%)".to_string()),
            window_x: 10,
            window_y: 20,
            window_width: 380,
            window_height: 600,
            is_fixed: false,
        };
        save_screen_config(&db, &req).unwrap();
        req.display_name = None;
        req.window_x = 30;
        req.is_fixed = true;
        let saved = save_screen_config(&db, &req).unwrap();
        assert_eq!(saved.display_name.as_deref(), Some("1屏: 1920x1080 (100%)"));
        assert_eq!((saved.window_x, saved.is_fixed), (30, true));
        let loaded = get_screen_config(&db, "1_1920x1080@100").unwrap().unwrap();
        assert_eq!((loaded.id, loaded.window_x), (saved.id, 30));
        assert!(get_screen_config(&db, "missing").unwrap().is_none());

        update_screen_config_name(&db, "1_1920x1080@100", "办公室").unwrap();
        let listed = list_screen_configs(&db).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].display_name.as_deref(), Some("办公室"));

        delete_screen_config(&db, "1_1920x1080@100").unwrap();
        assert!(list_screen_configs(&db).unwrap().is_empty());
    }

    // ---- 屏幕组合 ID（原 fence.rs）与显示名（原 appStore.ts）----

    /// 与前端 `appStore.generateScreenConfigId()` 逐字一致
    #[test]
    fn screen_config_id_matches_the_frontend_format() {
        assert_eq!(screen_config_id_of(&[]), "unknown");
        assert_eq!(screen_config_id_of(&[(1920, 1080, 1.0)]), "1_1920x1080@100");
        // 双屏：按面积降序，与枚举顺序无关
        assert_eq!(
            screen_config_id_of(&[(2560, 1600, 1.25), (3840, 2160, 1.5)]),
            "2_3840x2160@150_2560x1600@125"
        );
        assert_eq!(
            screen_config_id_of(&[(3840, 2160, 1.5), (2560, 1600, 1.25)]),
            "2_3840x2160@150_2560x1600@125"
        );
        // 同面积：稳定排序，保持枚举顺序（竖屏与横屏）
        assert_eq!(
            screen_config_id_of(&[(1920, 1080, 1.0), (1080, 1920, 1.5)]),
            "2_1920x1080@100_1080x1920@150"
        );
        assert_eq!(
            screen_config_id_of(&[(1080, 1920, 1.5), (1920, 1080, 1.0)]),
            "2_1080x1920@150_1920x1080@100"
        );
        // 常见缩放：1.1 * 100 = 110.00000000000001，JS Math.round 与 Rust round 都得 110
        assert_eq!(
            screen_config_id_of(&[(2560, 1440, 1.1), (1920, 1080, 1.75), (1366, 768, 1.0)]),
            "3_2560x1440@110_1920x1080@175_1366x768@100"
        );
        assert_eq!(
            screen_config_id_of(&[(3000, 2000, 2.25)]),
            "1_3000x2000@225"
        );
    }

    #[test]
    fn display_names_follow_the_frontend_format() {
        assert_eq!(screen_config_display_name("unknown"), "未知配置");
        assert_eq!(screen_config_display_name("legacy"), "旧版配置");
        assert_eq!(
            screen_config_display_name("2_3840x2160@150_1920x1080@100"),
            "2屏: 3840x2160 (150%) + 1920x1080 (100%)"
        );
        assert_eq!(
            screen_config_display_name("1_1920x1080"),
            "1屏: 1920x1080 (undefined%)"
        );
    }
}

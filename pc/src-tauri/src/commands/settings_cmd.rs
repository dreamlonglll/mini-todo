use crate::db::settings_kv;
use crate::db::Database;
use tauri::State;

/// 前端可以按键名直接读写的设置（白名单 + 合法取值）。
///
/// 只开放没有副作用的界面偏好；WebDAV 密码、同步基准、窗口几何等由专用命令维护的键一律不开放
/// （WebView 里的脚本不应能任意改写 settings 表）。
const FRONTEND_SETTINGS: [(&str, &[&str]); 1] = [("view_mode", &["list", "quadrant"])];

fn frontend_setting_values(key: &str) -> Result<&'static [&'static str], String> {
    FRONTEND_SETTINGS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, values)| *values)
        .ok_or_else(|| format!("不允许直接读写设置项：{}", key))
}

/// 读取白名单内的单个设置项（目前只有 `view_mode`，供 `todoStore.loadViewMode`）；键不存在返回 null。
///
/// 前端一直在调用 `get_setting` / `set_setting`，但后端以前没有这两个命令，视图模式从未保存成功。
#[tauri::command]
pub fn get_setting(db: State<Database>, key: String) -> Result<Option<String>, String> {
    frontend_setting_values(&key)?;
    db.with_connection(|conn| settings_kv::get_setting(conn, &key))
        .map_err(|e| e.to_string())
}

/// 写入白名单内的单个设置项（目前只有 `view_mode`，供 `todoStore.saveViewMode`）；取值不合法时报错。
/// 值没变时不刷新 `updated_at`（不触发设置重新同步）。
#[tauri::command]
pub fn set_setting(db: State<Database>, key: String, value: String) -> Result<(), String> {
    write_frontend_setting(&db, &key, &value)
}

fn write_frontend_setting(db: &Database, key: &str, value: &str) -> Result<(), String> {
    if !frontend_setting_values(key)?.contains(&value) {
        return Err(format!("设置项 {} 的取值不合法：{}", key, value));
    }
    db.with_connection(|conn| settings_kv::set_setting(conn, key, value).map(|_| ()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_system_fonts() -> Result<Vec<String>, String> {
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

#[tauri::command]
pub fn get_todo_font_family(db: State<Database>) -> Result<String, String> {
    db.with_connection(|conn| {
        Ok(settings_kv::get_setting(conn, "todo_font_family")?.unwrap_or_default())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_todo_font_family(db: State<Database>, font_family: String) -> Result<(), String> {
    db.with_connection(|conn| {
        settings_kv::set_setting(conn, "todo_font_family", &font_family).map(|_| ())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_todo_font_size(db: State<Database>) -> Result<i32, String> {
    db.with_connection(|conn| {
        Ok(settings_kv::get_setting(conn, "todo_font_size")?
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(14))
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_todo_font_size(db: State<Database>, font_size: i32) -> Result<(), String> {
    let size = font_size.clamp(12, 20);
    db.with_connection(|conn| {
        settings_kv::set_setting(conn, "todo_font_size", &size.to_string()).map(|_| ())
    })
    .map_err(|e| e.to_string())
}

/// 获取通知类型设置
#[tauri::command]
pub fn get_notification_type(db: State<Database>) -> Result<String, String> {
    db.with_connection(|conn| {
        Ok(settings_kv::get_setting_or(
            conn,
            "notification_type",
            "system",
        ))
    })
    .map_err(|e| e.to_string())
}

/// 设置通知类型
#[tauri::command]
pub fn set_notification_type(db: State<Database>, notification_type: String) -> Result<(), String> {
    // 验证通知类型
    let valid_type = match notification_type.as_str() {
        "system" | "app" => notification_type,
        _ => "system".to_string(),
    };

    db.with_connection(|conn| {
        settings_kv::set_setting(conn, "notification_type", &valid_type).map(|_| ())
    })
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_whitelisted_settings_are_writable_from_the_frontend() {
        let db = Database::new_in_memory().unwrap();
        write_frontend_setting(&db, "view_mode", "quadrant").unwrap();
        assert_eq!(
            db.with_connection(|c| settings_kv::get_setting(c, "view_mode"))
                .unwrap()
                .as_deref(),
            Some("quadrant")
        );
        assert!(write_frontend_setting(&db, "view_mode", "grid").is_err());
        for key in ["webdav_password", "webdav_remote_etag", "is_fixed", ""] {
            assert!(write_frontend_setting(&db, key, "list").is_err(), "{key}");
            assert!(frontend_setting_values(key).is_err(), "{key}");
        }
    }
}

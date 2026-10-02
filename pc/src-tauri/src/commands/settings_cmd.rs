use crate::db::settings_kv::{get_setting, get_setting_or, set_setting};
use crate::db::Database;
use tauri::State;

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
    db.with_connection(|conn| Ok(get_setting(conn, "todo_font_family")?.unwrap_or_default()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_todo_font_family(db: State<Database>, font_family: String) -> Result<(), String> {
    db.with_connection(|conn| set_setting(conn, "todo_font_family", &font_family).map(|_| ()))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_todo_font_size(db: State<Database>) -> Result<i32, String> {
    db.with_connection(|conn| {
        Ok(get_setting(conn, "todo_font_size")?
            .and_then(|v| v.parse::<i32>().ok())
            .unwrap_or(14))
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_todo_font_size(db: State<Database>, font_size: i32) -> Result<(), String> {
    let size = font_size.clamp(12, 20);
    db.with_connection(|conn| set_setting(conn, "todo_font_size", &size.to_string()).map(|_| ()))
        .map_err(|e| e.to_string())
}

/// 获取通知类型设置
#[tauri::command]
pub fn get_notification_type(db: State<Database>) -> Result<String, String> {
    db.with_connection(|conn| Ok(get_setting_or(conn, "notification_type", "system")))
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

    db.with_connection(|conn| set_setting(conn, "notification_type", &valid_type).map(|_| ()))
        .map_err(|e| e.to_string())
}

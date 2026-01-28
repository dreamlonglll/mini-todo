use tauri::{State, Window};
use crate::db::{Database, AppSettings, WindowPosition, WindowSize};

#[cfg(target_os = "windows")]
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongW, SetWindowLongW, GWL_EXSTYLE, WS_EX_TOOLWINDOW, WS_EX_APPWINDOW,
};
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::HWND;

#[tauri::command]
pub fn get_settings(db: State<Database>) -> Result<AppSettings, String> {
    db.with_connection(|conn| {
        let is_fixed: bool = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'is_fixed'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(val == "true")
                },
            )
            .unwrap_or(false);

        let window_position: Option<WindowPosition> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_position'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(serde_json::from_str(&val).ok())
                },
            )
            .unwrap_or(None);

        let window_size: Option<WindowSize> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'window_size'",
                [],
                |row| {
                    let val: String = row.get(0)?;
                    Ok(serde_json::from_str(&val).ok())
                },
            )
            .unwrap_or(None);

        Ok(AppSettings {
            is_fixed,
            window_position,
            window_size,
        })
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn save_settings(db: State<Database>, settings: AppSettings) -> Result<(), String> {
    db.with_connection(|conn| {
        // 保存 is_fixed
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('is_fixed', ?, datetime('now', 'localtime'))",
            [if settings.is_fixed { "true" } else { "false" }],
        )?;

        // 保存窗口位置
        if let Some(pos) = &settings.window_position {
            let pos_json = serde_json::to_string(pos).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_position', ?, datetime('now', 'localtime'))",
                [&pos_json],
            )?;
        }

        // 保存窗口尺寸
        if let Some(size) = &settings.window_size {
            let size_json = serde_json::to_string(size).unwrap_or_default();
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('window_size', ?, datetime('now', 'localtime'))",
                [&size_json],
            )?;
        }

        Ok(())
    })
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_window_fixed_mode(window: Window, fixed: bool) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use raw_window_handle::HasWindowHandle;
        
        if let Ok(handle) = window.window_handle() {
            if let raw_window_handle::RawWindowHandle::Win32(win32_handle) = handle.as_raw() {
                let hwnd = HWND(win32_handle.hwnd.get() as *mut _);
                
                unsafe {
                    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
                    
                    if fixed {
                        // 设置为工具窗口样式，不显示在任务栏，忽略 Win+D
                        let new_style = (ex_style as u32 | WS_EX_TOOLWINDOW.0) & !WS_EX_APPWINDOW.0;
                        SetWindowLongW(hwnd, GWL_EXSTYLE, new_style as i32);
                    } else {
                        // 恢复为普通窗口样式
                        let new_style = (ex_style as u32 & !WS_EX_TOOLWINDOW.0) | WS_EX_APPWINDOW.0;
                        SetWindowLongW(hwnd, GWL_EXSTYLE, new_style as i32);
                    }
                }
            }
        }
    }
    
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (window, fixed);
    }

    Ok(())
}

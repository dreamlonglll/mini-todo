use tauri::{State, Window, WebviewWindow};
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

        let text_theme: String = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'text_theme'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "dark".to_string());

        Ok(AppSettings {
            is_fixed,
            window_position,
            window_size,
            text_theme,
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

        // 保存文本主题
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES ('text_theme', ?, datetime('now', 'localtime'))",
            [&settings.text_theme],
        )?;

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

/// 重置窗口位置和大小（用于 Tauri 命令）
#[tauri::command]
pub fn reset_window(window: Window) -> Result<(), String> {
    reset_window_impl(&window)
}

/// 重置 WebviewWindow 位置和大小（用于托盘菜单）
pub fn reset_webview_window(window: WebviewWindow) -> Result<(), String> {
    reset_window_impl(&window)
}

/// 内部重置窗口实现
fn reset_window_impl<T: tauri::Runtime>(window: &impl WindowExt<T>) -> Result<(), String> {
    // 重置到屏幕左上角（10%边距），默认大小 380x600
    let default_width = 380.0;
    let default_height = 600.0;
    
    // 获取主显示器信息并计算 10% 边距位置
    if let Ok(Some(monitor)) = window.primary_monitor() {
        let scale = monitor.scale_factor();
        let size = monitor.size();
        let position = monitor.position();
        
        // 计算 10% 边距
        let margin_x = (size.width as f64 * 0.1 / scale) as i32;
        let margin_y = (size.height as f64 * 0.1 / scale) as i32;
        
        let x = position.x + margin_x;
        let y = position.y + margin_y;
        
        // 设置位置
        let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition { x, y }));
        
        // 设置大小
        let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize { 
            width: default_width, 
            height: default_height 
        }));
        
        // 确保可调整大小
        let _ = window.set_resizable(true);
    }
    
    Ok(())
}

/// 窗口扩展 trait
trait WindowExt<R: tauri::Runtime> {
    fn primary_monitor(&self) -> tauri::Result<Option<tauri::Monitor>>;
    fn set_position(&self, position: tauri::Position) -> tauri::Result<()>;
    fn set_size(&self, size: tauri::Size) -> tauri::Result<()>;
    fn set_resizable(&self, resizable: bool) -> tauri::Result<()>;
}

impl<R: tauri::Runtime> WindowExt<R> for Window<R> {
    fn primary_monitor(&self) -> tauri::Result<Option<tauri::Monitor>> {
        Window::primary_monitor(self)
    }
    fn set_position(&self, position: tauri::Position) -> tauri::Result<()> {
        Window::set_position(self, position)
    }
    fn set_size(&self, size: tauri::Size) -> tauri::Result<()> {
        Window::set_size(self, size)
    }
    fn set_resizable(&self, resizable: bool) -> tauri::Result<()> {
        Window::set_resizable(self, resizable)
    }
}

impl<R: tauri::Runtime> WindowExt<R> for WebviewWindow<R> {
    fn primary_monitor(&self) -> tauri::Result<Option<tauri::Monitor>> {
        WebviewWindow::primary_monitor(self)
    }
    fn set_position(&self, position: tauri::Position) -> tauri::Result<()> {
        WebviewWindow::set_position(self, position)
    }
    fn set_size(&self, size: tauri::Size) -> tauri::Result<()> {
        WebviewWindow::set_size(self, size)
    }
    fn set_resizable(&self, resizable: bool) -> tauri::Result<()> {
        WebviewWindow::set_resizable(self, resizable)
    }
}

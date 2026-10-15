//! 单实例（Windows）。
//!
//! 与旧版 `tauri-plugin-single-instance` 2.x 使用**同一组名字与消息格式**，新旧两版互相识别，
//! 不会同时打开同一个数据库：
//! - 命名互斥量 `com.tauri-app.mini-todo-sim`：已存在说明有实例在运行；
//! - 接收窗口：类名 `com.tauri-app.mini-todo-sic`、标题 `com.tauri-app.mini-todo-siw` 的隐藏顶层窗口
//!   （旧版用 `FindWindowW` 找它，所以不能是 message-only 窗口），收 `WM_COPYDATA`
//!   （`dwData = 1542`，内容 `"<cwd>|<arg0>|<arg1>…\0"`）；
//! - 第二个实例把参数发过去后退出；参数含 `--autostart`（开机自启拉起）时静默，不唤起窗口。
//!
//! 开发专用数据隔离（`MINI_TODO_DATA_DIR`，仅 debug）时不启用（要与正在运行的安装版并存）。
//! 非 Windows 不做单实例（Linux 只用于 CI 与截图）。

use gpui_kit::App;

use super::autostart::AUTOSTART_ARG;

/// 旧版 Tauri identifier（互斥量 / 窗口名的前缀）
pub const IDENTIFIER: &str = "com.tauri-app.mini-todo";

/// 首个实例持有的互斥量（进程结束时由系统释放；持有到进程退出即可）
pub struct InstanceGuard {
    #[cfg(windows)]
    _mutex: Option<windows::Win32::Foundation::HANDLE>,
}

/// 进程入口第一步（打开数据库之前）。返回 `None` 表示已有实例在运行（参数已转交），调用方应直接退出。
pub fn acquire() -> Option<InstanceGuard> {
    if mini_todo_core::db::paths::dev_data_dir_override().is_some() {
        return Some(InstanceGuard {
            #[cfg(windows)]
            _mutex: None,
        });
    }
    imp::acquire()
}

/// 在 GPUI 主线程（`Application::run` 回调里）建好接收窗口；收到第二个实例的唤起请求时调用 `on_activate`。
/// 开发隔离 / 非 Windows 时什么也不做。
pub fn listen(cx: &mut App, on_activate: impl Fn(&mut App) + 'static) {
    if mini_todo_core::db::paths::dev_data_dir_override().is_some() {
        return;
    }
    imp::listen(cx, on_activate);
}

/// 第二个实例转交的参数是否需要唤起窗口（开机自启拉起的不打扰用户）
pub fn should_activate(args: &[String]) -> bool {
    !args.iter().any(|a| a == AUTOSTART_ARG)
}

/// 解析 `WM_COPYDATA` 内容 `"<cwd>|<arg0>|…"`（去掉结尾的 `\0`）→ `(cwd, args)`
pub fn parse_payload(data: &str) -> (String, Vec<String>) {
    let data = data.trim_end_matches('\0');
    let mut parts = data.split('|');
    let cwd = parts.next().unwrap_or_default().to_string();
    (cwd, parts.map(str::to_string).collect())
}

#[cfg(windows)]
mod imp {
    use std::sync::OnceLock;
    use std::time::Duration;

    use gpui_kit::App;
    use windows::Win32::Foundation::{
        ERROR_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM,
    };
    use windows::Win32::System::DataExchange::COPYDATASTRUCT;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, FindWindowW, RegisterClassExW, SendMessageW, WM_COPYDATA,
        WNDCLASSEXW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
    };
    use windows::core::{HSTRING, PCWSTR};

    use super::{IDENTIFIER, InstanceGuard, parse_payload, should_activate};

    /// 与 tauri-plugin-single-instance 相同的 `COPYDATASTRUCT::dwData`
    const COPYDATA_SINGLE_INSTANCE: usize = 1542;

    static ACTIVATE_TX: OnceLock<async_channel::Sender<()>> = OnceLock::new();

    fn names() -> (HSTRING, HSTRING, HSTRING) {
        (
            HSTRING::from(format!("{IDENTIFIER}-sim")),
            HSTRING::from(format!("{IDENTIFIER}-sic")),
            HSTRING::from(format!("{IDENTIFIER}-siw")),
        )
    }

    pub fn acquire() -> Option<InstanceGuard> {
        let (mutex_name, class_name, window_name) = names();
        let mutex = unsafe { CreateMutexW(None, true, &mutex_name) };
        let already = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if !already {
            return Some(InstanceGuard { _mutex: mutex.ok() });
        }

        let args: Vec<String> = std::env::args().collect();
        if !should_activate(&args) {
            log::info!("[app] 开机自启拉起的重复实例，静默退出");
            return None;
        }
        // 首个实例可能还在启动、接收窗口尚未建好：最多等 2 秒
        for _ in 0..20 {
            if let Ok(target) = unsafe { FindWindowW(&class_name, &window_name) }
                && !target.is_invalid()
            {
                let cwd = std::env::current_dir().unwrap_or_default();
                let payload = format!("{}|{}\0", cwd.display(), args.join("|"));
                let bytes = payload.as_bytes();
                let data = COPYDATASTRUCT {
                    dwData: COPYDATA_SINGLE_INSTANCE,
                    cbData: bytes.len() as u32,
                    lpData: bytes.as_ptr() as *mut _,
                };
                unsafe {
                    SendMessageW(
                        target,
                        WM_COPYDATA,
                        Some(WPARAM(0)),
                        Some(LPARAM(&data as *const COPYDATASTRUCT as isize)),
                    );
                }
                log::info!("[app] 检测到已运行的实例，已请求它唤起窗口");
                return None;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        log::warn!("[app] 已有实例在运行但找不到它的接收窗口，退出");
        None
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_COPYDATA && lparam.0 != 0 {
            let data = unsafe { &*(lparam.0 as *const COPYDATASTRUCT) };
            if data.dwData == COPYDATA_SINGLE_INSTANCE && !data.lpData.is_null() {
                let bytes = unsafe {
                    std::slice::from_raw_parts(data.lpData as *const u8, data.cbData as usize)
                };
                let (_, args) = parse_payload(&String::from_utf8_lossy(bytes));
                if should_activate(&args) {
                    log::info!("[app] 检测到重复启动，唤起已运行的窗口");
                    if let Some(tx) = ACTIVATE_TX.get() {
                        let _ = tx.try_send(());
                    }
                }
                return LRESULT(1);
            }
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    pub fn listen(cx: &mut App, on_activate: impl Fn(&mut App) + 'static) {
        let (tx, rx) = async_channel::unbounded::<()>();
        if ACTIVATE_TX.set(tx).is_err() {
            return;
        }
        let (_, class_name, window_name) = names();
        unsafe {
            let instance = GetModuleHandleW(PCWSTR::null()).unwrap_or_default();
            let class = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(wnd_proc),
                hInstance: instance.into(),
                lpszClassName: PCWSTR(class_name.as_ptr()),
                ..Default::default()
            };
            RegisterClassExW(&class);
            // 隐藏的顶层窗口（不显示、不进任务栏）；旧版用 FindWindowW 找它，不能是 message-only
            if let Err(e) = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                &class_name,
                &window_name,
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            ) {
                log::warn!("[app] 创建单实例接收窗口失败: {e}");
                return;
            }
        }
        cx.spawn(async move |cx| {
            while rx.recv().await.is_ok() {
                cx.update(|cx| on_activate(cx));
            }
        })
        .detach();
    }
}

#[cfg(not(windows))]
mod imp {
    use gpui_kit::App;

    use super::InstanceGuard;

    pub fn acquire() -> Option<InstanceGuard> {
        Some(InstanceGuard {})
    }

    pub fn listen(_cx: &mut App, _on_activate: impl Fn(&mut App) + 'static) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_parsing_matches_tauri_format() {
        let (cwd, args) = parse_payload("C:\\Users\\me|C:\\app\\mini-todo.exe|--autostart\0");
        assert_eq!(cwd, "C:\\Users\\me");
        assert_eq!(args, vec!["C:\\app\\mini-todo.exe", "--autostart"]);
        assert!(!should_activate(&args));
        let (_, args) = parse_payload("/home|/usr/bin/mini-todo");
        assert!(should_activate(&args));
    }
}

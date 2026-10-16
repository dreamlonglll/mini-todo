//! 单实例（Windows、macOS）。
//!
//! 与旧版 `tauri-plugin-single-instance` 2.x 使用**同一组名字与消息格式**，新旧两版互相识别，
//! 不会同时打开同一个数据库。Windows：
//! - 命名互斥量 `com.tauri-app.mini-todo-sim`：已存在说明有实例在运行；
//! - 接收窗口：类名 `com.tauri-app.mini-todo-sic`、标题 `com.tauri-app.mini-todo-siw` 的隐藏顶层窗口
//!   （旧版用 `FindWindowW` 找它，所以不能是 message-only 窗口），收 `WM_COPYDATA`
//!   （`dwData = 1542`，内容 `"<cwd>|<arg0>|<arg1>…\0"`）；
//! - 第二个实例把参数发过去后退出；参数含 `--autostart`（开机自启拉起）时静默，不唤起窗口。
//!
//! macOS（2.5.0 mac 版插件的实现，见 `single_instance/unix_socket.rs`）：Unix socket `/tmp/com_tauri_app_mini_todo_si.sock`，
//! 第二个实例连上后写 `"<cwd>\0\0<arg0>\0<arg1>…"` 再退出（含 `--autostart` 时也照发，由首个实例判断是否唤起，与旧版一致）；
//! 首个实例在后台线程接收、经 channel 交给前台唤起主窗口，退出时删除 socket 文件。从 Finder / Dock 再次打开同一个 `.app`
//! 时 LaunchServices 本来就只激活已运行的进程，socket 兜住的是开机自启（LaunchAgent 直接执行可执行文件）、`open -n`、
//! 终端直接运行，以及新旧两版同时存在的情况。
//!
//! 开发专用数据隔离（`MINI_TODO_DATA_DIR`，仅 debug）时不启用（要与正在运行的安装版并存）。
//! Linux 不做单实例（只用于 CI 与截图）。

use gpui_kit::App;

use super::autostart::AUTOSTART_ARG;

#[cfg(any(target_os = "macos", all(test, unix)))]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod unix_socket;

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

/// 在 GPUI 主线程（`Application::run` 回调里）开始接收第二个实例的唤起请求（Windows：建好接收窗口；macOS：起接收线程，
/// 并登记退出时删除 socket 文件），收到时调用 `on_activate`。开发隔离 / Linux 时什么也不做。
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

/// 解析 Windows `WM_COPYDATA` 内容 `"<cwd>|<arg0>|…"`（去掉结尾的 `\0`）→ `(cwd, args)`
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

#[cfg(target_os = "macos")]
mod imp {
    use std::os::unix::net::UnixListener;
    use std::sync::Mutex;

    use gpui_kit::App;

    use super::unix_socket::{self, Acquired, SocketFile};
    use super::{IDENTIFIER, InstanceGuard, should_activate};

    /// `acquire` 时 bind 好的监听 socket，`listen` 时取走交给接收线程。在这之间到达的连接在内核队列里等着
    /// （第二个实例 connect 立即成功、写完即退出，不受影响）
    static SERVER: Mutex<Option<(UnixListener, SocketFile)>> = Mutex::new(None);

    pub fn acquire() -> Option<InstanceGuard> {
        let path = unix_socket::socket_path(IDENTIFIER);
        // 与旧版插件一样：cwd 不是 UTF-8 时发空串；参数按 UTF-8 有损转换（旧版 `env::args()` 遇到非 UTF-8 会 panic）
        let cwd = std::env::current_dir()
            .ok()
            .and_then(|dir| dir.to_str().map(str::to_string))
            .unwrap_or_default();
        let args: Vec<String> = std::env::args_os()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        match unix_socket::acquire(&path, &unix_socket::encode_payload(&cwd, &args)) {
            Acquired::Forwarded => {
                if should_activate(&args) {
                    log::info!("[app] 检测到已运行的实例，已请求它唤起窗口");
                } else {
                    log::info!("[app] 开机自启拉起的重复实例，静默退出");
                }
                None
            }
            Acquired::Primary(server) => {
                *SERVER.lock().unwrap_or_else(|e| e.into_inner()) = Some(server.split());
                Some(InstanceGuard {})
            }
            Acquired::Standalone(e) => {
                log::warn!("[app] 单实例检测失败（{}），照常启动: {e}", path.display());
                Some(InstanceGuard {})
            }
        }
    }

    pub fn listen(cx: &mut App, on_activate: impl Fn(&mut App) + 'static) {
        let Some((listener, socket_file)) = SERVER.lock().unwrap_or_else(|e| e.into_inner()).take()
        else {
            return;
        };
        // 正常退出（⌘Q / 托盘「退出」/ Dock「退出」/ 注销都经 `terminate:` → GPUI 的 on_app_quit）时删除 socket 文件；
        // 崩溃留下的残留由下次启动 connect 得 ConnectionRefused 时清理
        cx.on_app_quit(move |_| {
            socket_file.remove();
            async {}
        })
        .detach();

        let (tx, rx) = async_channel::unbounded::<()>();
        let spawned = std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                unix_socket::serve(listener, |_cwd, args| {
                    if should_activate(&args) {
                        log::info!("[app] 检测到重复启动，唤起已运行的窗口");
                        let _ = tx.try_send(());
                    }
                });
            });
        if let Err(e) = spawned {
            log::warn!("[app] 启动单实例接收线程失败: {e}");
            return;
        }
        cx.spawn(async move |cx| {
            while rx.recv().await.is_ok() {
                cx.update(|cx| {
                    // 第二个实例在别的应用里被拉起：先把本应用切到前台，否则主窗口落在当前前台应用之后
                    cx.activate(true);
                    on_activate(cx);
                });
            }
        })
        .detach();
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
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

//! 收纳箱专用的平台能力（W3）。
//!
//! - [`choose_color`]：收纳箱外观面板的「自定义颜色」。旧版是 WebView2 里的 `<input type="color">`，在 Windows 上弹出
//!   系统 `ChooseColor` 通用对话框；这里直接调用 `ChooseColorW`（父窗口为收纳箱窗口），在独立线程里运行，不阻塞
//!   GPUI 主线程的消息循环。其它平台没有系统取色对话框，返回 `None`（只记日志）。

use gpui_kit::{App, Task};

use super::Hwnd;

/// 弹出系统取色对话框；`initial` 为当前颜色 `(r, g, b)`。用户取消 / 不支持时为 `None`
pub fn choose_color(
    cx: &mut App,
    owner: Option<Hwnd>,
    initial: (u8, u8, u8),
) -> Task<Option<(u8, u8, u8)>> {
    imp::choose_color(cx, owner, initial)
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;
    use std::sync::Mutex;

    use gpui_kit::{App, Task};
    use windows::Win32::Foundation::{COLORREF, HWND};
    use windows::Win32::UI::Controls::Dialogs::{
        CC_ANYCOLOR, CC_FULLOPEN, CC_RGBINIT, CHOOSECOLORW, ChooseColorW,
    };

    use super::Hwnd;

    /// 对话框里的 16 个「自定义颜色」槽：进程内保留（与系统对话框的常见用法一致）
    static CUSTOM_COLORS: Mutex<[u32; 16]> = Mutex::new([0x00FF_FFFF; 16]);

    fn colorref(r: u8, g: u8, b: u8) -> COLORREF {
        COLORREF(u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16))
    }

    fn show(owner: Option<isize>, initial: (u8, u8, u8)) -> Option<(u8, u8, u8)> {
        let mut custom = CUSTOM_COLORS
            .lock()
            .map(|g| *g)
            .unwrap_or([0x00FF_FFFF; 16])
            .map(COLORREF);
        let mut options = CHOOSECOLORW {
            lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
            hwndOwner: HWND(owner.unwrap_or(0) as *mut c_void),
            rgbResult: colorref(initial.0, initial.1, initial.2),
            lpCustColors: custom.as_mut_ptr(),
            Flags: CC_RGBINIT | CC_FULLOPEN | CC_ANYCOLOR,
            ..Default::default()
        };
        let chosen = unsafe { ChooseColorW(&mut options) }.as_bool();
        if let Ok(mut slots) = CUSTOM_COLORS.lock() {
            *slots = custom.map(|c| c.0);
        }
        chosen.then(|| {
            let v = options.rgbResult.0;
            (
                (v & 0xFF) as u8,
                ((v >> 8) & 0xFF) as u8,
                ((v >> 16) & 0xFF) as u8,
            )
        })
    }

    pub fn choose_color(
        cx: &mut App,
        owner: Option<Hwnd>,
        initial: (u8, u8, u8),
    ) -> Task<Option<(u8, u8, u8)>> {
        let owner = owner.map(|h| h.0);
        let (tx, rx) = futures::channel::oneshot::channel();
        let spawned = std::thread::Builder::new()
            .name("fence-color".into())
            .spawn(move || {
                let _ = tx.send(show(owner, initial));
            });
        if let Err(e) = spawned {
            log::error!("[fence] 无法启动取色对话框线程: {e}");
            return Task::ready(None);
        }
        cx.background_executor()
            .spawn(async move { rx.await.ok().flatten() })
    }
}

#[cfg(not(windows))]
mod imp {
    use gpui_kit::{App, Task};

    use super::Hwnd;

    pub fn choose_color(
        _cx: &mut App,
        _owner: Option<Hwnd>,
        _initial: (u8, u8, u8),
    ) -> Task<Option<(u8, u8, u8)>> {
        log::info!("[fence] 系统取色对话框仅在 Windows 上可用");
        Task::ready(None)
    }
}

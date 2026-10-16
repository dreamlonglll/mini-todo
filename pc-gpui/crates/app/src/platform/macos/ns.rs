//! macOS AppKit 原始调用（objc2 0.6 / objc2-app-kit 0.3）：`platform::macos` 的底层。
//!
//! 这个文件**不依赖 GPUI 与本 crate 的其它模块**：入参是窗口号 / NSView 指针，返回普通的 f64 矩形与 bool，坐标是 AppKit
//! 全局点坐标（主屏左下角为原点，y 向上）。这样 Linux / Windows 上也能把它原样编进 `pc-gpui/scripts/mac-check` 做
//! `--target aarch64-apple-darwin` 的类型检查（整个 app 只能在 macOS 上编译）。
//!
//! - **只在主线程调用**：AppKit 对象都是 `MainThreadOnly`。每个函数先 `MainThreadMarker::new()`，不在主线程时返回
//!   `None` / `false`（记一次警告）。本 crate 的调用方都在 GPUI 前台任务 / 事件处理里，GPUI 在 macOS 上的前台就是主线程。
//! - **窗口用窗口号（`windowNumber`，窗口服务器分配的 id）指代**，每次调用都在 `NSApp.windows` 里按窗口号重新找：
//!   GPUI 关窗后 NSWindow 可能已经释放，存下来的对象指针会悬空（地址还可能被新窗口复用），所以这里从不解引用存下来的
//!   指针；找不到就当窗口已经不在，什么都不做。
//! - `setFrame:` / `orderOut:` / 改 style mask 会**同步**触发 GPUI 的窗口回调（`windowDidMove:` / `windowDidResize:` …），
//!   只能在 App 未被借用时调用（见 `platform` 模块文档）；读操作与 [`prepare_new_window`] 不触发回调，随时可调。
//! - 收纳箱专用：桌面层（[`make_desktop_layer`]，层级用 CoreGraphics 的 `CGWindowLevelForKey` 运行时查询）与「自定义颜色」的
//!   系统取色面板（[`show_color_panel`] / [`color_panel_state`]，共享的 NSColorPanel，不按窗口号）。

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSColor, NSColorPanel, NSColorSpace, NSColorSpaceModel, NSEvent,
    NSFloatingWindowLevel, NSNormalWindowLevel, NSPanel, NSScreen, NSView, NSWindow,
    NSWindowAnimationBehavior, NSWindowButton, NSWindowCollectionBehavior, NSWindowLevel,
    NSWindowStyleMask,
};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, ns_string};

// CoreGraphics（AppKit 本来就链接它）：objc2-app-kit 只导出 NSNormalWindowLevel 以上的层级常量，没有桌面层，
// 按 Apple 的建议用运行时查询的 `CGWindowLevelForKey`（不为一个函数多引入 objc2-core-graphics）
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGWindowLevelForKey(key: i32) -> i32;
}

/// `kCGDesktopIconWindowLevelKey`（`CGWindowLevel.h` 的 `CGWindowLevelKey` 枚举值）
const DESKTOP_ICON_WINDOW_LEVEL_KEY: i32 = 18;

/// AppKit 全局点坐标矩形（左下原点，y 向上）
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn from_ns(r: NSRect) -> Self {
        Self {
            x: r.origin.x,
            y: r.origin.y,
            width: r.size.width,
            height: r.size.height,
        }
    }

    fn to_ns(self) -> NSRect {
        NSRect::new(
            NSPoint::new(self.x, self.y),
            NSSize::new(self.width, self.height),
        )
    }
}

/// 一块屏幕
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    /// `NSScreenNumber`（= `CGDirectDisplayID` = GPUI 的 `DisplayId`）；取不到时 0
    pub display_id: u32,
    pub frame: Rect,
    pub visible: Rect,
    pub scale: f64,
}

fn main_thread() -> Option<MainThreadMarker> {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let mtm = MainThreadMarker::new();
    if mtm.is_none() && !WARNED.swap(true, Ordering::Relaxed) {
        log::warn!("[platform] AppKit 调用不在主线程，已忽略（之后不再提示）");
    }
    mtm
}

fn screen_number(screen: &NSScreen) -> Option<u32> {
    let description = screen.deviceDescription();
    let value = description.objectForKey(ns_string!("NSScreenNumber"))?;
    let number = value.downcast_ref::<NSNumber>()?;
    Some(number.unsignedIntValue())
}

/// 全部屏幕（`NSScreen.screens` 顺序：第 0 个是菜单栏所在的主屏，它的左下角就是 AppKit 坐标原点）
pub fn screens() -> Vec<Screen> {
    let Some(mtm) = main_thread() else {
        return Vec::new();
    };
    NSScreen::screens(mtm)
        .to_vec()
        .iter()
        .map(|screen| Screen {
            display_id: screen_number(screen).unwrap_or(0),
            frame: Rect::from_ns(screen.frame()),
            visible: Rect::from_ns(screen.visibleFrame()),
            scale: screen.backingScaleFactor(),
        })
        .collect()
}

/// 系统光标位置（`NSEvent.mouseLocation`，全局点坐标）
pub fn mouse_location() -> Option<(f64, f64)> {
    main_thread()?;
    let location = NSEvent::mouseLocation();
    Some((location.x, location.y))
}

/// GPUI 窗口内容视图（raw-window-handle `AppKitWindowHandle::ns_view`）所在 NSWindow 的窗口号
///
/// # Safety
///
/// `ns_view` 必须指向一个存活的 NSView：调用方持有对应 GPUI `Window` 的引用期间成立
pub unsafe fn window_number_of_view(ns_view: *mut c_void) -> Option<isize> {
    main_thread()?;
    if ns_view.is_null() {
        return None;
    }
    // SAFETY: 见函数文档；只在主线程上借用，不保存
    let view: &NSView = unsafe { &*ns_view.cast::<NSView>() };
    let number = view.window()?.windowNumber();
    (number > 0).then_some(number)
}

/// 按窗口号在本应用的窗口里找（含已隐藏 / 最小化的窗口）
fn find(mtm: MainThreadMarker, number: isize) -> Option<Retained<NSWindow>> {
    if number <= 0 {
        return None;
    }
    NSApplication::sharedApplication(mtm)
        .windows()
        .to_vec()
        .into_iter()
        .find(|window| window.windowNumber() == number)
}

fn with_window<R>(number: isize, f: impl FnOnce(&NSWindow) -> R) -> Option<R> {
    let mtm = main_thread()?;
    let window = find(mtm, number)?;
    Some(f(&window))
}

/// 窗口还在（本应用的窗口列表里有这个窗口号）
pub fn is_alive(number: isize) -> bool {
    with_window(number, |_| ()).is_some()
}

/// 窗口 frame（全局点坐标）。GPUI 的窗口都带 `FullSizeContentView`，frame 就是内容区（= 可见区域）
pub fn window_frame(number: isize) -> Option<Rect> {
    with_window(number, |window| Rect::from_ns(window.frame()))
}

/// 窗口当前所在屏幕的 `NSScreenNumber`（窗口完全在屏幕外时 `None`）
pub fn window_display_id(number: isize) -> Option<u32> {
    with_window(number, |window| {
        window.screen().and_then(|s| screen_number(&s))
    })
    .flatten()
}

/// `setFrame:display:`（同步触发 GPUI 的移动 / 缩放回调）
pub fn set_frame(number: isize, frame: Rect) -> bool {
    with_window(number, |window| {
        window.setFrame_display(frame.to_ns(), true)
    })
    .is_some()
}

/// `setFrameOrigin:`（frame 左下角，全局点坐标；同步触发移动回调）
pub fn set_origin(number: isize, x: f64, y: f64) -> bool {
    with_window(number, |window| window.setFrameOrigin(NSPoint::new(x, y))).is_some()
}

/// 置顶：`NSFloatingWindowLevel` ⇄ `NSNormalWindowLevel`
pub fn set_floating(number: isize, floating: bool) -> bool {
    let level = if floating {
        NSFloatingWindowLevel
    } else {
        NSNormalWindowLevel
    };
    with_window(number, |window| window.setLevel(level)).is_some()
}

/// 收纳箱的窗口层级：Finder 画桌面图标的那一层之上一层（= -2147483602）。在桌面图标之上、所有普通窗口（0）之下；
/// 正好等于桌面图标层时与 Finder 的图标窗口前后顺序不定，点击偶尔被它截走，所以 +1
pub fn desktop_layer_level() -> NSWindowLevel {
    // SAFETY: 纯查询，参数是头文件里的枚举值
    let icon_level = unsafe { CGWindowLevelForKey(DESKTOP_ICON_WINDOW_LEVEL_KEY) };
    icon_level as NSWindowLevel + 1
}

/// 收纳箱：把窗口放到桌面层（[`desktop_layer_level`]）并设好"像桌面一样"的行为，不显示、不改 key 窗口。
/// 幂等，建窗后（窗口仍隐藏时）调用一次、每次重新显示前再调用一次。
///
/// - collectionBehavior **整体替换**（GPUI 的 `PopUp` 建窗时设的是 `CanJoinAllSpaces | FullScreenAuxiliary`）：
///   所有桌面空间都显示（`CanJoinAllSpaces`）、调度中心 / 显示桌面时不动也不隐藏（`Stationary`；非 Normal 层级的窗口默认
///   `Transient`，会在调度中心里被藏起来）、不参与 ⌘\` 循环（`IgnoresCycle`）、自己不进全屏也不进别的应用的全屏空间
///   （`FullScreenNone`）；
/// - `canHide = NO`：⌘H（`[NSApp hide:]`）与别的应用的「隐藏其他」都不隐藏它；
/// - `hidesOnDeactivate = NO`（NSPanel 默认 YES）、不进「窗口」菜单、去掉显示 / 隐藏的系统动画。
///
/// 收纳箱窗口之后绝不能再调 [`set_floating`]（会把层级改回 0 / 3）
pub fn make_desktop_layer(number: isize) -> bool {
    with_window(number, |window| {
        window.setLevel(desktop_layer_level());
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle
                | NSWindowCollectionBehavior::FullScreenNone,
        );
        window.setCanHide(false);
        window.setHidesOnDeactivate(false);
        window.setExcludedFromWindowsMenu(true);
        window.setAnimationBehavior(NSWindowAnimationBehavior::None);
    })
    .is_some()
}

/// 窗口实际生效的层级与行为（写进日志供 e2e 核对：CGWindowList 只给得出层级，collectionBehavior / canHide 读不到）
pub fn describe_window(number: isize) -> Option<String> {
    with_window(number, |window| {
        format!(
            "level={} behavior={:#x} canHide={} excludedFromWindowsMenu={} hidesOnDeactivate={} \
             nonactivating={} panel={} visible={}",
            window.level(),
            window.collectionBehavior().0,
            window.canHide(),
            window.isExcludedFromWindowsMenu(),
            window.hidesOnDeactivate(),
            window
                .styleMask()
                .contains(NSWindowStyleMask::NonactivatingPanel),
            window.downcast_ref::<NSPanel>().is_some(),
            window.isVisible(),
        )
    })
}

/// 显示（`orderFrontRegardless`：应用不在前台时也显示到同层最前，不改 key / main 窗口）
pub fn order_front(number: isize) -> bool {
    with_window(number, |window| window.orderFrontRegardless()).is_some()
}

/// 隐藏（`orderOut:`）
pub fn order_out(number: isize) -> bool {
    with_window(number, |window| window.orderOut(None)).is_some()
}

pub fn is_visible(number: isize) -> bool {
    with_window(number, |window| window.isVisible()).unwrap_or(false)
}

pub fn is_miniaturized(number: isize) -> bool {
    with_window(number, |window| window.isMiniaturized()).unwrap_or(false)
}

/// 从程序坞还原
pub fn deminiaturize(number: isize) -> bool {
    with_window(number, |window| window.deminiaturize(None)).is_some()
}

/// 激活并成为 key 窗口（`makeKeyAndOrderFront:`；配合 [`activate_app`] 才会切到前台）
pub fn make_key_and_order_front(number: isize) -> bool {
    with_window(number, |window| window.makeKeyAndOrderFront(None)).is_some()
}

/// 键盘焦点的现状（诊断日志用）：本应用是否在前台、当前 key 窗口的窗口号、窗口 `number` 是不是 key 窗口。
/// 只读，不触发 GPUI 回调，随时可调
pub fn describe_focus(number: isize) -> Option<String> {
    let mtm = main_thread()?;
    let app = NSApplication::sharedApplication(mtm);
    let key_window = app
        .keyWindow()
        .map_or_else(|| "无".to_string(), |w| format!("#{}", w.windowNumber()));
    let this_key = find(mtm, number).is_some_and(|w| w.isKeyWindow());
    Some(format!(
        "appActive={} keyWindow={key_window} thisKey={this_key}",
        app.isActive()
    ))
}

/// 把本应用切到前台（`activateIgnoringOtherApps:`，与 GPUI `cx.activate(true)` 相同）
pub fn activate_app() -> bool {
    let Some(mtm) = main_thread() else {
        return false;
    };
    // macOS 14 起建议用 `activate`，但它只是"请求"、常被系统拒绝；GPUI 自己也用这个
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    true
}

/// 隐藏系统红绿灯按钮（关闭 / 最小化 / 缩放）：界面自己画标题栏
fn hide_standard_buttons(window: &NSWindow) {
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = window.standardWindowButton(kind) {
            button.setHidden(true);
        }
    }
}

/// 建窗后立即调用：隐藏红绿灯；面板（GPUI 的 `PopUp` / `Floating` 窗口是 NSPanel）默认在应用失活时自动隐藏，
/// 关掉这个行为（应用内通知要在别的应用在前台时也显示）。不触发 GPUI 回调
pub fn prepare_new_window(number: isize) -> bool {
    with_window(number, |window| {
        hide_standard_buttons(window);
        window.setHidesOnDeactivate(false);
    })
    .is_some()
}

/// 改 style mask 的几个位。AppKit 改 mask 时可能重建标题栏（红绿灯重新出现）并换掉第一响应者（键盘输入不再进 GPUI
/// 视图）：改完重新隐藏按钮、把第一响应者还回去
fn update_style(window: &NSWindow, set: NSWindowStyleMask, clear: NSWindowStyleMask) {
    let old = window.styleMask();
    let mut new = old;
    new.remove(clear);
    new.insert(set);
    if new == old {
        return;
    }
    let responder = window.firstResponder();
    window.setStyleMask(new);
    if let Some(responder) = responder {
        window.makeFirstResponder(Some(&responder));
    }
    hide_standard_buttons(window);
}

/// 可缩放（`NSWindowStyleMaskResizable`；与 Windows 不同，缩放完全由 AppKit 按 mask 处理，运行时改了就生效）
pub fn set_resizable(number: isize, resizable: bool) -> bool {
    with_window(number, |window| {
        let bit = NSWindowStyleMask::Resizable;
        if resizable {
            update_style(window, bit, NSWindowStyleMask::empty());
        } else {
            update_style(window, NSWindowStyleMask::empty(), bit);
        }
    })
    .is_some()
}

/// 可最小化（`NSWindowStyleMaskMiniaturizable`）
pub fn set_miniaturizable(number: isize, miniaturizable: bool) -> bool {
    with_window(number, |window| {
        let bit = NSWindowStyleMask::Miniaturizable;
        if miniaturizable {
            update_style(window, bit, NSWindowStyleMask::empty());
        } else {
            update_style(window, NSWindowStyleMask::empty(), bit);
        }
    })
    .is_some()
}

/// 锁定前是否可缩放（解锁时恢复），按窗口号登记
fn resizable_before_lock() -> &'static Mutex<HashMap<isize, bool>> {
    static MAP: std::sync::OnceLock<Mutex<HashMap<isize, bool>>> = std::sync::OnceLock::new();
    MAP.get_or_init(Default::default)
}

/// 固定模式的几何锁定：`setMovable:NO`（用户拖不动，程序摆窗不受影响）+ 去掉可缩放；解锁时恢复锁定前的可缩放
pub fn set_geometry_locked(number: isize, locked: bool) -> bool {
    with_window(number, |window| {
        if locked {
            let was_resizable = window.styleMask().contains(NSWindowStyleMask::Resizable);
            resizable_before_lock()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(number)
                .or_insert(was_resizable);
            window.setMovable(false);
            update_style(
                window,
                NSWindowStyleMask::empty(),
                NSWindowStyleMask::Resizable,
            );
        } else {
            let restore = resizable_before_lock()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&number);
            window.setMovable(true);
            if restore == Some(true) {
                update_style(
                    window,
                    NSWindowStyleMask::Resizable,
                    NSWindowStyleMask::empty(),
                );
            }
        }
    })
    .is_some()
}

// ----------------------------------------------------------------------
// 收纳箱「自定义颜色」：系统取色面板（共享的 NSColorPanel）
// ----------------------------------------------------------------------

/// 显示系统取色面板，初始色 `initial`（sRGB 0~255）、不显示透明度。
///
/// NSPanel 默认在应用失活时自动隐藏（`hidesOnDeactivate`），用户点一下桌面面板就消失、会被轮询当成"关掉了"：关掉这个行为。
/// 本应用多半不在前台（收纳箱是不激活应用的面板），先把应用切到前台再 `makeKeyAndOrderFront:`。
/// 会让当前 key 窗口失去 key（触发 GPUI 回调），只在 App 未被借用时调用
pub fn show_color_panel(initial: (u8, u8, u8)) -> bool {
    let Some(mtm) = main_thread() else {
        return false;
    };
    let panel = NSColorPanel::sharedColorPanel(mtm);
    panel.setShowsAlpha(false);
    panel.setHidesOnDeactivate(false);
    let channel = |v: u8| f64::from(v) / 255.0;
    let color = NSColor::colorWithSRGBRed_green_blue_alpha(
        channel(initial.0),
        channel(initial.1),
        channel(initial.2),
        1.0,
    );
    panel.setColor(&color);
    activate_app();
    panel.makeKeyAndOrderFront(None);
    true
}

/// 取色面板的现状
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ColorPanelState {
    /// 还显示着（用户关掉面板后为 false）
    pub visible: bool,
    /// 当前颜色（sRGB 0~255）；换不成 RGB 时 `None`
    pub color: Option<(u8, u8, u8)>,
}

/// 取色面板的现状；面板从未建过时 `None`
pub fn color_panel_state() -> Option<ColorPanelState> {
    let mtm = main_thread()?;
    if !NSColorPanel::sharedColorPanelExists(mtm) {
        return None;
    }
    let panel = NSColorPanel::sharedColorPanel(mtm);
    Some(ColorPanelState {
        visible: panel.isVisible(),
        color: srgb8(&panel.color()),
    })
}

/// 换成 sRGB 的 0~255 分量。对非 RGB 模型的颜色（图案色、灰度…）读 `redComponent` 会抛 ObjC 异常（穿过 Rust 帧直接
/// abort），所以先换色彩空间（换不了返回 nil）、再确认是 RGB 模型
fn srgb8(color: &NSColor) -> Option<(u8, u8, u8)> {
    let srgb = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    if srgb.colorSpace().colorSpaceModel() != NSColorSpaceModel::RGB {
        return None;
    }
    let to8 = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some((
        to8(srgb.redComponent()),
        to8(srgb.greenComponent()),
        to8(srgb.blueComponent()),
    ))
}

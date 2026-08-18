# 固定模式下消除任务栏图标闪现

## 背景

固定模式（`IS_FIXED_MODE == true`）下主窗口应当完全不出现在 Windows 任务栏，也不参与
Alt+Tab。实现方式是手动给 HWND 打 `WS_EX_TOOLWINDOW`、去 `WS_EX_APPWINDOW`
（`commands/window.rs::apply_fixed_ex_style`）。

但用户观察到：**点击系统托盘图标的一瞬间**，以及**贴边折叠状态下鼠标移入唤起的一瞬间**，
任务栏会闪现该程序的图标，停留约 0.2s 后消失。

## 根因

`WS_EX_TOOLWINDOW` 是绕开 tao 手动设置的，tao 内部并不知道它的存在。

tao 维护自己的一套 `WindowFlags`，任何窗口 flag 变更（`set_always_on_top` / `show` /
`unminimize` / `set_resizable`）都会触发 `WindowFlags::apply_diff`
（tao-0.34.5 `platform_impl/windows/window_state.rs:426-455`）：

```rust
if diff != WindowFlags::empty() {
    let (style, style_ex) = new.to_window_styles();          // 只按 tao 自己的 flags 重算
    SetWindowLongW(window, GWL_STYLE, style.0 as i32);
    SetWindowLongW(window, GWL_EXSTYLE, style_ex.0 as i32);  // 整体覆写，抹掉 TOOLWINDOW
    SetWindowPos(window, None, 0,0,0,0, ... | SWP_FRAMECHANGED);  // 让 shell 立刻生效
}
```

`to_window_styles()` 中 `ON_TASKBAR` flag → 加回 `WS_EX_APPWINDOW`，且永远不会产出
`WS_EX_TOOLWINDOW`。于是每次 tao 操作窗口，任务栏按钮立刻出现。

现有的 `reassert_fixed_taskbar_style` 是**事后补救**：通过 `run_on_main_thread` 排队再把
样式打回去。排队到执行之间的这段时间就是可见的闪烁窗口期。该设计只能保证最终状态正确，
无法消除中间态。

触发点：

| 现象 | 触发点 | tao 调用 |
|---|---|---|
| 点托盘一闪 | `bring_main_window_to_front` (window.rs:1101-1103) | `unminimize()` + `show()` + `set_always_on_top(true)` 三连击 |
| 折叠展开一闪 | `tick_auto_hide` Restore 分支 (window.rs:576) | `set_always_on_top(true)` |
| 收回时 | `tick_auto_hide` Hide 分支 (window.rs:560) | `set_always_on_top(false)` |
| 最小化守护 | `lib.rs:267-268` | `unminimize()` + `show()` |

## 需求

固定模式下，程序图标在任务栏中**零出现、零闪烁**。不是"闪得更短"，而是根本不产生
任务栏按钮。

非固定模式行为完全不变（正常出现在任务栏、正常参与 Alt+Tab）。

## 方案

固定模式下不再走 tao 的窗口 flag 通路，改用 Win32 直接完成等价操作，从源头不产生
ex style 覆写：

- `set_always_on_top(x)` → `SetWindowPos(hwnd, HWND_TOPMOST/HWND_NOTOPMOST, 0,0,0,0,
  SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)`，只翻 `WS_EX_TOPMOST` 位，不碰 TOOLWINDOW
- `show()` / `unminimize()` → `ShowWindow(hwnd, SW_SHOWNOACTIVATE / SW_RESTORE)`

非固定模式下继续走 tao 原路径，保持行为不变。

### 已排除的备选方案

改用 tao 自带的 `set_skip_taskbar()`（走 `ITaskbarList::DeleteTab` COM 调用，
不受 `apply_diff` 影响）。不采用：它只摘任务栏按钮，不影响 Alt+Tab，会丢掉当前
`WS_EX_TOOLWINDOW` 带来的「不参与 Alt+Tab」行为。

### 已知代价

tao 内部 `WindowFlags` 会与真实窗口状态漂移（tao 以为未置顶、实际已置顶）。后果是
后续任何 tao 侧的 flag 操作（如前端 `appStore.ts:387` 的 `setResizable(false)`）
会顺手把置顶撤掉。因此 `reassert_fixed_taskbar_style` 需保留并扩展为
「同时重申 TOOLWINDOW 样式 + 置顶态」的兜底。

## 验收标准

1. 固定模式下单击/双击托盘图标唤起窗口，任务栏全程无图标出现
2. 固定模式下贴边折叠，鼠标移入唤起，任务栏全程无图标出现
3. 固定模式下窗口收回折叠，任务栏无图标出现
4. 固定模式下窗口仍不参与 Alt+Tab
5. 唤起后窗口确实被抬到最前（能盖住最大化窗口），置顶到期后正常撤销
6. 退出固定模式后，窗口正常出现在任务栏、正常参与 Alt+Tab
7. `cargo check` / `cargo test` 通过

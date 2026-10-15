# Desktop Window E2E Thinking Guide

> **Purpose**: Before claiming a main-window mode / style / Z-order change — or any desktop-fence
> window change — "works", run the checks below. Unit tests cannot see Win32 state; the only proof
> is the live window.

---

## When to Use

- [ ] You changed anything under `IS_FIXED_MODE` / `IS_DESKTOP_MODE` handling in `commands/window.rs`
- [ ] You added a tao call (`set_resizable`, `set_minimizable`, `set_always_on_top/bottom`) anywhere on the main window
- [ ] You touched `apply_fixed_ex_style`, owner (`GWLP_HWNDPARENT`), `SetWindowPos`, or the 200 ms polling thread
- [ ] You changed how `screen_configs` / `is_fixed` / `fixed_embed_desktop` are restored at startup
- [ ] 改了收纳箱窗口：`commands/fence.rs` 的建窗参数、`EMBEDDED_WINDOWS` / `tick_embedded_windows`、`desktop_attach_hwnd`、
      `lacks_tool_window_style` 自愈、`FenceView.vue` 的移动 / 缩放 / 折叠、`fence_layouts` 的写入时机
- [ ] 改了会移动用户文件的收纳箱路径（`services/fence_fs`、`fence_move_in` / `fence_restore_item` / `delete_fence`）

---

## 开发版与安装版并存（动手前）

1. **必须带隔离变量启动开发版**：`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`（测收纳箱屏幕组合时再加
   `MINI_TODO_SCREEN_ID_FILE`），都指向会话临时目录。不带就不要启动——开发版会打开真实库、被单实例插件转交给安装版。
   变量含义见 `.trellis/spec/backend/desktop-fences.md` §3.6。
2. **按 PID / exe 路径（`target\debug\mini-todo.exe`）定位开发版**的进程与窗口；绝不操作安装版的进程、窗口、托盘图标
   （开发版托盘提示为"Mini Todo（开发版）"）；不切换托盘「开机自启动」（注册表项与安装版共用）。
3. **Explorer 重启（`restart-explorer`）每次先征得用户授权**，结束后确认 Explorer 已恢复。
4. 结束前清理测试数据目录、测试收纳根目录和桌面上的测试文件。

## 输入安全

- **不得对用户窗口注入 OS 级鼠标键盘输入**（`SetCursorPos` / `mouse_event` / `keybd_event` / `SendKeys` / 真实 OLE 拖动源）。
  教训：一次拖动源脚本的辅助线程没做 per-monitor DPI 感知，`SetCursorPos(-2300,1200)` 被虚拟化到 (0,1500)，鼠标按下落到了
  用户最大化的窗口上。
- 页面内交互优先用 CDP：`Input.dispatchMouseEvent` / `dispatchKeyEvent` / `insertText`，只作用于该 WebView、不动真实光标。
- 外部文件拖入用 `plugin:event|emit_to` 给 `fence-{id}` 发 `tauri://drag-enter` + `tauri://drag-drop`（payload `{ paths, position }`），
  之后走真实的 `IFileOperation`。
- 确需真实输入（Win+D、真实拖动标题栏）：只在用户离开、会话解锁、前台不是管理员权限程序时做；每个发输入的线程先设
  per-monitor DPI 感知，并先核对目标坐标命中的是开发版窗口；Win+D / Win+M 之后恢复用户的窗口。

---

## Checklist

1. **Start the dev build with CDP enabled** — `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`
   so `pc/scripts/e2e/cdp.mjs` can click title-bar buttons without the "first click lost" problem.
2. **Assert the Win32 state, not the UI** — `e2e.ps1 -Step json`; compare every field against the
   mode table in `.trellis/spec/backend/window-modes.md` §3 (owner, ex style, minimize box, zIndex, rect, DPI).
3. **Show Desktop twice** (`-Step win-d`), **Minimize All** (`-Step win-m`): never iconic, never hidden, rect unchanged, centre hit is still us.
4. **Something must be able to cover it** — show a plain top-level window over the rect; it must win `WindowFromPoint`.
5. **Real input only under "输入安全" rules** — `mouse.ps1` at `clientOrigin + css × devicePixelRatio`, verify with CDP DOM reads;
   otherwise use CDP input. Editor windows opened from the main window must take foreground on the first click and accept keys.
6. **Explorer restart** (`-Step restart-explorer`, kills `explorer.exe`, user authorization first) — window alive, re-attached, Win+D still immune.
7. **Process restart** — kill the exe by PID, start it again (vite separately), state and rect restored from `screen_configs`.
8. **Loop the transitions** several times — same HWND, process alive, rect unchanged after every step.
9. **Put the user's mode back** before you leave (they run fixed mode daily) and delete any test todos through the store, not raw SQL.

## 收纳箱专属检查项

细节与断言见 `.trellis/spec/backend/desktop-fences.md` §3.1 与 §6。

1. `fence.ps1 -Step json`：每个收纳箱 ownerIsProgman / toolWindow / 无 appWindow / 无 minimizeBox / 无 thickFrame / 无 topmost /
   无 noActivate，且新建、折叠、锁定、改名、改外观、显示隐藏、重置位置之后都不变。
2. 新建与「显示/隐藏所有收纳箱」×3：UIA 轮询任务栏无按钮。
3. Win+D ×2 + 探针（`fence.ps1 -Step probe-start`）：收纳箱从不最小化；第二次 Win+D 后探针恢复、原前台窗口拿回焦点。
4. Explorer 重启：全部收纳箱第一次轮询内重挂到新 Progman。
5. 拖动 / 缩放结束后 `fences` 与当前组合 `fence_layouts` 同时落库；启动夹回、屏幕组合切换、程序挪窗**不**落库。
6. 混合 DPI：折叠高度按目标显示器缩放（150% → 48px，125% → 40px）。
7. 主窗口固定 + 嵌入且有收纳箱时重启进程：主窗口约 200ms 内恢复 toolWindow（`lacks_tool_window_style` 自愈）。
8. 文件层只在桌面新建 `__fence_e2e_` 前缀的测试文件；绝不移动、改名、删除用户真实的桌面文件。

---

## Gotchas Seen

- `SendKeys` goes through the user's IME: text arrives transformed and Element Plus ignores
  `input` events while composing. Switch to an English layout or use CDP `Input.insertText`.
- PowerShell parses `0xFFFFFFFF` as `-1`; wheel deltas need the explicit `+ 2^32` wrap (done in `mouse.ps1`).
- Do not put `$` or escaped double quotes in the JS you pass to `cdp.mjs` from PowerShell.
- The window rect can move a few pixels while you are paused and the user pokes at it — snapshot `rect` at the start of a loop, not from memory.
- 前台是管理员权限程序（部分游戏客户端）或屏幕已锁定时，模拟输入与 Shell `ToggleDesktop()` 都会被丢弃。
- 带隔离变量时开发版的 WebView2 用户数据在 `<data>\EBWebView\EBWebView`，CDP 9222 只看得到开发版页面；`cdp.mjs` 默认目标排除了
  `#/fence`，操作收纳箱页面用 `--target '#/fence?id=N'`。

→ Executable details: `.trellis/spec/backend/window-modes.md` §6, `.trellis/spec/backend/desktop-fences.md` §6, scripts in `pc/scripts/e2e/`.

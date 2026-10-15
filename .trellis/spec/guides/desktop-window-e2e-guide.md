# Desktop Window E2E Thinking Guide

> **Purpose**: Before claiming a main-window mode / style / Z-order change — or any desktop-fence
> window change — "works", run the checks below. Unit tests cannot see Win32 state; the only proof
> is the live window.
>
> 自动化套件：`pc-gpui/scripts/e2e-windows/`（真实 GPUI 窗口 + 真实鼠标键盘输入 + Win32 只读断言 + 截图取色 + 读 `data.db`；
> 用例表与本地运行方法见其 README）。CI：`.github/workflows/pc-gpui-e2e-windows.yml`，在一次性的 GitHub 托管 Windows 虚拟机上跑
> （门槛见 `.trellis/spec/pc-gpui/verification.md` §6）。GPUI 窗口没有 DOM / CDP：按钮位置按布局常量（逻辑像素）点击，会随内容移动的
> 目标先截图按颜色识别再点。旧 Tauri 版的 CDP / `e2e.ps1` / `fence.ps1` 工具已随旧版删除。

---

## When to Use

- [ ] 改了主窗口模式：`pc-gpui/crates/app/src/services/window_mode.rs`（`plan_mode` / `apply_mode` / 200ms 轮询 / 贴边隐藏）
- [ ] 改了 `platform/embed.rs`、`platform/subclass.rs`（`FLAG_BOTTOM` / `FLAG_LOCK_GEOMETRY`）、`platform/windows.rs` 的样式 / owner / Z 序 / 几何
- [ ] 改了启动时 `screen_configs` / `is_fixed` / `fixed_embed_desktop` 的恢复
- [ ] 改了收纳箱窗口：`services/fences.rs` 的建窗参数与嵌入守护、`windows/fence/` 的移动 / 缩放 / 折叠 / 外部拖入、`fence_layouts` 的写入时机
- [ ] 改了会移动用户文件的收纳箱路径（core `services/fence_fs`、`fence::items::{move_in, restore, move_to}`、`fence::delete`）
- [ ] 改了通知窗口定位、托盘、单实例、开机自启、Toast、DPAPI，或 release profile / `build.rs`

---

## 开发版与安装版并存（在自己的机器上跑之前）

1. **能用 CI 就用 CI**（`workflow_dispatch` 触发 `pc-gpui-e2e-windows.yml`）：套件会最小化其它窗口、重启 Explorer、改开机自启注册表项、
   移动测试文件，在一次性虚拟机上跑最安全。
2. **必须带隔离变量启动开发版**：`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`（测收纳箱屏幕组合时再加
   `MINI_TODO_SCREEN_ID_FILE`），都指向会话临时目录（`run_e2e.py` 的隔离用例自动设置）。不带就不要启动——开发版会打开真实库、
   被单实例转交给安装版。变量含义见 `.trellis/spec/backend/desktop-fences.md` §3.6。
3. **按 PID / exe 路径（`pc-gpui\target\debug\mini-todo.exe`）定位开发版**的进程与窗口；绝不操作安装版的进程、窗口、托盘图标
   （开发版托盘提示为"Mini Todo（开发版）"）；不切换托盘「开机自启动」（注册表项 `HKCU\…\Run\mini-todo` 与安装版共用；
   套件 I06 会写入再删除它，已开启开机自启的机器跑完要重新打开）。
4. **不隔离的 P 组**（真实数据目录、日志位置、单实例唤起）只在一次性虚拟机 / 没装正式版的机器上加 `--allow-real-profile` 跑。
5. **Explorer 重启**（S06 / F06 会 `taskkill explorer.exe` 再拉起）每次先征得用户授权，结束后确认 Explorer 已恢复。
6. 结束前清理测试数据目录、测试收纳根目录和桌面上的测试文件。

## 输入安全

- 套件用**真实输入**（`SetCursorPos` + `SendInput`、真实 OLE 拖放源）。在用户机器上跑 = 接管用户的鼠标键盘：只在用户离开、会话解锁、
  前台不是管理员权限程序时跑；中文输入法切到英文；测试期间不动鼠标键盘。
- 教训（旧版）：一次拖动源脚本的辅助线程没做 per-monitor DPI 感知，`SetCursorPos(-2300,1200)` 被虚拟化到 (0,1500)，鼠标按下落到了
  用户最大化的窗口上。每个发输入的线程先设 per-monitor DPI 感知，并先核对目标坐标命中的是开发版窗口；Win+D / Win+M 之后恢复用户的窗口。
- 不要用 `SendKeys`（走输入法，文字会被转换）。

---

## Checklist

1. **Assert the Win32 state, not the UI** — owner、ex style（TOOLWINDOW / APPWINDOW / TOPMOST / NOACTIVATE）、`WS_MINIMIZEBOX`、
   Z 序、客户区几何、`WM_NCHITTEST` 结果；逐字段对照 `.trellis/spec/backend/window-modes.md` §3 的模式表与 `desktop-fences.md` §3.1 的状态表。
2. **Show Desktop**（Win+D）**/ Minimize All**（Win+M）：嵌入的窗口从不最小化、不隐藏、几何不变；经典固定被最小化后一个 tick 内还原。
3. **Something must be able to cover it** — 普通顶层窗口（`probe-window.ps1` 探针）盖在上面时必须赢 `WindowFromPoint`。
4. **Real input** — 编辑类窗口从主窗口打开后第一次点击就拿到前台并接受按键；固定模式下拖标题栏 / 边框不动。
5. **Explorer restart** — 窗口存活、重挂到新 Progman、Win+D 仍免疫。
6. **Process restart** — 按 PID 结束再启动，状态与几何从 `screen_configs` / `fences` / `fence_layouts` 恢复。
7. **Loop the transitions** several times — 同一 HWND、进程存活、每一步之后几何不变。
8. **Put the user's mode back** before you leave (they run fixed mode daily) and delete test todos through the app, not raw SQL.

## 收纳箱专属检查项

细节与断言见 `.trellis/spec/backend/desktop-fences.md` §3.1 与 §6。

1. 每个收纳箱 owner = 桌面宿主 / TOOLWINDOW / 无 APPWINDOW / 无 MINIMIZEBOX / 无 TOPMOST / 无 NOACTIVATE，且新建、折叠、锁定、改名、
   改外观、显示隐藏、重置位置之后都不变；新建与「显示/隐藏所有收纳箱」时任务栏不出现按钮。
2. Win+D：收纳箱从不最小化；普通应用窗口能盖住它。
3. Explorer 重启：全部收纳箱第一次守护 tick 内重挂到新 Progman。
4. 拖动 / 缩放结束后 `fences` 与当前组合 `fence_layouts` 同时落库，且可见区域 = 库里的几何；启动夹回、屏幕组合切换、程序挪窗**不**落库。
5. 混合 DPI：折叠高度按目标显示器缩放（150% → 48px，125% → 40px）——CI 虚拟机是单屏，需用户实机验收。
6. 外部拖入：新实例第一次拖入、改名回车后 / 单击标题后紧接着的第一次拖入都要生效（GPUI 输入模态问题，e2e F04 / F10 / F11）。
7. 文件层只在测试收纳根目录与桌面上 `__fence_e2e_` 前缀的测试文件上操作；绝不移动、改名、删除用户真实的桌面文件。

---

## Gotchas Seen

- 前台是管理员权限程序（部分游戏客户端）或屏幕已锁定时，模拟输入与 Shell `ToggleDesktop()` 都会被丢弃。
- PowerShell parses `0xFFFFFFFF` as `-1`; wheel deltas need the explicit `+ 2^32` wrap.
- The window rect can move a few pixels while you are paused and the user pokes at it — snapshot the rect at the start of a loop, not from memory.
- GPUI 窗口在左 / 右 / 下有不可见边框（96 DPI 下各 8px）：断言几何时比较**客户区**，不要比较 `GetWindowRect`。
- 无 GPU 的 CI 虚拟机上 GPUI 走 D3D11 WARP 软件渲染，截图取色依赖桌面是纯色（`set-display.ps1`）。

→ Executable details: `.trellis/spec/backend/window-modes.md` §6, `.trellis/spec/backend/desktop-fences.md` §6,
`pc-gpui/scripts/e2e-windows/README.md`.

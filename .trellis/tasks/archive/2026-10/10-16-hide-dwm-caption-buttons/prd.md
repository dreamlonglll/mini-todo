# 去掉 DWM 透出的系统标题栏按钮（收纳箱 / 主窗口右上角小 ×）

## Goal

3.0.0-pre 实机上，收纳箱与主窗口（深色主题 + 背景透明）右上角出现一个系统的小 ×，窗口获得焦点时 × 后面还有一块暗红底
（延伸到收纳箱"…"菜单按钮下面）。这是 DWM 画的系统标题栏关闭按钮从半透明客户区透出来，不是我们的界面。目标：所有窗口不再出现
系统标题栏按钮，且不破坏窗口的其它系统行为（任务栏、最大化、Alt+F4、几何）。

## What I already know（2026-10-16 实测，用户机 Win11 26H2 build 26300，144 / 120 DPI 双屏）

- **根因**：gpui-pre-windows 0.3.9 建"无标题栏"窗口时样式为 `WS_OVERLAPPED | WS_SYSMENU`（+ 可缩放时 `WS_THICKFRAME | WS_MAXIMIZEBOX`、
  可最小化时 `WS_MINIMIZEBOX`），系统给没有 `WS_POPUP` 的顶层窗口补上 `WS_CAPTION`；GPUI 只在 `WM_NCCALCSIZE`
  （`gpui-pre-windows-0.3.9/src/events.rs:764` `handle_calc_client_size`）把客户区顶边拉到窗口顶，标题栏样式仍在。
  DWM 因此仍在右上角布置标题栏按钮：`DWMWA_CAPTION_BUTTON_BOUNDS`（attr 5）实测收纳箱 74×44、固定模式主窗口 62×44
  （TOOLWINDOW 只有关闭钮）、设置窗口 185×37（三个钮）。
- 按钮画在客户区内容**之下**（DirectComposition 透明窗口 `WS_EX_NOREDIRECTIONBITMAP`），只在客户区半透明处透出：
  收纳箱底色、深色主题主窗口底色 → 可见；浅色主窗口、设置等不透明窗口 → 被盖住。
- 激活时 DWM 画成红底（`WM_NCACTIVATE(TRUE)` 模拟可复现）；命中测试由 GPUI 返回 `HTCLIENT`，**只是视觉问题**，点不到。
- **实验（debug 构建，隔离数据目录）**：

  | 实验 | 结果 |
  |---|---|
  | 收纳箱原样（PrintWindow `PW_RENDERFULLCONTENT`） | 右上角小 ×，与用户截图一致 |
  | `WM_NCACTIVATE(TRUE)` | × 后出现暗红底 |
  | 运行时去掉 `WS_SYSMENU` + `SWP_FRAMECHANGED` | × 消失，按钮区宽度 0，圆角 / 描边不变 |
  | 浅色主窗口（APPWINDOW 或 TOOLWINDOW） | 看不到（内容不透明） |
  | 收纳箱改成 APPWINDOW + 最小化 / 最大化 | 仍只透出一个 ×（更大） → 普通模式深色主窗口同样受影响 |
  | 普通模式主窗口去掉 `WS_SYSMENU` 后标题栏双击（`WM_NCLBUTTONDBLCLK` + `HTCAPTION`） | 仍最大化 / 还原 |
  | 去掉 `WS_SYSMENU` 后 Alt+F4（`WM_SYSKEYDOWN` VK_F4 + Alt） | 仍关闭窗口 |
  | 单击任务栏按钮最小化 / 还原 | **未测成**（Win11 任务栏按钮 UIA 只有 ScrollItem，无法无鼠标触发）→ 放进 e2e 用真实点击验证 |

- 所有窗口都经 `crates/app/src/windows/mod.rs` 的 `open_window`（主窗口、编辑、子任务、已完成、设置、通知、收纳箱、gallery），
  建好后 `place_client_area` 在 `cx.spawn` 里按客户区校正一次 —— 唯一的公共切入点（实现时改名为 `adjust_native_window`，
  同时负责去掉 `WS_SYSMENU`）。
- 样式写操作已有 `platform/windows.rs::raw::update_long`（改位 + `SWP_FRAMECHANGED`）；`window_mode` / `embed` 只改
  `WS_EX_TOOLWINDOW/APPWINDOW`、`WS_MINIMIZEBOX`、`WS_THICKFRAME|WS_MAXIMIZEBOX`，不碰 `WS_SYSMENU`；GPUI 运行期只有
  `toggle_fullscreen` 会整体重写样式（本项目不用）。
- 唯一用到 GPUI `WindowControlArea` 的是 `ui/title_bar.rs` 的 `Drag`（→ `HTCAPTION`），没有用 Close / Min / Max。
- 测试为什么没发现：Linux 截图回归没有 DWM；Windows e2e 只断言四角取色与 Win32 状态。

## Requirements

1. **建窗后统一去掉 `WS_SYSMENU`（仅 Windows）**：在 `windows::open_window` 的建窗后处理里（与 `place_client_area` 同一个
   `cx.spawn` 延后块，App 未借用），对所有窗口执行；先于收纳箱 / 隐藏建窗窗口的显示（不闪 ×）。
2. 新增平台能力：`Hwnd::set_system_menu(bool)`（Windows：`raw::update_long(GWL_STYLE, WS_SYSMENU)`；fallback no-op），
   命名与文档风格对齐现有 `set_minimizable` / `set_resizable_frame`；只加实际用到的接口。
3. **不改** `WS_CAPTION` / `WS_MINIMIZEBOX` / `WS_MAXIMIZEBOX` / `WS_THICKFRAME`：外框 ↔ 客户区换算、窗口模式、任务栏最小化、
   双击最大化都依赖它们。
4. 不加 200ms 守护自愈（GPUI 运行期不改写 `WS_SYSMENU`；由 e2e 断言兜底）。
5. **文档**：`platform/mod.rs` 模块文档（"外框 ≠ 客户区"一节旁）写明为什么去掉 `WS_SYSMENU`；`.trellis/spec/pc-gpui/gpui-conventions.md`
   相应小节加约定（GPUI 窗口不带 `WS_SYSMENU`，否则 DWM 标题栏按钮会从半透明客户区透出）；`crates/app/README.md` 接口表补
   `set_system_menu`；`window-modes.md` / `desktop-fences.md` 的样式表如有逐位列出则同步。
6. **Windows e2e**（`pc-gpui/scripts/e2e-windows/`）：
   - `winlib.Win`：`flags()` 增加 `sysMenu`；新增 `caption_buttons()`（`DWMWA_CAPTION_BUTTON_BOUNDS`，返回宽度或矩形）。
   - 断言"无 `WS_SYSMENU` 且 DWM 标题栏按钮区宽度为 0"：W01–W12 每个窗口、S01 / S02 / S03 / S07（主窗口各模式）、F01（收纳箱）、
     N01（通知）。
   - S01 增加：标题栏双击 → `IsZoomed` → 再双击 → 还原。
   - 新增用例（S 组）：普通模式**单击任务栏按钮**（UIA 在 `Shell_TrayWnd` 下找名字以窗口所属应用开头的按钮取 BoundingRectangle，
     真实鼠标点击）→ 已激活时最小化 → 再单击 → 还原并在前台。
   - README 用例表同步。

## Acceptance Criteria

- [ ] 收纳箱、主窗口（普通 / 经典固定 / 嵌入桌面 × 浅色 / 深色半透明）右上角不再出现系统 ×；激活时没有红底
      （PrintWindow 或实机截图核对）
- [ ] 所有应用窗口 `WS_SYSMENU` 已去掉、`DWMWA_CAPTION_BUTTON_BOUNDS` 宽度为 0（e2e 断言）
- [ ] 普通模式主窗口：单击任务栏按钮可最小化 / 还原；标题栏双击可最大化 / 还原；Alt+F4 行为不变
- [ ] 窗口几何不变：W 组逻辑尺寸、F07 收纳箱几何一致性、S01 拖动落库照常通过
- [ ] `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、
      `cargo test --workspace --locked`（pc-gpui）全绿
- [ ] Windows e2e W / S / N / F 组通过（本机或 CI `pc-gpui-e2e-windows.yml`）

## Definition of Done

- 上述验收项全部满足；spec / README / 模块文档更新；e2e 用例表同步
- 用户实机（安装版升级到修复后的构建）确认收纳箱与主窗口的小 × 消失

## Decision (ADR-lite)

**Context**：GPUI 0.3.9 的无标题栏窗口保留了 `WS_CAPTION | WS_SYSMENU`，DWM 标题栏按钮在半透明 DirectComposition 客户区下透出。
gpui-kit 精确钉 `=0.7.3`，不能改 GPUI。

**Decision**：建窗后由应用统一去掉 `WS_SYSMENU`（对所有窗口，不只透明窗口），保留其余样式位。

**Consequences**：
- 失去系统菜单：Alt+Space、在标题栏右键弹出的系统菜单不再出现（自绘标题栏的窗口本来就不该有；旧 Tauri 版拖动区也没有）。
- Alt+F4、标题栏双击最大化已实测不受影响；任务栏单击最小化靠 e2e 验证 —— 若失败，退路是普通模式主窗口保留 `WS_SYSMENU`
  另找办法（如去掉 `WS_DLGFRAME` 让窗口没有标题栏，需重核外框 ↔ 客户区换算），届时回到 Plan。
- 不透明窗口去掉后无可见变化，但保持"所有 GPUI 窗口都没有系统标题栏按钮"这一不变量，以后改成透明也不会复发。

## Out of Scope

- 修改 / 升级 gpui-pre、gpui-kit
- 去掉 `WS_CAPTION`、改用 `WS_POPUP`、DWM 非客户区渲染策略（`DWMWA_NCRENDERING_POLICY`）等更大改动
- `WS_SYSMENU` 的 200ms 守护自愈
- 窗口上沿那条 DWM 细线等其它非客户区外观问题（另行观察）

## Technical Notes

- 关键文件：`pc-gpui/crates/app/src/windows/mod.rs`（`open_window` / `adjust_native_window`，原 `place_client_area`）、`platform/mod.rs`、`platform/windows.rs`
  （`raw::update_long`、`set_minimizable` 的写法）、`platform/fallback.rs`、`services/fences.rs`（隐藏建窗 → 定位 → 嵌入 → 显示的顺序）
- e2e：`pc-gpui/scripts/e2e-windows/winlib.py`（`Win.flags`）、`run_e2e.py`（`WINDOW_CASES` / `group_windows`、`group_states` s01–s07、
  F01、N01）、`uia-find.ps1`（UIA 查找的写法可参考）
- 实验脚本（会话 scratchpad，不入库）：`probe-caption.ps1`（样式 + DWM 按钮区）、`printwin.py`（PrintWindow 抓单窗）、
  `winctl.py`、`dblclick-test.py`、`taskbar-test.py`
- Windows 上编译检查：`cargo clippy --workspace --all-targets --locked -- -D warnings`（在 `pc-gpui/` 下，本机即 Windows 目标）

# 桌面模式：主窗口嵌入桌面图标层之上，Win+D 不最小化

## Goal

新增第三种窗口模式「桌面模式」（普通 / 固定 / 桌面），把主窗口"嵌入"Windows 桌面：
位于桌面图标之上、所有应用窗口之下，按 Win+D / Win+M / 任务栏"显示桌面"时不被最小化、
不被盖住；保持现有的透明背景（深色主题下 rgba 半透明底透出壁纸）。

背景：现有「固定模式」只是给顶层窗口加 `WS_EX_TOOLWINDOW`，Win+D 仍会把它最小化，
`lib.rs` 里的 200ms 轮询线程再把它还原（`restore_if_minimized`），用户看到的是"消失再弹回"，
不是嵌入效果。

## What I already know

### 用户决定（2026-09-07）

* 嵌入位置：**桌面图标之上**（不是壁纸层 WorkerW 之下）
* 作为**独立的第三种模式**，保留现有固定模式及其贴边隐藏 / 唤起 / 唤起置顶功能不变
* **不接受实色底**：透明背景是硬性要求，做不出透明就不算完成

### 现有实现（repo 事实）

* 固定模式状态：`IS_FIXED_MODE` 原子量 + settings `is_fixed` + `screen_configs.is_fixed`
  （`pc/src-tauri/src/commands/window.rs`、`db/models.rs`、`commands/data.rs`）
* 固定模式入口：TitleBar 锁按钮（`components/TitleBar.vue`）、托盘 CheckMenuItem `toggle_fixed`
  （`lib.rs`，emit `tray-toggle-fixed` → `appStore.toggleFixedMode`）
* 前端 `WindowMode = 'normal' | 'fixed'`（`types/app.ts`），`appStore.windowMode` / `isFixed`
* 固定模式绕开 tao 的 flag 通路直接用 Win32 改样式 / Z 序，因为 tao `WindowFlags::apply_diff`
  会整体覆写 `GWL_EXSTYLE`（见 `.trellis/tasks/archive/2026-08/08-18-fix-taskbar-icon-flash/prd.md`）；
  `reassert_fixed_window_state` 通过 `run_on_main_thread` 排队兜底
* 200ms 轮询线程（`lib.rs` setup）：固定模式下 `restore_if_minimized` + `tick_auto_hide`
* 主窗口：`tauri.conf.json` `decorations:false, transparent:true, shadow:false`；`lib.rs`
  给它打 `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`
* 主窗口打开编辑 / 设置 / 已完成窗口时传 `parent: appWindow`（`views/MainView.vue`
  414 / 484 / 548 行），Windows 上即 owner 关系
* `is_fixed` 走 settings 导出 / 导入 / WebDAV 同步（`data.rs` `read_app_settings` /
  `write_app_settings`；`sync_cmd.rs` 把 `settings` 作为整体 `serde_json::Value` 传输）
* `windows` crate 0.58，已启用 `Win32_Foundation` / `Win32_UI_WindowsAndMessaging` /
  `Win32_Graphics_Dwm`；`SetParent` / `FindWindowExW` / `SendMessageTimeoutW` / `EnumWindows`
  / `GetParent` / `IsWindow` 都在 `Win32_UI_WindowsAndMessaging` 内，无需新依赖
* 版本：tauri 2.9.5 / tao 0.34.5 / wry 0.53.5 / webview2-com 0.38.2

### 主窗口对键盘的依赖

主窗口本身基本是鼠标操作（勾选、拖拽排序、点开详情）；标题编辑 / 描述编辑 / 设置全部在
独立顶层 WebView 窗口中完成，不受嵌入影响。

## Assumptions (temporary)

* 嵌入后窗口不可拖拽、不可缩放（与固定模式一致："固定在用户指定位置"）；要挪位置先退回普通模式
* 桌面模式下不需要贴边隐藏 / 唤起 / 唤起置顶（窗口永远在所有应用窗口之下，这三者逻辑上不成立）
* 桌面模式下窗口天然不在任务栏、不参与 Alt+Tab（子窗口 / owner 关系都满足），不需要额外样式
* 桌面模式与固定模式互斥：进入其一即退出另一个

## Open Questions

* （技术，原型解决）透明背景在"桌面子窗口"形态下是否可行；若不可行，是否有保持顶层窗口
  但同样免疫 Win+D 的方案（见 Technical Approach 的方案 B）

## Requirements (evolving)

1. 新增窗口模式 `desktop`，前端 `WindowMode = 'normal' | 'fixed' | 'desktop'`
2. 入口：
   * TitleBar 新增「桌面模式」图标按钮（Element Plus Icons，不用 emoji），与锁按钮并列；
     桌面模式下该按钮高亮，点击退出桌面模式回到普通模式
   * 托盘菜单新增 CheckMenuItem「桌面模式」，与「固定模式」互斥，勾选状态双向同步
3. 进入桌面模式：
   * 窗口停留在当前位置，位于桌面图标之上、所有应用窗口之下
   * Win+D / Win+M / 任务栏"显示桌面"后窗口仍可见、未最小化、未被桌面盖住；再次 Win+D 恢复其它窗口后仍在原位
   * 保持透明背景效果（深色主题 rgba 底 + 圆角），与普通模式观感一致
   * 鼠标操作（勾选完成、点开详情、拖拽排序、右键菜单、滚动）正常
   * 不可拖拽移动、不可缩放（同固定模式）
   * 从主窗口打开的编辑 / 设置 / 已完成窗口是普通顶层窗口，能正常显示在最前并获得键盘焦点
4. 退出桌面模式：窗口恢复为普通顶层窗口，位置不变，任务栏 / Alt+Tab 恢复正常
5. 持久化：settings 新键 `is_desktop`（v27 migration）+ `screen_configs.is_desktop` 列；
   启动时按当前屏幕配置恢复桌面模式（先定位、后嵌入）；纳入导出 / 导入 / WebDAV 同步
   （`AppSettings` 加 `#[serde(default)]` 字段，旧备份兼容）
6. 健壮性：Explorer 重启 / 崩溃后桌面宿主窗口失效，需在轮询中检测并自动重新挂载；
   轮询线程在桌面模式下跳过 `restore_if_minimized` / `tick_auto_hide`
7. 兼容 Windows 10 与 Windows 11（含 24H2 之后的桌面窗口层级变化）

## Acceptance Criteria (evolving)

* [ ] 桌面模式下按 Win+D，窗口全程可见、`IsIconic == false`，且未被桌面盖住；再按 Win+D 恢复后仍在原位
* [ ] 桌面模式下按 Win+M、点击任务栏最右侧"显示桌面"，表现同上
* [ ] 桌面模式下窗口在桌面图标之上（图标被窗口区域遮挡）、在任意普通窗口之下（把资源管理器窗口拖到其上方能盖住它）
* [ ] 桌面模式下透明背景与普通模式一致：壁纸透出、圆角保留，无黑底 / 无白底 / 无残影
* [ ] 桌面模式下鼠标勾选完成、点开待办详情、拖拽排序、滚动均正常
* [ ] 桌面模式下从主窗口打开编辑窗口，编辑窗口在最前且可以输入文字
* [ ] 桌面模式下任务栏无图标、Alt+Tab 不出现
* [ ] 退出桌面模式后窗口成为普通顶层窗口，位置不变，任务栏 / Alt+Tab 恢复
* [ ] 桌面模式与固定模式互斥切换正确，TitleBar 与托盘勾选状态一致
* [ ] 重启应用后自动恢复桌面模式且位置正确（多屏、主屏不在最左侧的负坐标场景）
* [ ] 任务管理器重启 explorer.exe 后 5 秒内窗口自动重新挂载到桌面
* [ ] 导出 → 导入、WebDAV 上传 → 下载应用后 `is_desktop` 保留；导入旧版备份（无该字段）不报错
* [ ] `cargo check` / `cargo test` / `npm run build`（vue-tsc）通过

## Definition of Done (team quality bar)

* Rust 侧 Win32 逻辑有单元测试覆盖纯函数部分（坐标换算、宿主查找结果判定）
* `cargo check` / `cargo test` / `npm run build` 通过
* CLAUDE.md 更新：窗口模式说明、迁移版本 v27、同步覆盖范围
* 版本号（package.json / tauri.conf.json / Cargo.toml）按发布流程递增

## Out of Scope (explicit)

* 桌面模式下拖拽移动 / 缩放窗口（先退普通模式再调整）
* 桌面模式下的贴边隐藏 / 唤起 / 唤起置顶
* 嵌入到壁纸层（图标之下）
* 实色底回退方案
* macOS / Linux 上的等价实现（命令在非 Windows 平台返回错误或 no-op）
* 重写现有固定模式

## Technical Approach

### 方案 A：SetParent 挂到桌面图标宿主窗口（用户描述的"嵌入"）

* 找到承载 `SHELLDLL_DefView` 的宿主（Win10 为 `Progman` 或经 `0x052C` 分裂出的 `WorkerW`；
  Win11 24H2 起层级有变化，需两套查找逻辑）
* `SetParent(hwnd, host)` 前把 `WS_POPUP` 换成 `WS_CHILD`；挂载后 `SetWindowPos` 把它排到
  `SHELLDLL_DefView` 之上（同级 Z 序）
* 坐标：子窗口 `SetWindowPos` 用父窗口客户区坐标，需按宿主的屏幕原点换算；`GetWindowRect`
  仍返回屏幕坐标，所以持久化的位置不用改
* 风险：DWM 透明是否对子窗口生效（**硬性要求**）；跨进程父子关系共享输入队列；tao 的
  `apply_diff` 会把 `WS_CHILD` 抹回 `WS_POPUP`，需要与现有 `reassert_fixed_window_state` 同类的兜底

### 方案 B：保持顶层 + owner = Progman + Rainmeter 式"显示桌面"检测

* 窗口仍是顶层窗口，透明路径不变
* `SetWindowLongPtr(GWLP_HWNDPARENT, progman)` 让它永远压在桌面正上方；去掉 `WS_MINIMIZEBOX`
* 轮询检测"显示桌面"把桌面宿主抬到顶层的情况，把自己重新插到宿主正上方
* 风险：Win+D 对无最小化框 / 有 owner 的工具窗口到底是"最小化"还是"只被盖住"，需实测

### 决策路径

1. 先做原型（spike）：在真实主窗口上分别验证 A 与 B 的 Win+D 免疫性、透明、鼠标输入
2. A 透明可行 → 采用 A；A 透明不可行而 B 免疫 Win+D → 采用 B；两者都不行 → 回到用户处报告
3. 原型结论写回本 PRD 的 Decision 节，再进入正式实现

## Research References

* `research/win32-desktop-embed.md` — 桌面窗口层级、Lively / Rainmeter 做法、Show desktop 行为（进行中）
* `research/tao-wry-transparency.md` — tao / wry 透明实现与子窗口透明可行性（进行中）

## Decision (ADR-lite)

待原型验证后填写。

## Technical Notes

* 相关文件：`pc/src-tauri/src/commands/window.rs`、`pc/src-tauri/src/lib.rs`、
  `pc/src-tauri/src/db/{models,migrations}.rs`、`pc/src-tauri/src/commands/data.rs`、
  `pc/src/stores/appStore.ts`、`pc/src/types/app.ts`、`pc/src/components/TitleBar.vue`、
  `pc/src/views/MainView.vue`
* 新增 settings 键必须走 CLAUDE.md「维护检查清单」12 项
* 桌面模式下 MainView 打开子窗口不要再传 `parent: appWindow`（owner 会被解析成桌面宿主）
* 轮询线程已存在（200ms），桌面模式的宿主存活检测 / 重挂载复用它，不新开线程
* 自动化验证手段：`(New-Object -ComObject Shell.Application).ToggleDesktop()` 触发显示桌面；
  PowerShell + Win32 查询 `IsIconic` / `GetParent` / `GetWindowRect`；截屏比对透明效果

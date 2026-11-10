# 第 5 步 B 部分（窗口、托盘与设置）实现说明与待同步的文档

> 范围：`pc-gpui/crates/app`。托盘删「固定模式」、「重置位置」作用于全部清单 / 日历窗口、整个应用的提示不再落进隐藏的窗口、
> 设置 → 屏幕配置按组合汇总。本步没有改 `CLAUDE.md` 与 `.trellis/spec/**`，下面是第 6 步要同步进去的要点。没有改 core。
> 提交：`fd53d81`（删托盘固定模式）、`f40f00c`（重置位置）、`3b4f1e3`（提示）、`6b6cedf`（屏幕配置页）。

## 1. 托盘「固定模式」删除（PRD R9）

- `services/tray.rs`：删菜单项、`menu_id::TOGGLE_FIXED`、`tray::set_fixed_checked`、`Tray.fixed`；`window_mode::apply_mode` 不再给
  `DEFAULT_LIST` 回写勾选。菜单现在：添加待办项、新增待办清单、显示日历（勾选）、分隔、收纳箱三项、分隔、重置位置、打开设置、
  开机自启动（勾选）、分隔、退出。macOS 菜单栏图标是同一份菜单；`services/mac_menu.rs` 没有固定模式入口，不用改。
- 锁定只剩各窗口标题栏的锁按钮（`window_mode::toggle_fixed(cx, key)`）。

## 2. 托盘「重置位置」（PRD R9：全部错开、解锁、取消隐藏）

实现：`window_mode::reset_all`；纯函数 `window_mode::machine::{reset_plan, reset_size, reset_rect}`；`window_mode::store::save_all`；
`block_windows::all_keys`。

- **排列**：`all_keys` = 默认清单、其它清单按 id、日历最后（不管开没开过窗）。第 i 个从 `window_logic::reset_window_position`
  （主显示器左上、`尺寸 × 10% ÷ 缩放` 边距 = 以前主窗口的重置位置）向右下错开 i × 32 逻辑像素；**按这个窗口自己的尺寸**判断，
  再错开就超出显示器右边或下边时从第一个位置重来。尺寸：清单 380×600；日历与清单等高、宽 `calendar_window::logic::default_width(600)` = 596
  （不小于日历最小宽 420）。
- **决定：没开过窗的窗口——先写布局，再开窗**（两种办法的结合）。顺序：
  1. 重算屏幕组合 ID → `store::save_all` 一个事务把全部窗口重置后的几何（**未锁定**）写进当前组合（默认清单照旧登记 `screen_configs`；
     清单已被删的跳过）。
  2. `block_windows::prepare_wake_all`：所有清单清掉 `hidden`，没开过窗的现在开——`open_block` 同步读布局，读到的就是第 1 步写的，
     **直接开在重置后的位置、不带锁定**，不会先在旧位置出现再跳。
  3. 已开的窗口：清贴边状态、撤置顶、（要显示的）还原最小化、`set_client_rect`、隐藏着的摆好后再显示；锁定的 `set_fixed(false)`（切完保存），
     其余 `save_window_state`（读回实际客户区覆盖第 1 步的记录）。
  4. 按排列顺序 `wake`：**后面的在上面**（经典层叠，每个窗口的标题栏都露出来；获得焦点的是最后一个——日历开着时是日历）。
  - 理由：只"先开窗"的话，没开过窗的窗口会按旧布局（可能在屏幕外、带锁定）建出来再被挪走、解锁；只"写库清 hidden"的话窗口不会出现，
    不满足"取消隐藏"。先写库保证重启后仍是重置后的位置（含"显示日历"关着、从没建过的日历窗口）。第 1 步写库失败只记日志，第 3 步照样摆好、各自保存。
- **日历**：只在「显示日历」开着时显示（`should_wake`）；关着时已建的窗口在隐藏状态下摆好、解锁、保存，没建过的只写布局——下次显示时出现在重置后的位置。
  不改「显示日历」本身。
- macOS 先 `cx.activate(true)`（同 `bring_all_to_front`）；Linux 不能移动窗口，只按重置尺寸 `window.resize`。
- 单测：`machine.rs` 的 `reset_keeps_the_first_window_where_the_main_window_used_to_go`、`reset_wraps_around_instead_of_leaving_the_monitor`
  （改为带尺寸，补"按自己的尺寸换行"）、新增 `reset_plan_cascades_lists_then_the_calendar`；`store.rs` 新增 `reset_writes_every_window_including_ones_never_opened`。

## 3. 设置 → 屏幕配置（PRD R11）

- 仍每个屏幕组合一行（`screen_configs`，只由默认清单窗口保存时登记；收纳箱 / 其它窗口不新增行，同以前）。
- 第三行以前是单个主窗口的「固定模式 / 普通模式 | 位置: (x, y)」，多窗口下不成立（那只是默认清单窗口的）→ 改为这个组合保存了哪些窗口：
  `保存了 3 个清单窗口、日历窗口、2 个收纳箱的位置 | 2 个窗口锁定`（锁定只算清单 / 日历窗口，收纳箱的锁定不按组合记；没有有效尺寸的记录不算；
  都没有时 `没有保存窗口位置`）。打开设置窗口时后台读（`screen::load_layout_summaries`：`window_prefs::layouts_for_screen` +
  `fence::Placement::for_screen(..).layout_count()`），读出来之前占一行空白。删掉了 `config_meta`。
- 删除确认：`确定删除屏幕配置 "X" 吗？这个屏幕组合下保存的清单窗口、日历窗口与收纳箱的位置会一并删除。`（core 本来就连带删）。
- **决定：当前组合仍不能删**（按钮置灰 + `不能删除当前正在使用的屏幕配置`，沿用）。所以不存在"正在显示的窗口失去记录"：删了也会在窗口下次保存时写回，
  没有意义。注意"当前"是打开设置窗口时现算的组合 ID；窗口保存用的 `AppState.current_screen_config_id` 只在启动 / 重置位置时算——运行中插拔
  显示器后两者可能不同，这时删掉的可能正是窗口在写的那个组合：窗口不动，下次保存（拖动 / 锁定 / 关闭）重新写一条（含 `screen_configs` 行）。
- 卡片提示「应用会根据不同的屏幕组合自动保存和恢复窗口位置」仍准确，没改。
- 单测：`summary_counts_every_window_of_the_combination`、`summaries_follow_the_database_and_deletion`。

## 4. 整个应用的提示（新模块 `services/notice.rs`）

- 以前固定显示在默认清单窗口：自动同步失败（`auto_sync`）、加载待办失败（`AppStateEvent::Message`，原来只有默认清单窗口订阅）、
  导入成功（设置 → 数据）、托盘新建清单失败。现在都经 `notice::show / show_from / show_topic`。
- **目标窗口**（纯函数 `pick_target`）：只在看得见的清单 / 日历窗口里挑（`window_mode::is_shown`：已开窗、原生显示中、没最小化、没贴边藏起；
  Linux 开着就算）——当前激活的 → 发起操作的（导入：打开设置的窗口 `SettingsWindow.target`）→ 最近激活过的（`window_mode::on_activated`
  里记）→ 默认清单 → 其它按 key。显示前 `defer`（调用方可能在别的窗口的事件处理里）。
- **决定：都看不见时先存着**（最多 3 条；同类型同文字只留一条；同一主题只留最新），下一次有清单 / 日历窗口被激活时（托盘左键唤起、
  点任务栏、新开窗口……）等 300ms 再挑窗口补显示（托盘唤起会依次激活多个窗口，等最上面那个）；仍然都看不见就继续存着。
  **任何一次同步成功**（`services::sync::run_sync`，含手动同步）撤回还没显示的「自动同步失败」，避免恢复后再看到过期的失败提示。
  没有用系统通知：后台错误弹系统通知太打扰，也要新的平台路径。
- 窗口自己的操作结果（标题栏同步、改名、删除清单、行操作、切换固定模式失败）仍显示在那个窗口里，不经 notice。
  `MINI_TODO_DEBUG_OPEN=toast:…` / `confirm:…` 仍固定在默认清单窗口（截图用，没改）。
- 单测：`the_active_visible_window_wins`、`hidden_windows_are_skipped`、`pending_notices_are_deduplicated_and_capped`。

## 5. R9 核对（第 4 步有没有漏）

- 托盘左键（300ms 去抖 + `activate_app`）、单实例（mac 先 `cx.activate(true)`）、Dock `on_reopen`、mac 贴边隐藏中被激活 → 都是
  `window_mode::bring_all_to_front`：一致，没改。
- 唤起置顶按窗口：`wake` 里每个窗口 `ModeState::wake`（嵌入桌面只显示 + 聚焦，其余临时置顶 600ms 后按各自贴边状态决定撤不撤）。没改。
  新开的窗口（`prepare_wake_all` 刚建的）不经 `wake`，建窗时已显示并激活。
- 托盘「添加待办项」→ `editor::open(.., None, Edit)` → `DEFAULT_LIST_ID`，建到默认清单。没改。
- notes-ui §7「新清单的窗口记录」：core `save_layout` 没有属性行时按默认值补建（hidden = false），同步来的清单开窗保存几何后就有记录，行为正确，不用改。
- 已知 mac 限制（不改）：GPUI 只在**没有可见窗口**时回调 Dock 唤起；日历或任一清单窗口显示着时点 Dock 只激活应用，不会带出隐藏的清单窗口
  （与托盘左键不完全一样，只能靠菜单栏图标左键）。

## 6. 用户能感知到的变化

1. 托盘 / 菜单栏图标菜单没有「固定模式」了，锁定用各窗口标题栏的锁按钮。
2. 「重置位置」把所有清单窗口和日历窗口（开着日历时）从主显示器左上角层叠排开、全部解锁、隐藏的清单窗口重新出现并带到前面，重启后保持；
   日历窗口变成 596×600（与清单等高）。「显示日历」关着时日历不出现，下次打开时在层叠里的位置。
3. 自动同步失败、加载待办失败、导入成功、新建清单失败的提示出现在当前用着的清单 / 日历窗口里；窗口全藏起来时，下次唤起窗口才看到（同步已恢复的不再显示）。
4. 设置 → 屏幕配置每行显示这个组合保存了几个清单窗口、日历、几个收纳箱的位置与锁定数；删除确认说明会一起删除这些位置。

## 7. 本地无法验证的部分

- 没有装 `aarch64-apple-darwin`，`scripts/mac-check` 没跑；本步没有改 mac-check 覆盖的文件（`ns.rs`、`autostart.rs`、`unix_socket.rs`、
  `mac_icon.rs`、core `fence_fs/mac.rs`）。新增的 mac 专属代码只有 `reset_all` 开头的 `#[cfg(target_os = "macos")] cx.activate(true);`（GPUI 胶水，
  与 `bring_all_to_front` 同一写法）；`tray.rs` 的 `imp`、`mac_menu.rs` 在 Windows 上也编译，本地 clippy 覆盖。建议推 `claude/**` 跑
  `pc-gpui-ci.yml` 与 `pc-gpui-e2e-macos.yml`。
- 没跑 GUI：层叠的 z 序与焦点、隐藏窗口摆好后再显示有没有闪、mac 上 `orderOut:` 状态下 `setFrame:` 再显示、提示 300ms 后落在哪个窗口、
  设置页第三行的排版，都只有单测覆盖纯逻辑部分。

## 8. 第 6 步（文档、e2e）要改的

- **CLAUDE.md**：「窗口特殊功能 → 固定模式」去掉"/ 托盘「固定模式」"；「多屏幕位置记忆」的"托盘「重置位置」把主窗口放回…"改成第 2 节；
  「系统功能」托盘右键菜单列表（删固定模式，加新增待办清单 / 显示日历）；「核心架构」跨窗口通信表可加一行"整个应用的提示 → `services::notice`"；
  项目结构 `services/` 加 `notice`。
- **`spec/backend/window-modes.md`**：§2 签名删 `toggle_fixed` 的"托盘「固定模式」"、加 `reset_plan` / `reset_size` / `is_shown` / `store::save_all`；
  §3 Ordering rules 第 8 条（托盘勾选回写）删除；重置位置一段按第 2 节重写（含"先写库再开窗"、日历规则、层叠 z 序）；macOS 场景 §4 的
  "放回主显示器左上区域、尺寸复位"同步；§6 测试名（第 2 节）。
- **`spec/pc-gpui/gpui-conventions.md`** §2.9：整个应用的提示走 `services::notice`，不要再写死默认清单窗口；§2.15 激活一条加 `reset_all`。
- **`crates/app/README.md`**：服务表 `tray`（删固定模式勾选）、`window_mode`、新增 `notice`；设置窗口屏幕配置页。
- **mac e2e**（`scripts/e2e-macos/run_e2e.py` / README）：M07 的 `TRAY_ITEMS` 去掉「固定模式」（并补「新增待办清单」「显示日历」）；
  **M10 整个依赖菜单栏「固定模式」**，要改成点标题栏锁按钮（或删掉菜单栏部分），`settings.is_fixed` 也不再写（锁定在 `window_layouts.is_fixed`）。
- **Windows / Linux e2e**：没用托盘固定模式。可补：重置位置后所有清单窗口可见、`window_layouts` 当前组合下是重置后的矩形且 `is_fixed = 0`、
  隐藏的清单窗口被带出来、日历关着时不出现。
- **regress.py**：设置 → 屏幕配置的截图第三行文字变了（样例数据的两条屏幕配置），参考图要更新。

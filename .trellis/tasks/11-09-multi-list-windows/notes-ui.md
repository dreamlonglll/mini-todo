# 第 4 步（拆窗）实现说明与待同步的文档

> 范围：`pc-gpui/crates/app`。主窗口拆成清单窗口（每个清单一个）+ 日历窗口（单例），关闭 = 隐藏，清单名改名 / 删除，
> 托盘「新增待办清单」「显示日历」。本步没有改 `CLAUDE.md` 与 `.trellis/spec/**`，下面是要同步进去的要点（第 6 步合并时改）。
> 提交：`a35061c`（下拉项置灰、标题栏双击判断）、`cb032ed`（拆窗本体）。

## 0. 新的结构一览

| 位置 | 内容 |
|---|---|
| `windows/list_window.rs` + `list_window/` | 清单窗口（原 `main_window`）：`ListWindow::new(list_id, ..)`；`title_bar`（版本号只在默认清单窗口）、`name`（双击改名 / 右键菜单 / 删除确认，`NameEditor`、`rename_outcome`、`delete_message`）、`list` / `quadrant` / `todo_item` / `logic`；`DEFAULT_WIDTH/HEIGHT` 380×600、`MIN_WIDTH/HEIGHT` 320×400 |
| `windows/calendar_window.rs` + `calendar_window/` | 日历窗口：`title_bar`（月份导航、今天、日 / 夜、锁、设置；按钮复用 `list_window::title_bar` 的 `theme_button` / `lock_button` / `settings_button`）、`grid`（原 `main_window/calendar.rs`）、`logic`（网格 / 跨天条 / `first_rect` / `default_width`，`MIN_WIDTH/HEIGHT` 420×480） |
| `services/block_windows.rs` | 清单 / 日历窗口的生命周期（见 §2）：`start`、`request_close`、`prepare_wake_all` / `should_wake`、`ensure_default_list`、`show_list`、`set_calendar_visible`、`create_list`、`reconcile` / `plan_reconcile`、`new_list_rect`；5 秒变更轮询与检查更新也在这里 |
| `windows/mod.rs` | `WindowKind::{List, Calendar}`（删 `Main`）；`WindowManager::{block_window, default_list, block_key_of, any_block_has_modal_child}`（删 `main`）；`DebugOpen::{List(id), NewList}` |
| `windows/block.rs` | `BlockDrag::is_double_click_release`（`TitlePress.double` + `last_press_dragged`，同收纳箱） |
| `ui/ep_dropdown.rs` | `EpDropdownItem::disabled(bool)` |
| `windows/editor` | `editor::open_new(cx, parent, list_id)`；`EditorWindow.new_list_id`；`build_create_request(form, list_id)` 总写 `Some(list_id)` |
| `windows/completed.rs` | `completed::open(cx, parent, list_id)`、`completed_in_list`（按有效清单） |
| `services/tray.rs` | 菜单项 `new_list`「新增待办清单」、`show_calendar`「显示日历」（勾选）；`tray::set_calendar_checked` |

## 1. 登记与模态（`spec/pc-gpui/gpui-conventions.md` §2.4 / §2.9、`crates/app/README.md` §4.3）

- 清单窗口 `WindowKind::List`、key `list:<清单 id>`；日历 `WindowKind::Calendar`、key `calendar`（= core 窗口属性表的 key，`BlockKey::registry_key`）。
  `WindowManager::block_window(cx, key)` 按 key 找（含原生隐藏着的），`default_list(cx)` = 默认清单窗口。
- "打开方是主窗口"的规则推广到**任一清单 / 日历窗口**：子窗口关闭后 `reload_todos`，设置窗口关闭后重读设置 + `auto_sync::start`。
  §2.9 的 `main_has_modal_child` 改为 `WindowManager::any_block_has_modal_child`（任一清单 / 日历窗口开着模态子窗口时，5 秒轮询与同步后的刷新跳过）。
- 托盘「添加待办项」/「打开设置」、应用菜单「设置…」、导入成功 / 自动同步失败的提示、`DebugOpen` 的 toast / confirm 都用默认清单窗口。
- 设置窗口的「深色主题」切的是打开它的窗口（清单或日历；托盘 / 应用菜单打开时是默认清单窗口）——3453b32 已有，这里只是打开方可以是日历窗口了。

## 2. 生命周期（`spec/backend/window-modes.md` 新增一节「清单 / 日历窗口的生命周期」；CLAUDE.md「窗口一览」「系统功能」）

| 时机 | 行为 |
|---|---|
| 启动 `block_windows::start`（`lib.rs` 里替代 `main_window::open`） | 迁移 ui-state → **同步**读清单、窗口属性、运行时偏好、屏幕组合（建窗前就要知道深色 / 视图）→ 默认清单窗口（总是显示，清掉它的 `hidden`）→ 「显示日历」开着时开日历 → `reconcile` 开其它没隐藏的清单 → 观察 `lists_revision` → 5 秒轮询 → 检查更新。**隐藏的窗口启动时不建**，第一次显示时才建 |
| 关闭（Alt+F4 / 系统关闭、macOS ⌘W） | `request_close`：有模态子窗口时不关、把子窗口带到前台；否则保存几何、写 `hidden = true`（日历 = 关掉「显示日历」并回写托盘勾选）、`platform::hide_window`，**不注销**（`window_mode` / 吸附组照常找得到；隐藏的窗口不算吸附目标）。Linux 不能程序化隐藏：真的关窗，最后一个窗口关掉就退出 |
| Windows 退出 | 去掉了"所有窗口都关了就退出"（以前在主窗口的 `on_closed` 里）：只有托盘「退出」退出，同 macOS 的 ⌘Q |
| 托盘左键 / 单实例 / Dock（`window_mode::bring_all_to_front`） | 先 `prepare_wake_all`：所有清单都要显示——开过窗的清掉 `hidden`、没开过窗的现在开窗（新开的不再唤起）；日历只在「显示日历」开着时唤起（`should_wake`）。macOS 上贴边隐藏中的窗口被激活（`on_activated`）也走这里，所以也会带出隐藏的清单窗口（PRD：Dock 唤起 = 托盘左键） |
| 清单列表变化（`AppState::lists_revision`，同步 / 5 秒轮询 / 本机新建改名删除 / 窗口属性变化都会加一） | `observe` 里 `defer(reconcile)`：清单已不存在的窗口关掉（先关它的模态子窗口；默认清单永不关）；没开窗的清单里，本机没有窗口记录（同步 / cloud 新建）或记录是显示中的开窗，记录是隐藏的不开。同步来的新清单：主显示器工作区里从右上角找不与显示中的吸附组窗口重叠的空位（`fence::default_fence_rect` 同一算法，边距 24、步长 32 逻辑像素），380×600、未锁定、浅色、列表视图（`WindowPrefs` 默认值） |
| 「显示日历」（设置 → 常规、托盘勾选、调试 `calendar`） | `set_calendar_visible`：写日历的 `hidden`、回写托盘勾选；显示时没建过就建（当前屏幕组合没有布局 → `calendar_window::logic::first_rect`：贴默认清单窗口右侧、等高，宽度按高度让日格大致是方的；右边放不下放左边，再放不下夹回工作区），建过就 `bring_to_front`；隐藏时保存几何 + 原生隐藏（Linux 关窗） |
| 托盘「新增待办清单」（调试 `list:new`） | `create_list`：`AppState::create_list(None)`（后台按库里最新的清单自动取名"清单 / 清单 2…"（`state::next_list_name`），同一个后台任务建好显示中的窗口记录）→ 光标所在显示器上找空位开 380×600 的窗口 → 进入改名。**时序**：清单建好那一刻 `lists_revision` 加一，`reconcile` 先于新建的回调开窗；所以位置事先放进 `BlockWindows.creating`，`reconcile` 对"有窗口记录、没开窗"的清单用它，回调再 `open_list(.., At(rect))`（已开则直接拿视图）后 `begin_rename` |

- 窗口的系统标题：默认清单窗口固定 `Mini Todo`（e2e 按它找窗口），其它清单窗口 = 清单名（改名后跟着改），日历窗口 `日历`。
- 5 秒变更轮询从主窗口挪到 `block_windows`（整个应用一个）；`AppStateEvent::Message`（加载待办失败）只在默认清单窗口里显示。

## 3. 清单窗口标题栏（PRD R6；`crates/app/README.md` 模块地图、CLAUDE.md「核心功能 → 待办管理」）

- **双击清单名改名**：清单名的 `on_mouse_up` 问 `BlockDrag::is_double_click_release(position)`（按下时 `click_count >= 2` 且上一次按下没拖动窗口；松手离按下点不超过 3px；拖动开始时按下已清掉）。清单名不 `occlude()`，在它上面按下照样拖窗口。锁定时不挂拖动，双击照样能改名。
- **改名**：`EpInput`（small，宽 `theme::main_window::LIST_NAME_INPUT_WIDTH` = 160、放不下时收缩，限 50 字）；Enter / 失焦提交、Esc 放弃（同收纳箱：输入框外层 `on_key_down`）；
  `rename_outcome`：`lists::normalize_name` 报错（空名 / 超长）或与其它清单重名（`names_conflict`，不区分大小写）→ `message::warning` 提示 core 同款文字、保持原名；去首尾空白后与原名相同 → 不写库；只改大小写算改名。改名中默认清单窗口的版本号让位（不显示）。
- **右键清单名** → `EpDropdownMenu`（清单名下方左对齐、带箭头）：「重命名」「删除清单」，默认清单的删除 `disabled`（禁用色、not-allowed、点击无效）。菜单是浅色的（同收纳箱「更多」），不跟随窗口日 / 夜。
- **删除**：后台 `lists::todo_count` → 确认框「删除清单」：`确定删除清单「X」吗？N 项待办将移到「<默认清单现在的名字>」。`（N = 0 时只有第一句；确定按钮「删除」）→ `AppState::delete_list`（core `lists::delete`）→ 成功后 `lists_revision` 变化，对账关窗；失败在本窗口提示 `删除清单失败：…`。
- 「已完成」按钮只在本清单有已完成时显示，打开的已完成窗口只列本清单（有效清单）的已完成；FAB 新建建到本清单。

## 4. 设置 / 托盘 / 调试开关

- 设置 → 常规「展示日历」：说明改为"开启后在独立的日历窗口中显示所有清单的待办"，开关直接调 `block_windows::set_calendar_visible`（与托盘「显示日历」同一个状态，设备本地）。
- 托盘菜单：添加待办项、**新增待办清单**、**显示日历（勾选）**、分隔、收纳箱三项、分隔、固定模式、重置位置、打开设置、开机自启动、分隔、退出。
  「显示日历」初值在 `tray::start` 时读 `AppState::calendar_visible()`（`block_windows::start` 在托盘之前）；点击时按 AppState 切换，`set_calendar_visible` 回写真实状态。
- `pc-gpui/scripts/README.md` 的 MINI_TODO_DEBUG_OPEN 表（本步没改）：`main` = 默认清单窗口（总会打开）；新增 `list:<id>`（显示清单窗口，没开过就开窗）、`list:new`（同托盘「新增待办清单」，进入改名）；
  `calendar` 改为"显示日历窗口（同设置「展示日历」/ 托盘「显示日历」）"；`completed` 打开默认清单的已完成。

## 5. 用户能感知到的变化

1. 日历是独立窗口：开日历不再加宽主窗口；日历窗口能单独拖动、缩放、锁定、切日 / 夜，与清单窗口 / 收纳箱互相吸附。
2. 关闭任何清单 / 日历窗口都只是隐藏（重启后保持隐藏，默认清单窗口除外）；Windows 上关掉所有窗口不再退出程序，托盘左键全部唤起。
3. 托盘多了「新增待办清单」「显示日历」；新清单窗口出现在光标所在显示器上并直接进入改名。
4. 双击 / 右键清单名可改名、删除清单；默认清单不能删（菜单项置灰）。
5. 「已完成」只看本清单；FAB 新建的待办在本清单；其它清单窗口不显示版本号。
6. 同步来的新清单自动开窗，同步删除的清单自动关窗。

## 6. 本地无法验证的部分

- **mac**：没有新增 `cfg(target_os = "macos")` 代码；mac 走的共用路径没有在 Mac 上跑过：⌘W → `block_windows::request_close` → `platform::hide_window`（`orderOut:`）；
  所有清单窗口都隐藏后点 Dock → `on_reopen`（GPUI 只在没有可见窗口时回调）→ `bring_all_to_front` 带出隐藏的窗口；新清单进入改名时普通窗口能否拿到 key（收纳箱是 NSPanel 才要那套宽限，清单窗口是普通窗口，没加）。
  `services/mac_menu.rs`、`services/tray.rs` 的 `imp`（Windows 也编译）本地 clippy 覆盖。建议推 `claude/**` 跑 `pc-gpui-ci.yml` 与 `pc-gpui-e2e-macos.yml`。
- **没跑 GUI**：新清单窗口的位置与改名聚焦、右键菜单位置、隐藏 / 唤起、日历初始位置都只有单测覆盖纯逻辑部分。
- 改名的 Esc：与收纳箱同一写法；如果 gpui-component 的输入框先把 Esc 处理成失焦，失焦会先提交（收纳箱同样如此），需要真机确认。

## 7. 第 5 步要注意的地方

- **「重置位置」**（R9：全部错开、解锁、取消隐藏）：启动时隐藏的窗口**没有建**，`window_mode::reset_all` 只遍历登记的窗口——要么先 `block_windows::prepare_wake_all` 把清单窗口都开出来（日历按「显示日历」），要么对没建的窗口直接 `window_prefs::save_layouts` 写布局并清 `hidden`。
- **托盘「固定模式」删除**：`window_mode::apply_mode` 里对 `DEFAULT_LIST` 回写 `tray::set_fixed_checked` 的那段与 `tray.rs` 的 `fixed` 勾选项一起删。
- **编辑表单「清单」下拉**：新建时预选 `EditorWindow.new_list_id`（FAB = 本清单，托盘 / 日历 = 默认清单）；编辑已有待办时只有用户改了选择才在 `UpdateTodoRequest` 带 `list_id: Some(..)`（契约 §12-6）。`build_create_request` 现在直接用 `new_list_id`，加下拉后改成下拉的值。
- **行右键「移动到」**：`AppState` 还没有包装 `core::todo::move_to_list` 的写方法（要像其它写操作一样 `mutation_seq += 1`、更新本地副本）。`EpDropdownMenu` 只有浅色，PRD 要求跟随窗口日 / 夜——加深色变体时把清单名的右键菜单一起换掉。
- **日历筛选下拉**：`AppState::set_calendar_filter` 已有；日历窗口按 `AppState::calendar_todos()` 显示，筛选变了自动重绘。
- **提示落在隐藏的窗口里**：自动同步失败、导入成功、加载待办失败的提示固定显示在默认清单窗口，它被隐藏时看不到；可以考虑改成当前激活的清单 / 日历窗口。
- **新清单的窗口记录**：同步来的清单开窗后，`save_window_state` 只写布局；是否补建 `window_prefs` 行取决于 core `save_layout`（没有行时下次启动仍按"没有记录"自动开窗，行为正确；用户关掉时 `update_prefs` 会补建）。
- **`services::sync::describe_sync_report` / `has_local_data_changes`** 已在 3453b32 计入清单（notes-core §2.2 的提醒已处理）。

## 8. 第 6 步（e2e / 文档）要改的

- Windows e2e：以前关掉主窗口（Alt+F4 / `WM_CLOSE`）进程会退出，现在只是隐藏、进程常驻——依赖"关窗即退出"的用例与收尾要改成托盘「退出」或 `taskkill`。窗口标题 `Mini Todo` 仍是默认清单窗口；日历窗口标题 `日历`。
- regress.py / shot.sh：`calendar` 场景不再是加宽的主窗口，而是单独的日历窗口（裁剪按窗口标题）；可加 `list:new` 场景截改名态。
- mac e2e M 组：主窗口用例改成多窗口；⌘W 后窗口还在（隐藏）、进程不退。
- 已知不稳定的 core 单测：`todo::tests::save_image_validates_extension_and_size_before_writing` 在整套并行跑时偶发失败（临时目录名按纳秒时间 + tag，与同 tag 的测试撞名），单独跑或重跑通过；与本步无关，没改。

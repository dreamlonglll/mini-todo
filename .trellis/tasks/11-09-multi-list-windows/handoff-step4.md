# 交接：第 4 步拆窗（2026-11-09 暂停，原代理上下文已满，由新代理接手）

**工作位置**：主检出 `D:\Git\mini-todo`，分支 `feat/multi-list`，不用 worktree。

## 必读
- `prd.md`：R4、R5、R6、R7 的窗口部分、R9、R10、R11
- `contract.md`：§3、§11、§12-6（编辑时只有用户真的改了清单才传 `list_id`）、§13
- `notes-window.md`（第 3 步底座）、`notes-core.md`（core API 与给 app 的提醒）、本目录的 `notes-ui.md`（如果原代理已经写过）

## 已提交（在 `d5d67f7` 之后）
- `2bb49b3`：`services/window_mode/store.rs` 改为读写 `core::window_prefs` 的布局表，对所有 `BlockKey` 生效。默认清单窗口保存时仍会 upsert 一行 `screen_configs`，这样屏幕配置页能看到新组合。不再写旧的全局几何键和 `is_fixed` 等
- `d1a953a`：修复滚动条滑块上显示缩放光标
- `3453b32`：
  - `AppState` 去掉全局的 `view_mode` / `is_dark_theme` / `show_calendar` / `subtask_expand_all`，改成 `window_prefs` 的内存副本，按 `BlockKey` 读写
  - `AppState` 维护清单列表和 `lists_revision`
  - 按有效清单过滤待办
  - `ui-state.json` 的展开全部迁移到 `list:1`
  - 同步报告计入清单的增删改
  - `main_window` 已改名为 `list_window`，日历挪到了 `calendar_window/grid.rs`

**这三个提交没有人复核过是否全绿。** 动手之前先在 `pc-gpui/` 下跑一遍门槛。

## 未提交的半成品（原代理停在改 `windows/mod.rs` 的 `WindowManager` / `DebugOpen`）
- 新文件：`services/block_windows.rs`、`windows/calendar_window.rs`、`windows/calendar_window/{logic,title_bar}.rs`
- 已修改：`services/{mod,snap}.rs`、`services/window_mode/mod.rs`、`windows/calendar_window/grid.rs`、`windows/list_window.rs` 和 `list_window/{list,logic,quadrant,title_bar,todo_item}.rs`

先用 `git diff` 和 `git status` 读懂这些改动，在它的基础上接着做。哪部分不成立就改掉，不要整体推倒重来。

## 剩余范围（第 4 步，以 prd 为准）
1. **WindowManager**：把 `WindowKind::Main` 拆成清单窗口和日历窗口，登记 key 自定。"打开方是主窗口"的逻辑推广到任一清单窗口或日历窗口，包括模态、遮罩、关闭后 `reload_todos`、设置关闭后重读设置。`main_has_modal_child` 推广成"任一清单窗口有模态子窗口"
2. **清单窗口标题栏**：
   - 双击清单名就地改名，照收纳箱 `TitlePress.double` 的写法，不要用 `on_click`；空名或重名时提示并恢复原名，锁定状态下也允许改名
   - 右键菜单「重命名 / 删除清单」，默认清单的「删除」置灰；删除前确认"N 项待办将移到「待办清单」"（`core::lists::todo_count`），确认后执行 `core::lists::delete`，再关窗
   - 版本号只在默认清单窗口显示
   - 「已完成」只看本清单：给 `completed::open` 加清单参数
   - FAB 新建的待办放进本清单
3. **日历窗口**：独立的单例窗口，原来的分栏、加宽逻辑全部删掉。标题栏放月份导航、今天、日 / 夜、锁、设置。如果已经读出了 `calendar_filter`，就按它筛选。第一次显示且没有布局时，贴默认清单窗口右侧、与它等高。设置 → 常规的「显示日历」控制日历窗口的隐藏属性
4. **生命周期**：
   - 启动时每个清单开一个窗口，默认清单窗口总是显示
   - 关闭 = 隐藏，并记住 `hidden`；关闭时不 `unregister`
   - Windows 上去掉"所有窗口都关了就退出"；Linux 保留"最后一个可见窗口关闭就退出"
   - 同步或 cloud 新建的清单，本机没有窗口记录时，自动在空位开一个 380×600 的窗口；同步删除的清单，关闭对应窗口
   - `bring_all_to_front` 会显示隐藏的窗口，并清除 `hidden`
5. **托盘**（只做这两项）：
   - 「新增待办清单」：名字自动取"清单 / 清单 2…"，在光标所在显示器上找空位，建好后直接进入改名
   - 「显示日历」：勾选项，和设置里的开关是同一个状态
6. **单测**：纯逻辑部分

不做（第 5 步）：编辑表单的清单下拉、行右键「移动到」、日历筛选下拉、托盘「固定模式」和「重置位置」的调整、屏幕配置页。

## 约束
- 不改 core 的同步合并逻辑：core 的 §13 修订另一个代理正在它的 worktree 里做，`Todo` 之后会多一个 `list_updated_at` 字段，合并时由协调者处理
- 不改 `CLAUDE.md`、`.trellis/spec/**`，文档要点写进 `notes-ui.md`
- 不 push、不跑 e2e、不启动 GUI
- 门槛：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`
- 遵守 gpui-conventions §2.2：`platform::Hwnd` 的原始调用只在 `cx.spawn` 里、`update` 之外做；阻塞调用走 `db::spawn_db`；颜色和尺寸只从 `theme.rs` 取
- 中文提交，`<type>: 摘要` 加 `- ` 列表、两者之间不空行，用 heredoc 写；按逻辑拆成几个提交，每个都要能编译、测试通过

## 交付说明
提交列表；每个范围项落在哪里、自行做了哪些决定；用户能感知到的行为变化；本地无法编译验证的 mac 代码；第 5 步要注意的地方；门槛结果。

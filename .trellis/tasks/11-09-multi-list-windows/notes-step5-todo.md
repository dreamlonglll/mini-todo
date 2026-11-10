# 第 5 步 A 部分（待办相关的交互）实现说明与待同步的文档

> 范围：`pc-gpui/crates/app`（core 没改，也没加 API）。编辑表单「清单」下拉、待办行右键「移动到 ▸」、`EpDropdownMenu` 深色变体、
> 日历清单筛选。B 部分（托盘、重置位置、屏幕配置页、提示路由）不在这里。本步没有改 `CLAUDE.md` 与 `.trellis/spec/**`，
> 下面是要同步进去的要点（第 6 步合并时改）。

| 提交 | 内容 |
|---|---|
| `4c74b15` | `theme::ep_menu(dark)`；`ep_popper` 深色外框 / 箭头、`Anchor::at`、`Placement::RightStart`；`EpDropdownMenu` 的 dark / width / anchor / on_hover 与菜单项 submenu / note / checked；清单名右键菜单跟随日 / 夜 |
| `67838e8` | 待办行右键「移动到 ▸」（`list_window/row_menu.rs`）、`AppState::move_todo_to_list` |
| `aebd04a` | 编辑表单「清单」下拉、只读详情「所属清单」 |
| `edac75e` | 日历标题栏清单筛选（`calendar_window/filter.rs`）、`AppState::calendar_filter`、新增 EP `Filter` 图标 |

## 1. 落点

| 需求 | 位置 | 单测 |
|---|---|---|
| 「清单」下拉（「四象限」下面） | `windows/editor/mod.rs`（`list_select: EpSelect<i64>`、`list_id` = 选择、`original_list_id` = 预选、`known_lists`、`on_app_state_changed`、`reconcile_list_choice`）、`form.rs` | `editor::tests::list_choice_falls_back_when_its_list_is_deleted`、`list_options_keep_the_list_order` |
| 只在改了清单时带 `list_id`（契约 §12-6） | `editor/save.rs`：`FormValues.list_id`、`Originals.list_id`、`list_change(selected, original)`；`build_create_request(form)` 用下拉的值 | `save::tests::update_sends_the_list_only_when_the_user_changed_it`、`create_request_…` |
| 只读详情「所属清单：<名称>」 | `editor/view.rs`（徽章行，象限徽章之后、提醒徽章之前；`List` 图标） | `view::tests::list_badge_names_the_list` |
| 行右键「移动到 ▸」 | `windows/list_window/row_menu.rs`（`RowMenu`、`move_targets` / `move_item` / `target_item` / `parse_target`）；`todo_item.rs` 的 `render_row` 挂右键；`list_window.rs` 渲染 / 关闭 | `row_menu::tests::*` |
| 移动的写操作 | `state.rs`：`AppState::move_todo_to_list`、`set_local_list_id` | `state::tests::moving_a_todo_takes_it_out_of_its_list_at_once_and_can_be_undone` |
| `EpDropdownMenu` 深色变体 | `theme.rs`：`EpMenuColors` / `ep_menu(dark)`；`ui/ep_popper.rs`：`frame_for(dark)`、`PopperOptions.dark`；`ui/ep_dropdown.rs`：`.dark(bool)` | `theme::tests::menu_colors_follow_the_window_theme`、`ep_dropdown::tests::*`、`ep_popper::tests::*` |
| 日历清单筛选 | `windows/calendar_window/filter.rs`（按钮、下拉、`next_filter` / `menu_items` / `parse_command`）；`title_bar.rs` 放按钮；`state.rs`：`calendar_filter()`、`effective_calendar_filter`、`set_calendar_filter` 写入计数、`delete_list` 清理内存筛选 | `filter::tests::*`、`state::tests::calendar_filter_drops_deleted_lists_and_falls_back_to_all` |

## 2. 行为细节（要同步进 spec / README / CLAUDE.md 的要点）

### 2.1 编辑表单「清单」下拉（CLAUDE.md「核心功能 → 待办管理」、`crates/app/README.md` 编辑窗口一节）

- 选项 = `AppState::lists`（按 `sortOrder`、id），宽度同「提前提醒」。新建预选打开方的清单（清单窗口 FAB = 本清单；托盘「添加待办项」、
  调试开关 = 默认清单），打开方的清单已不存在时预选默认清单；建到选中的清单。
- 编辑已有待办：预选它的有效清单（先按本地副本，加载完成后 `fill_form` 按库里的再定一次）。保存时 `list_id = list_change(选择, 预选)`：
  选择与预选相同就是 `None`（改到别的再改回来也算没改），归属未知的待办没改就不会写成 1。新建后为写重复设置补的那次更新不带清单。
- 观察 `AppState`：清单增删改名时刷新选项；**选中的清单被删** → 回到预选；**预选的清单被删** → 预选与（没改过的）选择都回到默认清单
  （它的待办已移到 / 兜底到默认清单，不算修改）。只读详情时选择与预选跟着待办现在的有效清单（别的窗口「移动到」、同步改了归属），
  所以点 [编辑] 后下拉显示的是最新归属。
- 未保存修改（Esc 确认）只看"清单改没改"（快照里放 `list_change()`）。
- `EditorWindow.new_list_id` 字段删了：`EditorWindow::new(todo_id, new_list_id, ..)` 的参数还在，只用来定预选。

### 2.2 待办行右键「移动到 ▸」（CLAUDE.md「核心功能」、spec `gpui-conventions.md` §2.5 弹层）

- 列表与四象限的待办行（`render_row` 共用）右键 → 在按下位置弹出菜单（`Anchor::at`，下方放不下翻到上方、水平夹回窗口）。
  唯一一项「移动到」（`Rank` 图标 + 级联箭头）；**悬停或点击**展开子菜单，子菜单贴在主菜单右边（`Placement::RightStart`，间距 4，
  右边放不下翻到左边，再放不下夹回窗口），按清单顺序列出其它清单（不含本清单；宽 120~220 按最长名估算，长名截断；最高 320 滚动）。
  展开后一直开到整个菜单关闭（主菜单只有一项，不做"移出收起"）。
- 只有默认清单（没有其它清单）时「移动到」置灰、不画箭头、右侧显示"没有其它清单"，悬停 tooltip 同文案。
- 选中 → `AppState::move_todo_to_list`：`mutation_seq += 1`、**先改本地副本**（本窗口里这条随即消失、目标清单窗口里出现）、后台
  `core::todo::move_to_list`、成功用库里的结果替换本地项、失败把归属改回去（本地没被别处改过时）并在本窗口提示 `移动待办失败：…`。
  `sortOrder` 不变（core 只改归属；在目标清单里按原来的 `sortOrder` 排）。
- 关闭：点菜单外（点在子菜单上不算主菜单外：主菜单的 `on_dismiss` 用 `window.mouse_position()` 排除子菜单外框；子菜单的锚点是主菜单
  外框，点主菜单不算子菜单外）、选中、这条待办已不是本清单的未完成待办（`on_state_changed` 检查：被移走 / 完成 / 删除 / 同步改了归属）、
  切换列表 / 四象限、窗口失活。拖拽排序中不开；清单名菜单与行菜单互斥。
- 窗口失活时清单名的右键菜单现在也收起（第 4 步没有）。

### 2.3 `EpDropdownMenu` / `ep_popper` 的新能力（spec `gpui-conventions.md` §2.7 / §2.8、`ui/mod.rs` 表已改）

- `theme::ep_menu(dark) -> &EpMenuColors`：浅色 = 原来的浅色弹层（`ep()` 的白底 / `#E4E7ED` / `#606266` / `#ECF5FF` / 阴影 .12）；
  深色 = Element Plus 暗黑模式 `theme-chalk/dark/css-vars.css` 的值：底 `#1D1E1F`、边框 `#414243`、分隔线 `#363637`、文字 `#CFD3DC`、
  说明 `#A3A6AD`、禁用 `#6C6E72`、悬停底 `rgb(24,34,43)`、主色 `#409EFF`、阴影 `0 0 12px rgba(0,0,0,.72)`。
  §2.7 的"深色主题只作用于主窗口"要改成：深色只作用于清单 / 日历窗口，窗口里的下拉菜单（行右键、清单名右键、日历筛选）跟着用 EP 深色；
  收纳箱「更多」、编辑窗口导入菜单、各种选择器面板仍是浅色。
- `ep_popper`：`frame_for(dark)`（`frame()` = 浅色）；`PopperOptions` 新增必填字段 `dark`（箭头配色）；`Anchor::at(point)` 零尺寸锚点
  （右键菜单；`contains` 永远为假）；`Placement::RightStart`（侧边弹出不画箭头）。
- `EpDropdownMenu`：`.dark(bool)`、`.width(f32)`（定宽时文字截断）、`.anchor(Anchor)`（量本菜单外框）、`.on_hover(command, hovered)`、
  `panel_width()` / `panel_height()`（文字宽度按 ASCII 0.62 字号、其它 1 字号估算）；`EpDropdownItem`：`.submenu()`、`.note(text)`
  （置灰项同时作为 tooltip）、`.checked(bool)`（文字主色 + 右侧勾）。没加进控件样例窗口（`gallery` 本来就没有下拉菜单）。

### 2.4 日历清单筛选（PRD R7；CLAUDE.md「视图模式 → 日历视图」、spec `window-modes.md` 或 app README 的日历窗口一节）

- 日历标题栏右区最左边一个漏斗按钮（EP `Filter` 图标，新增 `assets/icons/ep/filter.svg`，`EpIcon::Filter`），tooltip「清单筛选」；
  不是"全部"时按钮主色。点开下拉（按钮下方右对齐、带箭头、跟随本窗口的日 / 夜；宽 120~220、最高 320 滚动）：「全部清单」+ 分隔线 +
  按清单顺序的每个清单，勾选显示；点选项不关菜单，点外面 / 再点按钮 / 窗口失活关闭。
- 规则（`filter::next_filter`）：「全部清单」→ 全部（`None`）；"全部"时所有项都打勾，取消一个 = 其余清单；勾上 / 取消某个清单；
  **勾满了或一个不剩都回到"全部"**（所以只有一个清单时点它没有效果）；选了具体清单时以后新建的清单不勾。
- `AppState::calendar_filter()`：内存里的筛选只算现有清单、空了按"全部"（同步删除清单后 `apply_loaded_lists` 可能因为写过窗口属性
  没替换内存副本），`calendar_todos()` 改用它；本机 `delete_list` 成功后同样清理内存筛选（core `lists::delete` 已清理库里的）。
  "兜底到默认清单的待办按默认清单参与筛选"仍是 `in_calendar_filter` + `effective_list_id`。
- `set_calendar_filter` 加了写入计数 `calendar_filter_seq`：连续点选时，前一次写库的结果（core 规范化后的值 / 失败回滚）晚到不覆盖后一次的选择。
- 标题栏最窄 420 时的宽度：左区约 226 + 右区 4 个按钮 136 + 内边距与间距 44 ≈ 406，放得下。

## 3. 自行做的决定

1. **右键菜单是级联的**：主菜单只有「移动到 ▸」，子菜单悬停 / 点击展开（PRD 写的是"只有「移动到 ▸」，列出其它清单"）。没做"移出收起"。
2. **移动是乐观更新**：本地先改归属再写库（PRD"移走后随即消失"），失败回滚。其它写操作（`update_todo`）是成功后才改本地。
3. **移动不改 `sortOrder`**（core `move_to_list` 原样），目标清单里按原值排序，可能与目标清单的待办同值（同值按后端顺序）。
4. **编辑表单"改了清单"= 最终选择 ≠ 预选**：改过去再改回来不提交；预选的清单被删除时一起回到默认清单、不算修改。
5. **只读详情的徽章总是显示**（含只有默认清单时），文案「所属清单：<名称>」。
6. **筛选是复选清单式**（同日历应用的"日历列表"）："全部"时全部打勾；勾满自动回到"全部"（之后新建的清单也显示）。
7. **下拉菜单深色取 Element Plus 暗黑模式的值**，不是窗口的半透明用户底色（弹层要不透明才看得清）。
8. **窗口失活时收起**本窗口的右键菜单 / 筛选下拉（清单名菜单一并改了）。

## 4. 用户能感知到的变化

1. 编辑待办多了「清单」下拉，可以把待办改到别的清单；新建时默认是打开它的那个清单。只读详情显示「所属清单」。
2. 列表 / 四象限里右键待办 → 「移动到 ▸」→ 选清单，待办立刻从当前窗口消失、出现在目标清单窗口。只有一个清单时菜单项置灰。
3. 深色的清单 / 日历窗口里，右键菜单与筛选下拉是深色的（以前清单名的右键菜单在深色窗口里也是白色）。
4. 日历窗口标题栏多了漏斗按钮，可以只看某几个清单的待办；筛选不是"全部"时按钮变蓝。

## 5. 第 6 步（文档、e2e）要注意的地方

- **文档**：§2 各条；`crates/app/README.md` 的模块地图加 `list_window/row_menu.rs`、`calendar_window/filter.rs`，接口表加
  `AppState::{move_todo_to_list, calendar_filter}`；spec §2.7 的深色范围（§2.3）；CLAUDE.md「视图模式 / 待办管理」加右键移动、清单下拉、
  日历筛选；`ui/mod.rs` 的表已改。
- **e2e / 截图**（都没跑）：
  - Windows / Linux / mac e2e 的待办编辑用例：表单在「四象限」下多了一项，靠坐标点击「时间范围」「提醒时间」「重复」等后面表单项的用例
    要往下挪 80px（一个表单项：label 22 + 8、框 32、下边距 18）；只读详情的徽章行多了一个徽章（窄窗口里可能折行）。
  - 建议补：右键行 → 悬停「移动到」→ 点子菜单项（按数据库断言 `todos.list_id`、`list_updated_at` 刷新）；只有默认清单时菜单项置灰；
    编辑表单改清单保存；编辑归属未知的待办不动清单保存后 `list_id` 仍为 NULL（契约 §12-6 的回归）；日历筛选（`window_prefs.calendar_filter`
    的存储值：取消一个 → `[其余]`、勾满 → NULL）。右键菜单的位置只能截图看。
  - regress.py 可加场景：深色清单窗口的行右键菜单（含子菜单）、日历筛选下拉；现在没有 `MINI_TODO_DEBUG_OPEN` 能直接打开这些菜单，
    要截图需要 xdotool 右键 / 点击，或加一个调试开关。
- **本地无法验证**（没启动 GUI）：子菜单悬停展开、点子菜单项不被主菜单的点外关闭吞掉（依赖 `on_mouse_down_out` 在 Capture 阶段、
  `window.mouse_position()` 与上一帧量到的子菜单外框）、菜单宽度估计是否合适（中文 / 英文清单名）、深色菜单的观感、筛选按钮的位置。
  mac 没有新增 `cfg` 代码，走的是共用路径。
- 已知不稳定的 core 单测 `todo::tests::save_image_validates_extension_and_size_before_writing`：本步整套跑过一次全绿。

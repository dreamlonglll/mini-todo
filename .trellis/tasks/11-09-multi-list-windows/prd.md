# 多待办清单窗口 + 独立日历窗口（issue #14 需求 2）

## Goal

把主窗口拆成两种独立的顶层窗口：**待办清单窗口**（可多个，逻辑同收纳箱）与**日历窗口**（单例）。每个清单是一个分类
（如"工作 / 生活 / 其他"），待办只属于一个清单；每个窗口的锁定、日 / 夜模式、列表 / 四象限各自独立。清单跨设备同步，
窗口属性设备本地。来源：issue #14 TIANDI141 的追加需求（列表视图加分类）。

发布：3.1.0，先出 `3.1.0-pre`。issue #14 的需求 1（粘贴图片默认 240px）不在本任务内，单独发 3.0.7。

## Requirements

### R1 数据模型（清单 = 分区）

* todo 新增 `list_id`（`todos.list_id INTEGER NOT NULL DEFAULT 1`，不加外键）；模型字段 `listId`（`#[serde(default = 1)]`）
* 新表 `todo_lists(id, name, sort_order, created_at, updated_at)`，接 v28 的 `local_seq` 触发器（参与同步）
* **默认清单**：id 固定为 1，名字"待办清单"（= 现在标题栏写死的文字），可改名、不可删除。迁移 v31 建它，`updated_at` 写
  `2000-01-01 00:00:00`（两台设备各自迁移出的 id 1 不互相抢，任何一次改名都胜出）；所有存量待办 `list_id = 1`
* **兜底**：`list_id` 缺失 / 指向不存在（含已删除）的清单 → 显示在默认清单里（读侧兜底，不改写记录）
* **新清单 id**：与 cloud `api/ids.rs` 同一算法（UNIX 毫秒 × 1000 + 随机 0..999，越界退回安全随机值），撞主键换 id 重试；
  算法挪进 core 两端共用。（待办 `AUTOINCREMENT` 的两台 PC 撞号问题不在本任务内，另开任务）
* **名称**：PC 上去首尾空格、不区分大小写不允许重名；空名恢复原名。同步造成的重名容忍（cloud 按名查找时 409）
* **顺序**：`sort_order`，新建排最后；第一版没有调整顺序的界面。编辑表单下拉、右键「移动到」、日历筛选都按它排
* **删除清单**：其下待办全部移到默认清单（刷新 `updated_at`，随同步传播），然后删清单、写清单墓碑。确认框："N 项待办将移到「待办清单」"

### R2 同步

* 同步文档新增两个顶层键：
  * `todoLists`: `[{ id, name, sortOrder, createdAt, updatedAt }]`
  * `listTombstones`: `[{ entityId, deletedAt }]`
* **清单墓碑不放进共用的 `tombstones`**：v2.5.0 / 3.0.x 的 `parse_tombstone` 丢弃未知 `entityType`，上传时又按本地墓碑表重建
  `tombstones`，旧设备同步一次就会抹掉 `"list"` 墓碑，未同步到删除的新设备随后按"并集保留"把清单复活。新顶层键经
  `#[serde(flatten)] extra` 被旧版本原样带回（v2.5.0 已有，已核实）
* 合并规则与 todo 一致：记录级 LWW（平局保留本地）、`deletedAt >= updatedAt` 压制、并集、墓碑并集取较大值、保留 30 天；
  PC `sync_store` 与 cloud `sync/merge.rs` 两处一致
* 旧设备的退化（接受）：旧设备编辑过的待办以缺 `listId` 的形式上传 → 新设备按默认值落进默认清单；旧设备不显示清单
* 同步键 `is_fixed` / `text_theme` / `view_mode` / `show_calendar` 只作迁移初始值，之后不再驱动界面（保留在 `SYNCED_SETTING_KEYS`、不删，
  旧设备照常读写）

### R3 导出 / 导入

* `EXPORT_VERSION` → `4.1`：顶层 `todoLists`，todo 带 `listId`；窗口属性不导出（同收纳箱）
* 导入 4.1：清单按"恢复备份"语义（保留 id；备份里没有的清单写清单墓碑；默认清单始终保留，只取备份里的名字）
* 导入 3.0 / 4.0（无 `todoLists`）：**现有清单不动**，所有待办进默认清单
* 旧版本导入 4.1：清单信息被忽略（`import_json` 不校验 version，已核实）

### R4 窗口（设备本地属性）

* 窗口种类：清单窗口（每个清单一个）、日历窗口（单例）。都是独立顶层窗口
* 每窗口设备本地属性：是否隐藏、日 / 夜、列表 / 四象限（仅清单）、展开全部子任务（仅清单，原 `ui-state.json` 全局值改为按窗口）、
  日历清单筛选（仅日历）；按屏幕组合：位置、尺寸、是否锁定（同现在 `screen_configs` 的口径与物理像素换算）
* 不同步、不导出、不接 `local_seq`；登记到 `backend/sync-protocol.md` §3.6 与 CLAUDE.md「当前同步覆盖范围」
* **迁移初始值**（升级后第一次启动）：
  * 默认清单窗口继承原主窗口：各屏幕组合的位置 / 尺寸 / `is_fixed`、`text_theme`、`view_mode`、`ui-state.json` 的展开全部
  * 原来开着日历（`show_calendar = true`）：把加宽的主窗口按原分栏切开——左边是默认清单（`pre_calendar_width` / 40% 口径），
    右边是日历窗口，日历继承同样的锁定与日 / 夜
  * 原来没开日历：日历窗口第一次显示时贴默认清单右侧、与之等高
* **同步来的新清单**：本机没有窗口记录 → 自动开窗（收纳箱找空位逻辑、主显示器、380×600、未锁定、浅色、列表视图）
* **同步来的清单删除**：关掉对应窗口、删窗口记录，原待办按兜底落进默认清单

### R5 锁定与日 / 夜

* 锁定按窗口（每个窗口标题栏一把锁）；**嵌入桌面 / 贴边自动隐藏 / 唤起置顶**仍是全局设置，作用于每个锁定的窗口；
  贴边隐藏状态机按窗口各一份
* 日 / 夜按窗口；深色底色与透明度（`window_bg_color` / `window_bg_alpha`）仍是全局
* 未锁定的窗口进任务栏 / Alt+Tab（同一 AppUserModelID，Windows 合并成一个任务栏图标），锁定后不进（同现在的主窗口）

### R6 标题栏

| 元素 | 默认清单窗口 | 其它清单窗口 | 日历窗口 |
|---|---|---|---|
| 清单名（双击改名 / 右键「重命名 / 删除清单」） | ✓（删除置灰） | ✓ | — |
| 版本号 + 更新红点 | ✓ | — | — |
| 展开全部子任务 | ✓ | ✓ | — |
| 已完成（只看本清单，恢复后回原清单） | ✓ | ✓ | — |
| 列表 / 四象限 | ✓ | ✓ | — |
| 月份导航 / 今天 / 清单筛选 | — | — | ✓ |
| 日 / 夜、锁、设置 | ✓ | ✓ | ✓ |
| 同步 | ✓ | ✓ | — |
| 新建待办 FAB（建到本清单） | ✓ | ✓ | — |

* 改名同收纳箱：就地编辑，锁定中也允许
* 新建清单后窗口直接进入改名

### R7 日历

* 显示所有清单的待办（跨天条，颜色仍用待办自己的颜色；清单没有颜色属性）；点击打开只读详情；不能新建待办（同现在）
* **清单筛选**（下拉多选，设备本地）：取值"全部"或一组清单 id，默认"全部"。"全部"包含以后新建的清单；选了具体清单时新清单默认不勾；
  被删除的清单自动移出，筛选空了回到"全部"；兜底到默认清单的待办按默认清单参与筛选

### R8 待办在清单间移动

* 编辑表单在「四象限」下加「清单」下拉（`ui/ep_select.rs`）：新建时预选当前窗口的清单，托盘新建预选默认清单；只读详情显示"所属清单"
* 待办行右键（列表与四象限）：只有「移动到 ▸」，列出其它清单；只有默认清单时置灰并提示"没有其它清单"。样式用 `ep_dropdown`，跟随窗口日 / 夜
* 不做跨窗口拖动

### R9 关闭 / 隐藏与托盘

* 关闭窗口（Alt+F4 / ⌘W）= 隐藏，不删清单、不退出。Windows 去掉"所有窗口都关了就退出"，与 macOS 一致：只有托盘「退出」/ ⌘Q 退出。
  Linux（无托盘）保留"最后一个可见窗口关闭即退出"
* 隐藏状态重启后保持；默认清单窗口例外（启动时总是显示）
* 托盘：
  * 左键：显示并唤起所有清单窗口（含隐藏的）与日历（开着时）；每个窗口按自己的锁定状态走唤起置顶规则
  * 「添加待办项」：建到默认清单
  * 新增「新增待办清单」、「显示日历」（勾选；与设置 → 常规「显示日历」同一状态，改为设备本地）
  * 删除「固定模式」勾选
  * 「重置位置」：所有清单窗口与日历窗口在主显示器左上起错开排列，全部解除锁定、取消隐藏
  * 收纳箱三项、打开设置、开机自启动、退出不变
* 单实例唤起、macOS Dock 唤起 = 托盘左键

### R10 吸附

* 清单、日历、收纳箱三种窗口为同一个吸附组，双向吸附；规则照搬收纳箱（`windows/fence/snap.rs`、`spec/backend/desktop-fences.md`）：
  同名边对齐、缩放时同名边或等宽 / 等高、±10 逻辑像素、只看光标所在显示器上显示中的窗口（锁定的也算目标，隐藏 / 贴边藏起的不算）、
  目标画 2px `#409EFF` 高亮线、按住 Alt（macOS Option）临时不吸附
* 清单 / 日历窗口的移动与缩放改成自绘（不进系统拖动循环，8 个缩放手柄）；失去 Aero Snap（主窗口本来没有最大化）
* 跨窗口吸附 / 高亮状态从 `services::fences` 提出为通用的"桌面块吸附"服务

### R11 设置

* 常规：「显示日历」改为设备本地（与托盘勾选同一状态）；嵌入桌面 / 贴边隐藏 / 唤起置顶不变（全局）
* 屏幕配置：仍每个屏幕组合一行；删除一个组合连带删除该组合下所有清单 / 日历 / 收纳箱的布局

### R12 cloud（完整读写）

* `TODO_FIELDS` 加 `listId`（整数；写入时校验清单存在，否则 400）；`GET /todos` 支持 `listId=` 与 `list=<名称>` 过滤
* `GET /lists`、`POST /lists`、`PATCH /lists/:id`、`DELETE /lists/:id`；`:id` 接受数字 id 或清单名（重名 409）；
  删除默认清单 400；删除时其下待办移到默认清单（刷新 `updatedAt`）+ 写 `listTombstones`
* 清单不分配短码
* `sync/doc.rs` 读写 `todoLists` / `listTombstones`；`sync/merge.rs` 清单合并与 PC 一致；新清单 id 用现有 `ids.rs`
* Skill：`minitodo.py` 加 `lists` 子命令与 `--list` 参数，`SKILL.md` 同步

### R13 平台

* Windows 与 macOS 同一套代码；mac 差异沿用现有规则（无嵌入桌面、锁定 = `setMovable:NO` + 不可缩放、任务栏 / Alt+Tab 不适用）
* Linux 只为 CI / 截图回归 / e2e

## Acceptance Criteria

* [ ] 从 3.0.x 升级：数据库迁移到 v31，所有待办在"待办清单"里；默认清单窗口的位置、尺寸、锁定、日 / 夜、视图与升级前主窗口一致；
      原来开着日历的，日历成为右侧独立窗口
* [ ] 托盘「新增待办清单」→ 新窗口出现并进入改名；双击标题改名；右键删除 → 确认后待办全部出现在默认清单
* [ ] 每个窗口的锁定、日 / 夜、列表 / 四象限、展开全部互不影响，重启后各自恢复；按屏幕组合各自记忆位置
* [ ] FAB 新建的待办在本清单；托盘「添加待办项」在默认清单；编辑表单下拉与右键「移动到」能把待办移到别的清单
* [ ] 已完成窗口只显示本清单的已完成，恢复后回到原清单
* [ ] 日历显示所有清单的待办；筛选规则符合 R7；筛选设备本地
* [ ] 关闭窗口 = 隐藏；托盘左键全部唤起；Windows 上关掉所有窗口不退出；「重置位置」全部错开、解锁、取消隐藏
* [ ] 两台设备：A 建清单 → B 同步后自动开窗；A 删清单 → B 关窗、待办落进默认清单；A、B 离线各建一个清单不撞 id
* [ ] 混跑旧版本（3.0.6）的设备参与同步后：清单定义与清单墓碑不丢、已删清单不复活；旧设备编辑过的待办落进默认清单（接受的退化）
* [ ] 导出 4.1 含清单；导入 4.1 按恢复语义；导入 4.0 不动现有清单
* [ ] cloud：`/lists` CRUD、`listId` 校验与过滤、按名查找重名 409、删默认清单 400；cloud 写出的清单 / 待办能被 PC 反序列化
* [ ] 吸附：清单 ↔ 清单、清单 ↔ 日历、清单 ↔ 收纳箱双向吸附，高亮与 Alt 规则同收纳箱；收纳箱原有吸附不回归
* [ ] 原有行为不回归：贴边隐藏、嵌入桌面（Win+D 免疫、Explorer 重启重挂）、唤起置顶、提醒、同步、收纳箱

## Definition of Done

* `pc-gpui/`：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` 绿
* `cloud/`：`cargo test --locked` 绿；Skill 单测绿
* mac 专属代码改了跑 `scripts/mac-check`；mac GPUI 胶水推 `claude/**` 分支看 `pc-gpui-e2e-macos.yml`
* Windows e2e、Linux e2e（S 组同步）、mac e2e 相关用例更新并跑通（M 组主窗口用例改为多窗口；补清单用例：新建 / 改名 / 删除 / 隐藏后唤起 / 拖动落库）
* `scripts/regress.py` 场景更新，逐张看对照图
* 文档：CLAUDE.md（核心功能、窗口一览、迁移版本、同步覆盖范围、sync-data 结构、cloud API）、`spec/backend/window-modes.md`、
  `spec/backend/sync-protocol.md`（§3.6 与新顶层键）、`spec/backend/desktop-fences.md`（吸附组）、`spec/pc-gpui/data-compat.md`、
  `spec/pc-gpui/gpui-conventions.md`、`crates/app/README.md`、`crates/core/README.md`、`cloud/README.md`

## Technical Approach（按依赖拆提交，每步可编译、测试全绿）

1. **core 数据层**：迁移 v31（`todo_lists` + 触发器、`todos.list_id`、窗口属性表）、清单 CRUD（`core::lists`）、id 算法、
   `sync_store` 清单读写 / 合并 / `listTombstones`、`build_sync_doc`、导出导入 4.1。界面仍是单主窗口，行为不变
2. **cloud**：`model.rs`、`sync/doc.rs`、`sync/merge.rs`、`api/` 的 `/lists` 与 `listId`、Skill
3. **多窗口底座**：`services::window_mode` 从单例（`main_hwnd`）改为按窗口的状态表（模式、贴边隐藏、置顶、几何保存）；
   吸附从 `services::fences` 提出为通用服务；清单 / 日历窗口的自绘移动与缩放
4. **拆窗**：`main_window` → 清单窗口（列表 / 四象限 / FAB / 标题栏）+ 日历窗口；`WindowManager` 的 `WindowKind` 与登记 key；
   迁移初始布局；关闭 = 隐藏；同步来的清单开 / 关窗
5. **交互**：托盘、编辑表单下拉、行右键「移动到」、已完成按清单过滤、日历筛选、设置页（显示日历、屏幕配置）
6. **文档与 e2e**

## Decision (ADR-lite)

**Context**：issue #14 要"列表视图加分类"。直接在主窗口加标签页满足不了"每块独立锁定 / 日夜"，也不能把清单分开摆在桌面上。

**Decision**：清单 = 待办的分区（`list_id`）+ 独立顶层窗口（同收纳箱）；日历独立成单例窗口；清单同步、窗口属性设备本地；
清单墓碑放独立顶层键 `listTombstones`；cloud 完整读写清单。

**Consequences**：
* `window_mode` 要从"一个主窗口"改成"N 个窗口"，是最重的底层改造；"主窗口"概念在代码里消失（`WindowManager::main` 等调用点全部要改）
* 清单 / 日历窗口失去 Aero Snap，换来与收纳箱一致的吸附
* 旧版本设备编辑过的待办会回到默认清单（接受）；cloud 成为清单的第二个写入方，两端合并规则必须一致
* 同步键 `is_fixed` / `text_theme` / `view_mode` / `show_calendar` 变成只读的历史键

## Out of Scope

* 跨清单汇总窗口（"全部"）、跨清单四象限
* 跨窗口拖动待办
* 清单窗口折叠
* 每窗口的底色 / 透明度、每窗口的嵌入 / 贴边 / 置顶偏好
* 清单颜色、清单排序界面、清单短码
* 「显示清单 ▸」逐个唤起子菜单
* 待办 id 两台 PC 撞号（另开任务）

## Technical Notes

* app：`windows/main_window.rs`（`MainWindow`、`open`、`resize_for_calendar`、`left_panel_width`）、`main_window/title_bar.rs`
  （:160 标题"待办清单"、右侧按钮 :319–410）、`main_window/{list,quadrant,calendar,todo_item}.rs`、`windows/completed.rs`、
  `windows/editor/form.rs`（:62 `render_form`）、`windows/mod.rs`（`WindowManager`、`WindowKind`、`ChildOf`、模态 / 遮罩）、
  `services/window_mode.rs`（`WindowMode` 单例、`apply_mode`、`bring_main_to_front`、`reset_main_window`、`save_window_state`）、
  `services/tray.rs`（:195–244 菜单项）、`services/fences.rs`（吸附运行时、找空位、`monitor_areas`）、`windows/fence/snap.rs`、
  `windows/fence/mod.rs`（自绘移动 / 缩放）、`state.rs`（`view_mode` / `is_dark_theme` / `show_calendar` / `is_fixed` 等全局字段要拆到窗口）、
  `windows/settings/{general,screen}.rs`、`platform/{embed,subclass}.rs`、`services/mac_menu.rs`（Dock 唤起）
* core：`db/migrations.rs`（`LATEST_VERSION` → 31）、`db/models.rs`、`db/sync_store.rs`（`parse_tombstone`、`merge_remote`）、
  `sync/mod.rs`（`SyncData` 的 `extra`、`build_sync_doc`）、`data.rs`（`EXPORT_VERSION`）、`settings.rs`（screen_configs、屏幕组合 ID）、
  `db/settings_kv.rs`（`SYNCED_SETTING_KEYS`）
* cloud：`src/model.rs`（`TODO_FIELDS`）、`src/api/ids.rs`（id 算法）、`src/sync/{doc,merge}.rs`、`src/api/`、`skill/minitodo/`
* 兼容事实（已核实）：v2.5.0 `SyncData` 有 `#[serde(flatten)] extra`（原样保留未知顶层键）；v2.5.0 / 当前 `parse_tombstone` 丢弃未知
  `entityType`；`import_json` 不校验 version
* 旧版本「用本地覆盖云端」（`ForcePush`）也带回远端的 `extra`（v2.5.0 `sync_cmd.rs:1319`、当前 `sync/mod.rs:1292`，已核实）：
  旧设备强推不会清掉 `todoLists` / `listTombstones`，但它上传的所有待办都没有 `listId` → 其它设备上全部落进默认清单。
  在 data-compat 里写明这一退化

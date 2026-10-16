# 收纳箱隐藏图标（点开头自动隐藏、显示隐藏的图标、右键隐藏）

## Goal

收纳箱里有些项目用户不想看到（`.DS_Store`、`.gitkeep` 之类的点文件，或者暂时用不到但又不想移出收纳箱的图标）。
给收纳箱加"隐藏"能力：点开头的文件自动隐藏、右键可以隐藏单个图标、每个收纳箱可以切换"显示隐藏的图标"。
隐藏只影响收纳箱的显示，**不改文件本身**（不改文件属性、不移动文件）。

## Requirements

### R1 点开头的文件自动隐藏（固定开启，无开关）
* 文件名以 `.` 开头的项目（文件或文件夹，Windows 上也算）视为"自动隐藏"，平时不显示
* 判定只看文件名，纯函数（core，供 core / app 共用）
* 自动隐藏的项目不能单独"取消隐藏"（右键菜单里该项显示为禁用，说明"以 . 开头的项目自动隐藏"）
* 外部拖入时：有项目移入后属于自动隐藏、且当前收纳箱没开"显示隐藏的图标"，给一条提示
  （如"N 个以 . 开头的项目已自动隐藏，可在「更多」中显示隐藏的图标"），避免"拖进来就不见了"

### R2 右键隐藏 / 取消隐藏
* 项目右键菜单新增「隐藏」（`EpIcon::Hide`）；隐藏状态的项目（"显示隐藏的图标"打开时才看得到）显示「取消隐藏」（`EpIcon::View`）
* 隐藏标记存 `fence_items.hidden`（迁移 v30，`INTEGER NOT NULL DEFAULT 0`），重启后保持
* 对账保留隐藏标记（含只改大小写的重命名）；资源管理器里改成别的名字 = 新项目，标记丢失（与排序、原始路径一致）
* 「移动到…」另一个收纳箱：**不携带**隐藏标记（目标收纳箱里可见）；移回桌面后在桌面照常可见

### R3 显示隐藏的图标（每个收纳箱独立，记住状态）
* 标题栏「更多」菜单新增切换项：未开启时「显示隐藏的图标」（`EpIcon::View`），开启时「不显示隐藏的图标」（`EpIcon::Hide`）
  ——Element Plus 下拉菜单没有勾选态，用切换文案表达；菜单宽度相应调整
* 状态存 `fences.show_hidden`（迁移 v30，`INTEGER NOT NULL DEFAULT 0`），经 `fence::update(UpdateFenceRequest { show_hidden })` 写库，重启后保持
* 开启时隐藏项（自动隐藏 + 用户隐藏）照常排在网格里，半透明显示（图标与名字整体降低不透明度）
* 关闭时网格只显示可见项；全部项目都被隐藏时空状态提示"N 个项目已隐藏"（而不是"把桌面图标拖到这里"）

### R4 不变的东西
* 带 Windows 隐藏 / 系统属性的文件维持现状：永不显示，不受"显示隐藏的图标"影响；移入时仍 `skipped`
* 删除收纳箱：隐藏项（自动 + 用户）同样移回桌面；删除确认框的 N 计入它们
* 隐藏不改文件属性、不移动文件；收纳箱表仍是设备本地数据（不同步、不导出、不接 `local_seq` / 墓碑）

### R5 排序
* 显示列表过滤掉隐藏项时，箱内拖动排序结束后要把隐藏项按原来的相对位置并回完整顺序再交给 `items::reorder`，
  不打乱隐藏项的位置（纯函数 + 单测）
* 图标懒加载只请求显示中的项目；选中项 / 右键菜单对应项被隐藏后取消选中 / 关闭菜单

## Acceptance Criteria

* [ ] Windows 上往收纳文件夹放 `.gitkeep`（无隐藏属性），收纳箱不显示它
* [ ] 右键某图标 →「隐藏」→ 图标消失；重启应用后仍隐藏；资源管理器里文件仍可见、属性未改
* [ ] 「更多」→「显示隐藏的图标」→ 隐藏项（含 `.gitkeep`）半透明出现；右键用户隐藏的项 →「取消隐藏」恢复正常；
      右键点文件，「取消隐藏」禁用
* [ ] 「显示隐藏的图标」状态按收纳箱独立记住，重启后保持
* [ ] 带隐藏属性的文件无论开关如何都不显示
* [ ] 关闭显示隐藏时拖动排序，隐藏项的相对位置不变；打开显示隐藏后顺序符合预期
* [ ] 全部项目隐藏时空状态显示"N 个项目已隐藏"
* [ ] 拖入 `.env` 之类点文件时提示已自动隐藏
* [ ] 「移动到…」另一个收纳箱后在目标收纳箱可见
* [ ] 删除收纳箱时隐藏项一并移回桌面，确认框 N 含隐藏项
* [ ] 旧库（v29）升级到 v30：已有项目全部可见、`show_hidden` 关闭

## Definition of Done

* core 单测：迁移 v30（列默认值、`LATEST_VERSION == 30`、写收纳箱表不计 `local_seq`）、点文件判定、对账保留 `hidden`（含只改大小写）、
  设置 / 清除隐藏标记、`UpdateFenceRequest.show_hidden`、跨箱不携带；app logic 单测：可见过滤、排序合并、提示文案
* `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` 绿；
  Windows 目标 clippy（本机即 Windows）
* spec `desktop-fences.md`（2 签名、3.2 返回结构、3.3 数据库、3.4 文件层、4 错误矩阵、6 测试）、`CLAUDE.md`（迁移 v30、收纳箱功能描述）、
  `crates/core/README.md` / `crates/app/README.md` 相关段落同步
* 界面改动能截图就截图对照（`scripts/regress.py` 需 Linux / Xvfb，本机不可用时说明）

## Technical Approach

* **数据**（core，迁移 v30）：`ALTER TABLE fence_items ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0`；
  `ALTER TABLE fences ADD COLUMN show_hidden INTEGER NOT NULL DEFAULT 0`。`Fence` 加 `show_hidden: bool`（serde camelCase，
  `FENCE_COLUMNS` / `fence_from_row` 同步）；`UpdateFenceRequest` 加 `show_hidden: Option<bool>`。
* **列举结果**：`FenceEntry` 加隐藏信息（例如 `hidden: bool` 用户隐藏 + `auto_hidden: bool` 点文件，或一个枚举），
  `reconcile_rows` 从行里带出 `hidden`；`get_items` / `FenceRefresh` 仍返回**全部**项目（含隐藏），由 app 按 `show_hidden` 过滤显示。
* **写操作**：core `items::set_hidden(db, fence_id, file_name, hidden) -> R<Option<FenceRefresh>>`（校验文件名、不区分大小写匹配行，
  行不存在时先对账再写），后台执行，结果经 `deliver_refresh` / `set_items` 回窗口。
* **跨箱**：`move_to` 在目标 upsert 时 `hidden` 保持默认 0（新行）——若目标已有同名 NULL 行（监听先补的）也重置为 0。
* **界面**（app）：`grid.rs` 右键菜单加「隐藏 / 取消隐藏」、隐藏项半透明、`display_items()` 按 `show_hidden` 过滤、
  `finish_sort` 合并隐藏项；`panels.rs`「更多」菜单加切换项、`on_title_command` 走 `patch_fence`；
  `mod.rs` 空状态；`logic.rs` 放可见过滤 / 排序合并 / 提示文案纯函数。
* **兼容**：旧版本（3.0.0 等）打开 v30 库只记警告继续运行；新增列都有默认值，旧版本的显式列 SELECT / INSERT 不受影响，
  只是旧版本看不到隐藏效果（收纳箱数据不同步，只影响同一台机器上回退版本的情况）。

## Decision (ADR-lite)

**Context**：隐藏可以靠改文件的 Windows 隐藏属性，也可以只在本应用里记一个标记。
**Decision**：只在数据库里记标记（`fence_items.hidden` + `fences.show_hidden`），点文件按名字判定，不碰文件属性。
**Consequences**：文件在资源管理器里照常可见、移回桌面后照常显示，不会出现"移走后哪里都看不到"；
代价是资源管理器里改名（非大小写）会丢掉隐藏标记，回退到旧版本时隐藏不生效。点文件规则固定开启、没有开关（用户选择）。

## Out of Scope

* 修改文件的 Windows 隐藏属性；显示带隐藏属性的文件
* 点文件自动隐藏的开关；按扩展名 / 通配符的自定义隐藏规则
* 托盘里的全局"显示隐藏"入口
* 多选批量隐藏
* 收纳箱数据同步 / 导出

## Technical Notes

* 相关 spec：`.trellis/spec/backend/desktop-fences.md`、`.trellis/spec/backend/sync-protocol.md` §3.6、
  `.trellis/spec/pc-gpui/gpui-conventions.md`、`.trellis/spec/pc-gpui/data-compat.md`、`.trellis/spec/pc-gpui/verification.md`
* 涉及文件：core `services/fence_fs/folder.rs`（`FenceEntry`）、`services/fence_fs/naming.rs`（点文件判定可放这里）、
  `fence/items.rs`、`fence/mod.rs`、`db/models.rs`、`db/migrations.rs`；
  app `windows/fence/{mod,grid,panels,logic}.rs`
* 现状：`is_hidden_or_system` 在 Windows 只看属性、非 Windows 看点开头（非 Windows 上点文件根本不进列表）。
  R1 之后点文件在所有平台都进列表、标记为自动隐藏：`list_items_at` 在非 Windows 上不再按点开头排除
  （否则 Linux 截图 / 冒烟里"显示隐藏的图标"看不到点文件），Windows 分支不变。`is_hidden_or_system` 还被
  `ops.rs` 的 `plan_move_in` 使用（移入校验，有单测 `plan_move_in_validates_sources`），移入规则不变——
  如果要改它的非 Windows 分支，需拆出列举专用的判定，保持移入校验与单测行为不变。
* 删除确认框 N 用 `get_items` 的数量，隐藏项包含在内，符合 R4。

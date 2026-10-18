# 收纳箱拖动 / 缩放吸附对齐

## Goal

拖动收纳箱（移动）或拖手柄调整收纳箱宽高时，靠近其它收纳箱的同名边（±10 逻辑像素）就自动吸附对齐；缩放时宽 / 高接近其它收纳箱的
宽 / 高也吸附成一样，让多个收纳箱容易排得整齐。例：已有收纳箱 A（left=200、top=100、宽 400、高 300），拖收纳箱 B 使 B.top∈[90,110]
时吸到 top=100；缩放 B 使高度∈[290,310] 时吸到 300。

## Requirements

### 吸附规则

* **阈值**：10 逻辑像素 × 拖动中收纳箱的物理系数（`platform::physical_scale`），100% 缩放下就是 ±10px
* **目标**：除自己以外、当前显示中的收纳箱（含锁定的），取它们**屏幕上的可见矩形**（折叠的只有标题栏）；只取与被拖收纳箱
  在同一显示器（光标所在工作区）上的。拖动开始时快照一次（拖动期间其它收纳箱不会动）
* **横向 / 纵向独立**：两个方向各自找距离最近、且 ≤ 阈值的候选，可以同时吸 x 和 y；同距离按目标顺序取第一个（确定性）
* **移动**：同名边对齐——left↔left、right↔right、top↔top、bottom↔bottom；尺寸不变，只平移
* **缩放**：只调整被拖动的边（对边不动）。每个被拖动方向的候选：
  * 同名边对齐：拖 E 边时 right↔其它的 right，拖 W 边时 left↔left，N / S 同理
  * 等宽等高：拖 E / W 边时宽度 → 其它收纳箱的宽度；拖 N / S 边时高度 → 其它收纳箱的高度
  * 两类候选放在一起取最近；吸附后的尺寸不能小于最小尺寸（不满足的候选跳过）
* **不做**：相邻贴边（我的 left 吸别人的 right）、屏幕工作区边缘、中心线对齐
* **按住 Alt（macOS Option）临时不吸附**：每次移动事件按当时的修饰键判断，松开后下一次移动恢复吸附；不加设置项
* 落库的就是吸附后的几何（移动 / 缩放结束照常 `commit_geometry`）

### 视觉反馈

* 吸附生效时，**目标收纳箱**在对应边上画一条高亮线（主题色 `#409EFF`，约 2 逻辑像素，画在自己窗口内的边缘）：
  * 同名边对齐 → 目标的那条边
  * 等宽 → 目标的上、下两条边；等高 → 目标的左、右两条边
* 不吸附 / 按住 Alt / 松手结束拖动后立即清掉高亮；不新建覆盖窗口、不画跨窗口辅助线
* 被拖动的收纳箱自身不画高亮

### 平台

* Windows 与 macOS 都生效；Linux（GPUI 不能移动窗口）只要编译通过、纯逻辑有单测

## Acceptance Criteria

* [ ] A(200,100,400,300) 存在，拖 B 让 B.top 落在 [90,110] → B.top 吸到 100，松手后库里 `fences.y` 与当前组合 `fence_layouts` 都是 100
* [ ] 拖 B 让 B.right 落在 A.right ±10 内 → B.right = A.right（x 方向），y 方向同时可吸 top / bottom
* [ ] 缩放 B 的 S 边让高度落在 [290,310] → 高度 = 300；缩放 E 边让宽度落在 [390,410] → 宽度 = 400；对边不动
* [ ] 缩放 B 的 S 边让 B.bottom 落在 A.bottom ±10 内 → B.bottom = A.bottom
* [ ] 超出阈值不吸附、跟手；按住 Alt / Option 时不吸附，松开后恢复
* [ ] 吸附时目标收纳箱显示对应边高亮，结束拖动 / 脱离吸附后高亮消失
* [ ] 折叠的目标按标题栏矩形参与吸附；锁定的收纳箱自己不能拖，但能作为目标
* [ ] 原有行为不回归：3px 阈值、双击标题改名、锁定不能拖、拖动中不触发悬停展开 / 收回、只有用户拖动 / 缩放才落库
* [ ] 纯函数（移动吸附、缩放吸附、高亮边计算）有单元测试向量

## Definition of Done

* `pc-gpui/` 下 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` 绿
* mac 专属代码改了就跑 `scripts/mac-check`；mac 的 GPUI 胶水推 `claude/**` 分支看 `pc-gpui-e2e-macos.yml`
* Windows e2e 收纳箱拖动 / 缩放相关用例在本机跑通（本机是 Windows 11）；视需要补吸附用例
* 更新 `.trellis/spec/backend/desktop-fences.md`（移动 / 缩放 / 吸附）、`.trellis/spec/pc-gpui/gpui-conventions.md` §2.3（收纳箱不再用系统拖动）、
  CLAUDE.md「桌面收纳箱」一节

## Technical Approach

### 移动改成自绘拖动（与缩放同一套机制）

系统拖动循环（Windows `WM_NCLBUTTONDOWN(HTCAPTION)`、macOS `performWindowDragWithEvent:`）中途改不了位置；macOS 完全没有钩子。
所以收纳箱的移动改成和缩放一样的自绘拖动：

* 标题栏按下记下光标（屏幕物理坐标 `cursor_position_physical`）与客户区起点；超过 3px 阈值后进入移动会话（`gate.arm` 照旧，
  悬停展开逻辑不变）
* 移动事件（GPUI `on_drag_move`，不依赖悬停；Windows 上 GPUI 按下时已 `SetCapture`，光标出窗口也收得到）→ 新位置 = 起点 + 光标位移
  → 吸附 → 每帧最多一次摆窗（`on_next_frame`，只保留最新一帧）
* 移动只改位置（`set_client_position_physical`），尺寸取窗口当前值：混合 DPI 下跨屏时 `WM_DPICHANGED` 改尺寸不会被拖动覆盖
* 松手 → 最后一帧落地后直接 `commit_geometry`（同缩放），然后 `disarm`
* 收纳箱不再调用 `platform::begin_window_drag`（主窗口等其它窗口不变）

### 吸附纯逻辑（`windows/fence/logic.rs` 或新 `snap.rs`）

* `snap_move(rect, targets, threshold) -> (PxRect, Vec<SnapGuide>)`
* `snap_resize(raw_rect, dir, targets, threshold, min_w, min_h) -> (PxRect, Vec<SnapGuide>)`（在 `resize_rect` 之后调用）
* `SnapGuide { fence_id, edge: Top | Bottom | Left | Right }`；全部带测试向量

### 跨窗口高亮

* `services::fences` 的运行时里存「当前高亮」：`HashMap<fence_id, 边集合>`；拖动中的收纳箱每帧更新，变化时只 `notify` 受影响的目标视图
* 目标收纳箱 `render` 读这份状态，在窗口内边缘画高亮线；拖动结束 / Alt / 关窗时清空

## Decision (ADR-lite)

**Context**：移动走系统拖动循环，中途没法改位置（macOS 没有任何钩子；Windows 只能在子类过程里改 `WM_MOVING` 的 RECT，
且高亮、Alt 判断都要跨模态循环传给 GPUI）。
**Decision**：收纳箱移动改为自绘拖动，与现有缩放共用"屏幕物理坐标 + 每帧摆窗 + 松手落库"的机制；两平台一份实现。
**Consequences**：
* 窗口跟手可能落后一帧（缩放已经是这样）；Windows 上不再进系统移动循环（收纳箱不可缩放、没有贴边分屏，不损失功能）
* 拖动 / 落库的契约变了：移动结束直接落库，不再依赖 `observe_window_bounds` + 闸门防抖识别"用户拖动"；spec 要改
* 风险：Windows 上单按 Alt 松开可能触发系统菜单键（收纳箱没有 `WS_SYSMENU`，预期无反应），实现时验证；mac 的自绘拖动只能在 CI 验证

## Out of Scope

* 相邻贴边、屏幕边缘、中心线、网格吸附
* 跨窗口辅助线 / 覆盖窗口
* 设置开关
* 多选收纳箱一起移动
* 主窗口吸附

## Technical Notes

* `pc-gpui/crates/app/src/windows/fence/mod.rs`：`on_title_mouse_down/move`（:640–675）、`on_geometry_event`（:678）、`commit_geometry`（:699）、
  `flush_geometry`（:721）、`begin_resize` / `resize_move` / `flush_resize` / `end_resize`（:895–969）、`render_resize_handles`（:971）
* `pc-gpui/crates/app/src/windows/fence/logic.rs`：`resize_rect`、`ResizeDir`、`GeometryCommitGate`、`exceeds_drag_threshold`
* `pc-gpui/crates/app/src/services/fences.rs`：`FenceRuntime { windows: HashMap<i64, FenceWindowRecord> }`、`view_of`、`monitor_areas`、
  `apply_window_rect`
* `pc-gpui/crates/app/src/platform/`：`client_rect_physical`、`set_client_position_physical`、`cursor_position_physical`、`physical_scale`
* GPUI Windows 后端 `events.rs`：鼠标按下 `SetCapture`、抬起 `ReleaseCapture`（gpui-pre-windows 0.3.9 :476 / :506）
* spec：`.trellis/spec/backend/desktop-fences.md`「移动」「缩放」「几何落库」（:247–271）、`.trellis/spec/pc-gpui/gpui-conventions.md` §2.3
* e2e：`pc-gpui/scripts/e2e-windows/`（真实输入拖动 / 缩放）、`pc-gpui/scripts/e2e-macos/`（MF 组「拖动落库」）

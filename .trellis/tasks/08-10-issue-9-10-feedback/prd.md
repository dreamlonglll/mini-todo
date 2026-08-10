# 修复 issue 9 与 issue 10 的用户反馈

## Goal

GitHub issue #9（一些问题反馈）与 #10（任务栏图标触发动作的建议）由同一位用户（@KieMg）在 2026-08-10 提出，
共 6 条：3 条确认 bug、1 条功能缺失、2 条交互改进（其中 2 条同根因）。目标是全部修复并发布到 v2.2.0 之后的
下一个 release，让这批反馈一次性闭环。

## Requirements

MVP = 两个 issue 的全部 6 条，按 PR 分期实施。

### PR1 — #9-1 改四象限后颜色跟随

- 编辑模式改象限时，**仅当当前颜色等于旧象限的默认色**（即用户没手动改过）才同步为新象限默认色；
  用户自定义过的颜色必须保留。
- 四象限视图拖拽换象限（`QuadrantView.onDragChange`）走同一套策略，两条路径行为一致。

### PR2 — #10 托盘双击 + #9-5 贴边唤起置顶（同根因）

- 托盘双击改为「显示并置顶主窗口」（`unminimize` + `show` + `set_focus` + 临时置顶），不再弹新建待办窗口；
  新建待办保留在托盘右键菜单里。
- 贴边自动隐藏唤起时临时置顶，收回时取消置顶，使其能盖住最大化/无边框全屏窗口。
- 新增设置项「唤起时置顶」（默认开启），关闭后退回当前行为，以回应反馈者对全屏游戏被打扰的顾虑。

### PR3 — #9-2 子任务拖拽排序

- 编辑窗口（EditorView）的子任务列表支持拖拽排序，编辑模式与新建模式（pendingSubtasks）都支持。
- 排序语义为**纯手工顺序**：拖拽结果即最终顺序，取消「已完成沉底」，勾选完成后条目留在原位。
- 主界面展开区（`TodoItem.sortedSubtasks`）同步去掉按 `completed` 重排，改为直接按 `sort_order` 展示，
  确保同一份子任务在两个界面顺序一致。
- 主界面展开区本身**不**加拖拽能力（避免与外层待办拖拽嵌套误触）。

### PR4 — #9-4 自定义界面底色与背景透明度

- 设置面板「外观」区新增两项：界面底色（预设色板 + 取色器）、背景透明度（滑块）。
- 深色主题开关维持独立语义（只管文字/边框配色），不降级为底色预设，现有用户预期不变。
- 改动即时生效于主窗口（走 `app-settings-changed` 事件），无需重启。

### 收尾 — #9-3 发版

- 上述 PR 合并后发布新版本，一并带上已在 main 但未发版的 `c665551`（深色主题设置同步修复）。

## Acceptance Criteria

**PR1**

- [ ] 新建待办用默认象限（不紧急不重要/绿）保存后，编辑改为「重要且紧急」并保存，列表圆点变红
- [ ] 手动把颜色选成紫色的待办，改象限后颜色仍为紫色
- [ ] 在四象限视图把待办从一个象限拖到另一个象限，颜色按同一策略变化

**PR2**

- [ ] 固定模式 + 贴边隐藏下，前台为最大化浏览器时，鼠标触发唤起，窗口显示在浏览器之上
- [ ] 鼠标离开、窗口收回后，主窗口不再保持置顶（不遮挡其他窗口）
- [ ] 关闭「唤起时置顶」后，唤起行为退回当前的仅移动位置
- [ ] 双击托盘图标：主窗口显示、获得焦点并可见于最前，不再弹出新建待办窗口
- [ ] 托盘右键菜单「新建待办」仍可用

**PR3**

- [ ] 编辑窗口内拖拽子任务后顺序立即变化，关闭重开仍保持
- [ ] 勾选某个子任务为完成，它停留在原位置不沉底
- [ ] 主界面展开区的子任务顺序与编辑窗口一致
- [ ] 新建待办（尚未保存）时拖拽子任务，保存后顺序与拖拽结果一致

**PR4**

- [ ] 在设置里改界面底色，主窗口立即变色，无需重启
- [ ] 在设置里拖透明度滑块，主窗口背景透明度立即变化
- [ ] 重启应用后底色与透明度保持
- [ ] 导出数据 → 清库 → 导入，底色与透明度被还原
- [ ] WebDAV 上传后在另一端应用远端数据，底色与透明度同步过去

**收尾**

- [ ] 新版本的设置窗口深色主题开关显示真实状态，切换后主窗口即时变色（验证 `c665551` 已随版本发出）

## Definition of Done

- `cd pc && npm run build` 通过（含 vue-tsc）
- `cd pc/src-tauri && cargo check` 通过
- 涉及 settings 变更的 PR（PR2、PR4）逐项走完 CLAUDE.md 的 11 项导入导出/同步检查清单
- 行为变更同步更新 `.trellis/spec/` 相关规范
- 在真实 Windows 环境手动验证（贴边唤起、托盘双击、深色主题必须实机确认，无法靠编译验证）
- 发版遵循既有流程：三处版本号（`pc/package.json`、`pc/src-tauri/tauri.conf.json`、`pc/src-tauri/Cargo.toml`）
  同步递增 → 推 tag → **等 GitHub 构建成功** → 改写中文 release notes
- 发布后在 issue #9 / #10 下回复反馈者，逐条说明处理结果

## Technical Approach

### #9-1 颜色同步

`pc/src/views/EditorView.vue:248-253` 的 `handleQuadrantSelect` 去掉 `if (!isEdit.value)` 门槛，
改判「当前 color 是否等于旧象限默认色」：

```js
function handleQuadrantSelect(q) {
  const prev = form.value.quadrant
  if (form.value.color === getQuadrantColor(prev)) {
    form.value.color = getQuadrantColor(q)
  }
  form.value.quadrant = q
}
```

`QuadrantView.onDragChange`（`pc/src/components/QuadrantView.vue:50-62`）目前只调 `updateTodoQuadrant`，
需要确认 store 侧签名，让它能一并写 `color`。

### #10 + #9-5 置顶

- `pc/src-tauri/src/lib.rs:210-225` 双击分支：从 `emit("tray-add-todo")` 改为
  `unminimize + show + set_focus`，再叠一次短暂 `set_always_on_top(true)` → 取消，
  否则固定模式的 `WS_EX_TOOLWINDOW` 窗口仍可能被前台全屏窗口压住。
- `pc/src-tauri/src/commands/window.rs:441-489` `tick_auto_hide`：
  `AutoHideTransition::Restore` 分支 `set_always_on_top(true)`，`Hide` 分支 `set_always_on_top(false)`。
- 新增 settings key（如 `top_on_wake`，默认 true）→ migration v25，走完 11 项清单，
  并加入 `AppSettingKey` 联合类型以支持跨窗口即时生效。

### #9-2 子任务拖拽

- 后端新增 `reorder_subtasks(parent_id, ids)` 命令，实现照抄 `pc/src-tauri/src/commands/todo.rs:213`
  的 `reorder_todos`（按数组下标批量写 `sort_order`）。
- `pc/src/views/EditorView.vue:141-148` 的 `currentSubtaskList` 去掉 `completed` 排序，
  列表接 `vuedraggable`（参考 `TodoList.vue:59` 与 `QuadrantView.vue:118` 的既有用法，用独立 handle）。
- `pc/src/components/TodoItem.vue:106-108` 的 `sortedSubtasks` 同步去掉 `completed` 排序。
- 新建模式下拖的是本地 `pendingSubtasks` 数组，保存时按数组顺序依次 `create_subtask` 即可（后端按 MAX+1 递增）。

### #9-4 底色与透明度

- 先把 `pc/src/styles/main.scss:110-134` 硬编码的 `rgba(0,0,0,0.1~0.15) !important` 抽成
  `--app-bg-color` + `--app-bg-alpha` 两个变量（这是前置重构，不做则任何用户设置都压不过 `!important`）。
- 新增 settings key `window_bg_color`、`window_bg_alpha` → migration v26 → 11 项清单全过。
- 设置面板加色板 + 取色器 + 滑块，改动后 emit `app-settings-changed`，主窗口监听并更新 CSS 变量。
- 窗口本身已是 `transparent: true` / `decorations: false`（`pc/src-tauri/tauri.conf.json:22-23`），
  透明度靠 CSS alpha 即可实现，不需要动窗口属性。

## Decision (ADR-lite)

**Context**：6 条反馈涉及颜色语义、排序语义、窗口 Z 序与外观可配置性，每条都有多种合理实现。

**Decision**：
1. 象限颜色采用「智能同步」——只在颜色未被自定义时跟随象限，兼顾修复与自定义能力。
2. 子任务排序采用「纯手工顺序」，取消已完成沉底；两个界面统一按 `sort_order` 展示。
3. 拖拽只做在编辑窗口，主界面展开区保持只读，规避嵌套拖拽误触。
4. 置顶做成可关闭的设置项，不实现前台全屏进程检测。
5. 底色/透明度作为两个独立设置项，深色主题开关语义保持不变。

**Consequences**：
- 决策 1 的判定依赖「颜色恰好等于象限默认色」这一启发式：若用户手动挑的颜色刚好与象限默认色相同，
  仍会被视为未自定义。此边界可接受。
- 决策 2 改变了现有展示行为（已完成子任务不再自动沉底），属于用户可感知的变更，release notes 需要说明。
- 决策 4 无法覆盖独占全屏游戏场景（topmost 在该模式下本就不可靠），依赖用户自行关闭开关。
- 决策 5 使 PR4 需要一次样式变量化重构，是四个 PR 里风险最高的一个，排在最后。

## Out of Scope

- 「沉浸模式」自动检测前台全屏进程 —— 反馈者自己也认为成本高，用设置开关替代
- 深色主题覆盖设置/编辑器等独立 WebView（`appStore.ts:408-411` 的 label 限制）—— 独立议题，另行立项
- 主界面子任务展开区的拖拽能力
- 固定模式下让窗口重新出现在任务栏（`WS_EX_TOOLWINDOW` 是刻意设计）

## Technical Notes

### 根因定位（已核实）

| 条目 | 位置 | 根因 |
|---|---|---|
| #9-1 | `pc/src/views/EditorView.vue:248-253` | `if (!isEdit.value)` 使编辑模式不同步颜色；列表圆点读 `todo.color`（`TodoItem.vue:25-27`）而非 `quadrant` |
| #9-2 | 全项目 | 仅 `TodoList.vue:59`、`QuadrantView.vue:118` 用了 vuedraggable；后端无子任务批量重排命令 |
| #9-3 | — | `c665551` 已修，但 `git tag --contains c665551` 为空，最新 release v2.2.0（2026-08-09）不含它 |
| #9-4 | `pc/src/styles/main.scss:110-134` | 背景色硬编码 `rgba !important`，无可配置入口 |
| #9-5 | `pc/src-tauri/src/commands/window.rs:441-489` | 唤起只 `set_position`，Z 序不变；主窗口从未调用 `set_always_on_top`；固定模式 `WS_EX_TOOLWINDOW`（`window.rs:703`）使其也不在任务栏 |
| #10 | `pc/src-tauri/src/lib.rs:210-225` | 双击 emit `tray-add-todo`，而新建待办在右键菜单（`lib.rs:164-168`）已有入口 |

### 参考

- https://github.com/dreamlonglll/mini-todo/issues/9
- https://github.com/dreamlonglll/mini-todo/issues/10
- 相关提交 `c665551`（设置界面与主窗口一致性一揽子修复，含 `app-settings-changed` 事件机制，PR2/PR4 复用）
- 颜色常量：`pc/src/types/todo.ts` `QUADRANT_INFO` / `DEFAULT_COLOR` / `PRESET_COLORS`

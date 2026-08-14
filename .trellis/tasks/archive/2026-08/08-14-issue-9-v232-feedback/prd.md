# issue 9 v2.3.2 反馈：任务栏图标不一致 + 只读模式勾选子任务

## 背景

issue #9 用户 KieMg 在 v2.3.2 后追加反馈，本任务处理其中两点：

- 第 2 点（bug）：固定模式下任务栏 mini-todo 标签显隐不一致——
  开机自启后有标签；关/开一次锁定后标签消失；贴边唤起后标签重新出现，收回后不消失。
- 第 3 点（体验）：子任务完成操作藏得太深，要求只读详情模式下也能直接勾选子任务完成。

## 第 2 点根因

固定模式靠手动 `SetWindowLongW` 加 `WS_EX_TOOLWINDOW`（去 `WS_EX_APPWINDOW`）隐藏任务栏图标。
但 tao 在处理**任何**窗口 flag 变更（`set_always_on_top` / `show` / `unminimize` / `set_resizable` 等）时，
`WindowFlags::apply_diff` 会用它内部记录的 flags **整体重写** `GWL_EXSTYLE`，
把绕过 tao 手动设置的 `WS_EX_TOOLWINDOW` 抹掉。三个现象逐一对应：

1. 开机自启：前端恢复固定模式设好了样式，但贴边隐藏首次收回时
   `tick_auto_hide` Hide 分支调 `set_always_on_top(false)` → 样式被重写 → 图标复现；
2. 手动关/开锁定：重新执行 `set_window_fixed_mode(true)` 设回样式 → 图标消失（此时没有后续 tao 调用）；
3. 贴边唤起：Restore 分支 `set_always_on_top(true)` → 样式被重写 → 图标复现；
   收回时 `set_always_on_top(false)` 不会加回 TOOLWINDOW → 图标留存。

## 方案

### 第 2 点（pc/src-tauri/src/commands/window.rs）

- 抽出 `apply_fixed_ex_style(window, fixed)`：设置/恢复 `WS_EX_TOOLWINDOW` 样式，
  并补一次 `SetWindowPos(SWP_FRAMECHANGED)` 让任务栏立刻响应样式变更。
- 新增 `reassert_fixed_taskbar_style(window)`：通过 `run_on_main_thread` 把补样式排进
  主线程消息队列——tao 的窗口操作都是投递到该队列异步执行的，排队即可保证补样式发生在其后，
  无竞态。
- 在所有会触发 tao 重写主窗口样式的调用点之后补样式：
  - `tick_auto_hide` Hide / Restore 分支的 `set_always_on_top` 之后
  - `set_window_fixed_mode`（原地内联样式块改为统一走 reassert）
  - `set_auto_hide_enabled` 的 show / set_always_on_top 分支
  - `set_top_on_wake` 的关闭分支
  - `bring_main_window_to_front` 主流程结尾 + 延迟撤销置顶线程
- `reset_window_impl` 不处理：托盘重置的前端监听器会顺带退出固定模式，最终样式正确。

### 第 3 点（pc/src/views/EditorView.vue）

- 子任务复选框点击不再被 `isViewMode` 拦截（只读模式必有 id，走 `toggleSubtask` 直接落库）。
- 删除"只读模式复选框不可点"的 CSS 覆写，恢复 pointer 光标与 hover 反馈。
- 其余写入口（添加、删除、重命名、编辑子任务）仍随只读模式隐藏。

## 验收

- `cargo check`、`npm run build`（vue-tsc）通过。
- 固定模式：开机自启 / 手动切换 / 贴边唤起-收回，任务栏均无 mini-todo 标签；
  退出固定模式后标签恢复。
- 只读详情：可直接勾选/取消子任务完成态，其余编辑入口保持隐藏。

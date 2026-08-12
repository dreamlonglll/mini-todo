# PRD：修复 issue #9 v2.3.0 试用后的五条新反馈

## 背景

用户 KieMg 更新 v2.3.0 后在 issue #9 回复了 5 条新反馈。已通过 4 个并行探查代理逐条核实代码：
4 条成立，1 条（旧子任务拖拽）归因有误但现象真实。修复方向已与 owner 确认。

## 需求逐条

### 1. 窗口边缘 1px 过渡边框（成立）

现状：`decorations: false` + `shadow: false`，最外层容器无任何 border/outline/box-shadow
（`pc/src/styles/main.scss:158-172`），浅色白底/深色黑底与桌面硬切。

方案（采用反馈者的「动态颜色」建议）：
- 浅色模式：最外层加 `1px solid rgba(0,0,0,0.5)` 系边框（半透明黑叠在底色上 = 比底色更深）
- 深色模式：`1px solid rgba(255,255,255,0.5)` 系（半透明白 = 比底色更浅），透明度可酌情调低避免过亮
- **DWM 圆角联动**：Win11 上 `DWMWCP_ROUND`（`lib.rs:48`）会裁圆角而 CSS 是直角，直角边框会被裁「断头」。
  需给最外层同步加 `border-radius`（约 8px）+ `overflow: hidden`；Win10 无 DWM 裁剪，CSS 圆角 + 透明窗口
  自然呈现圆角，两平台一致。
- **固定模式不加边框**：固定模式是透明贴桌面场景，描边会破坏融入感。

### 2. 列表底部被「新建待办」FAB 遮挡（成立，100% 遮挡）

现状：`.fab-add` fixed 右下（底部占 68px 条带，`main.scss:697-724`），`.main-content`
底部 padding 仅 16px（`main.scss:336`）。滚动到底时最后一项删除按钮完整落入 FAB 命中区。

方案：`.main-content` 底部 padding 加大到 ≥76px，覆盖列表与四象限两个视图。
固定模式下 FAB 不渲染，若加 padding 造成观感问题则按条件类处理（实现时判断）。

### 3. 只读详情模式子任务允许拖拽（owner 已拍板）

现状：已有待办点开默认 `mode=view`，拖拽手柄 `v-if="!isViewMode"` 且 draggable
`:disabled="isViewMode"`（`EditorView.vue:1180-1197`）。数据层无任何问题（sort_order 正常递增）。

方案：只读模式下也渲染拖拽手柄并启用拖拽，`onSubtaskDragEnd` 照常持久化
（`reorder_subtasks`）。完成勾选/删除/新增等写入口继续保持隐藏（不推翻 4a8095d）。

### 4. 开机自启可靠性（成立）

现状（tauri-plugin-autostart 2.5.1）：
- `isEnabled()` 只判断注册表键存在，不校验路径是否指向当前 exe → 便携版移动/换装后开关永远显示已开启但不生效
- 托盘勾选只在启动时读一次（`lib.rs:119-127`），之后靠 muda 自动翻转（`lib.rs:184-194`），
  与设置面板互不同步，交叉操作可把状态改反（界面显示开启、注册表已删）

方案：
- 启动时若 `is_enabled()` 为 true，重新调用 `enable()` 刷新注册表路径到当前 exe（自愈搬移/换装场景）
- 托盘自启菜单项持有 handle（参照 `set_tray_toggle_fixed_item` 先例），任何一侧切换后
  显式 `set_checked(实际状态)`，不再依赖自动翻转
- 设置面板切换后同步托盘勾选
- 不处理（记录即可）：上游 auto-launch 路径未加引号、`--autostart` 参数未消费、无 single-instance

### 5. WebView2 运行时（成立）

现状：`tauri.conf.json` 无 `bundle.windows` 段 → 默认 `downloadBootstrapper`（装时需联网）；
CI 额外产出便携版 zip（裸 exe），无任何 WebView2 兜底；README 未写运行时要求。

方案：
- `bundle.windows.webviewInstallMode` 改为 `embedBootstrapper`（体积仅 +~2MB，离线安装包内嵌引导器）
- README 下载章节注明：便携版需系统已有 WebView2 Runtime（Win11 内置；Win10 部分环境需手动安装），附微软下载链接

## 验收

- `cd pc && npm run build`（vue-tsc）通过
- `cd pc/src-tauri && cargo check` 通过
- 逐项自查：浅/深色边框存在且四角不断头；列表滚到底删除按钮可点；只读详情可拖子任务且顺序持久化；
  自启开关与托盘勾选任意交叉操作后状态一致；tauri.conf.json 校验通过

## 不做

- 「沉浸模式」（自动检测前台全屏）— issue 里已说明往后放
- 主界面展开区子任务拖拽（嵌套拖拽误触问题，v2.3.0 已解释）
- 更换 autostart 插件 / single-instance

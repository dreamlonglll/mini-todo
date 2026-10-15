# Research: 编辑类与弹出类窗口 UI 规格（供 GPUI 重写 1:1 还原）

- **Query**: 为 EditorView / SubtaskEditorView / MarkdownEditor / CompletedView / NotificationView 写一份可直接照着用 GPUI 重新实现的 UI 规格（元素树、样式数值、交互、文案、命令契约、工具函数规则），并评估 Milkdown 在 GPUI 中的替代方案
- **Scope**: mixed（内部源码通读 + 依赖包 CSS 实测：element-plus 2.13.1 theme-chalk、@milkdown/theme-nord 7.22.2、@milkdown/preset-* 7.22.2、tauri 2.9.5 drag.js；gpui-component 0.7.3 本地源码）
- **Date**: 2026-10-15

---

## 0. 文件清单

| 文件 | 内容 |
|---|---|
| `pc/src/views/EditorView.vue` | 待办详情 / 编辑窗口（2330 行：只读详情 ⇄ 编辑、右侧子任务面板、描述放大弹窗） |
| `pc/src/views/SubtaskEditorView.vue` | 子任务查看 / 编辑窗口（含内存模式） |
| `pc/src/components/MarkdownEditor.vue` | Milkdown 封装（编辑 / 只读、图片上传、图片预览、链接点击） |
| `pc/src/views/CompletedView.vue` | 已完成列表窗口 |
| `pc/src/views/NotificationView.vue` | 应用内通知卡片 |
| `pc/src/utils/datetime.ts` (+test) | 时间解析 / 格式化（K1） |
| `pc/src/utils/imageRef.ts` (+test) | `minitodo-image://` 引用 ⇄ asset URL（K5） |
| `pc/src/utils/fileLink.ts` (+test) | 链接协议白名单 |
| `pc/src/utils/editorShortcuts.ts` (+test) | Esc / Ctrl+Enter |
| `pc/src/utils/notify.ts` | `notifyError` 失败提示格式 |
| `pc/src/utils/quadrant.ts`、`utils/color.ts` | 选象限时颜色联动、颜色比较 |
| `pc/src/types/todo.ts` | 预设色、象限定义、请求类型 |
| `pc/src/styles/main.scss` | 全局变量、Element Plus 覆盖 |
| `pc/src/views/MainView.vue` L427-557 | 打开已完成窗口 / 编辑器窗口的参数 |
| `pc/src/router/index.ts` | 路由 |
| `pc/src-tauri/src/commands/todo.rs` | get_todo / create_todo / update_todo / *subtask* / save_subtask_image / import_subtasks_from_paths |
| `pc/src-tauri/src/services/notification.rs` L1-460 | 应用内通知窗口的创建、尺寸、定位、叠放 |
| `pc/src-tauri/src/commands/notification_cmd.rs` | close_notification_window / close_all_notification_windows |
| `pc/src-tauri/src/db/models.rs` L141-217 | Create/Update 请求的 serde 结构 |
| `pc/src-tauri/src/db/time.rs` | 后端时间解析（与前端 datetime.ts 有边界差异，见 §9.5） |
| `pc/src-tauri/capabilities/default.json`、`tauri.conf.json` | 窗口权限、asset 协议 scope、CSP |

> TodoItem.vue 不创建任何窗口（任务描述里提到的 TodoItem 开窗不存在）；所有编辑器窗口都由 MainView / CompletedView 打开，子任务窗口只由 EditorView 打开，通知窗口只由 Rust 后端创建。

---

## 1. 全局约定（所有编辑/弹出窗口共用）

### 1.1 主题与 CSS 变量（这些窗口永远是浅色）

`appStore.applyThemeClass()` / `applyWindowBackground()` 只在 label 为 `main` 的窗口生效（appStore.ts L487-504），所以编辑器 / 子任务 / 已完成 / 通知窗口**永远不会挂 `body.dark-theme`**，变量取 `:root` 浅色值：

| 变量 | 值 |
|---|---|
| `--primary` | `#3B82F6` |
| `--primary-light` | `#60A5FA` |
| `--primary-dark` | `#2563EB` |
| `--bg-primary` | `transparent`（html/body 背景；编辑器窗口 `transparent:false`，WebView 默认白底） |
| `--bg-secondary` | `#F8FAFC` |
| `--bg-tertiary` | `#F1F5F9` |
| `--text-primary` | `#1E293B`（body 文字色，所有未指定颜色的文字） |
| `--text-secondary` | `#64748B` |
| `--text-tertiary` | `#94A3B8` |
| `--border` | `#E2E8F0` |
| `--border-light` | `#F1F5F9` |
| `--success` / `--warning` / `--error` | `#10B981` / `#F59E0B` / `#EF4444` |

全局：`* { margin:0; padding:0; box-sizing:border-box }`；
`body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'Microsoft YaHei', sans-serif; font-size:14px; line-height:1.5; color:#1E293B; -webkit-font-smoothing: antialiased }`。
Windows 上实际字体：拉丁字符 Segoe UI，中文回落 Microsoft YaHei。

> 字重匹配提示（CSS 字体匹配规则在 Windows 系统字体上的实际效果）：Segoe UI / 微软雅黑没有 500 字重，`font-weight:500` 实际渲染为 Regular(400)；`650` 实际渲染为 Bold(700)；`600` 对 Segoe UI 是 Semibold、对微软雅黑回落 Bold。GPUI 侧若想"看起来一样"，500 用 NORMAL，650 用 BOLD。

### 1.2 Element Plus（2.13.1，未配置 locale → 默认英文，未覆盖 `--el-color-*`）

项目**没有**修改 Element Plus 主题色，所以 Element Plus 组件用的是它自己的默认蓝 `#409EFF`，与项目自定义按钮用的 `#3B82F6` 不是同一个蓝。原样记录：

| EP 变量 | 值 |
|---|---|
| primary | `#409EFF`；light-3 `#79BBFF`；light-5 `#A0CFFF`；light-7 `#C6E2FF`；light-8 `#D9ECFF`；light-9 `#ECF5FF`；dark-2 `#337ECC` |
| success | `#67C23A`；light-5 `#B3E19D`；light-8 `#E1F3D8`；light-9 `#F0F9EB`；dark-2 `#529B2E` |
| warning | `#E6A23C`；light-5 `#F3D19E`；light-8 `#FAECD8`；light-9 `#FDF6EC`；dark-2 `#B88230` |
| danger/error | `#F56C6C`；light-8 `#FDE2E2`；light-9 `#FEF0F0` |
| info | `#909399`；light-8 `#E9E9EB`；light-9 `#F4F4F5` |
| text primary / regular / secondary / placeholder / disabled | `#303133` / `#606266` / `#909399` / `#A8ABB2` / `#C0C4CC` |
| border / light / lighter | `#DCDFE6` / `#E4E7ED` / `#EBEEF5` |
| fill / light / blank | `#F0F2F5` / `#F5F7FA` / `#FFFFFF` |
| disabled-bg | `#F5F7FA`；disabled-border `#E4E7ED`；disabled-text `#A8ABB2` |
| box-shadow | `0 12px 32px 4px rgba(0,0,0,.04), 0 8px 20px rgba(0,0,0,.08)` |
| box-shadow-light | `0 0 12px rgba(0,0,0,.12)` |
| overlay（对话框遮罩） | `rgba(0,0,0,.5)` |
| component size default / small | 32px / 24px |

**main.scss 对 EP 的覆盖**（全局）：

| 选择器 | 覆盖 |
|---|---|
| `.el-button` | `border-radius: 6px !important`；内部 `> span { display:inline-flex; align-items:center; gap:6px }` |
| `.el-input__wrapper` | `border-radius: 6px !important` |
| `.el-dialog` | `border-radius: 12px !important; margin-top: 5vh !important` |
| `.el-dialog__body` | `overflow: visible !important; max-height: 70vh; overflow-y: auto` |
| `.el-picker__popper, .el-popper.is-pure, .date-picker-popper-bottom` | `z-index: 3000 !important` |

**本文用到的 EP 组件默认外观**（数值来自 theme-chalk 2.13.1）：

| 组件 | 默认外观 |
|---|---|
| `el-button`（default） | 高 32，padding `8px 15px`，字号 14，字重 500，圆角 6（覆盖后）；白底、边 `#DCDFE6`、字 `#606266`；hover 字 `#409EFF` 底 `#ECF5FF` 边 `#C6E2FF`；active 边 `#409EFF`；相邻按钮 `.el-button+.el-button{margin-left:12px}` |
| `el-button size=small` | 高 24，padding `5px 11px`，字号 12 |
| `type=primary` | 底/边 `#409EFF` 字白；hover `#79BBFF`；active `#337ECC`；disabled 底/边 `#A0CFFF` |
| `type=success plain` | 字 `#67C23A` 底 `#F0F9EB` 边 `#B3E19D`；hover 字白 底/边 `#67C23A` |
| `type=warning plain` | 字 `#E6A23C` 底 `#FDF6EC` 边 `#F3D19E`；hover 字白 底/边 `#E6A23C` |
| `text`（无 type） | 透明底无边，字 `#606266`；hover 底 `#F5F7FA`；active 底 `#F0F2F5` |
| `text type=primary` | 同上，字 `#409EFF` |
| `:loading` | 按钮加 `is-loading`（不可点、覆盖一层 `rgba(255,255,255,.3)` 蒙版），图标位置换成旋转的 Loading 图标 |
| 按钮内图标+文字间距 | 图标后紧跟 `<span>` 时 EP 加 `margin-left:6px`，再叠加 main.scss 的 `gap:6px` = **12px**；图标后是裸文字节点时只有 gap **6px** |
| `el-icon` | `width/height = 1em`，`font-size: inherit`，`color: inherit`（不写 size 时随父字号） |
| `el-input`（default） | 高 32；wrapper `padding:1px 11px`，`box-shadow: 0 0 0 1px #DCDFE6 inset`，圆角 6；hover 描边 `#C0C4CC`；focus 描边 `#409EFF`；文字 14px `#606266`；placeholder `#A8ABB2`；disabled：底 `#F5F7FA`、描边 `#E4E7ED`、字 `#A8ABB2`、`cursor:not-allowed`；`maxlength` 只做原生长度限制（未开 show-word-limit，不显示计数） |
| `el-select`（default） | wrapper `min-height:32; padding:4px 12px; gap:6px; 14px`，描边同 input，focus `#409EFF`；右侧箭头 ArrowDown（展开时旋转 180°）；下拉项高 34、`padding:0 32px 0 20px`、14px `#606266`，hover 底 `#F5F7FA`，选中项字 `#409EFF` 加粗；下拉列表 `padding:6px 0`，浮层阴影 box-shadow-light、圆角 4、边 `#E4E7ED` |
| `el-input-number` | 默认宽 150（本项目都 `style="width:100%"`）；左右各一个 32px 宽按钮（左 `−` 右 `+`，底 `#F5F7FA`，hover 字 `#409EFF`，到达边界 disabled 字 `#A8ABB2`）；输入框 padding 左右 42、文字居中；`#suffix` 插槽文字显示在数字右侧输入框内 |
| `el-switch` | 核心 40×20、圆角 10；关 `#DCDFE6`，开 `#409EFF`；白色滑块 16×16，左 1px ↔ `calc(100% - 17px)`；过渡 0.3s；整体高 32 |
| `el-checkbox` | 高 32，`margin-right:30px`（最后一个 0）；方框 14×14 圆角 2 边 `#DCDFE6`（hover `#409EFF`）；选中底/边 `#409EFF` + 白勾；标签 14px `#606266`、`padding-left:8px`，选中时标签字 `#409EFF` |
| `el-date-picker type=date` | 外观就是 el-input：前缀 Calendar 图标（`#A8ABB2`）、可清空（hover 显示 CircleClose）、可直接输入；面板宽 322、日期格 32×30、今天 `#409EFF` 加粗、选中格 `#409EFF` 底白字、星期表头英文（Sun…Sat）、月份英文；type=date 无底部按钮 |
| `el-time-picker` | 外观 el-input + 前缀 Clock 图标，可清空；面板为时 / 分两列滚轮（format `HH:mm` 无秒列），底部英文按钮 **Cancel / OK** |
| `el-color-picker size=small` | 触发器 24×24，`padding:4px`，边 1px `#DCDFE6`、圆角 4；内部色块边 1px `#909399` 圆角 2，中间白色 ArrowDown（缩放 0.8）；面板 `padding:12px; width:300px`：SV 面板 280×180 + 竖向 hue 条 12×180，预设色 20×20 `gap:8px` 圆角 4（选中 `box-shadow:0 0 3px 2px #409EFF`），底部 hex 输入框（160px）+ **Clear**（文字按钮）+ **OK**（plain 按钮）；无透明度 |
| `el-dropdown-menu` | 浮层白底、圆角 4、阴影 box-shadow-light；`padding:5px 0`；项 `padding:5px 16px; line-height:22px; 14px #606266`；hover 底 `#ECF5FF` 字 `#409EFF` |
| `el-tooltip`（dark） | 底 `#303133`、字白、12px、`line-height:20px`、`padding:5px 11px`、圆角 4、带箭头 |
| `el-dialog` | 白底、阴影 box-shadow、`padding:16px`；header `padding-bottom:16px`；右上角关闭按钮 48×48（Close 图标 16px `#909399`，hover `#409EFF`）；footer `padding-top:16px`、右对齐；遮罩 `rgba(0,0,0,.5)`；开合动画 fade+zoom 0.3s |
| `ElMessageBox.confirm` | 遮罩 `rgba(0,0,0,.5)`；盒子水平垂直居中，`max-width:420px; width:100%`，`padding:12px`，圆角 4，阴影 box-shadow；标题 18px `#303133` 行高 24；右上关闭 X 40×40（16px `#909399`）；内容区：`type:'warning'` → 左侧 WarningFilled 24px `#E6A23C`，`gap:12px`，消息 14px `#606266`；按钮行右对齐 `padding-top:12px`：先「取消类」默认按钮、后「确认类」primary 按钮（default 尺寸 32 高）；点遮罩 / Esc / X = 取消 |
| `ElMessage` | 顶部居中，距顶 16px，多条向下叠放（间隔 16）；`padding:11px 15px`，圆角 4，1px 边；内容 14px、`line-height:1`、图标与文字 `gap:8px`；默认 3000ms 自动消失；Esc 关闭提示。类型色：success 底 `#F0F9EB` 边 `#E1F3D8` 字 `#67C23A`；warning 底 `#FDF6EC` 边 `#FAECD8` 字 `#E6A23C`；error 底 `#FEF0F0` 边 `#FDE2E2` 字 `#F56C6C`；info 底 `#F4F4F5` 边 `#E9E9EB` 字 `#909399` |
| `el-image-viewer` | 全屏遮罩 `#000` 50% 不透明；关闭按钮右上 (40,40) 44×44 圆形（底 `#606266`、白 Close 24px）；左右翻页按钮 44×44 圆形距左右 40；底部工具条 `bottom:30px; height:44px; radius:22px; padding:0 23px`（底 `#606266`）：缩小 / 放大 / 适配⇄原始尺寸 / 左旋 / 右旋；Esc 关闭；滚轮缩放、拖动平移；`infinite` 循环 |

### 1.3 失败提示：`notifyError(e, msg)`（utils/notify.ts）

- 控制台 `console.error(msg, e)`；界面 `ElMessage.error(detail ? \`${msg}：${detail}\` : msg)`（中文全角冒号）。
- `detail = errorMessage(e)`：字符串原样；Error 取 `.message`；含 string `message` 的对象取之；否则 `String(e ?? '')`；`trim()` 后超过 200 字符截断为前 200 字符 + `…`。

### 1.4 自绘标题栏的拖拽规则（三个可拖动窗口共用）

- 标题栏元素带 `data-tauri-drag-region="deep"`，同时 `@mousedown="onHeaderMouseDown"`：
  ```
  if e.buttons !== 1 → return
  if target 在 [data-tauri-drag-region="false"] 内 → return
  if target 在 button / input / textarea / select / a / [role=button] 内 → return
  e.preventDefault(); appWindow.startDragging()
  ```
- Tauri 2.9.5 的 `drag.js`（document 级 mousedown）**只看 `e.target` 自身**是否带该属性：鼠标按在标题栏容器本身（空白处）时，单击开始拖动、**双击切换最大化**（`internal_toggle_maximize`，窗口可缩放时生效）；按在标题文字（h2/span 子元素）上只拖动、双击不最大化。
- 拖动区及其后代 `user-select:none; cursor:default`（main.scss L87-101），`[data-tauri-drag-region="false"]` 区域恢复可选。

### 1.5 "模态子窗口"机制

- 打开方用 `isModalOpen` 标志 + 全窗口遮罩模拟模态；子窗口 `parent: <打开方窗口>`（Windows 上为 owner 关系：子窗口始终在 owner 之上、随 owner 最小化、不单独占任务栏按钮）。
- 打开方遮罩 `mousedown` 或打开方窗口获得焦点 → `activeModalWindow.setFocus()` 把子窗口带回前台（MainView、CompletedView 有；EditorView 的遮罩**没有**这个回焦逻辑，见 §3.15）。
- 子窗口 `tauri://destroyed` → 清标志、刷新打开方数据；`tauri://error` → 只清标志。
- 同一时刻每个打开方只允许一个子窗口（`if (isModalOpen) return`）。

### 1.6 生产构建的浏览器快捷键屏蔽（main.ts，所有窗口）

右键菜单禁用；F5、F12、Ctrl/Cmd + R/U/S/P/G/F/J/H/D/E、Ctrl+Shift + I/J/C/R/N 被 `preventDefault`。不含 Esc 与 Ctrl+Enter。GPUI 无此需求（无浏览器行为），列出仅为说明"Ctrl+S 不是保存"。

---

## 2. 窗口打开参数汇总

全部窗口（除通知）：`decorations:false`、`transparent:false`、`resizable:true`、未设 `minWidth/minHeight`（可任意缩小）、未设 `alwaysOnTop`、Tauri 默认 `shadow:true`（Windows 上无边框窗口有 DWM 阴影与 1px 边，Win11 圆角）、默认获得焦点。位置都是"在**打开方窗口所在显示器**上居中"（逻辑像素）。

| 窗口 | 打开方 / 触发 | label | url（hash 路由） | OS 标题 | 宽×高 | parent |
|---|---|---|---|---|---|---|
| 待办编辑器（已有） | MainView：列表 / 四象限 / 日历点击待办（`openEditor(todo)`） | `editor-${Date.now()}` | `#/editor?id={id}&mode=view` | `待办详情` | 1080×600 | 主窗口 |
| 待办编辑器（新建） | MainView：右下角悬浮 + 按钮 `openEditor()` | 同上 | `#/editor` | `新建待办` | 1080×600 | 主窗口 |
| 待办编辑器（新建，托盘） | 托盘「添加待办项」→ 事件 `tray-add-todo` → `openEditor(undefined, true)` | 同上 | `#/editor` | `新建待办` | 1080×600 | **无**（独立顶层窗口，会出现在任务栏） |
| 待办编辑器（从已完成） | CompletedView 点击行 | `editor-${Date.now()}` | `#/editor?id={id}&mode=view` | `待办详情` | 1080×600 | 已完成窗口 |
| 子任务窗口 | EditorView 子任务行的「查看 / 编辑」按钮 | `subtask-${mode}-${Date.now()}`（mode=`view`/`edit`） | `#/subtask-editor?id={subtaskId}[&memory=1][&mode=view]` | `查看子任务` / `编辑子任务` | 800×750 | 编辑器窗口 |
| 已完成 | 主窗口 TitleBar「已完成」按钮（title="已完成"）→ `openCompletedWindow()` | `completed-${Date.now()}` | `#/completed` | `已完成` | 460×550 | 主窗口 |
| 应用内通知 | Rust 通知调度器（通知类型 = `app`） | `notification_{counter}`（进程内自增，从 0 起） | `index.html#/notification?title=…&description=…&label=…` | `通知` | 320×120（内尺寸） | 无 |

居中公式（逻辑像素）：
```
monitor = currentMonitor() ?? primaryMonitor()      // 打开方窗口所在显示器
if monitor:
  s = monitor.scaleFactor
  x = round(monitor.position.x/s + (monitor.size.width/s  - W)/2)
  y = round(monitor.position.y/s + (monitor.size.height/s - H)/2)
else:   // 兜底：相对打开方窗口居中
  s = appWindow.scaleFactor(); pos = appWindow.outerPosition(); size = appWindow.outerSize()
  x = round(pos.x/s + (size.width/s - W)/2); y = round(pos.y/s + (size.height/s - H)/2)
```
注意是**整个显示器**居中（不是 work area）。编辑器宽 1080 = 左侧 700 + 右侧子任务面板 380。

通知窗口参数见 §11.1。

---

## 3. 待办编辑器窗口（EditorView.vue）

### 3.1 路由参数与模式

| query | 含义 |
|---|---|
| `id` | 待办 id；`parseInt`。无 id = 新建 |
| `mode=view` | 只读详情；**仅当同时有 id 时**生效（`isViewMode = mode==='view' && !!id`） |

状态：
- `todoId: number|null`（可变：新建后若后续步骤失败会原地切成该 id，见 §3.8）
- `isEdit = todoId !== null`（"已存在的待办"）
- `isViewMode`：初值见上；点击 [编辑] 置 false；**没有从编辑切回只读的入口**

三种形态：

| 形态 | 条件 | 标题栏 h2 | 左侧 | 底部栏 | 子任务面板 |
|---|---|---|---|---|---|
| 只读详情 | isViewMode | `待办详情` | §3.5 只读排版 | 不显示 | 可勾选完成、可拖拽排序、可「查看」；隐藏添加区、编辑 / 删除按钮、双击改名 |
| 编辑已有 | !isViewMode && isEdit | `编辑待办` | §3.6 表单 | 完成任务/重新打开 + 取消 + 保存 | 全功能（含导入） |
| 新建 | !isEdit | `新建待办` | §3.6 表单（标题框自动聚焦） | 取消 + 创建 | 子任务暂存内存（§3.13），无导入按钮、不能双击改名 |

### 3.2 表单状态与默认值

| 字段 | 新建默认值 | 从 `get_todo` 加载 |
|---|---|---|
| `form.title` | `''` | `todo.title` |
| `form.description` | `''` | `todo.description || ''`（存储形式 Markdown） |
| `form.color` | `'#10B981'`（DEFAULT_COLOR） | `todo.color` |
| `form.quadrant` | `4`（不紧急不重要） | `todo.quadrant` |
| `form.notifyAt` | `null` | `todo.notifyAt` |
| `form.notifyBefore` | `15` | `todo.notifyBefore` |
| `form.startTime` / `endTime` | `null` | 同名字段 |
| 拆分字段 `notifyDate/notifyTime`、`startDate/startTimeValue`、`endDate/endTimeValue` | `null` | `splitDateTime(value, kind)` → `{date:'YYYY-MM-DD', time:'HH:mm'}`；kind 分别 `notify`/`start`/`end` |
| `customNotifyBefore` | `15` | 若 `notifyBefore ∉ {0,5,15,30,60}` → `isCustomNotifyBefore=true; customNotifyBefore=notifyBefore` |
| `isCustomNotifyBefore` | `false` | 同上 |
| `repeatEnabled` | `false` | `!!todo.repeatEnabled` |
| `repeatType` | `'daily'` | `todo.repeatType` 非空时取之（否则保留 'daily'） |
| `repeatInterval` | `1` | `todo.repeatInterval || 1` |
| `repeatWeekdays: number[]` | `[]` | `"1,3,5".split(',').map(Number).filter(1..7)`；空 → `[]` |
| `repeatMonthDay` | `1` | `todo.repeatMonthDay || 1` |
| `originalNotifyAt/StartTime/EndTime` | `null` | 加载值（用于判断"清除"） |

拆分字段与 form 的联动（watch，立即同步）：
- `[notifyDate, notifyTime]` 变 → `form.notifyAt = composeDateTime(notifyDate, notifyTime, 'notify')`
- `[startDate, startTimeValue]` 变 → `form.startTime = composeDateTime(…, 'start')`
- `[endDate, endTimeValue]` 变 → `form.endTime = composeDateTime(…, 'end')`
- 只选日期不选时间时，存储值补默认时刻（提醒 09:00:00、开始 00:00:00、截止 23:59:00），**但时间选择器本身仍为空**（显示 placeholder "时间"）。
- 清空日期 → 存储值变 null；时间选择器保留旧值但被禁用。

`loadTodo(refreshForm = true)`：
```
if !todoId → return
todo = invoke('get_todo', { id })          // 失败：notifyError(e, '加载待办失败')
if refreshForm: 用 todo 重建 form + 拆分字段 + 自定义提前 + 重复设置（上表）
```
`refreshForm=false`（"只刷新当前待办"）：只更新 `todo`（子任务列表与 completed 状态来源），**不重建左侧表单**，避免冲掉用户未保存的修改（issue #11）。所有子任务操作后、子任务编辑窗口关闭后都用 `loadTodo(false)`。

挂载顺序（onMounted）：绑定快捷键 → 有 id 则 `await loadTodo()`，否则聚焦标题输入框 → `await markFormClean()`（等一帧让 watcher 把时间规范化后再拍快照）。

### 3.3 整体布局（元素树）

```
div.editor-window            flex(row); height:100vh; bg:#FFFFFF
├─ div.main-area             flex:1; flex-direction:column; min-width:0     （宽 = 窗口宽 − 380）
│  ├─ div.window-header      标题栏，§3.4
│  ├─ div.view-content       只读排版（isViewMode）§3.5
│  │  或 div.editor-content  表单（!isViewMode）§3.6
│  └─ div.window-footer      底部栏（!isViewMode）§3.7
├─ div.subtask-panel         右侧子任务面板，固定宽 380，§3.11
├─ div.modal-overlay         子任务窗口打开时（isSubtaskEditorOpen）§3.15
└─ el-dialog                 描述放大编辑弹窗（append-to-body）§3.14
```

### 3.4 标题栏 `.window-header`

| 属性 | 值 |
|---|---|
| 布局 | flex，`align-items:center; justify-content:space-between` |
| padding | `12px 16px` |
| min-height | 44px（box-sizing:border-box） |
| 实际高度 | 内容最高是 32px 的 el-button → 12+32+12 = 56 + 下边框 1 = **57px**（与右侧面板头 57px 对齐） |
| border-bottom | `1px solid #E2E8F0` |
| 拖拽 | §1.4 |

子元素：
1. `h2`：文字见 §3.1；`font-size:16px; font-weight:600; line-height:1.2; margin:0`；颜色继承 `#1E293B`。
2. `div.header-actions`（`data-tauri-drag-region="false"`）：flex，`align-items:center; gap:2px`；相邻 el-button `margin-left:0`（覆盖 EP 的 12px）。
   - [编辑]（仅 isViewMode）：`el-button text type="primary"`（default 尺寸：高 32、padding 8px 15px、14px、`#409EFF`，hover 底 `#F5F7FA`）；内容 `Edit` 图标（14px）+ `<span>编辑</span>`，图标与文字间距 12px（§1.2）。点击：`isViewMode = false`（原地切换，不重新加载、不重拍快照）。
   - [关闭]：`el-button text`（字 `#606266`），只有 `Close` 图标 14px（按钮 44×32）。点击 `handleClose()` = **直接关窗，不做未保存确认**（只有 Esc 走确认，见 §3.10）。

### 3.5 只读详情排版 `.view-content`

```
div.view-content                 flex:1; padding:24px 28px; overflow-y:auto
├─ h1.view-title                 {{ form.title }}
├─ div.view-meta                 徽章行
│  ├─ span.meta-badge            象限徽章（总是显示）
│  │  ├─ span.badge-dot          8×8 圆点，bg = 象限色
│  │  └─ 文本 象限名
│  └─ span.meta-badge.notify-badge   提醒徽章（notifyStatusText 非空时）
│     ├─ el-icon Bell 13px
│     └─ 文本 notifyStatusText
└─ MarkdownEditor readonly .view-markdown    （form.description 非空）
   或 div.view-empty-desc "暂无描述"
```

| 节点 | 样式 |
|---|---|
| `h1.view-title` | `margin:0 0 14px; font-size:22px; font-weight:650; line-height:1.35; color:#1E293B; word-break:break-word`；`todo.completed` 为真时 `color:#94A3B8; text-decoration:line-through` |
| `.view-meta` | flex，`flex-wrap:wrap; align-items:center; gap:8px; margin-bottom:18px` |
| `.meta-badge` | inline-flex，`align-items:center; gap:6px; padding:3px 10px; border-radius:999px; font-size:12px; font-weight:500; line-height:1.6` |
| 象限徽章 | `color = quadrant.color`；`background = quadrant.bgColor`（见下表） |
| `.badge-dot` | 8×8、`border-radius:50%`、`flex-shrink:0`、bg = quadrant.color |
| `.notify-badge` | `color:#64748B; background:#F1F5F9`；图标 Bell 13px 与文字 gap 6 |
| `.view-markdown` | MarkdownEditor 只读；其内部 `.milkdown` 的 `padding:0`（覆盖默认 12px 16px）；渲染样式见 §5.5 |
| `.view-empty-desc` | `color:#94A3B8; font-size:13px`，文字 `暂无描述` |

象限定义（types/todo.ts `QUADRANT_INFO`，顺序即选择器网格顺序）：

| id | name | color | bgColor |
|---|---|---|---|
| 1 | 重要且紧急 | `#EF4444` | `rgba(239, 68, 68, 0.1)` |
| 2 | 重要不紧急 | `#F59E0B` | `rgba(245, 158, 11, 0.1)` |
| 3 | 紧急不重要 | `#3B82F6` | `rgba(59, 130, 246, 0.1)` |
| 4 | 不紧急不重要 | `#10B981` | `rgba(16, 185, 129, 0.1)` |

"优先级展示"就是这枚象限徽章（只读模式没有单独的优先级字段）。只读模式**不显示**开始 / 截止时间、颜色。

`notifyStatusText`（只读提醒文案，逐字）：
```
fmt(v) = formatDateTime(v, 'YYYY-MM-DD HH:mm', 'notify') ?? v       // 空值 → ''
if repeatEnabled:
  unit = {daily:'天', weekly:'周', monthly:'月'}[repeatType] || '天'
  text = `每 ${repeatInterval} ${unit}重复`
  if repeatType==='weekly' && repeatWeekdays 非空:
     names = 按 周一..周日 固定顺序过滤出选中的 label，用 '、' 连接
     text += `（${names}）`
  else if repeatType==='monthly':
     text += `（每月 ${repeatMonthDay} 号）`
  if form.notifyAt: text += `，下次 ${fmt(form.notifyAt)}`
  return text
if form.notifyAt:
  text = `提醒 ${fmt(form.notifyAt)}`
  if form.notifyBefore > 0: text += `（提前 ${form.notifyBefore} 分钟）`
  return text
return ''                         // 不显示徽章
```
例：`每 2 周重复（周一、周三），下次 2026-10-20 09:00`；`提醒 2026-10-20 14:30（提前 15 分钟）`。注意"每 1 天重复"与括号均为中文全角括号、中文逗号。

### 3.6 编辑表单 `.editor-content`

`.editor-content { flex:1; padding:16px; overflow-y:auto }`；内含 `el-form label-position="top"`（无校验规则，无 label-width）。

**el-form-item（label-position=top）通用外观**：块级；`margin-bottom:18px`；label `display:block; width:fit-content; line-height:22px; margin-bottom:8px; font-size:14px; color:#606266`；content 区 `display:flex; flex-wrap:wrap; align-items:center; line-height:32px; font-size:14px`。`required` 项在 label 前加红色 `*`（`#F56C6C`，`margin-right:4px`），仅视觉，不做校验提示。

表单项依次（`v-if` 条件见各行）：

| # | label | 组件与 props | 说明 |
|---|---|---|---|
| 1 | `标题`（required，带红 *） | `el-input v-model=form.title placeholder="请输入待办标题" maxlength="100"`（default 尺寸 32 高、宽 100%） | 新建时挂载即 `focus()`。空标题不阻止输入，保存时静默忽略（见 §3.8） |
| 2 | 自定义 label：`描述` + 放大按钮 | `MarkdownEditor v-model=form.description class="description-editor"` | 详见下 |
| 3 | `颜色` | 8 个预设色按钮 + `el-color-picker v-model=form.color :predefine=8 色 size="small"` | 详见下 |
| 4 | `四象限` | 2×2 网格自定义按钮 | 详见下 |
| 5 | `时间范围` | 两行：`el-date-picker`(开始日期)+`el-time-picker`；`el-date-picker`(截止日期)+`el-time-picker` | 详见下 |
| 6 | `提醒时间`；重复开启时 label 改为 `首次提醒` | `el-date-picker`(选择日期) + `el-time-picker`(时间) | 详见下 |
| 7 | `提前提醒`（仅 `form.notifyAt && !repeatEnabled`） | `el-select` + 自定义时 `el-input-number` | 详见下 |
| 8 | `重复` | `el-switch v-model=repeatEnabled` | |
| 9 | `重复方式`（仅 repeatEnabled） | `el-select v-model=repeatType style="width:100%"`，选项 `每天`=`daily` / `每周`=`weekly` / `每月`=`monthly` | |
| 10 | 动态 label：`'每 ' + repeatInterval + ' ' + 单位`，单位 天/周/月（如 `每 1 天`、`每 3 周`）（仅 repeatEnabled） | `el-input-number v-model=repeatInterval :min=1 :max=99 style="width:100%"` | label 随数字实时变化 |
| 11 | `星期`（仅 repeatEnabled && weekly） | `el-checkbox-group v-model=repeatWeekdays class="weekday-checkbox-group"`，7 个 `el-checkbox`：`周一`=1 … `周日`=7 | `.weekday-checkbox-group { display:flex; flex-wrap:wrap; gap:4px 0 }`；每个 checkbox 右边距 30（最后 0），窄时换行。**不校验**至少选一天 |
| 12 | `每月几号`（仅 repeatEnabled && monthly） | `el-input-number v-model=repeatMonthDay :min=1 :max=31 style="width:100%"` | |

#### 3.6.2 描述（Markdown）

label 插槽：
```
div.desc-label            flex; align-items:center; gap:6px
├─ span "描述"
└─ button.desc-expand-btn  title="放大编辑"  → openDescDialog()（§3.14）
   └─ el-icon FullScreen 14px
```
`.desc-expand-btn`：22×22，flex 居中，`padding:0; color:#94A3B8; background:transparent; border:none; border-radius:4px; cursor:pointer; transition: all .15s`；hover `color:#3B82F6; background:#EFF6FF`。

编辑器容器 `.description-editor`（就是 MarkdownEditor 根元素）：
| 属性 | 值 |
|---|---|
| width | 100% |
| border | `1px solid #E2E8F0`；`:focus-within` 变 `#3B82F6`（过渡 border-color .15s） |
| border-radius | 8px |
| min-height / max-height | 120px / 280px，`overflow-y:auto`（超出在容器内滚动） |
| background | `#FFFFFF` |
| 内部 `.milkdown` | `padding:8px 12px; min-height:110px` |
| 内部 `.ProseMirror` | `min-height:100px` |

编辑行为见 §5（所见即所得、图片粘贴上传、链接 Ctrl+点击打开等）。无 placeholder。

#### 3.6.3 颜色

```
div.color-picker-row        flex; align-items:center; gap:8px
├─ button.color-preset-btn × 8    title=色名；bg=色值；点击 form.color = 色值
└─ el-color-picker size=small     predefine = 8 个色值
```
预设色（`PRESET_COLORS`，顺序即显示顺序）：

| name（title 提示） | value |
|---|---|
| 红色 | `#EF4444` |
| 橙色 | `#F59E0B` |
| 黄色 | `#EAB308` |
| 绿色 | `#10B981` |
| 青色 | `#06B6D4` |
| 蓝色 | `#3B82F6` |
| 紫色 | `#8B5CF6` |
| 粉色 | `#EC4899` |

`.color-preset-btn`：24×24、`border-radius:4px; border:2px solid transparent; padding:0; cursor:pointer; transition: all .15s`；hover `transform: scale(1.1)`；激活（`form.color === value`，**区分大小写的字符串相等**）`border-color:#3B82F6; box-shadow:0 0 0 2px rgba(59,130,246,.3)`。
自定义颜色由 el-color-picker 写回（hex 字符串）；面板点 **Clear** 会把 `form.color` 设为 `null`（新建时随后 `create_todo` 因 `color: String` 反序列化失败而报"创建待办失败：…"；更新时 `color` 缺省 = 不修改）。

#### 3.6.4 四象限

```
div.quadrant-picker              display:grid; grid-template-columns:1fr 1fr; gap:8px; width:100%
└─ button.quadrant-btn × 4       顺序：1 重要且紧急 | 2 重要不紧急 / 3 紧急不重要 | 4 不紧急不重要
   ├─ span.quadrant-indicator    10×10 圆，bg=象限色，flex-shrink:0
   └─ span.quadrant-name         象限名
```
| 属性 | 值 |
|---|---|
| `.quadrant-btn` | flex，`align-items:center; gap:8px; padding:10px 12px; background:<bgColor>; border:2px solid transparent; border-radius:8px; cursor:pointer; transition: all .2s ease`；内容左对齐 |
| hover | `border-color: <象限色>` |
| 选中（`form.quadrant === id`） | `border-color:<象限色>; box-shadow: 0 0 0 2px <象限色 30% 不透明度>`（color-mix 30% + transparent） |
| `.quadrant-name` | `font-size:12px; color:#334155; font-weight:500` |

点击 `handleQuadrantSelect(id)`：
```
form.color = resolveQuadrantColor(form.color, oldQuadrant, id)
form.quadrant = id
resolveQuadrantColor(cur, prev, next) =
  isSameColor(cur, colorOf(prev)) ? colorOf(next) : cur      // isSameColor：trim + 小写后比较
colorOf(q) = QUADRANT_INFO[q].color ?? '#10B981'
```
即：颜色还是旧象限默认色（用户没改过）时跟随新象限变色；手动挑过的颜色保留。新建默认色 `#10B981` = 象限 4 色，所以新建时点"重要且紧急"颜色会自动变 `#EF4444`。

#### 3.6.5 时间范围

```
div.time-range-picker          width:100%; flex column; gap:8px
├─ div.time-range-row          flex; align-items:center; gap:8px
│  ├─ el-date-picker.date-picker-sm   v-model=startDate  type=date placeholder="开始日期" format/value-format="YYYY-MM-DD"   flex:1
│  └─ el-time-picker.time-picker-sm   v-model=startTimeValue placeholder="时间" format/value-format="HH:mm"  width:90px; flex-shrink:0; :disabled="!startDate"
└─ div.time-range-row
   ├─ el-date-picker  v-model=endDate placeholder="截止日期"（同上）
   └─ el-time-picker  v-model=endTimeValue placeholder="时间" :disabled="!endDate"（宽 90）
```
所有日期 / 时间选择器：`teleported`，`popper-options = { placement:'top-start', modifiers:[{name:'flip', enabled:false}] }` → **面板总是向上弹出、左对齐，不翻转**。

#### 3.6.6 提醒时间

```
div.notify-datetime-picker     flex; gap:8px; width:100%
├─ el-date-picker.date-picker  v-model=notifyDate placeholder="选择日期" (YYYY-MM-DD)   flex:1
└─ el-time-picker.time-picker  v-model=notifyTime placeholder="时间" (HH:mm)  width:100px; flex-shrink:0; :disabled="!notifyDate"
```

#### 3.6.7 提前提醒（`form.notifyAt && !repeatEnabled`）

- `el-select :model-value="isCustomNotifyBefore ? -1 : form.notifyBefore" style="width:100%" @change=handleNotifyBeforeChange`

| 选项 label | value |
|---|---|
| 准时 | 0 |
| 5 分钟前 | 5 |
| 15 分钟前 | 15 |
| 30 分钟前 | 30 |
| 1 小时前 | 60 |
| 自定义 | -1 |

```
handleNotifyBeforeChange(v):
  if v === -1: isCustomNotifyBefore = true;  form.notifyBefore = customNotifyBefore
  else:        isCustomNotifyBefore = false; form.notifyBefore = v
```
- 自定义时下方追加 `el-input-number v-model=customNotifyBefore :min=1 :max=1440 style="width:100%; margin-top:8px"`，后缀插槽文字 `分钟前`。切回预设再切回自定义时保留上次自定义数值。

#### 3.6.8 重复

见表单项 8-12。开启重复时：提醒项 label 改为"首次提醒"、"提前提醒"整项隐藏、保存时 `notifyBefore` 发 0。

### 3.7 底部栏 `.window-footer`（仅编辑形态）

| 属性 | 值 |
|---|---|
| 布局 | flex，`justify-content:space-between; align-items:center; gap:12px; flex-wrap:wrap` |
| padding | `10px 16px` |
| border-top | `1px solid #E2E8F0` |
| 高度 | 10 + 24 + 10 + 1 = 45px |

唯一子元素 `div.footer-right`（flex，`gap:6px; flex-shrink:0`）。**注意**：`space-between` 只有一个子元素时等同 `flex-start`，所以按钮组实际**靠左**排列（这是现状）。相邻 el-button 之间还有 EP 的 `margin-left:12px`，所以按钮间距实际 = 6 + 12 = **18px**。

按钮（全部 `size="small"`：高 24、12px、padding 5px 11px、圆角 6；图标后是裸文字，间距 6）：

| 按钮 | 条件 | 组件 | 内容 | 行为 |
|---|---|---|---|---|
| 完成任务 | `isEdit && todo && !todo.completed` | `type="success" plain`，`:loading=isUpdatingCompleteState` | `CircleCheck` + `完成任务` | `updateTodoCompleted(true)` |
| 重新打开 | `isEdit && todo && todo.completed` | `type="warning" plain`，loading 同上 | `RefreshLeft` + `重新打开` | `updateTodoCompleted(false)` |
| 取消 | 总是 | default | `Close` + `取消` | `handleClose()` 直接关窗（不确认） |
| 保存 / 创建 | 总是 | `type="primary"`，`:loading=saving :disabled=saving` | saving 时不渲染图标（只显示 loading 圈）；否则 isEdit ? `Check` : `Plus`；文字 isEdit ? `保存` : `创建` | `handleSave()` |

注意：保存按钮在标题为空时**不禁用**（与子任务窗口不同）。

### 3.8 保存逻辑（字段映射与流程）

#### 3.8.1 handleSave（防重入）

```
if saving → return                     // 防双击 / 连按 Ctrl+Enter
if form.title.trim() === '' → return   // 静默，无提示
if isCustomNotifyBefore: form.notifyBefore = customNotifyBefore
saving = true
try: todoId !== null ? saveExistingTodo(todoId) : saveNewTodo()
finally: saving = false
```
标题**不 trim** 原样提交（只用 trim 判空）。

#### 3.8.2 新建 saveNewTodo

1. `invoke('create_todo', { data })`，`data: CreateTodoRequest`：

| 字段 | 值 |
|---|---|
| `title` | `form.title` |
| `description` | `toStorageMarkdown(form.description) || undefined`（空串不发） |
| `color` | `form.color` |
| `quadrant` | `form.quadrant` |
| `notifyAt` | `form.notifyAt || undefined` |
| `notifyBefore` | `repeatEnabled ? 0 : form.notifyBefore` |
| `startTime` / `endTime` | `form.startTime || undefined` / `form.endTime || undefined` |

   失败 → `notifyError(e, '创建待办失败')`，结束（窗口不关）。
2. `create_todo` 不接受重复字段：若 `repeatEnabled`，再 `invoke('update_todo', { id: created.id, data: buildUpdateRequest() })`；失败 → `notifyError(e, '待办已创建，但重复提醒未保存，请点击「保存」重试')`，`ok=false`。
3. ok 时 `flushPendingSubtasks(created.id)`（§3.13.3）。
4. 若 2 或 3 失败：`enterEditState(created)`（`todoId=created.id; todo=created; original*=created.*`）+ `loadTodo(false)` → 窗口留着并变成"编辑待办"形态（标题、按钮"保存"、出现导入按钮、若有残留暂存子任务显示黄色提示条），用户再点保存走更新分支、不会重复建待办。
5. 全部成功：`ElMessage.success('待办已创建')`，`handleClose()` 关窗（提示在关闭中的窗口里，实际几乎看不到）。

#### 3.8.3 更新 saveExistingTodo(id)

1. `invoke('update_todo', { id, data: buildUpdateRequest() })` → 成功 `enterEditState(result)`；失败 `notifyError(e, '保存待办失败')` 结束。
2. 若还有 `pendingSubtasks`（新建时没建成的）→ `flushPendingSubtasks(id)`；失败 → `loadTodo(false)` 结束（窗口不关）。
3. 成功：`ElMessage.success('待办已保存')`，关窗。

#### 3.8.4 buildUpdateRequest（UpdateTodoRequest，camelCase JSON）

```
shouldClearNotifyAt  = originalNotifyAt  !== null && !form.notifyAt
shouldClearStartTime = originalStartTime !== null && !form.startTime
shouldClearEndTime   = originalEndTime   !== null && !form.endTime
wasRepeatEnabled  = !!todo?.repeatEnabled
shouldClearRepeat = wasRepeatEnabled && !repeatEnabled

// 提醒时间没改就不发（后端收到 notifyAt 会把 notified 复位 → 已提醒过的待办改个标题后一分钟内又会再弹）
notifyAtUnchanged = isSameMinute(form.notifyAt, originalNotifyAt, 'notify')
                    && normalizeDateTime(originalNotifyAt, 'notify') === originalNotifyAt   // 库里本来就是规范格式
sendNotifyAt = !!form.notifyAt && (!notifyAtUnchanged || repeatEnabled)
```

| 字段 | 值 |
|---|---|
| `title` | `form.title` |
| `description` | `toStorageMarkdown(form.description)`（**清空时也发空串**：后端把 null/缺省当"不修改"） |
| `color` | `form.color` |
| `quadrant` | `form.quadrant` |
| `notifyAt` | `sendNotifyAt ? form.notifyAt : undefined` |
| `notifyBefore` | `repeatEnabled ? 0 : form.notifyBefore` |
| `clearNotifyAt` | `shouldClearNotifyAt` |
| `startTime` / `endTime` | `form.startTime || undefined` / `form.endTime || undefined`（有值就发，不比较是否改过） |
| `clearStartTime` / `clearEndTime` | 见上 |
| `clearRepeat` | `shouldClearRepeat` |
| `repeatEnabled` | `repeatEnabled || undefined`（只发 true 或不发） |
| `repeatType` | `repeatEnabled ? repeatType : undefined` |
| `repeatInterval` | `repeatEnabled ? repeatInterval : undefined` |
| `repeatWeekdays` | `repeatEnabled && weekly && 非空 ? 升序后 join(',')（如 "1,3,5"） : undefined`（weekly 但一个没选 → 不发 → 后端保留旧值） |
| `repeatMonthDay` | `repeatEnabled && monthly ? repeatMonthDay : undefined` |

后端 `update_todo` 语义（todo.rs L142-254）：所有时间字段先 `normalize_datetime`（空串 = 未填；无法识别报错 `提醒时间格式无法识别：…` / `开始时间格式无法识别：…` / `截止时间格式无法识别：…`）；`clearNotifyAt` → `notify_at=NULL, notified=0`，否则有 notifyAt → 写入并 `notified=0`；`clearRepeat` → `repeat_enabled=0, repeat_type=NULL, repeat_interval=1, repeat_weekdays=NULL, repeat_month_day=NULL, notify_before=0`；`repeatEnabled=true` 同时 `notify_before=0`；没有任何字段 → 错误 `No fields to update`；`updated_at` 严格递增。返回更新后的 Todo（含 subtasks）。`create_todo`：`quadrant` 缺省 4、`notifyBefore` 缺省 0、`sort_order = 当前未完成最大值 + 1`。

#### 3.8.5 完成 / 重新打开 updateTodoCompleted(completed)

```
if !isEdit || !todoId || isUpdatingCompleteState → return
if todo.completed === completed → return
isUpdatingCompleteState = true
invoke('update_todo', { id, data: { completed } }) → 成功 handleClose()（无提示）
失败 → notifyError(e, completed ? '标记完成失败' : '重新打开失败')
finally isUpdatingCompleteState = false
```
注意：只提交 `completed`，表单里未保存的修改会随关窗丢弃。

### 3.9 未保存修改快照

```
formSnapshot() = JSON.stringify([
  form.title, toStorageMarkdown(form.description), form.color, form.quadrant, form.notifyAt,
  isCustomNotifyBefore ? customNotifyBefore : form.notifyBefore,
  form.startTime, form.endTime,
  repeatEnabled, repeatType, repeatInterval, 升序(repeatWeekdays), repeatMonthDay,
  pendingSubtasks.map(s => [s.title, s.content, s.completed]),
])
```
- 组件创建时先按空表单拍一次（加载完成前按 Esc 不会误报）；`onMounted` 加载后 `markFormClean()`（nextTick 后重拍）。
- 子任务的增删改（已存在待办）直接落库，不计入快照；只有新建时的暂存子任务计入。

### 3.10 关闭与确认

| 入口 | 行为 |
|---|---|
| 标题栏 X、底部「取消」 | `appWindow.close()` 直接关，**不确认** |
| Esc（§8） | `closeWithConfirm()` |
| 保存成功 / 完成成功 | 自动关窗 |

```
closeWithConfirm():
  if saving || isUpdatingCompleteState → return          // 进行中的操作结束后会自行关窗
  if !isViewMode && formSnapshot() !== cleanSnapshot:
     ElMessageBox.confirm('有未保存的修改，确定放弃并关闭吗？', '放弃修改',
        { confirmButtonText:'放弃', cancelButtonText:'继续编辑', type:'warning' })
     取消 → return
  handleClose()
```

### 3.11 右侧子任务面板 `.subtask-panel`

```
div.subtask-panel                    width:380px; flex-shrink:0; flex column; bg:#FAFBFC; border-left:1px solid #E2E8F0
├─ div.panel-header                  拖拽区（同 §1.4）
│  └─ h3 "子任务"
└─ div.panel-content                 flex:1; padding:16px; overflow-y:auto; flex column
   ├─ div.subtask-progress           （列表非空）
   │  ├─ div.progress-info
   │  │  ├─ span.progress-text       "{done} / {total}"
   │  │  └─ span.progress-label      "已完成"
   │  └─ div.progress-bar
   │     └─ div.progress-fill        width = percent%
   ├─ div.add-subtask                （!isViewMode）
   │  └─ div.add-subtask-input
   │     ├─ el-icon.input-icon Plus
   │     ├─ input  placeholder="添加子任务..."
   │     ├─ button.add-btn  (Plus + "添加")       （输入 trim 后非空时，淡入淡出 .15s）
   │     └─ el-dropdown(trigger=click) > button.import-btn (Upload)   （isEdit）
   ├─ div.pending-subtask-hint       （isEdit && pendingSubtasks.length>0）
   │  ├─ el-icon Warning 14
   │  └─ span "{n} 个子任务尚未创建，点击「保存」重试"
   ├─ div.subtask-list-editor        （列表非空）
   │  └─ draggable(handle=.subtask-drag-handle) > div.subtask-item-editor × N
   └─ div.subtask-empty              （列表为空）
      ├─ el-icon.empty-icon List
      └─ span "暂无子任务"
```

当前列表 `currentSubtaskList = isEdit ? todo.subtasks : pendingSubtasks`（后端按 `sort_order ASC, id ASC` 返回；**纯手工顺序**，完成的不沉底）。新建后部分失败转成编辑态时，列表显示已落库的子任务，残留的暂存项只体现在黄色提示条里。

| 节点 | 样式 |
|---|---|
| `.panel-header` | flex，`align-items:center; justify-content:space-between; padding:12px 16px; min-height:57px; box-sizing:border-box; border-bottom:1px solid #E2E8F0; background:#FFFFFF` |
| `h3` | `margin:0; font-size:16px; font-weight:600; line-height:1.2; color:#334155` |
| `.subtask-progress` | flex，`align-items:center; gap:12px; margin-bottom:16px; padding:12px 14px; background: linear-gradient(135deg, #F0F9FF 0%, #E0F2FE 100%); border-radius:10px` |
| `.progress-info` | flex column，`min-width:50px` |
| `.progress-text` | `font-size:16px; font-weight:600; color:#0369A1` |
| `.progress-label` | `font-size:11px; color:#64748B` |
| `.progress-bar` | `flex:1; height:6px; background:#E2E8F0; border-radius:3px; overflow:hidden` |
| `.progress-fill` | `height:100%; background: linear-gradient(90deg, #3B82F6 0%, #06B6D4 100%); border-radius:3px; transition: width .3s ease`；`percent = total? Math.round(done/total*100) : 0` |
| `.add-subtask` | `margin-bottom:12px` |
| `.add-subtask-input` | flex，`align-items:center; gap:8px; padding:8px 12px; background:#F8FAFC; border:1px dashed #CBD5E1; border-radius:8px; transition: all .2s`；hover `border-color:#94A3B8; background:#F1F5F9`；`:focus-within` `border-color:#3B82F6; border-style:solid; background:#FFFFFF; box-shadow:0 0 0 3px rgba(59,130,246,.1)` |
| `.input-icon` | `color:#94A3B8; font-size:16px; flex-shrink:0` |
| `input` | `flex:1; border:none; outline:none; background:transparent; font-size:13px; color:#334155`；placeholder `#94A3B8` |
| `.add-btn` | inline-flex，`align-items:center; gap:6px; padding:4px 12px; font-size:12px; font-weight:500; color:#FFFFFF; background:#3B82F6; border:none; border-radius:6px; cursor:pointer; transition: all .15s`；hover `#2563EB`；active `transform: scale(.96)`；内容 Plus 图标 + `添加` |
| `.import-btn` | 28×28 flex 居中，`padding:0; color:#64748B; background:transparent; border:1px solid #CBD5E1; border-radius:6px; cursor:pointer; flex-shrink:0; transition: all .15s`；hover `color:#3B82F6; border-color:#3B82F6; background:#EFF6FF`；`title="导入子任务"`；图标 Upload 14px |
| `.pending-subtask-hint` | flex，`align-items:center; gap:6px; margin-bottom:12px; padding:8px 12px; border-radius:8px; background:#FFFBEB; color:#B45309; font-size:12px` |
| `.subtask-list-editor` | `flex:1; min-height:0; overflow-y:auto; padding-right:4px`；滚动条宽 4px、轨道透明、滑块 `#CBD5E1` 圆角 2 |
| `.subtask-empty` | `flex:1`，flex column 居中，`padding:24px 16px; color:#94A3B8; text-align:center`；图标 List `font-size:32px; margin-bottom:8px; opacity:.5`；文字 13px |

#### 3.11.1 子任务行 `.subtask-item-editor`

```
div.subtask-item-editor[.completed][.readonly]     （.readonly 无样式）
├─ el-icon.subtask-drag-handle  Rank 14px   title="拖拽排序"
├─ div.custom-checkbox[.checked]  → 点击切换完成
│  └─ el-icon.check-icon Check   （完成时）
├─ input.subtask-inline-input     （正在行内改名的这一行）
│  或 span.subtask-title          {{ title }}   双击 → 行内改名（仅 isEdit && !isViewMode）
└─ div.subtask-actions            （非改名中；hover 行时显示）
   ├─ button.action-btn.view-btn    View    title="查看子任务"  → 打开子任务窗口(view)
   ├─ button.action-btn.edit-btn    Edit    title="编辑子任务"  → 打开子任务窗口(edit)   （!isViewMode）
   └─ button.action-btn.delete-btn  Delete  title="删除子任务"  → deleteSubtask           （!isViewMode）
```

| 节点 | 样式 |
|---|---|
| 行 | flex，`align-items:center; gap:10px; padding:10px 12px; margin-bottom:6px`（最后一行 0）；`background:#FFFFFF; border:1px solid #E2E8F0; border-radius:8px; position:relative; user-select:none`；`transition: border-color .2s, box-shadow .2s, background .2s`（**不含 transform**，否则拖拽克隆体跟不上鼠标） |
| 行 hover | `border-color:#CBD5E1; box-shadow:0 2px 4px rgba(0,0,0,.04)`；显示 `.subtask-actions`；拖拽手柄 `opacity:1` |
| 行 `.completed` | `background:#F8FAFC; border-color:#E2E8F0`；标题 `text-decoration:line-through; color:#94A3B8` |
| 拖拽占位（ghost-class `dragging`） | `opacity:.5` |
| `.subtask-drag-handle` | `flex-shrink:0; color:#94A3B8; cursor:grab`（按下 `grabbing`）；`opacity:0`，行 hover 时 1，过渡 .15s |
| `.custom-checkbox` | 20×20，`border:2px solid #CBD5E1; border-radius:50%`，flex 居中，`cursor:pointer; transition: all .2s; flex-shrink:0`；hover 边 `#3B82F6`；`.checked`：`background: linear-gradient(135deg, #3B82F6 0%, #06B6D4 100%); border-color:transparent`，内部 Check 图标白色 12px |
| `.subtask-title` | `flex:1; font-size:13px; color:#334155; line-height:1.4; word-break:break-word; cursor:default; padding:2px 4px; border-radius:4px; transition: background .15s`；hover `background:#F1F5F9` |
| `.subtask-inline-input` | `flex:1; font-size:13px; color:#334155; line-height:1.4; padding:2px 4px; border:1px solid #3B82F6; border-radius:4px; outline:none; background:#FFFFFF; font-family:inherit; user-select:text` |
| `.subtask-actions` | 默认 `display:none`；hover 行时 flex；`align-items:center; gap:2px; position:absolute; right:6px; top:50%; transform:translateY(-50%); background:rgba(255,255,255,.95); padding:2px 4px; border-radius:4px; box-shadow:0 1px 4px rgba(0,0,0,.1); z-index:5`（浮在标题上方） |
| `.action-btn` | 24×24 flex 居中，`padding:0; background:transparent; border:none; border-radius:4px; cursor:pointer; color:#94A3B8; transition: all .15s`；图标约 14px（继承按钮字号） |
| view hover | `background:#E0E7FF; color:#6366F1` |
| edit hover | `background:#DBEAFE; color:#3B82F6` |
| delete hover | `background:#FEE2E2; color:#EF4444` |

#### 3.11.2 子任务行为

| 操作 | 已存在待办（isEdit） | 新建（!isEdit） |
|---|---|---|
| 添加（输入框回车 / 「添加」按钮） | `title=trim()` 非空 → `invoke('create_subtask', { data:{ parentId: todoId, title } })` → `loadTodo(false)` → 清空输入框；失败 `notifyError(e, '添加子任务失败')`（输入框保留） | 追加到 `pendingSubtasks`：`{ id: --counter（-1,-2,…）, title, content:null, completed:false }`，清空输入框 |
| 勾选完成 | `invoke('update_subtask', { id, data:{ completed: !cur } })` → `loadTodo(false)`；失败 `notifyError(e, '更新子任务失败')` | 本地取反 |
| 拖拽排序 | 结束后 `invoke('reorder_subtasks', { ids: 当前顺序 })`；失败 `notifyError(e, '子任务排序保存失败')` 并 `loadTodo(false)` 回滚到服务端顺序 | 只改数组顺序（保存时按此顺序创建） |
| 删除 | 确认后 `invoke('delete_subtask', { id })` → `loadTodo(false)`；失败 `notifyError(e, '删除子任务失败')` | 确认后从数组移除 |
| 双击改名 | 进入行内输入框（自动聚焦并全选）；Enter（非 IME 组合中）或失焦 → `trim` 非空则 `invoke('update_subtask', { id, data:{ title } })` → `loadTodo(false)`（失败 `notifyError(e, '修改子任务标题失败')`），空则取消；Esc → 取消并 `preventDefault`（不会触发窗口级 Esc 关窗） | 不可用 |
| 查看 / 编辑 | 打开子任务窗口（§3.13） | 打开内存模式子任务窗口 |

- 只读详情形态下：仍可勾选完成、拖拽排序（issue #9）、点「查看」；添加、编辑、删除、改名、导入均隐藏。
- 删除确认框：`ElMessageBox.confirm(\`确定删除子任务"${title}"吗？\`, '删除确认', { confirmButtonText:'删除', cancelButtonText:'取消', type:'warning' })`（标题两侧是 ASCII 双引号 `"`）。
- 添加输入框只响应**无修饰键的 Enter**（`@keydown.enter.exact`）；IME 组合中（`isComposing || keyCode===229`）忽略；`preventDefault` 后 `addSubtask()`。Ctrl+Enter 冒泡到窗口级 = 保存待办。
- 拖拽：vuedraggable / SortableJS 1.14，`handle=.subtask-drag-handle`、`animation:200`（其它行让位动画 200ms）、`force-fallback:true`（拖拽时生成跟随鼠标的克隆体：`position:fixed; opacity:.8; pointer-events:none; z-index:100000`）、原位置占位 `opacity:.5`。

### 3.12 从文件导入子任务（仅 isEdit 且非只读）

`el-dropdown trigger="click"` 挂在 `.import-btn` 上，菜单项：

| 文案 | command | 动作 |
|---|---|---|
| `选择文件 (.md/.txt)` | `files` | 系统打开文件对话框：title `导入子任务（选择 .md/.txt 文件或文件夹）`，`multiple:true, directory:false`，filters `[{ name:'文本文件', extensions:['md','txt'] }]` |
| `选择文件夹（递归导入）` | `folder` | 系统选择文件夹对话框：title `选择文件夹（递归导入 .md/.txt 文件）`，`directory:true` |

取消选择 → 什么也不做。选中后 `invoke('import_subtasks_from_paths', { parentId: todoId, paths })` → `loadTodo(false)` → `ElMessage.success(\`成功导入 ${n} 个子任务\`)`；失败 `notifyError(e, '导入失败')`。未保存的新建待办调用时提示 `ElMessage.warning('请先保存待办后再导入子任务')`（UI 上不可达，因为按钮只在 isEdit 时出现）。

后端规则（todo.rs L556-758）：
- 路径是文件：扩展名 md/txt（大小写不敏感）才收；是文件夹：递归，所选文件夹为第 0 层、最多到第 5 层，递归时跳过符号链接，读不了的子目录只记日志跳过。
- 超过 1000 个文件 → `文件过多（超过 1000 个），请选择更小的文件夹`；一个都没有 → `未找到 .md 或 .txt 文件`。
- 按路径排序去重；标题 = 文件名去扩展名后 trim（空则跳过）；内容按 UTF-8 → 带 BOM 的 UTF-8/UTF-16LE/UTF-16BE → GBK 解码；单文件 >10MB 失败。
- 任一文件读失败 → 整体取消：`以下 {N} 个文件无法读取，已取消导入：{文件名（原因）、…}`（最多列 10 个，多余加 ` 等`；原因如 `超过 10MB`、`不是 UTF-8 / UTF-16 / GBK 文本`）。
- 单事务插入，sort_order 接在现有最大值之后；父待办不存在 → `待办不存在`。返回创建的 SubTask 数组。

### 3.13 打开子任务窗口与内存模式协议

#### 3.13.1 openSubtaskWindow(subtaskId, mode: 'edit'|'view')

```
if isSubtaskEditorOpen → return                     // 同一时刻只开一个
isMemoryMode = !isEdit                              // 新建待办里的子任务尚未落库（负数 id）
url   = `#/subtask-editor?id=${subtaskId}` + (isMemoryMode ? '&memory=1' : '') + (mode==='view' ? '&mode=view' : '')
label = `subtask-${mode}-${Date.now()}`
isSubtaskEditorOpen = true
if isMemoryMode:
   pending = pendingSubtasks.find(id) ?? (isSubtaskEditorOpen=false; return)
   监听 'subtask-memory-ready'  (payload {pendingId})            → pendingId 匹配时 emit 'subtask-memory-init' {pendingId, title, content}
   监听 'subtask-memory-save'   (payload {pendingId,title,content}) → 匹配时 pending.title = title; pending.content = content || null
new WebviewWindow(label, { url, title: mode==='edit' ? '编辑子任务' : '查看子任务',
                           width:800, height:750, x, y（§2 居中公式，基于编辑器所在显示器）,
                           resizable:true, decorations:false, transparent:false, parent: 编辑器窗口 })
on destroyed: isSubtaskEditorOpen=false; 取消监听; if mode==='edit' && !isMemoryMode → loadTodo(false)
on error:     isSubtaskEditorOpen=false; 取消监听
异常: isSubtaskEditorOpen=false; 取消监听; notifyError(e, '打开子任务窗口失败')
```

#### 3.13.2 内存模式时序（事件是全局广播，靠 pendingId 过滤；PRD D5：GPUI 改为同进程直接回调）

```
编辑器(父)                                 子任务窗口
 1. listen ready / save
 2. 创建窗口 ───────────────────────────▶  3. onMounted: 先 listen 'subtask-memory-init'
                                           4. emit 'subtask-memory-ready' {pendingId}
 5. 收到 ready → emit 'subtask-memory-init' {pendingId, title, content}
                                           6. 收到 init → 填 title / content → 重拍快照
                                           …用户编辑…
                                           7. 保存：emit 'subtask-memory-save' {pendingId, title: trim 后, content: toStorageMarkdown(内容)}
                                              → 关窗
 8. 收到 save → 更新 pending.title / pending.content（'' → null）
 9. destroyed → 取消监听（不 loadTodo）
```

#### 3.13.3 暂存子任务落库 flushPendingSubtasks(parentId)

```
for pending in 当前 pendingSubtasks 的拷贝（按顺序）:
  created = invoke('create_subtask', { data:{ parentId, title: pending.title,
                                              content: pending.content ? toStorageMarkdown(pending.content) : undefined } })
     失败 → notifyError(e, `${pendingSubtasks.length} 个子任务未能创建，请点击「保存」重试`); return false
  pendingSubtasks 移除该项
  if pending.completed: invoke('update_subtask', { id: created.id, data:{ completed:true } })   // 失败只 console.error
return true
```
成功一个移除一个；失败即停，剩下的留在 pendingSubtasks 里下次保存重试，已创建的不会重复。

### 3.14 描述放大编辑弹窗（el-dialog）

打开 `openDescDialog()`：`descDraft = form.description`（存储形式）；`descPreview = descDraft`；显示。

| 属性 | 值 |
|---|---|
| 组件 | `el-dialog v-model=descDialogVisible :fullscreen=descDialogMaximized width="88%" :close-on-click-modal=false append-to-body @closed=onDescDialogClosed`（Esc 关闭默认开启） |
| 非全屏 | 宽 = 窗口宽 88%；`margin-top:5vh`（main.scss !important）、`margin-bottom:50px`；圆角 12（!important）；padding 16；遮罩 `rgba(0,0,0,.5)` |
| 全屏 | EP 设宽 100%、高 100%、`margin-top:0`、圆角 0，但 main.scss 的 `margin-top:5vh !important` 与 `border-radius:12px !important` 仍生效 → 实际是从 5vh 处开始、高 100vh 的 12px 圆角面板（底部超出视口 5vh，遮罩层可滚动）。GPUI 还原时可按"撑满窗口"处理 |
| `.el-dialog__body` | `max-height:70vh; overflow-y:auto`（main.scss） |

结构：
```
header (#header 插槽)
└─ div.desc-dialog-header        flex; align-items:center; justify-content:space-between; padding-right:32px（给 EP 自带关闭 X 留位）
   ├─ span.desc-dialog-title     "编辑描述"   15px 600 #1E293B
   └─ button.desc-dialog-max-btn  title = 已最大化 ? "还原" : "最大化"   FullScreen 14px
                                 24×24 flex 居中，padding 0，#94A3B8，透明，无边，圆角 4，过渡 .15s；hover #3B82F6 / bg #EFF6FF
   （+ EP 自带右上关闭 X，48×48 热区）
body
└─ div.desc-dialog-body[.is-fullscreen]   flex(row); height:60vh（全屏：calc(100vh - 150px)）; border:1px solid #E2E8F0; border-radius:8px; overflow:hidden
   ├─ textarea.desc-source   v-model=descDraft  placeholder="输入 Markdown 源码..."  spellcheck=false
   │                         flex:1; min-width:0; height:100%; padding:12px 14px; border:none; outline:none; resize:none;
   │                         font: 13px/1.6 Consolas, 'Courier New', monospace; color:#334155; bg:#F8FAFC
   ├─ div.desc-divider       width:1px; bg:#E2E8F0; flex-shrink:0
   └─ MarkdownEditor readonly :model-value=descPreview  .desc-preview   flex:1; min-width:0; overflow-y:auto; bg:#FFFFFF（内部 .milkdown padding 12px 16px）
footer（右对齐）
├─ el-button size=small "取消"            → descDialogVisible=false
└─ el-button type=primary size=small "确定" → form.description = descDraft; 关闭
```
- 预览：`descDraft` 每次变化重置 300ms 定时器，到点 `descPreview = descDraft`（防抖）。
- 源码框是普通 textarea：**不支持图片粘贴上传**；显示的是存储形式（`minitodo-image://…`）。
- 最大化按钮 `toggleDescDialogMaximize()`：
  ```
  未最大化 → wasWindowMaximized = await appWindow.isMaximized(); 若否则 appWindow.maximize(); descDialogMaximized = true
  已最大化 → descDialogMaximized = false; 若 !wasWindowMaximized 则 appWindow.unmaximize()
  ```
- 关闭（确定 / 取消 / X / Esc 都会走 `@closed`）：清预览定时器；若仍处于最大化 → 还原（同上）。取消 / X / Esc 丢弃草稿。
- 弹窗打开期间：窗口级 Esc / Ctrl+Enter 都让路（§8：`.el-overlay` 可见）；Esc 由弹窗自己处理 = 关闭弹窗（丢弃）。

### 3.15 子任务窗口打开时的遮罩

`div.modal-overlay`（`isSubtaskEditorOpen`）：`position:fixed; inset:0; background:rgba(0,0,0,.15); z-index:9999; cursor:not-allowed`。无点击处理（不会把子任务窗口带回前台）。同时窗口级快捷键 `enabled()` 返回 false（Esc / Ctrl+Enter 全部忽略）。

---

## 4. 子任务窗口（SubtaskEditorView.vue）

### 4.1 参数与状态

| query | 含义 |
|---|---|
| `id` | 子任务 id（`parseInt`；内存模式下为负数暂存 id） |
| `mode=view` | 只读 |
| `memory=1` | 内存模式（不碰数据库，数据经 §3.13.2 事件交换） |

状态：`title=''`、`markdownContent=''`（存储形式）、`saving=false`。
加载：非内存模式 `invoke('get_subtask', { id })` → `title=result.title; markdownContent=result.content || ''` → `markClean()`；失败 `notifyError(e, '加载子任务失败')`（后端找不到时详情为 rusqlite 的 `Query returned no rows`）。内存模式见 §3.13.2。

快照：`JSON.stringify([title.trim(), toStorageMarkdown(markdownContent)])`；创建时先按空值拍一次。

### 4.2 元素树与样式

```
div.subtask-editor-window       flex column; height:100vh; bg:#FFFFFF
├─ div.window-header            拖拽区（§1.4）
│  ├─ h2  isViewMode ? "查看子任务" : "编辑子任务"
│  └─ div.window-controls       [data-tauri-drag-region=false]
│     ├─ button.control-btn.maximize-btn  title="最大化"  FullScreen 14   → 切换窗口最大化
│     └─ button.control-btn.close-btn     title="关闭"    Close 14        → 直接关窗（不确认）
├─ div.editor-content
│  ├─ div.form-field
│  │  ├─ label.field-label "标题"
│  │  └─ el-input v-model=title placeholder="请输入子任务标题" maxlength="200" :disabled=isViewMode
│  └─ div.form-field.editor-field
│     ├─ label.field-label "内容 (Markdown)"
│     └─ MarkdownEditor v-model=markdownContent :readonly=isViewMode .milkdown-editor-wrapper
└─ div.window-footer
   └─ div.footer-right
      ├─ el-button（default）  Close + (isViewMode ? "关闭" : "取消")   → 直接关窗
      └─ el-button type=primary :loading=saving :disabled="!title.trim() || saving"  Check + "保存"   （!isViewMode）
```

| 节点 | 样式 |
|---|---|
| `.window-header` | flex，`align-items:center; justify-content:space-between; padding:12px 16px; min-height:44px; border-bottom:1px solid #E2E8F0`；实际高 12+28+12+1 = **53px** |
| `h2` | `margin:0; font-size:16px; font-weight:600; line-height:1.2`（色 `#1E293B`） |
| `.window-controls` | flex，`align-items:center; gap:2px` |
| `.control-btn` | 32×28 flex 居中，`border:none; background:transparent; cursor:pointer; border-radius:4px; color:#64748B; transition: all .15s`；hover `background:#F1F5F9; color:#334155`；`.close-btn:hover` `background:#FEE2E2; color:#EF4444`。最大化按钮 title 恒为"最大化"（已最大化时也一样） |
| `.editor-content` | `flex:1; padding:16px; overflow-y:auto`；flex column，`gap:16px` |
| `.form-field` | flex column，`gap:6px` |
| `.field-label` | `font-size:13px; font-weight:600; color:#334155` |
| 标题 el-input | default 尺寸（32 高）；只读时 disabled 外观（底 `#F5F7FA`、字 `#A8ABB2`、描边 `#E4E7ED`、`not-allowed`） |
| `.editor-field` | `flex:1`；flex column |
| `.milkdown-editor-wrapper` | `flex:1; border:1px solid #E2E8F0; border-radius:8px; min-height:300px; overflow-y:auto; background:#FFFFFF`；内部 `.milkdown` `min-height:290px`（padding 默认 12px 16px），`.ProseMirror` `min-height:280px`；**无** focus 高亮 |
| `.window-footer` | flex，`justify-content:flex-end`（右对齐）; `align-items:center; padding:12px 16px; border-top:1px solid #E2E8F0` |
| `.footer-right` | flex，`gap:8px`（再叠加 EP 相邻按钮 12px → 20px） |
| 按钮 | **default 尺寸**（高 32、14px、padding 8px 15px），与待办编辑器底部的 small 不同 |

### 4.3 行为

```
handleSave():
  if isViewMode || saving || !title.trim() → return
  content = toStorageMarkdown(markdownContent)
  saving = true
  内存模式: emit('subtask-memory-save', { pendingId: id, title: title.trim(), content })
  否则:     invoke('update_subtask', { id, data:{ title: title.trim(), content } })
  成功 → appWindow.close()（无提示）；失败 → notifyError(e, '保存子任务失败')
  finally saving = false

closeWithConfirm():     // Esc
  if saving → return
  if !isViewMode && snapshot 变了 → ElMessageBox.confirm('有未保存的修改，确定放弃并关闭吗？', '放弃修改',
                                      { confirmButtonText:'放弃', cancelButtonText:'继续编辑', type:'warning' })，取消则 return
  close
```
- 快捷键：`bindEditorShortcuts({ onClose: closeWithConfirm, onSubmit: handleSave })`（无 enabled 条件）。只读时 Esc 直接关，Ctrl+Enter 无效。
- 标题 trim 后保存；空标题时保存按钮禁用。
- `update_subtask` 后端：title / content / completed / sortOrder 任一有值即更新；全空报 `No fields to update`；`updated_at` 严格递增；返回 SubTask。
- 关闭后（编辑模式、非内存）父编辑器 `loadTodo(false)` 刷新子任务列表。

---

## 5. Markdown 编辑器（MarkdownEditor.vue）

### 5.1 接口与数据契约

- props：`modelValue: string`（**存储形式**，图片引用为 `minitodo-image://<name>`），`readonly?: boolean`。
- emit：`update:modelValue(storage)`。
- 内部：进编辑器前 `toDisplayMarkdown(storage, imagesDir)`（规范引用 / 旧 asset URL → 本机 asset URL），出编辑器 `toStorageMarkdown(markdown)`。
- `imagesDir = await getImagesDir()`（`invoke('get_images_dir')`，每窗口查一次，失败下次重试；后端会 `create_dir_all`）；拿到目录后才创建编辑器（首帧即可显示图片）。拿不到时图片引用原样（不显示，但不改数据）。
- 外部赋值（`watch modelValue`）：`next = toStorageMarkdown(value ?? '')`；与内部内容相同则忽略；否则 `replaceAll(toDisplayMarkdown(next))` 并记录"回声"（编辑器把同一内容规范化后的 Markdown）。
- 用户输入（listener `markdownUpdated`，Milkdown 默认约 200ms 防抖）：与上次相同 / 等于回声 → 忽略；`storage = toStorageMarkdown(markdown)`，等于内部内容 → 忽略；否则 emit。→ 加载后不会因为"规范化写法"被误判为已修改。
- `readonly` 变化 → 销毁重建编辑器（listener / upload 只在编辑模式注册）。创建是异步的，用代次号丢弃过期实例。

### 5.2 Milkdown 插件

| 插件 | 编辑模式 | 只读模式 | 作用 |
|---|---|---|---|
| `@milkdown/theme-nord` | ✓ | ✓ | 编辑器 class `prose dark:prose-invert milkdown-theme-nord` + 样式表（含 Tailwind preflight） |
| `commonmark` preset | ✓ | ✓ | 段落、标题 1-6、引用、代码块、行内代码、粗体、斜体、链接、图片、有序/无序列表、分隔线、硬换行、HTML（原样文本） |
| `gfm` preset（remark-gfm） | ✓ | ✓ | 表格（含对齐）、任务清单、删除线、脚注、自动链接 |
| `listener` | ✓ | – | markdownUpdated 回调 |
| `upload` | ✓ | – | 粘贴 / 拖入图片上传（§5.6） |
| `clipboard` | ✓ | – | 粘贴纯文本时按 Markdown 解析成富文本（"粘贴 MD 源码自动解析"）；粘贴 HTML 按 HTML 解析；从 VS Code 复制的代码自动建代码块；复制时序列化为 Markdown |
| history（撤销/重做） | **未启用** | – | 没有 `@milkdown/plugin-history`，Ctrl+Z 无可靠撤销 |

链接 DOM：Milkdown 会按自身安全协议表清洗 `href`（`file:` 也会清成空串），本组件用 `linkAttr` 把原始地址另存到 `data-href`；link mark 的 `title` 会成为原生 tooltip。

编辑模式下的 Markdown 输入规则 / 快捷键（Milkdown 7.22.2 预设自带，属于"所见即所得"能力）：

| 输入 | 效果 |
|---|---|
| `# ` … `###### ` | 标题 |
| `> ` | 引用 |
| `- ` / `* ` / `+ ` | 无序列表 |
| `1. ` | 有序列表 |
| ` ``` ` | 代码块 |
| `**x**` / `*x*` / `` `x` `` / `~~x~~` | 粗体 / 斜体 / 行内代码 / 删除线 |
| `[ ] ` / `[x] `（在列表项开头） | 任务项（未选 / 已选） |
| `|3x2| ` | 插入 3 列 × 2 行表格（行数至少 2） |
| Mod-b / Mod-i / Mod-e / Mod-Alt-x | 粗体 / 斜体 / 行内代码 / 删除线 |
| Mod-Alt-0 / 1…6 / 7 / 8 / c、Mod-Shift-b | 段落 / 标题 / 有序 / 无序 / 代码块、引用 |
| Shift-Enter | 硬换行 |
| Tab / Shift-Tab | 列表缩进；表格内下一格 / 上一格 |
| 表格内 Enter / Mod-Enter | 跳出表格（会 preventDefault → 窗口级 Ctrl+Enter 不触发保存） |

任务清单的复选框**仅显示，不能点击切换**（Milkdown 预设没有点击切换逻辑，只能删掉重打 `[x] `）。

### 5.3 序列化细节（影响现有数据，GPUI 渲染时要兼容）

- 空段落序列化为单独一行 `<br />`（`EMPTY_LINE_PLACEHOLDER`），读回时还原为空段落。→ 现有描述里可能有独占一行的 `<br />`，渲染应显示为一个空行而不是字面文本。
- 列表、强调等符号由 remark-stringify 统一写法（`remarkMarker` 保留 `_`/`*`），特殊字符会被反斜杠转义（如 `\[`、`\*`）。
- 图片序列化为 `![alt](src)`，alt 为上传文件名。

### 5.4 只读模式

`editable: () => false`；不注册 listener / upload / clipboard；链接普通单击即按白名单打开（§7）；图片点击预览（§5.7）。

### 5.5 渲染样式（有效值 = theme-nord 规则 + 本组件 scoped 覆盖；`!important` 的 theme 规则胜出）

容器：

| 选择器 | 样式 |
|---|---|
| `.markdown-editor` | flex column，`min-height: inherit` |
| `.markdown-editor-container` | `flex:1; min-height:inherit`；flex column |
| `.milkdown` | `flex:1; padding:12px 16px`（各处覆盖：描述编辑框 `8px 12px`，只读详情 `0`） |
| `.ProseMirror` / `.editor` | `outline:none`；`white-space: break-spaces; word-wrap: break-word` |
| 基准文字 | 继承 body：14px、`#1E293B`、body 字体栈；**段落是 16px**（theme 的 `--text-base: 1rem`） |
| 选中节点（编辑模式） | `outline: 2px solid #88CCFF` |

元素（"em" 按所在元素字号换算；块间 margin 按 CSS 外边距折叠取较大者）：

| 元素 | 字号 / 字重 / 行高 | 外边距 | 颜色 / 其它 |
|---|---|---|---|
| 段落 p | 16px / 400 / 1.6（25.6px） | `0.4em 0` = 6.4px 上下 | `#1E293B` |
| h1 | 30px / 700 / 1.25 | `0.6em 0 0.3em` = 18px / 9px | |
| h2 | 24px / 600 / 1.333 | 14.4px / 7.2px | |
| h3 | 20px / 500 / 1.4 | 12px / 6px | |
| h4 | 16px / 500 / 1.5 | 上 12px，下 0 | |
| h5、h6 | 14px / 500 / 1.4286 | 上 8px，下 0 | |
| 粗体 / 斜体 / 删除线 | bolder / italic / line-through | | |
| 链接 a | 继承 | | 颜色继承正文 `#1E293B`，**下划线**；有 title 时原生提示 |
| 行内代码 | 0.9em（段落内 14.4px）/ 400 / 1.4286 | | 字体 `ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace`；字色 `#5E81AC`；底 `#F1F5F9`；`padding:2px 6px`；圆角 4 |
| 代码块 pre | 14px / 1.4286（20px），等宽字体 | 0（靠相邻段落 6.4px） | 底 `#1E293B`；字 `#E2E8F0`；`padding:12px 16px`；圆角 8；`white-space: pre-wrap`（长行折行）；无语法高亮、不显示语言名；内部 code 透明底无 padding 继承色 |
| 引用 blockquote | 容器 14px，内部段落 16px | `0.5em 0` = 7px | 左边框 `3px solid #3B82F6`；`padding-left:12px`；字色 `#64748B`；**字体为衬线** `ui-serif, Georgia, Cambria, "Times New Roman", Times, serif`（中文在 Windows 上回落宋体）；非斜体 |
| ul / ol | | `0.4em 0` = 5.6px | `padding-left:24px`；标记 disc / decimal，outside，**标记颜色 `#5E81AC`**；嵌套每层再缩 24px |
| li | | `margin-bottom:4px` | li 内是段落（16px，上下 6.4px） |
| 任务项 li | | | 无列表符号；在 `left:-20px; top:0.4em(5.6px)` 画 13×13 方框：`border:1.5px solid #94A3B8; border-radius:3px; background:#FFFFFF`；已勾选：底 / 边 `#3B82F6`，白色对勾（3×6 的右下边框 1.5px、旋转 45°，位于 `left:-15px; top:0.52em`） |
| hr | | `1em 0` = 14px | 无边框，`border-top:1px solid #E2E8F0` |
| table | 14px / 1.4286（20px） | `0.6em 0` = 8.4px | `width:100%; border-collapse:collapse; table-layout:fixed`（各列等宽）；阴影 `0 4px 6px -1px rgba(0,0,0,.1), 0 2px 4px -2px rgba(0,0,0,.1)` |
| th / td | | | `border:1px solid #E2E8F0`；**padding `12px 24px`**（theme 的 !important 覆盖了 scoped 的 6px 10px）；`vertical-align:top`；对齐按 GFM 对齐行（默认 left）；单元格内段落 margin 0 |
| th | | | 底 `#F8FAFC`；600 |
| td 奇数列（第 1、3、5…列） | | | 底 `#F9FAFB`（Tailwind gray-50）；偶数列白 |
| 图片 img | | 上下 0（theme !important），左右 0 | 行内块（在段落里）；`max-width:100%; height:auto`；圆角 6；阴影 `0 4px 6px -1px rgba(0,0,0,.1), 0 2px 4px -2px rgba(0,0,0,.1)`；`cursor:pointer`；hover `opacity:.85; box-shadow:0 2px 8px rgba(0,0,0,.15)`（过渡 .15s） |
| 空段落 `<br />` | 同段落 | 同段落 | 显示为空行 |
| 内联 HTML | | | commonmark 的 html 节点以文本原样显示 |

> 系统深色模式（`prefers-color-scheme: dark`）下 theme-nord 会把 td 奇数列底改 `#111827`、tr 线改 gray-600；编辑器窗口没有锁定 WebView 主题，这是现有实现的潜在差异，GPUI 应固定使用上表浅色值。

### 5.6 图片上传协议

触发（仅编辑模式）：
- 粘贴：`clipboardData.files` 非空时（`enableHtmlFileUploader:true`：右键"复制图片"这类同时带 HTML 的剪贴板也走上传）；插入位置 = 当前选区起点。
- 拖入：`dataTransfer.files`；插入位置 = 鼠标坐标处（`posAtCoords`），取不到用选区起点。
- 上传期间在插入点放一个**空的** `<span>` 占位（不显示"Upload in progress..."）。

前端逐张处理（同一批并行，单张失败不影响其它）：
```
images = files.filter(f => f.type.startsWith('image/'))      // 非图片文件静默忽略
for image:
  ext = resolveImageExtension(image)                           // §6.4
  if !ext → ElMessage.warning(`不支持的图片格式：${image.name || image.type}（支持 png / jpg / webp / gif / bmp）`); 跳过
  if image.size > 20*1024*1024 → ElMessage.warning(`图片超过 20MB，未插入：${image.name}`); 跳过
  path = invoke('save_subtask_image', <Uint8Array 原始字节>, { headers: { 'x-image-ext': ext } })
     失败 → notifyError(e, '图片保存失败'); 跳过
  插入 image 节点 { src: toAssetUrl(path), alt: image.name }   // 序列化时被 toStorageMarkdown 换成 minitodo-image://<name>
```
浏览器里粘贴截图时 `File.name` 通常是 `image.png`（所以 alt 多为 `image.png`）。

后端 `save_subtask_image`（async，`spawn_blocking` 落盘）：
| 项 | 规则 |
|---|---|
| 请求体 | 原始字节（`InvokeBody::Raw`）；IPC 退回 postMessage 时为数字数组，每项须为 0–255 整数，否则 `图片数据格式错误：字节数组只能包含 0–255 的整数`；其它形态 `图片数据必须是原始字节或字节数组` |
| 扩展名 | 请求头 `x-image-ext`：trim、去前导 `.`、转小写，白名单 `png / jpg / jpeg / webp / gif / bmp`；否则 `不支持的图片格式："<原值>"（支持 png / jpg / jpeg / webp / gif / bmp）` |
| 大小 | 0 字节 `图片内容为空`；> 20MB `图片超过 20MB，未保存` |
| 文件名 | 后端生成 `<Unix 毫秒>_<6 位 [a-z0-9] 随机>.<ext>`，须通过 `is_safe_image_name`（不合法 `生成的图片文件名不合法: …`） |
| 写入 | images 目录（`%LOCALAPPDATA%\mini-todo\images\`）下先写 `.<name>.part` 再改名；目标已存在则换名重试，最多 8 次（`生成图片文件名失败，请重试`）；写失败 `写入图片失败: …`；建目录失败 `创建图片目录失败: …` |
| 返回 | 图片绝对路径字符串（非 UTF-8 → `图片路径包含非法字符`） |

### 5.7 图片预览

容器上监听 click：目标是 `IMG` 且有 src → 阻止默认与冒泡，收集编辑器内所有 `.ProseMirror img` 的 src（按文档顺序），从被点的那张开始打开 `el-image-viewer`（`z-index:10000`，§1.2 外观；Esc / 关闭按钮关闭）。编辑模式与只读模式都生效（编辑模式下同时会选中该图片节点）。

### 5.8 链接点击

ProseMirror `handleDOMEvents.click` 与容器 click 监听都会调用 `handleLinkClick(event, { readonly })`（同一事件打标记只处理一次）；容器 `auxclick`（中键）落在链接上一律 `preventDefault`。规则见 §7。

---

## 6. 图片引用规则（utils/imageRef.ts，K5）

### 6.1 常量

| 名称 | 值 |
|---|---|
| `IMAGE_REF_SCHEME` | `minitodo-image://` |
| `IMAGE_EXTENSIONS` | `png, jpg, jpeg, webp, gif, bmp` |
| `MAX_IMAGE_BYTES` | `20 * 1024 * 1024` |
| `SAFE_NAME` | `[A-Za-z0-9][A-Za-z0-9._-]{0,127}`（首字符字母数字，总长 1–128） |
| `NAME_END` | `(?![A-Za-z0-9._%-])`（名字后不能紧跟名字字符或 `%`） |
| `SEP` | `(?:%5[Cc]|%2[Ff]|/)`（编码的 `\`、编码的 `/`、或未编码 `/`） |
| `PATH_CHAR` | `(?:[A-Za-z0-9\-_.!~*'()/]|\\[()]|%[0-9A-Fa-f]{2})`（encodeURIComponent 不编码的字符、序列化时的 `\(` `\)` 转义、百分号编码；不含空白 / 引号 / 方括号 / 冒号，匹配不会越出当前 URL） |

匹配正则（全局）：
```
minitodo-image://(SAFE_NAME)NAME_END
|
(?:https?://asset\.localhost|asset://localhost)/PATH_CHAR*?SEP images SEP(SAFE_NAME)NAME_END
```
（第 1 组 = 规范形式文件名；第 2 组 = 父目录名为 `images` 的旧 asset URL 里的文件名；`*?` 为非贪婪。）
> Rust 移植注意：`regex` crate 不支持 `(?!…)`，需用 `fancy-regex` 或在匹配后手动检查下一个字符；`\d`/`[A-Za-z]` 一律写 ASCII 字符类。

### 6.2 函数

| 函数 | 行为 |
|---|---|
| `isSafeImageName(name)` | `^SAFE_NAME$` 且不含 `..` |
| `toImageRef(name)` | `minitodo-image://${name}` |
| `replaceImageRefs(md, f)` | 对每个匹配取 name（组 1 ?? 组 2）；`name && isSafeImageName(name)` 则替换为 `f(name)`，否则保留原文 |
| `toStorageMarkdown(md)` | 空串原样；否则 `replaceImageRefs(md, toImageRef)`。幂等：规范形式 → 规范形式；旧 asset URL → 规范形式 |
| `toDisplayMarkdown(md, imagesDir)` | md 空或 imagesDir 为空 → 原样；否则 `replaceImageRefs(md, name => toAssetUrl(joinPath(imagesDir, name)))` |
| `joinPath(dir, name)` | 分隔符：dir 含 `\` 且不含 `/` → `\`，否则 `/`；dir 已以 `\` 或 `/` 结尾则直接拼接 |
| `toAssetUrl(path)` | `convertFileSrc(path)`（Windows：`http://asset.localhost/` + encodeURIComponent(路径)；macOS/Linux：`asset://localhost/…`），再把 `(`→`%28`、`)`→`%29`（避免不成对括号截断 Markdown 链接目标） |
| `getImagesDir()` | 每窗口缓存一次 `invoke('get_images_dir')`；失败 console.error、返回 null、下次重试 |
| `resolveImageExtension({name, type})` | 先按 MIME（小写）：`image/png→png, image/jpeg→jpg, image/jpg→jpg, image/pjpeg→jpg, image/webp→webp, image/gif→gif, image/bmp→bmp, image/x-ms-bmp→bmp`；否则取文件名最后一个 `.` 之后转小写，在白名单内则返回（可返回 `jpeg`）；否则 null |

### 6.3 测试向量（imageRef.test.ts，Windows 形态 convertFileSrc）

| 输入 | 结果 |
|---|---|
| `isSafeImageName`：`a`、`img_1.png`、`1700000000000_ab12cd.png`、`img_1727-x.JPEG`、128 个 `A` | true |
| `isSafeImageName`：`''`、`.hidden.png`、`-x.png`、`../x.png`、`a..b.png`、`a/b.png`、`a\b.png`、`a b.png`、`a%2Fb.png`、`图片.png`、129 个 `A` | false |
| `toStorageMarkdown('before ![shot](http://asset.localhost/C%3A%5CUsers%5Cbob%5C…%5Cimages%5C1700_abc.png) after')` | `before ![shot](minitodo-image://1700_abc.png) after` |
| `https://asset.localhost/…images%5Cx.png`、`asset://localhost/<编码的 /Users/…/images/y.gif>`、`asset://localhost/home/bob/.local/share/mini-todo/images/z.png`（未编码 /） | 都换成 `minitodo-image://x.png` 等 |
| 其它设备 / 其它用户路径（`D:\Data\alice\mini-todo\images\2.png` 的 asset URL） | `minitodo-image://2.png` |
| 父目录不是 images（`…\Pictures\p.png`）、`https://example.com/images/w.png`、`…images\..\secret.png`、`minitodo-image://../secret.png`、普通链接与代码文本 | 原样不变 |
| 序列化转义 `\(` `\)` 的旧 URL、`<…>` 包裹的目标 | `![a](minitodo-image://1_a.png)`、`![a](<minitodo-image://1_a.png>)` |
| `toDisplayMarkdown('![a](minitodo-image://a.png)', 'C:\…\images')` | `![a](http://asset.localhost/<encode(C:\…\images\a.png)>)`；目录尾部已有 `\` 不重复；目录含括号时括号被编码；`toStorageMarkdown(toDisplayMarkdown(x)) === x` |
| `resolveImageExtension({name:'image.png', type:'image/jpeg'})` | `jpg`；`{'clip','IMAGE/PNG'}`→`png`；`{'x','image/x-ms-bmp'}`→`bmp`；`{'Photo.JPEG',''}`→`jpeg`；`{'a.b.webp','application/octet-stream'}`→`webp`；svg / 无扩展名 / tiff → null |

---

## 7. 链接白名单（utils/fileLink.ts）

### 7.1 resolveLinkTarget(href, windows = 当前是否 Windows) → `external{url}` | `file{path}` | `blocked`

```
raw = href.trim();  空 → blocked
url = new URL(raw)            // WHATWG 解析：大小写、内嵌 \t \n 被规范化；无 base 的相对地址解析失败 → blocked
switch url.protocol:
  'http:' | 'https:' | 'mailto:' → external(url.href)        // 用规范化后的 href
  'file:' → path = extractFilePath(raw, windows); path ? file(path) : blocked
  其它 → blocked
```

### 7.2 extractFilePath(href, windows)

```
if !/^file:\/\/\//i.test(href) → null              // 只认三斜杠；file://host/share（UNC）拒绝
raw = href.slice(8).split(/[?#]/)[0]                // 先去查询串 / 片段再解码（%23 %3F 属于文件名）
rest = decodeURIComponent(raw)；解码失败 → null
windows:  path = rest 中 / 全换成 \ ；必须匹配 ^[A-Za-z]:\\ （排除 \\server\share 与 \\?\ 前缀）否则 null
其它:     path = '/' + rest；以 // 开头 → null
```

### 7.3 handleLinkClick(event, { readonly }) → boolean（true = 已消费）

```
if event 已标记处理 → true
anchor = event.target.closest('a')；没有 → false（不阻止默认）
event.preventDefault(); 标记已处理              // 无论如何 WebView 都不导航
href = anchor['data-href'] ?? anchor.href ?? ''  // 优先原始地址
link = resolveLinkTarget(href)
file     → stopPropagation; revealItemInDir(path)（在资源管理器中定位并选中该文件）; true
external → if !readonly && !ctrlKey && !metaKey → false     // 编辑排版普通点击只放光标
           stopPropagation; openUrl(url)（系统默认程序）; true
blocked  → true（什么都不做）
```
`preventLinkAuxClick`：中键点在链接上 → `preventDefault`。

| 场景 | 只读 | 编辑 |
|---|---|---|
| http / https / mailto | 单击打开 | Ctrl/Cmd+单击打开；普通单击放光标 |
| file:///盘符路径 | 单击在资源管理器定位 | 单击定位 |
| javascript: / data: / vbscript: / blob: / ftp: / tauri: / asset: / 相对地址 / `#anchor` / UNC | 无动作 | 无动作 |

### 7.4 测试向量（fileLink.test.ts）

| 输入 | 结果 |
|---|---|
| `extractFilePath('file:///C:/Users/bob/notes.txt', true)` | `C:\Users\bob\notes.txt` |
| `'FILE:///d:/a%20b/c.md'` (win) | `d:\a b\c.md` |
| `'file:///C:/a/b.txt?x=1#frag'` | `C:\a\b.txt`；`'file:///C:/a/%23b%3F.txt'` → `C:\a\#b?.txt` |
| `file://server/share/a.txt`、`file://///server/share/a.txt`、`file:////server/share/a.txt`、`file:///relative/a.txt`、`file:///%5C%5C?%5CC:%5Ca.txt` (win) | null |
| `'file:///home/bob/a%20b.txt'` (posix) | `/home/bob/a b.txt`；`file:////server/share` → null |
| `'file:///C:/a%E0%A4%A.txt'`、`https://…`、`C:\a.txt` | null |
| `resolveLinkTarget('https://example.com/a?b=1')` | external `https://example.com/a?b=1`；`HTTP://Example.com` → `http://example.com/`；`mailto:bob@example.com` 原样；`'  https://example.com/a b  '` → `https://example.com/a%20b` |
| `javascript:alert(1)`、`JaVaScRiPt:…`、`java\tscript:…`、`' javascript:…'`、`java\nscript:…`、`data:text/html,…`、`vbscript:…`、`blob:…`、`ftp://…`、`tauri://localhost`、`asset://localhost/…` | blocked |
| `''`、`'   '`、`#anchor`、`./a.md`、`/abs/path`、`example.com` | blocked |

Rust 移植：用 `url` crate（WHATWG 兼容，会剥掉首尾 C0/空格与内嵌 tab/换行）得到相同的 protocol 与 `href` 规范化；`decodeURIComponent` 失败语义 = 百分号编码不是合法 UTF-8 时返回 null。

---

## 8. 快捷键（utils/editorShortcuts.ts）

```
isImeComposing(e) = e.isComposing || e.keyCode === 229

resolveEditorShortcut(e):
  if isImeComposing(e) || e.altKey || e.shiftKey → null
  mod = e.ctrlKey || e.metaKey
  if e.key === 'Escape' && !mod → 'close'
  if e.key === 'Enter'  &&  mod → 'submit'
  → null

hasOpenOverlay(includeMessages):
  有 .el-popper[aria-hidden="false"]（下拉、日期 / 时间 / 颜色面板、dropdown 菜单、el-tooltip）→ true
  选择器 = includeMessages ? '.el-overlay, .el-image-viewer__wrapper, .el-message'
                           : '.el-overlay, .el-image-viewer__wrapper'
  其中任一元素已渲染（getClientRects 非空）→ true

bindEditorShortcuts({ onClose, onSubmit, enabled? }):   // window 冒泡阶段 keydown，返回解绑函数
  if e.defaultPrevented || e.repeat → return               // 已被输入框 / 编辑器 / EP 组件处理的不再响应
  s = resolveEditorShortcut(e); if !s → return
  if enabled && !enabled() → return
  if hasOpenOverlay(s === 'close') → return                // Esc 时 ElMessage 提示也算弹层（先关提示）；Ctrl+Enter 不算
  e.preventDefault(); s==='close' ? onClose() : onSubmit()
```

| 窗口 | onClose | onSubmit | enabled |
|---|---|---|---|
| 待办编辑器 | closeWithConfirm（§3.10） | `!isViewMode` 时 handleSave | `!isSubtaskEditorOpen` |
| 子任务窗口 | closeWithConfirm（§4.3） | handleSave（只读时内部直接 return） | – |

测试向量：`Escape`→close；`Ctrl+Enter`、`Meta+Enter`→submit；`Enter`、`Ctrl+Escape`、`Ctrl+Shift+Enter`、`Ctrl+Alt+Enter`、`Ctrl+s` → null；组合输入中的 Esc / Ctrl+Enter（含仅 keyCode 229）→ null。

GPUI 映射要点："弹层打开时 Esc 先关弹层"——GPUI 中下拉 / 日期面板 / 对话框 / 确认框 / 图片预览 / Toast 处于打开状态时，窗口级 Esc 不应关窗；输入法组合中按键不触发；子任务窗口打开期间父编辑器快捷键全部失效；被内层控件消费的 Ctrl+Enter（如表格内跳出）不触发保存（源码编辑器方案下无此情况）。

其它按键约定：添加子任务输入框 = 无修饰 Enter；行内改名 = Enter 保存 / Esc 取消（消费掉，不关窗）/ 失焦保存。

---

## 9. 时间工具（utils/datetime.ts，K1）——需要 1:1 移植

### 9.1 常量

- `DateTimeKind = 'start' | 'end' | 'notify' | 'generic'`
- 仅日期时的默认时刻：start `00:00:00`、end `23:59:00`、notify `09:00:00`、generic `00:00:00`
- `STORAGE_FORMAT = 'YYYY-MM-DD HH:mm:ss'`（dayjs 记法；即 `%Y-%m-%d %H:%M:%S`）
- 正则（`\d` 在 JS 中仅 ASCII 数字；Rust 请写 `[0-9]`）：
  ```
  ^(\d{4})-(\d{2})-(\d{2})(?:[Tt ](\d{2}):(\d{2})(?::(\d{2})(?:\.\d+)?)?\s*([Zz]|[+-]\d{2}(?::?\d{2})?)?)?$
  ```

### 9.2 函数

| 函数 | 伪代码 / 规则 |
|---|---|
| `parseDateTime(value, kind='generic') → Date(本地) \| null` | 空 → null；`value.trim()` 后匹配正则，不匹配 → null；`1≤月≤12`，`1≤日≤当月天数`（不滚动，2026-02-29 → null）；无时间部分 → 用 kind 默认时刻；有时间：`时≤23, 分≤59, 秒≤59`（秒缺省 0，小数秒丢弃）；有时区后缀：`Z/z`=0，否则符号 + 去掉 `:` 后前两位为时、后两位为分（缺省 0），`时>23 或 分>59` → null；结果 = UTC(y,m,d,h,mi,s) − 偏移，再换算到本机墙钟；无后缀 = 直接按本机墙钟构造 |
| `formatStorageDateTime(date)` | `YYYY-MM-DD HH:mm:ss`（本地） |
| `normalizeDateTime(value, kind='generic')` | parse → format；失败 null。幂等 |
| `composeDateTime(date, time, kind)` | `!date` → null；否则 `normalizeDateTime(time ? \`${date} ${time}\` : date, kind)`（time 为 `''`/null/undefined 视为未选） |
| `splitDateTime(value, kind='generic')` | parse 失败 → null；否则 `{ date:'YYYY-MM-DD', time:'HH:mm' }`（秒丢弃） |
| `isSameMinute(a, b, kind='generic')` | 两者都空（null/undefined/''）→ true；任一解析失败 → false；`floor(ms/60000)` 相等 |
| `formatDateKey(date)` | `YYYY-MM-DD` |
| `toDateKey(value, kind='generic')` | parse → `YYYY-MM-DD`；失败 null |
| `formatDateTime(value, pattern='YYYY-MM-DD HH:mm', kind='generic')` | parse → 按 dayjs 格式串输出；失败 null（调用方决定隐藏还是回落原文） |

本文涉及的显示用法：

| 位置 | 调用 |
|---|---|
| 编辑器只读提醒文案 | `formatDateTime(v, 'YYYY-MM-DD HH:mm', 'notify') ?? v` |
| 已完成列表时间 | `formatDateTime(notifyAt, 'MM-DD HH:mm', 'notify') ?? notifyAt` |
| 已完成排序键 | `normalizeDateTime(updatedAt || createdAt) ?? 原值`（generic） |
| 编辑器拆分 / 合成 | `splitDateTime` / `composeDateTime`（kind 对应 notify/start/end） |
| 保存时判断提醒是否改过 | `isSameMinute(form, original, 'notify') && normalizeDateTime(original, 'notify') === original` |

### 9.3 测试向量（datetime.test.ts，固定 TZ = Asia/Shanghai UTC+8）

| 输入 | 结果 |
|---|---|
| `parse('2026-10-02 09:30:15')` | 2026-10-02 09:30:15 |
| `'2026-10-02T09:30:15'`、`'2026-10-02t09:30'`、`'2026-10-02 09:30'`、`'2026-10-02T09:30:15.123'` | 09:30:15 / 09:30:00 / 09:30:00 / 09:30:15 |
| `'  2026-10-02 09:30:00\n'` | 09:30:00（trim） |
| `'2026-10-02T01:00:00Z'`、`'…01:00:00.500z'`、`'…09:00:00+08:00'`、`'+0800'`、`'+08'` | 全部 09:00:00 |
| `'2026-10-01T21:00:00-05:00'` | 2026-10-02 10:00:00 |
| `'2026-10-02 09:00 +05:30'`（时间与时区间有空格） | 11:30:00 |
| `'2026-10-02'` start / end / notify / generic | 00:00 / 23:59 / 09:00 / 00:00 |
| `'2026-10-02 18:45:00'` notify | 18:45（显式时间优先） |
| `2026-02-29`、`2026-04-31 10:00:00`、`2026-13-01`、`2026-00-10`、`2026-10-00` | null；`2028-02-29` 合法 |
| `24:00:00`、`23:60`、`23:59:60`、`9:00`、`+24:00`、`+08:60`、`'2026-10-02Z'` | null |
| null、undefined、`''`、`tomorrow`、`2026/10/02 09:00`、`20261002`、`2026-10-02 09:00:00T09:00:00` | null |
| `normalize('2026-10-02T09:05')` | `2026-10-02 09:05:00`；`'…T09:05:07.999'`→`09:05:07`；`'…T01:05:07Z'`→`09:05:07`；`('2026-10-02','end')`→`23:59:00`；`'2026-10-01T22:30:00-02:00'`→`2026-10-02 08:30:00` 且再规范化不变 |
| `compose('2026-10-02','14:30','notify')` | `2026-10-02 14:30:00`；`('…','14:30:15','start')`→`14:30:15`；`(d,null,'start')`→`00:00:00`；`(d,'','end')`→`23:59:00`；`(d,undefined,'notify')`→`09:00:00`；`(null,'14:30')`、`('',null)` → null |
| `split('2026-10-02 14:30:59')` | `{date:'2026-10-02', time:'14:30'}`；`('2026-10-02','end')`→`23:59`；`'2026-10-01T20:00:00Z'`→`2026-10-02 04:00` |
| `isSameMinute('…09:00:30','…09:00:00')` | true；`'2026-10-02T01:00:00Z'` vs `'…09:00:00'` true；`('2026-10-02','…09:00:00','notify')` true、`'start'` false；不同分钟 / 日期 / 一边为空 false；`(null, undefined)`、`('', null)` true；`('garbage','garbage')` false |
| `formatDateTime('2026-10-02 09:05:00')` | `2026-10-02 09:05`；`('…T09:05:00','MM-DD HH:mm')`→`10-02 09:05`；`('2026-10-02','MM-DD HH:mm','notify')`→`10-02 09:00` |
| `toDateKey('2026-10-01T16:30:00Z')` | `2026-10-02` |

### 9.4 dayjs 格式记号（本文出现过的）

`YYYY` 四位年、`MM` 两位月、`DD` 两位日、`HH` 两位 24 小时、`mm` 两位分、`ss` 两位秒。

### 9.5 与后端 `db/time.rs` 的边界差异（移植时以前端为准用于 UI）

| 输入 | 前端 datetime.ts | 后端 db::time |
|---|---|---|
| 时间与时区之间有空白 `2026-10-02 09:00 +05:30` | 接受 | 拒绝（时间段含空格） |
| 一位数时 / 分 / 秒 `2026-10-02 9:00` | 拒绝（要求两位） | 接受（每段 1–2 位） |
| 非 ASCII 数字 | 拒绝 | 拒绝 |
后端在保存时还会再 `normalize_datetime` 一次（无法识别则报错），所以 UI 只会产出规范格式，差异只影响读取外部数据时的显示。

---

## 10. 已完成窗口（CompletedView.vue）

### 10.1 数据与行为

- 加载：`invoke('get_todos')` → 过滤 `completed` → 按 `normalizeDateTime(updatedAt || createdAt) ?? 原值` **降序**（字符串比较）。失败 `notifyError(e, '加载已完成待办失败')`。只在挂载时、恢复 / 删除后、从这里打开的编辑器关闭后重新加载（不轮询）。
- 搜索：`q = searchQuery.trim().toLowerCase()`；空 → 全部；否则标题或描述（原始 Markdown 文本）小写包含 q。
- **没有**分页、分组、"清空全部"功能。
- 恢复：`invoke('update_todo', { id, data:{ completed:false } })` → 重新加载；失败 `notifyError(e, '恢复待办失败')`。
- 删除：`ElMessageBox.confirm(\`确定要删除待办"${title}"吗？\`, '删除确认', { confirmButtonText:'删除', cancelButtonText:'取消', type:'warning' })` → `invoke('delete_todo', { id })`（后端连带子任务并写墓碑）→ 重新加载；失败 `notifyError(e, '删除待办失败')`。
- 点击行（非按钮）：打开待办编辑器只读详情（§2 表，parent = 已完成窗口，1080×600 居中于已完成窗口所在显示器），模态守卫同 §1.5；关闭后重新加载列表；失败 `notifyError(e, '打开编辑器失败')`。
- 编辑器打开期间：显示遮罩 `rgba(0,0,0,.3)`（`z-index:999; cursor:not-allowed`），遮罩 mousedown 或本窗口获得焦点 → 把编辑器带回前台。
- 关闭按钮：直接 `appWindow.close()`；主窗口随后 `fetchTodos()`。
- 无键盘快捷键（Esc 不关闭）。

### 10.2 元素树与样式

```
div.completed-window              flex column; height:100vh; bg:#FFFFFF; overflow:hidden
├─ div.modal-overlay              （编辑器打开时）
├─ div.title-bar                  拖拽区（§1.4）
│  ├─ div.title-text
│  │  ├─ span "已完成"
│  │  └─ span.count-badge {{ filteredTodos.length }}      （随搜索结果变化）
│  └─ div.window-controls [drag=false]
│     └─ button.close-btn  内联 SVG ✕
├─ div.search-bar
│  └─ div.search-input-wrapper
│     ├─ el-icon.search-icon Search 16
│     ├─ input.search-input placeholder="搜索已完成的待办..." spellcheck=false
│     └─ button.search-clear  Close 14     （有输入时）→ 清空
└─ div.completed-list
   ├─ div.completed-item × N  → 点击打开编辑器
   │  ├─ div.item-color                     bg = todo.color
   │  ├─ div.item-content
   │  │  ├─ div.item-title {{ title }}
   │  │  └─ div.item-time  Clock 12 + 'MM-DD HH:mm'      （有 notifyAt）
   │  └─ div.item-actions
   │     ├─ el-tooltip "恢复为未完成"(top) > button.action-btn.restore  RefreshLeft 16   （click.stop）
   │     └─ el-tooltip "删除"(top)        > button.action-btn.delete   Delete 16        （click.stop）
   ├─ div.empty-state  Search 40 + "未找到匹配的待办"     （结果为空且有搜索词）
   └─ div.empty-state  Check 40  + "暂无已完成的待办"     （结果为空且无搜索词）
```

| 节点 | 样式 |
|---|---|
| `.title-bar` | flex，`align-items:center; justify-content:space-between; height:42px; padding:0 12px 0 16px; background:#F8FAFC; border-bottom:1px solid #E2E8F0; user-select:none; flex-shrink:0` |
| `.title-text` | flex，`align-items:center; gap:8px; font-size:14px; font-weight:600; color:#1E293B` |
| `.count-badge` | inline-flex 居中，`min-width:20px; height:20px; padding:0 6px; border-radius:10px; background:#E2E8F0; font-size:12px; font-weight:500; color:#64748B` |
| `.close-btn` | 28×28 flex 居中，`border:none; border-radius:6px; background:transparent; color:#94A3B8; cursor:pointer; transition: all .15s`；hover `background:#FEE2E2; color:#EF4444`；图标为 10×10 SVG：`<path d="M1 1L9 9M9 1L1 9" stroke="currentColor" stroke-width="1.5" stroke-linecap="round"/>` |
| `.search-bar` | `padding:12px 16px; flex-shrink:0` |
| `.search-input-wrapper` | flex，`align-items:center; gap:8px; padding:0 12px; height:36px; background:#F1F5F9; border-radius:8px; border:1px solid transparent; transition: all .2s`；`:focus-within` `background:#FFFFFF; border-color:#93C5FD; box-shadow:0 0 0 3px rgba(59,130,246,.1)` |
| `.search-icon` | `color:#94A3B8; flex-shrink:0` |
| `.search-input` | `flex:1; border:none; outline:none; background:transparent; font-size:13px; color:#334155; line-height:1`；placeholder `#94A3B8` |
| `.search-clear` | 20×20 flex 居中，`border:none; border-radius:4px; background:transparent; color:#94A3B8; cursor:pointer; flex-shrink:0`；hover `background:#E2E8F0; color:#64748B` |
| `.completed-list` | `flex:1; overflow-y:auto; padding:0 16px 16px` |
| `.completed-item` | flex，`align-items:center; gap:12px; padding:12px 16px; margin-bottom:8px`（最后 0）；`background:#F8FAFC; border-radius:10px; cursor:pointer; transition: all .2s`；hover `background:#F1F5F9` 且 `.item-actions` 不透明 |
| `.item-color` | 8×8 圆，`flex-shrink:0` |
| `.item-content` | `flex:1; min-width:0` |
| `.item-title` | `font-size:14px; color:#64748B; text-decoration:line-through; white-space:nowrap; overflow:hidden; text-overflow:ellipsis` |
| `.item-time` | flex，`align-items:center; gap:4px; margin-top:4px; font-size:12px; color:#94A3B8` |
| `.item-actions` | flex，`align-items:center; gap:4px; opacity:0; transition: opacity .15s`（行 hover 显示） |
| `.action-btn` | 32×32 flex 居中，`border:none; border-radius:6px; cursor:pointer; transition: all .15s` |
| `.restore` | `background:#DBEAFE; color:#3B82F6`；hover `#BFDBFE` / `#2563EB` |
| `.delete` | `background:#FEE2E2; color:#EF4444`；hover `#FECACA` / `#DC2626` |
| tooltip | EP dark tooltip（§1.2），上方弹出 |
| `.empty-state` | flex column 居中，`padding:60px 20px; color:#94A3B8; text-align:center`；图标 40px `margin-bottom:12px; opacity:.5`；文字 14px |

---

## 11. 应用内通知（NotificationView.vue + services/notification.rs）

### 11.1 窗口（由 Rust 创建）

| 属性 | 值 |
|---|---|
| 触发 | 通知调度器每整分钟扫描到期提醒；settings `notification_type` = `app` 时走应用内通知（否则系统通知：标题 = 待办标题，正文 = 描述或 `待办事项提醒`） |
| label | `notification_{n}`（进程内计数器自增） |
| URL | `index.html#/notification?title={urlencode(标题)}&description={urlencode(描述 ?? '待办事项提醒')}&label={urlencode(label)}` |
| 尺寸 | 内尺寸 320×120（逻辑像素）；`resizable:false` |
| 外观 | `decorations:false`；`transparent:true`（非 macOS）；`always_on_top:true`；`skip_taskbar:true`；`focused:false`（不抢焦点）；`visible:true`；OS 标题 `通知` |
| 自动关闭 | **无**（一直显示到用户点击） |
| 多条叠放 | 见 11.2 |

### 11.2 定位与叠放算法

```
MARGIN = 20, SPACING = 10                          // 逻辑像素
area = 主显示器 work area（物理 / scale → 逻辑；取不到 work area 用整块显示器；无显示器用 0,0,1920,1080）
slot = 当前未占用的最小编号（BTreeSet）；窗口 Destroyed 时归还
size = 上次观测到的真实窗口尺寸 ?? 请求尺寸 320×120
rows = max(1, floor((area.h − 2·MARGIN + SPACING) / (size.h + SPACING)))
cols = max(1, floor((area.w − 2·MARGIN + SPACING) / (size.w + SPACING)))
slot' = slot mod (rows·cols)
col = slot' / rows ; row = slot' mod rows
x = area.x + area.w − MARGIN − size.w − col·(size.w + SPACING)
y = area.y + area.h − MARGIN − size.h − row·(size.h + SPACING)
```
即从工作区右下角开始自下而上堆叠，一列满了向左开新列，全部占满后取模复用。建窗后读真实外框尺寸（某些平台强加最小尺寸，如 GTK 实际 320×200），与摆放时假设相差 ≥0.5 逻辑像素则按真实尺寸重算本槽位并移动，同时记住真实尺寸供后续通知使用；`Resized` 事件也会触发一次校正。

### 11.3 前端

- `title = query.title || '待办提醒'`；`description = query.description`（空串则不渲染）；`label = query.label`。query 只解码一次（不能再 decodeURIComponent，含 `%` 的标题会抛错）。
- 描述是待办的**原始 Markdown 文本**（不渲染）。
- 整个窗口任意位置单击 = 关闭；右上角关闭按钮同样关闭（`click.stop`）。
- 关闭：`invoke('close_notification_window', { windowLabel: label })`（后端找到窗口则 `close()`）；label 为空或调用失败 → `appWindow.close()`。
- 后端另有 `close_all_notification_windows`（关闭所有 `notification_` 前缀窗口），已注册但前端无调用方。

### 11.4 元素树与样式

```
div.notification-window      100vw×100vh; flex 居中; background:transparent; cursor:pointer   → click 关闭
└─ div.notification-card     100%×100%
   ├─ div.notification-header
   │  ├─ div.notification-icon    Bell 24（白）
   │  └─ button.close-btn         Close 14
   ├─ div.notification-content
   │  ├─ h3.notification-title    {{ title }}
   │  └─ p.notification-desc      {{ description }}（非空）
   └─ div.notification-footer
      └─ span.notification-time   "Mini Todo"
```

| 节点 | 样式 |
|---|---|
| `.notification-card` | `background:rgba(255,255,255,.98); border-radius:12px; box-shadow:0 8px 32px rgba(0,0,0,.15), 0 2px 8px rgba(0,0,0,.1); border:1px solid rgba(0,0,0,.05)`；flex column；`overflow:hidden`（卡片撑满窗口，阴影基本被窗口边界裁掉，只剩透明窗口里的圆角卡片） |
| `.notification-header` | flex，`align-items:center; justify-content:space-between; padding:12px 14px 8px`（高 56） |
| `.notification-icon` | 36×36 flex 居中，`background: linear-gradient(135deg, #3B82F6 0%, #06B6D4 100%); border-radius:8px; color:#FFFFFF` |
| `.close-btn` | 24×24 flex 居中，`border:none; background:#F1F5F9; border-radius:6px; color:#64748B; cursor:pointer; transition: all .2s`；hover `background:#E2E8F0; color:#334155` |
| `.notification-content` | `flex:1; padding:0 14px; overflow:hidden` |
| `.notification-title` | `margin:0 0 4px; font-size:14px; font-weight:600; color:#1E293B; white-space:nowrap; overflow:hidden; text-overflow:ellipsis`（行高继承 1.5 → 21px） |
| `.notification-desc` | `margin:0; font-size:12px; color:#64748B; line-height:1.4`；最多 2 行省略（`-webkit-line-clamp:2`） |
| `.notification-footer` | `padding:8px 14px 12px`；flex 右对齐（高 36.5） |
| `.notification-time` | `font-size:11px; color:#94A3B8`，文字固定 `Mini Todo`（不是时间） |

> 实测换算：120 − 卡片上下边框 2 − header 56 − footer 36.5 = 内容区约 25.5px，标题占 21 + 4 = 25px，**描述行在 Windows（窗口严格 320×120）上实际被裁掉、看不见**；只有平台强制更高窗口时（如 GTK 200px）才显示描述。这是现状，GPUI 若要"一致"应保持同样的盒模型。

---

## 12. 后端命令契约汇总（GPUI 中直接调用 core 函数）

| 命令 | 参数（JSON，camelCase） | 返回 | 主要错误文案 |
|---|---|---|---|
| `get_todo` | `{ id }` | `Todo`（含 `subtasks`，按 sort_order, id 排序） | `待办不存在` |
| `get_todos` | – | `Todo[]`（`completed ASC, sort_order ASC, created_at DESC`） | |
| `create_todo` | `{ data: CreateTodoRequest }` | `Todo` | 时间格式错误 |
| `update_todo` | `{ id, data: UpdateTodoRequest }` | `Todo` | `No fields to update`、`待办不存在`、时间格式错误 |
| `delete_todo` | `{ id }` | – | |
| `create_subtask` | `{ data: { parentId, title, content? } }` | `SubTask`（sort_order = 同父最大值 + 1；不接受 completed） | |
| `update_subtask` | `{ id, data: { title?, content?, completed?, sortOrder? } }` | `SubTask` | `No fields to update` |
| `get_subtask` | `{ id }` | `SubTask` | `Query returned no rows` |
| `delete_subtask` | `{ id }` | – | |
| `reorder_subtasks` | `{ ids: number[] }` | – | 下标即 sort_order，只更新变化的行 |
| `import_subtasks_from_paths` | `{ parentId, paths: string[] }` | `SubTask[]` | §3.12 |
| `get_images_dir` | – | images 目录绝对路径 | |
| `save_subtask_image` | 原始字节 + header `x-image-ext` | 图片绝对路径 | §5.6 |
| `close_notification_window` | `{ windowLabel }` | – | |

类型（types/todo.ts）：`Todo { id, title, description|null, color, quadrant(1-4), notifyAt|null, notifyBefore, notified, completed, sortOrder, startTime|null, endTime|null, createdAt, updatedAt, repeatEnabled?, repeatType?('daily'|'weekly'|'monthly'|null), repeatInterval?, repeatWeekdays?("1,3,5"|null), repeatMonthDay?(1-31|null), subtasks: SubTask[] }`；`SubTask { id, parentId, title, content|null, completed, sortOrder, createdAt, updatedAt }`。

---

## 13. 中文文案总表（逐字）

| 位置 | 文案 |
|---|---|
| 编辑器 h2 | `待办详情` / `编辑待办` / `新建待办` |
| 编辑器按钮 | `编辑`、`完成任务`、`重新打开`、`取消`、`保存`、`创建`、`添加` |
| 编辑器 label | `标题`、`描述`、`颜色`、`四象限`、`时间范围`、`提醒时间` / `首次提醒`、`提前提醒`、`重复`、`重复方式`、`每 {n} 天|周|月`、`星期`、`每月几号` |
| placeholder | `请输入待办标题`、`开始日期`、`截止日期`、`时间`、`选择日期`、`添加子任务...`、`输入 Markdown 源码...`、`请输入子任务标题`、`搜索已完成的待办...` |
| 提前提醒选项 | `准时`、`5 分钟前`、`15 分钟前`、`30 分钟前`、`1 小时前`、`自定义`；数字框后缀 `分钟前` |
| 重复方式选项 | `每天`、`每周`、`每月` |
| 星期 | `周一`、`周二`、`周三`、`周四`、`周五`、`周六`、`周日` |
| 预设色 title | `红色`、`橙色`、`黄色`、`绿色`、`青色`、`蓝色`、`紫色`、`粉色` |
| 象限名 | `重要且紧急`、`重要不紧急`、`紧急不重要`、`不紧急不重要` |
| 原生 title 提示 | `放大编辑`、`最大化` / `还原`（描述弹窗）、`拖拽排序`、`查看子任务`、`编辑子任务`、`删除子任务`、`导入子任务`、`最大化`、`关闭`（子任务窗口） |
| 只读 | `暂无描述`；提醒文案见 §3.5 |
| 子任务面板 | `子任务`、`{done} / {total}`、`已完成`、`暂无子任务`、`{n} 个子任务尚未创建，点击「保存」重试` |
| 导入菜单 | `选择文件 (.md/.txt)`、`选择文件夹（递归导入）`；对话框标题 `导入子任务（选择 .md/.txt 文件或文件夹）`、`选择文件夹（递归导入 .md/.txt 文件）`；过滤器名 `文本文件` |
| 描述弹窗 | `编辑描述`、`取消`、`确定` |
| 子任务窗口 | `查看子任务` / `编辑子任务`、`标题`、`内容 (Markdown)`、`关闭` / `取消`、`保存` |
| 已完成窗口 | `已完成`、tooltip `恢复为未完成` / `删除`、`未找到匹配的待办`、`暂无已完成的待办` |
| 通知 | 默认标题 `待办提醒`；默认描述（后端）`待办事项提醒`；页脚 `Mini Todo` |
| 确认框 | `放弃修改` / `有未保存的修改，确定放弃并关闭吗？` / `放弃` / `继续编辑`；`删除确认` / `确定删除子任务"{title}"吗？` / `删除` / `取消`；`删除确认` / `确定要删除待办"{title}"吗？` / `删除` / `取消` |
| 成功提示 | `待办已创建`、`待办已保存`、`成功导入 {n} 个子任务` |
| 警告提示 | `请先保存待办后再导入子任务`、`不支持的图片格式：{name 或 type}（支持 png / jpg / webp / gif / bmp）`、`图片超过 20MB，未插入：{name}` |
| 失败提示（`{前缀}：{详情}`） | `加载待办失败`、`创建待办失败`、`待办已创建，但重复提醒未保存，请点击「保存」重试`、`{n} 个子任务未能创建，请点击「保存」重试`、`保存待办失败`、`标记完成失败`、`重新打开失败`、`添加子任务失败`、`导入失败`、`更新子任务失败`、`删除子任务失败`、`修改子任务标题失败`、`子任务排序保存失败`、`打开子任务窗口失败`、`加载子任务失败`、`保存子任务失败`、`图片保存失败`、`加载已完成待办失败`、`恢复待办失败`、`删除待办失败`、`打开编辑器失败` |
| 后端错误 | 见 §3.8.4、§3.12、§5.6、§12 |

---

## 14. GPUI 还原难点与方案

### 14.1 Markdown：Milkdown 所见即所得 → "源码编辑 + 渲染预览"（PRD D7）

GPUI / gpui-component 0.7.3 没有 ProseMirror 式富文本编辑器，无法 1:1 还原"在渲染结果上直接编辑"（输入规则、选中节点、表格内 Tab 跳格、粘贴 HTML 转富文本等）。本地源码核实到的可用部件：

| 需求 | gpui-component 0.7.3 / gpui-base 0.7.3 能力（本地 `~/.cargo/registry/src/*/gpui-component-0.7.3`、`gpui-base-0.7.3`） |
|---|---|
| 只读渲染 | `TextView::markdown(id, text)`（解析器 `ParseOptions::gfm()`：表格 / 任务清单 / 删除线 / 自动链接）；`.style(TextViewStyle)` 可设 `paragraph_gap`、`heading_font_size(Fn(level, base))`、`code_block`/`table`/`table_head`/`table_cell` 的 `StyleRefinement`、`inline_code: HighlightStyle`；`.selectable(bool)`；`.scrollable(bool)` |
| 链接白名单 | `.on_link_click(Fn(&url, &ClickEvent, …))`：**必须设置**，否则默认对左键 / 中键直接 `open_url`，会绕过 §7 白名单（file:/// 定位、UNC 拒绝、javascript: 拦截） |
| 图片引用映射 | gpui-base `TextView::image_source(Fn(&SharedUri) -> ImageSource)`：把 `minitodo-image://<name>` 与旧 asset URL（§6 正则组 2）映射成 `images_dir/<name>` 本地文件；compat 包装层未转出此方法，需要直接用 `gpui_base::TextView` 或 `TextViewState` |
| 自定义块 | `.markdown_block_parser` / `.markdown_block_renderer`：可用于任务项复选框外观、`<br />` 空行、表格奇数列底色等 |
| 源码编辑 | `Textarea` / `Editor`（多行，Editor 可开 markdown 语法高亮）；`.on_paste(Fn(&ClipboardItem, …) -> bool)` 可拦截剪贴板图片 / 文件；拖入用 GPUI `on_drop::<ExternalPaths>` |

建议方案：
1. **只读排版**（编辑器只读详情、子任务查看、描述弹窗右侧预览）：`TextView::markdown` + 按 §5.5 有效值配置样式；链接点击走 §7（只读语义：普通单击即打开）；图片点击打开自绘预览层。
2. **编辑排版**（编辑器描述框、子任务编辑）：多行源码编辑器（等宽或正文字体均可；描述框保持 min 120 / max 280 高、1px `#E2E8F0` 边、圆角 8、focus 边 `#3B82F6`；子任务内容区 min 300 撑满）。可选在源码下方 / 侧边显示实时预览，或提供"编辑 / 预览"切换；描述放大弹窗保持左源码（13px/1.6 等宽、底 `#F8FAFC`、padding 12/14）+ 1px 分隔 + 右预览（300ms 防抖）的现有布局，最大化联动窗口最大化（记住进入前是否已最大化）。
3. **图片上传**：`on_paste` 中遇到图片（`ClipboardEntry::Image`）或文件路径 → 按 §5.6 前端规则判扩展名 / 大小（同样的警告文案）→ 直接调用 core 的 `store_image(images_dir, ext, bytes)` → 在光标处插入 `![<alt>](minitodo-image://<name>)`（alt：有文件名用文件名，剪贴板位图没有文件名时可用 `image.<ext>` 对齐浏览器的 `image.png`）。拖入文件同理（插入光标处或拖放位置）。
4. **数据**：载入编辑器前先 `to_storage_markdown`（把旧 asset URL 规范化）；保存时再跑一次；快照比较用存储形式（§3.9）。渲染前把独占一行的 `<br />`（`^<br\s*/?\s*>$`，大小写不敏感）当空段落；转义字符（`\[` 等）由 Markdown 解析器自然处理。
5. **失去的能力（写入已知偏差）**：Markdown 输入规则自动转换、Ctrl+B/I/E 等格式快捷键（可选在源码编辑器里实现"包裹选区"作为补偿）、粘贴 HTML 富文本转换、所见即所得表格编辑、编辑中 Ctrl+点击链接（源码里没有可点击链接，预览中按只读语义）。任务清单在原版中也不可点，无损失。

各 Markdown 元素在 GPUI 渲染时需要的样式（直接取 §5.5 表）：段落 16px/1.6 上下 6.4；h1-h6 30/24/20/16/14/14 px、字重 700/600/500/500/500/500、上下边距见表；行内代码 `#5E81AC` 字 + `#F1F5F9` 底 + 2/6 padding + 圆角 4、等宽 0.9em；代码块 `#1E293B` 底 `#E2E8F0` 字 padding 12/16 圆角 8、14px/20px、折行；引用 3px `#3B82F6` 左线 + 12 左内边距 + `#64748B` 字 + 衬线字体；列表 24 缩进、标记 `#5E81AC`、项间 4；任务框 13×13 1.5px `#94A3B8` 圆角 3 / 勾选 `#3B82F6`；hr 1px `#E2E8F0` 上下 14；表格 14px、全宽等分列、单元格 1px `#E2E8F0` 边、padding 12/24、表头底 `#F8FAFC` 600、奇数列 td 底 `#F9FAFB`、外阴影；图片圆角 6、外阴影、hover 透明度 .85、点击预览；链接下划线、同正文色。

### 14.2 其它控件映射

| 原组件 | GPUI 建议 | 注意 |
|---|---|---|
| el-input / el-select / el-switch / el-checkbox / el-input-number | gpui-component Input / Select / Switch / Checkbox / NumberInput | 颜色要改成 EP 默认蓝 `#409EFF` 系（不是 `#3B82F6`），尺寸 32 / 24 |
| el-date-picker + el-time-picker | `time::DatePicker` + 自绘时 / 分选择（或 time_field） | 面板向上弹出不翻转；时间框在日期为空时禁用；原版面板为英文（月份 / 星期 / Cancel / OK） |
| el-color-picker | gpui-component ColorPicker + 预设色 | 24×24 触发器；预设 8 色 |
| el-dialog / ElMessageBox | Dialog / AlertDialog | 遮罩 `rgba(0,0,0,.5)`；确认按钮顺序"取消在左、确认在右"；Esc / 点遮罩 = 取消（描述弹窗点遮罩不关） |
| ElMessage | Notification / 自绘顶部居中 Toast | 3s；四种配色（§1.2） |
| el-dropdown | Popover + 菜单 | |
| el-tooltip / 原生 title | Tooltip | 已完成窗口用 EP 深色 tooltip，其余是原生 title |
| vuedraggable | GPUI `on_drag` / `on_drop` 自绘 | 只有手柄可拖；占位 50% 透明、拖影 80% 透明跟随鼠标；200ms 让位动画 |
| el-image-viewer | 自绘全屏预览层 | 缩放 / 旋转 / 翻页 / Esc |

### 14.3 窗口层面

- **owner（parent）关系**：编辑器 → 主窗口 / 已完成窗口，子任务 → 编辑器。需要 Win32 `SetWindowLongPtrW(GWLP_HWNDPARENT)` 或 GPUI 若支持的 parent 选项，保证子窗口浮在打开方之上、不单独占任务栏；托盘新建的编辑器无 owner。
- **模态模拟**：打开方显示遮罩（主窗口 / 已完成 30% 黑、编辑器 15% 黑），打开方被点击或获得焦点时把子窗口激活到前台（编辑器打开子任务窗口时原版没有回焦逻辑）；子窗口关闭后刷新打开方（只刷新当前待办）。
- **无系统标题栏 + 自绘拖拽**：GPUI `titlebar: None` + 在标题栏区域 `start_window_move`；双击空白处切换最大化；按钮 / 输入区不触发拖动。Windows 上原版有 DWM 阴影与圆角（Tauri `shadow:true`）。
- **通知窗口**：无边框、透明背景、置顶、不进任务栏、**不抢焦点**、不可缩放；位置用 §11.2 算法（主显示器 work area、逻辑像素）。GPUI 需确认 `WindowKind::PopUp` / `focus:false` / 透明背景在 Windows 上的表现。
- **窗口最大化 API**：描述弹窗"最大化"联动窗口最大化，子任务窗口有最大化按钮——需要 GPUI 的 maximize / restore / is_maximized（`Window::zoom_window` 语义在不同平台不同，需实测 gpui-pre 0.3.9）。

### 14.4 现状中的"非直觉"行为（1:1 还原时注意，不是建议修改）

1. 待办编辑器底部按钮组因 `space-between` + 单子元素而**靠左**；子任务窗口底部靠右。
2. 标题栏 X / 「取消」不做未保存确认，只有 Esc 确认。
3. 保存 / 完成成功后的 `ElMessage.success` 出现在正在关闭的窗口里，实际看不到。
4. 编辑器"保存"在标题为空时可点但静默无反应；子任务"保存"在标题为空时禁用。
5. 应用内通知的描述行在 320×120 窗口内被裁掉（§11.4）。
6. 描述弹窗全屏态仍有 5vh 顶边距与 12px 圆角（main.scss `!important` 覆盖了 EP 全屏样式）。
7. EP 组件主色 `#409EFF` 与项目自绘元素 `#3B82F6` 并存。

---

## Caveats / Not Found

- 未在真实 WebView 中截图核对；Element Plus 组件的像素值来自 theme-chalk 2.13.1 CSS 源码，Milkdown 渲染有效值是按 CSS 层叠规则（`@layer`、specificity、`!important`）推算的，个别值（如表格外边框颜色在 collapse 模型下被单元格边框覆盖、`min-height: inherit` 的最终取值依赖样式注入顺序）建议用 Vue 版在浏览器中渲染后再对照一次（PRD 验收标准里已有参考截图步骤）。
- `ElMessageBox` 按钮尺寸按 EP 2.x 默认（default 32px）记录，未从 EP JS 源码二次确认。
- vuedraggable fallback 克隆体的 `opacity:.8` 等来自 SortableJS 1.14 默认行为（未读其源码，按已知默认值记录）。
- Milkdown `markdownUpdated` 的防抖时长（约 200ms）为 listener 插件默认值，未读源码确认；不影响 GPUI 方案。
- 任务描述中提到的"CompletedView 清空 / 分页 / 分组"、"NotificationView 稍后提醒 / 完成按钮 / 自动关闭"、"TodoItem 打开窗口"在现有代码中**都不存在**。
- 依赖包仅下载到会话 scratchpad 阅读（`/tmp/claude-0/-home-user-mini-todo/9b3d7942-6fc3-57a4-8b18-2becb0556be0/scratchpad/pkgs/`），未改动仓库其它文件。

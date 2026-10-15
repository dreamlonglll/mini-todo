# Research: 主窗口 UI 规格（Tauri + Vue → Rust + GPUI 重写参考）

- **Query**: 为主窗口写一份足够精确、可直接照着用 GPUI（gpui-pre 0.3.9 + gpui-component 0.7.3）重新实现的 UI 规格：窗口、全局样式、元素树、TitleBar、TodoList/TodoItem、QuadrantView、CalendarView、FAB、状态与逻辑、全部中文文案、GPUI 还原难点
- **Scope**: internal（源码通读）+ external（Element Plus 2.13.1 / @element-plus/icons-vue 2.3.2 / sortablejs 1.14.0 / lunar-javascript 1.7.7 / tauri 2.9.5 drag.js 的默认样式与行为，均从 npm/crates 包源码核对）
- **Date**: 2026-10-15
- **说明**: 所有 px 均为 CSS 像素 = Tauri 逻辑像素（DIP）。"浅色 / 深色" 指应用自己的主题开关（`isDarkTheme`），不是系统主题。CSS 变量已解析成最终值。凡标注"按 CSS 推断，未实测"的地方，是在没有浏览器的环境下按层叠规则推出的结论。

---

## 0. 源文件清单

| 文件 | 作用 |
|---|---|
| `pc/src/main.ts` | 入口：Pinia、Router、加载 `styles/main.scss`；生产模式禁用右键菜单与刷新/开发者工具快捷键 |
| `pc/src/App.vue` | 只有 `<router-view />` |
| `pc/src/router/index.ts` | hash 路由：`/` → MainView（主窗口），`/editor` `/settings` `/notification` `/subtask-editor` `/completed` `/fence` 为子窗口 |
| `pc/src/views/MainView.vue` | 主窗口根：布局、模态遮罩、FAB、所有监听/定时器/同步逻辑、打开子窗口 |
| `pc/src/components/TitleBar.vue` | 标题栏 |
| `pc/src/components/TodoList.vue` | 列表视图（vuedraggable）+ 空状态 |
| `pc/src/components/TodoItem.vue` | 单个待办行（列表与四象限共用） |
| `pc/src/components/QuadrantView.vue` | 四象限视图 |
| `pc/src/components/CalendarView.vue` | 日历（异步组件，懒加载） |
| `pc/src/styles/main.scss` | 全局样式、CSS 变量、深色主题、固定模式、TodoItem 样式、FAB |
| `pc/src/stores/appStore.ts` / `todoStore.ts` / `index.ts` | Pinia 状态 |
| `pc/src/utils/quadrant.ts` `color.ts` `holiday.ts` `lunar.ts` `syncReport.ts` `notify.ts` `datetime.ts` | 工具函数 |
| `pc/src/types/todo.ts` `app.ts` `lunar-javascript.d.ts` | 类型与常量 |
| `pc/src/plugins/element.ts` | `ElMessage` / `ElMessageBox` 唯一入口 |
| `pc/index.html` | `<html lang="zh-CN">`，内联 `body{background:#FFFFFF}`（被 main.scss 的 `background: var(--bg-primary)` 覆盖为 transparent） |
| `pc/src-tauri/tauri.conf.json` | 主窗口参数 |
| `pc/src-tauri/src/lib.rs` | 主窗口 DWM 圆角、托盘菜单与 `tray-*` 事件 |
| `pc/src-tauri/src/commands/window.rs` | 固定/嵌入/贴边隐藏、`get_window_persist_state`、`get_window_background` 等 |
| `pc/src-tauri/src/commands/holiday.rs` | 节假日数据源 |
| `docs/images/list.png` `grid.png` | 旧版（v1.4.3）截图，日历开启 + 深/浅色斜切对比，可作视觉参考（无主题/同步/展开按钮，属旧版） |

---

## 1. 窗口本身

### 1.1 窗口参数（`tauri.conf.json` + `lib.rs`）

| 属性 | 值 |
|---|---|
| title | `Mini Todo` |
| 默认尺寸 | 380 × 600 |
| 最小尺寸 | 320 × 400 |
| resizable | true（固定模式时前端 `setResizable(false)`） |
| decorations | false（无系统标题栏，无关闭/最小化按钮——主窗口界面里也**没有**关闭/最小化按钮，只能经托盘「退出」或 Alt+F4） |
| transparent | true（像素级透明，界面靠 CSS 画圆角矩形） |
| shadow | false（无系统阴影） |
| alwaysOnTop | false |
| center | true（之后由前端 `initSettings` 重新定位） |
| Windows 圆角 | 启动时 `DwmSetWindowAttribute(DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND)`（Win11 系统级 8px 圆角裁剪；Win10 无此效果，靠 CSS 圆角） |
| macOS | WKWebView 底色置为 (0,0,0,0) |

启动定位（`appStore.initSettings`）：
- 生成屏幕组合 ID（见 §14.1），`get_screen_config { configId }` 有记录 → 恢复 `PhysicalPosition(windowX, windowY)`、`PhysicalSize(windowWidth, windowHeight)`，`isFixed = savedConfig.isFixed`，若固定再 `applyFixedMode()`（先定位后固定）。
- 无记录 → 主显示器居中：逻辑 380×600，物理坐标 `centerX = round(monitor.x + (monitor.w − 380·scale)/2)`，`centerY` 同理，`setSize(Physical(380·scale, 600·scale))`，然后 `saveWindowState()` 建初始记录；`isFixed = false`。

### 1.2 外观分层（自外向内）

```
窗口（全透明）
└─ html, body           背景 var(--bg-primary)：浅色 transparent；深色 body 强制 transparent !important
   └─ #app              100%×100%；仅深色时填底色 rgba(R,G,B,A) 且 border-radius 8px（固定模式下 0）
      └─ .app-container 100% 高；浅色：#FFFFFF 底 + 1px rgba(0,0,0,0.5) 描边 + 8px 圆角
                        深色：底色被强制 transparent（!important），描边 rgba(255,255,255,0.5)，8px 圆角
```

`.app-container` 用 `box-sizing: border-box`（全局 `*`），1px 描边画在窗口最外圈像素内侧；`overflow: hidden`。

### 1.3 四种外观组合（GPUI 需要实现的最终效果）

| 组合 | 窗口底 | 圆角 | 外描边 | 说明 |
|---|---|---|---|---|
| 浅色 · 普通 | `#FFFFFF` 不透明 | 8px | 1px `rgba(0,0,0,0.5)` | 圆角外的 4 个角透明 |
| 浅色 · 固定（经典固定，未嵌入） | `#FFFFFF` 满铺矩形 | 0 | 无 | `body.fixed-mode` 生效 |
| 深色 · 普通 | `rgba(R,G,B,A)`（用户底色/透明度） | 8px | 1px `rgba(255,255,255,0.5)`（叠在底色之上） | 透出桌面 |
| 深色 · 固定（经典固定） | `rgba(R,G,B,A)` 满铺矩形 | 0 | 无 | |
| 任一主题 · 固定 + 嵌入桌面 | 同"普通" | 同"普通" | 同"普通" | 嵌入态**不挂** `body.fixed-mode`，保留圆角与描边 |

- `body.fixed-mode` 的挂载条件（MainView `watch`，immediate）：`appStore.isFixed && !appStore.isEmbeddedInDesktop`，其中 `isEmbeddedInDesktop = IS_WINDOWS && isFixed && fixedEmbedDesktop`。
- 固定模式下除上表外的视觉差异：只有标题栏锁按钮高亮（`Lock` 图标、主色）。FAB、列表、hover 等全部照常。行为差异：标题栏不可拖动、窗口不可缩放。

### 1.4 窗口底色与透明度（`window_bg_color` / `window_bg_alpha`）

- **只在深色主题下生效**；浅色主题完全忽略（浅色永远是 #FFFFFF 不透明）。
- 来源：`invoke('get_window_background') → { color: string, alpha: number }`，缺省 `#000000` / `0.45`（后端 `DEFAULT_WINDOW_BG_COLOR/ALPHA`；`set_window_background` 把 alpha clamp 到 0..1）。
- 前端写入（仅 label 为 `main` 的窗口）：`--app-bg-rgb = hexToRgbChannels(color)`（`#RGB` / `#RRGGBB` → `"r, g, b"`，非法回落 `"0, 0, 0"`），`--app-bg-alpha = String(alpha)`。
- 用于：`body.dark-theme #app { background: rgba(var(--app-bg-rgb), var(--app-bg-alpha)) }`，并同时作为深色下的 `--bg-primary` 与 `--bg-secondary`（只有标题栏"今天"按钮的浅色态用到 `--bg-secondary`，深色态它被覆盖为 transparent，所以实际上只有 #app 这一层填色）。
- 预设色（设置窗口用，`types/app.ts PRESET_BG_COLORS`）：黑色 `#000000`、深灰 `#1E293B`、深蓝 `#0F2942`、深紫 `#2E1065`、深绿 `#052E24`、深棕 `#2B1A0F`。
- 底色只画一层：所有内部容器（`.app-container`、`.title-bar`、`.main-content`、`.right-panel`、`.todo-item`）在深色下都被 `!important` 强制透明，"用户设的透明度所见即所得"。

### 1.5 深色主题的触发

- `isDarkTheme = (get_text_theme === 'light')`（**注意语义反转**：settings 键 `text_theme` 描述文字颜色，`'light'` = 浅色文字 = 深色主题开启；后端缺省 `'dark'` → 浅色主题）。
- 开启时 `document.body.classList.toggle('dark-theme', true)`（仅主窗口）；同时 `.app-container`、`.title-bar`、`.right-panel`、`.quadrant-view`、`.calendar-view` 各自也带 `dark-theme` 类。

### 1.6 字体

| 用途 | font-family | 字号 / 行高 | 备注 |
|---|---|---|---|
| 全局（body） | `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Microsoft YaHei', sans-serif` | 14px / 1.5 | Windows 上实际：拉丁字符 Segoe UI，中文回退 Microsoft YaHei；`-webkit-font-smoothing: antialiased` |
| 待办标题 `.todo-title` | `var(--todo-font-stack)` | `var(--todo-font-size)`（默认 14px）/ 1.5 | |
| 待办元信息 `.todo-meta`、子任务行 `.subtask-item` | `var(--todo-font-stack)` | `var(--todo-sub-font-size)`（= 字号 − 2，默认 12px）/ 1.5 | |
| Element Plus 按钮文字（"今天"） | 未设置 → Chromium 按钮默认 `system-ui` | 12px（small） | Windows 中文系统一般是 Microsoft YaHei UI |

字体设置生效流程（`appStore.loadTodoFontSettings` → `applyTodoFontStyles`）：
- `get_todo_font_family → string`（缺省 `''`），`get_todo_font_size → i32`（缺省 14；后端写入时 clamp 12..20）。
- `--todo-font-stack = family ? "\"<family>\", -apple-system, \"Segoe UI\", \"Microsoft YaHei\", sans-serif" : "-apple-system, \"Segoe UI\", \"Microsoft YaHei\", sans-serif"`
- `--todo-font-size = <size>px`，`--todo-sub-font-size = <size − 2>px`
- 触发：`initSettings` 末尾；事件 `todo-font-changed`（设置窗口改字体后发出）；`reloadAppSettings()`。
- 只影响待办标题、元信息行、子任务行；标题栏、四象限头、日历、空状态都不受影响。

### 1.7 其它全局行为（`main.ts`，仅生产构建）

- 禁用右键菜单（`contextmenu` preventDefault）→ 主窗口**没有任何右键菜单**。
- 屏蔽按键：F5、F12；Ctrl/Cmd + R U S P G F J H D E；Ctrl/Cmd + Shift + I J C R N。GPUI 版本无浏览器快捷键，无需对应实现。

### 1.8 macOS 差异（`body.platform-macos`，非目标平台，仅记录）

深色下 `--bg-tertiary` 为 `rgba(255,255,255,0.12)`；待办行 hover 背景 `rgba(255,255,255,0.12) !important`（Windows 深色下没有 hover 背景，见 §6.6）；空状态背景 transparent。

---

## 2. 全局样式 Token（CSS 变量解析表）

### 2.1 颜色（`:root` 与 `body.dark-theme`）

| 变量 | 浅色 | 深色（Windows/Linux） |
|---|---|---|
| `--primary` | `#3B82F6` | 同 |
| `--primary-light` | `#60A5FA` | 同 |
| `--primary-dark` | `#2563EB` | 同 |
| `--bg-primary` | `transparent` | `rgba(R,G,B,A)`（用户底色，默认 `rgba(0,0,0,0.45)`） |
| `--bg-secondary` | `#F8FAFC` | `rgba(R,G,B,A)` |
| `--bg-tertiary` | `#F1F5F9` | `rgba(255,255,255,0.1)` |
| `--text-primary` | `#1E293B` | `#FFFFFF` |
| `--text-secondary` | `#64748B` | `rgba(255,255,255,0.85)` |
| `--text-tertiary` | `#94A3B8` | `rgba(255,255,255,0.65)` |
| `--border` | `#E2E8F0` | `rgba(255,255,255,0.2)` |
| `--border-light` | `#F1F5F9` | `rgba(255,255,255,0.1)` |
| `--success` | `#10B981` | 同 |
| `--warning` | `#F59E0B` | 同 |
| `--error` | `#EF4444` | 同 |
| `--priority-high/-bg` | `#EF4444` / `#FEF2F2` | 同（主窗口未使用） |
| `--priority-medium/-bg` | `#F59E0B` / `#FFFBEB` | 同（未使用） |
| `--priority-low/-bg` | `#10B981` / `#ECFDF5` | 同（未使用） |
| `--el-color-primary`（Element Plus） | `#409eff` | 同（**注意**：重复提醒徽标用的是这个 EP 蓝，不是 `#3B82F6`） |
| `--color-success` | 未定义 → 回落 `#10B981` | 同 |

### 2.2 阴影 / 圆角 / 间距 / 过渡

| 变量 | 浅色 | 深色 |
|---|---|---|
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.05)` | none |
| `--shadow-base` | `0 1px 3px rgba(0,0,0,0.1), 0 1px 2px rgba(0,0,0,0.06)` | none |
| `--shadow-md` | `0 4px 6px -1px rgba(0,0,0,0.1), 0 2px 4px -1px rgba(0,0,0,0.06)` | none |
| `--shadow-lg` | `0 10px 15px -3px rgba(0,0,0,0.1), 0 4px 6px -2px rgba(0,0,0,0.05)` | none |

| 圆角 | 值 | 间距 | 值 | 过渡 | 值 |
|---|---|---|---|---|---|
| `--radius-sm` | 4px | `--space-1` | 4px | `--transition-fast` | 150ms |
| `--radius-base` | 6px | `--space-2` | 8px | `--transition-base` | 200ms |
| `--radius-md` | 8px | `--space-3` | 12px | `--transition-slow` | 300ms |
| `--radius-lg` | 12px | `--space-4` | 16px | | |
| `--radius-xl` | 16px | `--space-5` / `--space-6` | 20px / 24px | | |

过渡函数未写时为 CSS 默认 `ease`。

### 2.3 全局重置与其它

- `* { margin:0; padding:0; box-sizing:border-box }`（按钮默认内边距也被清零）。
- 标题栏及其子元素 `user-select: none`；待办行 `user-select: none` → 主窗口里文字都不可选中。
- 滚动条（`.main-content`）：宽 6px；轨道 transparent；滑块 `var(--border)`（浅 `#E2E8F0` / 深 `rgba(255,255,255,0.2)`），圆角 3px；滑块 hover `var(--text-tertiary)`（浅 `#94A3B8` / 深 `rgba(255,255,255,0.65)`）；无箭头按钮；非覆盖式（出现时占 6px 宽度）。
- Element Plus 全局覆盖（main.scss）：`.el-button { border-radius: 6px !important }`；`.el-button > span { display:inline-flex; align-items:center; gap:6px }`；`.el-input__wrapper` 圆角 6px。

### 2.4 主窗口里不生效 / 未使用的样式（无需还原）

`.quick-add`、`.completed-section`、`.completed-list`、`.completed-btn(-wrapper)`、`.archive-btn`、`.title-btn.btn-close`、`.el-dialog *`、`.subtask-list .subtask-item.completed`、`.subtask-checkbox`、`--priority-*`、根节点的 `.with-calendar` 类（无任何规则）、`.drag-handle` / `.view-toggle-btn` 类（无规则）、所有 `-webkit-app-region`（Tauri/WebView2 下不起作用，拖动靠 `startDragging`）。

---

## 3. 元素树总览

```
Window（逻辑 380×600 起，透明，无装饰）
└─ #app（深色：底色 + 圆角 8；固定：圆角 0）
   └─ div.app-container[.dark-theme]  flex 纵向；100%高；overflow hidden；浅 #FFF；1px 描边；圆角 8
      │   事件：mouseenter/mouseleave（贴边隐藏光标上报）、mouseover/mouseout（FAB 变暗委托）
      ├─ div.modal-overlay           [v-if 有子窗口打开] position fixed 全窗；rgba(0,0,0,0.3)；z 999；cursor not-allowed；mousedown→聚焦子窗口
      ├─ TitleBar  div.title-bar      flex 横向；align center；space-between；padding 12px 16px；高约 52px
      │   ├─ div.title-left           flex；align center；gap 12；[日历开启：width 40%；min-width 280；不收缩]
      │   │   ├─ span.app-title-wrapper   flex；align baseline
      │   │   │   ├─ span.app-title       "待办清单" 16px / 600 / text-primary
      │   │   │   └─ span.version-tag     "v2.5.0" ml 4；11px；text-tertiary；nowrap
      │   │   │       └─ span.update-dot  [有新版本] 6×6 红点；ml 2；脉冲动画
      │   │   └─ button.subtask-toggle-btn 28×28；ml auto；圆角 6；图标 16（Fold/Expand）
      │   ├─ div.title-center         [v-if 日历开启] flex；align center；gap 12；flex 1
      │   │   ├─ div.calendar-nav     flex；align center；gap 4
      │   │   │   ├─ el-button(text, small).nav-btn   ArrowLeft
      │   │   │   ├─ span.current-month  "2026年10月" 15px/600；min-width 90；居中
      │   │   │   └─ el-button(text, small).nav-btn   ArrowRight
      │   │   └─ el-button(small).today-btn  "今天"
      │   └─ div.title-right          flex；align center；gap 8
      │       ├─ button.title-btn     [已完成数>0] Finished
      │       ├─ button.title-btn     视图切换（四宫格 SVG / List）
      │       ├─ button.title-btn     主题（Moon / Sunny）
      │       ├─ button.title-btn     固定（Unlock / Lock）
      │       ├─ button.title-btn     同步（Refresh，同步中旋转）
      │       └─ button.title-btn     设置（Setting）
      ├─ div.main-body[.split-layout]  flex 1；纵向（日历开启时横向）；overflow hidden
      │   ├─ div.left-panel            flex 纵向；overflow hidden；[分栏：width var(--left-panel-width, 40%)；min-width 280；不收缩]
      │   │   └─ div.main-content      flex 1；overflow-y auto；padding 0 16px 76px；margin-bottom 6px
      │   │       ├─ [列表] div.todo-list
      │   │       │   ├─ div（vuedraggable 根）→ TodoItem × N
      │   │       │   └─ div.empty-state  [没有任何待办（含已完成）时]
      │   │       └─ [四象限] div.quadrant-view → 2×2 网格 → 每格：头 + 可拖拽列表
      │   └─ div.right-panel[.dark-theme]  [v-if 日历开启] flex 1；overflow hidden；padding 12（深色 8）；透明
      │       └─ CalendarView（异步组件）
      └─ button.fab-add[.fab-dimmed]   position fixed；right 20 / bottom 20；48×48 圆；z 100（变暗时 4）；Plus 24
```

叠放关系（z-index，均在根层叠上下文里）：待办操作条 5 < FAB 100（变暗时 4，沉到操作条之下）< 模态遮罩 999 < Element Plus 弹层（tooltip/消息/确认框，2000 起递增）。日历待办条 10 / hover 15 只在日历网格内部比较。拖拽浮层（SortableJS fallback）z 100000。

注意：`position: fixed` 的元素（遮罩、FAB）不受 `.app-container` 的 `overflow: hidden` 与圆角裁剪——遮罩是满窗口矩形，会盖住透明的 4 个圆角区域（Win11 上被 DWM 圆角裁掉，看不出；Win10 上能看到直角暗色角）。

---

## 4. TitleBar（标题栏）

### 4.1 容器

| 属性 | 值 |
|---|---|
| 布局 | flex 横向，`align-items:center`，`justify-content:space-between` |
| padding | 12px 16px |
| 背景 | 透明（深色下 `rgba(0,0,0,0.1)` 被 `!important` transparent 覆盖） |
| 高度 | 由最高子元素 28px（按钮）+ 上下 24 = **52px** |
| 光标 | default；所有子元素 `user-select:none` |
| 类 | `.no-drag`（固定模式）、`.dark-theme`（深色） |
| `data-tauri-drag-region` | 固定模式 `"false"`，否则 `"deep"` |

各区 `title-left` / `title-center` / `title-right` 之间没有额外 margin（靠 space-between；日历开启时 `title-center` 是 `flex:1`，所以紧跟 `title-left` 之后左对齐）。

### 4.2 拖动窗口

`@mousedown`（标题栏根）处理逻辑：
1. `e.buttons !== 1`（非仅左键）→ 忽略；
2. `appStore.isFixed`（含嵌入桌面）→ 忽略（固定模式不可拖）；
3. 目标在 `[data-tauri-drag-region="false"]` 内（版本号、展开按钮、`title-center` 整块、`title-right` 整块）→ 忽略；
4. 目标是/在 `button, input, textarea, select, a, [role="button"]` 内 → 忽略；
5. 否则 `preventDefault()` 并 `appWindow.startDragging()`。

可拖动区域 = 标题栏的空白/内边距、"待办清单"文字、`title-left` 中的空白。
另外 Tauri 2.9.5 的 `drag.js`：当 mousedown 的**目标元素本身**带非 `"false"` 的 `data-tauri-drag-region`（即直接点在标题栏根的空白处）时，单击 `start_dragging`，**双击（detail=2）调用 `internal_toggle_maximize`（窗口可缩放时最大化/还原）**。点在"待办清单"文字上双击不会最大化。GPUI 版：`window.start_window_move()`；双击最大化可选。

### 4.3 左区 `title-left`

| 元素 | 样式 | 行为 |
|---|---|---|
| `title-left` | flex，align center，gap 12px；日历开启时 `width:40%; min-width:280px; flex-shrink:0`（CSS `:has(.title-center)`），按标题栏内容宽度的 40% | |
| `app-title-wrapper` | flex，`align-items: baseline`（标题与版本号基线对齐） | |
| `app-title` "待办清单" | 16px，600，`--text-primary` | 可拖动 |
| `version-tag` "v{APP_VERSION}" | margin-left 4px；11px；`--text-tertiary`（浅 `#94A3B8` / 深 `rgba(255,255,255,0.65)`）；nowrap；cursor default | 无更新时点击无反应。有更新（`.has-update`）：cursor pointer，hover 颜色变 `--text-secondary`；点击弹确认框（§12.5）。`APP_VERSION` 来自 `getVersion()`（当前 `2.5.0`），读取失败时只显示 "v" |
| `update-dot` | 6×6，margin-left 2px，`#EF4444`，圆形，`vertical-align: middle`；动画 `pulse 2s infinite`：0% `box-shadow: 0 0 0 0 rgba(239,68,68,0.7)` → 70% `0 0 0 4px rgba(239,68,68,0)` → 100% `0 0 0 0 rgba(239,68,68,0)`（向外扩散 4px 并淡出的光环） | 仅 `hasUpdate` 时显示 |
| `subtask-toggle-btn` | 28×28，`margin-left: auto`（日历开启时 title-left 定宽 40%，按钮被推到 title-left 右缘；非日历模式 title-left 是内容宽，按钮紧跟版本号后 12px）；padding 0；无边框；圆角 6px；背景透明；颜色 `--text-secondary`；`transition: all 150ms`；hover：背景 `--bg-tertiary`、颜色 `--text-primary`；`.active`（全部展开时）：颜色 `--primary #3B82F6`（active 优先于 hover 的颜色，hover 背景照常） | 图标 16px：展开全部状态 → `Fold`，否则 `Expand`；tooltip（原生 title）：`收起全部子任务` / `展开全部子任务`；点击 `appStore.toggleSubtaskExpandAll()` |

### 4.4 中区 `title-center`（仅日历开启时）

| 元素 | 样式（浅色） | 深色覆盖 | 行为 |
|---|---|---|---|
| `title-center` | flex，align center，gap 12px，flex 1 | | 不可拖动 |
| `calendar-nav` | flex，align center，gap 4px | | |
| `nav-btn`（左/右） | `el-button text size=small`：高 24px，padding 5px 11px，字号 12px（图标 12×12，`el-icon` 1em），圆角 6px（全局覆盖），无边框、透明底；颜色 `--text-secondary !important`；hover：颜色 `--primary #3B82F6`、背景 `--bg-tertiary #F1F5F9`（!important）；`transition: .1s`；按下（active）同 hover | 颜色 `#FFFFFF`；hover 颜色 `#60A5FA`、背景 `rgba(255,255,255,0.1)` | 左 `ArrowLeft` → `calendar-prev`；右 `ArrowRight` → `calendar-next`。无 tooltip |
| `current-month` | 15px，600，min-width 90px，居中，`--text-primary` | 白色 | 文本来自日历组件 `currentMonthText`：`${year}年${month+1}月`（如 `2026年10月`）；日历异步组件未加载完前为空串 |
| `today-btn` "今天" | `el-button size=small`（非 text）：高 24px，padding 5px 11px，12px，font-weight 500，1px 边框，圆角 6px；颜色 `--text-secondary`、背景 `--bg-secondary #F8FAFC`、边框 `--border #E2E8F0`（均 !important）；hover：颜色 `--primary`、背景 `--bg-tertiary`、边框 `--primary` | 颜色白、背景 transparent、边框 `rgba(255,255,255,0.4)`；hover：颜色 white、背景 `rgba(255,255,255,0.15)`、边框 `rgba(255,255,255,0.5)` | → `calendar-today` |

键盘聚焦（focus-visible）时 Element Plus 按钮有 `outline: 2px solid rgb(160,207,255); outline-offset: 1px`。

### 4.5 右区 `title-right` 按钮（从左到右）

公共样式 `.title-bar .title-right .title-btn`：28×28，flex 居中，圆角 6px，背景透明，无边框，颜色 `--text-secondary`（浅 `#64748B` / 深 `rgba(255,255,255,0.85)`），cursor pointer，`transition: all 150ms`；hover：背景 `--bg-tertiary`（浅 `#F1F5F9` / 深 `rgba(255,255,255,0.1)`）、颜色 `--text-primary`；`.active`：颜色 `--primary #3B82F6`（active 与 hover 同时成立时颜色取 active，背景取 hover）。图标均 16×16。区内 gap 8px。tooltip 都是原生 `title` 属性（系统 tooltip，约 0.5–1s 延迟）。

| # | 显示条件 | 图标 | tooltip | active 条件 | 功能 |
|---|---|---|---|---|---|
| 1 | `todoStore.todoCount.completed > 0` | `Finished` | `已完成` | — | 打开"已完成"子窗口（§11）。不显示数量 |
| 2 | 总是 | 列表模式：自绘四宫格 SVG；四象限模式：`List` | 列表模式 `切换到四象限视图`；四象限模式 `切换到列表视图` | `viewMode === 'quadrant'` | `setViewMode(list↔quadrant)` + `set_setting { key:'view_mode', value }` |
| 3 | 总是 | 浅色：`Moon`；深色：`Sunny` | 浅色 `切换到深色主题`；深色 `切换到浅色主题` | 深色主题 | `appStore.toggleDarkTheme()` |
| 4 | 总是 | 固定：`Lock`；未固定：`Unlock` | 固定 `取消固定`；未固定 `固定窗口` | `isFixed`（含嵌入） | `appStore.toggleFixedMode()` |
| 5 | 总是 | `Refresh` | 同步中 `同步中...`；否则 `同步数据` | —（类 `.syncing`） | 手动同步（§14.6）。同步中 `disabled` 且图标旋转 `spin 1s linear infinite`（0→360°）。自动同步进行中同样旋转+禁用（共享 `isSyncing`）。disabled 时无其它视觉变化（作者样式覆盖了浏览器默认灰显，hover 仍会变底色） |
| 6 | 总是 | `Setting` | `设置` | — | 打开设置子窗口（§11） |

四宫格 SVG（列表模式下的视图切换图标）：`width=16 height=16 viewBox="0 0 24 24" fill="currentColor"`，4 个 `rect`：`(3,3) 8×8 rx1`、`(13,3) 8×8 rx1`、`(3,13) 8×8 rx1`、`(13,13) 8×8 rx1`。

### 4.6 宽度预算与溢出（按 CSS 推断）

右区 6 个按钮 = 6×28 + 5×8 = 208px（无"已完成"时 172px）。左区约："待办清单" 64px + 版本号约 29px + 4 + gap 12 + 28 ≈ 137px（有更新红点再 +8）。窗口 380 时标题栏内容宽 380−2−32 = 346px，带"已完成"按钮时几乎刚好放下。更窄（最小 320）时 flex 会压缩：按钮是 flex item（`min-width:auto`）可被压到 16px（图标宽），中文标题可逐字换行。GPUI 版需自定溢出策略（建议保持按钮 28px 不收缩，标题截断）。

---

## 5. 主内容区与 TodoList（列表视图）

### 5.1 布局容器

| 元素 | 样式 |
|---|---|
| `main-body` | flex 1；`flex-direction: column`（日历开启 `.split-layout` → `row`）；overflow hidden |
| `left-panel` | flex 纵向；overflow hidden；非日历模式下**没有** `flex:1`（高度=内容高，超出时收缩到 main-body 高度，靠 `main-content` 滚动）；日历模式：`width: var(--left-panel-width, 40%)`、`min-width: 280px`、`flex-shrink: 0`，高度被拉伸到满 |
| `main-content` | flex 1；`overflow-y: auto`；padding **0 16px 76px**（底部 76 = FAB bottom 20 + 高 48 + 余量 8，避免最后几项的操作按钮被 FAB 盖住）；margin-bottom **6px**（贴边自动隐藏时窗口只留 4 物理像素在屏幕内，保证残留条恒为背景色）；滚动条见 §2.3 |
| `todo-list` | 无样式 |
| vuedraggable 根 | 普通 div（无类） |

待办行左边缘到窗口左边缘：描边 1 + 内边距 16 + 行内边距 12 = 29px 处是颜色圆点；标题文字起点 = 29 + 8 + 12 = 49px。

### 5.2 列表数据

- 列表 = `todoStore.pendingTodos`（未完成，按 `sortOrder` 升序；后端 `get_todos` 原始顺序为 `completed ASC, sort_order ASC, created_at DESC`，同 sortOrder 时保持该顺序）。**已完成的待办不在主窗口列表出现**（只能在"已完成"子窗口看）。
- 本地副本 `localPendingList`：store 变化时整体替换；**拖拽进行中不替换**（避免打乱 DOM），拖拽结束统一重建。

### 5.3 空状态

显示条件：`localPendingList.length === 0 && completedCount === 0`（没有任何待办，含已完成）。**若只有已完成待办，列表区完全空白，不显示空状态。**

| 元素 | 样式 | 文案 |
|---|---|---|
| `.empty-state` | flex 纵向，水平垂直居中，padding 24px，颜色 `--text-tertiary`，文字居中；深色下背景 `rgba(0,0,0,0.1)`（无圆角，占满 main-content 内宽） | |
| 图标 | `Document`，48×48，颜色 `var(--text-tertiary)` | |
| `p` | margin-top 8px，14px | `暂无待办事项` |
| `p.hint` | margin-top 4px，12px | `点击右下角悬浮按钮添加待办项` |

---

## 6. TodoItem（列表与四象限共用）

### 6.1 结构与尺寸

```
div.todo-item          flex 横向；align-items flex-start；position relative；padding 8px 12px；margin-bottom 4px；
│                      圆角 6px；背景透明；cursor pointer；transition background 150ms；user-select none
├─ div.color-dot       8×8 圆，背景 = todo.color；margin-top 6px；margin-right 12px；不收缩；cursor grab（按下 grabbing）
│                      ::before 四周各外扩 8px（命中区 24×24，不占布局）；它是拖拽手柄
├─ div.todo-content    flex 1；min-width 0
│  ├─ div.todo-title   字号 --todo-font-size（14）；字体 --todo-font-stack；颜色 --text-primary；word-break: break-word（多行换行，不截断）
│  ├─ div.todo-meta    [有子任务 / 有可解析的提醒时间 / 开启重复 之一才渲染]
│  │                   flex；align center；gap 8px；margin-top 4px；字号 --todo-sub-font-size（12）；颜色 --text-tertiary
│  │  ├─ span.subtask-count[.expanded]   [子任务数>0]
│  │  ├─ el-tooltip > span.repeat-badge  [repeatEnabled]
│  │  └─ span.notify-time                [notifyAt 可解析]
│  └─ div.subtask-list [展开 && 子任务数>0]（带过渡）
│     └─ div.subtask-item[.is-completed] × M
└─ div.todo-actions    默认 display none；.todo-item:hover 时 display flex（绝对定位浮层）
   ├─ button.complete-btn
   ├─ button.top-btn   [未完成]
   └─ button.delete-btn
```

单行高度（默认字号）：8 + 21（14×1.5）+ 8 = 37px，加 margin-bottom 4 → 行距 41px；有 meta 行再 + 4 + 18 = 59px（行距 63px）。颜色圆点垂直位置：margin-top 6 → 圆心约在标题首行中线（10px 处）。

### 6.2 状态样式

| 状态 | 浅色 | 深色（Windows/Linux） |
|---|---|---|
| 默认背景 | transparent | transparent（!important） |
| hover 背景 | `rgba(0,0,0,0.03)`，150ms 过渡；同时显示操作条 | **无背景变化**（`body.dark-theme #app .app-container .todo-item{background:transparent !important}` 压过了 `rgba(255,255,255,0.15)` 的 hover 规则）；仍显示操作条 |
| 拖拽占位（`.dragging`，原元素留在列表中的位置） | 背景 `rgba(59,130,246,0.1)` + `--shadow-md`；四象限内另加 `opacity:0.5` 且背景改为所在象限的 `--quadrant-bg`（!important） | 背景 transparent、阴影 none；四象限内 `opacity:0.5` |
| `.completed`（主窗口中不会出现） | 标题删除线；颜色规则 `--text-tertiary` 与后写的 `.todo-item .todo-content .todo-title { color: --text-primary }` 同优先级，后者胜 → 实际是 primary 色 + 删除线 | 同 |

### 6.3 元信息行（`.todo-meta`）

| 元素 | 样式 | 内容 / 行为 |
|---|---|---|
| `subtask-count` | flex，align center，gap 4px，cursor pointer，圆角 4px，padding 0 4px，**margin 0 −4px**（背景框向两侧外扩 4px，不占布局），`transition: color 150ms, background 150ms`；hover：颜色 `--text-secondary`、背景 `rgba(0,0,0,0.05)`（**深色下也是这个值**：深色覆盖规则 `rgba(255,255,255,0.1)` 优先级不够）；`.expanded` 时颜色 `--text-secondary` | `Finished` 图标 12px + 文本 `{已完成子任务数}/{子任务总数}` + 展开箭头 10px（展开 `ArrowDown`，收起 `ArrowRight`，直接换图标，不旋转）。点击：阻止冒泡，切换本行展开 |
| `repeat-badge` | flex，align center，颜色 **`#409eff`**（`--el-color-primary`），cursor default | `RefreshRight` 12px；`el-tooltip`（placement top，show-after 300ms）内容见 §6.5。点击冒泡 → 打开详情 |
| `notify-time` | flex，align center，gap 4px | `Bell` 12px + `formatDateTime(notifyAt, 'MM-DD HH:mm', 'notify')`，如 `10-15 09:00`（无年份；仅日期的 notifyAt 补 09:00；无法解析则整个元素不显示） |

**没有**的东西（任务描述中提到但源码不存在）：主列表**不显示开始时间 / 截止时间**，**没有过期 / 今天的变色**，**没有已提醒/未提醒区分**，**没有复选框**（完成走 hover 操作条的"完成"按钮），**没有优先级文字或色条**（只有左侧 8px 颜色圆点，颜色即 `todo.color`）。

### 6.4 子任务列表

层叠合并后的最终样式（全局 `.subtask-list` 与嵌套 `.todo-item .todo-content .subtask-list` 同时命中）：

| 元素 | 最终样式 |
|---|---|
| `.subtask-list` | margin-top 4px；**margin-left 20px**；padding-left 2px；**border-left 1px solid var(--border)**（浅 `#E2E8F0` / 深 `rgba(255,255,255,0.2)`）；overflow hidden |
| `.subtask-item` | flex，align center，gap 6px，padding 2px 0，字号 `--todo-sub-font-size`，字体 `--todo-font-stack`，颜色 `--text-secondary`（行高 18px → 每行 22px） |
| 状态图标 12px | 未完成 `CirclePlus`，颜色 `--text-tertiary`；已完成 `SuccessFilled`，颜色 `#10B981` |
| `.subtask-title` | flex 1；单行；`text-overflow: ellipsis`；min-width 0；已完成：删除线 + `--text-tertiary` |
| 标题 tooltip | `el-tooltip` placement top，show-after 500ms，内容 = 完整标题；标题为空或 `title.length < 20`（UTF-16 长度）时禁用 |

- 顺序：后端 `sort_order`（不把已完成沉底）。
- 子任务行点击会冒泡到整行 → 打开待办详情。
- 展开/收起过渡（Vue `<Transition name="subtask-list">`，首次渲染无动画）：进入/离开期间 `transition: all 0.2s ease; max-height: 300px`；起点/终点 `opacity: 0; max-height: 0`。即 200ms 内高度从 0 拉到 300px 上限（内容超过 300px 时动画期间被裁到 300、结束后跳到全高）、同时淡入；收起反向。
- 展开状态：每行本地 `subtaskExpanded`，初值 = 全局 `subtaskExpandAll`；全局开关变化时所有行同步为全局值；之后可单行点击覆盖。行组件重建（如列表↔四象限切换）会重置为全局值。

### 6.5 重复提醒 tooltip 文本（`repeatTooltip`）

`interval = repeatInterval || 1`；星期名 `['一','二','三','四','五','六','日']`（1=周一 … 7=周日）。

| repeatType | interval = 1 | interval > 1 |
|---|---|---|
| `daily` | `每天重复` | `每 ${n} 天重复` |
| `weekly` | `每周 周${days}`（无星期时只有 `每周`） | `每 ${n} 周 周${days}`（无星期时 `每 ${n} 周`） |
| `monthly` | `每月 ${day} 号` | `每 ${n} 月 ${day} 号` |
| 其它非空 | `重复提醒` | `重复提醒` |
| 为空 / 未开启 | `''` | |

`days` = `repeatWeekdays` 按逗号拆分、去空、映射为星期名（映射不到保留原值）、用 `、` 连接，如 `"1,3,5"` → `每周 周一、三、五`。`day = repeatMonthDay || 1`。

### 6.6 操作条（`.todo-actions`）

| 属性 | 浅色 | 深色 |
|---|---|---|
| 显示 | 仅 `.todo-item:hover` 时（无过渡，瞬显） | 同 |
| 定位 | absolute，right 8px，top 8px（相对待办行），z-index 5；**覆盖在标题右侧之上**（标题不为它预留空间） | 同 |
| 布局 | flex，align center，gap 4px，padding 2px 4px，圆角 4px | 同 |
| 背景 / 阴影 | `rgba(255,255,255,0.95)` / `0 1px 4px rgba(0,0,0,0.1)` | `rgba(30,30,30,0.9)` / `0 1px 4px rgba(0,0,0,0.3)` |
| 尺寸 | 3 个按钮时 88×28（3×24 + 2×4 + 2×4） | |

按钮 `.action-btn`：24×24，flex 居中，圆角 4px，颜色 `--text-secondary`，透明底，无边框，`transition: all 150ms`；hover 背景 `--bg-tertiary`，hover 颜色按按钮：

| 按钮 | 图标 16px | tooltip（原生 title） | hover 颜色 | 显示条件 | 点击 |
|---|---|---|---|---|---|
| 完成 | 未完成 `Select`；已完成 `RefreshLeft` | `完成` / `取消完成` | `#10B981` | 总是 | 阻止冒泡；`toggleComplete(id)` → `update_todo { id, data:{ completed: !completed } }`。成功后该项从列表消失（变为已完成），标题栏出现"已完成"按钮 |
| 置顶 | `Top` | `置顶` | `#F59E0B` | 未完成 | 阻止冒泡；`reorder_todos { ids: [当前id, ...其余未完成按现顺序] }` |
| 删除 | `Delete` | `删除` | `#EF4444` | 总是 | 阻止冒泡；弹确认框（§12.5）；确认后 `delete_todo { id }` |

### 6.7 交互汇总

- 单击行（含标题、元信息中除子任务计数外的部分、子任务行、颜色圆点在未拖动时）→ 打开待办详情子窗口（`#/editor?id={id}&mode=view`）。
- 拖拽只能从颜色圆点（24×24 命中区）开始。拖拽后紧随的 click 被 SortableJS 吞掉，不会打开详情。
- 操作条按钮、子任务计数点击都阻止冒泡。

---

## 7. 拖拽排序（vuedraggable 4.1.0 / SortableJS 1.14.0）

### 7.1 配置

| 选项 | 列表视图 | 四象限 |
|---|---|---|
| `handle` | `.color-dot` | `.color-dot` |
| `ghostClass` | `dragging` | `dragging` |
| `animation` | 200（ms，其它项让位时的平移动画） | 200 |
| `forceFallback` | true | true |
| `group` | 无 | `{ name:'quadrant-todos', pull:true, put:true }`（四个象限互通） |
| `item-key` | id | id |

SortableJS 默认值（未覆盖）：`fallbackTolerance 0`（按下后一移动就开始拖）；自动滚动 `scrollSensitivity 30px`、`scrollSpeed 10`、`bubbleScroll true`（拖到滚动容器边缘 30px 内自动滚动）。

### 7.2 视觉

- **浮动影子**（fallback ghost）：原元素克隆，追加到列表容器内，`position: fixed`，`opacity 0.8`，`z-index 100000`，`pointer-events none`，尺寸=原元素，跟随鼠标，变换原点为按下点；它不带 `.dragging`、不处于 hover，所以浅色下没有背景（只有圆点+文字浮在上面）。
- **占位**：原元素留在列表里随鼠标改变位置，带 `.dragging` 样式（§6.2）。其它项 200ms 平移让位。

### 7.3 落库

- **列表视图**：`@start` 置 `dragging=true`（期间忽略 store 刷新）；`@end` → `reorder_todos { ids: 本地列表新顺序 }`（store 内同时把各项 `sortOrder` 改为下标）；无论成败都按 store 重建本地列表（失败时回到拖拽前顺序，错误提示 `保存排序失败：…`）。
- **四象限**（`@change`）：
  - `added`（从别的象限拖入）：若 `todo.quadrant !== 目标象限` → 新颜色 `resolveQuadrantColor(todo.color, 旧象限, 新象限)`（当前颜色与旧象限默认色相同——忽略大小写、去空白比较——则改为新象限默认色，否则保留用户自选色）→ `update_todo { id, data:{ quadrant, color } }`（color 总会带上）；失败则不排序、按 store 重建（待办退回原象限）。
  - `moved`（象限内移动）或 `added` 成功后：`reorder_todos { ids: mergeQuadrantOrder(全局未完成 id 顺序, 本象限新 id 顺序) }`——全局顺序中属于本象限的"槽位"按本象限新顺序依次填入，其它象限的待办位置不变；本象限顺序中多出的 id 追加到末尾。两个列表都在第一个 await 之前取快照。
  - `removed`（源象限）忽略。
  - 拖拽中（`dragging`）与落库中（计数 `dragPersisting>0`）都不按 store 重建，全部结束后重建一次。

---

## 8. QuadrantView（四象限）

### 8.1 结构

```
div.quadrant-view[.dark-theme]   width 100%；height 100%；padding 8px
└─ div.quadrant-grid             grid 2 列 × 2 行（1fr 1fr / 1fr 1fr）；gap 8px；height 100%
   └─ div.quadrant-cell × 4      flex 纵向；圆角 8px；overflow hidden；1px 边框；transition all 0.2s ease
      │                          内联 CSS 变量 --quadrant-color / --quadrant-bg
      ├─ div.quadrant-header     flex；align center；gap 6px；padding 8px 10px；底边 1px rgba(128,128,128,0.1)；不收缩
      │  ├─ span.quadrant-indicator  8×8 圆，背景 = 象限色
      │  ├─ span.quadrant-title      12px / 500；flex 1
      │  └─ span.quadrant-count      数字；11px；padding 1px 6px；圆角 10px（胶囊）
      └─ div.quadrant-content    flex 1；overflow-y auto；overflow-x hidden；min-height 0
         ├─ div.quadrant-list    （vuedraggable 根）padding 4px；min-height 100%
         │  └─ TodoItem.quadrant-todo-item × N   margin-bottom 4px（最后一个 0）
         └─ div.quadrant-empty   [本象限 0 项] flex 居中；height 100%；min-height 60px；12px；--text-tertiary；"暂无待办"
```

### 8.2 象限定义（`types/todo.ts QUADRANT_INFO`）与位置

| id | 名称 | 位置 | 象限色 `--quadrant-color` | 底色 `--quadrant-bg` |
|---|---|---|---|---|
| 1 | `重要且紧急` | 左上 | `#EF4444` | `rgba(239, 68, 68, 0.1)` |
| 2 | `重要不紧急` | 右上 | `#F59E0B` | `rgba(245, 158, 11, 0.1)` |
| 3 | `紧急不重要` | 左下 | `#3B82F6` | `rgba(59, 130, 246, 0.1)` |
| 4 | `不紧急不重要` | 右下 | `#10B981` | `rgba(16, 185, 129, 0.1)` |

默认象限（新建）：4；默认颜色 `#10B981`。数据：`todosByQuadrant`（仅未完成，按 sortOrder），`quadrant` 值非法的待办归入第 1 象限。

### 8.3 样式（浅 / 深）

| 元素 | 浅色 | 深色 |
|---|---|---|
| 格子背景 | `--quadrant-bg`（象限色 10%） | `rgba(0,0,0,0.15)`（不再区分象限） |
| 格子边框 | `1px rgba(128,128,128,0.1)`；hover → 象限色（如 `#EF4444`），0.2s ease | `rgba(255,255,255,0.1)`；hover → `rgba(255,255,255,0.2)` |
| 头部背景 | `rgba(128,128,128,0.05)` | `rgba(0,0,0,0.1)` |
| 头部下边框 | `1px rgba(128,128,128,0.1)` | 同 |
| 标题颜色 | `--text-secondary #64748B` | `--text-primary #FFF` |
| 计数胶囊 | 背景 `rgba(128,128,128,0.1)`，颜色 `--text-tertiary` | 背景 `rgba(255,255,255,0.1)`，颜色 `--text-secondary` |
| 空状态文字 | `--text-tertiary` | 同 |
| 滚动条 | 宽 4px，轨道透明；滑块规则 `rgba(128,128,128,0.2)` / hover `0.3`、圆角 2px（见 §17 第 6 条：该滑块规则在编译产物中可能无效） | 同 |

TodoItem 在格子内的样式与列表完全一致（hover、操作条、子任务都相同），只是宽度窄：默认 380 宽窗口下每格约 161px 宽，待办标题可用宽约 109px。

### 8.4 观察到的实际效果（重要）

- **空象限看不到"暂无待办"**：`quadrant-list` 的 `min-height:100%` 已占满内容区，`quadrant-empty` 排在其后被挤到可视区外（需在格内滚动才看到）。`docs/images/grid.png` 中计数为 0 的象限确实没有文字，印证此点。设计意图显然是居中显示"暂无待办"。
- 非日历模式下 `.left-panel` 没有 `flex:1`，四象限的 `height:100%` 依赖其内容高度——按 CSS 规范推断，待办很少时四象限网格可能不会撑满窗口高度（未实测；日历分栏模式下 left-panel 被横向 flex 拉伸，确定撑满，截图如此）。
- 默认窗口（380×600，非日历）下若撑满：main-content 高 540，内容区 464，四象限 448（去 padding），每格约 161×220。

---

## 9. CalendarView（日历）

### 9.1 显示条件与窗口联动

- 只有 `appStore.showCalendar`（`get_show_calendar`，缺省 false）为 true 时渲染；开关只在设置窗口里（主窗口没有入口），经 `app-settings-changed {key:'showCalendar'}` 或设置窗口关闭时的 `reloadAppSettings()` 生效。
- 组件懒加载（`defineAsyncComponent`）。
- 开启后布局变为左右分栏：左 `left-panel`（宽 `--left-panel-width`，回落 40%，最小 280），右 `right-panel`（flex 1，padding 12px，深色 8px，透明）。
- **窗口尺寸联动**（MainView `watch(showCalendar)`，仅在挂载完成后的切换才生效；启动时已开启则不调整）：
  - 开启：`scale = scaleFactor()`，`logicalW/H = outerSize / scale`；记 `preCalendarWidth = logicalW`；设置 `--left-panel-width = logicalW px`；`gridH = logicalH − 44 − 30 − 24`；`cellH = gridH / 6`；`gridW = cellH × 7`；`rightPanelW = gridW + 24`；`setSize(LogicalSize(round(logicalW + rightPanelW), logicalH))`（使日格近似正方形；44/30 是标题栏/星期头的近似值，实际为 52/34）。
  - 关闭：移除 `--left-panel-width`；若有 `preCalendarWidth` → `setSize(LogicalSize(preCalendarWidth, logicalH))` 并清空；若启动时就开着日历（`preCalendarWidth` 为空），关闭后窗口**不缩回**。
  - 例：600 高 → gridH 502、cellH 83.67、gridW 585.67 → 新宽 380 + 609.67 ≈ 990。
- 标题栏中间区出现月份导航（§4.4），调用日历组件暴露的 `prevMonth()` / `nextMonth()` / `goToToday()`，读取 `currentMonthText`。
- FAB 是 `position: fixed` 的，开启日历后它**浮在日历右下角**之上。

### 9.2 结构

```
div.calendar-view[.dark-theme]   flex 纵向；height 100%；透明；圆角 8px；overflow hidden
├─ div.weekday-header            grid 7 等分列
│  └─ div.weekday-cell × 7       padding 8px 4px；居中；12px / 500；--text-secondary；高 34px
│                                文本：一 二 三 四 五 六 日（周一开头）
└─ div.calendar-grid             flex 1；grid 7 列 × 6 行（1fr）；position relative；overflow hidden；border-top 1px
   ├─ div.calendar-cell × 42     padding 3px 4px；min-height 60px；flex 纵向；右/下 1px 边框；position relative
   │  └─ div.cell-date-row       flex；align center；space-between；gap 2px；不收缩
   │     ├─ div.cell-date-area   flex；align center；gap 2px；不收缩
   │     │  ├─ span.cell-date    日期数字；13px / 500；line-height 1.4；--text-primary
   │     │  └─ span.badge-rest "休" | span.badge-work "班"  [节假日数据命中时]
   │     └─ div.cell-lunar       农历/节气/节日；10px；line-height 1.4；右对齐；单行省略；可收缩
   └─ div.todo-bar × K           绝对定位的跨天待办条（覆盖在格子之上）
      └─ span.bar-title          [仅本行段是开始段时] 待办标题
```

### 9.3 网格与格子样式

| 项 | 浅色 | 深色 |
|---|---|---|
| 网格顶边 | `1px solid var(--border)` = `#E2E8F0` | `rgba(255,255,255,0.6)` |
| 格子右/下边框 | `#E2E8F0` | `rgba(255,255,255,0.6)` |
| 每行第 7 格 | 无右边框 | 同 |
| 第 36–42 格（最后一行） | 无下边框 | 同 |
| 外框 | 无左边框、无外圈（只有内部网格线 + 顶线） | 同 |
| 非本月格（`.other-month`） | 日期数字、农历、班/休角标 `opacity 0.4` | 同 |
| 今天（`.is-today`，仅本月格） | 日期数字变 20×20 圆形：背景 `#3B82F6`，文字白色，inline-flex 居中 | 同 |
| 日期数字颜色 | `#1E293B` | `#FFFFFF` |
| 农历普通 | `--text-tertiary` `#94A3B8` | `rgba(255,255,255,0.65)` |
| 农历·传统节日（`.is-festival`） | `#EF4444`，500 | 同 |
| 农历·节气（`.is-solar-term`） | `#10B981`，500 | 同 |
| 星期头文字 | `#64748B` | `rgba(255,255,255,0.85)` |
| 角标"休" | 白字，底 `#EF4444` | 同 |
| 角标"班" | 白字，底 `#F59E0B` | 同 |
| 角标尺寸 | 9px / 600，line-height 1，padding 1px 2px，圆角 2px | 同 |

点击格子**没有任何行为**；只有待办条可点。

### 9.4 日期网格生成规则

- 周一为一周第一天：`firstDay = (getDay()==0 ? 6 : getDay()−1)`。
- 42 格固定 6 行：前补上月尾（`isCurrentMonth=false`），本月 1..N，后补下月头至 42 格。
- `isToday` 只对本月格判断（与响应式 `todayKey` 比较）。
- 每格 `dateStr` 为本地 `YYYY-MM-DD`；`row = floor(index/7)`，`col = index%7`。
- "今天"刷新：组件内定时器到下一个本地午夜 +1s 时刷新并重新排程；窗口获得焦点时也刷新（`onFocusChanged`）；点"今天"按钮时先刷新再跳转。
- 月份切换：`prevMonth`（1 月 → 去年 12 月）、`nextMonth`、`goToToday`（跳到当前年月）。年月变化触发节假日重新加载。

### 9.5 农历（`utils/lunar.ts`，lunar-javascript 1.7.7）

用到的 API：`Solar.fromYmd(y, m, d)`（字符串输入）、`Solar.fromDate(date)`、`solar.getLunar()`、`lunar.getDayInChinese()`、`lunar.getMonthInChinese()`、`lunar.getYearInGanZhi()`、`lunar.getDay()`、`lunar.getJieQi()`、`lunar.getFestivals()`、`lunar.toString()`（后两项及干支年算出但日历未使用）。

显示规则 `getLunarDisplayText(dateStr)`，优先级 **传统节日 > 节气 > 农历日**：
1. `getFestivals()[0]` 非空 → 显示节日，类型 `festival`（红）。lunar-javascript 的 `getFestivals` 只含：正月初一 `春节`、正月十五 `元宵节`、二月初二 `龙头节`、五月初五 `端午节`、七月初七 `七夕节`、八月十五 `中秋节`、九月初九 `重阳节`、腊月初八 `腊八节`，以及腊月最后一天（12 月且日 ≥29 且次日进入新农历年）`除夕`。键为 `月-日`，闰月（月份为负）不匹配节日。不含公历节日、不含"其他节日"（如中元节、寒食节）。
2. `getJieQi()` 非空（当天恰为 24 节气之一）→ 显示节气，类型 `solarTerm`（绿）：冬至 小寒 大寒 立春 雨水 惊蛰 春分 清明 谷雨 立夏 小满 芒种 夏至 小暑 大暑 立秋 处暑 白露 秋分 寒露 霜降 立冬 小雪 大雪。
3. 否则：农历初一显示月份 `getMonthInChinese() + '月'`（`正月 二月 … 十月 冬月 腊月`，闰月前缀 `闰`，如 `闰六月`）；其它日子显示 `getDayInChinese()`（`初一…初十 十一…十九 二十 廿一…廿九 三十`）。类型 `lunar`（灰）。

Rust 版需要等价的农历库（输出必须与上述文字逐字一致）。

### 9.6 节假日（`utils/holiday.ts` + 后端 `fetch_holidays`）

- 数据源：后端 `fetch_holidays { year }` 请求 `https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json`（reqwest，超时 10s / 连接 5s），成功写缓存 `<数据目录>/holidays/{year}.json`；失败回退本地缓存；都没有则 Err。返回 `[{ date:'YYYY-MM-DD', name, isOffDay }]`。
- 前端映射：`isOffDay=true` → `isHoliday`（显示 **休**）；在数据中但 `isOffDay=false` → 调休上班（显示 **班**）；不在数据中 → 无角标。周末本身不显示"休"。
- 前端重试：每年最多 3 次，间隔 `2000ms × attempt`（2s、4s）；成功且非空的结果进内存缓存（按年）。
- 日历加载：当前年；1 月时加载上一年，12 月时加载下一年（覆盖补齐格）。合并后若为空，延迟 `5000 × (n+1)` ms 重载，最多 3 次（5s、10s、15s）。用自增序号丢弃过期加载；卸载时作废。

### 9.7 待办条（跨天条）

数据：`props.todos = todoStore.todos`（**全部待办，含已完成**）。

- 开始日期：`startTime` 可解析（仅日期补 00:00）→ 其日期；否则 `createdAt` 的日期；都无法解析 → 不上日历。
- 截止日期：`endTime` 可解析（仅日期补 23:59）→ 其日期；否则等于开始日期（单日条）。
- 与 42 格日期范围无交集 → 跳过。开始早于首格 → 从第 0 格起；截止晚于末格 → 到第 41 格止。
- 按行拆段：起始行从 `startCell.col` 到行末（或 `endCell.col`），中间行 0..6，末行 0..`endCell.col`。`isStart` = 起始行且开始日期在范围内；`isEnd` 同理（`is-start` / `is-end` 类没有任何样式，所有段四角都是 4px 圆角）。
- 泳道（lane）分配（每行独立）：按 `startKey` 升序、再按 `endKey` 升序排序；依次为每段找与已分配段（列区间重叠：`!(a.end < b.start || b.end < a.start)`）不冲突的最小 lane。
- 位置（相对网格 padding box，网格高宽含 6 行 7 列）：
  - `left = calc(startCol × (100%/7) + 2px)`
  - `width = calc((endCol − startCol + 1) × (100%/7) − 4px)`
  - `top = calc(row × (100%/6) + (28 + lane × 22)px)`（BAR_TOP_OFFSET 28，BAR_HEIGHT 20，BAR_GAP 2）
  - height 20px，圆角 4px，padding 0 6px，flex 居中（纵向），背景 = `todo.color`，cursor pointer，z-index 10，overflow hidden。
- 标题 `bar-title`（只在 `isStart` 段显示）：11px，白色，单行省略，`text-shadow: 0 1px 2px rgba(0,0,0,0.2)`。
- hover 联动：鼠标进入任一段 → 同一待办的所有段 `.is-hovered`：`opacity 0.85`、`transform: scale(1.02)`、z-index 15；`transition: opacity 0.2s, transform 0.1s`。
- 已完成待办：`opacity 0.5`，标题删除线；hover 时 `0.65`。
- 点击条（阻止冒泡）→ `select-todo` → 打开该待办详情子窗口（带 parent，view 模式）。
- 泳道过多时条会溢出到下一行格子上（只在网格底边被裁），**没有"+N 更多"**。截止早于开始且跨行的待办不渲染；同行时宽度为负（不可见）。

---

## 10. FAB（悬浮新建按钮）与"操作模式"

| 属性 | 值 |
|---|---|
| 元素 | `button.fab-add`，原生 title `新建待办` |
| 定位 | `position: fixed; right: 20px; bottom: 20px`（相对窗口客户区；固定模式、日历模式都显示） |
| 尺寸 | 48×48，圆形 |
| 背景 / 前景 | `#3B82F6` / 白色；图标 `Plus` 24px |
| 阴影 | `0 4px 12px rgba(59,130,246,0.4)`（深色相同） |
| 过渡 | `all 200ms`（ease） |
| z-index | 100 |
| hover | 背景 `#2563EB`；`scale(1.05)`；阴影 `0 6px 16px rgba(59,130,246,0.5)` |
| active（按下） | `scale(0.95)` |
| 变暗 `.fab-dimmed` | `opacity 0.35`；z-index 4（沉到操作条 z5 之下）；变暗时 hover 本体 → `opacity 1`（仍可点） |
| 点击 | `openEditor()`（新建，无 id，编辑模式） |

"操作模式"变暗逻辑（根容器 `mouseover` / `mouseout` 委托，常量 `FAB_DIM_ENTER_DELAY = 750ms`，`FAB_DIM_LEAVE_DELAY = 1500ms`）：
- `mouseover` 目标**在** `.todo-actions` 内：若来源（relatedTarget）也在 `.todo-actions` 内则忽略；否则清除恢复计时；若未变暗且无进入计时 → 750ms 后 `fabDimmed = true`。
- `mouseover` 目标**不在** `.todo-actions` 内（包括悬停到 FAB 本身）：取消进入计时；若已变暗且无恢复计时 → 1500ms 后 `fabDimmed = false`。（兜底：删除/完成会让操作条连同行直接消失，不触发 mouseout。）
- `mouseout` 目标在 `.todo-actions` 内且去向不在 `.todo-actions` 内：取消进入计时，启动恢复计时（同上）。
- 卸载时清除两个计时器。

---

## 11. 模态遮罩与子窗口

主窗口用一个 `isModalOpen` 标志保证同一时刻只开一个子窗口（已打开时再请求直接忽略）。打开期间：
- 主窗口显示 `.modal-overlay`（满窗口 `rgba(0,0,0,0.3)`，z 999，cursor not-allowed）；在遮罩上 mousedown → `activeModalWindow.setFocus()`。
- 主窗口获得焦点 → 把焦点交给子窗口（不刷新）。
- 5s 变更轮询跳过；同步报告不刷新列表。
- 子窗口销毁（`tauri://destroyed`）→ 清标志 → `fetchTodos()`（设置窗口额外 `reloadAppSettings()` + `startAutoSync()`）；创建失败（`tauri://error`）→ 只清标志。

| 子窗口 | 触发 | label | url | title | 尺寸 | resizable | parent |
|---|---|---|---|---|---|---|---|
| 待办详情 | 点击待办 / 日历条 | `editor-{Date.now()}` | `#/editor?id={id}&mode=view` | `待办详情` | 1080×600 | 是 | 主窗口 |
| 新建待办 | FAB | 同上 | `#/editor` | `新建待办` | 1080×600 | 是 | 主窗口 |
| 新建待办（托盘） | `tray-add-todo` | 同上 | `#/editor` | `新建待办` | 1080×600 | 是 | **无**（`centerOnScreen=true` 只影响 parent） |
| 设置 | 齿轮 / `tray-open-settings` | `settings-{ts}` | `#/settings` | `设置` | 680×560 | 否 | 主窗口 |
| 已完成 | "已完成"按钮 | `completed-{ts}` | `#/completed` | `已完成` | 460×550 | 是 | 主窗口 |

位置：当前显示器（`currentMonitor() || primaryMonitor()`）逻辑坐标居中：`x = round(mon.x/scale + (mon.w/scale − W)/2)`，y 同理；取不到显示器时以主窗口为中心。所有子窗口 `decorations:false, transparent:false`。

---

## 12. Element Plus 组件与默认外观（GPUI 需手工还原）

主窗口实际用到的 EP 组件只有：`el-icon`、`el-button`（日历导航与"今天"）、`el-tooltip`（重复徽标、子任务长标题），以及函数式 `ElMessage`、`ElMessageBox`。**主窗口没有用到 el-checkbox、el-dropdown、el-popover、el-dialog**（`components.d.ts` 里的那些组件属于设置/编辑窗口）。

### 12.1 el-icon

`display:inline-flex; width/height: 1em; font-size: 由 :size 决定（未给时继承）; color: var(--color, inherit); svg fill currentColor`。图标 SVG 均为 `viewBox 0 0 1024 1024` 的单色填充路径（附录 A）。

### 12.2 el-button（EP 2.13.1）

| 变体 | 尺寸 | 其它 |
|---|---|---|
| 默认 | 高 32px，padding 8px 15px，14px，font-weight 500，1px 边框 `#dcdfe6`，底 `#ffffff`，字 `#606266`；hover：字 `#409eff`、底 `rgb(236,245,255)`、边框 `rgb(198,226,255)`；active：边框 `#409eff` | 相邻按钮 margin-left 12px；`transition .1s` |
| `size=small` | 高 24px，padding 5px 11px，12px | |
| `text` | 底透明、无边框；hover 底 `#f5f7fa`；active 底 `#f0f2f5` | |
| `type=primary`（确认框确认键） | 底/边框 `#409eff`，白字；hover `rgb(121,187,255)`；active `rgb(51,126,204)` | |

全局覆盖：圆角统一 6px（!important）；`> span` gap 6px。主窗口内 `.nav-btn` / `.today-btn` 的颜色全部被 !important 覆盖（§4.4）。

### 12.3 el-tooltip（默认 effect dark）

背景 `#303133`，1px 边框 `#303133`，文字 `#ffffff`，12px，line-height 20px，padding 5px 11px，圆角 4px，min-width 10px；带 10×10 旋转 45° 的方形箭头（向外凸 5px，与气泡同色）；距触发元素 offset 12px；`show-after` 按调用（300 / 500ms），`hide-after` 200ms；淡入淡出 `opacity 0.2s linear`；挂到 body，z-index 2000 起；位置不够时自动翻转。内容为空串时仍可能弹出一个空的小气泡（重复类型为空时）。

### 12.4 ElMessage（toast）

默认：顶部居中（`top` placement），距窗口顶 16px，多条时下一条 = 上一条底部 + 16px；`duration 3000ms`（鼠标悬停暂停计时），无关闭按钮；宽度自适应，`max-width: calc(100% − 32px)`；padding 11px 15px；gap 8px；1px 边框；圆角 4px；无阴影；内容 14px，`line-height: 1`；图标随 body 字号（14px）。进入/离开：`opacity` + `translateY(-100%)`（opacity 0.3s、transform/top 0.4s）。Esc 关闭。显示在**主窗口内部**（窗口很窄时文字会换行）。

| type | 背景 | 边框 | 文字/图标色 | 图标 |
|---|---|---|---|---|
| success | `rgb(240,249,235)` | `rgb(225,243,216)` | `#67c23a` | `SuccessFilled` |
| info | `rgb(244,244,245)` | `rgb(233,233,235)` | `#909399` | `InfoFilled` |
| warning | `rgb(253,246,236)` | `rgb(250,236,216)` | `#e6a23c` | `WarningFilled` |
| error | `rgb(254,240,240)` | `rgb(253,226,226)` | `#f56c6c` | `CircleCloseFilled` |

EP 自己的颜色不受应用深色主题影响（深色主题只改应用变量）。

### 12.5 ElMessageBox.confirm

遮罩：满窗口 `rgba(0,0,0,0.5)`，z-index 2000 起；淡入 `opacity 0.2s linear`；对话框 `msgbox-fade-in 0.3s`（opacity 0 + translateY(−20px) → 1）。对话框：白底，圆角 4px，阴影 `0 12px 32px 4px rgba(0,0,0,0.04), 0 8px 20px rgba(0,0,0,0.08)`，padding 12px，`width:100%; max-width:420px`（窗口 380 宽时 = 380 − 2×16 = 348px），垂直水平居中。
- 头部：标题 18px、line-height 24px、`#303133`，padding-bottom 12px；右上角关闭按钮 40×40 区域，`Close` 图标 16px `#909399`，hover `#409eff`。
- 内容：flex，gap 12px，align center；状态图标 24px（warning `WarningFilled #e6a23c`；info `InfoFilled #909399`）；消息 14px、line-height 24px、`#606266`。
- 按钮行：右对齐，padding-top 12px；取消（默认按钮）在左、确认（primary）在右，间距 12px，圆角 6px（全局覆盖）。
- 行为：点遮罩 / Esc / 关闭按钮 = 取消；打开时自动聚焦确认键；Enter 触发聚焦按钮。

主窗口内的两处确认框：

| 场景 | 标题 | 内容 | 确认 | 取消 | type |
|---|---|---|---|---|---|
| 删除待办 | `删除确认` | `确定要删除待办"${title}"吗？`（英文双引号） | `删除` | `取消` | warning |
| 版本更新（点版本号且有更新） | `版本更新` | `发现新版本 ${latestVersion}，是否前往下载？` | `前往下载` | `稍后再说` | info |

版本更新确认后 `openUrl(releaseUrl || 'https://github.com/dreamlonglll/mini-todo/releases')`（系统浏览器）。

---

## 13. 图标清单（@element-plus/icons-vue 2.3.2）

| 位置 | 图标 | 尺寸 |
|---|---|---|
| FAB | `Plus` | 24 |
| 展开/收起全部子任务 | `Expand` / `Fold` | 16 |
| 日历导航 | `ArrowLeft` / `ArrowRight` | 12（继承按钮字号） |
| 已完成 | `Finished` | 16 |
| 视图切换 | 自绘四宫格 SVG / `List` | 16 |
| 主题 | `Moon` / `Sunny` | 16 |
| 固定 | `Unlock` / `Lock` | 16 |
| 同步 | `Refresh` | 16 |
| 设置 | `Setting` | 16 |
| 空状态 | `Document` | 48 |
| 子任务计数 | `Finished` + `ArrowDown` / `ArrowRight` | 12 + 10 |
| 重复徽标 | `RefreshRight` | 12 |
| 提醒时间 | `Bell` | 12 |
| 子任务状态 | `SuccessFilled` / `CirclePlus` | 12 |
| 操作条 | `Select` / `RefreshLeft`、`Top`、`Delete` | 16 |
| ElMessage | `SuccessFilled` / `InfoFilled` / `WarningFilled` / `CircleCloseFilled` | 14 |
| ElMessageBox | `WarningFilled` / `InfoFilled`（24）、`Close`（16） | |

全部 SVG 路径数据见附录 A（可直接存成 `.svg` 资源供 GPUI `svg()` 使用，路径 `fill="currentColor"`，GPUI 中用 `text_color` 着色）。

---

## 14. 状态与逻辑

### 14.1 appStore（`stores/appStore.ts`）

模块级：`APP_VERSION = ref('')`（`getVersion()`）；`IS_WINDOWS = /windows/i.test(userAgent)`；`GITHUB_OWNER='dreamlonglll'`、`GITHUB_REPO='mini-todo'`。

| 状态 | 类型 | 默认 | 来源 / 说明 |
|---|---|---|---|
| `isFixed` | bool | false | `screen_configs.is_fixed`（按屏幕组合） |
| `isDarkTheme` | bool | false | `get_text_theme === 'light'` |
| `windowPosition` | `{x,y}` \| null | null | 物理像素 |
| `windowSize` | `{width,height}` \| null | null | 物理像素 |
| `currentScreenConfigId` | string | '' | 见下 |
| `screenConfigs` | ScreenConfig[] | [] | `list_screen_configs` |
| `showCalendar` | bool | false | `get_show_calendar` |
| `autoHideEnabled` | bool | true | `get_auto_hide_enabled` |
| `topOnWake` | bool | true | `get_top_on_wake` |
| `fixedEmbedDesktop` | bool | false | `get_fixed_embed_desktop` |
| `isEmbeddedInDesktop`（computed） | bool | | `IS_WINDOWS && isFixed && fixedEmbedDesktop` |
| `windowBgColor` | string | `#000000` | `get_window_background.color` |
| `windowBgAlpha` | number | 0.45 | `get_window_background.alpha` |
| `subtaskExpandAll` | bool | false | localStorage `subtask-expand-all` === `'1'`（仅前端，不进库不同步） |
| `todoFontFamily` | string | '' | `get_todo_font_family` |
| `todoFontSize` | number | 14 | `get_todo_font_size` |
| `hasUpdate` / `latestVersion` / `releaseUrl` | bool / string\|null / string\|null | false / null / null | GitHub 检查 |

屏幕组合 ID：`availableMonitors()` → 每个 `{width, height, scaleFactor: round(scale×100)}`（物理分辨率）→ 按像素数降序 → `${数量}_${W}x${H}@${缩放%}_...`，如 `2_2560x1440@125_1920x1080@100`；无显示器/异常 `unknown`。显示名：`unknown` → `未知配置`，`legacy` → `旧版配置`，否则 `${数量}屏: ${W}x${H} (${缩放}%) + ...`。

动作：

| 动作 | 逻辑 / 命令 |
|---|---|
| `initSettings()` | 平台类 → `loadAppVersion` → 生成屏幕 ID → `loadAutoHideEnabled` → `loadTopOnWake` → `loadFixedEmbedDesktop` → `loadWindowBackground` → `loadDarkTheme` → `get_screen_config {configId}` 恢复或居中（§1.1）→ `loadScreenConfigs` → `loadShowCalendar` → `loadTodoFontSettings`。整体 try/catch 只打日志 |
| `loadShowCalendar` / `loadAutoHideEnabled` / `loadTopOnWake` / `loadFixedEmbedDesktop` | 对应 `get_*`，失败回落默认 |
| `setShowCalendar(show)` 等 `set*`（设置窗口用） | 先改状态再 `invoke`，失败 `notifyError` 并回滚：`设置日历显示失败` / `设置贴边自动隐藏失败` / `设置唤起置顶失败` / `设置嵌入桌面失败` / `设置界面底色失败` |
| `loadTodoFontSettings()` | §1.6 |
| `toggleFixedMode()` | 翻转 `isFixed` → `applyFixedMode()` 或 `applyNormalMode()` → `saveWindowState()`；异常 `切换固定模式失败` |
| `applyFixedMode()` | `setResizable(false)`；`IS_WINDOWS && fixedEmbedDesktop` ? `set_window_desktop_mode {enabled:true}` : `set_window_fixed_mode {fixed:true}`；异常 `进入固定模式失败` |
| `applyNormalMode()` | `setResizable(true)`；`set_window_fixed_mode {fixed:false}`；`set_window_desktop_mode {enabled:false}`（两者幂等）；异常 `退出固定模式失败` |
| `loadDarkTheme()` | `get_text_theme` → `isDarkTheme` → `applyThemeClass()`（仅 label `main`：body 加/去 `dark-theme`） |
| `setDarkTheme(enabled)` / `toggleDarkTheme()` | 先改状态并应用类 → `set_text_theme { theme: enabled ? 'light' : 'dark' }`；失败 `设置深色主题失败` 并回滚 |
| `loadWindowBackground()` / `setWindowBackground(color, alpha)` | `get_window_background` / `set_window_background {color, alpha}` → `applyWindowBackground()`（仅 `main`） |
| `saveWindowState()` | `get_window_persist_state → {position:{x,y}, size:{width,height}}`（后端固定读 label `main`；贴边隐藏中返回锚点位置/尺寸）→ `save_screen_config { config:{ configId, displayName, windowX, windowY, windowWidth: round, windowHeight: round, isFixed } }` → `save_settings { settings:{ isFixed, fixedEmbedDesktop, windowPosition, windowSize, autoHideEnabled, topOnWake, windowBgColor, windowBgAlpha, textTheme: isDark?'light':'dark' } }`；异常只打日志 |
| `loadScreenConfigs()` / `deleteScreenConfig(id)` | `list_screen_configs` / `delete_screen_config {configId}` |
| `checkForUpdates()` | `GET https://api.github.com/repos/dreamlonglll/mini-todo/releases/latest`（`Accept: application/vnd.github.v3+json`）；`compareVersions(tag_name, APP_VERSION) > 0`（去 `v`、按 `.` 数值逐段比较）→ `hasUpdate=true`、`latestVersion=tag_name`、`releaseUrl=html_url`；失败静默 |
| `getReleasesUrl()` | `releaseUrl || https://github.com/dreamlonglll/mini-todo/releases` |
| `toggleSubtaskExpandAll()` | 翻转并写 localStorage `'1'/'0'` |

### 14.2 todoStore（`stores/todoStore.ts`）

| 状态 / 计算 | 说明 |
|---|---|
| `todos: Todo[]` | 全部待办（含子任务） |
| `viewMode: 'list' \| 'quadrant'` | 默认 `'list'` |
| `pendingTodos` | `!completed`，按 `sortOrder` 升序 |
| `completedTodos`（内部） | `completed`，按 `sortOrder` 降序 |
| `todoCount` | `{ total, pending, completed }` |
| `todosByQuadrant` | `{1:[],2:[],3:[],4:[]}`，仅 pending；非法象限归 1 |

| 动作 | 逻辑 / 命令 |
|---|---|
| `fetchTodos(): Promise<bool>` | **单飞**：在途时新请求只置 `fetchQueued`，结束后补拉一次，所有调用方等最终结果。每轮：先 `get_change_seq` 再 `get_todos`；若期间发生本地写操作（`mutationSeq` 变化）则丢弃结果并补拉；成功记录 `loadedChangeSeq`。连续失败只在第一次提示 `加载待办失败：…`，成功后复位 |
| `refreshIfChanged()` | 在途则跳过；`get_change_seq` 为 null 或等于 `loadedChangeSeq` 则跳过；否则 `fetchTodos()` |
| `updateTodo(id, data)` | `mutationSeq++`；`update_todo {id, data}` → 用返回值替换本地项；失败 `更新待办失败：…` |
| `deleteTodo(id)` | `mutationSeq++`；`delete_todo {id}` → 本地移除；失败 `删除待办失败：…` |
| `toggleComplete(id)` | `updateTodo(id, {completed: !completed})` |
| `reorderTodos(ids)` | `mutationSeq++`；`reorder_todos {ids}` → 本地 `sortOrder = 下标`；失败 `保存排序失败：…` |
| `updateTodoQuadrant(id, q, color?)` | `updateTodo(id, color ? {quadrant:q, color} : {quadrant:q})` |
| `setViewMode(m)` / `loadViewMode()` / `saveViewMode()` | 内存 / `get_setting {key:'view_mode'}`（只接受 `list`/`quadrant`）/ `set_setting {key:'view_mode', value}`（失败只打日志） |

`get_change_seq` 读取失败只在第一次 `console.warn`，之后返回 null（轮询失效但不报错）。

### 14.3 MainView 启动时序（`onMounted`）

1. `await appStore.initSettings()`
2. `await todoStore.fetchTodos()`
3. `await todoStore.loadViewMode()`
4. `await nextTick()`；`calendarResizeReady = true`（此后日历开关变化才调整窗口宽度）
5. `appStore.checkForUpdates()`（不 await）
6. 注册窗口与事件监听（§14.4）
7. `startAutoSync()`
8. `setInterval(5000)`：`!isModalOpen` 时 `todoStore.refreshIfChanged()`
9. `set_auto_hide_cursor_inside {inside:true}`

卸载：注销全部监听；作废进行中的 `startAutoSync`（代次+1）并停表；清 FAB 计时器、轮询、防抖；`set_auto_hide_cursor_inside {inside:false}`。

### 14.4 监听的事件

| 来源 | 事件 | payload | 处理 |
|---|---|---|---|
| 窗口 | `onCloseRequested` | — | `saveWindowState()`（不阻止关闭） |
| 窗口 | `onMoved` | — | 防抖 500ms → `saveWindowState()` |
| 窗口 | `onResized` | — | 防抖 500ms（与移动共用一个计时器）→ `saveWindowState()` |
| 窗口 | `onFocusChanged` | bool | 获得焦点：有子窗口 → 聚焦子窗口；否则 `fetchTodos()`。（日历组件另有监听：获得焦点刷新"今天"） |
| 后端 | `tray-toggle-fixed` | — | `toggleFixedMode()` |
| 后端 | `tray-reset-window` | — | 后端已把窗口挪到主显示器左上（10% 边距）、尺寸 380×600、可缩放。前端：若固定 → `toggleFixedMode()`（退回普通并保存）；否则清防抖并立即 `saveWindowState()`；最后 `initSettings()` |
| 后端 | `tray-add-todo` | — | `openEditor(undefined, true)` |
| 后端 | `tray-open-settings` | — | `openSettings()`（后端已 show + focus 主窗口） |
| 设置窗口 | `data-imported` | — | `fetchTodos()` → `reloadAppSettings()` → `ElMessage.success('数据导入成功')` |
| 设置窗口 | `todo-font-changed` | — | `loadTodoFontSettings()` |
| 设置窗口 | `app-settings-changed` | `{ key }` | `showCalendar` → `loadShowCalendar`；`autoHide` → `loadAutoHideEnabled`；`topOnWake` → `loadTopOnWake`；`fixedEmbedDesktop` → `loadFixedEmbedDesktop`（只同步状态/`body.fixed-mode`，窗口切换后端已做）；`windowBackground` → `loadWindowBackground`；`theme` → `loadDarkTheme`；`sync` → `startAutoSync()`；`update` → `checkForUpdates()` |
| 后端 | `sync-completed` | `SyncReport \| null` | 有报告 → `applySyncReport(report)`；null → `fetchTodos()` |
| 子窗口 | `tauri://destroyed` / `tauri://error` | — | §11 |

`reloadAppSettings()` 顺序：`loadViewMode` → `loadShowCalendar` → `loadAutoHideEnabled` → `loadFixedEmbedDesktop` → `loadTopOnWake` → `loadWindowBackground` → `loadDarkTheme` → `loadTodoFontSettings`。

### 14.5 主窗口用到的全部 invoke 命令

| 命令 | 参数 | 返回 | 调用方 |
|---|---|---|---|
| `get_screen_config` | `{ configId }` | `ScreenConfig \| null` | initSettings |
| `save_screen_config` | `{ config: SaveScreenConfigRequest }` | — | saveWindowState |
| `list_screen_configs` | — | `ScreenConfig[]` | initSettings |
| `get_window_persist_state` | — | `{ position:{x,y}, size:{width,height} }` | saveWindowState |
| `save_settings` | `{ settings: AppSettings }` | — | saveWindowState |
| `get_auto_hide_enabled` / `get_top_on_wake` / `get_fixed_embed_desktop` / `get_show_calendar` | — | bool | load* |
| `get_window_background` | — | `{ color, alpha }` | loadWindowBackground |
| `get_text_theme` / `set_text_theme` | — / `{ theme }` | string / — | 主题 |
| `get_todo_font_family` / `get_todo_font_size` | — | string / i32 | 字体 |
| `set_window_fixed_mode` | `{ fixed }` | — | 固定/普通 |
| `set_window_desktop_mode` | `{ enabled }` | — | 嵌入/普通 |
| `set_auto_hide_cursor_inside` | `{ inside }` | — | 根 mouseenter/leave、挂载/卸载（错误忽略） |
| `get_change_seq` | — | i64 | 轮询 |
| `get_todos` | — | `Todo[]` | fetchTodos |
| `update_todo` | `{ id, data: UpdateTodoRequest }` | `Todo` | 完成、改象限 |
| `delete_todo` | `{ id }` | — | 删除 |
| `reorder_todos` | `{ ids: number[] }` | — | 拖拽、置顶 |
| `get_setting` / `set_setting` | `{ key:'view_mode' }` / `{ key:'view_mode', value }` | `string \| null` / — | 视图模式 |
| `get_sync_settings` | — | `SyncSettings` | 手动同步前检查、装自动同步定时器 |
| `webdav_sync` | 手动 `{ full: true }`；自动 无参数 | `SyncReport` | 同步 |
| `fetch_holidays` | `{ year }` | `[{date, name, isOffDay}]` | 日历 |
| Tauri API | `getVersion`、`startDragging`、`setPosition/Size`、`setResizable`、`scaleFactor`、`outerSize/Position`、`currentMonitor/primaryMonitor/availableMonitors`、`WebviewWindow`、`openUrl`（opener 插件） | | |

### 14.6 同步

**手动同步**（标题栏按钮 `handleSync`）：
1. `isSyncing` 已为 true → 直接返回。
2. `isSyncing = true`；`get_sync_settings`；`webdavUrl` 为空 → `ElMessage.warning('请先在设置中配置 WebDAV 服务器')` 并结束。
3. `webdav_sync { full: true }`（总是无条件 GET）→ `autoSyncFailing = false` → `applySyncReport(report)`。
4. `ElMessage({ type: status==='no_changes' ? 'info' : 'success', message: describeSyncReport(report) })`；`describeSkippedRecords(report)` 非空再 `ElMessage.warning(...)`。
5. 异常：`isSyncBusyError(e)`（错误文本含 `同步正在进行中`）→ `ElMessage.info('已有同步正在进行，请稍后再试')`；否则 `notifyError(e, '同步失败')`。
6. finally `isSyncing = false`。

**自动同步**：
- `startAutoSync()`：代次 +1 → `get_sync_settings`（失败只打日志）→ 若代次已过期则放弃 → 停旧定时器 → 若 `autoSync && webdavUrl`：`setInterval(max(1, syncInterval || 15) 分钟)` 调 `runAutoSync()`。调用时机：挂载、设置窗口关闭、`app-settings-changed {key:'sync'}`。
- `runAutoSync()`：`isSyncing` 为 true 跳过本轮；置 true → `webdav_sync`（无参）→ 成功 `autoSyncFailing=false`、`applySyncReport`、跳过记录只 `console.warn`；失败：忙碌错误不算失败；否则若尚未处于失败段 → `autoSyncFailing=true` 并 `ElMessage.error(detail ? '自动同步失败：' + detail : '自动同步失败')`（**同一段连续失败只提示一次**，成功或手动同步成功后复位）→ finally 置 false。

**`applySyncReport(report)`**：以 `JSON.stringify(report)` 去重（手动同步返回值与 `sync-completed` 事件是同一份报告，只处理一次）；`hasLocalDataChanges(report) && !isModalOpen` → `fetchTodos()`；`report.settingsApplied` → `reloadAppSettings()`。

`syncReport.ts` 原文（GPUI 版需逐字复刻文案）：

```ts
function count(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : 0
}

export function hasLocalDataChanges(report: SyncReport): boolean {
  return (
    count(report.todosInserted) + count(report.todosUpdated) + count(report.todosDeleted) +
    count(report.subtasksInserted) + count(report.subtasksUpdated) + count(report.subtasksDeleted) > 0
  )
}

const STATUS_TEXT: Record<SyncReport['status'], string> = {
  no_changes: '已是最新，没有需要同步的更改',
  pulled: '已从云端拉取更新',
  pushed: '已将本地更改推送到云端',
  merged: '已合并本地与云端的更改',
}

export function describeSyncReport(report: SyncReport): string {
  const base = STATUS_TEXT[report.status] ?? '同步完成'
  const counters: Array<[unknown, string]> = [
    [report.todosInserted, '新增待办'],
    [report.todosUpdated, '更新待办'],
    [report.todosDeleted, '删除待办'],
    [report.subtasksInserted, '新增子任务'],
    [report.subtasksUpdated, '更新子任务'],
    [report.subtasksDeleted, '删除子任务'],
    [report.imagesDownloaded, '下载图片'],
    [report.imagesUploaded, '上传图片'],
  ]
  const parts = counters.filter(([n]) => count(n) > 0).map(([n, label]) => `${label} ${count(n)}`)
  if (report.settingsApplied) parts.push('已应用云端设置')
  return parts.length > 0 ? `${base}（${parts.join('，')}）` : base
}

export function describeSkippedRecords(report: SyncReport): string | null {
  const skipped = count(report.recordsSkipped)
  return skipped > 0 ? `有 ${skipped} 条云端记录无法识别，已跳过（未删除任何本地数据）` : null
}

export function isSyncBusyError(e: unknown): boolean {
  return String(e).includes('同步正在进行中')
}
```

示例输出：`已合并本地与云端的更改（新增待办 2，更新子任务 1，已应用云端设置）`（全角括号、全角逗号分隔、标签与数字之间一个半角空格）。

`SyncReport` 字段：`status ('no_changes'|'pulled'|'pushed'|'merged')`、`lastSyncAt`、`todosInserted/Updated/Deleted`、`subtasksInserted/Updated/Deleted`、`recordsSkipped`、`imagesUploaded`、`imagesDownloaded`、`settingsApplied`。

### 14.7 错误提示（`notify.ts`）

```ts
const MAX_DETAIL_LENGTH = 200
errorMessage(e): 字符串原样 / Error.message / 带 string message 的对象 / String(e ?? '')；trim；超过 200 字截断并加 '…'
notifyError(e, msg): console.error(msg, e); ElMessage.error(detail ? `${msg}：${detail}` : msg)
```

即错误 toast 格式 `操作描述：错误详情`（全角冒号）。

### 14.8 贴边自动隐藏的光标上报

- 根容器 `mouseenter` → `set_auto_hide_cursor_inside {inside:true}`；`mouseleave` → `{inside:false}`；挂载时 true、卸载时 false；错误忽略。
- 后端 `evaluate_auto_hide_transition` 优先用系统光标坐标判断；拿不到光标坐标时才用这个上报值（注释说明主要为 macOS 唤起）。贴边隐藏本身（10px 吸附阈值、420ms 隐藏延迟、隐藏后留 4 物理像素、唤起热区等）全部在后端 `commands/window.rs`，前端只负责上报与 `main-content` 底部 6px 留白。

### 14.9 窗口状态保存

- 移动/缩放 → 500ms 防抖 → `saveWindowState()`；关闭请求 → 立即保存；固定切换 → 保存。
- 保存的是**物理像素**位置与尺寸，按"屏幕组合 ID"分别记忆（`screen_configs`），并同时写 settings。

---

## 15. 文案清单（逐字）

**标题栏**：`待办清单`、`v{版本号}`（如 `v2.5.0`）、`今天`、`{年}年{月}月`（如 `2026年10月`）

**原生 tooltip（title 属性）**：
- `收起全部子任务` / `展开全部子任务`
- `已完成`
- `切换到四象限视图` / `切换到列表视图`
- `切换到深色主题` / `切换到浅色主题`
- `固定窗口` / `取消固定`
- `同步数据` / `同步中...`
- `设置`
- `新建待办`（FAB）
- `完成` / `取消完成`、`置顶`、`删除`（待办操作条）

**el-tooltip**：重复提醒（`每天重复`、`每 {n} 天重复`、`每周`、`每周 周{一、三…}`、`每 {n} 周`、`每 {n} 周 周{…}`、`每月 {d} 号`、`每 {n} 月 {d} 号`、`重复提醒`）；子任务完整标题

**空状态**：`暂无待办事项`、`点击右下角悬浮按钮添加待办项`；四象限 `暂无待办`

**四象限标题**：`重要且紧急`、`重要不紧急`、`紧急不重要`、`不紧急不重要`

**日历**：星期 `一 二 三 四 五 六 日`；角标 `休`、`班`；农历文字见 §9.5

**确认框**：
- `删除确认` / `确定要删除待办"{标题}"吗？` / `删除` / `取消`
- `版本更新` / `发现新版本 {版本}，是否前往下载？` / `前往下载` / `稍后再说`

**ElMessage**：
- success：`数据导入成功`；同步报告（非 no_changes）
- info：同步报告（no_changes：`已是最新，没有需要同步的更改`）；`已有同步正在进行，请稍后再试`
- warning：`请先在设置中配置 WebDAV 服务器`；`有 {n} 条云端记录无法识别，已跳过（未删除任何本地数据）`
- error：`同步失败：{详情}`、`自动同步失败：{详情}` / `自动同步失败`、`加载待办失败：{详情}`、`更新待办失败：{详情}`、`删除待办失败：{详情}`、`保存排序失败：{详情}`、`切换固定模式失败：{详情}`、`进入固定模式失败：{详情}`、`退出固定模式失败：{详情}`、`设置深色主题失败：{详情}`（以及设置窗口触发的 `设置日历显示失败`、`设置贴边自动隐藏失败`、`设置唤起置顶失败`、`设置嵌入桌面失败`、`设置界面底色失败`）

**同步报告片段**：`已是最新，没有需要同步的更改`、`已从云端拉取更新`、`已将本地更改推送到云端`、`已合并本地与云端的更改`、`同步完成`、`新增待办`、`更新待办`、`删除待办`、`新增子任务`、`更新子任务`、`删除子任务`、`下载图片`、`上传图片`、`已应用云端设置`

**子窗口标题**：`待办详情`、`新建待办`、`设置`、`已完成`；主窗口 `Mini Todo`

**屏幕配置显示名**：`未知配置`、`旧版配置`、`{n}屏: {W}x{H} ({s}%) + …`

**托盘（后端，主窗口响应其事件）**：`添加待办项`、`新增收纳箱`、`显示/隐藏所有收纳箱`、`重置收纳箱位置`、`固定模式`、`重置位置`、`打开设置`、`开机自启动`、`退出`

---

## 16. GPUI 还原难点与近似做法

| CSS 效果 | 出处 | 难点 | 建议近似 |
|---|---|---|---|
| 透明窗口 + 自绘圆角矩形 + 半透明底色 | #app / .app-container | 需逐像素 alpha | `WindowBackgroundAppearance::Transparent`（gpui-pre 0.3.9 有此枚举）+ 根 div `rounded(8px)`、`bg(rgba)`、`border_1`；Windows 再调 `DwmSetWindowAttribute(DWMWCP_ROUND)`；固定模式 radius 0、无边框 |
| backdrop-filter / 毛玻璃 | — | **主窗口未使用**（`docs/开发文档/04-UI设计规范.md` 写的"毛玻璃"已过时） | 无需实现 |
| `!important` 层叠导致的"实际效果" | 深色 hover 无背景、深色子任务计数 hover 用浅色值等 | GPUI 没有层叠 | 按本文"最终值"直接写死，不要按 SCSS 字面重推 |
| 主题 CSS 变量 | 全局 | — | 建 `Theme { light, dark }` 结构，字段对应 §2 表；底色由 `window_bg_color/alpha` 计算 `Hsla` |
| 彩色外发光阴影 | FAB `0 4px 12px rgba(59,130,246,0.4)`、操作条阴影、确认框阴影 | | GPUI `BoxShadow { color, offset, blur_radius, spread_radius }`，可多层 |
| `transform: scale()` | FAB hover 1.05 / active 0.95；日历条 hover 1.02 | GPUI 普通 div 没有布局变换 | 按中心点手动改尺寸/偏移（如 FAB 48→50.4，位置各减 1.2），或用 `with_animation` 插值；日历条可只做 opacity |
| CSS transition（150/200ms 颜色/背景/边框） | 按钮、行、象限格 | GPUI 无隐式过渡 | 可直接瞬变；要还原则用 `with_animation` + hover 状态记录 |
| `@keyframes spin` | 同步图标 | | `svg().with_transformation(Transformation::rotate(...))` + `with_animation(Animation::new(1s).repeat())` |
| `@keyframes pulse`（box-shadow 光环） | 更新红点 | | 叠一个绝对定位圆，`with_animation` 2s 循环：0→70% 半径 3→7px、alpha 0.7→0，70→100% 保持透明 |
| `max-height` 展开/收起 + 淡入 | 子任务列表 | | 记录动画进度，外层 `h(px(progress × min(内容高, 300)))` + `overflow_hidden` + `opacity(progress)`；或直接瞬切 |
| `::before` 扩大命中区 | 颜色圆点 8px → 24px | | 外包 24×24 透明容器作手柄并用负 margin（−8px）抵消，使布局仍按 8px 计算 |
| `display:none` → hover 显示 | 操作条 | | `group("todo-item")` + 子元素 `invisible()` / `group_hover` 显示，或在行上 `on_hover` 记录 hover id |
| 委托式 mouseover/mouseout + 计时器 | FAB 变暗 | | 操作条 `on_hover(bool)` 回调 + `cx.spawn` 定时器（750ms / 1500ms），任何其它元素 hover 时启动恢复计时 |
| z-index 交换（FAB 4 ↔ 100） | FAB 变暗时沉到操作条之下 | GPUI 按绘制顺序叠放 | FAB 正常时用 `deferred()` 最后绘制；变暗时改为在列表之前绘制（或给操作条更高 `deferred` priority） |
| `position: fixed` 遮罩 / FAB | 模态遮罩、FAB | | 根容器 `relative`，子元素 `absolute` 铺满 / 右下 20px；遮罩 `occlude()` 吞掉鼠标并 mousedown 聚焦子窗口 |
| 原生 `title` tooltip | 标题栏按钮、操作条、FAB | | GPUI `.tooltip()`（默认延迟可用 `tooltip_show_delay` 调整） |
| el-tooltip 深色气泡 + 箭头 | 重复徽标、子任务标题 | 箭头 | gpui-component Tooltip 或自绘：`#303133` 底、白字 12px、padding 5×11、圆角 4；箭头可用旋转 45° 的小方块 svg 或省略 |
| ElMessage 堆叠 toast | 全局 | | gpui-component Notification 或自绘顶部居中堆栈（offset 16、间距 16、3s 自动消失、悬停暂停），配色按 §12.4 |
| ElMessageBox 模态确认 | 删除、版本更新 | | gpui-component Dialog/AlertDialog 或自绘：`rgba(0,0,0,0.5)` 遮罩 + 白卡片（§12.5） |
| SortableJS 拖拽（fallback 影子 0.8 透明、占位、200ms 让位动画、跨容器、边缘自动滚动、拖后吞 click） | 列表、四象限 | GPUI `on_drag` 只提供拖拽预览 + `on_drop`，没有自动让位/排序 | `on_drag(DraggedTodo)` 渲染 0.8 透明影子；每行 `drag_over` 计算插入位置并实时重排本地列表（占位行 `.dragging` 样式）；四个象限各自 `on_drop`；靠近滚动容器边缘 30px 时手动滚动；拖拽结束后忽略下一次 click |
| 自定义滚动条（6px / 4px） | main-content、象限 | | gpui-component Scrollbar 或 `overflow_y_scroll()` + 自绘滑块；颜色 §2.3 |
| CSS Grid 2×2、7×6、`1fr` | 四象限、日历 | | gpui-pre 0.3.9 有 `grid()` + `grid_cols(n)` / `grid_rows(n)`；或 flex + `flex_1` |
| 百分比 + px 的 `calc()` 绝对定位 | 日历待办条 | | 用 `left(relative(col/7))` 等分数长度 + 内层 `mx(2px)`；或 `canvas`/布局回调拿网格 bounds 后按像素计算 |
| `text-overflow: ellipsis` / 换行 | 子任务标题、农历、日历条 / 待办标题 | | `truncate()` / `text_ellipsis()`；标题默认换行 |
| `text-decoration: line-through` | 已完成子任务、已完成日历条 | | `line_through()`（StrikethroughStyle） |
| `text-shadow` | 日历条标题 | GPUI 无文字阴影 | 先画一层 1px 下移的 `rgba(0,0,0,0.2)` 文字再画白字，或省略 |
| 元素组透明度 | 非本月格 0.4、已完成条 0.5、FAB 0.35、四象限占位 0.5 | | `opacity()` |
| `align-items: baseline` | 标题 + 版本号 | | GPUI flex 无 baseline 对齐 → 用 `items_end` + 微调 padding-bottom（16px 与 11px 字号基线差约 3–4px） |
| `:has()` 选择器 | 日历开启时 title-left 定宽 40% | | 直接按 `show_calendar` 条件设宽度 |
| 字体回退（Segoe UI + Microsoft YaHei）/ 用户字体 | 全局 / 待办 | | GPUI `font_family` + `FontFallbacks`；待办标题用用户字体 |
| 窗口拖动 / 双击最大化 | 标题栏 | | `window.start_window_move()`（mousedown，固定模式禁用）；双击最大化可选 |
| 光标形状 | grab / grabbing / pointer / not-allowed / default | | GPUI `CursorStyle::OpenHand` / `ClosedHand` / `PointingHand` / `OperationNotAllowed` / `Arrow`（gpui-pre 0.3.9 `platform.rs` 中存在这些枚举值） |
| 子窗口模态 | 编辑/设置/已完成 | GPUI 多窗口无原生 owner/modal | 主窗口自维护 `modal_open` + 遮罩 + 聚焦转交；Windows 可用 owner 窗口 |

---

## 17. Caveats / Not Found（实现者需要知道的"源码真实行为"）

1. **主列表不显示开始/截止时间，没有过期/今天变色，没有复选框，没有优先级文字**；只显示提醒时间（`MM-DD HH:mm`）、子任务计数、重复徽标。开始/截止时间只用于日历条。
2. **已完成待办不在主窗口列表/四象限出现**；`TodoItem` 的"已完成"外观（`RefreshLeft`、`取消完成`、无置顶按钮、删除线）在主窗口中不会出现，仅 CompletedView 复用时可见（该窗口不在本文范围）。
3. 深色主题（Windows/Linux）下待办行 hover **没有**背景变化，拖拽占位也没有背景/阴影——源于 `!important` 层叠；子任务计数 hover 背景在深色下仍是 `rgba(0,0,0,0.05)`（深色覆盖规则优先级不足）。
4. 空象限的"暂无待办"被挤出可视区（截图印证）。
5. 非日历模式下 `.left-panel` 无 `flex:1`，四象限可能不撑满高度（按 CSS 推断，未实测；日历分栏模式确定撑满）。
6. QuadrantView 的 `.quadrant-content::-webkit-scrollbar-thumb` 规则写在原生 CSS 嵌套里，编译产物为 `&::-webkit-scrollbar-thumb { &[data-v-…] {…} }`，按 CSS Nesting 规范 `&` 不能代表伪元素，该规则大概率无效 → 象限内滚动条滑块可能不可见（宽 4px 的轨道样式有效）。未实测。
7. 打开日历时窗口加宽的公式用的是近似值（标题栏 44、星期头 30），实际为 52 / 34；启动时就开着日历则不调整宽度、`--left-panel-width` 未设置（左栏回落 40%），之后关闭日历窗口不会缩回。
8. 直接双击标题栏根元素空白处会触发 Tauri 的最大化切换（窗口可缩放时）。
9. 主窗口没有关闭/最小化按钮。
10. `openEditor(todo, centerOnScreen)` 的 `centerOnScreen` 只决定是否设置 parent，定位始终是当前显示器居中。
11. 日历把**已完成**待办也画成条（半透明 + 删除线）；条数多时溢出到下一行，没有"更多"提示。
12. `docs/开发文档/04-UI设计规范.md` 部分内容（`$bg-primary: #FFFFFF`、"固定模式毛玻璃 / 100% 透明"）与当前实现不一致，以本文为准。
13. Element Plus 默认样式值取自 element-plus 2.13.1（lockfile 锁定版本）的 theme-chalk；tooltip/message 默认值取自其 `es/` 源码。
14. 文中"按 CSS 推断"的条目（第 5、6 条，§4.6 溢出）没有在真实 WebView 中验证（本环境无可用浏览器）。

### Related Specs

- `.trellis/spec/frontend/state-management.md` — 刷新触发器表、fetchTodos 单飞、模态期间跳过刷新、跨窗口设置事件
- `.trellis/spec/backend/window-modes.md` — 普通 / 经典固定 / 嵌入桌面三态与 `body.fixed-mode` 的对应关系
- `.trellis/spec/frontend/quality-guidelines.md` — `body.fixed-mode` 历史残留样式的教训
- `.trellis/spec/backend/sync-protocol.md` — SyncReport 与同步语义

### External References

- element-plus 2.13.1（npm）theme-chalk：`el-button.css`、`el-popper.css`、`el-message.css`、`el-message-box.css`、`el-overlay.css`、`base.css`
- @element-plus/icons-vue 2.3.2（npm）`dist/index.js` 图标路径
- sortablejs 1.14.0 `modular/sortable.esm.js`（fallback 影子、点击吞噬、AutoScroll 默认值）
- lunar-javascript 1.7.7 `lunar.js`（`getFestivals`、`getJieQi`、`MONTH`/`DAY` 中文表）
- tauri 2.9.5 crate `src/window/scripts/drag.js`、`window/plugin.rs::internal_toggle_maximize`
- gpui-pre 0.3.9 crate（核对了 `grid_cols`、`opacity`、`line_through`、`text_ellipsis`、`tooltip_show_delay`、`on_drag`/`on_drop`/`drag_over`、`deferred`、`with_animation`、`svg().with_transformation`、`start_window_move`、`WindowBackgroundAppearance` 的存在；未核对 gpui-component 0.7.3 的组件 API）
- NateScarlet/holiday-cn：`https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json`

---

## 附录 A：图标 SVG 路径（@element-plus/icons-vue 2.3.2，全部 `viewBox="0 0 1024 1024"`，`fill="currentColor"`）

- **Plus**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M480 480V128a32 32 0 0 1 64 0v352h352a32 32 0 1 1 0 64H544v352a32 32 0 1 1-64 0V544H128a32 32 0 0 1 0-64z"/></svg>`
- **ArrowLeft**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M609.408 149.376 277.76 489.6a32 32 0 0 0 0 44.672l331.648 340.352a29.12 29.12 0 0 0 41.728 0 30.59 30.59 0 0 0 0-42.752L339.264 511.936l311.872-319.872a30.59 30.59 0 0 0 0-42.688 29.12 29.12 0 0 0-41.728 0"/></svg>`
- **ArrowRight**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M340.864 149.312a30.59 30.59 0 0 0 0 42.752L652.736 512 340.864 831.872a30.59 30.59 0 0 0 0 42.752 29.12 29.12 0 0 0 41.728 0L714.24 534.336a32 32 0 0 0 0-44.672L382.592 149.376a29.12 29.12 0 0 0-41.728 0z"/></svg>`
- **ArrowDown**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M831.872 340.864 512 652.672 192.128 340.864a30.59 30.59 0 0 0-42.752 0 29.12 29.12 0 0 0 0 41.6L489.664 714.24a32 32 0 0 0 44.672 0l340.288-331.712a29.12 29.12 0 0 0 0-41.728 30.59 30.59 0 0 0-42.752 0z"/></svg>`
- **Expand**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M128 192h768v128H128zm0 256h512v128H128zm0 256h768v128H128zm576-352 192 160-192 128z"/></svg>`
- **Fold**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M896 192H128v128h768zm0 256H384v128h512zm0 256H128v128h768zM320 384 128 512l192 128z"/></svg>`
- **Finished**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M280.768 753.728 691.456 167.04a32 32 0 1 1 52.416 36.672L314.24 817.472a32 32 0 0 1-45.44 7.296l-230.4-172.8a32 32 0 0 1 38.4-51.2zM736 448a32 32 0 1 1 0-64h192a32 32 0 1 1 0 64zM608 640a32 32 0 0 1 0-64h319.936a32 32 0 1 1 0 64zM480 832a32 32 0 1 1 0-64h447.936a32 32 0 1 1 0 64z"/></svg>`
- **List**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M704 192h160v736H160V192h160v64h384zM288 512h448v-64H288zm0 256h448v-64H288zm96-576V96h256v96z"/></svg>`
- **Lock**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M224 448a32 32 0 0 0-32 32v384a32 32 0 0 0 32 32h576a32 32 0 0 0 32-32V480a32 32 0 0 0-32-32zm0-64h576a96 96 0 0 1 96 96v384a96 96 0 0 1-96 96H224a96 96 0 0 1-96-96V480a96 96 0 0 1 96-96"/><path fill="currentColor" d="M512 544a32 32 0 0 1 32 32v192a32 32 0 1 1-64 0V576a32 32 0 0 1 32-32m192-160v-64a192 192 0 1 0-384 0v64zM512 64a256 256 0 0 1 256 256v128H256V320A256 256 0 0 1 512 64"/></svg>`
- **Unlock**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M224 448a32 32 0 0 0-32 32v384a32 32 0 0 0 32 32h576a32 32 0 0 0 32-32V480a32 32 0 0 0-32-32zm0-64h576a96 96 0 0 1 96 96v384a96 96 0 0 1-96 96H224a96 96 0 0 1-96-96V480a96 96 0 0 1 96-96"/><path fill="currentColor" d="M512 544a32 32 0 0 1 32 32v192a32 32 0 1 1-64 0V576a32 32 0 0 1 32-32m178.304-295.296A192.064 192.064 0 0 0 320 320v64h352l96 38.4V448H256V320a256 256 0 0 1 493.76-95.104z"/></svg>`
- **Moon**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M240.448 240.448a384 384 0 1 0 559.424 525.696 448 448 0 0 1-542.016-542.08 391 391 0 0 0-17.408 16.384m181.056 362.048a384 384 0 0 0 525.632 16.384A448 448 0 1 1 405.056 76.8a384 384 0 0 0 16.448 525.696"/></svg>`
- **Sunny**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M512 704a192 192 0 1 0 0-384 192 192 0 0 0 0 384m0 64a256 256 0 1 1 0-512 256 256 0 0 1 0 512m0-704a32 32 0 0 1 32 32v64a32 32 0 0 1-64 0V96a32 32 0 0 1 32-32m0 768a32 32 0 0 1 32 32v64a32 32 0 1 1-64 0v-64a32 32 0 0 1 32-32M195.2 195.2a32 32 0 0 1 45.248 0l45.248 45.248a32 32 0 1 1-45.248 45.248L195.2 240.448a32 32 0 0 1 0-45.248m543.104 543.104a32 32 0 0 1 45.248 0l45.248 45.248a32 32 0 0 1-45.248 45.248l-45.248-45.248a32 32 0 0 1 0-45.248M64 512a32 32 0 0 1 32-32h64a32 32 0 0 1 0 64H96a32 32 0 0 1-32-32m768 0a32 32 0 0 1 32-32h64a32 32 0 1 1 0 64h-64a32 32 0 0 1-32-32M195.2 828.8a32 32 0 0 1 0-45.248l45.248-45.248a32 32 0 0 1 45.248 45.248L240.448 828.8a32 32 0 0 1-45.248 0m543.104-543.104a32 32 0 0 1 0-45.248l45.248-45.248a32 32 0 0 1 45.248 45.248l-45.248 45.248a32 32 0 0 1-45.248 0"/></svg>`
- **Refresh**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M771.776 794.88A384 384 0 0 1 128 512h64a320 320 0 0 0 555.712 216.448H654.72a32 32 0 1 1 0-64h149.056a32 32 0 0 1 32 32v148.928a32 32 0 1 1-64 0v-50.56zM276.288 295.616h92.992a32 32 0 0 1 0 64H220.16a32 32 0 0 1-32-32V178.56a32 32 0 0 1 64 0v50.56A384 384 0 0 1 896.128 512h-64a320 320 0 0 0-555.776-216.384z"/></svg>`
- **Setting**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M600.704 64a32 32 0 0 1 30.464 22.208l35.2 109.376c14.784 7.232 28.928 15.36 42.432 24.512l112.384-24.192a32 32 0 0 1 34.432 15.36L944.32 364.8a32 32 0 0 1-4.032 37.504l-77.12 85.12a357 357 0 0 1 0 49.024l77.12 85.248a32 32 0 0 1 4.032 37.504l-88.704 153.6a32 32 0 0 1-34.432 15.296L708.8 803.904c-13.44 9.088-27.648 17.28-42.368 24.512l-35.264 109.376A32 32 0 0 1 600.704 960H423.296a32 32 0 0 1-30.464-22.208L357.696 828.48a352 352 0 0 1-42.56-24.64l-112.32 24.256a32 32 0 0 1-34.432-15.36L79.68 659.2a32 32 0 0 1 4.032-37.504l77.12-85.248a357 357 0 0 1 0-48.896l-77.12-85.248A32 32 0 0 1 79.68 364.8l88.704-153.6a32 32 0 0 1 34.432-15.296l112.32 24.256c13.568-9.152 27.776-17.408 42.56-24.64l35.2-109.312A32 32 0 0 1 423.232 64H600.64zm-23.424 64H446.72l-36.352 113.088-24.512 11.968a294 294 0 0 0-34.816 20.096l-22.656 15.36-116.224-25.088-65.28 113.152 79.68 88.192-1.92 27.136a293 293 0 0 0 0 40.192l1.92 27.136-79.808 88.192 65.344 113.152 116.224-25.024 22.656 15.296a294 294 0 0 0 34.816 20.096l24.512 11.968L446.72 896h130.688l36.48-113.152 24.448-11.904a288 288 0 0 0 34.752-20.096l22.592-15.296 116.288 25.024 65.28-113.152-79.744-88.192 1.92-27.136a293 293 0 0 0 0-40.256l-1.92-27.136 79.808-88.128-65.344-113.152-116.288 24.96-22.592-15.232a288 288 0 0 0-34.752-20.096l-24.448-11.904L577.344 128zM512 320a192 192 0 1 1 0 384 192 192 0 0 1 0-384m0 64a128 128 0 1 0 0 256 128 128 0 0 0 0-256"/></svg>`
- **Document**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M832 384H576V128H192v768h640zm-26.496-64L640 154.496V320zM160 64h480l256 256v608a32 32 0 0 1-32 32H160a32 32 0 0 1-32-32V96a32 32 0 0 1 32-32m160 448h384v64H320zm0-192h160v64H320zm0 384h384v64H320z"/></svg>`
- **Bell**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M512 64a64 64 0 0 1 64 64v64H448v-64a64 64 0 0 1 64-64"/><path fill="currentColor" d="M256 768h512V448a256 256 0 1 0-512 0zm256-640a320 320 0 0 1 320 320v384H192V448a320 320 0 0 1 320-320"/><path fill="currentColor" d="M96 768h832q32 0 32 32t-32 32H96q-32 0-32-32t32-32m352 128h128a64 64 0 0 1-128 0"/></svg>`
- **CirclePlus**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M352 480h320a32 32 0 1 1 0 64H352a32 32 0 0 1 0-64"/><path fill="currentColor" d="M480 672V352a32 32 0 1 1 64 0v320a32 32 0 0 1-64 0"/><path fill="currentColor" d="M512 896a384 384 0 1 0 0-768 384 384 0 0 0 0 768m0 64a448 448 0 1 1 0-896 448 448 0 0 1 0 896"/></svg>`
- **Delete**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M160 256H96a32 32 0 0 1 0-64h256V95.936a32 32 0 0 1 32-32h256a32 32 0 0 1 32 32V192h256a32 32 0 1 1 0 64h-64v672a32 32 0 0 1-32 32H192a32 32 0 0 1-32-32zm448-64v-64H416v64zM224 896h576V256H224zm192-128a32 32 0 0 1-32-32V416a32 32 0 0 1 64 0v320a32 32 0 0 1-32 32m192 0a32 32 0 0 1-32-32V416a32 32 0 0 1 64 0v320a32 32 0 0 1-32 32"/></svg>`
- **RefreshLeft**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M289.088 296.704h92.992a32 32 0 0 1 0 64H232.96a32 32 0 0 1-32-32V179.712a32 32 0 0 1 64 0v50.56a384 384 0 0 1 643.84 282.88 384 384 0 0 1-383.936 384 384 384 0 0 1-384-384h64a320 320 0 1 0 640 0 320 320 0 0 0-555.712-216.448z"/></svg>`
- **RefreshRight**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M784.512 230.272v-50.56a32 32 0 1 1 64 0v149.056a32 32 0 0 1-32 32H667.52a32 32 0 1 1 0-64h92.992A320 320 0 1 0 524.8 833.152a320 320 0 0 0 320-320h64a384 384 0 0 1-384 384 384 384 0 0 1-384-384 384 384 0 0 1 643.712-282.88"/></svg>`
- **Select**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M77.248 415.04a64 64 0 0 1 90.496 0l226.304 226.304L846.528 188.8a64 64 0 1 1 90.56 90.496l-543.04 543.04-316.8-316.8a64 64 0 0 1 0-90.496"/></svg>`
- **SuccessFilled**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M512 64a448 448 0 1 1 0 896 448 448 0 0 1 0-896m-55.808 536.384-99.52-99.584a38.4 38.4 0 1 0-54.336 54.336l126.72 126.72a38.27 38.27 0 0 0 54.336 0l262.4-262.464a38.4 38.4 0 1 0-54.272-54.336z"/></svg>`
- **Top**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M572.235 205.282v600.365a30.118 30.118 0 1 1-60.235 0V205.282L292.382 438.633a28.913 28.913 0 0 1-42.646 0 33.43 33.43 0 0 1 0-45.236l271.058-288.045a28.913 28.913 0 0 1 42.647 0L834.5 393.397a33.43 33.43 0 0 1 0 45.176 28.913 28.913 0 0 1-42.647 0l-219.618-233.23z"/></svg>`
- **InfoFilled**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M512 64a448 448 0 1 1 0 896.064A448 448 0 0 1 512 64m67.2 275.072c33.28 0 60.288-23.104 60.288-57.344s-27.072-57.344-60.288-57.344c-33.28 0-60.16 23.104-60.16 57.344s26.88 57.344 60.16 57.344M590.912 699.2c0-6.848 2.368-24.64 1.024-34.752l-52.608 60.544c-10.88 11.456-24.512 19.392-30.912 17.28a12.99 12.99 0 0 1-8.256-14.72l87.68-276.992c7.168-35.136-12.544-67.2-54.336-71.296-44.096 0-108.992 44.736-148.48 101.504 0 6.784-1.28 23.68.064 33.792l52.544-60.608c10.88-11.328 23.552-19.328 29.952-17.152a12.8 12.8 0 0 1 7.808 16.128L388.48 728.576c-10.048 32.256 8.96 63.872 55.04 71.04 67.84 0 107.904-43.648 147.456-100.416z"/></svg>`
- **WarningFilled**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M512 64a448 448 0 1 1 0 896 448 448 0 0 1 0-896m0 192a58.43 58.43 0 0 0-58.24 63.744l23.36 256.384a35.072 35.072 0 0 0 69.76 0l23.296-256.384A58.43 58.43 0 0 0 512 256m0 512a51.2 51.2 0 1 0 0-102.4 51.2 51.2 0 0 0 0 102.4"/></svg>`
- **CircleCloseFilled**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M512 64a448 448 0 1 1 0 896 448 448 0 0 1 0-896m0 393.664L407.936 353.6a38.4 38.4 0 1 0-54.336 54.336L457.664 512 353.6 616.064a38.4 38.4 0 1 0 54.336 54.336L512 566.336 616.064 670.4a38.4 38.4 0 1 0 54.336-54.336L566.336 512 670.4 407.936a38.4 38.4 0 1 0-54.336-54.336z"/></svg>`
- **Close**: `<svg viewBox="0 0 1024 1024"><path fill="currentColor" d="M764.288 214.592 512 466.88 259.712 214.592a31.936 31.936 0 0 0-45.12 45.12L466.752 512 214.528 764.224a31.936 31.936 0 1 0 45.12 45.184L512 557.184l252.288 252.288a31.936 31.936 0 0 0 45.12-45.12L557.12 512.064l252.288-252.352a31.936 31.936 0 1 0-45.12-45.184z"/></svg>`

- **四宫格（自绘，列表模式下的视图切换图标）**: `<svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><rect x="3" y="3" width="8" height="8" rx="1"/><rect x="13" y="3" width="8" height="8" rx="1"/><rect x="3" y="13" width="8" height="8" rx="1"/><rect x="13" y="13" width="8" height="8" rx="1"/></svg>`

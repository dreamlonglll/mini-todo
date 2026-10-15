# Research: 设置窗口与桌面收纳箱窗口的 UI 规格（供 GPUI 1:1 重写）

- **Query**：为设置窗口（SettingsView）与桌面收纳箱窗口（FenceView + FenceItemGrid）写一份可直接照着用 GPUI（gpui-pre 0.3.9 + gpui-kit / gpui-component 0.7.3）重新实现的 UI 规格：元素、样式、文案、交互、调用的命令与事件
- **Scope**：以内部代码为主（Vue/TS/SCSS/Rust 源码 + `.trellis/spec/backend/desktop-fences.md`）；Element Plus 默认样式取自 `pc/node_modules/element-plus@2.13.1/theme-chalk`；GPUI 能力取自本机 cargo registry 中的 `gpui-pre-0.3.9`、`gpui-pre-windows-0.3.9`、`gpui-component-0.7.3` 源码
- **Date**：2026-10-15

> 约定：本文所有尺寸单位为 **CSS 像素 = 逻辑像素（DIP）**，除非标明"物理像素"。颜色为十六进制或 `rgba()`。
> "逐字"文案用「」或代码块给出，**包括全角/半角标点、空格与省略号的写法**（例如 `导出中...` 是三个半角点，`移动到…` 是一个省略号字符）。

---

## 0. 源文件清单

| 文件 | 说明 |
|---|---|
| `pc/src/views/SettingsView.vue` | 设置窗口（1743 行，逻辑 + 模板 + scoped 样式） |
| `pc/src/views/MainView.vue` L559-623 | 打开设置窗口（`new WebviewWindow` 参数）、模态遮罩、关闭后的刷新；L254-363 `reloadAppSettings` 与 `app-settings-changed` 处理 |
| `pc/src/stores/appStore.ts` | 设置项的 load/set（每个 setter 的失败文案与回滚）、屏幕配置、检查更新 |
| `pc/src/types/app.ts` | `AppSettingKey`、`SyncSettings`、`SyncReport`、`ScreenConfig`、`PRESET_BG_COLORS`、默认底色 |
| `pc/src/utils/syncReport.ts` | SyncReport 提示文案 |
| `pc/src/utils/notify.ts` | `notifyError` / `errorMessage`（统一失败提示格式） |
| `pc/src/utils/color.ts` | `isSameColor` / `hexToRgbChannels` |
| `pc/src/views/FenceView.vue` | 收纳箱窗口外壳（1050 行） |
| `pc/src/components/FenceItemGrid.vue` | 收纳箱项目网格（635 行） |
| `pc/src/utils/fence.ts` / `fence.test.ts` | 收纳箱纯函数与单测向量 |
| `pc/src/types/fence.ts` | 收纳箱类型与前后端共用常量 |
| `pc/src/styles/main.scss` | 全局变量与 Element Plus 覆盖（`.el-button` / `.el-input__wrapper` 圆角） |
| `pc/src-tauri/src/commands/settings_cmd.rs` | 字体、通知类型、字体列表命令 |
| `pc/src-tauri/src/commands/window.rs` | 日历 / 贴边隐藏 / 唤起置顶 / 嵌入桌面 / 底色 / 主题 / 屏幕配置 / 托盘自启同步命令 |
| `pc/src-tauri/src/commands/sync_cmd.rs` | 同步设置读写、测试连接、三个同步命令、`SyncReport` |
| `pc/src-tauri/src/commands/data.rs` | `read_app_settings` / `write_app_settings`、导入导出命令 |
| `pc/src-tauri/src/commands/fence.rs` / `fence_items.rs` | 收纳箱命令（签名见 §6） |
| `pc/src-tauri/src/services/fence_fs/{ops,folder,naming,icon}.rs` | `ItemOpResult` / `FenceEntry` / 显示名 / 图标档位 |
| `.trellis/spec/backend/desktop-fences.md` | 收纳箱契约（建窗参数、几何落库、错误矩阵） |

---

## 1. 通用约定（两个窗口共用）

### 1.1 全局样式基线（`main.scss`）

| 项 | 值 |
|---|---|
| 全局 reset | `* { margin:0; padding:0; box-sizing:border-box }`（所以 `<p>`、`<h2>`、`<h3>` 默认无外边距，只有组件里显式写的 margin） |
| body 字体 | `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'Microsoft YaHei', sans-serif`，14px，`line-height: 1.5`，抗锯齿 |
| html/body 背景 | `transparent`（`--bg-primary`）；`index.html` 内联 `body{background:#FFFFFF}` 被 main.scss 覆盖（只在首帧可能闪白） |
| `--primary` | `#3B82F6`（`--primary-light #60A5FA`，`--primary-dark #2563EB`） |
| `--transition-fast` | 150ms |
| Element Plus 覆盖 | `.el-button { border-radius: 6px !important }`、`.el-button > span { display:inline-flex; align-items:center; gap:6px }`、`.el-input__wrapper { border-radius: 6px !important }`（**el-select 的 wrapper 不受影响，仍是 4px**） |
| 深色主题 / 底色 CSS 变量 | 只作用于 label 为 `main` 的窗口（`applyThemeClass` / `applyWindowBackground` 里 `if (appWindow.label !== 'main') return`）；设置窗口与收纳箱窗口**始终是自己的配色**，不随深色主题变化 |
| Element Plus 语言 | 未配置 locale（`main.ts` 无 `ElConfigProvider`），所以组件内置文案是**英文**：color-picker 弹层按钮「Clear」「OK」、select 无数据「No data」、筛选无匹配「No matching data」 |

### 1.2 Element Plus 2.13.1 默认主题变量（实际用到的）

| 变量 | 值 |
|---|---|
| `--el-color-primary` | `#409EFF`（light-3 `rgb(121,187,255)`、light-5 ≈ `#A0CFFF`、light-9 `rgb(236,245,255)`、dark-2 `rgb(51,126,204)`） |
| `--el-color-danger` | `#F56C6C`（light-3 `rgb(248,152,152)`、light-5 ≈ `#FAB6B6`、light-9 `rgb(254,240,240)`） |
| `--el-color-success` / warning / info | `#67C23A` / `#E6A23C` / `#909399` |
| `--el-border-color` / light / lighter | `#DCDFE6` / `#E4E7ED` / `#EBEEF5` |
| `--el-text-color-primary` / regular / secondary / placeholder | `#303133` / `#606266` / `#909399` / `#A8ABB2` |
| `--el-fill-color-blank` / light / 默认 | `#FFFFFF` / `#F5F7FA` / `#F0F2F5` |
| `--el-border-radius-base` | 4px |
| `--el-font-size-base` | 14px |
| `--el-component-size` / small | 32px / 24px |
| `--el-transition-duration` / fast | 0.3s / 0.2s |
| `--el-box-shadow-light` | `0 0 12px rgba(0,0,0,0.12)` |
| `--el-overlay-color-lighter`（MessageBox 遮罩） | `rgba(0,0,0,0.5)` |

### 1.3 Element Plus 组件尺寸速查（GPUI 自绘时按此还原）

| 组件 | 用到的变体 | 尺寸 / 颜色 / 行为 |
|---|---|---|
| `el-switch` | 默认尺寸；`loading`；`disabled` | 外层 inline-flex 高 32；轨道 `min-width 40 × 20`，圆角 10，1px 边框同底色；关 `#DCDFE6`，开 `#409EFF`；拇指 16×16 白色圆，关时 `left:1px`，开时 `left: calc(100% - 17px)`；颜色/位置过渡 0.3s；`disabled` 或 `loading` → 整体 `opacity .6`、`cursor: not-allowed`；`loading` 时拇指里显示旋转的 Loading 图标 |
| `el-slider` | 默认 / `size="small"`；`show-tooltip` | 外层高 32（small 24）；轨道 6px 高 `#E4E7ED`、圆角 3；已选段 `#409EFF`；拇指 20×20 白底 + 2px `#409EFF` 边框圆，hover/拖动 `scale(1.2)`，`cursor: grab/grabbing`；点击轨道跳到该值；禁用时已选段与拇指边框 `#A8ABB2`；`show-tooltip`（默认 true）时 hover/拖动在拇指上方显示深色气泡（bg `#303133`、白字 12px、padding 5px 11px、圆角 4、带箭头） |
| `el-radio-group` + `el-radio-button` | `size="small"` | 按钮组水平拼接；每格 1px `#DCDFE6` 边框、白底、文字 `#606266`、small 字号 12、padding 5px 11px（设置页覆盖为 **6px 12px**）；首格左圆角 4、末格右圆角 4；hover 文字 `#409EFF`；选中：底 `#409EFF`、边框 `#409EFF`、白字；禁用：文字 `#A8ABB2`，不可点 |
| `el-input` | `size="small"` | 高 24，字号 12，padding `1px 7px`；边框 = 1px inset `#DCDFE6`，hover `#C0C4CC`，聚焦 `#409EFF`；圆角 **6**（被 main.scss 覆盖）；文字 `#606266`，占位 `#A8ABB2`；`clearable`：非空且 hover/聚焦时右侧出现圆形清除图标；`maxlength` 只限制输入长度（未开 `show-word-limit`，不显示计数） |
| `el-select` | 默认（字体选择）/ `size="small"`（同步间隔） | 默认：min-height 32、padding 4px 12px、字号 14、圆角 4；small：min-height 24、padding 2px 8px、字号 12；右侧下拉箭头，展开时旋转 180°；弹层：白底、1px `#E4E7ED` 边框、`--el-box-shadow-light`、圆角 4，选项高 34、padding `0 32px 0 20px`、字号 14、hover 底 `#F5F7FA`、选中项文字 `#409EFF` 加粗；`filterable`：在框内直接输入过滤（不区分大小写的子串匹配）；`clearable`：hover 时箭头变清除图标 |
| `el-color-picker` | `size="small"`、不带 alpha | 触发器 24×24，1px `#DCDFE6` 边框、圆角 4、padding 4，内部是当前色色块（带 1px `#909399` 边框）；禁用时色块 `opacity .3`、`cursor:not-allowed`；弹层：内容宽 300、padding 12；SV 面板 280×180；色相条 280×12（水平）；下方 hex 输入框 + 「Clear」（文字按钮）与「OK」（小号 plain 按钮）；点 OK 才触发 `change`，点 Clear 触发 `change(null)` |
| `el-button` | `text`（设置窗口关闭）/ `size="small"` / `type="danger"` / `type="primary"` / `loading` | 默认高 32、padding 8px 15px、字号 14；small 高 24、padding 5px 11px、字号 12；圆角 **6**（覆盖）；默认按钮：白底、1px `#DCDFE6`、文字 `#606266`，hover 文字 `#409EFF`、底 `#ECF5FF`、边框 light-7；primary：底/边 `#409EFF`、白字，hover light-3；danger：底/边 `#F56C6C`、白字，hover light-3；`text`：透明无边框，hover 底 `#F5F7FA`；禁用：primary/danger 用 light-5 色，`cursor:not-allowed`；`loading`：左侧旋转图标并禁用 |
| `el-dropdown` + menu | `trigger="click"`、`placement="bottom-end"`、`max-height=200` | 弹层白底、1px `#E4E7ED` 边框、`--el-box-shadow-light`、圆角 4、默认带小箭头；菜单 padding 5px 0；菜单项 padding 5px 16px、行高 22、字号 14、文字 `#606266`、图标在左（右边距 5px）；hover 底 `#ECF5FF`、文字 `#409EFF`；`divided` 项上方 1px `#EBEEF5` 分隔线（上下各 6px 外边距）；点击项后自动关闭；点击外部关闭 |
| `ElMessage`（toast） | `success/warning/info/error`；`duration`；`showClose` | 窗口内**顶部居中**，距顶 16px，多条向下叠放；padding 11px 15px、圆角 4、1px 边框、gap 8、左侧类型图标；`max-width: calc(100% - 32px)`；默认 3000ms 自动消失；配色：success 底 `#F0F9EB` 字 `#67C23A`，warning 底 `#FDF6EC` 字 `#E6A23C`，info 底 `#F4F4F5` 字 `#909399`，error 底 `#FEF0F0` 字 `#F56C6C`（边框为对应 light-8）；`showClose` 时右侧 16px 关闭图标 |
| `ElMessageBox.confirm` | `type: 'warning' / 'info'` | 半透明遮罩 `rgba(0,0,0,0.5)` 覆盖整窗；对话框最大宽 420、padding 12、圆角 4、白底、阴影；标题 18px `#303133`；右上角 × 关闭；正文左侧 24px 状态图标（warning 黄 / info 灰）+ 14px `#606266` 文字；底部按钮右对齐：取消（默认按钮）在左、确认（primary）在右；Esc / × / 点遮罩 = 取消（Promise reject，代码里一律当"用户取消"静默处理） |

### 1.4 失败提示的统一格式（`utils/notify.ts`）

```text
errorMessage(e):
  text = e 是字符串 ? e : e 是 Error ? e.message : e 有字符串 message ? e.message : String(e ?? '')
  text = trim(text)
  若 text 长度 > 200 → text[0..200] + '…'
notifyError(e, msg):
  detail = errorMessage(e)
  ElMessage.error(detail ? `${msg}：${detail}` : msg)     // 注意是全角冒号「：」
```

Tauri 命令的 `Err(String)` 到前端就是字符串，所以 `notifyError` 显示为「操作描述：后端中文错误」。

### 1.5 用到的 Element Plus 图标（@element-plus/icons-vue）

GPUI 侧需要同名 SVG（gpui-kit-assets 或自带 SVG）。

| 窗口 | 图标名 |
|---|---|
| 设置 | `ArrowDown` `ArrowRight` `Bell` `Brush` `Calendar` `Check` `Clock` `Close` `Connection` `Delete` `Download` `Folder` `Hide` `InfoFilled` `Monitor` `Moon` `Promotion` `Refresh` `Setting` `Sunny` `Timer` `Top` `Upload` `View` `WarningFilled` |
| 收纳箱 | `ArrowDown` `ArrowUp` `Brush` `Close` `Delete` `EditPen` `FolderOpened` `InfoFilled` `Lock` `MoreFilled` `Unlock` |
| 网格 | `ArrowLeft` `Back` `Document` `Folder` `FolderOpened` `Loading`（旋转） `Rank` `Right` `TopRight` |

`el-icon` 默认 `width = height = 1em`（跟随所在元素字号），颜色继承 `currentColor`；`:size="N"` 直接设字号 N。

---

## 2. 设置窗口（SettingsView，路由 `#/settings`）

### 2.1 打开方式与窗口参数

入口：
1. 主窗口标题栏「设置」按钮（`TitleBar.vue`：`title="设置"`、`Setting` 图标 16px）→ `emit('open-settings')` → `MainView.openSettings()`
2. 托盘菜单「打开设置」（id `open_settings`）→ 后端向主窗口 emit `tray-open-settings` → `openSettings()`

`MainView.openSettings()`（L560-623）：

| 参数 | 值 |
|---|---|
| 同时只允许一个"模态子窗口" | `isModalOpen` 为 true 时直接 return（编辑器 / 已完成 / 设置共用这个标志） |
| label | `settings-${Date.now()}` |
| url | `#/settings` |
| title | `设置` |
| 尺寸 | **680 × 560** 逻辑像素（`width/height` 是内容区；无边框所以即外框） |
| 位置 | 主窗口所在显示器（`currentMonitor()`，取不到用 `primaryMonitor()`）的**整块显示器**（不是工作区）居中：`x = round(mx + (mw − 680)/2)`、`y = round(my + (mh − 560)/2)`，其中 `mx,my,mw,mh` = 显示器物理位置/尺寸 ÷ 缩放；两者都取不到时以主窗口外框为参照居中 |
| `resizable` | false |
| `decorations` | false（自绘标题栏） |
| `transparent` | false |
| `parent` | 主窗口（owned window：始终在主窗口之上、随主窗口最小化） |

主窗口在设置窗口打开期间：
- 显示全窗遮罩 `.modal-overlay`：`position:fixed; inset:0; background: rgba(0,0,0,0.3); z-index:999; cursor:not-allowed`；在遮罩上 `mousedown` → 把设置窗口 `setFocus()` 拉到前台
- 主窗口获得焦点时（`onFocusChanged(true)`）若有模态窗口 → 把它拉到前台，不刷新列表
- 5 秒变更轮询暂停（`if (!isModalOpen) refreshIfChanged()`）
- 设置窗口销毁（`tauri://destroyed`）后：`isModalOpen=false` → `todoStore.fetchTodos()` → `reloadAppSettings()`（重读 viewMode、showCalendar、autoHide、fixedEmbedDesktop、topOnWake、windowBackground、darkTheme、字体）→ `startAutoSync()`（按新设置重建自动同步定时器）

### 2.2 整体布局

```
┌──────────────────────────────── 680 ────────────────────────────────┐
│ window-header  (padding 12px 20px, 白底, 底边 1px #E2E8F0)            │  ≈57px
│  「设置」17px/600/#1E293B                              [×] text按钮  │
├───────────────┬─────────────────────────────────────────────────────┤
│ settings-menu │ settings-content (flex:1, padding 20px 24px,        │
│ 160px 白底    │  overflow-y:auto)                                    │
│ 右边 1px      │                                                      │
│ #E2E8F0       │  <当前面板>                                          │
│ padding 8px   │                                                      │
│ overflow-y    │                                                      │
│ auto          │                                                      │
└───────────────┴─────────────────────────────────────────────────────┘
整窗背景 #F8FAFC（.settings-window: flex column, height 100vh）
```

- 五个面板都常驻（`v-show` 切换），切换标签**不丢失状态**（例如密码输入框里的内容、「高级」的展开态）。
- 没有任何键盘快捷键（无 Esc 关闭）。

### 2.3 标题栏（`.window-header`）

| 项 | 值 |
|---|---|
| 布局 | flex，`align-items:center`，`justify-content:space-between`，padding `12px 20px`，白底，`border-bottom: 1px solid #E2E8F0` |
| 标题 | `<h2>设置</h2>`：17px、600、`#1E293B`（行高继承 1.5 ≈ 25.5px） |
| 关闭按钮 | `el-button text`（默认尺寸 32px 高、padding 8px 15px），内容 `Close` 图标（继承 14px）；hover 底 `#F5F7FA`；点击 → `appWindow.close()` |
| 标题栏高度 | 由关闭按钮撑开：12 + 32 + 12 = 56，加 1px 底边 ≈ 57px |
| 拖动窗口 | 标题栏 `data-tauri-drag-region="deep"` + `@mousedown`：仅左键（`e.buttons === 1`），目标不在 `[data-tauri-drag-region="false"]`、`button, input, textarea, select, a, [role="button"]` 内 → `preventDefault` + `startDragging()`。窗口不可缩放，所以双击标题栏不会最大化 |

### 2.4 左侧菜单（`.settings-menu`）

| key | 文案 | 图标 |
|---|---|---|
| `general` | 常规 | `Setting` |
| `appearance` | 外观 | `Brush` |
| `data` | 数据与同步 | `Folder` |
| `screen` | 屏幕配置 | `Monitor` |
| `about` | 关于 | `InfoFilled` |

默认选中 `general`。菜单项样式：

| 状态 | 样式 |
|---|---|
| 默认 | flex、gap 8、padding `10px 12px`、圆角 8、13px / 500、文字 `#64748B`、`cursor:pointer`、`user-select:none`、过渡 0.15s；图标 16px；项之间无间距（项高 ≈ 10+19.5+10 ≈ 39.5px） |
| hover | 底 `#F1F5F9`，文字 `#334155` |
| active | 底 `#EFF6FF`，文字 `#3B82F6` |

### 2.5 内容区通用样式

| class | 样式 |
|---|---|
| `.panel-title`（`<h3>`） | flex、`align-items:center`、gap 8、`margin: 0 0 16px`、15px / 600、`#1E293B` |
| `.settings-row` | flex、`align-items:center`、`justify-content:space-between`、padding `12px 0`、`border-bottom: 1px solid #F1F5F9`；**同级最后一个子元素**（`:last-child`）去掉底边且 `padding-bottom: 0` |
| `.row-left` | flex、`align-items:center`、gap 12 |
| `.row-icon` | 18px、`#64748B` |
| `.row-content` | flex column |
| `.settings-label` | 14px、500、`#334155` |
| `.settings-desc` | 12px、`#94A3B8`、`margin-top: 2px` |
| `.form-section` | `margin-bottom: 20px` |
| `.form-label` | flex、`align-items:center`、`justify-content:space-between`、13px / 500、`#64748B`、`margin-bottom: 8px` |
| `.form-label-value` | 12px、`#3B82F6`、600 |
| `.card-hint` | flex、`align-items:flex-start`、gap 6、12px、`#64748B`；内部图标 14px、`#94A3B8`、`margin-top:1px` |
| `.card-hint.warning-hint` | `margin-top:4px`、文字 `#B45309`、图标 `#F59E0B` |
| `.section-divider` | 高 1px、`#E2E8F0`、`margin: 20px 0` |
| `.data-btn` | 见 §2.8.6 |

### 2.6 初始化与跨窗口同步

`onMounted` 顺序：
1. `autoStart = await isEnabled()`（tauri-plugin-autostart，读注册表；失败只打日志）
2. `appStore.loadScreenConfigs()` → `invoke('list_screen_configs')`
3. `loadAppSettingsForDisplay()`：`get_notification_type`（`'app'` 以外一律当 `'system'`）→ `loadShowCalendar` → `loadAutoHideEnabled` → `loadTopOnWake` → `loadFixedEmbedDesktop` → `loadWindowBackground` → `loadDarkTheme` → `get_todo_font_family` + `get_todo_font_size`
4. `loadSyncSettings()` → `get_sync_settings`，并把 `webdavPassword` 置空
5. `systemFonts = await invoke('get_system_fonts')`
6. 监听全局事件 `sync-completed`（payload `SyncReport | null`）：payload 为空忽略；否则 `syncSettings.lastSyncAt = report.lastSyncAt`；`report.settingsApplied` 为真时再执行一次 `loadAppSettingsForDisplay()`

卸载时取消 `sync-completed` 监听。

**"设置改动通知主窗口"的机制**：设置窗口与主窗口不共享状态。每次改动后 `emit('app-settings-changed', { key })`（全局事件），主窗口按 key 重新从数据库加载：

| key | 由哪项触发 | 主窗口处理（`MainView` L335-363） |
|---|---|---|
| `showCalendar` | 展示日历 | `appStore.loadShowCalendar()`（随后 watch 调整窗口宽度） |
| `autoHide` | 贴边自动隐藏 | `loadAutoHideEnabled()` |
| `topOnWake` | 唤起时置顶 | `loadTopOnWake()` |
| `fixedEmbedDesktop` | 固定模式时嵌入桌面 | `loadFixedEmbedDesktop()`（窗口切换已由后端命令完成） |
| `windowBackground` | 界面底色 / 背景透明度 | `loadWindowBackground()` |
| `theme` | 深色主题 | `loadDarkTheme()` |
| `sync` | 保存 WebDAV 配置 / 自动同步开关 / 同步间隔 / 清除密码 | `startAutoSync()` |
| `update` | 检查更新发现新版本 | `appStore.checkForUpdates()`（刷新标题栏版本红点） |

另外两个独立事件：`todo-font-changed`（字体 / 字号，无 payload，主窗口 `loadTodoFontSettings()`）、`data-imported`（导入成功，无 payload，主窗口 `fetchTodos()` + `reloadAppSettings()` + `ElMessage.success('数据导入成功')`）。

**所有开关都是"即时生效"**（改动即写库 + 发事件），没有"保存"按钮；唯一的例外是 WebDAV 地址 / 用户名 / 密码需要点「保存配置」（但切换"自动同步"开关或"同步间隔"时也会连带保存当前输入框里的地址 / 用户名 / 密码，见 §2.8.4）。

### 2.7 「常规」面板（`activeMenu === 'general'`）

面板标题：`常规`。以下各行从上到下排列（全部是 `.settings-row`），右侧控件均为 Element Plus 组件。

| # | 左侧图标 | label | 说明文字（settings-desc） | 控件 | 默认值（库里缺键时） | 改动后 | 失败处理 |
|---|---|---|---|---|---|---|---|
| 1 | `Monitor` | 开机自启 | （无） | `el-switch`，`v-model` + `:loading` | 由插件 `isEnabled()` 读系统自启注册表项；读失败为 false | `enable()` / `disable()`（tauri-plugin-autostart）→ 成功提示 `已开启开机自启` / `已关闭开机自启`（success）→ 不等待地 `invoke('sync_auto_start_state', { enabled })` 同步托盘「开机自启动」勾选（该调用失败只打日志） | `ElMessage.error('设置开机自启失败')`，开关回滚为 `!value`；切换期间 `loading`（开关半透明 + 拇指转圈） |
| 2 | `Calendar` | 展示日历 | 开启后主界面将显示日历视图 | `el-switch` `:model-value` | `show_calendar` = false | `set_show_calendar { show }` → emit `app-settings-changed {key:'showCalendar'}` | 回滚 + `notifyError`：`设置日历显示失败：…` |
| 3 | `Monitor` | 贴边自动隐藏 | 固定模式下，贴边后自动隐藏并在边缘唤起 | `el-switch` | `auto_hide_enabled` = **true** | `set_auto_hide_enabled { enabled }` → emit `{key:'autoHide'}` | 回滚 + `设置贴边自动隐藏失败：…` |
| 4 | `Top` | 唤起时置顶 | 贴边唤起时显示在其它窗口之上；关闭后可能被全屏窗口遮挡 | `el-switch`，**`disabled` 当「贴边自动隐藏」关闭** | `top_on_wake` = **true** | `set_top_on_wake { enabled }` → emit `{key:'topOnWake'}` | 回滚 + `设置唤起置顶失败：…` |
| 5 | `Monitor` | 固定模式时，嵌入桌面中 | `固定后窗口嵌在桌面图标之上、其它窗口之下，显示桌面（Win+D）时不会被隐藏；贴边隐藏与唤起置顶不再生效。 需要 Windows 11 24H2 及以上`（模板里两句之间换行，渲染为**一个空格**） | `el-switch`；**整行仅 Windows 显示**（`/windows/i.test(navigator.userAgent)`） | `fixed_embed_desktop` = false | `set_fixed_embed_desktop { enabled }`（后端：当前已固定时立即切换普通固定 ⇄ 嵌入桌面；非 Windows 只记偏好）→ emit `{key:'fixedEmbedDesktop'}` | 回滚 + `设置嵌入桌面失败：…` |
| 6 | `Moon` | 深色主题 | 启用半透明深色外观 | `el-switch` | `text_theme` = `'dark'` → 深色主题**关**（注意语义反转：`text_theme === 'light'` 才表示深色主题开） | `set_text_theme { theme: enabled ? 'light' : 'dark' }` → emit `{key:'theme'}` | 回滚 + `设置深色主题失败：…` |
| 7 | `Brush` | 界面底色 | 仅深色主题下生效 | 6 个预设色圆点 + `el-color-picker size="small"`；**深色主题关闭时全部 disabled** | `window_bg_color` = `#000000` | `set_window_background { color, alpha: 当前alpha }` → emit `{key:'windowBackground'}`；color-picker 的 `change(null)`（点 Clear）按 `#000000` 处理 | 回滚 color+alpha + `设置界面底色失败：…` |
| 8 | `Sunny` | 背景透明度 | 数值越低越透明，仅深色主题下生效 | `el-slider` min 5 / max 100 / step 5，tooltip 格式 `${val}%`，右侧数值文字；深色主题关闭时 disabled | `window_bg_alpha` = 0.45（显示 45%） | 拖动中（`@input`）只改本窗口显示值；松手（`@change`）`set_window_background { color: 当前color, alpha: val/100 }` → emit `{key:'windowBackground'}` | 回滚 + `设置界面底色失败：…` |
| 9 | `Bell` | 通知方式 | 选择待办提醒的通知展示方式 | `el-radio-group size="small"`：`系统通知`（value `system`）/ `软件通知`（value `app`）；切换期间整个组 disabled | `notification_type` = `system` | 先把显示值改为新值 → `set_notification_type { notificationType }`（后端：非 `system`/`app` 一律存 `system`）→ 提示 `已切换为系统通知` / `已切换为软件通知`（success）。**不发** `app-settings-changed`（通知调度在后端读库） | `ElMessage.error('设置通知类型失败')`，回滚为旧值 |

细节：

- **第 7 行「界面底色」控件**（`.bg-color-picker`：flex、gap 6、不收缩）：
  - 预设按钮（顺序、`title` 提示、颜色）：`黑色 #000000`、`深灰 #1E293B`、`深蓝 #0F2942`、`深紫 #2E1065`、`深绿 #052E24`、`深棕 #2B1A0F`
  - 圆点：20×20、圆形、`border: 2px solid transparent`、`box-shadow: 0 0 0 1px #E2E8F0`（外圈细灰线）、背景 = 该色；hover（未禁用）`scale(1.15)`；当前色（`isSameColor`，不区分大小写）边框 `#3B82F6`；禁用 `opacity .4`、`cursor:not-allowed`；过渡 0.15s
  - 之后是 24×24 的 `el-color-picker`（`:model-value` 为当前底色），弹层见 §1.3
- **第 7、8、9 行布局**：这三行加了 `flex-wrap: wrap; gap: 8px`，`.row-left { flex:1; min-width:150px }`——窗口窄时右侧控件换到下一行（680 宽下实际不换行）
- **第 8 行滑块区**（`.bg-alpha-slider`）：flex、gap 12、宽 **180**、不收缩；`el-slider` 占满剩余；右侧 `.alpha-value`：12px、`#64748B`、`min-width:34px`、右对齐，内容 `${round(alpha*100)}%`
- **第 9 行单选按钮**：覆盖 `.el-radio-button__inner { padding: 6px 12px; font-size: 12px }`
- 第 9 行是面板最后一个子元素 → 无底边、`padding-bottom: 0`
- 第 5 行的 desc 较长，会在 `row-left` 内自动换行

### 2.8 「外观」面板（`activeMenu === 'appearance'`）

面板标题：`外观`。

| 区块 | 控件 | 规格 |
|---|---|---|
| 待办字体 | `.form-section` > `label.form-label`「待办字体」+ `el-select`（默认尺寸 32 高，`width:100%`，`filterable`，`clearable`，占位 `跟随系统`） | 选项 = `get_system_fonts()` 返回的全部字体族名（Windows：DirectWrite 系统字体集合，按名排序去重；非 Windows 返回空数组 → 弹层显示「No data」）；**每个选项用它自己的字体渲染**（`:style="{ fontFamily: font }"`）。选中 → 本地 `fontFamily = val` → `set_todo_font_family { fontFamily: val }` → emit `todo-font-changed`；失败只打日志 |
| 字体大小 | `.form-section` > `label.form-label`：左「字体大小」，右 `.form-label-value` `${fontSize}px` + `el-slider`（min 12、max 20、step 1、**无 tooltip**、`v-model` 实时改本地值） | 拖动时预览与右上角数值实时变化；松手 `set_todo_font_size { fontSize }`（后端 `clamp(12, 20)`）→ emit `todo-font-changed`；失败只打日志。默认 14 |
| 预览 | `.font-preview` | padding 16、白底、1px `#E2E8F0`、圆角 8、文字 `#334155`、行高 1.5；字体 = 当前选择（空则继承 body 字体栈），字号 = 当前 `fontSize`px；内容逐字：`待办事项预览 Todo Preview 1234` |

`.form-section` 之间 `margin-bottom: 20px`。

### 2.9 「数据与同步」面板（`activeMenu === 'data'`）

自上而下：数据管理 → 分隔线 → 云同步 (WebDAV) 表单 → 分隔线 → 自动同步 / 同步间隔 → 立即同步 → 说明 → 「高级」折叠区。

#### 2.9.1 数据管理

- 面板标题：`数据管理`
- `.data-actions`：flex、gap 12、`margin-bottom: 12px`，两个 `.data-btn`（各 `flex:1`）：
  - `.data-btn.primary`：`Download` 图标 + `导出数据`（进行中显示 `导出中...` 且禁用）
  - `.data-btn`：`Upload` 图标 + `导入数据`（进行中 `导入中...` 且禁用）
- `.card-hint`：`InfoFilled`(14) + `导出为 ZIP 压缩包，可用于备份或迁移到其他设备`

**导出流程**（`handleExport`）：
1. `exporting = true`
2. 系统保存对话框：标题 `导出待办数据`；默认文件名 `mini-todo-backup-YYYY-MM-DD.zip`（**本地日期**）；过滤器 `[{ name: 'ZIP 压缩包', extensions: ['zip'] }]`
3. 选了路径 → `export_data_to_file { filePath }`（后端：生成 v4.0 JSON，写 ZIP，内含单个 `data.json`，Deflated）→ `ElMessage.success('导出成功')`；取消对话框什么都不提示
4. 失败：`ElMessage.error('导出失败: ' + String(e))`（**半角冒号 + 空格**，不走 notifyError）
5. finally `exporting = false`

**导入流程**（`handleImport`）：
1. 系统打开对话框：标题 `导入待办数据`；过滤器 `[{ name: 'ZIP / JSON 文件', extensions: ['zip', 'json'] }]`；单选文件
2. 取消 → 结束
3. 确认框（**选完文件之后才弹**）：正文 `导入将覆盖现有的所有待办数据，确定继续吗？`，标题 `导入确认`，按钮 `确定导入` / `取消`，`type: 'warning'`；取消 → 静默结束
4. `importing = true`
5. 路径以 `.zip` 结尾（区分大小写）→ `import_data_from_file { filePath }`；否则前端 `readTextFile(path)` → `import_data { jsonData }`（后端两者最终都走 `import_data_raw`：先 `VACUUM INTO` 备份，事务内清空并按原 id 插入、写墓碑、写设置；`import_data_from_file` 自己按文件头 `PK` 判断 ZIP）
6. 成功 → `emit('data-imported')` → **关闭设置窗口**（成功提示由主窗口弹 `数据导入成功`）
7. 失败（且不是用户取消）→ `ElMessage.error('导入失败: ' + String(e))`
8. finally `importing = false`

#### 2.9.2 云同步 (WebDAV) 表单

标题行（`h3.panel-title`）：`云同步 (WebDAV)`，右侧（`margin-left:auto`）仅当 `lastSyncAt` 非空时显示 `.last-sync-time`：`上次同步: YYYY-MM-DD HH:mm`（11px、`#94A3B8`、400）。时间格式化：`formatDateTime(lastSyncAt, 'YYYY-MM-DD HH:mm')`（`lastSyncAt` 是带时区偏移的 ISO 字符串，按本地时间显示；无法解析时显示原文）。

`.sync-form`：flex column、gap 12。

| 字段 | 结构 | 规格 |
|---|---|---|
| 服务器地址 | `.form-item`（column, gap 4）> `label.form-label`「服务器地址」+ `el-input size="small" clearable`，占位 `https://dav.example.com/dav` | 当 `/^http:\/\//i.test(url.trim())` 时在输入框下方显示 `.card-hint.warning-hint`：`WarningFilled`(14, `#F59E0B`) + `当前地址使用 http://，账号密码与待办数据将以明文传输，建议改用 https://`（文字 `#B45309`），随输入实时出现/消失 |
| 用户名 / 密码（同一行） | `.form-row`（flex, gap 12），两个 `.form-item.flex-1` | 用户名：`label`「用户名」+ `el-input size="small"`，占位 `用户名` |
| 密码 | `label.form-label`：左 `<span>密码</span>`，右侧仅当 `hasPassword` 时显示文字按钮 `清除已保存的密码`（`.link-btn`：无边框透明、12px、400、`#3B82F6`；hover `#2563EB` + 下划线；清除进行中禁用 `opacity .6`） | `el-input size="small"`，`type` 在 `password` / `text` 间切换，`autocomplete="new-password"`；占位：`hasPassword ? '已保存（留空则不修改）' : '密码'`；后缀图标（`.password-toggle`：`#94A3B8`，hover `#64748B`，可点）：明文态显示 `View`、密文态显示 `Hide`，点击切换。**输入框永远不回显已保存的密码**（后端 `get_sync_settings` 恒返回空串） |
| 操作 | `.form-actions`（flex, gap 8, `margin-top:4px`），两个 `.data-btn`（各 flex:1） | `Connection` + `测试连接`（进行中 `测试中...` 且禁用）；`.primary`：`Check` + `保存配置` |

`.form-label` 在 `.form-item` 中仍有 `margin-bottom: 8px`（再加 `.form-item` 的 gap 4）。

**hasPassword 语义**：后端只告诉前端"是否已保存过密码"。保存时：`clearPassword: true` → 清空；`webdavPassword` 非空 → 加密保存新密码（Windows DPAPI）；为空 → 保留旧密码。

**测试连接**（`testConnection`）：
1. 地址为空 → `ElMessage.warning('请输入 WebDAV 服务器地址')`，结束
2. `webdav_test_connection { url, username, password: 输入框值 || null }`（null = 后端用已保存的密码）
3. 成功 → `ElMessage.success('连接成功')`；失败 → `notifyError(e, '连接失败')`，常见后端原因：`认证失败，请检查用户名和密码`、`服务器返回状态码: {n}`、`WebDAV 地址必须以 http:// 或 https:// 开头`、`未配置 WebDAV 服务器`
4. **注意**：测试连接**不保存**配置

**保存配置**（`saveSyncSettings` → `persistSyncSettings(false)`）：
1. `save_sync_settings { settings: {...当前表单, webdavPassword: 输入值, clearPassword: false} }`（后端：URL 去首尾空白、`syncInterval = max(1, n)`；换了服务器或用户名时清空同步基准与"上次同步时间"）
2. emit `app-settings-changed {key:'sync'}`
3. 重新 `get_sync_settings`（刷新 `hasPassword`、清空密码框）
4. 成功 → `ElMessage.success('同步设置已保存')`；失败 → `notifyError(e, '保存同步设置失败')`

**清除已保存的密码**（`handleClearPassword`）：
1. 确认框：正文 `确定清除已保存的 WebDAV 密码吗？清除后需要重新输入密码才能同步。`，标题 `清除密码`，按钮 `清除` / `取消`，`type:'warning'`；取消 → 结束
2. `clearingPassword = true` → `persistSyncSettings(true)`（同上，`clearPassword: true`、`webdavPassword: ''`）→ 成功 `ElMessage.success('已清除保存的密码')`
3. finally `clearingPassword = false`

#### 2.9.3 自动同步（分隔线之后）

| 行 | 图标 | label | desc | 控件 | 默认 | 改动后 |
|---|---|---|---|---|---|---|
| 自动同步 | `Timer` | 自动同步 | 定时自动与云端合并同步 | `el-switch v-model="syncSettings.autoSync"` | false（`webdav_auto_sync`） | `@change` → `saveSyncSettings()`（= 保存整个表单 + emit `{key:'sync'}` + 成功提示 `同步设置已保存`） |
| 同步间隔（仅自动同步开启时显示） | `Clock` | 同步间隔 | （无） | `el-select size="small"`，宽 **120**；选项 `5 分钟`(5) / `10 分钟`(10) / `15 分钟`(15) / `30 分钟`(30) / `60 分钟`(60) | 15（`webdav_sync_interval`） | `@change` → `saveSyncSettings()` |

主窗口 `startAutoSync()`：`autoSync && webdavUrl` 时按 `max(1, syncInterval || 15)` 分钟 `setInterval` 跑自动同步。

#### 2.9.4 立即同步与说明

- `.sync-actions`（flex、gap 12、`margin-top:16px`、`margin-bottom:12px`）：一个 `.data-btn.primary`（因 flex:1 占满整行）：`Refresh` + `立即同步`（`syncAction === 'sync'` 时 `同步中...`）；**禁用条件**：有任何同步动作进行中，或地址为空
- `.card-hint`：`InfoFilled` + `通过 WebDAV 协议同步待办数据和图片：本地与云端的新增、修改、删除会自动合并，以较新的修改为准`

#### 2.9.5 「高级」折叠区

- 切换按钮 `.advanced-toggle`：inline-flex、gap 4、`margin-top:16px`、无边框透明、12px、`#64748B`（hover `#334155`）；内容 = 12px 图标（展开 `ArrowDown` / 收起 `ArrowRight`）+ `高级`；默认收起
- 展开后 `.advanced-sync`：`margin-top:8px`、padding 12、`border: 1px solid #FDE68A`、圆角 8、底 `#FFFBEB`，内含：
  - `.card-hint`（普通灰色，**不是** warning-hint）：`WarningFilled`(14, `#94A3B8`) + `仅在两端数据出现异常时使用：会以一端为准整体覆盖另一端，被覆盖一端独有的数据将被删除`
  - `.sync-actions`（`margin-top:12px; margin-bottom:0`）：两个 `.data-btn.danger`：
    - `Download` + `用云端覆盖本地`（`syncAction === 'pull'` 时 `覆盖中...`）
    - `Upload` + `用本地覆盖云端`（`syncAction === 'push'` 时 `覆盖中...`）
    - 禁用条件同「立即同步」

**二次确认文案**（逐字，正文是两段字符串拼接，中间无空格）：

| 动作 | 标题 | 正文 | 确认 / 取消 | type |
|---|---|---|---|---|
| 用云端覆盖本地 | `用云端覆盖本地` | `将让本地数据与云端完全一致：本地有而云端没有的待办和子任务会被删除，应用设置也会被云端覆盖（窗口位置等设备相关设置除外）。此操作无法撤销，建议先导出备份。确定继续吗？` | `覆盖本地` / `取消` | warning |
| 用本地覆盖云端 | `用本地覆盖云端` | `将让云端数据与本地完全一致：云端有而本地没有的待办和子任务会被删除，其它设备与云端服务下次同步时也会随之删除。此操作无法撤销。确定继续吗？` | `覆盖云端` / `取消` | warning |

#### 2.9.6 同步命令执行（`runSyncCommand`）与 SyncReport 展示

```text
runSyncCommand(action ∈ {sync, pull, push}, command, failureText, args?):
  if url 为空: ElMessage.warning('请先配置 WebDAV 服务器'); return
  if syncAction 非空: return                       // 本窗口内防重入
  syncAction = action
  try:
    report = invoke(command, args)
    syncSettings.lastSyncAt = report.lastSyncAt
    ElMessage({ type: report.status == 'no_changes' ? 'info' : 'success', message: describeSyncReport(report) })
    if (s = describeSkippedRecords(report)): ElMessage.warning(s)
    if report.settingsApplied: loadAppSettingsForDisplay()
  catch e:
    if String(e) 包含 '同步正在进行中': ElMessage.info('已有同步正在进行，请稍后再试')
    else notifyError(e, failureText)
  finally syncAction = null
```

| 按钮 | command | args | failureText |
|---|---|---|---|
| 立即同步 | `webdav_sync` | `{ full: true }`（手动同步总是无条件 GET） | `同步失败` |
| 用云端覆盖本地 | `webdav_force_pull` | — | `用云端覆盖本地失败` |
| 用本地覆盖云端 | `webdav_force_push` | — | `用本地覆盖云端失败` |

后端成功且"本地数据或设置有变化"时会向所有窗口广播 `sync-completed`（payload 同 SyncReport），主窗口据此刷新；设置窗口自己也会收到（见 §2.6）。

`SyncReport`（camelCase）：`status: 'no_changes'|'pulled'|'pushed'|'merged'`、`lastSyncAt`（ISO 带偏移）、`todosInserted/Updated/Deleted`、`subtasksInserted/Updated/Deleted`、`recordsSkipped`、`imagesUploaded`、`imagesDownloaded`、`settingsApplied`。

`describeSyncReport(report)`（逐字移植）：

```text
STATUS_TEXT = {
  no_changes: '已是最新，没有需要同步的更改',
  pulled:     '已从云端拉取更新',
  pushed:     '已将本地更改推送到云端',
  merged:     '已合并本地与云端的更改',
}
base = STATUS_TEXT[status] ?? '同步完成'
counters（顺序固定）= [
  (todosInserted,'新增待办'), (todosUpdated,'更新待办'), (todosDeleted,'删除待办'),
  (subtasksInserted,'新增子任务'), (subtasksUpdated,'更新子任务'), (subtasksDeleted,'删除子任务'),
  (imagesDownloaded,'下载图片'), (imagesUploaded,'上传图片'),
]
parts = [ `${label} ${n}` for (n,label) in counters if n > 0 ]     // 非有限数 / 缺失按 0
if settingsApplied: parts.push('已应用云端设置')
return parts 为空 ? base : `${base}（${parts.join('，')}）`        // 全角括号、全角逗号
```

例：`已合并本地与云端的更改（新增待办 2，更新子任务 1，上传图片 3，已应用云端设置）`

`describeSkippedRecords(report)`：`recordsSkipped > 0` → `有 ${n} 条云端记录无法识别，已跳过（未删除任何本地数据）`，否则 null。

#### 2.9.7 `.data-btn` 样式

| 状态 | 样式 |
|---|---|
| 默认 | `flex:1`；flex 居中、gap 8；padding `10px 16px`；`1px solid #E2E8F0`；圆角 8；白底；13px / 500；文字 `#334155`；过渡 0.2s；图标 16px |
| hover（未禁用） | 底 `#F8FAFC`，边框 `#CBD5E1` |
| 禁用 | `opacity .6`，`cursor:not-allowed` |
| `.primary` | 底 / 边框 `#3B82F6`，白字；hover 底 / 边框 `#2563EB` |
| `.danger` | 文字 `#DC2626`，边框 `#FECACA`（白底）；hover 底 `#FEF2F2`、边框 `#FCA5A5` |

### 2.10 「屏幕配置」面板（`activeMenu === 'screen'`）

- 面板标题：`屏幕配置`
- `.card-hint`（额外 `margin-bottom:12px`）：`InfoFilled` + `应用会根据不同的屏幕组合自动保存和恢复窗口位置`
- 数据：`list_screen_configs()`，按 `updated_at DESC`

**空态**（`screenConfigs.length === 0`）：`.empty-configs`：column 居中、padding 24、`#94A3B8`；`Monitor` 图标 28px（`opacity .5`、`margin-bottom:8px`）+ 13px `暂无保存的屏幕配置`

**列表**（`.config-list`：column、gap 8），每项 `.config-item`：

| 部位 | 样式 / 内容 |
|---|---|
| 容器 | flex、`align-items:center`、`space-between`；padding `12px 14px`；白底；圆角 8；`1px solid #E2E8F0`；过渡 0.2s；hover 底 `#F8FAFC`；**当前配置**（`.active`）边框 `#3B82F6`、底 `#EFF6FF` |
| `.config-name` | 13px / 500 / `#334155`，flex、gap 8：`displayName \|\| '未命名配置'`，当前配置时追加徽标 `.current-badge`：`当前`（10px、padding `2px 8px`、背景 `linear-gradient(135deg, #3B82F6 0%, #06B6D4 100%)`、白字、圆角 10、500） |
| `.config-detail` | 11px、`#64748B`、`margin-top:4px`、单行省略：`formatConfigInfo(configId)` |
| `.config-meta` | 11px、`#94A3B8`、`margin-top:2px`：`{isFixed ? '固定模式' : '普通模式'} \| 位置: ({windowX}, {windowY})`，例：`普通模式 \| 位置: (1280, 300)` |
| 删除按钮 `.config-actions`（不收缩、`margin-left:8px`） | `el-button type="danger" text size="small"`，内容 `Delete` 图标；当前配置时 `disabled` |

`formatConfigInfo(configId)`：

```text
'legacy'  → '旧版本迁移的配置'
'unknown' → '未知屏幕配置'
parts = configId.split('_'); 若 parts.len < 2 → 原样返回 configId
count = parts[0]
monitors = parts[1..].map(p => { [res, scale] = p.split('@'); `${res} ${scale}%` })
→ `${count} 个显示器: ${monitors.join(', ')}`        // 半角冒号 + 空格、半角逗号 + 空格
例：'2_2560x1440@125_1920x1080@100' → '2 个显示器: 2560x1440 125%, 1920x1080 100%'
```

（`displayName` 由主窗口保存时生成，`generateScreenConfigDisplayName`：`legacy → '旧版配置'`、`unknown → '未知配置'`，否则 `${count}屏: ${parts[1..].map(p => `${res} (${scale}%)`).join(' + ')}`，例 `2屏: 2560x1440 (125%) + 1920x1080 (100%)`。屏幕组合 ID 格式见 §5.4。）

**删除**（`handleDeleteConfig(config)`）：
1. `config.configId === currentConfigId` → `ElMessage.warning('不能删除当前正在使用的屏幕配置')`，结束
2. 确认框：正文 `确定删除屏幕配置 "${displayName || configId}" 吗？`（**半角双引号**），标题 `删除确认`，按钮 `删除` / `取消`，`type:'warning'`
3. `delete_screen_config { configId }`（后端同一事务删 `screen_configs` 行与 `fence_layouts WHERE screen_config_id = ?`，即**连带删除收纳箱在该屏幕组合下的布局**）→ 重新 `list_screen_configs`
4. 成功 `ElMessage.success('删除成功')`；失败 `ElMessage.error('删除失败')`（不带原因）

> 见 §2.13 第 1 条：当前实现里设置窗口拿不到"当前屏幕配置 ID"。

### 2.11 「关于」面板（`activeMenu === 'about'`）

无面板标题。

| 部位 | 样式 / 内容 |
|---|---|
| `.about-content` | flex、`align-items:center`、gap 16、`margin-bottom:20px` |
| `.app-logo` | 60×60、圆角 14、背景 `linear-gradient(135deg, #3B82F6 0%, #06B6D4 100%)`、白色 `Promotion` 图标 36px 居中 |
| `.app-name` | `Mini Todo`：18px / 600 / `#1E293B`，`margin-bottom:4px` |
| `.app-version` | 13px、`#64748B`、flex、gap 8、`margin-bottom:4px`：`版本 {APP_VERSION}`；有新版本时追加 `.update-badge`：`新版本 {latestVersion}`（11px、padding `2px 8px`、底 `#FEE2E2`、字 `#EF4444`、圆角 10、500） |
| `.app-desc` | `一个简洁高效的桌面待办应用`：12px、`#94A3B8` |
| `.check-update-btn` | 宽 100%；flex 居中、gap 8；padding `12px 16px`；`1px solid #E2E8F0`；圆角 8；白底；14px / 500；`#334155`；hover 底 `#F8FAFC`、边框 `#CBD5E1`；禁用 `opacity .6`；内容 `Refresh`(16) + `检查更新`（进行中 `检查中...` 且禁用） |

**检查更新**（`handleCheckUpdate`）：
1. `checking = true` → `appStore.checkForUpdates()`：读本地版本（`getVersion()`）→ `GET https://api.github.com/repos/dreamlonglll/mini-todo/releases/latest`（`Accept: application/vnd.github.v3+json`）→ `compareVersions(tag_name, 本地版本) > 0`（去掉前缀 `v`、按 `.` 分段数值比较，缺段补 0）则 `hasUpdate = true`、`latestVersion = tag_name`、`releaseUrl = html_url`；网络 / 非 2xx 时静默不改状态
2. 有更新 → emit `app-settings-changed {key:'update'}` → 确认框：正文 `发现新版本 ${latestVersion}，是否前往下载？`，标题 `版本更新`，按钮 `前往下载` / `稍后再说`，`type:'info'` → 确认则用系统浏览器打开 `releaseUrl`（缺省 `https://github.com/dreamlonglll/mini-todo/releases`）
3. 无更新 → `ElMessage.success('当前已是最新版本')`
4. 异常且不是取消 → `ElMessage.info('检查更新失败，请稍后重试')`
5. finally `checking = false`

### 2.12 设置窗口用到的后端命令（签名）

参数名：前端 camelCase，对应 Rust snake_case。

| 命令 | 参数 | 返回 | 读/写的 settings 键（默认值） | 所在文件 |
|---|---|---|---|---|
| `get_notification_type` / `set_notification_type` | `notificationType: String` | `String` / `()` | `notification_type`（`system`；非法值存 `system`） | settings_cmd.rs |
| `get_system_fonts` | — | `Vec<String>`（排序去重；非 Windows 空） | — | settings_cmd.rs |
| `get_todo_font_family` / `set_todo_font_family` | `fontFamily: String` | `String` / `()` | `todo_font_family`（空串 = 跟随系统） | settings_cmd.rs |
| `get_todo_font_size` / `set_todo_font_size` | `fontSize: i32` | `i32` / `()` | `todo_font_size`（14；写入 `clamp(12,20)`） | settings_cmd.rs |
| `get_show_calendar` / `set_show_calendar` | `show: bool` | `bool` / `()` | `show_calendar`（false） | window.rs |
| `get_auto_hide_enabled` / `set_auto_hide_enabled` | `enabled: bool` | `bool` / `()` | `auto_hide_enabled`（true）；写后同步运行时贴边状态，关闭时把藏起的主窗口拉回、撤销置顶 | window.rs |
| `get_top_on_wake` / `set_top_on_wake` | `enabled: bool` | `bool` / `()` | `top_on_wake`（true）；关闭时立即撤销主窗口置顶 | window.rs |
| `get_fixed_embed_desktop` / `set_fixed_embed_desktop` | `enabled: bool` | `bool` / `()` | `fixed_embed_desktop`（false）；Windows 上若当前固定则立即切换模式 | window.rs |
| `get_text_theme` / `set_text_theme` | `theme: String`（`light`=深色主题开） | `String` / `()` | `text_theme`（`dark`） | window.rs |
| `get_window_background` / `set_window_background` | `color: String, alpha: f64` | `{ color, alpha }` / `()` | `window_bg_color`（`#000000`）、`window_bg_alpha`（0.45；写入 `clamp(0,1)`），同一事务 | window.rs |
| `list_screen_configs` | — | `Vec<ScreenConfig>`（`updated_at DESC`） | 表 `screen_configs` | window.rs |
| `delete_screen_config` | `configId: String` | `()` | 连带删 `fence_layouts` | window.rs |
| `sync_auto_start_state` | `enabled: bool` | `()` | 只同步托盘勾选 | window.rs |
| `get_sync_settings` | — | `SyncSettings` | `webdav_url/username/password(→hasPassword)/auto_sync/sync_interval/last_sync_at/device_id` | sync_cmd.rs |
| `save_sync_settings` | `settings: SyncSettings`（含可选 `clearPassword`） | `()` | 同上 | sync_cmd.rs |
| `webdav_test_connection`（async） | `url, username, password: Option<String>` | `bool` | — | sync_cmd.rs |
| `webdav_sync`（async） | `full: Option<bool>` | `SyncReport`；emit `sync-completed` | — | sync_cmd.rs |
| `webdav_force_pull` / `webdav_force_push`（async） | — | `SyncReport`；emit `sync-completed` | — | sync_cmd.rs |
| `export_data_to_file`（async） | `filePath` | `()` | — | data.rs |
| `import_data_from_file`（async） / `import_data`（async） | `filePath` / `jsonData` | `()` | 改写 `AppSettings` 全部键 | data.rs |

`SyncSettings`（camelCase）：`webdavUrl, webdavUsername, webdavPassword（读取恒空）, hasPassword（只读）, clearPassword?（只写）, autoSync, syncInterval（默认 15）, lastSyncAt: string|null, deviceId`。

所有同步命令共用后端互斥：重入时报错字符串 `同步正在进行中`。

### 2.13 设置窗口：实现层面的现状说明（照现有行为记录，供实现者决定是否保留）

1. **"当前"屏幕配置标识在设置窗口里不生效**：`currentConfigId = appStore.currentScreenConfigId`，但设置窗口是独立 WebView、独立 Pinia 实例，它只调用了 `loadScreenConfigs()`，从未调用 `initSettings()` / `generateScreenConfigId()`，所以 `currentScreenConfigId` 恒为 `''`。现状结果：列表里没有任何一项显示「当前」徽标 / 蓝色高亮，删除按钮对所有项都可点（包括正在使用的配置）。代码意图（模板与 `handleDeleteConfig`）是标出当前配置并禁止删除它；屏幕组合 ID 的生成规则见 §5.4，GPUI 版可直接在后端算。
2. **关于页版本号初始为空**：`APP_VERSION` 只在 `initSettings()`（主窗口）与 `checkForUpdates()` 里加载，设置窗口打开时显示 `版本 `（后面为空），点过「检查更新」后才出现版本号；`hasUpdate` 同理在设置窗口里初始为 false。
3. **清空字体选择**：`el-select` 清空时 `change` 的值是 `undefined`（Element Plus 2.13 `valueOnClear` 默认 `undefined`），`invoke('set_todo_font_family', { fontFamily: undefined })` 会因缺少必填参数失败（只打 console 日志）：预览变回系统字体，但库里仍是旧字体、主窗口不变。
4. 切换"自动同步"开关或"同步间隔"会把表单里**尚未点保存**的地址 / 用户名 / 密码一起保存（`persistSyncSettings` 发送整个表单）。
5. 导入时 `.ZIP`（大写扩展名）会走 `readTextFile` + `import_data` 的 JSON 分支。

---

## 3. 收纳箱窗口（FenceView，路由 `#/fence?id={id}`，label `fence-{id}`）

### 3.1 窗口参数（后端 `open_fence_window` 建窗，前端不建）

| 项 | 值 |
|---|---|
| 无边框 / 透明 / 无阴影 | `decorations(false)` `transparent(true)` `shadow(false)`；Windows 11 上再设 DWM 圆角 `DWMWCP_ROUND` |
| 任务栏 / 焦点 | `skip_taskbar(true)` `visible(true)` `focused(false)`（不抢焦点，但**必须可激活**，不能 `focusable(false)` / `WS_EX_NOACTIVATE`） |
| 层级 | `always_on_bottom(true)`，之后挂 owner = Progman + `WS_EX_TOOLWINDOW`（嵌入桌面层，详见 `desktop-fences.md` §3.1） |
| 可缩放 | `resizable(false)`（无 `WS_THICKFRAME`）；`minimizable(false)`、`maximizable(false)` |
| 关闭 | `CloseRequested` → `prevent_close()`（Alt+F4 无效）；删除收纳箱走 `destroy()` |
| 标题 | 收纳箱标题（改名后 `set_title` 同步） |
| 最小尺寸 | 展开态 160 × 160；折叠态 160 × 32（逻辑像素） |
| 新建默认尺寸 | `(88×4 + 8×2) × (32 + 96×3 + 8×2)` = **368 × 336** DIP（150% 下 552×504 物理像素） |
| 折叠态窗口高 | `round(32 × 目标显示器缩放)` 物理像素（只剩标题栏） |
| 展开 | 用库里的展开尺寸；下方空间不够时整体上移（向上展开），并夹回标题栏所在工作区 |
| 几何单位 | 库里 `x/y/width/height` 为物理像素（x/y 是虚拟屏幕坐标，折叠态即标题栏左上角；width/height 是展开态尺寸） |

### 3.2 数据与常量

`Fence`（`get_fence` / `update_fence` 等返回，camelCase）：`id, title, folderName, x, y, width, height, collapsed, locked, bgColor, bgAlpha, iconSize, sortOrder, createdAt, updatedAt`。新建默认：`collapsed=false`、`locked=false`、`bgColor='#000000'`、`bgAlpha=0.45`、`iconSize=48`、标题 `收纳箱` / `收纳箱 2` / `收纳箱 3`…（第一个未被占用的）。

FenceView 内部时间常量：

| 常量 | 值 | 作用 |
|---|---|---|
| `HOVER_EXPAND_DELAY` | 150 ms | 折叠态鼠标进入后多久临时展开 |
| `HOVER_COLLAPSE_DELAY` | 400 ms | 鼠标离开后多久收回 |
| `DRAG_COLLAPSE_DELAY` | 300 ms | 拖文件离开 / 放下后多久收回 |
| `GEOMETRY_ARM_MS` | 3000 ms | 拖动闸门的续期窗口（见 §5.3 `GeometryCommitGate`） |
| `GEOMETRY_COMMIT_DEBOUNCE` | 300 ms | 拖动停下多久后落库 |
| `ICON_SIZE_SAVE_DELAY` | 400 ms | Ctrl+滚轮调图标大小后的落库防抖 |
| `DELETE_KEPT_MESSAGE_DURATION` | 8000 ms | 删除中止提示的显示时长 |
| 拖入结果提示时长 | 5000 ms | `moveIn` 的 ElMessage |
| `RESIZE_HANDLES` | `['n','s','w','e','nw','ne','sw','se']` | 8 个缩放手柄 |

本地状态：`fence`、`loadError`、`expandReasons: Set<'hover'|'drag'|'menu'|'panel'|'itemMenu'>`、`panel: 'none'|'appearance'|'delete'|'onedrive'`、`editingTitle`、`titleDraft`、`dragHover`、`alphaPreview: number|null`、`items: FenceEntry[]`、`deleteCount: number|null`、`deleting`。

派生：

```text
collapsed    = fence?.collapsed ?? false
locked       = fence?.locked ?? false
viewExpanded = fence 存在 && (!fence.collapsed || expandReasons 非空)     // 窗口是否显示完整高度
canResize    = fence 存在 && !locked && !collapsed && !editingTitle        // 临时展开态也不能缩放
bgAlpha      = alphaPreview ?? fence?.bgAlpha ?? 0.45
```

### 3.3 视觉结构与样式

```
.fence-window（100%×100%，flex column，overflow hidden，圆角 8，1px 半透明白边）
├─ .fence-titlebar（高 32）
│   ├─ .fence-title（或改名时的 el-input）
│   └─ .fence-actions：[折叠/展开] [固定/取消固定] [更多 ▾]
├─ .fence-body（v-show = viewExpanded；flex:1；padding 8）
│   ├─ FenceItemGrid（items 非空） 或 .fence-empty（空）
│   └─ .fence-panel（外观 / 删除确认 / OneDrive 提示，绝对定位盖住 body）
└─ 8 个 .resize-handle（仅 canResize）
```

| 元素 | 样式 |
|---|---|
| `.fence-window` | `position:relative`；flex column；100%×100%；`overflow:hidden`；文字 `#FFF`；字体 `-apple-system, 'Segoe UI', 'Microsoft YaHei', sans-serif`；**13px**；背景 `rgba(R,G,B, bgAlpha)`（RGB 来自 `hexToRgbChannels(bgColor)`）；`border: 1px solid rgba(255,255,255,0.14)`；圆角 8；`user-select:none`；body 背景透明（窗口透明区域透出桌面） |
| `.fence-titlebar` | flex、不收缩、`align-items:center`、gap 6、高 **32**、padding `0 4px 0 10px`、`cursor:default`；**仅展开视图**时 `border-bottom: 1px solid rgba(255,255,255,0.08)` |
| `.fence-title` | `flex:1`、`min-width:0`、单行省略号、600、`text-shadow: 0 1px 2px rgba(0,0,0,0.4)`；hover 原生 tooltip = 完整标题；加载失败时这里显示 `loadError` 文本（且不渲染右侧按钮） |
| `.fence-title-input` | `el-input size="small"`（24 高、白底、圆角 6、文字 `#606266`），`flex:1`，`maxlength=40`；输入框内允许选中文字 |
| `.fence-actions` | flex、不收缩、gap 2；`opacity: .55`，**鼠标在窗口内任意位置**（`.fence-window:hover`）或按钮聚焦时 `opacity: 1`，过渡 150ms；`mousedown` / `dblclick` 在此容器上 `stopPropagation`（不触发拖动 / 改名） |
| `.fence-btn` | 24×24、flex 居中、无边框透明、圆角 4、颜色 `rgba(255,255,255,0.85)`、`cursor:pointer`；hover 底 `rgba(255,255,255,0.14)`；`.active`（已固定）颜色 `#60A5FA`；图标 14px |
| `.fence-body` | `position:relative`、`flex:1`、`min-height:0`、padding 8、`overflow:hidden`、圆角 `0 0 8px 8px`；拖文件经过时（`.is-drag-hover`）`outline: 2px dashed rgba(255,255,255,0.45); outline-offset: -6px`（body 内侧约 4~6px 处的虚线框） |
| `.fence-empty` | flex column 居中、gap 8、高 100%、`rgba(255,255,255,0.55)`、居中对齐；`FolderOpened` 图标 28px + `把桌面图标拖到这里` |
| `.fence-panel` | `position:absolute; inset:0`（盖住整个 body 区域，含 padding）；flex column、gap 10、padding `10px 12px`、`overflow:auto`；背景 `rgba(15,23,42,0.92)` |

根元素 class：`is-collapsed`（= `!viewExpanded`）、`is-locked`、`is-drag-hover`。

### 3.4 标题栏交互

#### 3.4.1 移动窗口（超过 3px 才拖）

```text
titlebar.mousedown(e):
  if e.button != 0: return
  if 目标在 button / input / textarea / .el-input / .el-dropdown 内: return
  pressPoint = (e.screenX, e.screenY)                 // 屏幕 CSS 像素

titlebar.mousemove(e):
  if pressPoint 为空: return
  if e.buttons != 1: pressPoint = null; return        // 左键已松开
  if !exceedsDragThreshold(e.screenX - pressPoint.x, e.screenY - pressPoint.y): return   // |dx|>3 或 |dy|>3
  pressPoint = null
  if locked || editingTitle: return                    // 固定 / 改名中不移动
  geometryGate.arm(now)
  appWindow.startDragging()                            // 系统拖动循环（Win32：WM_NCLBUTTONDOWN + HTCAPTION）

titlebar.mouseup: pressPoint = null
```

为什么不在 mousedown 里直接拖：系统拖动循环会吞掉 mouseup，双击改名就收不到；也**不能**用 `data-tauri-drag-region`（双击会最大化，冲掉嵌入样式）。

拖动结束的落库：窗口 `onMoved` / `onResized` 事件 → `onGeometryEvent()`：

```text
onGeometryEvent():
  if !geometryGate.accept(now): return                 // 闸门没开（系统挪窗 / 程序调整）→ 忽略
  clearTimeout(commitTimer)
  commitTimer = setTimeout(commitGeometry, 300)

commitGeometry():
  commitTimer = undefined
  fence = invoke('commit_fence_geometry', { id, expandedView: viewExpanded })   // 后端读窗口当前外框落库
  失败: notifyError(e, '保存收纳箱位置失败')
  （落库后不关闸门：按住不放停顿后继续拖仍然算数；闸门在最后一次事件 3000ms 后自然失效）

flushGeometry():                                      // 任何程序调整（折叠/展开、patch、开始缩放）之前调用
  if commitTimer 存在: clearTimeout; await commitGeometry()
  geometryGate.disarm()
```

#### 3.4.2 双击标题改名

- `dblclick`（标题栏非按钮区域）→ `startRename()`：`titleDraft = fence.title`、`editingTitle = true`，下一帧聚焦并**全选**输入框
- 菜单「重命名」同样调用 `startRename()`
- 提交 / 取消：

| 触发 | 行为 |
|---|---|
| Enter（非输入法组合中） | `preventDefault` → `finishRename(true)` |
| Escape（非输入法组合中） | `preventDefault` → `finishRename(false)`（放弃） |
| 输入框 blur | `finishRename(true)` |
| 窗口失去焦点（`window.blur`，例如点到桌面） | 若在改名 → `finishRename(true)` |

输入法组合判断：`e.isComposing || e.keyCode === 229`。

```text
finishRename(commit):
  if !editingTitle: return
  editingTitle = false
  if !commit: return
  title = normalizeFenceTitle(titleDraft)            // §5.2
  if title == null（空 / 超 40 字符）或 title == fence.title: return   // 静默放弃，不提示
  result = invoke('rename_fence', { id, title })     // RenameFenceResult { fence, folderError }
  applyFence(result.fence)
  if result.folderError: ElMessage.warning(`标题已修改，但收纳文件夹没能跟着改名：${result.folderError}`)
  失败: notifyError(e, '重命名收纳箱失败')
```

#### 3.4.3 标题栏按钮

| 按钮 | tooltip（原生 title） | 图标 | 点击 |
|---|---|---|---|
| 折叠 / 展开 | 折叠态 `展开`，展开态 `折叠` | 折叠态 `ArrowDown`，展开态 `ArrowUp` | `patchFence({ collapsed: !collapsed }, '折叠收纳箱失败')`，然后移除 `hover` 展开原因（刚点折叠的收纳箱不因鼠标还在上面又弹开） |
| 固定 / 取消固定 | 已固定 `取消固定`，未固定 `固定位置和大小` | 已固定 `Lock`（`.active` 蓝色 `#60A5FA`），未固定 `Unlock` | `patchFence({ locked: !locked }, '固定收纳箱失败')` |
| 更多 | `更多` | `MoreFilled` | 打开 `el-dropdown` 菜单（见下） |

`patchFence(patch, failMsg)`：`flushGeometry()` → `update_fence { id, patch }` → `applyFence(返回值)`；失败 `notifyError(e, failMsg)`。

`applyFence(next)`：如果图标大小还在防抖未落库（`iconSizeTimer` 存在），保留本地 `iconSize`，其余字段用 `next`。

「固定」= 只锁定移动和缩放：仍可拖入文件、折叠、改名、打开项目。锁定不改任何窗口样式。

#### 3.4.4 「更多」菜单（`el-dropdown`）

- `trigger="click"`、`placement="bottom-end"`、`max-height=200`（超出滚动）、Element Plus 默认浅色弹层（白底，见 §1.3）
- 菜单项（逐字，带图标）：
  1. `重命名`（`EditPen`）→ `startRename()`
  2. `外观`（`Brush`）→ `panel = 'appearance'`
  3. `删除收纳箱`（`Delete`，`divided`：上方分隔线）→ `openDeletePanel()`
- `visible-change(true)` → 加 `menu` 展开原因（折叠态下窗口先展开，菜单才有地方弹出）；`visible-change(false)` → **下一帧**再移除 `menu` 原因（让菜单项打开的面板先接管 `panel` 原因，避免收回又展开）

### 3.5 折叠 / 临时展开状态机

持久化折叠态 `fence.collapsed`（`update_fence` 写库）与窗口实际形态分离；窗口形态只由 `viewExpanded` 决定：

```text
viewExpanded = !fence.collapsed || expandReasons 非空
expandReasons ⊆ { hover, drag, menu, panel, itemMenu }

watch(viewExpanded → expanded):
  串行队列（viewQueue）里追加：
    await flushGeometry()                               // 刚拖完未落库的位置先落库：后端按库里的几何算形态
    invoke('set_fence_view', { id, expanded })          // 不写库；失败只打日志
```

| 原因 | 加入 | 移除 |
|---|---|---|
| `hover` | 根元素 `mouseenter`：清掉 hoverTimer；**仅持久化折叠态**时，150ms 后若拖动闸门未打开（`!geometryGate.isArmed(now)`）则加入 | `mouseleave`：清掉 hoverTimer；若有 hover 原因，400ms 后 `releaseHover()`：此时闸门仍打开（正在拖窗口）则再等 400ms 重试，否则移除。另：点击折叠/展开按钮后立即移除 |
| `drag` | 外部文件拖拽 `enter` / `over`：清掉 dragTimer、`dragHover = true`、加入 | `leave` / `drop`：`dragHover = false`；`drop` 时执行移入；300ms 后移除 |
| `menu` | 标题栏菜单打开 | 菜单关闭的下一帧 |
| `panel` | `panel !== 'none'` | `panel === 'none'`（同时若离开外观面板则清掉 `alphaPreview`） |
| `itemMenu` | 网格右键菜单打开（子组件 `menu-visible(true)`） | 右键菜单关闭 |

展开态下这些原因不影响窗口（`viewExpanded` 已为 true）。临时展开时没有缩放手柄。

### 3.6 缩放（窗口内 8 个手柄）

窗口是 `resizable(false)`，系统缩放循环无效，所以完全由前端手柄 + `set_fence_bounds` 实现。只在 `canResize` 时渲染手柄。

手柄几何（相对 `.fence-window`，`position:absolute; z-index:10`，透明不可见，只有光标变化）：

| 方向 | 位置 / 尺寸 | 光标 |
|---|---|---|
| `n` | `top:0; left:10px; right:10px; height:5px` | `ns-resize` |
| `s` | `bottom:0; left:10px; right:10px; height:5px` | `ns-resize` |
| `w` | `left:0; top:10px; bottom:10px; width:5px` | `ew-resize` |
| `e` | `right:0; top:10px; bottom:10px; width:5px` | `ew-resize` |
| `nw` | `top:0; left:0; 10×10` | `nwse-resize` |
| `se` | `right:0; bottom:0; 10×10` | `nwse-resize` |
| `ne` | `top:0; right:0; 10×10` | `nesw-resize` |
| `sw` | `bottom:0; left:0; 10×10` | `nesw-resize` |

注意 `n` 手柄与标题栏重叠（标题栏上沿 5px 是缩放区）。

流程：

```text
pointerdown(direction, e):
  if e.button != 0 || !canResize || resizing: return
  preventDefault; handle.setPointerCapture(e.pointerId)       // 拖出窗口仍收到事件
  记 pointer = (id, screenX, screenY)
  resizePressed = true
  await flushGeometry()
  (position, size) = await (outerPosition(), outerSize())      // 物理像素
  if !resizePressed: return                                     // 异步期间已松开 → 不进入缩放
  resizing = { direction, pointerId, startX, startY, start: {x,y,width,height}, dpr: devicePixelRatio || 1 }

pointermove(e):
  if !resizing || e.pointerId != resizing.pointerId || 左键未按下: return
  pendingBounds = resizeRect(start, direction,
                             (e.screenX - startX) * dpr, (e.screenY - startY) * dpr,
                             round(160 * dpr), round(160 * dpr))           // §5.3
  若本帧尚未排队: requestAnimationFrame(flushBounds)                         // 每帧最多一次

flushBounds():
  bounds = pendingBounds; pendingBounds = null
  boundsInFlight = boundsInFlight.then(() => invoke('set_fence_bounds', { id, x, y, width, height }))  // 串行；失败只打日志

pointerup / pointercancel(e):
  resizePressed = false
  if !resizing || pointerId 不符: return
  resizing = null; releasePointerCapture
  若有未执行的帧: cancelAnimationFrame + 立即 flushBounds()
  await boundsInFlight
  await commitGeometry()                                        // 直接落库，不经闸门
```

缩放期间窗口 `onResized` 事件因闸门已被 `flushGeometry` 关闭而被忽略。

### 3.7 外观面板（`panel === 'appearance'`）

```
┌ .fence-panel ───────────────────────────┐
│ 外观                               [×]  │  .panel-header：flex space-between，600
│ 背景色                                   │  .panel-label：12px rgba(255,255,255,.7)
│ ● ● ● ● ● ● ◐                            │  .color-swatches：flex wrap gap 6
│ 透明度                                   │
│ ──────●────────────────────   45%        │  .alpha-slider：flex gap 10 padding 0 4px
└─────────────────────────────────────────┘
```

| 部位 | 规格 |
|---|---|
| 关闭按钮 | `.fence-btn`，tooltip `关闭`，`Close` 图标 14px → `panel = 'none'` |
| `.panel-row` | flex column、gap 6 |
| 预设色圆点 `.color-swatch` | 22×22、圆形、`border: 2px solid rgba(255,255,255,0.25)`、背景 = 该色；当前色（`isSameColor`）边框 `#FFF`；tooltip = 预设名；顺序与设置窗口相同：`黑色 #000000`、`深灰 #1E293B`、`深蓝 #0F2942`、`深紫 #2E1065`、`深绿 #052E24`、`深棕 #2B1A0F` |
| 自定义色 `.color-swatch.custom` | 同尺寸圆点，tooltip `自定义颜色`；背景色 = 当前 `bgColor`，叠加 `conic-gradient(#ef4444, #f59e0b, #10b981, #3b82f6, #8b5cf6, #ef4444)` 并 `background-blend-mode: soft-light`（彩虹色环与当前色混合）；内部一个铺满且透明的原生 `<input type="color" :value="bgColor">`，点击弹出系统取色对话框，`change` 后 `setBgColor(value)` |
| `setBgColor(color)` | 与当前色相同（不区分大小写）则忽略；否则 `patchFence({ bgColor: color }, '设置收纳箱背景色失败')`（后端只收 `#RRGGBB`，统一存大写） |
| 透明度滑块 | `el-slider size="small"`、min **5**、max **100**、step **5**、**无 tooltip**；值 = `round(bgAlpha*100)`；`@input` → `alphaPreview = v/100`（窗口背景实时预览，不写库）；`@change` → `patchFence({ bgAlpha: v/100 }, '设置收纳箱透明度失败')` 后 `alphaPreview = null` |
| 数值 `.alpha-value` | 宽 36、12px、`rgba(255,255,255,0.8)`、右对齐：`${round(bgAlpha*100)}%` |

注意面板内的 Element Plus 组件（滑块、按钮）保持 Element Plus 默认浅色配色（主色 `#409EFF`），不随深色面板变化。

### 3.8 删除确认面板（`panel === 'delete'`）

打开（菜单「删除收纳箱」→ `openDeletePanel()`）：`panel = 'delete'`、`deleteCount = null` → `refreshItems()`（重新列举）→ `deleteCount = items.length`（以文件夹实际列举为准，隐藏文件不计入）。

| 部位 | 规格 |
|---|---|
| `.delete-text`（flex:1） | 标题 `.delete-title`：`删除收纳箱「${fence.title}」？`（600、`margin-bottom:6px`、`word-break: break-all`）；说明 `.delete-desc`（12px、`rgba(255,255,255,0.7)`） |
| 说明文字 `deleteHint` | `deleteCount === null` → `正在统计其中的项目…`；`=== 0` → `收纳箱是空的，收纳文件夹会一并删除`；否则 `其中的 ${n} 个项目会移回桌面` |
| `.panel-actions` | flex、gap 8、右对齐：`el-button size="small"`「取消」（删除进行中禁用）→ `panel='none'`；`el-button size="small" type="danger"`「删除」（`loading = deleting`；统计中 `deleteCount === null` 时禁用）→ `confirmDelete()` |

```text
confirmDelete():
  if deleting: return
  deleting = true
  result = invoke('delete_fence', { id })            // DeleteFenceResult { deleted, restored, remaining, failures }
  if result.deleted:
    // 后端已删记录并 destroy 本窗口，前端什么都不做
  else:
    panel = 'none'
    ElMessage({ type:'warning', message: describeDeleteKept(result), duration: 8000, showClose: true })   // §5.5
    refreshItems()
  失败（Err）: notifyError(e, '删除收纳箱失败')        // 例：「收纳箱里还有文件正在移动，请稍后再删除」
  finally deleting = false
```

删除过程中可能弹出 Windows 的 UAC / "文件正在使用"对话框（以收纳箱窗口为父窗口）。

### 3.9 OneDrive 提示面板（`panel === 'onedrive'`）

拖入结果 `onedriveNotice === true` 时打开（整个设备只会出现一次）。

| 部位 | 内容 |
|---|---|
| 标题 `.delete-title.notice-title`（flex、gap 6、居中对齐、600） | `InfoFilled` 图标 16px + `桌面由 OneDrive 同步` |
| 说明 `.delete-desc` | `收进收纳箱的项目会从桌面移走，移出桌面会同步到你的其他设备（它们的桌面上也会少掉这些图标）。 项目仍保存在收纳文件夹里，移回桌面即可恢复。`（模板里两句之间换行 → 渲染为**一个空格**） |
| 按钮 | 右对齐 `el-button size="small" type="primary"`「知道了」→ `panel = 'none'` |

### 3.10 外部拖入（从资源管理器 / 桌面拖文件进来）

监听 `getCurrentWebview().onDragDropEvent`（OLE 文件拖放），**锁定状态下照样可以拖入**：

| 事件 | 行为 |
|---|---|
| `enter` / `over` | 清 dragTimer；`dragHover = true`（body 虚线框）；加 `drag` 展开原因 |
| `leave` | `dragHover = false`；300ms 后移除 `drag` 原因 |
| `drop`（payload `paths: string[]`） | `dragHover = false`；`moveIn(paths)`；300ms 后移除 `drag` 原因 |

整个窗口（含标题栏）都是放置区；没有"不接受"状态。

```text
moveIn(paths):
  if paths 为空: return
  result = invoke('fence_move_in', { id, paths })     // FenceMoveInResult { results（与 paths 一一对应）, onedriveNotice }
  notice = describeMoveIn(summarizeOpResults(result.results))     // §5.5
  if notice: ElMessage({ type: notice.type, message: notice.message, duration: 5000 })
  if result.onedriveNotice: panel = 'onedrive'
  失败: notifyError(e, '收纳失败')
  最后总是 refreshItems()
```

全部成功（或只有被跳过的项，如 `desktop.ini`、隐藏文件、已在本收纳箱）时**不提示**。

### 3.11 内容列表刷新

```text
refreshItems():                                      // 列举 + 对账
  seq = ++itemsSeq
  list = invoke('get_fence_items', { id })           // 后端按收纳文件夹对账后按 sort_order 返回
  if seq == itemsSeq: items = list                   // 只采用最后一次发起的
  失败只打日志（不弹提示；删除中也会走到这里）

setItems(list):                                      // 后端推送 / 子组件排序后
  itemsSeq += 1; items = list
```

- 启动：`get_fence(id)` 成功后 `refreshItems()`
- 后端推送：`appWindow.listen('fence-items-changed', e => setItems(e.payload))`（`emit_to('fence-{id}')`，只发给本窗口；触发点：目录监听 300ms 去抖、跨箱移动后的来源/目标、删除中止后）
- 子组件 `@changed` → `refreshItems()`；`@update:items` → `setItems()`

### 3.12 图标大小（Ctrl+滚轮）

子组件发 `update:iconSize(size)` → `fence.iconSize = size`（立即重排）→ 400ms 防抖 → `update_fence { id, patch: { iconSize } }`（**不采用返回值**，期间可能又滚了几格）；失败 `notifyError(e, '保存图标大小失败')`。卸载时若还有未落库的值，立即保存一次。

### 3.13 键盘与生命周期

| 时机 | 行为 |
|---|---|
| `onMounted` | `parseFenceId(route.query.id)` 为 null → `loadError = '收纳箱参数无效'`；`get_fence` 失败 → `loadError = errorMessage(e) \|\| '读取收纳箱失败'`（标题位置显示该文本，无按钮、无 body 内容）；成功 → 刷新列表，注册 `keydown`、`blur`、`fence-items-changed`、`onMoved`、`onResized`、`onDragDropEvent` |
| `keydown` Escape（非输入法、非删除中） | `panel !== 'none'` → 关闭面板（网格右键菜单打开时它先拦截 Esc，见 §4.5） |
| `window.blur` | 改名中 → 提交改名；（网格同时关闭右键菜单） |
| `onUnmounted` | 取消全部监听与计时器；有未落库的图标大小则立即保存；取消未执行的缩放帧 |

### 3.14 收纳箱窗口文案汇总（逐字）

| 场景 | 文案 | 形式 |
|---|---|---|
| 空态 | `把桌面图标拖到这里` | 文本 |
| tooltip | `展开` / `折叠` / `取消固定` / `固定位置和大小` / `更多` / `关闭` / `自定义颜色` / 预设色名 | 原生 title |
| 菜单 | `重命名` / `外观` / `删除收纳箱` | dropdown |
| 外观面板 | `外观` / `背景色` / `透明度` / `${n}%` | 文本 |
| 删除面板 | `删除收纳箱「${title}」？` / `正在统计其中的项目…` / `收纳箱是空的，收纳文件夹会一并删除` / `其中的 ${n} 个项目会移回桌面` / `取消` / `删除` | 文本 / 按钮 |
| OneDrive | `桌面由 OneDrive 同步` / 说明见 §3.9 / `知道了` | 文本 / 按钮 |
| 加载错误 | `收纳箱参数无效` / `读取收纳箱失败`（或后端错误原文，如 `收纳箱 #3 不存在`） | 标题位置 |
| 失败提示（notifyError 前缀） | `折叠收纳箱失败` / `固定收纳箱失败` / `设置收纳箱背景色失败` / `设置收纳箱透明度失败` / `保存收纳箱位置失败` / `重命名收纳箱失败` / `删除收纳箱失败` / `收纳失败` / `保存图标大小失败` | error toast |
| 改名警告 | `标题已修改，但收纳文件夹没能跟着改名：${folderError}` | warning toast |

---

## 4. 项目网格（FenceItemGrid）

Props：`fenceId`、`items: FenceEntry[]`、`iconSize`（CSS 像素）。Emits：`update:items(list)`、`update:iconSize(size)`、`changed`（请父组件重新列举）、`menu-visible(bool)`。

`FenceEntry`：`fileName`（文件夹内实际文件名，唯一键）、`displayName`（`.lnk` / `.url` 去后缀；文件夹原样）、`path`（完整路径）、`isDir`、`isShortcut`、`modifiedMs: number|null`。

### 4.1 布局

| 元素 | 样式 |
|---|---|
| 滚动容器 `.fence-grid-scroll` | 高 100%；`overflow-x:hidden; overflow-y:auto`；细滚动条（`scrollbar-width: thin`，滑块 `rgba(255,255,255,0.25)`、轨道透明）；点击空白处 → 取消选中；滚轮事件见 §4.7 |
| 网格 `.fence-grid` | CSS grid：`grid-template-columns: repeat(auto-fill, cellWidth)`，**cellWidth = iconSize + 40**；`gap: 2px 0`（行间 2px，列间 0）；`align-content:start; justify-content:start`（从左上排起，右侧剩余空白不拉伸）；`min-height:100%` |
| 单元 `.fence-item` | `position:relative`；flex column、`align-items:center`、gap 4；padding `6px 4px 4px`；圆角 6；`cursor:default`；背景/阴影过渡 0.15s |
| 图标框 `.fence-item-icon` | `iconSize × iconSize`，flex 居中，不收缩 |
| 图标图片 | 100%×100%，`object-fit:contain`，`filter: drop-shadow(0 1px 2px rgba(0,0,0,0.35))`，不可拖 |
| 名称 `.fence-item-name` | 宽 100%；12px；行高 16；居中；`text-shadow: 0 1px 2px rgba(0,0,0,0.65)`；`word-break: break-all`（任意字符处断行）；**最多 2 行**，超出省略号（`-webkit-line-clamp: 2`） |

列数 = `floor(可用宽度 / (iconSize+40))`，可用宽度 = 窗口宽 − 2（根边框）− 16（body padding）−（出现纵向滚动条时的滚动条宽）。

不同图标大小的单元尺寸（单元高 = 6 + iconSize + 4 + 名称高 + 4，名称 1 行 16 / 2 行 32）：

| iconSize | 单元宽 | 单元高（1 行名 / 2 行名） |
|---|---|---|
| 24（最小） | 64 | 54 / 70 |
| 48（默认） | 88 | 78 / 94 |
| 64 | 104 | 94 / 110 |
| 96 | 136 | 126 / 142 |
| 128（最大） | 168 | 158 / 174 |

（步长 8：24, 32, 40, …, 128。）

### 4.2 单元状态样式

| 状态 | 样式 |
|---|---|
| hover | 底 `rgba(255,255,255,0.12)` |
| 选中 `.is-selected`（单选，`selected = fileName`） | 底 `rgba(96,165,250,0.32)` + 内描边 `inset 0 0 0 1px rgba(147,197,253,0.65)` |
| 拖动排序占位 `.is-ghost` | `opacity .35` |
| 图标加载中 | 图标框为空（不显示任何东西） |
| 图标加载失败 | 占位图标：文件夹 `Folder`、其他 `Document`；字号 = `iconSize × 0.72`；颜色 `rgba(255,255,255,0.75)` |
| 正在打开 `.is-launching` | 图标（或占位）`opacity .45`；图标框上叠一个居中的旋转 `Loading` 图标（字号 `iconSize × 0.45`、白色）；tooltip 变为 `正在打开「${displayName}」…` |
| tooltip（原生 title） | 平时 = `displayName` |

### 4.3 图标懒加载

| 常量 | 值 |
|---|---|
| `MAX_ICON_REQUESTS` | 3（同时在途的请求数；后端图标在单个 STA 线程排队） |
| `ICON_PRELOAD_MARGIN` | 64px（可见区域上下外扩，滚动时提前加载） |

```text
dpr    = window.devicePixelRatio || 1             // 窗口 resize（跨显示器缩放变化）时更新
bucket = iconBucket(iconRequestPx(iconSize, dpr)) // §5.6：ceil(iconSize*dpr) 向上取整到档位
key(item) = `${item.path}|${item.modifiedMs ?? ''}|${bucket}`    // 路径 / 修改时间 / 档位任一变化都会重新请求
icons: Map<key, loading | ok(url) | failed>
requested: Set<key>                               // 同一个 key 只请求一次（失败也不重试）

可见性：IntersectionObserver(root = 滚动容器, rootMargin = 64px) 观察每个单元（data-file = fileName）
每当 列表 / 可见集合 / bucket 变化：对可见且未请求过的项 requestIcon(item)

requestIcon(item): requested.add(key); icons[key] = loading; 入队 {key, fileName, sizePx: bucket}; pump()
pump(): while 在途 < 3 且队列非空:
  invoke('fence_item_icon', { id: fenceId, fileName, sizePx })  → 返回缓存 PNG 绝对路径
    ok   → icons[key] = ok(convertFileSrc(path))
    fail → icons[key] = failed（console.warn）
    finally → 在途 -1; pump()
<img> onerror（PNG 被清理等）→ icons[key] = failed
```

后端缓存：`<数据目录>\fence-icons\{路径哈希 FNV-1a 64}-{mtime 毫秒}-{大小}-{档位}.png`；PNG 为正方形，边长通常等于档位（小图标程序按大档位请求时可能返回 48×48，前端一律拉伸到 `iconSize`）；`.lnk` 取目标图标、无快捷方式箭头；图片 / 视频显示类型图标而非缩略图。

### 4.4 选择与打开

| 操作 | 行为 |
|---|---|
| 单击项 | 关闭右键菜单；`selected = fileName`（事件不冒泡到容器） |
| 单击空白 | `selected = null` |
| 双击项 | 关闭右键菜单；选中；若该项已在打开中则忽略；`launching.add` → `fence_open_item { id, fileName }`（`ShellExecuteExW`；失效快捷方式约 3 秒后才返回错误）→ 失败 `notifyError(e, `打开「${displayName}」失败`)` → finally `launching.delete` |
| 列表变化 | 选中项已不在列表 → 取消选中；右键菜单对应项已不在 → 关闭菜单；**拖动排序进行中不跟随 props 更新** |

### 4.5 自绘右键菜单

右键单元（`contextmenu`，阻止默认、不冒泡）→ `openMenu(e, item)`：

```text
selected = item.fileName
targets = invoke('list_fences') 去掉当前收纳箱（按 sort_order, id 排序）；失败则 []
menu = { x: e.clientX, y: e.clientY, item, page: 'main', targets }
若之前没有菜单: emit('menu-visible', true)       // 父组件加 itemMenu 展开原因
下一帧 clampMenu()

clampMenu():                                        // 收纳箱窗口很小，靠右/靠下时往回挪
  rect = 菜单实际尺寸
  x = max(4, min(x, 窗口宽 - rect.width - 4))
  y = max(4, min(y, 窗口高 - rect.height - 4))
```

菜单容器 `.fence-item-menu`：`position:fixed`（相对窗口视口）、`z-index:100`、flex column、`min-width:150px`、`max-width: 100vw − 8px`、`max-height: 100vh − 8px`（超出纵向滚动）、padding 4、背景 `rgba(15,23,42,0.96)`、`1px solid rgba(255,255,255,0.12)`、圆角 6、阴影 `0 6px 18px rgba(0,0,0,0.45)`。菜单内点击 / 右键不冒泡。

菜单项 `.menu-item`：flex、不收缩、gap 8、`align-items:center`、**高 28**、padding `0 8px`、12px、`rgba(255,255,255,0.9)`、左对齐、不换行、无边框透明、圆角 4；hover（未禁用）底 `rgba(255,255,255,0.12)`；禁用 `opacity .5`、`cursor:default`；图标 12px（跟随字号）。

**主页（`page === 'main'`）**：

| 项 | 图标 | 行为 |
|---|---|---|
| `打开` | `TopRight` | 同双击 `openItem` |
| `打开所在位置` | `FolderOpened` | 关菜单 → `fence_reveal_item { id, fileName }`（资源管理器打开收纳文件夹并选中该项）→ 失败 `notifyError(e, '打开所在位置失败')` |
| `移回桌面` | `Back` | 关菜单 → `fence_restore_item { id, fileName }` → 见下方结果处理（action = `移回桌面`）；成功且无附注且有 `destPath` → `ElMessage.success(`已移回 ${parentDir(destPath)}`)`；然后 `dropLocally` |
| `移动到…` | `Rank` | 有其他收纳箱：右侧（`margin-left:auto`）`Right` 箭头图标，点击进入"移动到"页（下一帧 clamp）；没有其他收纳箱：**禁用**，tooltip `没有其他收纳箱`，右侧显示小字 `没有其他收纳箱`（11px、`rgba(255,255,255,0.6)`、`margin-left:auto`） |

**"移动到"页（`page === 'move'`）**：

1. 返回项：`ArrowLeft` 图标 + `移动到` → 回到主页（下一帧 clamp）
2. 分隔线 `.menu-divider`：高 1px、`margin: 3px 4px`、`rgba(255,255,255,0.1)`
3. 每个目标收纳箱一项：只显示标题（`.menu-target`：单行省略号；tooltip = 完整标题），点击 → `moveItemTo(item, target)`：关菜单 → `fence_move_item { id, fileName, targetId }` → 结果处理（action = `移动到「${target.title}」`）→ 成功 `dropLocally`；异常 `notifyError(e, '移动失败')`

**关闭菜单的时机**：点菜单以外的任意位置（document 捕获阶段 `pointerdown`）、窗口失焦、Esc（窗口捕获阶段，`stopPropagation`，所以这次 Esc 不会再关父组件的面板）、不按 Ctrl 的滚轮、开始拖动排序、单击 / 双击任一项、执行任一菜单动作、菜单对应项从列表消失、组件卸载。关闭时 `emit('menu-visible', false)`。

**逐项结果处理**（`reportResult(result, action)`，返回是否已移走）：

```text
status == 'moved':     if result.error: ElMessage.warning(result.error)   // 已移动但带附注
                       return true
status == 'cancelled': ElMessage.info(`已取消${action}`)                  // 例：已取消移回桌面
status == 'failed':    notifyError(result.error ?? '', `${action}失败`)   // 例：移回桌面失败：正被其他程序占用
status == 'skipped':   不提示
return false

dropLocally(item):    // 不等目录监听事件，先从本地列表去掉
  list = list 去掉该 fileName; emit('update:items', list); emit('changed')
```

### 4.6 箱内拖动排序

- 实现：vuedraggable（SortableJS），`force-fallback = true`（不用 HTML5 原生拖放，因为 Windows 上它与原生文件拖放互斥）：按下项后移动超过 **4px**（`fallback-tolerance`）才开始拖；一个跟随鼠标的克隆体随指针移动；原位置/目标位置的元素以 `.is-ghost`（`opacity .35`）显示；其余项 **150ms** 动画让位（`animation`）
- `@start`：`dragging = true`（期间不跟随 props）、关闭右键菜单
- `@end`：

```text
dragging = false
names = list.map(fileName)
if names 与 props.items 的顺序相同: return
emit('update:items', list)
invoke('reorder_fence_items', { id, fileNames: names })    // 新的完整顺序；失败 notifyError(e, '保存排序失败')
emit('changed')                                            // 补一次列举（拖动期间被跳过的推送）
```

- **拖出**：网格**不支持**把项目拖出窗口（拖到桌面 / 另一个收纳箱）——拖动只在本箱内排序。跨箱只能用右键「移动到…」；移回桌面用「移回桌面」。（后端 `fence_move_in` 能识别"从另一个收纳箱文件夹拖来的路径"并按跨箱处理，但那条路径来自资源管理器里打开的收纳文件夹，不是网格拖动。）

### 4.7 Ctrl+滚轮调整图标大小

| 常量 | 值 |
|---|---|
| `WHEEL_STEP_INTERVAL` | 40ms（触控板一次手势连发很多事件，节流） |
| 步长 / 上下限 | 8 / 24 ~ 128（§5.1） |

```text
wheel(e) on 滚动容器:
  if !e.ctrlKey: closeMenu(); return               // 普通滚动照常滚动
  preventDefault
  if now - lastWheelAt < 40ms: return
  lastWheelAt = now
  next = nextIconSize(iconSize, e.deltaY)          // 上滚放大一档、下滚缩小一档
  if next != iconSize: emit('update:iconSize', next)
```

---

## 5. 常量与纯函数（`types/fence.ts`、`utils/fence.ts` 等，Rust 端 1:1 移植）

### 5.1 常量

| 常量 | 值 | 后端对应 |
|---|---|---|
| `FENCE_TITLE_BAR_HEIGHT` | 32 | `FENCE_TITLE_BAR_DIP = 32.0` |
| `FENCE_MIN_WIDTH` | 160 | `MIN_WIDTH_DIP = 160.0` |
| `FENCE_MIN_EXPANDED_HEIGHT` | 160 | `MIN_EXPANDED_HEIGHT_DIP = 160.0` |
| `FENCE_TITLE_MAX_LENGTH` | 40（字符数，按 Unicode 码点） | `MAX_TITLE_CHARS = 40` |
| `FENCE_BG_ALPHA_MIN_PERCENT` / `MAX` | 5 / 100 | — |
| `FENCE_ICON_SIZE_MIN` / `MAX` | 24 / 128 | `MIN_ICON_SIZE` / `MAX_ICON_SIZE` |
| `FENCE_ICON_SIZE_STEP` | 8 | — |
| `FENCE_ICON_BUCKETS` | `[16, 24, 32, 48, 64, 96, 128, 256]` | `fence_fs::ICON_SIZE_BUCKETS` |
| `FENCE_ITEMS_CHANGED_EVENT` | `'fence-items-changed'` | `FENCE_ITEMS_CHANGED_EVENT` |
| `PRESET_BG_COLORS` | 黑色 `#000000`、深灰 `#1E293B`、深蓝 `#0F2942`、深紫 `#2E1065`、深绿 `#052E24`、深棕 `#2B1A0F` | — |
| `DEFAULT_BG_COLOR` / `DEFAULT_BG_ALPHA` | `#000000` / 0.45 | `DEFAULT_WINDOW_BG_*`、`DEFAULT_FENCE_BG_ALPHA` |
| 后端：`CELL_WIDTH_DIP` / `CELL_HEIGHT_DIP` | 88 / 96 | 新建尺寸用 |
| 后端：`DEFAULT_COLUMNS` / `DEFAULT_ROWS` / `CONTENT_PADDING_DIP` | 4 / 3 / 8 | 新建尺寸用 |
| 后端：`SCREEN_MARGIN_DIP` / `LAYOUT_GAP_DIP` / `CASCADE_STEP_DIP` | 24 / 16 / 32 | 新建 / 重置位置 |

### 5.2 标题与 id

```rust
/// 路由 query 的 id：只接受纯数字串表示的正安全整数（GPUI 里 id 直接传入，可不移植）
fn parse_fence_id(raw: &str) -> Option<i64> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) { return None; }
    let id: i64 = raw.parse().ok()?;               // JS: Number.isSafeInteger(id) && id > 0
    (id > 0 && id <= 9_007_199_254_740_991).then_some(id)
}

/// 与后端 normalize_title 同规则：控制字符（U+0000–U+001F、U+007F–U+009F）换成空格 → trim；
/// 为空或超过 40 个字符返回 None（前端静默不提交；后端报错「收纳箱标题不能为空」/「收纳箱标题不能超过 40 个字符」）
fn normalize_fence_title(raw: &str) -> Option<String> {
    let s: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = s.trim();
    if t.is_empty() || t.chars().count() > 40 { None } else { Some(t.to_string()) }
}
```

测试向量：`'12'→12`、`['7','8']→7`；`undefined, null, '', '0', '-1', '1.5', 'abc', '12abc', ' 3', 数字 3, '99999999999999999999'` → None。`'  工作  '→'工作'`、`'a\nb\tc'→'a b c'`、`'   '`/`'\n\t'`→None、40 个「字」通过、41 个→None。

### 5.3 拖动阈值、几何闸门、缩放矩形

```rust
/// 按下后移动超过阈值（CSS 像素，默认 3）才算拖动
fn exceeds_drag_threshold(dx: f64, dy: f64, threshold: f64 /* = 3.0 */) -> bool {
    dx.abs() > threshold || dy.abs() > threshold
}

/// 判定"用户拖动 / 缩放结束"的闸门（时间单位 ms）
struct GeometryCommitGate { window_ms: f64, armed_until: f64 /* 初始 -∞ */ }
impl GeometryCommitGate {
    fn arm(&mut self, now: f64)            { self.armed_until = now + self.window_ms; }
    /// 移动/缩放事件：闸门开着就续期并返回 true（应安排防抖落库），否则 false
    fn accept(&mut self, now: f64) -> bool {
        if now > self.armed_until { return false; }
        self.armed_until = now + self.window_ms; true
    }
    fn is_armed(&self, now: f64) -> bool   { now <= self.armed_until }
    fn disarm(&mut self)                   { self.armed_until = f64::NEG_INFINITY; }
}

enum Dir { N, S, W, E, NW, NE, SW, SE }
struct PxRect { x: i32, y: i32, width: i32, height: i32 }   // 物理像素

/// 拖哪条边改哪条边，对边不动；不小于最小尺寸（拖左/上边缩到最小就停，右下角不被推走）
fn resize_rect(start: PxRect, dir: Dir, dx: f64, dy: f64, min_w: f64, min_h: f64) -> PxRect {
    let (mut x, mut y, mut w, mut h) = (start.x as f64, start.y as f64, start.width as f64, start.height as f64);
    if dir 含 'e' { w = (start.width as f64 + dx).max(min_w); }
    if dir 含 's' { h = (start.height as f64 + dy).max(min_h); }
    if dir 含 'w' { w = (start.width as f64 - dx).max(min_w); x = start.x as f64 + start.width as f64 - w; }
    if dir 含 'n' { h = (start.height as f64 - dy).max(min_h); y = start.y as f64 + start.height as f64 - h; }
    PxRect { x: js_round(x), y: js_round(y), width: js_round(w), height: js_round(h) }
}
// js_round = JS Math.round：逢 .5 向 +∞（Rust 需用 (v + 0.5).floor()，不要用 f64::round）
// 调用：dx/dy = (指针屏幕位移 CSS px) × dpr；min_w = min_h = round(160 × dpr)
```

测试向量（`start = {x:100, y:200, w:552, h:504}`，min 240×240）：

| dir | dx | dy | 结果 |
|---|---|---|---|
| se | 30 | 40 | {100, 200, 582, 544} |
| e | 30 | 40 | {100, 200, 582, 504} |
| s | 30 | 40 | {100, 200, 552, 544} |
| nw | -20 | -10 | {80, 190, 572, 514} |
| ne | 20 | 10 | {100, 210, 572, 494} |
| sw | 20 | 10 | {120, 200, 532, 514} |
| se | -1000 | -1000 | {100, 200, 240, 240} |
| nw | 1000 | 1000 | {412, 464, 240, 240} |
| e | 10.4 | 0 | width 562 |

闸门向量（window 1000）：未 arm 时 `accept(0)=false`、`isArmed(0)=false`；`arm(0)` 后 `accept(900)=true`、`accept(1800)=true`、`isArmed(2700)=true`、`accept(2900)=false`；`arm(0)` 后 `accept(1001)=false`；`arm(2000); disarm()` 后 `accept(2001)=false`。阈值向量：(0,0)/(3,-3)→false，(4,0)/(0,-4)→true。

### 5.4 屏幕组合 ID（设置窗口"当前配置"与收纳箱布局共用）

```text
monitors = 全部显示器（物理像素宽, 高, 缩放比例），按系统枚举顺序
若为空 → 'unknown'（读取失败也是 'unknown'）
infos = monitors.map(m => (w, h, round(scale × 100)))            // JS Math.round
按 w×h 降序**稳定**排序（同面积保持枚举顺序）
→ `${monitors.len}_` + infos.map(`${w}x${h}@${scalePct}`).join('_')
例：'2_2560x1440@125_1920x1080@100'
```

（Rust 已有 `commands/fence.rs::screen_config_id_of`，与前端 `appStore.generateScreenConfigId()` 逐字一致。）

### 5.5 逐项结果汇总与提示文案

```rust
/// 路径最后一段（\ 与 / 都认；先去掉末尾分隔符）
fn base_name(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rfind(['\\', '/']) { Some(i) => t[i+1..].into(), None => t.into() }
}
/// 上级目录（分隔符位置 > 0 才截断，否则原样返回去尾分隔符后的字符串）
fn parent_dir(path: &str) -> String {
    let t = path.trim_end_matches(['\\', '/']);
    match t.rfind(['\\', '/']) { Some(i) if i > 0 => t[..i].into(), _ => t.into() }
}

struct OpSummary { moved: usize, cancelled: usize, failed: Vec<(String /*name*/, String /*reason*/)>, notes: Vec<(String, String)> }

fn summarize_op_results(results: &[ItemOpResult]) -> OpSummary {
    // 对每项：name = base_name(source)
    // moved     → moved += 1；若 error 非空 → notes.push((name, error))
    // cancelled → cancelled += 1
    // failed    → failed.push((name, error 非空 ? error : "未知原因"))
    // skipped   → 不计
}

const MAX_LISTED_ITEMS: usize = 2;

/// 拖入结果提示；None = 不提示
fn describe_move_in(s: &OpSummary) -> Option<(Kind /*Error|Warning|Info*/, String)> {
    let mut parts = vec![];
    if !s.failed.is_empty() {
        let listed = s.failed[..min(2)].map(|(n, r)| format!("{n}（{r}）")).join("、");
        let more = if s.failed.len() > 2 { " 等" } else { "" };          // 注意 " 等" 前有半角空格
        parts.push(format!("{} 项未能收纳：{listed}{more}", s.failed.len()));
    }
    if s.cancelled > 0 { parts.push(format!("已取消 {} 项", s.cancelled)); }
    if let Some((n, note)) = s.notes.first() { parts.push(format!("{n}：{note}")); }   // 只列第一条附注
    if parts.is_empty() { return None; }
    if s.moved > 0 { parts.insert(0, format!("已收纳 {} 项", s.moved)); }
    let kind = if !s.failed.is_empty() { if s.moved > 0 { Warning } else { Error } }
               else if s.notes.is_empty() { Info } else { Warning };
    Some((kind, parts.join("；")))                                       // 全角分号
}

/// 删除中止（收纳箱保留）提示
fn describe_delete_kept(r: &DeleteFenceResult) -> String {
    // reasons: base_name(failure.source).to_lowercase() → failure.error.unwrap_or("")
    let names: Vec<String> = if !r.remaining.is_empty() { r.remaining.clone() }
                             else { r.failures.map(|f| base_name(&f.source)) };
    if names.is_empty() {
        let restored = if r.restored > 0 { format!("{} 个项目已移回桌面，但", r.restored) } else { "".into() };
        return format!("{restored}收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除");
    }
    let listed = names[..min(2)].map(|n| match reasons.get(n.to_lowercase()) {
        Some(reason) if !reason.is_empty() => format!("{n}（{reason}）"), _ => n.clone() }).join("、");
    let more = if names.len() > 2 { " 等" } else { "" };
    let head = format!("{} 个项目未能移回桌面，收纳箱已保留", names.len());
    if listed.is_empty() { head } else { format!("{head}：{listed}{more}") }
}
```

测试向量（逐字）：

| 输入 | 输出 |
|---|---|
| moved `a.lnk` + skipped `desktop.ini` | 汇总 `{moved:1, cancelled:0, failed:[], notes:[]}`，提示 None |
| cancelled `a.lnk` | `(Info, "已取消 1 项")` |
| failed a.lnk「正被其他程序占用」、b.lnk「不存在」、c.lnk(null) | `(Error, "3 项未能收纳：a.lnk（正被其他程序占用）、b.lnk（不存在） 等")` |
| moved a.lnk、cancelled b.lnk、failed c.lnk「没有权限」 | `(Warning, "已收纳 1 项；1 项未能收纳：c.lnk（没有权限）；已取消 1 项")` |
| moved a.lnk 带附注「源文件未能删除」 | `(Warning, "已收纳 1 项；a.lnk：源文件未能删除")` |
| 删除：restored 1，remaining [b.txt, C.lnk, hidden.tmp]，failures b.txt「正被其他程序占用」、c.lnk(cancelled)「用户取消了操作」 | `3 个项目未能移回桌面，收纳箱已保留：b.txt（正被其他程序占用）、C.lnk（用户取消了操作） 等` |
| 删除：remaining []，failures a.lnk「没有权限」 | `1 个项目未能移回桌面，收纳箱已保留：a.lnk（没有权限）` |
| 删除：restored 3，remaining []，failures [] | `3 个项目已移回桌面，但收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除` |
| 删除：restored 0，全空 | `收纳文件夹未能删除（可能被占用），收纳箱已保留，可稍后再删除` |
| `base_name('C:\Users\me\Desktop\Chrome.lnk')` / `('C:\Users\me\Desktop\文件夹\')` / `('/home/me/a.txt')` / `('a.txt')` | `Chrome.lnk` / `文件夹` / `a.txt` / `a.txt` |
| `parent_dir('C:\Users\me\Desktop\Chrome.lnk')` / `('/home/me/a.txt')` | `C:\Users\me\Desktop` / `/home/me` |

后端逐项原因（`error` 字段）常见值：`用户取消了操作`、`已跳过`、`需要完整路径`、`无法收纳磁盘根目录`、`无法访问: …`、`重复的项目`、`不能收纳收纳目录本身或其上级目录`、`不能收纳桌面文件夹本身或其上级目录`、`已在该收纳箱中`、`不能把收纳箱文件夹放进收纳箱`、`收纳文件夹中找不到该项目`；moved 附注如 `已放入目标位置，但源文件未能删除…`、`已移动，但无法确定新的文件名…`。

### 5.6 图标尺寸

```rust
const ICON_BUCKETS: [u32; 8] = [16, 24, 32, 48, 64, 96, 128, 256];
fn icon_bucket(size_px: u32) -> u32 { ICON_BUCKETS.iter().copied().find(|b| *b >= size_px).unwrap_or(256) }
fn icon_request_px(icon_size: f64, dpr: f64) -> u32 { (icon_size * if dpr > 0.0 { dpr } else { 1.0 }).ceil() as u32 }
fn clamp_icon_size(size: f64) -> i32 { js_round(size).clamp(24, 128) }
fn next_icon_size(current: i32, delta_y: f64) -> i32 {
    if delta_y == 0.0 { return clamp_icon_size(current as f64); }
    clamp_icon_size((current + if delta_y < 0.0 { 8 } else { -8 }) as f64)
}
```

向量：`icon_bucket(0)=16, (48)=48, (49)=64, (72)=96, (1000)=256`；`icon_request_px(48,1.5)=72, (48,1.25)=60, (40,1.75)=70, (48,0)=48`；`next_icon_size(48,-100)=56, (48,100)=40, (48,0)=48, (128,-100)=128, (24,100)=24`；`clamp_icon_size(1000)=128, (-5)=24`。

### 5.7 颜色工具（`utils/color.ts`）

```rust
/// HEX 比较（去空白、不区分大小写）：取色器可能写回小写，预设是大写
fn is_same_color(a: &str, b: &str) -> bool { a.trim().to_lowercase() == b.trim().to_lowercase() }

/// '#3B82F6' → (59, 130, 246)；3 位简写展开；非法输入回落 (0, 0, 0)
fn hex_to_rgb(hex: &str) -> (u8, u8, u8) {
    let n = hex.trim().trim_start_matches('#');
    let full = if n.len() == 3 { n.chars().flat_map(|c| [c, c]).collect::<String>() } else { n.to_string() };
    if full.len() != 6 || !full.chars().all(|c| c.is_ascii_hexdigit()) { return (0, 0, 0); }
    let v = u32::from_str_radix(&full, 16).unwrap();
    ((v >> 16) as u8, (v >> 8) as u8, v as u8)
}
```

---

## 6. 收纳箱后端命令契约（前端调用的部分）

参数：前端 camelCase（`fileName`、`targetId`、`sizePx`、`expandedView`、`fileNames`）↔ Rust snake_case。

| 命令 | 同步/异步 | 参数 | 返回 | 前端调用点 | 典型错误 |
|---|---|---|---|---|---|
| `get_fence` | 同步 | `id` | `Fence` | 窗口启动 | `收纳箱 #{id} 不存在` |
| `list_fences` | 同步 | — | `Vec<Fence>`（`sort_order, id`） | 右键菜单「移动到…」目标 | — |
| `rename_fence` | async | `id, title` | `RenameFenceResult { fence, folderError: Option<String> }`；同时 `set_title` 窗口标题 | 改名 | `收纳箱标题不能为空` / `收纳箱标题不能超过 40 个字符` / `收纳箱正在删除`；移动中 → Ok + folderError `收纳箱里有文件正在移动，文件夹暂未改名（之后再改一次标题即可同步）` |
| `update_fence` | 同步 | `id, patch: { collapsed?, locked?, bgColor?, bgAlpha?, iconSize? }` | `Fence` | 折叠、固定、背景色、透明度、图标大小 | `背景色格式无效：{v}`（整个 patch 不写）；`bgAlpha` 夹 0~1（非有限数 → 0.45）；`iconSize` 夹 24~128 |
| `set_fence_view` | 同步 | `id, expanded` | `()`（不写库） | `viewExpanded` 变化 | `收纳箱 #{id} 的窗口不存在` |
| `set_fence_bounds` | 同步 | `id, x, y, width, height`（物理像素） | `()`（一次 `SetWindowPos`，不写库） | 缩放每帧 | 同上 |
| `commit_fence_geometry` | 同步 | `id, expandedView` | `Fence`（写 `fences.x/y/width/height` + 当前组合 `fence_layouts`；`expandedView=false` 时只落位置和宽度） | 拖动结束（防抖）、缩放松手 | — |
| `delete_fence` | async | `id` | `DeleteFenceResult { deleted, restored, remaining: Vec<String>, failures: Vec<ItemOpResult> }`；`deleted=true` 时后端 `destroy` 窗口 | 删除确认 | `收纳箱正在删除` / `收纳箱里还有文件正在移动，请稍后再删除` / `无法确定桌面路径` |
| `get_fence_items` | async | `id` | `Vec<FenceEntry>`（对账后按排序） | 刷新列表 | `收纳箱 #{id} 不存在或正在删除`（前端静默） |
| `fence_move_in` | async | `id, paths: Vec<String>` | `FenceMoveInResult { results（与 paths 一一对应）, onedriveNotice }` | 外部拖入 | `收纳箱正在删除` |
| `fence_restore_item` | async | `id, fileName` | `ItemOpResult` | 右键「移回桌面」 | `文件名不合法: …` |
| `fence_move_item` | async | `id, fileName, targetId` | `ItemOpResult`；成功后对目标收纳箱 `reconcile_and_notify` | 右键「移动到…」 | `目标收纳箱与当前收纳箱相同` |
| `fence_open_item` | async | `id, fileName` | `()` | 双击 / 「打开」 | `快捷方式指向的目标不存在，可能已被移动或卸载`（约 3 秒后）/ `文件不存在` |
| `fence_reveal_item` | async | `id, fileName` | `()` | 「打开所在位置」 | `文件不存在` |
| `fence_item_icon` | async | `id, fileName, sizePx: u32` | `String`（缓存 PNG 绝对路径） | 图标懒加载 | 失败 → 占位图标 |
| `reorder_fence_items` | 同步 | `id, fileNames: Vec<String>`（完整新顺序） | `()` | 拖动排序结束 | `保存排序失败: …` |
| `create_fence` / `toggle_fences_visibility` / `reset_fence_positions` | — | — | `Fence` / `bool` / `()` | 前端未调用（托盘入口由后端直接处理：「新增收纳箱」「显示/隐藏所有收纳箱」「重置收纳箱位置」） | — |

`ItemOpResult`（camelCase）：`source`（源完整路径）、`status: 'moved'|'cancelled'|'failed'|'skipped'`、`fileName`（moved 时新文件名）、`destPath`（moved 时新完整路径）、`error`（原因 / 附注）。

**事件**：`fence-items-changed`，`app.emit_to("fence-{id}", …)`，payload `Vec<FenceEntry>`（完整、已排序）。

非 Windows：移动 / 打开 / 定位 / 图标命令返回 `Err("桌面收纳箱仅支持 Windows")`，托盘不显示收纳箱菜单项。

---

## 7. GPUI 还原难点与近似方案

以下基于本机 cargo registry 中 `gpui-pre 0.3.9`、`gpui-pre-windows 0.3.9`、`gpui-component 0.7.3` 的源码检查结果。

| # | 难点 | 现状（Tauri/Vue） | GPUI 侧情况与近似方案 |
|---|---|---|---|
| 1 | **从资源管理器拖入文件** | `onDragDropEvent` enter/over/leave/drop + paths | gpui-pre-windows 已实现 OLE `IDropTarget`（`WindowsDragDropHandler`，只接 `CF_HDROP`，回报 `DROPEFFECT_COPY`，鼠标显示"复制"光标）。平台事件 `FileDropEvent::{Entered{position,paths}, Pending{position}, Submit{position}, Exited, Ended}` → 映射：Entered/Pending = enter/over，Exited/Ended = leave，Submit = drop。元素级 API：`div().on_drop::<ExternalPaths>(…)`、`.drag_over::<ExternalPaths>(|style,…| …)`（虚线框可用 `.border_dashed()` + 2px 半透明白边内缩 4~6px 的子层实现）、`.can_drop(…)`。`drag` 展开原因建议在根元素用 `drag_over` / `on_drag_move::<ExternalPaths>` 打开，在 `Exited`/`Ended`/drop 后 300ms 撤销 |
| 2 | **标题栏"超过 3px 才拖动窗口"且保留双击改名** | `startDragging()`（tao：`ReleaseCapture` + `WM_NCLBUTTONDOWN(HTCAPTION)`） | gpui-pre 的 `Window::start_window_move()` 在 Windows 上是**空实现**（平台 trait 默认 no-op）；`window_control_area(WindowControlArea::Drag)` 会把区域命中为 `HTCAPTION`，系统接管后客户区收不到 mousedown/双击（等同于 `data-tauri-drag-region` 的问题）。近似：客户区自己处理 mouse_down / mouse_move（gpui-windows 在按下时 `SetCapture`），超过 3px 后通过 `HasWindowHandle` 取 HWND，`ReleaseCapture()` + `PostMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION, 屏幕坐标)`（与 tao 相同）。拖动结束检测：`cx.observe_window_bounds` + `GeometryCommitGate` 原样移植。设置窗口标题栏不需要双击，可直接 `window_control_area(Drag)`（窗口不可缩放，gpui 不加 `WS_MAXIMIZEBOX`，双击不会最大化） |
| 3 | **窗口内 8 个缩放手柄**（窗口 `resizable(false)`） | 指针捕获 + 每帧 `set_fence_bounds`（`SetWindowPos`） | gpui 的 `start_window_resize` 依赖系统缩放循环（需要 `WS_THICKFRAME`），不可用。近似：8 个绝对定位透明 div，`.cursor(CursorStyle::ResizeUpDown / ResizeLeftRight / ResizeUpLeftDownRight / ResizeUpRightDownLeft)`；按下时记录 **屏幕物理坐标**（Win32 `GetCursorPos`，因为拖左/上边时窗口在动，窗口相对坐标不可靠）与窗口外框（`GetWindowRect`）；移动时 `resize_rect` → `SetWindowPos(SWP_NOZORDER|SWP_NOACTIVATE)`（gpui 没有 set_position/set_bounds API，需要 raw HWND）；每帧最多一次（可用 `window.on_next_frame` 或 16ms 节流）；松手 `commit_fence_geometry` |
| 4 | **自绘右键菜单 / 标题栏下拉菜单在很小的窗口里** | `position:fixed` 自绘菜单 + clamp；el-dropdown | gpui-pre-windows 明确不支持原生弹出窗口（`AnchoredPopup` 返回 `PopupNotSupportedError`，回落到窗口内 popover），所以菜单必须画在收纳箱窗口内——保留"菜单打开 → 加 `menu`/`itemMenu` 展开原因 → 窗口先展开"的逻辑。右键菜单建议自绘（`deferred(anchored().position(point).snap_to_window())` 或绝对定位 + 手动 clamp 4px），以便做两页（主页 / 移动到）切换与精确的深色样式；标题栏菜单可用 gpui-component `PopupMenu`/`DropdownMenu`，但样式需改成 Element Plus 浅色（白底、`#606266` 文字、hover `#ECF5FF`/`#409EFF`、分隔线） |
| 5 | **颜色选择器** | 设置：`el-color-picker`（SV 面板 + 色相条 + hex 输入 + 英文 Clear/OK）；收纳箱：原生 `<input type=color>`（Windows 系统取色对话框） | gpui-component `ColorPicker` 是调色板 + 可选 featured colors 的弹层，与 Element Plus 的 SV 面板外观不同。近似：设置窗口自绘 SV 面板（280×180，可用两层 `linear_gradient` 叠加：横向白→色相、纵向透明→黑）+ 280×12 色相条 + hex 输入 + Clear/OK；收纳箱自定义色直接调用 Win32 `ChooseColorW` 通用对话框（与 WebView2 行为一致），父窗口设为收纳箱 HWND，并在阻塞线程/非主线程处理以免卡 UI |
| 6 | **滑块"拖动预览、松手落库"** | `el-slider` 的 `input` / `change` 两个事件 | gpui-component `Slider` 的事件在拖动中连续触发；需要自己区分"松手"（监听 mouse_up 或在 Slider 外包一层记录按下状态），松手时才写库；设置窗口透明度滑块还需 hover/拖动时的 `45%` 气泡 tooltip，字号滑块不要 tooltip |
| 7 | **Element Plus 视觉一致性** | 各组件默认主题 | gpui-component 自带主题（shadcn 风格）与 Element Plus 差异大（开关尺寸、主色 `#409EFF`、输入框 24/32 高、按钮 6px 圆角等）。建议按 §1.3 速查表做一套 `ElTheme` 常量与自绘小组件（Switch 40×20、RadioButton 组、Input small、Select、Button 变体），或通过 gpui-component 的 Theme 变量尽量逼近 |
| 8 | **字体下拉：可搜索、每项用自身字体渲染** | `el-select filterable` + option `font-family` | 字体列表可沿用 DirectWrite 枚举（现有 `get_system_fonts` 逻辑），或 gpui `window.text_system().all_font_names()`；gpui-component `Select` 支持搜索（`SearchableListDelegate`），自定义 item 渲染时对文本设 `font_family(name)`；数百个字体建议用虚拟列表（`uniform_list`） |
| 9 | **Toast（ElMessage）与确认框（ElMessageBox）** | 窗口内顶部居中 toast；窗口内模态确认框 | gpui-component 有 `Notification`、`Dialog`/`AlertDialog`，需调成 Element Plus 外观：toast 距顶 16px 居中、四种配色、默认 3000ms（拖入结果 5000ms、删除中止 8000ms 且带关闭按钮）；确认框 420 宽、遮罩 `rgba(0,0,0,0.5)`、标题 18px、warning/info 图标、按钮"取消（默认）/确认（primary）"顺序，Esc/×/点遮罩 = 取消。注意收纳箱窗口很小（最小 160×160），toast 会被 `max-width: 100% − 32px` 限宽换行 |
| 10 | **设置窗口作为主窗口的"模态子窗口"** | owned window（`parent`）+ 主窗口遮罩 + 焦点转移 | gpui `WindowKind::Dialog` 会 `EnableWindow(parent, false)` 真正禁用父窗口（行为不同：主窗口无法接收点击）。若要与现状一致：创建普通窗口后用 Win32 `SetWindowLongPtrW(GWLP_HWNDPARENT, 主窗口)` 设 owner，主窗口自绘 `rgba(0,0,0,0.3)` 遮罩 + `cursor:not-allowed`，点击遮罩 / 主窗口获焦时把设置窗口激活到前台 |
| 11 | **透明背景 + 圆角 + 半透明底色** | `transparent(true)` + CSS `rgba` + DWM 圆角 | `WindowOptions.window_background = WindowBackgroundAppearance::Transparent`；根 div 画 `rounded(8px)` + `bg(rgba(bg, alpha))` + 1px `rgba(255,255,255,0.14)` 边框；Win11 上可另设 `DWMWCP_ROUND`。注意 gpui-pre-windows 默认使用 DirectComposition（`WS_EX_NOREDIRECTIONBITMAP`），透明合成需实测 |
| 12 | **嵌入桌面层** | tao `always_on_bottom` + owner=Progman + `WS_EX_TOOLWINDOW` + 200ms 自愈轮询 | gpui 的 `WindowKind::PopUp` 是 `WS_EX_TOOLWINDOW \| WS_EX_TOPMOST`（置顶，不可用），`Normal` 是 `WS_EX_APPWINDOW`。需建窗后自行改 ex style（去 APPWINDOW、加 TOOLWINDOW）、设 owner=Progman、并子类化 `WM_WINDOWPOSCHANGING` 把 Z 序压到 `HWND_BOTTOM`（替代 tao 的 always_on_bottom 钩子）。`is_minimizable=false`、`is_resizable=false` 可直接用 WindowOptions。细节见 `desktop-fences.md` §3.1（属于窗口层而非 UI 层，但收纳箱 UI 依赖它） |
| 13 | **文字阴影 / 图标投影** | `text-shadow`、`filter: drop-shadow` | gpui 只有 `box_shadow`（对元素矩形，不对文字/图片 alpha）。近似：文字先用 `rgba(0,0,0,.4~.65)` 在 (0,1) 偏移画一遍再画正文；图标投影可在提取 PNG 时预烘焙，或省略 |
| 14 | **自定义色圆点的 conic-gradient 彩虹环** | `conic-gradient` + `soft-light` 混合 | gpui 只有 `linear_gradient`。近似：预渲染一张小 PNG/SVG 彩虹环叠在当前色圆上（半透明），或用 6 段扇形 path（`PathBuilder`）绘制 |
| 15 | **名称 2 行截断 + 任意字符断行** | `-webkit-line-clamp: 2` + `word-break: break-all` | gpui `Styled::line_clamp(2)` + `text_ellipsis()`；gpui 的换行器遇到放不下的长单词会在字符处断开，接近 break-all；行高 16、12px、居中 |
| 16 | **图标懒加载与 PNG 显示** | IntersectionObserver + `convertFileSrc` | 可见性：按滚动偏移与单元格尺寸计算可见行（±64px），或用 `uniform_list`；图片：`img(path)` 直接加载缓存 PNG（或跳过 PNG，直接把 HICON 位图转成 `RenderImage`）；并发 3 个、同 key 不重复请求、失败显示 `Folder`/`Document` 占位 |
| 17 | **箱内拖动排序** | SortableJS fallback 模式：4px 容差、克隆体跟随、占位 `.35` 透明、150ms 让位动画 | gpui 内部拖拽：`.on_drag(payload, |…| 预览视图)`（gpui 拖动阈值 `DRAG_THRESHOLD = 2px`，需要 4px 可在 mouse_move 中自判后再开始）+ 目标单元 `.on_drop::<DraggedItem>()` / `.drag_over::<DraggedItem>()`；预览视图画 opacity 正常的克隆，原位置画 `.opacity(0.35)`；让位动画可省略或用 gpui `with_animation`。拖动期间暂停 `fence-items-changed` 的列表替换，松手后 `reorder_fence_items` + 重新列举 |
| 18 | **Ctrl+滚轮** | `wheel` + `ctrlKey` + 40ms 节流 | `on_scroll_wheel(|ev, …| ev.modifiers.control …)`；`ScrollDelta` 区分 Lines/Pixels，只取符号（上滚放大）；非 Ctrl 时照常滚动并关闭右键菜单 |
| 19 | **悬停临时展开的进入/离开计时** | `mouseenter`/`mouseleave` + 150/400ms | 根元素 `.on_hover(|hovered, …|)`（gpui 在鼠标离开窗口时会清除 hover），配合 `cx.spawn` + `Timer::after` 实现延时与取消；拖窗口期间（闸门打开）不展开、延后收回 |
| 20 | **改名时窗口失焦提交** | `window.blur` | `cx.observe_window_activation(window, …)`，失去激活时若在改名则提交 |
| 21 | **折叠态窗口实际只有 32 物理行** | 32px 标题栏 + 1px 根边框（标题栏下沿被裁掉 2px） | GPUI 版折叠态直接让标题栏占满 32（含边框）即可；展开态标题栏下方 1px `rgba(255,255,255,0.08)` 分隔线 |

---

## 8. Related Specs

- `.trellis/spec/backend/desktop-fences.md` — 收纳箱完整契约（建窗参数、Win32 状态表、几何落库时机、错误矩阵、e2e 步骤）
- `.trellis/spec/backend/sync-protocol.md` §3.6 — 收纳箱数据与 `fence_onedrive_notice_shown` 为设备本地数据，不进导出 / 同步
- `.trellis/spec/backend/window-modes.md` — 主窗口普通 / 固定 / 嵌入桌面三种模式（设置页「固定模式时，嵌入桌面中」「贴边自动隐藏」「唤起时置顶」的后端语义）

## Caveats / Not Found

- **未做运行时实测**：Element Plus 组件尺寸取自 2.13.1 的 theme-chalk CSS；el-dropdown 弹层与触发按钮的间距（offset）与箭头位置未从源码确认，按 Element Plus 默认（约 12px、带箭头）理解。
- **`<input type=color>` 在 WebView2 中的具体弹窗**按 Chromium Windows 版的通常行为（系统 `ChooseColor` 对话框）描述，未实机确认。
- **默认 368 宽的收纳箱实际列数**：按 CSS 计算可用宽 = 368 − 2（边框）− 16（padding）= 350 < 4 × 88 = 352，`auto-fill` 实际排 **3 列**；`FenceItemGrid` 注释与后端常量的意图是"刚好 4 列"。要与现状像素级一致应按 CSS 计算（3 列），要符合设计意图则需要调整 1~2px。
- §2.13 列出的设置窗口现状行为（"当前"屏幕配置不生效、版本号初始为空、清空字体不落库等）是按代码静态推断，未在运行中的应用里验证。
- gpui-pre / gpui-component 只检查了与本文相关的少量 API（`FileDropEvent`、`ExternalPaths`、`start_window_move` 的 Windows 实现、`WindowKind`、`CursorStyle`、`border_dashed`、`line_clamp`、`linear_gradient`、`Slider`/`Switch`/`ColorPicker`/`PopupMenu` 的存在性），没有逐一验证调用方式。

# 修复设置窗口开关与主窗口状态不一致

## 背景

设置界面（`pc/src/views/SettingsView.vue`）跑在独立的 Tauri WebView 里，
它的 `useAppStore()` 与主窗口是**两个互不相干的 Pinia 实例**。
常规页多个开关直接绑在这份「副本」store 上，导致读、写两侧都出问题。

## 根因与缺陷清单

| # | 缺陷 | 位置 |
|---|------|------|
| 1 | 深色主题开关恒显示「关」——设置窗口从未加载过主题，`loadDarkTheme` 也没从 store 导出 | `appStore.ts` / `SettingsView.vue:onMounted` |
| 2 | 深色主题改完主窗口不变色——只改了设置窗口自己的 store 与 body class | `appStore.ts:toggleDarkTheme` |
| 3 | **改深色主题/贴边隐藏会污染主窗口几何**——`saveWindowState()` 调 `get_window_persist_state(window: Window)`，取的是调用方（设置）窗口的位置尺寸，并把 `is_fixed` 写成设置窗口 store 的默认 `false` | `window.rs:get_window_persist_state` |
| 4 | 展示日历 / 贴边隐藏改完主窗口不实时响应——没有 emit 事件 | `SettingsView.vue` + `MainView.vue` |
| 5 | 自动同步开关/间隔改完，主窗口的定时器不重建 | `MainView.vue:startAutoSync` |
| 6 | 导入数据 / 应用云端数据会覆盖 settings 表，但主窗口只弹 toast，设置与列表都不重载 | `MainView.vue:'data-imported'` |
| 7 | 设置窗口检查到新版本，主窗口标题栏红点不亮 | `SettingsView.vue:handleCheckUpdate` |

补充：`body.dark-theme` 会把 `--text-primary` 改成白色（`main.scss:65`），
而设置窗口背景是硬编码浅色，因此深色 class 绝不能应用到设置窗口。

## 方案

**Rust**
- `get_window_persist_state` / `set_window_fixed_mode` 改为固定取 `get_webview_window("main")`，
  不再按调用方窗口取值
- 新增 `set_text_theme` / `get_text_theme`：只读写 `text_theme` 键，不触碰窗口几何

**Store**
- 导出 `loadDarkTheme`，新增 `setDarkTheme(enabled)`；`toggleDarkTheme` 复用它
- `setDarkTheme` / `setAutoHideEnabled` 都不再调 `saveWindowState()`
- `applyThemeClass()` 加主窗口守卫（`appWindow.label !== 'main'` 直接返回）

**跨窗口通知**
- 设置窗口发 `app-settings-changed` + `{ key }`；
  主窗口按 key 重载（`showCalendar` / `autoHide` / `theme` / `sync` / `update`）
- 主窗口抽出 `reloadAppSettings()`，在设置窗口关闭、数据导入、云端数据应用后统一调用

## 非本次范围

- `text_theme` 的存储语义是反的（`'light'` 表示深色主题开启），已加注释说明，未改 DB 语义（会破坏旧数据兼容）
- `pc/src/components/SettingsPanel.vue` 已无任何引用，是死代码
- 自动同步开关会把用户尚未点「保存配置」的 WebDAV 表单草稿一并落库（交互设计问题，非状态不一致）

## 验证

- `cargo check` 通过
- `npm run build`（含 vue-tsc）通过

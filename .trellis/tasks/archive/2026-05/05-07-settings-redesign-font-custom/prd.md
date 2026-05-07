# 设置界面重构 + 待办字体自定义

## Goal

重构设置窗口为左右分栏布局（左侧菜单 + 右侧内容面板），同时新增待办列表的字体族选择和字体大小自定义功能，回应用户 Issue #5 的第 3 点建议。

## What I already know

- 当前设置窗口 `SettingsView.vue` 是垂直滚动的单栏布局，480x720，不可缩放
- 设置项存储在 SQLite `settings` 表（key-value 结构）
- 当前 TodoItem 标题 14px，子任务/元信息 12px，字体链为系统字体
- 已有 CSS 变量系统（`src/styles/main.scss`），含 spacing、radii、shadows 等
- 设置窗口通过 `MainView.vue` 的 `openSettings()` 以独立 WebviewWindow 打开
- 深色主题通过 `body.dark-theme` class 切换

## Requirements

### 设置界面重构

- 窗口尺寸从 480x720 改为 680x560
- 左右分栏布局：左侧菜单 160px + 右侧内容面板
- 菜单分 6 项：
  1. 常规（开机自启、显示日历、贴边隐藏、深色主题、通知类型）
  2. 外观（字体选择、字体大小）
  3. 数据与同步（导入/导出 + WebDAV 同步）
  4. Agent 与调度（Agent 管理 + 任务调度器）
  5. 屏幕配置（窗口位置/大小）
  6. 关于（版本、更新）
- 菜单项点击切换右侧内容，不刷新页面

### 字体自定义

- **字体族选择**：通过 Rust 后端枚举 Windows 系统字体（DirectWrite），前端使用 `el-select` + `filterable` 下拉框
- **字体大小**：滑块控件，范围 12px ~ 20px，步进 1px，默认 14px
- **作用域**：仅影响待办列表的 todo 项内容（标题、子任务、元信息）
- **联动规则**：用户调标题字体大小，子任务/元信息自动 -2px
- **字体预览**：下拉选项用对应字体渲染字体名，设置页内显示预览文本
- **默认值**：字体族默认"跟随系统"（使用现有字体链）
- **实时预览**：选择/拖动时通过 Tauri 事件通知主窗口实时更新样式

### 存储与同步

- 字体族和字体大小都存入 `settings` 表（key: `todo_font_family`, `todo_font_size`）
- **不纳入**导入导出和 WebDAV 同步（字体族设备相关，字体大小用户也可能因屏幕不同而不同）

## Acceptance Criteria

- [ ] 设置窗口以左右分栏布局打开，左侧 6 个菜单项可切换
- [ ] 外观页面包含字体选择下拉框（可搜索、字体预览）和字体大小滑块
- [ ] 选择字体或拖动滑块时，主窗口待办列表实时更新样式
- [ ] 字体设置持久化到 settings 表，重启应用后保留
- [ ] 默认"跟随系统"时使用原有字体链，字体大小默认 14px
- [ ] 深色主题下设置界面和新增控件样式正常
- [ ] 导入导出和 WebDAV 同步不包含字体设置

## Definition of Done

- Lint / typecheck / cargo check 通过
- 深色/浅色主题下 UI 正常
- 设置窗口所有现有功能不受影响（回归验证）

## Decision (ADR-lite)

**Context**: 用户 Issue #5 建议自定义字体/间距/大小，同时设置窗口内容越来越多需要更好的组织方式。

**Decisions**:
1. 设置界面采用左右分栏布局（菜单 + 内容面板），而非继续垂直堆叠
2. 系统字体通过 Rust 后端 DirectWrite API 枚举，而非前端 JS API 或预设列表
3. 字体设置不纳入同步（设备相关）
4. 字体大小联动子任务（-2px），而非独立可调
5. 实时预览通过 Tauri 事件跨窗口通知

**Consequences**: 引入新的 Rust crate（`dwrote` 或 `font-enumeration`）；设置窗口尺寸变大但更易扩展。

## Out of Scope

- 字间距（letter-spacing）
- 行高（line-height）调整
- 项目间距调整
- 颜色/主题自定义（已有 dark/light）
- 鼠标穿透（Issue #5 第 1 点，已放弃）

## Technical Notes

- 设置窗口：`src/views/SettingsView.vue`，打开逻辑在 `src/views/MainView.vue:394`
- 待办项样式：`src/components/TodoItem.vue`（标题 14px，子任务 12px）
- CSS 变量：`src/styles/main.scss`
- 设置读写：`src-tauri/src/commands/settings_cmd.rs`
- 数据模型：`src-tauri/src/db/models.rs` → `AppSettings`
- 导入导出：`src-tauri/src/commands/data.rs`
- WebDAV 同步：`src-tauri/src/commands/sync_cmd.rs`

# Journal - dev (Part 1)

> AI development session journal
> Started: 2026-05-07

---



## Session 1: feat: 子任务标题展示

**Date**: 2026-05-07
**Task**: feat: 子任务标题展示
**Branch**: `main`

### Summary

实现 Issue #5 建议 2：TodoItem 中子任务计数区域可点击展开子任务标题列表，显示完成状态和标题，支持排序、截断 tooltip、展开动画和深色主题适配

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `facdc38` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 2: Issue #6: 待办列表自动刷新

**Date**: 2026-05-07
**Task**: Issue #6: 待办列表自动刷新
**Branch**: `main`

### Summary

分析 Issue #6，实现窗口焦点刷新 + 60s 低频轮询自动刷新待办列表，更新前端 state-management spec

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `22824aa` | (see git log) |
| `a67d270` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 3: 设置界面重构 + 待办字体自定义

**Date**: 2026-05-07
**Task**: 设置界面重构 + 待办字体自定义
**Branch**: `main`

### Summary

重构设置窗口为左右分栏布局（左侧菜单+右侧面板），新增外观设置支持系统字体选择和字体大小自定义，通过 Rust DirectWrite API 枚举字体，Tauri 事件实现跨窗口实时预览

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `289742a` | (see git log) |
| `223ed7d` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 4: feat: 重复提醒功能实现

**Date**: 2026-05-09
**Task**: feat: 重复提醒功能实现
**Branch**: `main`

### Summary

实现闹钟式重复提醒功能（daily/weekly/monthly），含数据库迁移、通知推进算法、EditorView UI、TodoItem 图标、导入导出覆盖。质量检查修复了容器 v-if 条件遗漏问题。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `7dde72f` | (see git log) |
| `9c56618` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 5: 移除 AI Agent 功能

**Date**: 2026-05-13
**Task**: 移除 AI Agent 功能
**Branch**: `main`

### Summary

全面移除 Mini-Todo 项目的 AI Agent / 任务调度 / 工作流 / 提示词模板 / 任务依赖五大模块（约 80 个文件改动）。数据库迁移 v23 DROP 5 张表 + DROP 17 列，导出版本 3.0 → 4.0（通过 serde 默认行为兼容旧 v3.0 备份），应用版本 1.6.4 → 2.0.0。子任务退化为纯 Markdown 子项，重复提醒/通知/WebDAV 同步/四象限/日历功能不受影响。trellis-check 验证零缺陷，cargo check / vue-tsc / vite build 均通过零 warning。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `23ab651` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 6: cloud-api-and-skill: 3-PR cloud REST + Skill + PC race fix

**Date**: 2026-05-13
**Task**: cloud-api-and-skill: 3-PR cloud REST + Skill + PC race fix
**Branch**: `main`

### Summary

新增 cloud/ 独立 Rust crate（axum + rusqlite + reqwest + WebDAV 客户端），实现：(1) PR1 只读骨架——/health + Bearer auth + 60s pull worker + 启动镜像图片；(2) PR2 REST CRUD 全套（/todos /subtasks /images）+ 1s tick push worker（CAS dirty + per-record LWW + tombstone 抑制 + 412 重试 + 7 天 tombstone GC）+ Claude Code skill（SKILL.md + Python CLI 9 子命令 + 跨平台 install 脚本）；(3) PR3 PC 端 race 修复——webdav.rs upload_bytes 加 if_unmodified_since + 返回 UploadOutcome；sync_cmd.rs 改条件 PUT + 412 重试循环 + merge_remote_into_local 单事务 per-record LWW；db v24 加 webdav_last_modified setting；app 版本 2.0.0 → 2.1.0。spec 沉淀：cross-layer-thinking-guide.md 新增'两端 SQLite 副本 + HTTP blob 双向同步'章节（时间格式对齐 / 条件 PUT 局限 / id 保留 / tombstone / 冲突矩阵 / Wrong vs Correct）。3 轮 trellis-check 共抓 7 个 finding（含 SQLite IFNULL 三参 500 / cmd_today 窗口错 / settings null 让 PC 反序列化失败 / 图片扩展名白名单）已自修。验证：cloud cargo build/clippy -D warnings/fmt/28 tests + pc cargo check/clippy/fmt/npm run build 全绿。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `87639f4` | (see git log) |
| `605f3e2` | (see git log) |
| `64e3d88` | (see git log) |
| `2a13fa9` | (see git log) |
| `efca93c` | (see git log) |
| `e75cd7a` | (see git log) |
| `d0d56af` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 7: 待办描述 Markdown 化 + 只读详情模式 + 发布 2.2.0

**Date**: 2026-08-09
**Task**: 待办描述 Markdown 化 + 只读详情模式 + 发布 2.2.0
**Branch**: `main`

### Summary

抽取可复用 MarkdownEditor 组件（Milkdown+GFM+clipboard），待办描述支持 MD 编辑与图片上传；新增只读详情模式（四入口统一默认进入，可原地切换编辑）；新增源码/预览分栏放大编辑弹窗（联动窗口最大化）；修复粘贴 MD 被转义与围栏代码块样式污染两个缺陷；沉淀 2 条 Common Mistake 到 component-guidelines；版本升至 2.2.0 并打 tag 发布

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `9b7b56c` | (see git log) |
| `1c68436` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 8: 任务台账清理：归档全部 8 个历史任务

**Date**: 2026-08-09
**Task**: 任务台账清理：归档全部 8 个历史任务
**Branch**: `main`

### Summary

按用户指令归档所有任务：6 个已完成（05-14 系列 5 个 + 08-09-health-fixes）、bootstrap 占位任务与 cloud-api 规划任务一并入 archive/2026-08/，台账清零

### Main Changes

(Add details)

### Git Commits

(No commits - planning session)

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 9: 修复设置界面开关与主窗口表现不一致

**Date**: 2026-08-10
**Task**: 修复设置界面开关与主窗口表现不一致
**Branch**: `main`

### Summary

定位根因为设置窗口作为独立 WebView 持有自己的 Pinia store 副本：读侧未加载真值、写侧不通知主窗口，且 get_window_persist_state 按调用方窗口取值导致设置窗口几何被误存为主窗口状态（连带清空 is_fixed）。修复方式：两个窗口级 Tauri 命令改为固定取 main 窗口，新增 set_text_theme 窄命令替代 saveWindowState，applyThemeClass 加主窗口守卫；新增 app-settings-changed 跨窗口事件并把 key 抽为 AppSettingKey 类型（拼错即 TS2678）。排查全部设置项后另修 3 处同类问题：自动同步定时器不重建、数据导入/云端应用后不重载设置、检查更新红点不同步。质量检查阶段自查再修 3 处：事件 key 字面量重复、setShowCalendar 缺失回滚、Rust 读取逻辑重复。spec 沉淀 2 条 Common Mistake

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `c665551` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 10: 修复 issue #9 v2.3.0 五条新反馈并发布 2.3.1

**Date**: 2026-08-12
**Task**: 修复 issue #9 v2.3.0 五条新反馈并发布 2.3.1
**Branch**: `main`

### Summary

核实并修复 KieMg 在 v2.3.0 试用后的五条反馈：窗口 1px 过渡描边+8px 圆角（固定模式除外）、列表底部避让悬浮按钮、只读详情放开子任务拖拽、自启注册表路径自愈+托盘/设置双入口勾选同步、安装包内嵌 WebView2 引导器。trellis-check 核查 13 项，修复 1 项错误处理语义反转；3 条经验写入 quality-guidelines.md。版本号升至 2.3.1。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `5ed316e` | (see git log) |
| `0b37f43` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 11: issue #9 第三轮反馈：子任务拖拽体验与固定模式新建入口

**Date**: 2026-08-13
**Task**: issue #9 第三轮反馈：子任务拖拽体验与固定模式新建入口
**Branch**: `main`

### Summary

修复子任务拖拽划选文字与拖影不跟手（transition: all 被 SortableJS fallback 克隆体继承是根因）；按 KieMg 方案 2 实现 FAB 操作模式（悬停操作按钮 0.75s 半透明沉底、离开 1.5s 恢复），固定模式也显示 FAB 可直接新建

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `8d73c7a` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 12: trellis-check：修复操作模式 FAB 卡沉底缺陷并沉淀拖拽规范

**Date**: 2026-08-13
**Task**: trellis-check：修复操作模式 FAB 卡沉底缺陷并沉淀拖拽规范
**Branch**: `main`

### Summary

核查发现删除待办时操作条移出 DOM 不触发 mouseout、FAB 卡在半透明沉底；mouseover 非操作区域兜底恢复。SortableJS force-fallback 继承 transition: all 的教训写入 component-guidelines

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `d937e42` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete


## Session 13: issue 9 v2.3.2 反馈：任务栏图标复现修复 + 只读模式勾选子任务

**Date**: 2026-08-14
**Task**: issue 9 v2.3.2 反馈：任务栏图标复现修复 + 只读模式勾选子任务
**Branch**: `main`

### Summary

定位并修复固定模式任务栏图标复现：tao 在 set_always_on_top/show 等操作时用内部 flags 整体重写 GWL_EXSTYLE，抹掉手动加的 WS_EX_TOOLWINDOW；在全部调用点后经主线程消息队列补写样式并加 SWP_FRAMECHANGED 刷新。只读详情模式子任务复选框恢复可点，直接切换完成态，其余写入口仍隐藏。

### Main Changes

(Add details)

### Git Commits

| Hash | Message |
|------|---------|
| `0dc53fd` | (see git log) |
| `f5c73b9` | (see git log) |

### Testing

- [OK] (Add test results)

### Status

[OK] **Completed**

### Next Steps

- None - task complete

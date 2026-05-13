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

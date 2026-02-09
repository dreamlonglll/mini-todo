# Mini-Todo 项目指南

本文档用于介绍 Mini-Todo 项目，帮助 AI 助手快速了解项目结构和开发规范。

## 项目简介

Mini-Todo 是一款基于 **Tauri 2.x + Vue 3 + TypeScript** 开发的 Windows 桌面待办事项管理应用，具有独特的窗口固定功能和现代化的用户界面。

> 详细信息请参阅：[项目概述](docs/开发文档/01-项目概述.md)

## 技术栈

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| 前端框架 | Vue 3 + TypeScript | 组合式 API，类型安全 |
| UI 组件库 | Element Plus | 包含图标库 @element-plus/icons-vue |
| 状态管理 | Pinia | Vue 官方推荐状态管理 |
| 桌面框架 | Tauri 2.x | 轻量级跨平台桌面框架 |
| 后端语言 | Rust | 高性能，内存安全 |
| 数据库 | SQLite (rusqlite) | 轻量级本地数据库 |
| 拖拽功能 | vuedraggable | Vue 拖拽排序库 |

> 详细架构请参阅：[技术架构](docs/开发文档/02-技术架构.md)

## 项目结构

```
mini-todo/
├── docs/                    # 文档目录
│   └── 开发文档/            # 开发相关文档
├── src/                     # Vue 前端源码
│   ├── assets/              # 静态资源
│   ├── components/          # Vue 组件
│   ├── stores/              # Pinia 状态管理
│   ├── types/               # TypeScript 类型定义
│   ├── utils/               # 工具函数
│   ├── App.vue              # 根组件
│   └── main.ts              # 入口文件
├── src-tauri/               # Tauri/Rust 后端源码
│   ├── src/
│   │   ├── commands/        # Tauri 命令
│   │   ├── db/              # 数据库操作
│   │   ├── lib.rs           # 库入口
│   │   └── main.rs          # 主入口
│   ├── Cargo.toml           # Rust 依赖配置
│   └── tauri.conf.json      # Tauri 配置
├── public/                  # 公共静态资源
├── package.json             # Node 依赖配置
└── vite.config.ts           # Vite 构建配置
```

## 核心功能

### 待办管理
- 创建、编辑、删除待办事项
- 支持一级子任务
- 优先级设置（高/中/低）
- 完成状态标记
- 拖拽排序

### 通知提醒
- Windows 系统通知
- 预设提醒时间（5/15/30 分钟）
- 自定义提前时间

### 窗口特殊功能
- **普通模式**：浅色主题，可拖拽移动
- **固定模式**：
  - 透明背景
  - 固定在用户指定位置
  - 忽略 Win+D（显示桌面）
  - 禁用关闭、最小化、拖拽

> 详细功能请参阅：[功能需求](docs/开发文档/03-功能需求.md)

## 开发规范

### UI 设计规范
- **组件库**：Element Plus
- **图标库**：Element Plus Icons（@element-plus/icons-vue）
- **禁止使用 emoji 图标**
- 设计理念：简洁现代、去除卡片边框、极简列表

#### 优先级颜色
| 级别 | 颜色代码 | 描述 |
|------|----------|------|
| 高 | #EF4444 (红色) | 紧急重要任务 |
| 中 | #F59E0B (橙色) | 一般重要任务 |
| 低 | #10B981 (绿色) | 不紧急任务 |

> 详细规范请参阅：[UI设计规范](docs/开发文档/04-UI设计规范.md)

### 数据库设计
- **数据库类型**：SQLite
- **存储位置**：`%APPDATA%/mini-todo/data.db`
- **主要表**：todos（待办表）、subtasks（子任务表）、settings（设置表）、screen_configs（屏幕配置表）
- **迁移版本**：当前 v1~v6，通过 `src-tauri/src/db/migrations.rs` 管理

> 详细设计请参阅：[数据库设计](docs/开发文档/05-数据库设计.md)

### 数据导入导出

> **重要**：当数据库结构变更（新增表/字段/设置项）时，必须同步更新导入导出功能！

- **导出版本号**：当前 `2.0`（位于 `src-tauri/src/commands/data.rs`）
- **关键文件**：
  - 模型定义：`src-tauri/src/db/models.rs` → `ExportData`、`AppSettings`
  - 导入导出逻辑：`src-tauri/src/commands/data.rs` → `export_data`、`import_data`
  - 前端类型：`src/types/todo.ts` → `ExportData`（前端只传递 JSON 字符串，无需严格同步）

#### 导出覆盖的数据

| 数据表/分类 | 导出内容 | 说明 |
|------------|---------|------|
| `todos` | 全部字段 | id, title, description, color, quadrant, notify_at, notify_before, notified, completed, sort_order, start_time, end_time, created_at, updated_at |
| `subtasks` | 全部字段 | id, parent_id, title, completed, sort_order, created_at, updated_at |
| `settings` | 7 个键值 | is_fixed, window_position, window_size, text_theme, show_calendar, view_mode, notification_type |
| `screen_configs` | **不导出** | 与机器屏幕配置绑定，不适合跨机器迁移 |
| `migrations` | **不导出** | 导入时通过迁移机制自动重建 |

#### 维护检查清单

当新增数据库迁移（如 v7）时，请检查：
1. 新增的 **settings 键值** 是否已加入 `AppSettings` 结构体（含 `#[serde(default)]`）
2. `export_data` 函数是否读取了新设置
3. `import_data` 函数是否写入了新设置
4. 新增的 **数据表** 是否需要纳入导出范围
5. 旧版数据导入的兼容性（新字段必须有默认值）
6. 导出版本号是否需要递增

## 开发命令

```bash
# 安装依赖
npm install

# 开发模式运行
npm run tauri dev

# 构建生产版本
npm run tauri build
```

## 开发进度

| 阶段 | 内容 | 状态 |
|------|------|------|
| 阶段1 | 基础架构搭建 | 已完成 |
| 阶段2 | 核心功能开发 | 已完成 |
| 阶段3 | 通知系统 | 待开始 |
| 阶段4 | 窗口特殊功能 | 已完成 |
| 阶段5 | 数据导入导出 | 待开始 |
| 阶段6 | UI美化与测试 | 待开始 |

> 详细进度请参阅：[开发进度计划](docs/开发文档/06-开发进度计划.md)

## 文档索引

| 文档 | 说明 |
|------|------|
| [01-项目概述.md](docs/开发文档/01-项目概述.md) | 项目简介、目标、技术栈 |
| [02-技术架构.md](docs/开发文档/02-技术架构.md) | 整体架构、前后端设计、依赖配置 |
| [03-功能需求.md](docs/开发文档/03-功能需求.md) | 功能需求规格说明 |
| [04-UI设计规范.md](docs/开发文档/04-UI设计规范.md) | 色彩、排版、组件样式规范 |
| [05-数据库设计.md](docs/开发文档/05-数据库设计.md) | 表结构、字段说明、常用查询 |
| [06-开发进度计划.md](docs/开发文档/06-开发进度计划.md) | 开发阶段、任务清单、里程碑 |

## 注意事项

1. **目标平台**：Windows 10/11
2. **运行环境**：需要 Node.js 和 Rust 开发环境
3. **图标使用**：仅使用 Element Plus Icons，禁止 emoji
4. **代码规范**：使用 TypeScript 类型定义，遵循 Vue 3 组合式 API

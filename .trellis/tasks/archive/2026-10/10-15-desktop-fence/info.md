# 桌面收纳箱 — 实现分工与共享契约

> 需求见 `prd.md`。本文件是两条并行工作线（A 窗口层 / B 文件层）共同遵守的契约，以及之后集成阶段的依据。
> 任何一方需要改动契约，先在交付报告里说明，由主会话统一调整。

## 1. 分工

| 工作线 | 范围 | 工作目录 |
|---|---|---|
| **A 窗口层** | PR0 窗口 spike + PR1：v28 迁移与收纳箱元数据 CRUD、多窗口桌面嵌入泛化、托盘 3 个菜单项、FenceView 外壳（标题栏、移动 / 缩放、固定、折叠、悬停展开、改名、删除确认、外观、几何持久化与恢复）、窗口 e2e | 主仓库 `D:\Git\mini-todo` |
| **B 文件层** | PR0 图标 spike + PR2 的纯 Rust 服务：收纳根目录与文件夹命名、移入 / 移回 / 跨箱移动（`IFileOperation`）、目录列举、`notify` 监听、图标提取与缓存、启动 / 打开所在位置、OneDrive / 公共桌面判定 | 独立 git worktree（只改 Rust） |
| **集成（之后）** | 把 B 的服务包成 Tauri 命令、FenceView 项目网格、外部拖入（`onDragDropEvent`）、箱内排序、项目右键菜单、移动到…、Ctrl+滚轮图标大小、删除收纳箱时还原文件、改名时改文件夹、监听事件推给窗口、OneDrive 提示 | 主仓库 |

### 文件归属（避免冲突）

* **A 独占**：`pc/src-tauri/src/commands/window.rs`、`pc/src-tauri/src/lib.rs`、`pc/src-tauri/src/commands/mod.rs`、
  新建 `pc/src-tauri/src/commands/fence.rs`、`pc/src-tauri/src/db/**`、`pc/src-tauri/capabilities/default.json`、
  `pc/src-tauri/tauri.conf.json`、`pc/src/**`（前端全部）
* **B 独占**：新建 `pc/src-tauri/src/services/fence_fs/**`（或 `fence_fs.rs` + 子模块）、`pc/src-tauri/src/services/mod.rs`（只加一行 `pub mod`）
* **共享但需谨慎**：`pc/src-tauri/Cargo.toml`——B 负责新增 windows features / `png`；A 若确需新 feature 可追加，集成时由主会话合并
* B **不得**注册 Tauri 命令、不得改 `lib.rs` / `commands/` / `db/`；A **不得**实现文件移动 / 图标提取

## 2. 共享契约

### 2.1 标识与路由

* 收纳箱窗口 label：`fence-{id}`（id 为 `fences.id`）
* 前端路由：`#/fence?id={id}` → `pc/src/views/FenceView.vue`
* 收纳根目录：`%USERPROFILE%\MiniTodo收纳箱\`；每个收纳箱一个子文件夹，名称存在 `fences.folder_name`
* 图标缓存目录：`%LOCALAPPDATA%\mini-todo\fence-icons\`

### 2.2 数据库（A 负责迁移 **v29**）

> 2026-10-15 更正：本地 main 已快进到 origin/main（913461c，2.5.0），上游 v28 已是 `sync_meta` + `tombstones`，
> `pub const LATEST_VERSION: i32 = 28`。收纳箱迁移改为 **v29**，同时把 `LATEST_VERSION` 改为 29 并更新迁移测试。
> 迁移是表驱动的（`MIGRATIONS` 表），按上游写法追加。prd / research 中引用的旧行号以当前代码为准。
> 新表**不得**接入 `sync_store` 的变更计数 / 墓碑触发器（见 `.trellis/spec/backend/sync-protocol.md`）。

```sql
CREATE TABLE fences (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    title        TEXT    NOT NULL,
    folder_name  TEXT    NOT NULL UNIQUE,      -- 收纳根目录下的子文件夹名（由 B 的 API 生成）
    x            INTEGER NOT NULL,             -- 物理像素，虚拟屏幕坐标
    y            INTEGER NOT NULL,
    width        INTEGER NOT NULL,             -- 物理像素，展开态尺寸
    height       INTEGER NOT NULL,
    collapsed    INTEGER NOT NULL DEFAULT 0,
    locked       INTEGER NOT NULL DEFAULT 0,
    bg_color     TEXT    NOT NULL DEFAULT '#000000',
    bg_alpha     REAL    NOT NULL DEFAULT 0.45,
    icon_size    INTEGER NOT NULL DEFAULT 48,  -- 逻辑像素，集成阶段 Ctrl+滚轮调整
    sort_order   INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL
);
CREATE TABLE fence_items (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    fence_id      INTEGER NOT NULL REFERENCES fences(id) ON DELETE CASCADE,
    file_name     TEXT    NOT NULL,            -- 收纳文件夹内的实际文件名
    original_path TEXT,                        -- 移入前的完整路径；Explorer 直接放入的为 NULL
    sort_order    INTEGER NOT NULL DEFAULT 0,
    UNIQUE (fence_id, file_name)
);
```

```sql
-- 2026-10-15 追加（PRD R5）：v29 尚未发布，直接加进 v29
CREATE TABLE fence_layouts (
    fence_id         INTEGER NOT NULL REFERENCES fences(id) ON DELETE CASCADE,
    screen_config_id TEXT    NOT NULL,          -- 与 screen_configs.config_id 同格式
    x                INTEGER NOT NULL,          -- 物理像素
    y                INTEGER NOT NULL,
    width            INTEGER NOT NULL,          -- 物理像素，展开态尺寸
    height           INTEGER NOT NULL,
    updated_at       TEXT    NOT NULL,
    PRIMARY KEY (fence_id, screen_config_id)
);
```

* `fences.x/y/width/height` 保留为"最近一次用户调整的位置"，用作没有当前屏幕组合记录时的回退
* `fence_items` 只是**排序与原始路径**的附加信息，内容以文件夹为准；集成阶段按文件夹对账（文件夹有、表里没有 → 补行；表里有、文件夹没有 → 删行）
* 时间格式与现有表一致：`datetime('now', 'localtime')`
* 收纳箱数据**不进** `ExportData` / `SyncData`（设备相关，同 `screen_configs`）

### 2.3 窗口层创建收纳箱时对文件层的依赖

A 在 PR1 阶段还拿不到 B 的 API。约定：A 在 `commands/fence.rs` 里用一个临时的本地函数生成 `folder_name`
（例如 `fence-{id}` 或清洗后的标题），并在函数上标 `// INTEGRATION: 替换为 services::fence_fs::create_fence_folder`。
删除收纳箱、改标题同理，在对应位置留 `// INTEGRATION:` 注释，集成阶段统一替换。

### 2.4 文件层对外 API 的要求（B 实现，签名由 B 定，完成后写进 `fence-fs-api.md`）

* 与数据库无关：输入是文件夹名 / 路径，输出是结果结构体；错误用 `Result<_, String>`（中文错误信息，与项目其他命令一致）
* 可从任意线程调用：需要 COM STA 的操作（`IFileOperation`、`IShellItemImageFactory`、`ShellExecuteExW`）在内部专用 STA 线程执行
* 必须提供：
  * 根目录解析（不存在则创建）、标题 → 文件夹名清洗（非法字符、保留名如 `CON`、末尾点和空格、长度）与去重（` (2)` 后缀）
  * 创建 / 重命名 / 删除空的收纳文件夹
  * 列举收纳文件夹内容：文件名、显示名（去 `.lnk` / `.url`）、完整路径、是否文件夹、是否快捷方式；跳过 `desktop.ini` 和隐藏 / 系统文件
  * 移入：多个源路径 → 收纳文件夹，逐项返回结果（成功时含**源路径**与**收纳后的实际文件名**；失败 / 用户取消 UAC 分别标明）；支持传入 owner HWND 供系统对话框使用
  * 移回：收纳文件夹内某项 → 原目录（原目录不存在则回用户桌面），重名自动改名不覆盖，返回最终路径
  * 跨箱移动：A 箱某项 → B 箱，返回新文件名
  * 批量还原（删除收纳箱时用）：逐项结果 + 是否已清空文件夹
  * 判定：桌面是否被 OneDrive 重定向、某路径是否在公共桌面（后者首版未用上，审查后已删除；公共桌面路径仍用于"禁止整体收纳桌面"的保护）
  * 启动（`ShellExecuteExW`，`SEE_MASK_FLAG_NO_UI`，exe 设工作目录）、在资源管理器中定位（`/select,`）
  * 图标：路径 + 像素尺寸 → 缓存好的 PNG 文件路径（按路径哈希 + mtime + 尺寸做缓存键）
  * 监听：监听收纳根目录，变化时回调受影响的文件夹名（需去抖，例如 300ms 合并）
* 纯逻辑必须有单元测试；涉及真实文件的测试只在临时目录里进行

## 3. 开发专用数据隔离（2026-10-15 用户决定，A 实现）

用户日常运行着安装版 2.5.0（`D:\Program Files\mini-todo\mini-todo.exe`），上游启用了 single-instance 插件，且默认共用真实库与 WebDAV。
为让开发版与之并存且不碰真实数据：

* 环境变量 **`MINI_TODO_DATA_DIR`**：**仅 debug 构建**（`cfg(debug_assertions)`）生效，设置后 `db::paths::app_data_dir()` 返回该目录
  （库、图片、备份、日志等一并隔离；新库没有 WebDAV 配置，因此不会同步）；同时**不注册 single-instance 插件**
* 环境变量 **`MINI_TODO_FENCES_ROOT`**：**仅 debug 构建**生效，设置后收纳根目录改为该目录（B 在根目录解析里实现）
* release 构建完全忽略这两个变量，行为不变；需有单元测试或代码注释说明这一点
* 实测时两个 `mini-todo.exe` 同时存在：e2e 脚本按 **PID / exe 路径**定位开发版窗口，不得误操作安装版
* 实测期间不要切换托盘「开机自启动」（注册表项与安装版共用）
* 用户已授权：实测中可以重启 `explorer.exe`（可多次），结束后确认 Explorer 已恢复

## 4. 安全规则（两条线都必须遵守）

* **绝不移动、改名、删除用户真实的桌面文件**。e2e / spike 只用自己在桌面上新建的测试文件（文件名以 `__fence_e2e_` 开头），结束后清理干净
* 实测**必须**带 `MINI_TODO_DATA_DIR` 与 `MINI_TODO_FENCES_ROOT`（指向会话临时目录下的测试目录），不得让开发版打开真实库
  `%LOCALAPPDATA%\mini-todo\data.db`；不得操作用户正在运行的安装版进程，也不得改动它的窗口模式
* 测试数据目录、测试收纳根目录结束前清理
* 任何外部网络请求都**不得**携带用户邮箱或其他个人信息
* 不提交 git（由主会话统一提交）

# Mini-Todo 项目指南

本文档用于介绍 Mini-Todo 项目，帮助 AI 助手快速了解项目结构和开发规范。

## 项目简介

Mini-Todo 是一款基于 **Tauri 2.x + Vue 3 + TypeScript** 开发的 Windows 桌面待办事项管理应用，定位为简洁、聚焦的本地待办工具，支持子任务、四象限、日历、重复提醒、系统通知与 WebDAV 云同步，另有收纳桌面图标的桌面收纳箱。

仓库按平台拆子目录，`pc/` 是 Windows 桌面端，`cloud/` 是云端 HTTP API（mini-todo 在远程 VPS 上的复刻，通过同一个 WebDAV 通道与 PC 同步数据，供 AI / Claude Code Skill 通过 REST 读写）。详见 `cloud/README.md`。

`pc-gpui/` 是用 **Rust + GPUI**（`gpui-kit =0.7.3` = gpui-pre 0.3.9 + gpui-component 0.7.3）重写的 PC 端，**不依赖 WebView2**，
元素 / 样式 / 功能与 `pc/` 一致，目标是取代 `pc/`。`pc/` 暂时保留，待用户在 Windows 上实机验收 GPUI 版后再删除。两套 PC 实现**数据完全兼容**
（同一数据目录、同一数据库迁移、同一同步文档、同一导出格式、同一 settings 键），关系与双改规则见下文「pc-gpui（GPUI 版 PC 端）」一节。

## 技术栈

### `pc/`（Tauri 版）

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| 前端框架 | Vue 3 + TypeScript | 组合式 API，类型安全 |
| UI 组件库 | Element Plus | `unplugin-vue-components` 按需引入；图标 @element-plus/icons-vue 逐个 SFC 显式导入 |
| 富文本编辑 | Milkdown 7.22.x | 描述 / 子任务 Markdown 编辑器；`prosemirror-model` 经 overrides 钉在 1.25.11（1.25.12 校验 `title: null`，会静默丢弃无标题图片） |
| 状态管理 | Pinia | Vue 官方推荐状态管理 |
| 桌面框架 | Tauri 2.x | tauri 2.9.5；插件 single-instance / log / notification / dialog / fs / opener / autostart |
| 后端语言 | Rust | 高性能，内存安全 |
| 数据库 | SQLite (rusqlite) | 轻量级本地数据库 |
| 拖拽功能 | vuedraggable | Vue 拖拽排序库（vite 把 `vue` 精确别名到 runtime 构建，避免带入模板编译器） |
| 异步运行时 | Tokio | Rust 异步任务调度 |
| WebDAV 客户端 | reqwest | 远端同步（blocking 客户端，只在 `spawn_blocking` 中使用） |
| 前端单测 | vitest | `npm run test`，`src/**/*.test.ts` |
| 端到端验证 | selenium + tauri-driver | `pc/scripts/e2e-linux/`（Linux 真实应用 + WebDAV + cloud），`pc/scripts/e2e/`（Windows 窗口模式 / 桌面收纳箱） |

### `pc-gpui/`（GPUI 版）

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| UI 框架 | gpui-kit `=0.7.3`（gpui-pre 0.3.9 + gpui-base / gpui-component 0.7.3） | Zed 的 GPUI 每周快照，**精确钉版本**；Windows 走 DirectX 11 + DirectComposition（透明窗口），Linux 走 wgpu（Vulkan，CI / Xvfb 用 mesa lavapipe） |
| 控件 | 自绘 Element Plus 风格控件（`crates/app/src/ui/`） | 外观数值照抄旧版；复杂输入基于 gpui-component（`InputState` 等），主题调成 EP（`#409EFF`） |
| 图标 | Element Plus Icons SVG | 从 `@element-plus/icons-vue` 抽取，`crates/app/assets/icons/{ep,app}/`，经 `assets.rs` 嵌入 |
| Markdown | gpui-base `TextView` + 自写块级排版（`markdown` crate） | 编辑为"源码 + 预览"（已知偏差，见下文） |
| 业务核心 | `mini-todo-core`（`crates/core`） | 由 `pc/src-tauri/src` 复制：rusqlite 0.32 bundled、reqwest 0.12 blocking、windows 0.58；不依赖 GPUI / Tauri / tokio，函数全部阻塞 |
| 农历 | tyme4rs | lunar-javascript 同一作者的 Rust 库，结果与旧前端逐日比对 |
| 平台集成 | windows 0.62（与 gpui 同版本）、tray-icon、notify-rust、auto-launch、rfd、embed-resource | 托盘、Toast、开机自启（与旧版同名同键）、带过滤器的文件对话框、exe 图标；单实例与旧版 Tauri 插件同名同格式 |
| 测试 | cargo test、`scripts/regress.py`（Xvfb 截图对照 + 冒烟）、`scripts/e2e-linux/`、`scripts/e2e-windows/` | 见 `.trellis/spec/pc-gpui/verification.md` |

## 项目结构

> 项目按平台拆分子目录。`pc/` 为 Windows 桌面端（Tauri），`pc-gpui/` 为 GPUI 重写版（将取代 `pc/`），未来可平行加入 `mobile/`、`web/`。

```
mini-todo/
├── docs/                                   # 共享文档（跨平台）
│   └── 开发文档/                            # 开发相关文档
├── pc/                                     # PC 端（Tauri 2.x + Vue 3）
│   ├── src/                                # Vue 前端源码
│   │   ├── assets/                         # 静态资源
│   │   ├── components/                     # Vue 组件
│   │   │   ├── CalendarView.vue            # 日历视图（MainView 中 defineAsyncComponent 懒加载）
│   │   │   ├── FenceItemGrid.vue           # 收纳箱项目网格（图标懒加载、打开、自绘右键菜单、箱内排序、Ctrl+滚轮）
│   │   │   ├── MarkdownEditor.vue          # 可复用 Milkdown 编辑器（编辑/只读、图片上传、链接白名单）
│   │   │   ├── QuadrantView.vue            # 四象限视图
│   │   │   ├── TitleBar.vue                # 标题栏
│   │   │   ├── TodoItem.vue                # 待办项组件
│   │   │   └── TodoList.vue                # 待办列表
│   │   ├── plugins/element.ts              # ElMessage / ElMessageBox 唯一入口（含样式）
│   │   ├── components.d.ts                 # unplugin-vue-components 生成，需提交
│   │   ├── router/                         # 路由配置
│   │   ├── stores/                         # Pinia 状态管理
│   │   │   ├── appStore.ts                 # 应用全局状态
│   │   │   └── todoStore.ts                # 待办状态（fetchTodos 单飞 + 过期响应丢弃）
│   │   ├── types/                          # TypeScript 类型定义
│   │   │   ├── app.ts                      # 应用类型（含 SyncReport / SyncSettings）
│   │   │   ├── fence.ts                    # 收纳箱类型与前后端共用常量
│   │   │   └── todo.ts                     # 待办类型
│   │   ├── utils/                          # 工具函数（*.test.ts 为 vitest 单测）
│   │   │   ├── datetime.ts                 # 时间解析/格式化，写入统一 YYYY-MM-DD HH:MM:SS
│   │   │   ├── imageRef.ts                 # Markdown 图片引用 minitodo-image://<name> ⇄ 本机 asset URL
│   │   │   ├── fileLink.ts                 # 链接点击协议白名单
│   │   │   ├── editorShortcuts.ts          # 编辑类窗口 Esc / Ctrl+Enter
│   │   │   ├── fence.ts                    # 收纳箱纯函数（几何落库闸门、缩放矩形、图标档位、提示文案）
│   │   │   ├── notify.ts                   # notifyError 统一失败提示
│   │   │   ├── syncReport.ts               # SyncReport 提示文案与刷新判断
│   │   │   ├── quadrant.ts / color.ts      # 象限与颜色
│   │   │   ├── holiday.ts                  # 节假日工具
│   │   │   └── lunar.ts                    # 农历工具
│   │   ├── views/                          # 页面视图
│   │   │   ├── CompletedView.vue           # 已完成视图
│   │   │   ├── EditorView.vue              # 待办编辑主视图
│   │   │   ├── FenceView.vue               # 桌面收纳箱窗口（独立 WebView，label fence-{id}，路由 #/fence?id=）
│   │   │   ├── MainView.vue                # 主视图（待办列表）
│   │   │   ├── SubtaskEditorView.vue       # 子任务编辑视图（独立 WebView）
│   │   │   ├── NotificationView.vue        # 通知视图
│   │   │   └── SettingsView.vue            # 设置视图
│   │   ├── App.vue                         # 根组件
│   │   └── main.ts                         # 入口文件
│   ├── src-tauri/                          # Tauri/Rust 后端源码
│   │   ├── src/
│   │   │   ├── commands/                   # Tauri 命令（前后端桥接）
│   │   │   │   ├── data.rs                 # 数据导入导出、AppSettings 读写（唯一来源）
│   │   │   │   ├── fence.rs                # 收纳箱元数据 CRUD、窗口生命周期、按屏幕组合记忆位置
│   │   │   │   ├── fence_items.rs          # 收纳箱内容：对账、移入/移回/跨箱、打开、图标、目录监听
│   │   │   │   ├── holiday.rs              # 节假日命令
│   │   │   │   ├── notification_cmd.rs     # 通知命令
│   │   │   │   ├── settings_cmd.rs         # 设置命令
│   │   │   │   ├── sync_cmd.rs             # WebDAV 同步（webdav_sync / force_pull / force_push）
│   │   │   │   ├── todo.rs                 # 待办 CRUD 命令
│   │   │   │   └── window.rs               # 窗口管理命令
│   │   │   ├── db/                         # 数据库层
│   │   │   │   ├── connection.rs           # 连接管理、迁移前备份、外键处理
│   │   │   │   ├── migrations.rs           # 数据库迁移（v1~v29，迁移表驱动）
│   │   │   │   ├── models.rs               # 数据模型定义
│   │   │   │   ├── settings_kv.rs          # settings 读写助手（只在值变化时刷新 updated_at）
│   │   │   │   ├── sync_store.rs           # 变更计数、墓碑、记录读写与规范化
│   │   │   │   ├── time.rs                 # 规范时间格式解析/规范化
│   │   │   │   ├── paths.rs                # 数据 / 图片目录、图片文件名校验（debug 构建可用 MINI_TODO_DATA_DIR 隔离）
│   │   │   │   └── backup.rs               # VACUUM INTO 备份（保留最近 5 份）
│   │   │   ├── services/                   # 业务服务层
│   │   │   │   ├── fence_fs/               # 收纳箱文件层（与数据库无关）：命名、IFileOperation 移动、图标提取缓存、notify 监听、打开
│   │   │   │   ├── notification.rs         # 通知服务（含定时调度、应用内通知窗口定位）
│   │   │   │   ├── webdav.rs               # WebDAV 客户端（条件 GET/PUT、PROPFIND）
│   │   │   │   └── secret.rs               # WebDAV 密码保护（Windows DPAPI）
│   │   │   ├── lib.rs                      # 库入口（插件注册、托盘、轮询线程）
│   │   │   └── main.rs                     # 主入口
│   │   ├── Cargo.toml                      # Rust 依赖配置
│   │   └── tauri.conf.json                 # Tauri 配置（含 CSP 与 asset 协议范围）
│   ├── scripts/
│   │   ├── e2e/                            # Windows 窗口模式 / 收纳箱 e2e 工具（e2e.ps1、fence.ps1、cdp.mjs 等）
│   │   └── e2e-linux/                      # Linux 端到端验证套件（见其 README）
│   ├── public/                             # 公共静态资源
│   ├── index.html                          # Vite 入口 HTML
│   ├── package.json                        # Node 依赖配置
│   ├── tsconfig.json                       # TypeScript 配置
│   ├── vitest.config.ts                    # 前端单测配置
│   └── vite.config.ts                      # Vite 构建配置
├── pc-gpui/                                # PC 端 GPUI 重写版（Cargo workspace，将取代 pc/；详见各 crate README）
│   ├── Cargo.toml                          # [workspace] members = crates/core、crates/app；版本 2.5.0；release thin LTO + strip
│   ├── Cargo.lock                          # 提交；CI 一律 --locked
│   ├── crates/
│   │   ├── core/                           # mini-todo-core：业务核心（不依赖 GPUI / Tauri / tokio，全部阻塞函数）
│   │   │   ├── README.md                   # 公开 API、线程约束、旧命令 → 新函数对照
│   │   │   ├── src/
│   │   │   │   ├── db/                     # ← pc/src-tauri/src/db 原样复制（迁移 v1~v29、sync_store、time、settings_kv、paths…）
│   │   │   │   ├── services/               # ← 原样复制：webdav.rs、secret.rs（DPAPI）、fence_fs/（收纳箱文件层）
│   │   │   │   ├── todo.rs / data.rs       # ← commands/todo.rs、data.rs（导入导出、AppSettings 读写）
│   │   │   │   ├── sync/                   # ← commands/sync_cmd.rs：sync::run(db, SyncMode) -> SyncReport（测试在 sync/tests.rs）
│   │   │   │   ├── settings.rs             # ← settings_cmd.rs + window.rs 的设置 / screen_configs + 屏幕组合 ID
│   │   │   │   ├── holiday.rs / reminder.rs  # 节假日（blocking）/ 提醒调度纯逻辑与通知窗口槽位、定位
│   │   │   │   ├── window_logic.rs         # 贴边隐藏状态机、needs_reattach、lacks_tool_window_style 等纯函数
│   │   │   │   ├── lunar.rs                # 新增：农历（tyme4rs），等价于 pc/src/utils/lunar.ts
│   │   │   │   └── fence/                  # ← fence.rs / fence_items.rs 中与窗口无关的部分（几何、对账、移入移回）
│   │   │   ├── examples/seed.rs            # 把导出格式 4.0 的 JSON 导入隔离数据目录（截图 / e2e 用）
│   │   │   └── tests/fixtures/             # 农历对照数据（gen_lunar.cjs 用 lunar-javascript 生成）
│   │   └── app/                            # mini-todo：GPUI 界面、窗口、平台集成（lib mini_todo + bin mini-todo）
│   │       ├── README.md                   # 模块地图、约定、接口表、调试开关（维护者必读）
│   │       ├── build.rs                    # Windows：嵌 exe 图标（资源 ID 1）+ 版本信息；manifest 由 gpui 嵌，不要重复
│   │       ├── assets/                     # icon.ico；icons/ep/*.svg（Element Plus）、icons/app/*.svg（自定义）
│   │       └── src/
│   │           ├── lib.rs / main.rs        # run()：日志 → 单实例 → 数据库 → Application(QuitMode::Explicit) → 主窗口 → 后台服务
│   │           ├── theme.rs                # 颜色 / 字体 / 圆角 / 间距 / 阴影 token 的唯一来源（按窗口分节）
│   │           ├── state.rs                # AppState（Entity + Global）：替代 Pinia store 与跨 WebView 事件
│   │           ├── db.rs                   # Db 全局（Arc<Database>）、spawn_db（阻塞调用放后台）
│   │           ├── assets.rs               # EpIcon / AppIcon 嵌入表
│   │           ├── util/                   # 旧前端 utils 的 1:1 移植（datetime、image_ref、file_link、editor_shortcuts）
│   │           ├── windows/                # WindowManager（mod.rs，含 DebugOpen）+ main_window/、editor/、subtask、completed、
│   │           │                           # notification、settings/、fence/、gallery（控件样例，仅调试）
│   │           ├── ui/                     # Element Plus 风格共享控件（button、input、ep_*、message、message_box、markdown*…）
│   │           ├── services/               # 托盘、单实例、开机自启、同步编排、自动同步、提醒调度、Toast、窗口模式、收纳箱、版本检查、日志
│   │           └── platform/               # GPUI 没有的窗口能力：windows.rs（Win32）、fallback.rs（Linux）、embed.rs、subclass.rs…
│   └── scripts/
│       ├── README.md                       # regress.py / shot.sh / MINI_TODO_DEBUG_OPEN 说明
│       ├── regress.py                      # Xvfb 一键回归：场景截图 + 与参考截图并排对照 + 端到端冒烟
│       ├── shot.sh                         # 单个场景截图 / xdotool 交互
│       ├── sample-data.json                # 样例数据（导出格式 4.0）
│       ├── e2e-linux/                      # Linux 端到端（真实应用 + Apache / nginx WebDAV + cloud + Skill）
│       └── e2e-windows/                    # Windows 端到端（真实窗口 + 真实输入 + Win32 断言；CI pc-gpui-e2e-windows.yml）
├── cloud/                                  # 云端 HTTP API（独立 Rust crate）
│   ├── Cargo.toml                          # axum + tokio + rusqlite + reqwest + ...
│   ├── config.example.toml                 # 配置示例
│   ├── README.md                           # 部署 / 配置说明
│   ├── deploy/
│   │   ├── minitodo-cloud.service          # systemd unit 示例（沙箱加固）
│   │   └── Caddyfile.example               # Caddy 反代示例
│   ├── skill/minitodo/                     # Claude Code Skill：SKILL.md + minitodo.py + 安装脚本
│   └── src/
│       ├── main.rs                         # tokio + axum 启动、优雅停机
│       ├── config.rs                       # config.toml 解析
│       ├── time.rs                         # 与 PC 一致的时间格式（按时区实时换算）
│       ├── model.rs                        # 写入字段校验与别名归一化（与 PC 模型一致）
│       ├── db/                             # rusqlite + KV-style schema（normalize.rs：存量归一化）
│       ├── sync/                           # WebDAV 客户端、pull/push、merge、doc、worker、图片镜像
│       └── api/                            # axum router + Bearer auth + /health
├── CLAUDE.md                               # 项目指南（本文档）
├── README.md
└── AGENTS.md
```

## 核心功能

### 待办管理
- 创建、编辑、删除待办事项
- 描述字段支持 Markdown（Milkdown 编辑器，支持图片粘贴/拖入上传，存于 `description` 字段）；
  支持粘贴 MD 源码自动解析（clipboard 插件）、GFM 表格/任务清单/删除线，编辑模式提供源码/预览分栏的放大编辑弹窗（联动窗口最大化）
- Markdown 中的图片统一存为 `minitodo-image://<文件名>`（`utils/imageRef.ts`），渲染时换成本机 images 目录的
  asset URL；兼容旧数据里的 `http://asset.localhost/<绝对路径>` / `asset://localhost/...`（按文件名映射到本机，
  修复跨设备裂图）。上传走 raw body：`invoke('save_subtask_image', bytes, { headers: { 'x-image-ext': 'png' } })`
  （IPC 退回 postMessage 时的数字数组同样接受），扩展名白名单 png/jpg/jpeg/webp/gif/bmp、单张 ≤20MB，文件名由后端生成
- 链接点击走协议白名单（`utils/fileLink.ts`）：http/https/mailto 交给系统程序，`file:///` 在资源管理器定位
  （拒绝 UNC），其余（javascript:/data: 等）一律拦截且不让 WebView 导航
- 编辑类窗口支持 Esc 关闭（有未保存修改时确认）、Ctrl/Cmd+Enter 保存；保存有防重入
- 只读详情模式：从列表/四象限/日历/已完成视图点击待办默认进入（`#/editor?id=x&mode=view`），
  左侧简化展示标题、Markdown 渲染描述、通知状态、优先级，右侧子任务面板功能照常，[编辑] 按钮原地切换
- 支持一级子任务（含 Markdown 详情、图片上传）
- 四象限分类（重要紧急 / 重要不紧急 / 紧急不重要 / 不紧急不重要）
- 自定义颜色标识
- 完成状态标记
- 拖拽排序
- 开始/截止时间

### 子任务
- 标题 + Markdown 内容（Milkdown 富文本编辑器）
- 支持图片粘贴/拖入上传
- 独立 WebView 详情窗口（编辑/查看双模式）
- 完成态切换、排序

### 通知提醒
- Windows 系统通知 / 应用内通知
- 预设提前提醒（5/15/30 分钟）
- 自定义提前时间
- 应用内通知窗口按显示器 work area 与缩放比例换算逻辑坐标，自下而上叠放、超出换列；按窗口实际尺寸（平台可能强加最小高度）
  重新定位并记住，保证不出屏

### 重复提醒
- 按天 / 周 / 周几 / 月几号循环
- 触发后自动推进到下一次提醒时间点（月重复缺日期时以 notify_at 的日为锚，月末自动钳位）
- 应用启动时补发错过的重复提醒
- 提醒时间兼容空格 / `T` 两种格式；单条提醒出错不影响其它提醒

### 视图模式
- **列表视图**：按排序/优先级展示
- **四象限视图**：拖拽分类
- **日历视图**：按日期展示（含农历、节假日）

### 数据导入导出
- 导出版本：`4.0`（位于 `pc/src-tauri/src/commands/data.rs`）
- 导出为 ZIP 压缩包（内含 `data.json`）
- 导入兼容 v3.0 和 v4.0 两个版本
  - v3.0 是历史导出格式（含已移除的 AI Agent 字段），新版本反序列化时通过 `#[serde(default)]` 静默忽略多余字段
- 直接 JSON 导入也支持
- 导入是"恢复备份"语义：导入前自动 `VACUUM INTO` 备份到 `backups/`；保留原 id；导入后消失的记录写墓碑，
  被导入记录的 `updated_at` 刷新为当前时间（保证恢复结果在同步中胜出）

### WebDAV 云同步
- 智能同步 `webdav_sync`：先 GET 远端并逐条合并（LWW + 墓碑），本地有变化才条件 PUT；手动按钮、设置页
  「立即同步」与自动同步共用（手动传 `{ full: true }` 无条件 GET；自动同步每 10 轮也做一次无条件 GET）。另有带二次确认的「用云端覆盖本地」`webdav_force_pull`、「用本地覆盖云端」
  `webdav_force_push`。不再有整库二选一的冲突对话框
- 同步范围：todos / subtasks / 墓碑 / 10 个应用设置项（按 `settingsUpdatedAt` 做 LWW，窗口位置与尺寸不从远端应用）/ 图片
- 自动同步可选（按间隔轮询）；同步命令在 `spawn_blocking` 中执行，不阻塞 UI 线程；同一时刻只允许一个同步
- gzip 压缩传输；**数据本身不加密**（传输安全取决于 https，WebDAV 服务商可读取内容）；
  WebDAV 密码在 Windows 上用 DPAPI 加密保存，前端只拿到 `hasPassword`，http:// 地址会提示明文风险

### 窗口特殊功能
- **普通模式**：浅色主题，可拖拽移动
- **固定模式**：
  - 透明背景
  - 固定在用户指定位置
  - 忽略 Win+D（显示桌面）
  - 禁用关闭、最小化、拖拽
- **固定模式时嵌入桌面**（设置 → 常规「固定模式时，嵌入桌面中」，仅 Windows 可见，Win11 24H2+ 保证 Win+D 免疫）：
  - 不是第三种模式，而是固定模式的一个全局开关：开启后进入固定模式即走后端"桌面模式"路径，
    关闭则是原来的固定模式；当前已固定时切换开关立即生效（`set_fixed_embed_desktop`）
  - 主窗口仍是顶层窗口：owner=Progman（`GWLP_HWNDPARENT`）+ tao `always_on_bottom` +
    `set_minimizable(false)` + `WS_EX_TOOLWINDOW`，位于桌面图标之上、所有应用窗口之下
  - Win+D / Win+M / 任务栏"显示桌面"时随 Progman 一起被抬起，不最小化、不被盖住
  - 透明背景 / 圆角 / DPI 与普通模式零差异（不做 SetParent 子窗口化；主窗口不挂 `body.fixed-mode`）
  - 不可拖拽、不可缩放；不做贴边隐藏 / 唤起置顶；不在任务栏、不参与 Alt+Tab
  - 200ms 轮询线程检测 Explorer 重启（`tick_desktop_mode`），owner 失效时自动重挂
  - 入口不变：TitleBar 锁按钮、托盘 CheckMenuItem「固定模式」（嵌入态托盘同样勾选）
  - 持久化：settings `fixed_embed_desktop`（v27）；当前是否固定仍由 `is_fixed` / `screen_configs.is_fixed` 记录
  - 后端：`commands/window.rs` `IS_DESKTOP_MODE` / `set_window_desktop_mode` / `desktop_attach` / `desktop_detach`；
    前端 `appStore.applyFixedMode` 按开关选择命令；与固定模式共用 `reassert_window_mode_state`
    兜底 tao `apply_diff` 的 ex style 覆写
  - 非 Windows：开关不显示；前端只在 `IS_WINDOWS && fixedEmbedDesktop` 时走桌面模式命令，
    后端 `set_window_desktop_mode(true)` 回退为普通固定模式并返回 Ok（同步来的开关不会让固定失败）
  - 启动竞态兜底：200ms 轮询线程发现固定 / 嵌入态的 ex style 被晚到的 tao 操作改成 APPWINDOW 时自愈
    （`lacks_tool_window_style`，`tick_desktop_mode` / `tick_fixed_style`），残留约 200ms 任务栏闪烁

### 桌面收纳箱（仅 Windows）
- 托盘「新增收纳箱」在光标所在显示器工作区的右上角找空位新建（默认 4 列 × 3 行、标题"收纳箱 / 收纳箱 2…"）。每个收纳箱是一个
  无边框透明的独立 WebView 窗口（label `fence-{id}`，路由 `#/fence?id={id}` → `FenceView.vue`）
- **始终嵌入桌面层**：owner=Progman + tao `always_on_bottom` + `WS_EX_TOOLWINDOW`（与主窗口嵌入共用 `desktop_attach_hwnd`），
  Win+D 后可见、不进任务栏 / Alt+Tab、不盖住应用窗口；tao 标志建窗时一次给齐，`show()` / `hide()` 后必须 `reassert_embedded_window`；
  `EMBEDDED_WINDOWS` 登记 + 200ms `tick_embedded_windows` 在 Explorer 重启后自动重挂
- **真收纳**：拖入（`onDragDropEvent`）的文件 / 快捷方式 / 文件夹经 `IFileOperation` 从桌面移到 `%USERPROFILE%\MiniTodo收纳箱\<收纳箱名>\`，
  重名加 ` (2)` 绝不覆盖，公共桌面项弹 UAC；**文件夹是内容的 source of truth**（`notify` 监听，Explorer 里增删随之刷新），
  数据库只存元数据与项目的排序 / 原始路径。不允许整体收纳用户桌面 / 公共桌面文件夹本身
- 双击打开（`ShellExecuteExW`）、自绘右键菜单（打开 / 打开所在位置 / 移回桌面 / 移动到…）、箱内拖动排序、Ctrl+滚轮图标大小、
  双击标题改名（收纳文件夹同步改名，失败则保留旧文件夹名并提示）、折叠（悬停 / 拖文件经过时临时展开）、
  「固定」= 锁定位置和大小（仍可拖入、折叠、改名）、每个收纳箱独立的背景色 / 透明度
- 移动 / 缩放：标题栏超过 3px 才 `startDragging`（不用 `data-tauri-drag-region`），缩放用前端手柄 + `set_fence_bounds`
  （窗口是 `resizable(false)`）；只有用户拖动 / 缩放结束与托盘重置才 `commit_fence_geometry` 落库，系统挪窗不落库
- 删除收纳箱：项目全部移回原位置（原目录不存在则回桌面）、空文件夹删除后才删记录并关窗口；有项目移不回去时删除中止、
  整个收纳箱保留并提示原因
- 按屏幕组合记忆位置（`fence_layouts`，ID 与 `screen_configs.config_id` / `generateScreenConfigId()` 同格式），约 2 秒检测一次
  屏幕组合变化；设置里删除屏幕配置时连带删除对应布局
- 托盘「显示/隐藏所有收纳箱」「重置收纳箱位置」作为兜底入口；首次从 OneDrive 重定向的桌面收纳时提示一次
- 关键实现：`commands/fence.rs`、`commands/fence_items.rs`、`services/fence_fs/`、`commands/window.rs`（嵌入附属窗口一节）、
  `views/FenceView.vue`、`components/FenceItemGrid.vue`；契约与易错点见 `.trellis/spec/backend/desktop-fences.md`
- 已知限制：Win10 / Win11 ≤23H2 上 Win+D 可能盖住收纳箱；每个收纳箱一个 WebView2 渲染进程（开发版每个约 +45~60MB，
  暂不加 `--process-per-site`）

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

### 数据库设计
- **数据库类型**：SQLite
- **存储位置**：`%LOCALAPPDATA%\mini-todo\`（`dirs::data_local_dir()`；Linux 为 `~/.local/share/mini-todo/`）
  - `data.db`、`images/`（用户图片）、`backups/`（迁移/导入前的 `VACUUM INTO` 快照，保留最近 5 份）、`holidays/`、
    `fence-icons/`（收纳箱图标 PNG 缓存，启动对账后清理过期项）
  - 日志：`%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（tauri-plugin-log，2MB 轮转）
  - 收纳箱文件：`%USERPROFILE%\MiniTodo收纳箱\<收纳箱文件夹>\`（用户可见的真实文件夹，有收纳箱后才创建；不在数据目录里，
    卸载 / 删库都不会丢文件）
  - GPUI 版（`pc-gpui/`）用**完全相同**的数据目录、收纳根目录与日志文件（日志由 app 的 `services::logging` 写，同样 2MB 轮转）
- **迁移版本**：当前 v1~v29，通过 `pc/src-tauri/src/db/migrations.rs` 的迁移表管理（`pc-gpui/crates/core/src/db/migrations.rs`
  是逐字相同的副本，两边必须同号同 SQL）；有待执行迁移时先备份，迁移期间关闭外键、提交前 `foreign_key_check`
  - v23：移除所有 AI Agent / 任务调度 / 工作流相关表和字段（详见迁移注释）
  - v24：新增 `webdav_last_modified` settings key
  - v25：新增 `top_on_wake` settings key，贴边唤起时是否临时置顶
  - v26：新增 `window_bg_color` / `window_bg_alpha` settings key，窗口底色与背景透明度
  - v27：新增 `fixed_embed_desktop` settings key，固定模式时是否嵌入桌面（仅 Windows）
  - v28：`sync_meta`（本地变更计数 `local_seq` / `synced_seq`，todos/subtasks 增删改触发器维护）、
    `tombstones` 表、存量时间统一为 `YYYY-MM-DD HH:MM:SS`、新增 `webdav_remote_etag` / `webdav_synced_settings_at`
  - v29：桌面收纳箱 `fences` / `fence_items` / `fence_layouts` 三张表（后两张 `ON DELETE CASCADE`）与 settings key
    `fence_onedrive_notice_shown`；均为设备本地数据，不接 `local_seq` 触发器与墓碑、不导出不同步

#### 主要数据表

| 表名 | 说明 |
|------|------|
| `todos` | 待办事项（含重复提醒字段） |
| `subtasks` | 子任务（标题 + Markdown 内容 + 完成态） |
| `settings` | 应用设置（键值对，带 `updated_at`，只经 `db::settings_kv` 写入） |
| `screen_configs` | 屏幕配置 |
| `sync_meta` | 同步计数器（`local_seq` / `synced_seq`），由触发器维护 |
| `tombstones` | 删除墓碑 `(entity_type, entity_id, deleted_at)`，保留 30 天 |
| `fences` | 桌面收纳箱元数据（标题、收纳文件夹名、物理像素几何、折叠 / 锁定、外观、图标大小） |
| `fence_items` | 收纳箱项目的排序与原始路径（内容以收纳文件夹为准，列举时对账） |
| `fence_layouts` | 收纳箱按屏幕组合记忆的位置 `(fence_id, screen_config_id)` |
| `migrations` | 迁移版本记录 |

#### 时间格式约定（PC / cloud / 前端一致）

- 存储与同步一律 `YYYY-MM-DD HH:MM:SS`（本地墙钟、无时区）：`notifyAt` / `startTime` / `endTime` /
  `createdAt` / `updatedAt` / 墓碑 `deletedAt` / `settingsUpdatedAt`
- 读取兼容空格 / `T`、有无秒、小数秒、`Z`/`±HH:MM` 后缀（换算本地）、仅日期（开始 00:00:00、截止 23:59:00、提醒 09:00:00）
- Rust 用 `db::time`，前端用 `utils/datetime.ts`；**禁止** `split('T')` 之类的手写解析
- 对已有记录写 `updated_at` 时取 `max(now, 旧值 + 1 秒)`（Rust `db::time::superseding`，SQL 片段 `SQL_SET_UPDATED_AT`），
  保证同一记录的新版本时间戳严格递增（秒级 LWW 下同秒两次编辑不丢）；参与同步的设置项按整个设置块的版本同样递增；
  删除墓碑 `deleted_at = max(now, 记录 updated_at)`；无法解析的时间在比较中视为最旧

### 数据导入导出与同步

> **重要**：当数据库结构变更（新增表/字段/设置项）时，必须同步更新导入导出、WebDAV 同步与 cloud 的字段校验！

- **导出版本号**：当前 `4.0`（位于 `pc/src-tauri/src/commands/data.rs`）
- **关键文件**：
  - 模型定义：`pc/src-tauri/src/db/models.rs` → `ExportData`、`AppSettings`
  - 设置读写（唯一来源）：`pc/src-tauri/src/commands/data.rs` → `read_app_settings` / `write_app_settings`；
    参与同步的键 `SYNCED_SETTING_KEYS` 与设置版本 `settings_version` 在 `db/settings_kv.rs`
  - 导入导出：`data.rs` → `export_data_internal`、`import_data_raw`（仅手动导入导出使用）
  - WebDAV 同步：`commands/sync_cmd.rs` → `SyncData`、`run_sync_blocking`、`build_sync_doc`；
    记录读写/合并/墓碑：`db/sync_store.rs`（`merge_remote`、`load_todos_with_subtasks`）
  - cloud 写入校验：`cloud/src/model.rs`（新增记录字段必须同步加入允许字段表，否则 AI 写入会被 400）
  - 前端类型：`pc/src/types/todo.ts` → `ExportData`（前端只传递 JSON 字符串，无需严格同步）
  - GPUI 版对应文件（`pc-gpui/crates/core/src/`，与 `pc/` 并存期间两边一起改）：`db/models.rs`、`db/settings_kv.rs`、`db/sync_store.rs`
    （原样复制）；`data.rs` → `read_app_settings` / `write_app_settings` / `export_json` / `import_json` / `export_to_file` /
    `import_from_file`（`EXPORT_VERSION`）；`sync/mod.rs` → `SyncData`、`run_sync_blocking`、`build_sync_doc`、`sync::run`。
    界面侧没有前端类型，`crates/app` 直接用 core 的 Rust 模型

#### sync-data.json.gz 结构

```jsonc
{
  "version": "4.0", "deviceId": "…", "updatedAt": "…",        // 元信息
  "todos": [ /* Todo，嵌套 subtasks */ ],
  "settings": { /* AppSettings */ }, "settingsUpdatedAt": "YYYY-MM-DD HH:MM:SS",
  "images": ["<安全文件名>"],
  "tombstones": [ { "entityType": "todo" | "subtask", "entityId": 123, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ]
  // 其它顶层键：所有写入方（PC / cloud）必须原样保留
}
```

#### 合并规则（PC `sync_store::merge_remote` 与 cloud `sync/merge.rs` 一致）

1. 记录级 LWW：比较规范化后的 `updatedAt`，**平局保留本地**
2. 墓碑：同 `(entityType, entityId)` 且 `deletedAt >= updatedAt` 时删除/压制记录；todo 墓碑连带子任务；
   删除后又被编辑（`updatedAt > deletedAt`）的记录保留
3. 只在一侧存在的记录保留（并集），**不做"缺席即删除"**（cloud 仅对没有 `tombstones` 键的旧版文档、且自身不 dirty 时保留旧行为）
4. 墓碑取并集、同键取较大 `deletedAt`，保留 30 天
5. 设置：远端 `settingsUpdatedAt` 比本地同步键的 `max(updated_at)` 新才应用；`windowPosition` / `windowSize` 不从远端应用；
   从未同步过的新设备直接采用远端设置
6. 远端记录反序列化失败只跳过并计数（`recordsSkipped`），不当作删除；远端文档整体解析失败直接报错、不上传

#### 当前同步覆盖范围

| 数据 | 是否同步 | 说明 |
|------|---------|------|
| `todos`（全字段） | 是 | 含重复提醒字段 |
| `subtasks`（全字段） | 是 | 标题 + Markdown 内容 + 完成态 |
| `tombstones` | 是 | 删除双向传播，保留 30 天 |
| `settings`（部分） | 是 | `SYNCED_SETTING_KEYS` 的 10 项（含 `is_fixed` / `fixed_embed_desktop`），按 `settingsUpdatedAt` LWW；不含 WebDAV 配置；窗口位置/尺寸只随导出备份 |
| `images`（文件） | 是 | 先传图片再传 sync-data；一次 PROPFIND 列远端清单；文件名经 `is_safe_image_name` 校验 |
| `screen_configs` | 否 | 设备特定的屏幕配置 |
| `fences` / `fence_items` / `fence_layouts` | 否 | 桌面收纳箱：路径与屏幕坐标是设备相关数据，性质同 `screen_configs`；也不进导入导出 |
| settings `fence_onedrive_notice_shown` | 否 | 设备本地提示标记，有意不进 `AppSettings` / `SYNCED_SETTING_KEYS` |
| `sync_meta` / `migrations` | 否 | 本机状态 / 结构性表 |

#### 向后兼容

- 旧 v3.0 备份内的 `agent_configs` / `workflow_steps` / `task_dependencies` / `prompt_templates` /
  `agent_executions` 等字段，以及 todo/subtask 上的 agent/调度/工作流字段，在 v4.0 反序列化时
  通过 serde 的"未知字段忽略"机制自动跳过，不会报错。
- `SyncData` 新字段一律 `#[serde(default)]`；未知顶层键经 `#[serde(flatten)] extra` 原样带回上传。

#### 维护检查清单

当新增数据库迁移时，请检查：
1. 新增的 **settings 键值** 是否已加入 `AppSettings`（含 `#[serde(default)]`）、`read_app_settings` / `write_app_settings`；
   需要跨设备同步的再加入 `SYNCED_SETTING_KEYS`（设备相关的不要加）。
   例外：纯设备本地的提示 / 状态键（如 v29 的 `fence_onedrive_notice_shown`）有意不进 `AppSettings`，
   登记在 `.trellis/spec/backend/sync-protocol.md` §3.6
2. 所有 settings 写入走 `db::settings_kv::set_setting`（禁止 `INSERT OR REPLACE`，否则每次写都会刷新同步版本）
3. 新增的 **数据表** 是否需要同步：需要则加触发器维护 `local_seq`、在 `sync_store` 中实现读写/合并/墓碑，
   并纳入 `build_sync_doc` 与 cloud 的 `sync/doc.rs`、`sync/merge.rs`
4. 新增的 **记录字段**：`models.rs`（`#[serde(default)]`）、`sync_store` 的行读写、`export_data_internal` / `import_data_raw`、
   cloud `src/model.rs` 允许字段与类型
5. 时间字段经 `db::time::normalize_datetime` 规范化；前端经 `utils/datetime.ts`（GPUI 版界面经 `crates/app/src/util/datetime.rs`）
6. 旧版数据导入的兼容性（新字段必须有默认值）
7. 导出版本号是否需要递增
8. **GPUI 版 core 同步修改**（直到 `pc/` 删除）：以上 1~7 在 `pc-gpui/crates/core` 里同样做一遍——迁移加到
   `crates/core/src/db/migrations.rs`（与 `pc/` **同号同 SQL**，`LATEST_VERSION` 相等）；`db/models.rs` / `db/settings_kv.rs` /
   `db/sync_store.rs` 与 `pc/src-tauri/src/db/` 保持逐字一致；`data.rs` 的 `read_app_settings` / `write_app_settings` /
   `export_json` / `import_json`；`sync/mod.rs` 的 `build_sync_doc`。需要界面的设置项再改 `crates/app`（`state.rs`、
   `windows/settings/`）。用 `diff -r pc/src-tauri/src/db pc-gpui/crates/core/src/db` 自查（只应剩 `connection.rs` 的 `test-util` cfg），
   两边 `cargo test` 都要绿。细则见 `.trellis/spec/pc-gpui/core-parity.md`

## 核心架构概念

> 本节描述 `pc/`（Tauri 版）的实现；业务流程（调度、同步、对账……）在 `pc-gpui/crates/core` 里逻辑相同，GPUI 版的窗口 / 事件 / 通信
> 对应关系见下文「pc-gpui（GPUI 版 PC 端）」。

### 通知调度流程

```
NotificationService::start_scheduler() (后台任务，对齐整分钟 tick)
  ├── 扫描 completed = 0 AND notified = 0 AND notify_at - COALESCE(notify_before,0) <= now（按时间排序）
  ├── 逐条处理，单条出错只记日志、不影响其它提醒
  ├── 触发系统通知 / 应用内通知窗口（work area + 缩放换算坐标，叠放与换列）
  ├── 重复提醒：解析 notify_at（空格 / T 均可）→ 计算下一次 → 写规范格式并重置 notified = 0
  └── 非重复：notified = 1
```

### 待办编辑/详情流程

```
TodoList / QuadrantView / CalendarView / CompletedView
  └── 点击待办 → 打开独立 WebView：/editor?id={todoId}&mode=view（新建时无 id，直接编辑模式）
       └── EditorView 双排版：只读详情（标题 + MD 渲染 + 通知状态 + 优先级）⇄ [编辑] 原地切换表单排版
            ├── 单条加载用 get_todo；子任务操作后只刷新当前待办
            └── 描述字段使用 MarkdownEditor 组件（Milkdown，modelValue 为存储形式）
```

### 子任务编辑流程

```
TodoItem / EditorView
  └── 点击编辑/查看
       └── 打开独立 WebView：/subtask-editor?id={subtaskId}&mode={view|edit}
            └── SubtaskEditorView 使用 MarkdownEditor 组件（Milkdown）加载 Markdown
                 └── 图片粘贴 → save_subtask_image（raw body）→ images 目录 → Markdown 写 minitodo-image://<name>
```

### 主窗口刷新

- 每 5s 调 `get_change_seq`（`sync_meta.local_seq`），变化时才 `fetchTodos`；聚焦、子窗口关闭、
  `sync-completed`、`data-imported` 也走同一个单飞 `fetchTodos`（过期响应丢弃）

### WebDAV 同步流程

```
webdav_sync()（async → spawn_blocking，单同步互斥，"同步正在进行中"）
  ├── GET sync-data（有基准 ETag 时 If-None-Match；本地有未上传变更 / 手动同步 / 每第 10 轮时无条件 GET）
  │     ├── 304 → 远端未变
  │     ├── 200 → 解析（失败直接报错）→ 单事务逐条合并（LWW + 墓碑 + 设置 LWW）→ 下载缺失图片
  │     └── 404 → 首次上传
  ├── 需要上传？（local_seq ≠ synced_seq / 设置更新 / 远端缺本地记录或墓碑）
  │     ├── 先上传缺失图片（一次 PROPFIND Depth:1 列清单）
  │     └── 条件 PUT：已知 ETag 时 If-Match（去 W/），从不发 If-Unmodified-Since
  │           412 → 等 1.1s → 重新 GET → 合并 → PUT（最多 4 次）
  └── 成功：基准取 PUT 响应 ETag → HEAD → PROPFIND；记录 synced_seq；emit sync-completed(SyncReport)
webdav_force_pull()  让本地等于远端（删除本地独有记录，不写墓碑）
webdav_force_push()  让远端等于本地（为远端独有记录写墓碑后上传）
```

### 桌面收纳箱流程

```
托盘「新增收纳箱」/ create_fence（async：同步命令 / 菜单回调里建窗会死锁）
  └── create_fence_folder（先建文件夹）→ INSERT fences → save_layout（当前屏幕组合）→ open_fence_window（tao 标志一次给齐）
       └── embed_window_in_desktop → EMBEDDED_WINDOWS 登记 → 200ms tick_embedded_windows 守护（每 10 tick 检测屏幕组合）
拖入 → fence_move_in（async，begin_moving 登记 → spawn_blocking）→ IFileOperation → upsert fence_items（搬运 / 记录原始路径）
目录变化 → notify（300ms 去抖）→ 数据库锁内列举对账 → emit_to("fence-{id}", "fence-items-changed")
删除 → delete_fence → restore_all → 文件夹已清空才删记录 + destroy 窗口，否则整个收纳箱保留
```

WebDAV 服务端差异（实测，见 `.trellis/spec/backend/sync-protocol.md`）：Apache mod_dav 的 `If-Unmodified-Since`
对未改动文件也会 412、写入后 1 秒内 ETag 为弱；nginx/Caddy 忽略 PUT 前置条件、PROPFIND 不返回 getetag、
`If-Modified-Since` 按整秒比较。所以两端只用 `If-Match` / `If-None-Match`，并且写前总是先 GET 合并。

### 前后端通信

- **Tauri invoke**：前端调用后端 Rust 命令（请求-响应）。会触网、读写大文件或做批量 SQL 的命令一律写成
  `async` 并在 `tokio::task::spawn_blocking` 里执行（Tauri 2 的同步命令跑在主线程，会卡住所有窗口）
- **Tauri emit/listen**：事件驱动通信（实时推送）
  - `tray-toggle-fixed`、`tray-reset-window`、`tray-add-todo`、`tray-open-settings`：托盘菜单事件
  - 托盘「新增收纳箱」「显示/隐藏所有收纳箱」「重置收纳箱位置」（菜单 id `new_fence` / `toggle_fences` / `reset_fences`，
    位于「添加待办项」之后，仅 Windows）由后端直接处理，不发前端事件；对应命令 `create_fence` / `toggle_fences_visibility` /
    `reset_fence_positions` 是同一实现
  - `fence-items-changed`（payload `FenceEntry[]`）：后端对账后 `emit_to("fence-{id}")`，只发给对应收纳箱窗口
  - `sync-completed`（payload `SyncReport`）：后端同步改动了本地数据后发出（手动 / 自动同步都会），主窗口据此刷新
  - `data-imported`：设置窗口导入数据后通知主窗口重载
  - `app-settings-changed`：设置窗口改动应用设置后通知主窗口按 `key` 重载（含 `fixedEmbedDesktop`）
  - `todo-font-changed`：字体设置变更通知
  - `subtask-memory-ready` / `-init` / `-save`：新建待办（尚未入库）时，编辑窗口与子任务窗口之间传递内存中的子任务

### 独立 WebView 窗口

部分功能使用独立 Tauri WebView 窗口：
- **EditorView**：待办详情 / 编辑
- **SubtaskEditorView**：子任务详情编辑（Markdown + 图片）
- **SettingsView**：设置
- **CompletedView**：已完成列表
- **NotificationView**：应用内通知弹窗（当通知类型为 "app" 时；按显示器 work area 与缩放换算位置，多条叠放）
- **FenceView**：桌面收纳箱（每个收纳箱一个窗口，label `fence-{id}`；后端 `commands/fence.rs` 建窗，始终嵌入桌面层，
  不可关闭，删除走 `destroy()`）

## pc-gpui（GPUI 版 PC 端）

`pc-gpui/` 是 `pc/` 的 Rust + GPUI 重写（任务 `10-15-gpui-rewrite`），去掉 WebView2，元素 / 样式 / 功能与旧版一致（上文「核心功能」
同样适用，已知偏差见下）。维护者入口：

- `pc-gpui/crates/app/README.md`：模块地图、约定、`AppState` / `WindowManager` / `ui::*` / `platform::*` / `services::*` 接口表、调试开关
- `pc-gpui/crates/core/README.md`：业务 API（逐个对照旧 `#[tauri::command]`）、线程与调用约束、旧事件 → 返回值
- `pc-gpui/scripts/README.md`：`regress.py` / `shot.sh` / `MINI_TODO_DEBUG_OPEN`；`scripts/e2e-linux/`、`scripts/e2e-windows/` 各有 README
- `.trellis/spec/pc-gpui/`：GPUI 约定（`gpui-conventions.md`）、与 `pc/` 的双改规则（`core-parity.md`）、验证门槛（`verification.md`）

### 与 `pc/` 的关系

- **数据完全兼容**：同一数据目录 `%LOCALAPPDATA%\mini-todo\`（库 / 图片 / 备份 / 节假日 / 图标缓存）、同一迁移 v1~v29、同一同步文档
  `sync-data.json.gz`（同一 `webdav_device_id`，切换版本不会被当成新设备）、同一导出格式 4.0、同一 settings 键、同一日志文件与收纳根目录。
  用户可以在两版之间直接切换，多台设备也可以混用（与 cloud 的协作不变）。
- **不要同时运行两版**：单实例与旧版 Tauri 插件同名同格式（互斥量 `com.tauri-app.mini-todo-sim`、接收窗口、`WM_COPYDATA`），两版互相识别——
  后启动的一版把参数转交给已运行的那一版（唤起其主窗口）后退出。
- 开机自启沿用同一注册表项 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\mini-todo`（`"<exe>" --autostart`）；已开启时每次启动都把路径
  刷新为当前 exe，因此指向最后启动的那一版。系统通知的 AppUserModelID 沿用 `com.tauri-app.mini-todo`。
- **双改规则（直到 `pc/` 删除）**：数据库迁移、settings 键、记录字段、同步 / 合并 / 时间 / 墓碑规则、导出格式必须 `pc/src-tauri` 与
  `pc-gpui/crates/core` 一起改（迁移同号同 SQL，否则混用版本时数据结构分叉或设置被重置）；`db/`、`services/{webdav,secret}.rs`、
  `services/fence_fs/` 两边保持逐字一致（`diff` 自查）；旧前端 `pc/src/utils/*.ts` 已 1:1 移植到 `crates/app/src/util/` 等处，修逻辑 bug
  两边一起修。纯界面改动以 pc-gpui 为准。细则 `.trellis/spec/pc-gpui/core-parity.md`。
- `.trellis/spec/backend/*` 的契约（同步协议、窗口模式语义、收纳箱数据与文件层）对 `pc-gpui/crates/core` 与 app 的对应实现同样适用。

### 架构要点

- `crates/core`（`mini-todo-core`）：`db/`、`services/{webdav,secret,fence_fs}` 原样复制；`commands/*.rs` 去掉 Tauri 外壳成为普通阻塞函数
  （`todo`、`data`、`sync`、`settings`、`holiday`、`reminder`、`window_logic`、`fence`）；新增 `lunar`（tyme4rs）。不依赖 GPUI / Tauri / tokio，
  错误仍是面向用户的中文 `String`。
- `crates/app`（`mini-todo`）：一个进程一个 GPUI `Application`（`QuitMode::Explicit`，托盘常驻），所有窗口共享同一个 `App`：

  | 旧版（Tauri / WebView） | GPUI 版 |
  |---|---|
  | Pinia `todoStore` / `appStore` | `state::AppState`（`Entity` + Global），各窗口 `cx.observe`；写操作经 `AppState` 方法在后台执行 |
  | `invoke` 同步命令 / `spawn_blocking` | `db::spawn_db(cx, \|db\| core::…)` 放后台，`WeakEntity::update` 回前台 |
  | `sync-completed` | `services::sync::run_sync` → `AppState::apply_sync_report` |
  | `data-imported` / `app-settings-changed` / `todo-font-changed` | 直接改 `AppState`，所有窗口随之刷新 |
  | `subtask-memory-ready` / `-init` / `-save` | 子任务窗口直接读写新建待办窗口的 `Entity<DraftSubtasks>`（`subtask::open_draft`） |
  | `fence-items-changed` | 目录监听回调（`fence-watch` 线程）经 channel 回前台 → 对应收纳箱窗口 |
  | 托盘事件 | tray-icon 回调 → 前台直接执行动作（`services::tray`） |
  | 独立 WebView 窗口（label） | GPUI 窗口：`windows::WindowManager` 统一登记 / 模态子窗口 / 遮罩 / owner / 关闭后刷新 / 居中 |
  | tao 窗口能力 + `commands/window.rs` Win32 | `platform/`（`windows.rs` Win32、`fallback.rs` Linux、`embed.rs` 嵌入桌面、`subclass.rs` 置底与几何锁定） |
  | Element Plus / Milkdown | `ui/` 下自绘的 EP 风格控件（每种一个实现）、`ui::markdown*` |

- 主窗口刷新与旧版相同：5s `change_seq` 轮询、子窗口关闭、同步完成都走 `AppState::fetch_todos`（单飞 + 过期丢弃），主窗口有模态子窗口时后台刷新跳过。
- 关键约定（详见 `.trellis/spec/pc-gpui/gpui-conventions.md`）：阻塞调用一律放后台；App 被借用时（事件处理 / `update` / `render`）不能同步调用
  会给窗口发消息的 Win32（`platform::*(window, cx, …)` 已自动延后）；Windows 上 `start_window_move()` 是空操作，标题栏用
  `ui::title_bar::drag_region()`；GPUI 没有 z-index（后绘制者在上，弹层只能画在窗口内）；本地图片 `img(PathBuf)`；颜色 / 尺寸 / 字体只从
  `theme.rs` 取；`gpui-kit` 精确钉 `=0.7.3`，升级按 spec 的步骤逐项核对。

### 已知偏差（相对 `pc/`）

1. **Markdown 编辑**：Milkdown 所见即所得无法在 GPUI 中 1:1 还原，编辑模式改为"Markdown 源码 + 渲染预览"（放大编辑弹窗仍是源码 / 预览分栏）；
   只读渲染、图片粘贴 / 拖入上传、`minitodo-image://` 引用、链接白名单规则不变。
2. **中文换行**：GPUI 的换行器不做避头尾，全角标点（`，。、；：！？）`）可能落到行首，多行中文的折行位置与浏览器可能差一个字。
3. GPUI 没有文字阴影、多色标渐变、位图变换等，个别装饰用近似实现（见 spec `gpui-conventions.md` §2.13）。
4. **Linux 仅用于 CI、截图回归与 e2e**：没有托盘 / 单实例 / 开机自启 / 嵌入桌面 / 收纳箱真实文件操作，最后一个窗口关闭即退出，设置里的字体下拉为空。
   Windows 专有能力（嵌入桌面、收纳箱、DPAPI、Toast、托盘、DirectComposition 透明）在 Linux 上只做交叉 clippy，靠 CI 的 Windows e2e 与用户实机验收。

### 构建与运行要点

- Windows release 构建必须在装了 Windows SDK 的 Windows 上做（gpui 的 build.rs 用 `fxc.exe` 预编译 HLSL 着色器，可用 `GPUI_FXC_PATH` 指定）：
  `cd pc-gpui && cargo build --release -p mini-todo` → `pc-gpui/target/release/mini-todo.exe`（单个 exe，不需要 WebView2 Runtime，暂无安装包）。
  Linux 无法交叉构建可运行的 Windows 版。
- debug 构建的 exe 在运行时按**构建机**上 cargo registry 的绝对路径编译着色器，只能在构建它的那台机器上运行。
- 试用：CI `pc-gpui-ci.yml` 的 Windows job 上传 release 产物 `mini-todo-gpui-windows-x64`（`mini-todo.exe`）。
- debug 专用环境变量：除下文「开发命令」表中的三个（同名同义）外，GPUI 版另有 `MINI_TODO_DEBUG_OPEN`（启动后打开指定窗口）、
  `MINI_TODO_DEBUG_LATEST_VERSION`、`MINI_TODO_DEBUG_DIALOG_PATH`、`MINI_TODO_DEBUG_WINDOWS_UI`、`MINI_TODO_DEBUG_FENCE_STATE`，
  同样只在 debug 构建读取（见 `pc-gpui/crates/app/README.md` §5）。

## cloud/ 子项目（云端 API）

`cloud/` 是一个独立的 Rust crate（**不在** `pc/` 的 Cargo workspace 里），部署到 VPS 上为
AI / Claude Code Skill 提供 mini-todo 数据的 HTTP REST 访问能力。部署、配置、Skill 见 `cloud/README.md`。
与 PC 的契约（和上文"时间格式约定""合并规则"是同一套）：

- **共用同一个 WebDAV `sync-data.json.gz` 通道**。云端是 WebDAV 客户端，不依赖 PC 在线
- **WebDAV 是 source of truth**，云端 SQLite 是缓存；未推送的本地改动（`meta.dirty`）推送成功前不会被拉取覆盖
- **时间**：`YYYY-MM-DD HH:MM:SS`，按 `config.toml` 的 `timezone`（IANA，须与 PC 一致）每次重新换算墙钟（DST 安全）
- **写冲突**：写前先 GET 合并，条件 PUT 只用 `If-Match`（去 `W/`）；412 等待后重新 GET → 合并 → PUT
- **字段契约**：SQLite 是 KV-style（`todos(id, data_json, updated_at)`），但 API 写入只接受 PC 模型里的字段
  （`src/model.rs`），未知字段 / 类型错误 → 400；合并远端时未知字段原样保留（可能是更新版 PC 加的新字段）
- **墓碑**：DELETE 写 `tombstones`，随 sync-data 双向传播，保留 30 天
- **短码**：`todo_seq` 表给每个 todo 分配 `C{seq}`，API 路径与 CLI 都可用；只存在于 cloud，不进同步文档

```
cloud/
├── src/main.rs               # 启动：config → SQLite → 首次 pull（spawn_blocking）→ worker → axum；停机限时补推
├── src/config.rs             # config.toml；缺必填字段报错退出
├── src/time.rs               # 规范时间格式、宽松解析、按 IANA 时区换算
├── src/model.rs              # 记录字段契约：写入校验、别名映射、派生 priority
├── src/db/                   # todos / subtasks / settings / meta / tombstones / todo_seq；normalize.rs 修存量数据
├── src/sync/webdav.rs        # GET（If-None-Match）/ PUT（If-Match）/ HEAD / PROPFIND / MKCOL
├── src/sync/doc.rs           # sync-data 文档读写（保留未知顶层键）
├── src/sync/merge.rs         # 与 PC 一致的记录 LWW + 墓碑 + 设置合并
├── src/sync/{pull,push}.rs   # 拉取合并；写前合并 + 条件 PUT + 412 重试；图片推送
├── src/sync/worker.rs        # pull 轮询（每 10 轮一次无条件全量）、push 去抖 1.5s（最长 10s）、失败指数退避
├── src/sync/images.rs        # 图片镜像（缺什么下什么，失败退避重试）
├── src/sync/mock_dav.rs      # 测试用进程内 WebDAV，可按 Apache / nginx 实测语义模拟
├── src/api/                  # axum：Bearer 鉴权、X-Sync-Status 头、5xx 不外泄内部细节
└── skill/minitodo/           # Claude Code Skill：SKILL.md + minitodo.py（含单测）+ install.{sh,ps1}
```

REST API（全部需要 `Authorization: Bearer <api_key>`，`:id` 可用 `C{seq}` 短码）：

| Method | Path | 说明 |
|---|---|---|
| GET | `/health` | `{status, sync, lastPullAt}` |
| GET | `/todos?...` | 列表；query: `completed` / `priority` / `quadrant` / `dueDateBefore` / `dueDateAfter` / `startDate` / `q` / `sort=±field` / `limit` / `offset` / `withSubtasks` |
| GET | `/todos/:id?withSubtasks=true` | 详情；默认嵌套 subtasks |
| POST | `/todos` | 创建；必填 `title`，其余字段按契约校验 |
| PATCH | `/todos/:id` | merge 更新；未提及字段保留 |
| DELETE | `/todos/:id` | 删除（连带 subtasks，写墓碑） |
| POST | `/todos/:id/subtasks` | 创建子任务；必填 `title` |
| PATCH | `/subtasks/:id` | merge 更新子任务（`parentId` 不可改） |
| DELETE | `/subtasks/:id` | 删除子任务 |
| GET | `/images/:name` | 返回图片 bytes |
| POST | `/images` | multipart 上传，`file` 字段；返回 `{name}` |
| POST | `/sync` | 手动 pull + push；全部成功 200、部分失败 207，返回 `{pull, push, pullError?, pushError?}` |
| POST | `/sync/pull` | 仅从 WebDAV 拉取 |
| POST | `/sync/push` | 仅推送到 WebDAV |

todo 可写字段以 `src/model.rs` 的 `TODO_FIELDS` 为准（`title` / `description` / `color` / `quadrant` / `notifyAt` /
`startTime` / `endTime` / `notifyBefore` / `notified` / `completed` / `sortOrder` / 重复提醒 5 个字段）。别名：
`dueDate`→`endTime`（仅日期补 23:59:00）、`priority`（high/medium/low）→`color`（#EF4444 / #F59E0B / #10B981）、
`notes`→`description`；`quadrant` 接受 1-4 或 `urgent_important` / `important_not_urgent` 等别名。`id` / `createdAt` /
`updatedAt` 等服务端字段写入时忽略。响应附派生字段 `priority`（自定义颜色为 null）与 `seq`，派生字段不入库。

排序字段白名单：`dueDate` / `startTime` / `priority` / `quadrant` / `sortOrder` /
`updatedAt` / `createdAt` / `title`，前缀 `-` 倒序、`+` 或无前缀正序。

所有响应附 `X-Sync-Status: healthy | stale | offline` 与 `X-Last-Sync-At`；offline
时额外加 `Warning: 110 "sync offline"`。

## 开发命令

```bash
# PC 前端（在 pc/ 下）
npm ci                     # 安装依赖（lockfile 只指向 registry.npmjs.org）
npm run tauri dev          # 开发模式运行
npm run tauri build        # 构建生产版本
npm run typecheck          # vue-tsc
npm run lint               # eslint
npm run test               # vitest 单测（src/utils/*.test.ts）
npm run build              # 类型检查 + vite 构建

# PC Rust（在 pc/src-tauri/ 下）
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked        # 含 sync_store 合并、迁移、WebDAV 互操作（tiny_http 模拟服务端）等测试

# cloud（在 cloud/ 下）
cargo test --locked        # 含 mock_dav 场景测试（Apache / nginx 语义）
python3 -m unittest discover -s skill/minitodo -p 'test_*.py'

# 端到端（Linux，真实应用 + 双 WebDAV + cloud + Skill），见 pc/scripts/e2e-linux/README.md
python3 pc/scripts/e2e-linux/run_e2e.py <app> <cloud> <old-app>

# GPUI 版（在 pc-gpui/ 下），门槛见 .trellis/spec/pc-gpui/verification.md
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked                          # core（迁移来的全部测试 + 农历对照）+ app（util / 纯逻辑）
cargo test -p mini-todo-core                             # 只测 core（不编译 GPUI）
# Windows 目标交叉检查（覆盖全部 cfg(windows) 代码；需 rustup target add x86_64-pc-windows-gnu + mingw gcc；只 check 不链接）
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
MINI_TODO_DATA_DIR=/tmp/mt cargo run -p mini-todo        # 隔离数据目录运行 debug 构建
cargo build --release -p mini-todo                       # 仅 Windows（需 Windows SDK 的 fxc.exe）→ target/release/mini-todo.exe
scripts/regress.py --ref <参考截图目录> --out /tmp/regress  # Xvfb 视觉回归：场景截图 + 并排对照图（--only 'main-*' 只跑部分）
scripts/regress.py --smoke --no-build                    # 跨窗口端到端冒烟（按数据库断言）
scripts/shot.sh --name main-list --window "Mini Todo" /tmp/shots   # 单个场景截图 / xdotool 交互

# GPUI 版端到端（仓库根目录）：Linux 见 pc-gpui/scripts/e2e-linux/README.md；Windows 见 pc-gpui/scripts/e2e-windows/README.md
python3 pc-gpui/scripts/e2e-linux/run_e2e.py pc-gpui/target/debug/mini-todo cloud/target/debug/minitodo-cloud [用例前缀 ...]
python pc-gpui/scripts/e2e-windows/run_e2e.py --exe pc-gpui/target/debug/mini-todo.exe --seed pc-gpui/target/debug/examples/seed.exe --out <输出目录>
```

debug 专用环境变量（**只在 debug 构建读取，release 一律忽略**；与正在运行的安装版并存做实测时必须带上，详见
`.trellis/spec/backend/desktop-fences.md` §3.6 与 `pc/scripts/e2e/README.md`）：

| 变量 | 作用 |
|---|---|
| `MINI_TODO_DATA_DIR` | 数据目录（库 / 图片 / 备份 / 图标缓存）、日志（`<dir>\logs`）、WebView2 用户数据（`<dir>\EBWebView`）都改到该目录；不注册单实例插件、不刷新开机自启注册表项；托盘提示"Mini Todo（开发版）"；asset 协议额外放行隔离目录的 images / fence-icons |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录改为该目录（代替 `%USERPROFILE%\MiniTodo收纳箱`） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容非空时替代屏幕组合 ID，用于模拟显示器组合变化 |

GPUI 版读取同样三个变量（同名同义；没有 WebView2 / asset 协议相关部分），另有 `MINI_TODO_DEBUG_OPEN` 等 GPUI 版专用的 debug 开关，
见 `pc-gpui/crates/app/README.md` §5 与 `pc-gpui/scripts/README.md`。

CI：`.github/workflows/pc-ci.yml`（Windows fmt/clippy/test、Linux clippy/test、前端 typecheck/lint/test/build）、
`cloud-ci.yml`（fmt/clippy/test + Skill 单测）、`pc-gpui-ci.yml`（GPUI 版：Windows fmt/clippy/test + release 构建并上传
`mini-todo-gpui-windows-x64` 产物，Linux fmt/clippy/test）、`pc-gpui-e2e-windows.yml`（GPUI 版 Windows e2e：debug 套件 + release 冒烟 / 单实例；
手动触发，或推送到 `claude/**` 分支且改动 `pc-gpui/**` 时运行）。

## 注意事项

1. **目标平台**：Windows 10/11；Linux 可编译运行（CI 与 e2e 用），嵌入桌面、桌面收纳箱等 Win32 特性在 Linux 上不可用
2. **运行环境**：需要 Node.js 和 Rust 开发环境（`pc-gpui/` 只需 Rust stable；Windows release 构建另需 Windows SDK）
3. **图标使用**：仅使用 Element Plus Icons，禁止 emoji
4. **代码规范**：使用 TypeScript 类型定义，遵循 Vue 3 组合式 API；`<el-*>` 组件按需引入，
   `ElMessage` / `ElMessageBox` 只能从 `src/plugins/element.ts` 导入（eslint 强制）
5. **Serde 命名**：Rust 模型使用 `#[serde(rename_all = "camelCase")]`，前端使用驼峰命名
6. **进程管理**：Windows 平台需要外部子进程时使用 `taskkill` 终止子进程树
7. **日志**：Rust 侧用 `log::info!` / `warn!` / `error!`（落盘到日志文件），不要用 `println!` / `eprintln!`
8. **设置读写**：一律经 `db::settings_kv`（只在值变化时更新 `updated_at`，否则会干扰设置同步）
9. **安全边界**：CSP `script-src 'self'`（禁止 `eval` / 内联脚本）；本地图片只经 asset 协议
   （scope 只有 `$LOCALDATA/mini-todo/images/**` 与收纳箱图标缓存 `$LOCALDATA/mini-todo/fence-icons/**`，不要放宽）访问，Markdown 里存 `minitodo-image://<name>`（`utils/imageRef.ts`）；
   链接只放行 http(s) / mailto / 本地文件（`utils/fileLink.ts`）；图片文件名经 `is_safe_image_name` 校验
10. **时间**：见"时间格式约定"，前后端都只用统一的工具函数解析与格式化
11. **两套 PC 实现并存**：`pc/`（Tauri）与 `pc-gpui/`（GPUI）共用数据，影响数据的改动（迁移 / settings 键 / 字段 / 同步规则 / 导出格式）
    必须两边一起改，直到 `pc/` 删除（见「pc-gpui」一节与 `.trellis/spec/pc-gpui/core-parity.md`）；不要在同一台机器上同时运行两版
12. **GPUI 版约定**：改 `pc-gpui/crates/app` 前读 `.trellis/spec/pc-gpui/gpui-conventions.md`（后台执行、App 借用期间不同步调 Win32、
    拖动窗口、z 序与弹层、`img(PathBuf)`、theme token 唯一来源、`ui/` 控件复用、debug 开关只在 debug 构建生效）；
    `gpui-kit` 精确钉 `=0.7.3`，不要随手升级；改了界面跑 `scripts/regress.py` 并逐张看对照图

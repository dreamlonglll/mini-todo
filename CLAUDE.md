# Mini-Todo 项目指南

本文档用于介绍 Mini-Todo 项目，帮助 AI 助手快速了解项目结构和开发规范。

## 项目简介

Mini-Todo 是一款 Windows / macOS 桌面待办事项管理应用，定位为简洁、聚焦的本地待办工具，支持子任务、四象限、日历、重复提醒、系统通知与
WebDAV 云同步，另有收纳桌面图标的桌面收纳箱（仅 Windows）。**macOS 版是预览版**：只在 GitHub 的 macOS runner 上自动化验证过，
未经真机验收（维护者没有 Mac），差异与限制见「macOS（预览）」与「已知偏差与限制」。

PC 端是 `pc-gpui/`：用 **Rust + GPUI**（`gpui-kit =0.7.3` = gpui-pre 0.3.9 + gpui-component 0.7.3）实现，**不依赖 WebView2**；Windows 上是单个
`mini-todo.exe`，macOS 上是 universal（arm64 + x86_64）的 `mini-todo.app`。3.0 起它是唯一的 PC 实现（目录名暂未改成 `pc/`）。
早期的 Tauri 2 + Vue 3 版（原 `pc/`）已在 3.0 删除，需要对照旧代码时从
git 历史取（删除前的最后一个提交是 `749405a`，例如 `git show 749405a:pc/src-tauri/src/commands/window.rs`）。GPUI 版与旧版**数据完全兼容**
（同一数据目录、同一数据库迁移、同一同步文档、同一导出格式、同一 settings 键），Windows 安装包原地升级旧版、mac 包与 2.5.0 mac 版是同一个
应用身份（拖进「应用程序」直接替换），见「兼容旧版沿用的名字」。

`cloud/` 是云端 HTTP API（mini-todo 在远程 VPS 上的复刻，通过同一个 WebDAV 通道与 PC 同步数据，供 AI / Claude Code Skill 通过 REST 读写），
详见下文「cloud/ 子项目」与 `cloud/README.md`。

## 技术栈

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| UI 框架 | gpui-kit `=0.7.3`（gpui-pre 0.3.9 + gpui-base / gpui-component 0.7.3） | Zed 的 GPUI 每周快照，**精确钉版本**；Windows 走 DirectX 11 + DirectComposition（透明窗口），macOS 走 Metal（gpui-kit 强制 `runtime_shaders`：着色器运行时编译，构建期不需要 Metal 工具链），Linux 走 wgpu（Vulkan，CI / Xvfb 用 mesa lavapipe） |
| 控件 | 自绘 Element Plus 风格控件（`crates/app/src/ui/`） | 外观数值照抄 UI 规格；复杂输入基于 gpui-component（`InputState` 等），主题调成 EP（`#409EFF`） |
| 图标 | Element Plus Icons SVG | 从 `@element-plus/icons-vue` 抽取，`crates/app/assets/icons/{ep,app}/`，经 `assets.rs` 嵌入 |
| Markdown | gpui-base `TextView` + 自写块级排版（`markdown` crate，GFM：表格 / 任务清单 / 删除线 / 自动链接） | 编辑为"源码 + 预览"（已知偏差，见下文） |
| 业务核心 | `mini-todo-core`（`crates/core`） | rusqlite 0.32 bundled（SQLite）、reqwest 0.12 blocking（WebDAV / 节假日）、windows 0.58；不依赖 GPUI / Tauri / tokio，函数全部阻塞 |
| 农历 | tyme4rs | lunar-javascript 同一作者的 Rust 库，结果与旧前端逐日比对（`tests/fixtures/lunar_fixture.json`） |
| 平台集成 | windows 0.62（与 gpui 同版本）、tray-icon、notify-rust、auto-launch、rfd、embed-resource；macOS：objc2 0.6 / objc2-app-kit 0.3（与 gpui-pre-macos 同版本） | 托盘 / 菜单栏图标、Toast、开机自启、带过滤器的文件对话框（rfd 仅 Windows）、exe 图标与版本信息；macOS 的 NSWindow / NSScreen 调用、系统通知走 GPUI `show_system_notification`（notify-rust 不在 mac 上编译） |
| 安装包 | NSIS（`pc-gpui/scripts/windows-installer.nsi`）+ 便携 zip；macOS universal dmg（`pc-gpui/scripts/macos/`） | 由 `release.yml` 构建：Windows x64 + macOS universal（整包 ad-hoc 签名，不公证） |
| 测试 | cargo test、`scripts/regress.py`（Xvfb 截图对照 + 冒烟）、`scripts/e2e-linux/`、`scripts/e2e-windows/`、`scripts/e2e-macos/`、`scripts/mac-check/`（mac 专属代码的本地交叉检查） | 见 `.trellis/spec/pc-gpui/verification.md` |

## 项目结构

> 项目按平台拆分子目录。`pc-gpui/` 为 Windows / macOS 桌面端（GPUI），`cloud/` 为云端 API，未来可平行加入 `mobile/`、`web/`。

```
mini-todo/
├── docs/                                   # 共享文档（跨平台）
│   └── 开发文档/                            # 开发相关文档
├── pc-gpui/                                # PC 端（Cargo workspace；详见各 crate README）
│   ├── Cargo.toml                          # [workspace] members = crates/core、crates/app；[workspace.package] version 是
│   │                                       # 版本号唯一来源（当前 3.0.1-pre）；release thin LTO + strip
│   ├── Cargo.lock                          # 提交；CI 一律 --locked
│   ├── crates/
│   │   ├── core/                           # mini-todo-core：业务核心（不依赖 GPUI / Tauri / tokio，全部阻塞函数）
│   │   │   ├── README.md                   # 公开 API、线程约束、与旧 Tauri 命令的对照
│   │   │   ├── src/
│   │   │   │   ├── db/                     # 连接 / 迁移前备份（connection、backup）、迁移 v1~v30（migrations）、模型（models）、
│   │   │   │   │                           # settings 读写助手（settings_kv）、变更计数 / 墓碑 / 合并（sync_store）、时间（time）、路径（paths）
│   │   │   │   ├── services/               # webdav.rs（条件 GET/PUT、PROPFIND）、secret.rs（DPAPI）、fence_fs/（收纳箱文件层）
│   │   │   │   ├── todo.rs / data.rs       # 待办 / 子任务 CRUD、图片保存；导入导出、AppSettings 读写（唯一来源）
│   │   │   │   ├── sync/                   # sync::run(db, SyncMode) -> SyncReport（测试在 sync/tests.rs）
│   │   │   │   ├── settings.rs             # 单项设置、运行时偏好、屏幕组合 ID、screen_configs
│   │   │   │   ├── holiday.rs / reminder.rs  # 节假日（blocking）/ 提醒调度纯逻辑与通知窗口槽位、定位
│   │   │   │   ├── window_logic.rs         # 贴边隐藏状态机、needs_reattach、lacks_tool_window_style 等纯函数
│   │   │   │   ├── lunar.rs                # 农历（tyme4rs）
│   │   │   │   └── fence/                  # 收纳箱元数据 / 几何 / 屏幕组合布局（mod.rs）与内容对账、移入移回（items.rs）
│   │   │   ├── examples/seed.rs            # 把导出格式 4.0 的 JSON 导入隔离数据目录（截图 / e2e 用）
│   │   │   └── tests/fixtures/             # 农历对照数据（gen_lunar.cjs 用 lunar-javascript 生成）
│   │   └── app/                            # mini-todo：GPUI 界面、窗口、平台集成（lib mini_todo + bin mini-todo）
│   │       ├── README.md                   # 模块地图、约定、接口表、调试开关（维护者必读）
│   │       ├── build.rs                    # Windows：嵌 exe 图标（资源 ID 1）+ 版本信息；manifest 由 gpui 嵌，不要重复（其它目标直接 return）
│   │       ├── assets/                     # icon.ico；icon.icns（mac bundle 图标，取自 2.5.0）、tray-template.png（mac 菜单栏模板图标）；
│   │       │                               # icons/ep/*.svg（Element Plus）、icons/app/*.svg（自定义）
│   │       └── src/
│   │           ├── lib.rs / main.rs        # run()：日志 → 单实例 → 数据库 → Application(QuitMode::Explicit) → 主窗口 → 后台服务
│   │           ├── theme.rs                # 颜色 / 字体 / 圆角 / 间距 / 阴影 token 的唯一来源（按窗口分节）
│   │           ├── state.rs                # AppState（Entity + Global）：跨窗口共享状态与待办 / 设置写操作
│   │           ├── db.rs                   # Db 全局（Arc<Database>）、spawn_db（阻塞调用放后台）
│   │           ├── assets.rs               # EpIcon / AppIcon 嵌入表
│   │           ├── util/                   # datetime、image_ref（minitodo-image://）、file_link（链接白名单）、editor_shortcuts
│   │           ├── windows/                # WindowManager（mod.rs，含 DebugOpen）+ main_window/、editor/、subtask、completed、
│   │           │                           # notification、settings/、fence/、gallery（控件样例，仅调试）
│   │           ├── ui/                     # Element Plus 风格共享控件（button、input、ep_*、message、message_box、markdown*…）
│   │           ├── services/               # 托盘 / 菜单栏图标、单实例、开机自启、同步编排、自动同步、提醒调度、系统通知、窗口模式、收纳箱、
│   │           │                           # 版本检查、日志；mac_menu.rs（macOS 应用菜单与 Dock 唤起）
│   │           └── platform/               # GPUI 没有的窗口能力：windows.rs（Win32）、macos/（mod.rs GPUI 胶水 + ns.rs AppKit 原始调用）、
│   │                                       # mac_geometry.rs（mac 坐标换算，全平台编译带单测）、fallback.rs（Linux）、embed.rs、subclass.rs…
│   └── scripts/
│       ├── README.md                       # regress.py / shot.sh / MINI_TODO_DEBUG_OPEN 说明，文末「macOS：打包、e2e 与交叉检查」
│       ├── regress.py                      # Xvfb 一键回归：场景截图 + 并排对照图 + 端到端冒烟
│       ├── shot.sh                         # 单个场景截图 / xdotool 交互
│       ├── sample-data.json                # 样例数据（导出格式 4.0）
│       ├── windows-installer.nsi           # NSIS 安装器（安装身份沿用旧 Tauri 版，见 .trellis/spec/pc-gpui/release.md）
│       ├── macos/                          # bundle.sh（组装 mini-todo.app + 整包 ad-hoc 签名）、make-dmg.sh（拖放安装 dmg）、gen-tray-icon.py
│       ├── mac-check/                      # 不属于 workspace 的小 crate：把不依赖 GPUI 的 mac 源文件 #[path] 进来，在 Linux / Windows 上
│       │                                   # --target aarch64-apple-darwin 做类型检查
│       ├── e2e-linux/                      # Linux 端到端（真实应用 + Apache / nginx WebDAV + cloud + Skill）
│       ├── e2e-windows/                    # Windows 端到端（真实窗口 + 真实输入 + Win32 断言；CI pc-gpui-e2e-windows.yml）
│       └── e2e-macos/                      # macOS 端到端（CGWindowList / 截图 / Vision OCR / AX / CGEvent；CI pc-gpui-e2e-macos.yml）
├── cloud/                                  # 云端 HTTP API（独立 Rust crate，见下文）
├── .github/workflows/                      # release / cache-warm / pc-gpui-ci / pc-gpui-e2e-windows / pc-gpui-e2e-macos / cloud-ci
├── CLAUDE.md                               # 项目指南（本文档）
├── README.md
└── AGENTS.md
```

## 核心功能

### 待办管理
- 创建、编辑、删除待办事项
- 描述字段支持 Markdown（存于 `description`）：编辑为"源码 + 预览"——平时显示渲染结果，单击切到源码编辑，失焦回到渲染；
  粘贴普通文字 / Markdown 源码原样插入；渲染支持 GFM 表格 / 任务清单 / 删除线 / 自动链接。编辑模式另有源码 / 预览分栏的放大编辑弹窗
  （`windows/editor/desc_dialog.rs`，「最大化」联动窗口最大化）
- Markdown 中的图片统一存为 `minitodo-image://<文件名>`（`crates/app/src/util/image_ref.rs`），渲染时映射成本机 images 目录下的文件
  （`img(PathBuf)`）；兼容旧数据里的 `http://asset.localhost/<绝对路径>` / `asset://localhost/...`（按文件名映射到本机，修复跨设备裂图，
  保存时换回规范形式）。粘贴 / 拖入图片 → 后台 `core::todo::save_image(bytes, ext)`：扩展名白名单 png/jpg/jpeg/webp/gif/bmp、单张 ≤20MB、
  文件名由后端生成，插入 `![<文件名>](minitodo-image://<name>)`；点图片打开预览
- 链接点击走协议白名单（`util/file_link.rs`）：http/https/mailto 交给系统程序，`file:///` 在资源管理器（macOS 访达）定位（拒绝 UNC），
  其余（javascript:/data: 等）一律拦截；只读排版普通单击即打开，编辑排版下 Ctrl/Cmd+单击
- 编辑类窗口支持 Esc 关闭（有未保存修改时确认）、Ctrl/Cmd+Enter 保存；保存有防重入
- 只读详情模式：从列表 / 四象限 / 日历 / 已完成视图点击待办默认进入（`editor::open(.., Some(id), EditorMode::View)`），
  左侧简化展示标题、Markdown 渲染描述、通知状态、优先级，右侧子任务面板功能照常，[编辑] 按钮原地切换
- 支持一级子任务（含 Markdown 详情、图片上传；可从 .md/.txt 文件或文件夹批量导入）
- 四象限分类（重要紧急 / 重要不紧急 / 紧急不重要 / 不紧急不重要）
- 自定义颜色标识、完成状态标记、拖拽排序（列表 / 四象限）、置顶
- 开始/截止时间

### 子任务
- 标题 + Markdown 内容
- 支持图片粘贴/拖入上传
- 独立的子任务窗口（查看 / 编辑双模式，800×750）；子任务列表双击标题行内改名
- 新建待办（尚未入库）时子任务暂存在内存：子任务窗口直接读写编辑窗口的 `Entity<DraftSubtasks>`
- 完成态切换、排序

### 通知提醒
- 系统通知 / 应用内通知窗口。系统通知：Windows 用 Toast（`notify-rust`，后台线程发，失败不推进、下一分钟重试）；macOS 用 GPUI 的
  `show_system_notification`（`UNUserNotificationCenter`，归属 bundle id `com.tauri-app.mini-todo`，只能在主线程调用，调度器经通道交给前台；
  拿不到投递结果，交出即视为已投递；只有从 ad-hoc 签名的 `.app` 启动才可用，第一次发通知时才请求授权）
- 预设提前提醒（5/15/30 分钟）、自定义提前时间
- 应用内通知窗口（320×120、置顶、不抢焦点）按主显示器工作区（逻辑像素）定位，自下而上叠放、超出换列（core `reminder` 的槽位与叠放算法）；
  按窗口实际尺寸重新定位并记住，保证不出屏

### 重复提醒
- 按天 / 周 / 周几 / 月几号循环
- 触发后自动推进到下一次提醒时间点（月重复缺日期时以 notify_at 的日为锚，月末自动钳位）
- 应用启动时补发错过的重复提醒
- 提醒时间兼容空格 / `T` 两种格式；单条提醒出错不影响其它提醒

### 视图模式
- **列表视图**：按排序/优先级展示
- **四象限视图**：拖拽分类
- **日历视图**：按日期展示（含农历、节气、传统节日、法定节假日与调休；跨天条，点击打开只读详情），开启后主窗口加宽

### 数据导入导出
- 导出版本：`4.0`（`pc-gpui/crates/core/src/data.rs` 的 `EXPORT_VERSION`）
- 导出为 ZIP 压缩包（内含 `data.json`），默认文件名 `mini-todo-backup-YYYY-MM-DD.zip`
- 导入兼容 v3.0 和 v4.0 两个版本（按 `PK` 魔数区分 ZIP / JSON，直接 JSON 也支持）
  - v3.0 是历史导出格式（含已移除的 AI Agent 字段），反序列化时通过 serde 的未知字段忽略与 `#[serde(default)]` 静默跳过
- 导入是"恢复备份"语义：导入前自动 `VACUUM INTO` 备份到 `backups/`；保留原 id；导入后消失的记录写墓碑，
  被导入记录的 `updated_at` 刷新为当前时间（保证恢复结果在同步中胜出）

### WebDAV 云同步
- 智能同步 `sync::run(db, SyncMode::Smart { full })`：先 GET 远端并逐条合并（LWW + 墓碑），本地有变化才条件 PUT；标题栏同步按钮与设置页
  「立即同步」传 `full: true`（无条件 GET），自动同步传 `false`（每第 10 轮也做一次无条件 GET）。设置页另有带二次确认的
  「用云端覆盖本地」（`SyncMode::ForcePull`）、「用本地覆盖云端」（`SyncMode::ForcePush`）。不再有整库二选一的冲突对话框
- 同步范围：todos / subtasks / 墓碑 / 10 个应用设置项（按 `settingsUpdatedAt` 做 LWW，窗口位置与尺寸不从远端应用）/ 图片
- 自动同步可选（按间隔轮询，失败段只提示一次）；同步在后台线程执行，不阻塞界面；同一时刻只允许一个同步（`"同步正在进行中"`）
- gzip 压缩传输；**数据本身不加密**（传输安全取决于 https，WebDAV 服务商可读取内容）；
  WebDAV 密码在 Windows 上用 DPAPI 加密保存，macOS / Linux 上明文存在本机数据库里（与 2.5.0 一致；mac 设置页有提示，钥匙串要开发者签名，暂不做），
  界面只拿到 `has_password`，http:// 地址会提示明文风险

### 窗口特殊功能
- **普通模式**：可拖动、可缩放；主窗口是透明窗口，界面自己画圆角底色：浅色主题白底，深色主题用设置里的「界面底色」+「背景透明度」（透出桌面）
- **固定模式**（标题栏锁按钮 / 托盘「固定模式」）：
  - 固定在用户指定位置：不可拖动、不可缩放（`platform::subclass` 的 `FLAG_LOCK_GEOMETRY` 拦截命中测试）
  - 不在任务栏、不参与 Alt+Tab（`WS_EX_TOOLWINDOW`）；被 Win+D / Win+M 最小化后由 200ms 轮询立即还原
  - 可选贴边自动隐藏、唤起时置顶（设置 → 常规）；经典固定时没有圆角与描边
- **固定模式时嵌入桌面**（设置 → 常规「固定模式时，嵌入桌面中」，仅 Windows 显示，Win11 24H2+ 保证 Win+D 免疫）：
  - 不是第三种模式，而是固定模式的一个全局开关：开启后进入固定模式即走嵌入路径，关闭则是经典固定；当前已固定时切换开关立即生效
    （主窗口观察到 `AppState.fixed_embed_desktop` 变化 → `window_mode::on_embed_preference_changed`）
  - 主窗口仍是顶层窗口：owner=Progman（`GWLP_HWNDPARENT`）+ 永远置底（`SetWindowSubclass` 处理 `WM_WINDOWPOSCHANGING`，`FLAG_BOTTOM`）+
    去掉 `WS_MINIMIZEBOX` + `WS_EX_TOOLWINDOW`，位于桌面图标之上、所有应用窗口之下；Win+D / Win+M / 任务栏"显示桌面"时随 Progman 一起被抬起
  - 透明背景 / 圆角 / DPI 与普通模式零差异（不做 SetParent 子窗口化）；不可拖拽、不可缩放；贴边隐藏与唤起置顶不生效
  - 200ms 轮询检测 Explorer 重启与样式被改（`platform::embed::needs_reattach`），自动重挂
  - 持久化：settings `fixed_embed_desktop`（v27）；当前是否固定仍由 `is_fixed` / `screen_configs.is_fixed` 记录
  - 实现：`crates/app/src/services/window_mode.rs`（`TargetMode` / `apply_mode` / 200ms 轮询）、`platform/embed.rs`、`platform/subclass.rs`；
    纯逻辑在 core `window_logic.rs`；契约见 `.trellis/spec/backend/window-modes.md`
  - 非 Windows：开关不显示；`TargetMode::for_state` 只在 Windows 上选嵌入，同步来的开关不会让固定失败
- **macOS 上的固定模式**（2.5.0 mac 版的简化语义，同一套 `window_mode` 状态机与 200ms 轮询）：`setMovable:NO` + 去掉可缩放（解锁时恢复）、
  被最小化（⌘M）立即还原、托盘 / 贴边唤起临时置顶（`NSFloatingWindowLevel`）、贴边隐藏用 `NSEvent.mouseLocation` 全局光标
  （往有菜单栏的屏幕顶边藏会被 AppKit 挡回，状态回滚为显示；贴边隐藏中被 Dock / ⌘Tab 激活时按托盘唤起拉回，`window_mode::on_main_activated`）；
  没有"不进任务栏 / ⌘Tab"（程序坞图标与 ⌘Tab 是整个应用的），没有嵌入桌面。契约见 `.trellis/spec/backend/window-modes.md`「Scenario: macOS」
- **多屏幕位置记忆**：按屏幕组合（`screen_configs`）记住主窗口位置、尺寸与是否固定；托盘「重置位置」把主窗口放回主显示器左上区域。
  macOS 的物理像素口径见 `platform/mac_geometry.rs`（全局点坐标 × 所有屏的最大缩放；屏幕组合 ID 按各屏自身缩放，与 2.5.0 一致）

### 桌面收纳箱（仅 Windows）
- 托盘「新增收纳箱」在光标所在显示器工作区的右上角找空位新建（默认 4 列 × 3 行、标题"收纳箱 / 收纳箱 2…"）。每个收纳箱是一个
  无系统标题栏的透明 GPUI 窗口（`WindowKind::Fence`，登记 key `fence-{id}`，`crates/app/src/windows/fence/`）
- **始终嵌入桌面层**：与主窗口嵌入共用 `platform::embed`（owner=Progman + 永远置底 + `WS_EX_TOOLWINDOW` + 无 `WS_MINIMIZEBOX`），
  Win+D 后可见、不进任务栏 / Alt+Tab、不盖住应用窗口；建窗参数一次给齐（`resizable: false`、`minimizable: false`、Windows 上先隐藏建好、
  定位并嵌入后再显示）；200ms 守护在 Explorer 重启后自动重挂（`services/fences.rs`）
- **真收纳**：外部拖入（`ui::external_drop`）的文件 / 快捷方式 / 文件夹经 `IFileOperation` 从桌面移到 `%USERPROFILE%\MiniTodo收纳箱\<收纳箱名>\`，
  重名加 ` (2)` 绝不覆盖，公共桌面项弹 UAC；**文件夹是内容的 source of truth**（`notify` 监听，Explorer 里增删随之刷新），
  数据库只存元数据与项目的排序 / 原始路径 / 隐藏标记。不允许整体收纳用户桌面 / 公共桌面文件夹本身
- 双击打开（`ShellExecuteExW`）、自绘右键菜单（打开 / 打开所在位置 / 隐藏或取消隐藏 / 移回桌面 / 移动到…）、箱内拖动排序、Ctrl+滚轮图标大小、
  双击标题改名（收纳文件夹同步改名，失败则保留旧文件夹名并提示）、折叠（悬停 / 拖文件经过时临时展开）、
  「固定」= 锁定位置和大小（仍可拖入、折叠、改名）、每个收纳箱独立的背景色 / 透明度
- **隐藏图标**（v30，只影响收纳箱的显示，不改文件属性、不移动文件）：以 `.` 开头的项目自动隐藏（固定开启，不能单独取消隐藏；
  拖入时提示一次）；右键「隐藏」/「取消隐藏」写 `fence_items.hidden`（`fence::items::set_hidden`）；「更多」→「显示隐藏的图标」
  （每个收纳箱独立，`fences.show_hidden`）打开时隐藏项半透明显示，关闭时不显示，全部隐藏时空状态显示"N 个项目已隐藏"。
  对账保留隐藏标记（含只改大小写的重命名），跨箱移动不携带；带 Windows 隐藏 / 系统属性的文件维持原样永不显示；删除收纳箱时隐藏项一并移回
- 移动 / 缩放：标题栏移动超过 3px 才 `platform::begin_window_drag`（标题栏不用 `drag_region()`，否则双击改名收不到），缩放用窗口内 8 个手柄 +
  按客户区 `SetWindowPos`；只有用户拖动 / 缩放结束与托盘重置才 `fence::commit_geometry` / `reset_positions` 落库，系统挪窗不落库。几何一律是客户区的物理像素
- 删除收纳箱：项目全部移回原位置（原目录不存在则回桌面）、空文件夹删除后才删记录并关窗口；有项目移不回去时删除中止、
  整个收纳箱保留并提示原因
- 按屏幕组合记忆位置（`fence_layouts`，ID 与 `screen_configs.config_id` 同格式，都由 core `settings::screen_config_id` 计算），约 2 秒检测一次
  屏幕组合变化；设置里删除屏幕配置时连带删除对应布局
- 托盘「显示/隐藏所有收纳箱」「重置收纳箱位置」作为兜底入口；首次从 OneDrive 重定向的桌面收纳时提示一次
- 关键实现：core `fence/`、`services/fence_fs/`；app `services/fences.rs`、`windows/fence/`、`platform/embed.rs`；
  契约与易错点见 `.trellis/spec/backend/desktop-fences.md`
- 已知限制：Win10 / Win11 ≤23H2 上 Win+D 可能盖住收纳箱

### 系统功能
- 托盘（Windows）/ 菜单栏图标（macOS，单色模板图标 `assets/tray-template.png`，随菜单栏深浅色着色）：左键单击唤起主窗口（300ms 去抖）；
  右键菜单：添加待办项、新增收纳箱、显示/隐藏所有收纳箱、重置收纳箱位置（这三项仅 Windows）、固定模式（勾选）、重置位置、打开设置、
  开机自启动（勾选）、退出
- 单实例：再次启动时把参数转交给已运行的实例并唤起其主窗口（开机自启拉起的不唤起）；Windows 命名互斥量 + `WM_COPYDATA`，macOS Unix socket
- 开机自启（Windows 注册表 Run 项 / macOS LaunchAgent）、版本更新检查（GitHub `releases/latest`，标题栏版本号旁红点；不会提示预发布版）
- 运行日志落盘（2MB 轮转，保留 1 份旧日志）

### macOS（预览）
- 能力对齐 2.5.0 mac 版（再加上 3.0 的全部界面与功能），没有收纳箱、嵌入桌面；**未经真机验收**，只靠 CI 的 macOS runner 自动化验证
  （`pc-gpui-e2e-macos.yml`）与用户反馈
- 身份与 2.5.0 mac 版相同（bundle id、`.app` 名、数据目录、日志、单实例 socket、LaunchAgent，见「兼容旧版沿用的名字」）；
  Dock 图标照常显示（不设 `LSUIElement`，GPUI 启动时会把激活策略重设为 Regular，要隐藏只能用 `set_activation_policy`）
- 窗口：所有窗口仍是自绘标题栏，系统红绿灯建窗后隐藏（`Hwnd::prepare_after_open`，改 style mask 后再隐藏一次），
  `WindowSpec` 设 `app_owns_titlebar_drag`；运行时摆窗 / 显示隐藏 / 置顶 / 几何锁定在 `platform/macos/ns.rs`（objc2 调 NSWindow / NSScreen）
- 应用菜单（`services/mac_menu.rs`，GPUI 不会自动建）：Mini Todo（关于 / 设置… ⌘, / 服务 / 隐藏 ⌘H / 隐藏其他 ⌥⌘H / 全部显示 / 退出 ⌘Q）、
  编辑（撤销 / 重做 / 剪切 / 拷贝 / 粘贴 / 全选，系统文件面板里的 ⌘C / ⌘V 靠它）、窗口（最小化 ⌘M / 关闭窗口 ⌘W）
- 关窗口（含主窗口、⌘W）不退出进程，菜单栏图标常驻；点 Dock 图标 / 在访达里再次打开时唤起主窗口（`on_reopen`，GPUI 只在没有可见窗口时回调）；
  ⌘Q / 应用菜单 / 菜单栏图标「退出」退出。从菜单栏图标 / 单实例唤起时先 `cx.activate(true)` 把应用切到前台
- 字体：界面 `.SystemUIFont`（San Francisco）+ `PingFang SC`，代码 `Menlo`，引用块 `Georgia` + `Songti SC`；设置里的字体下拉用 GPUI 的
  CoreText 枚举（去掉 `.` 开头的私有字体）
- 文件对话框（导入导出、子任务导入）用 GPUI 的原生 `NSSavePanel` / `NSOpenPanel`（非模态，没有扩展名过滤器）；WebDAV 密码明文保存（设置页提示）
- 开机自启从 App Translocation 临时位置（下载后没拖进「应用程序」就直接打开）运行时拒绝写入，提示先拖进「应用程序」

## 开发规范

### UI 设计规范
- **控件**：`crates/app/src/ui/` 下自绘的 Element Plus 风格控件，同一种控件只保留一个实现（缺能力就给现有控件加 builder 方法）
- **图标库**：Element Plus Icons（SVG 放 `crates/app/assets/icons/{ep,app}/`，在 `assets.rs` 表里登记）
- **颜色 / 尺寸 / 字体只从 `theme.rs` 取**，数值照抄 UI 规格（`.trellis/tasks/archive/2026-10/10-15-gpui-rewrite/research/ui-*.md`）
- **禁止使用 emoji 图标**
- 设计理念：简洁现代、去除卡片边框、极简列表

#### 优先级颜色
| 级别 | 颜色代码 | 描述 |
|------|----------|------|
| 高 | #EF4444 (红色) | 紧急重要任务 |
| 中 | #F59E0B (橙色) | 一般重要任务 |
| 低 | #10B981 (绿色) | 不紧急任务 |

### 数据库设计
- **数据库类型**：SQLite（rusqlite，bundled）
- **存储位置**：`%LOCALAPPDATA%\mini-todo\`（core `db::paths::app_data_dir()` = `dirs::data_local_dir()/mini-todo`；
  macOS 为 `~/Library/Application Support/mini-todo/`（与 2.5.0 mac 版相同）；Linux 为 `~/.local/share/mini-todo/`）
  - `data.db`、`images/`（用户图片）、`backups/`（迁移/导入前的 `VACUUM INTO` 快照，保留最近 5 份）、`holidays/`、
    `fence-icons/`（收纳箱图标 PNG 缓存，启动对账后清理过期项）、`ui-state.json`（展开全部子任务等界面状态）
  - 这个目录同时是 Windows 安装包的默认安装目录（与旧 Tauri 版一致）；安装器卸载只删自己装的程序文件，绝不递归删除该目录
  - 日志：`%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`；macOS `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`
    （没有 `logs` 子目录）（app `services::logging`，2MB 轮转；目录名 / 位置都为兼容旧版沿用）
  - 收纳箱文件：`%USERPROFILE%\MiniTodo收纳箱\<收纳箱文件夹>\`（用户可见的真实文件夹，有收纳箱后才创建；不在数据目录里，
    卸载 / 删库都不会丢文件）
- **迁移版本**：当前 v1~v30，通过 `pc-gpui/crates/core/src/db/migrations.rs` 的迁移表管理（`LATEST_VERSION`）；有待执行迁移时先备份，
  迁移期间关闭外键、提交前 `foreign_key_check`；库版本比程序新时只记警告、按现状继续运行（没有降级）
  - v23：移除所有 AI Agent / 任务调度 / 工作流相关表和字段（详见迁移注释）
  - v24：新增 `webdav_last_modified` settings key
  - v25：新增 `top_on_wake` settings key，贴边唤起时是否临时置顶
  - v26：新增 `window_bg_color` / `window_bg_alpha` settings key，窗口底色与背景透明度
  - v27：新增 `fixed_embed_desktop` settings key，固定模式时是否嵌入桌面（仅 Windows）
  - v28：`sync_meta`（本地变更计数 `local_seq` / `synced_seq`，todos/subtasks 增删改触发器维护）、
    `tombstones` 表、存量时间统一为 `YYYY-MM-DD HH:MM:SS`、新增 `webdav_remote_etag` / `webdav_synced_settings_at`
  - v29：桌面收纳箱 `fences` / `fence_items` / `fence_layouts` 三张表（后两张 `ON DELETE CASCADE`）与 settings key
    `fence_onedrive_notice_shown`；均为设备本地数据，不接 `local_seq` 触发器与墓碑、不导出不同步
  - v30：收纳箱隐藏图标：`fence_items.hidden`（右键「隐藏」）、`fences.show_hidden`（「显示隐藏的图标」），均 `INTEGER NOT NULL DEFAULT 0`；
    同 v29 是设备本地数据。旧版本打开 v30 库照常运行，只是看不到隐藏效果

#### 主要数据表

| 表名 | 说明 |
|------|------|
| `todos` | 待办事项（含重复提醒字段） |
| `subtasks` | 子任务（标题 + Markdown 内容 + 完成态） |
| `settings` | 应用设置（键值对，带 `updated_at`，只经 `db::settings_kv` 写入） |
| `screen_configs` | 屏幕配置 |
| `sync_meta` | 同步计数器（`local_seq` / `synced_seq`），由触发器维护 |
| `tombstones` | 删除墓碑 `(entity_type, entity_id, deleted_at)`，保留 30 天 |
| `fences` | 桌面收纳箱元数据（标题、收纳文件夹名、物理像素几何、折叠 / 锁定、外观、图标大小、显示隐藏的图标） |
| `fence_items` | 收纳箱项目的排序、原始路径与隐藏标记（内容以收纳文件夹为准，列举时对账） |
| `fence_layouts` | 收纳箱按屏幕组合记忆的位置 `(fence_id, screen_config_id)` |
| `migrations` | 迁移版本记录 |

#### 时间格式约定（PC / cloud 一致）

- 存储与同步一律 `YYYY-MM-DD HH:MM:SS`（本地墙钟、无时区）：`notifyAt` / `startTime` / `endTime` /
  `createdAt` / `updatedAt` / 墓碑 `deletedAt` / `settingsUpdatedAt`
- 读取兼容空格 / `T`、有无秒、小数秒、`Z`/`±HH:MM` 后缀（换算本地）、仅日期（开始 00:00:00、截止 23:59:00、提醒 09:00:00）
- 存储格式以 core `db::time` 为准（`normalize_datetime`、`format_canonical`、`parse_local_datetime`），界面显示 / 输入用
  `crates/app/src/util/datetime.rs`；**禁止**手写 `split('T')` 之类的解析
- 对已有记录写 `updated_at` 时取 `max(now, 旧值 + 1 秒)`（Rust `db::time::superseding`，SQL 片段 `SQL_SET_UPDATED_AT`），
  保证同一记录的新版本时间戳严格递增（秒级 LWW 下同秒两次编辑不丢）；参与同步的设置项按整个设置块的版本同样递增；
  删除墓碑 `deleted_at = max(now, 记录 updated_at)`；无法解析的时间在比较中视为最旧

### 数据导入导出与同步

> **重要**：当数据库结构变更（新增表/字段/设置项）时，必须同步更新导入导出、WebDAV 同步与 cloud 的字段校验！
> 同一份数据还会被 cloud 与其它设备上已安装的旧版本读写，规则见 `.trellis/spec/pc-gpui/data-compat.md`。

- **导出版本号**：当前 `4.0`（`pc-gpui/crates/core/src/data.rs` → `EXPORT_VERSION`）
- **关键文件**（均在 `pc-gpui/crates/core/src/` 下）：
  - 模型定义：`db/models.rs` → `Todo`、`SubTask`、`ExportData`、`AppSettings`
  - 设置读写（唯一来源）：`data.rs` → `read_app_settings` / `write_app_settings`；
    参与同步的键 `SYNCED_SETTING_KEYS` 与设置版本 `settings_version` 在 `db/settings_kv.rs`
  - 导入导出：`data.rs` → `export_json` / `import_json` / `export_to_file` / `import_from_file`（仅手动导入导出使用）
  - WebDAV 同步：`sync/mod.rs` → `SyncData`、`run`、`run_sync_blocking`、`build_sync_doc`；
    记录读写/合并/墓碑：`db/sync_store.rs`（`merge_remote`、`load_todos_with_subtasks`）
  - cloud 写入校验：`cloud/src/model.rs`（新增记录字段必须同步加入 `TODO_FIELDS` / `SUBTASK_FIELDS`，否则 AI 写入会被 400）
  - 界面侧没有单独的前端类型：`crates/app` 直接用 core 的 Rust 模型

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
- 其它设备上可能还在跑旧版本（≤ 2.5.0 的 Tauri 版或更早的 3.x）：新增字段 / 设置键在它们那里会被丢掉，见 `.trellis/spec/pc-gpui/data-compat.md`。

#### 维护检查清单

当新增数据库迁移时，请检查（路径均在 `pc-gpui/crates/core/src/` 下，另注明的除外）：
1. 新增的 **settings 键值** 是否已加入 `AppSettings`（含 `#[serde(default)]`）、`data.rs` 的 `read_app_settings` / `write_app_settings`；
   需要跨设备同步的再加入 `SYNCED_SETTING_KEYS`（设备相关的不要加）；需要界面的再改 `crates/app`（`state.rs`、`windows/settings/`）。
   例外：纯设备本地的提示 / 状态键（如 v29 的 `fence_onedrive_notice_shown`）有意不进 `AppSettings`，
   登记在 `.trellis/spec/backend/sync-protocol.md` §3.6
2. 所有 settings 写入走 `db::settings_kv::set_setting`（禁止 `INSERT OR REPLACE`，否则每次写都会刷新同步版本）
3. 新增的 **数据表** 是否需要同步：需要则加触发器维护 `local_seq`、在 `db/sync_store.rs` 中实现读写/合并/墓碑，
   并纳入 `sync/mod.rs` 的 `build_sync_doc` 与 cloud 的 `sync/doc.rs`、`sync/merge.rs`
4. 新增的 **记录字段**：`db/models.rs`（`#[serde(default)]`）、`db/sync_store.rs` 的行读写、`data.rs` 的导入导出、
   cloud `src/model.rs` 允许字段与类型
5. 时间字段经 `db::time::normalize_datetime` 规范化；界面经 `crates/app/src/util/datetime.rs`
6. 旧版数据导入的兼容性（新字段必须有默认值），以及对仍在用旧版本的设备的影响（`.trellis/spec/pc-gpui/data-compat.md`）
7. 导出版本号是否需要递增
8. `pc-gpui/` 下 `cargo test --workspace --locked` 与 `cloud/` 下 `cargo test --locked` 都要绿；同步相关的再跑 Linux e2e 的 S 组

## 兼容旧版沿用的名字

旧 Tauri 版（≤ 2.5.0）用过的这些名字全部原样沿用，保证安装包原地升级、数据与设置不丢、两版不会同时运行。**不要改**：

| 项 | 值 | 位置 |
|---|---|---|
| 数据目录 | `%LOCALAPPDATA%\mini-todo\` | core `db/paths.rs` |
| 日志 | `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（旧 Tauri identifier 作目录名） | app `services/logging.rs` |
| 单实例 | 命名互斥量 `com.tauri-app.mini-todo-sim`；接收窗口类名 `com.tauri-app.mini-todo-sic`、标题 `com.tauri-app.mini-todo-siw`；`WM_COPYDATA`（`dwData = 1542`，内容 `"<cwd>\|<arg0>\|…"`）——与旧版 `tauri-plugin-single-instance` 同名同格式，新旧两版互相识别 | app `services/single_instance.rs` |
| 开机自启 | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\mini-todo = "<exe>" --autostart`；已开启时每次启动把路径刷新为当前 exe | app `services/autostart.rs` |
| 系统通知 AppUserModelID | `com.tauri-app.mini-todo` | app `lib.rs` 的 `APP_IDENTIFIER` |
| 安装身份 | 产品名 `mini-todo`、卸载键 `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\mini-todo`、默认目录 `%LOCALAPPDATA%\mini-todo`（= 数据目录，卸载只删程序文件）、`$SMPROGRAMS\mini-todo.lnk`；已装旧版时先以 `/S /UPDATE` 调用旧卸载器再安装 | `pc-gpui/scripts/windows-installer.nsi`，契约见 `.trellis/spec/pc-gpui/release.md` |
| 收纳根目录 | `%USERPROFILE%\MiniTodo收纳箱\` | core `services/fence_fs/location.rs` |

macOS（与 2.5.0 mac 版 `mini-todo_2.5.0_universal.dmg` 的实物 Info.plist 与插件实现逐项一致；同样**不要改**）：

| 项 | 值 | 位置 |
|---|---|---|
| 应用身份 | bundle id `com.tauri-app.mini-todo`（= `APP_IDENTIFIER`）；`mini-todo.app`，`CFBundleExecutable` / `CFBundleName` / `CFBundleDisplayName` 都是 `mini-todo`；不设 `LSUIElement` ——用户把新包拖进「应用程序」时直接替换旧版，通知授权、LaunchAgent 里的路径都不变 | `pc-gpui/scripts/macos/bundle.sh`，契约见 `.trellis/spec/pc-gpui/release.md`「macOS 打包」 |
| 数据目录 | `~/Library/Application Support/mini-todo/`（`dirs::data_local_dir()/mini-todo`，与 Windows 同一表达式） | core `db/paths.rs` |
| 日志 | `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`（2.5.0 tauri-plugin-log `LogDir` 的位置，没有 `logs` 子目录） | app `services/logging.rs` |
| 单实例 | Unix socket `/tmp/com_tauri_app_mini_todo_si.sock`；第二个实例写 `"<cwd>\0\0<arg0>\0<arg1>…"` 后关闭连接退出；首个实例遇 `NotFound` / `ConnectionRefused` 删残留后 `bind`，退出时只删自己绑定的那个文件——与 tauri-plugin-single-instance 2.3.7 mac 实现同名同格式，新旧两版互相识别 | app `services/single_instance/unix_socket.rs` |
| 开机自启 | LaunchAgent `~/Library/LaunchAgents/mini-todo.plist`：`Label = mini-todo`、`ProgramArguments = [<可执行文件绝对路径>, "--autostart"]`、`RunAtLoad`（auto-launch `set_use_launch_agent(true)`，不用 AppleScript 登录项）；已开启时每次启动刷新路径 | app `services/autostart.rs` |
| 系统通知归属 | bundle id `com.tauri-app.mini-todo`（通知中心的授权与显示设置挂在它上面） | app `services/notify.rs` |

## 核心架构概念

### crate 划分与线程模型

- `crates/core`（`mini-todo-core`）：数据库、同步、导入导出、提醒、收纳箱文件层等全部业务逻辑，**阻塞函数**，错误是面向用户的中文 `String`；
  不依赖 GPUI / Tauri / tokio（`reqwest::blocking` 不能在 tokio runtime 里调用）。公开 API 见 `pc-gpui/crates/core/README.md`。
- `crates/app`（`mini-todo`）：一个进程一个 GPUI `Application`（`QuitMode::Explicit`，托盘常驻），所有窗口共享同一个 `App`。
  阻塞调用一律放后台：`db::spawn_db(cx, |db| core::…)` 返回 `Task<Result<T, String>>`，回前台用 `WeakEntity::update`。
- App 被借用时（事件处理 / `update` / `render`）不能同步调用会给窗口发消息的 Win32：`platform::*(window, cx, …)` 写操作已自动延后，
  `platform::Hwnd` 原始调用只在 `cx.spawn` 的 async 块里、`update` 之外用（`.trellis/spec/pc-gpui/gpui-conventions.md` §2.2）。
  macOS 的 `setFrame:` / `orderOut:` / 改 style mask 同样会同步触发 GPUI 回调，规则相同；AppKit 调用另外只能在主线程（GPUI 的前台），
  `Hwnd` 在 mac 上是 NSWindow 的窗口号、每次重新查找（`gpui-conventions.md` §2.15）。

### 启动顺序（`crates/app/src/lib.rs` 的 `run()`）

```
日志 → 单实例（已有实例：转交参数后退出）→ 打开数据库（失败弹框退出）→ Application(QuitMode::Explicit；macOS 另注册 on_reopen）
  → 主题 →（macOS）应用菜单 → UiLayer / Db / WindowManager / AppState → 单实例唤起监听 → 主窗口（按屏幕配置恢复几何与固定模式）
  → 托盘 / 菜单栏图标 → 自动同步定时器 → 提醒调度 → 恢复收纳箱窗口 → 开机自启自愈 → 加载设置与待办 →（debug）MINI_TODO_DEBUG_OPEN
```

### 跨窗口状态与通信

旧版的 Tauri invoke / emit 在 GPUI 版里都变成同进程的直接调用：

| 旧版（Tauri / WebView） | GPUI 版 |
|---|---|
| Pinia `todoStore` / `appStore` | `state::AppState`（`Entity` + Global），各窗口 `cx.observe`；写操作经 `AppState` 方法在后台执行 |
| `invoke` 命令 / `spawn_blocking` | `db::spawn_db(cx, \|db\| core::…)` 放后台，`WeakEntity::update` 回前台 |
| `sync-completed` | `services::sync::run_sync` → `AppState::apply_sync_report`（本地数据变了 → 重新拉待办；`settings_applied` → 重读设置） |
| `data-imported` / `app-settings-changed` / `todo-font-changed` | 直接改 `AppState`，所有窗口随之刷新 |
| `subtask-memory-ready` / `-init` / `-save` | 子任务窗口直接读写新建待办窗口的 `Entity<DraftSubtasks>`（`subtask::open_draft`） |
| `fence-items-changed` | `fence::items::FenceRefresh`：目录监听回调（`fence-watch` 线程）经 channel 回前台 → 对应收纳箱窗口 |
| 托盘事件 | tray-icon 回调（Windows 窗口过程 / macOS 主线程的 AppKit 事件）只把事件投进 channel → 前台执行动作（`services::tray`） |
| 独立 WebView 窗口（label） | GPUI 窗口：`windows::WindowManager` 统一登记 / 模态子窗口 / 遮罩 / owner / 关闭后刷新 / 居中 |
| tao 窗口能力 + `commands/window.rs` Win32 | `platform/`（`windows.rs` Win32、`macos/` AppKit、`fallback.rs` Linux、`embed.rs` 嵌入桌面、`subclass.rs` 置底与几何锁定） |
| Tauri 默认 App 菜单（mac） | `services::mac_menu`（`set_menus` + 全局 action + 快捷键；`on_reopen` 唤起主窗口） |

### 窗口一览（`windows::WindowManager`，`WindowKind`）

| 窗口 | 入口 | 说明 |
|---|---|---|
| 主窗口 | `main_window::open` / `show_and_activate` | 380×600，最小 320×400，透明，可缩放；开日历时加宽 |
| 待办详情 / 编辑 | `editor::open(cx, parent, todo_id, EditorMode::{View, Edit})` | 1080×600；`todo_id = None` 为新建 |
| 子任务 | `subtask::open` / `subtask::open_draft` | 800×750，查看 / 编辑 |
| 已完成 | `completed::open` | 460×550，搜索、恢复、删除 |
| 设置 | `settings::open(cx, parent, SettingsTab::{General, Appearance, Data, Screen, About})` | 680×560；常规 / 外观 / 数据与同步 / 屏幕配置 / 关于 |
| 应用内通知 | `notification::open` | 320×120，透明、`WindowKind::PopUp`、不抢焦点 |
| 收纳箱 | `services::fences`（`fence::open` 等转发） | 每个收纳箱一个窗口，key `fence-{id}`，始终嵌入桌面层，不可关闭（删除走 `close_deleted`） |
| 控件样例 | `gallery::open` | 仅调试（`MINI_TODO_DEBUG_OPEN=gallery`） |

打开方是主窗口 / 已完成 / 编辑窗口时，子窗口是模态的（同一时刻一个，打开方显示遮罩，Windows 上以打开方为 owner；macOS 没有 owner 关系）；
打开方是主窗口时，子窗口关闭后自动 `reload_todos`，设置窗口关闭后再重读设置并重启自动同步。

### 通知调度流程

```
services::scheduler（后台线程）
  ├── 启动 5s 后 reminder::catch_up_missed_repeats（补发错过的重复提醒）
  └── 每到整分钟：notification_kind(db) → reminder::run_due(db, deliver)
        ├── 扫描 completed = 0 AND notified = 0 AND notify_at - COALESCE(notify_before,0) <= now（按时间排序）
        ├── 逐条投递，单条出错只记日志、不影响其它提醒
        ├── System → 系统通知：Windows Toast 在后台线程发（失败不推进）；macOS 经通道交给前台 show_system_notification
        │     （拿不到投递结果，交出即视为已投递）
        ├── App → 经通道交给前台建应用内通知窗口（建窗失败不推进、下一分钟重试）
        ├── 重复提醒：解析 notify_at（空格 / T 均可）→ 计算下一次 → 写规范格式并重置 notified = 0
        └── 非重复：notified = 1
```

### 待办编辑/详情流程

```
列表 / 四象限 / 日历 / 已完成
  └── 点击待办 → editor::open(.., Some(id), EditorMode::View)（新建：FAB / 托盘「添加待办项」→ None，直接编辑）
       └── 编辑窗口：只读详情（标题 + MD 渲染 + 通知状态 + 优先级）⇄ [编辑] 原地切换表单
            ├── 写操作经 AppState（create_todo / update_todo / …，后台执行、更新本地副本）
            └── 描述字段使用 ui::markdown_editor（源码 + 预览，值为存储形式 minitodo-image://）
```

### 子任务编辑流程

```
编辑窗口子任务面板
  └── 查看 / 编辑 → subtask::open（已落库）/ subtask::open_draft（新建待办的暂存子任务）
       └── ui::markdown_editor 加载 Markdown
            └── 图片粘贴 / 拖入 → 后台 core::todo::save_image → images 目录 → Markdown 写 minitodo-image://<name>
```

### 主窗口刷新

- 每 5s `AppState::refresh_if_changed`（读 `todo::change_seq` = `sync_meta.local_seq`，变化时才 `fetch_todos`）；子窗口关闭、同步完成也走同一个
  单飞 `fetch_todos`（拉取期间有本地写操作则丢弃并补拉）；主窗口有模态子窗口时后台刷新跳过

### WebDAV 同步流程

```
services::sync::run_sync(cx, SyncMode)（后台线程；AppState.syncing 期间标题栏图标旋转）
  └── core sync::run(db, mode)（单同步互斥，"同步正在进行中"）
        ├── GET sync-data（有基准 ETag 时 If-None-Match；本地有未上传变更 / 手动同步 / 每第 10 轮时无条件 GET）
        │     ├── 304 → 远端未变
        │     ├── 200 → 解析（失败直接报错）→ 单事务逐条合并（LWW + 墓碑 + 设置 LWW）→ 下载缺失图片
        │     └── 404 → 首次上传
        ├── 需要上传？（local_seq ≠ synced_seq / 设置更新 / 远端缺本地记录或墓碑）
        │     ├── 先上传缺失图片（一次 PROPFIND Depth:1 列清单）
        │     └── 条件 PUT：已知 ETag 时 If-Match（去 W/），从不发 If-Unmodified-Since
        │           412 → 等 1.1s → 重新 GET → 合并 → PUT（最多 4 次）
        └── 成功：基准取 PUT 响应 ETag → HEAD → PROPFIND；记录 synced_seq；返回 SyncReport
  └── AppState::apply_sync_report（本地数据变了 → 重新拉待办；settings_applied → 重读设置与运行时偏好）
SyncMode::ForcePull  让本地等于远端（删除本地独有记录，不写墓碑）
SyncMode::ForcePush  让远端等于本地（为远端独有记录写墓碑后上传）
```

WebDAV 服务端差异（实测，见 `.trellis/spec/backend/sync-protocol.md`）：Apache mod_dav 的 `If-Unmodified-Since`
对未改动文件也会 412、写入后 1 秒内 ETag 为弱；nginx/Caddy 忽略 PUT 前置条件、PROPFIND 不返回 getetag、
`If-Modified-Since` 按整秒比较。所以两端只用 `If-Match` / `If-None-Match`，并且写前总是先 GET 合并。

### 桌面收纳箱流程

```
托盘「新增收纳箱」→ services::fences::create_fence
  └── 后台 fence::create（先建文件夹 → INSERT fences → 记当前屏幕组合布局）
       → open_fence_window（Windows 先隐藏建窗 → 按客户区定位 → TOOLWINDOW + 嵌入桌面层 → 显示；失败 rollback_create）
       → ensure_watcher；200ms 守护（Explorer 重启重挂；每 10 tick 检测屏幕组合）
拖入 → 后台 fence::items::move_in（先登记 begin_moving → IFileOperation → upsert fence_items，搬运 / 记录原始路径）→ FenceRefresh
目录变化 → notify（300ms 去抖）→ fence-watch 线程里数据库锁内列举对账 → FenceRefresh 经 channel → 对应收纳箱窗口
删除 → 后台 fence::delete → restore_all → 文件夹已清空才删记录 + close_deleted 关窗，否则整个收纳箱保留
```

### 已知偏差与限制

1. **Markdown 编辑**：旧版 Milkdown 所见即所得无法在 GPUI 中 1:1 还原，编辑模式改为"Markdown 源码 + 渲染预览"（放大编辑弹窗是源码 / 预览分栏）；
   只读渲染、图片粘贴 / 拖入上传、`minitodo-image://` 引用、链接白名单规则不变。
2. **中文换行**：GPUI 的换行器不做避头尾，全角标点（`，。、；：！？）`）可能落到行首，多行中文的折行位置与浏览器可能差一个字。
3. GPUI 没有文字阴影、多色标渐变、位图变换等，个别装饰用近似实现（见 spec `gpui-conventions.md` §2.13）。
4. **Linux 仅用于 CI、截图回归与 e2e**：没有托盘 / 单实例 / 开机自启 / 嵌入桌面 / 收纳箱真实文件操作，最后一个窗口关闭即退出，设置里的字体下拉为空。
   Windows 专有能力（嵌入桌面、收纳箱、DPAPI、Toast、托盘、DirectComposition 透明）在 Linux 上只做交叉 clippy，靠 CI 的 Windows e2e 与用户实机验收。
5. Win10 / Win11 ≤23H2 上 Win+D 可能盖住嵌入桌面的主窗口与收纳箱（显示桌面抬起的是 `WorkerW` 而不是 Progman）。
6. **macOS 是预览版，未经真机验收**：维护者没有 Mac，mac 代码只能在 macOS runner 上编译、跑 e2e（runner 屏幕 1x、单显示器、半虚拟化 GPU），
   Retina、多显示器 / 混合缩放、Spaces / 全屏、阴影与圆角观感、Intel 机型（x86_64 切片只在 arm64 runner 上交叉编译、`lipo` 合并，
   CI 没有运行过它）都靠用户反馈。另外：
   - 只有 ad-hoc 签名、不公证：首次打开要放行（`xattr -cr` 或「隐私与安全性 → 仍要打开」），每次升级都要重新放行
   - 系统通知拿不到投递结果（授权被拒也视为已投递），`cargo run` 的裸二进制收不到通知；runner 上测不了通知横幅与授权
   - 没有嵌入桌面、收纳箱，固定模式没有"不进 Dock / ⌘Tab"；固定模式在 Spaces / 全屏下的行为（`collectionBehavior`）没有处理
   - 混合缩放（Retina 笔记本 + 1x 外接屏）时主窗口位置的物理像素口径与 2.5.0 不同，2.5.0 记下的位置可能恢复不到（设备本地数据，只丢一次）
   - 文件对话框没有扩展名过滤器；WebDAV 密码明文；最低 macOS 11（2.5.0 写的是 10.13）

## cloud/ 子项目（云端 API）

`cloud/` 是一个独立的 Rust crate（**不在** `pc-gpui/` 的 Cargo workspace 里），部署到 VPS 上为
AI / Claude Code Skill 提供 mini-todo 数据的 HTTP REST 访问能力。部署、配置、Skill 见 `cloud/README.md`。
与 PC 的契约（和上文"时间格式约定""合并规则"是同一套）：

- **共用同一个 WebDAV `sync-data.json.gz` 通道**。云端是 WebDAV 客户端，不依赖 PC 在线
- **WebDAV 是 source of truth**，云端 SQLite 是缓存；未推送的本地改动（`meta.dirty`）推送成功前不会被拉取覆盖
- **时间**：`YYYY-MM-DD HH:MM:SS`，按 `config.toml` 的 `timezone`（IANA，须与 PC 一致）每次重新换算墙钟（DST 安全）
- **写冲突**：写前先 GET 合并，条件 PUT 只用 `If-Match`（去 `W/`）；412 等待后重新 GET → 合并 → PUT
- **字段契约**：SQLite 是 KV-style（`todos(id, data_json, updated_at)`），但 API 写入只接受 PC 模型里的字段
  （`src/model.rs`，以 `pc-gpui/crates/core/src/db/models.rs` 为准），未知字段 / 类型错误 → 400；合并远端时未知字段原样保留（可能是更新版 PC 加的新字段）
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
├── deploy/                   # minitodo-cloud.service（systemd 沙箱加固）、Caddyfile.example
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
# PC（在 pc-gpui/ 下），门槛见 .trellis/spec/pc-gpui/verification.md
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked                          # core（合并、迁移、WebDAV 互操作 tiny_http 模拟、农历对照……）+ app（util / 纯逻辑）
cargo test -p mini-todo-core                             # 只测 core（不编译 GPUI）
# Windows 目标交叉检查（Linux 上开发时用；覆盖全部 cfg(windows) 代码；需 rustup target add x86_64-pc-windows-gnu + mingw gcc；只 check 不链接）
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
# mac 专属代码的本地交叉检查（Linux / Windows 上都能跑；只覆盖不依赖 GPUI 的 mac 源文件，需 rustup target add aarch64-apple-darwin）
cargo clippy --manifest-path scripts/mac-check/Cargo.toml --target aarch64-apple-darwin -- -D warnings
MINI_TODO_DATA_DIR=/tmp/mt cargo run -p mini-todo        # 隔离数据目录运行 debug 构建（Windows 上与安装版并存实测时必须带上）
cargo build --release -p mini-todo --locked              # Windows：需 Windows SDK 的 fxc.exe（可用 GPUI_FXC_PATH 指定）→ target/release/mini-todo.exe
# macOS（只能在 Mac 上）：universal → .app（ad-hoc 签名）→ dmg，见 pc-gpui/scripts/README.md「macOS：打包、e2e 与交叉检查」
cargo build --release -p mini-todo --locked --target aarch64-apple-darwin --target x86_64-apple-darwin
bash scripts/macos/bundle.sh target/aarch64-apple-darwin/release/mini-todo target/x86_64-apple-darwin/release/mini-todo dist
scripts/regress.py --before <改动前的输出> --out /tmp/regress  # Xvfb 视觉回归：场景截图 + 并排对照图（--only 'main-*' 只跑部分）
scripts/regress.py --smoke --no-build                    # 跨窗口端到端冒烟（按数据库断言）
scripts/shot.sh --name main-list --window "Mini Todo" /tmp/shots   # 单个场景截图 / xdotool 交互

# cloud（在 cloud/ 下）
cargo test --locked        # 含 mock_dav 场景测试（Apache / nginx 语义）
python3 -m unittest discover -s skill/minitodo -p 'test_*.py'

# 端到端（仓库根目录）：Linux 见 pc-gpui/scripts/e2e-linux/README.md；Windows 见 pc-gpui/scripts/e2e-windows/README.md；
# macOS 见 pc-gpui/scripts/e2e-macos/README.md（一般推 claude/** 分支由 CI pc-gpui-e2e-macos.yml 跑）
python3 pc-gpui/scripts/e2e-linux/run_e2e.py pc-gpui/target/debug/mini-todo cloud/target/debug/minitodo-cloud [用例前缀 ...]
python pc-gpui/scripts/e2e-windows/run_e2e.py --exe pc-gpui/target/debug/mini-todo.exe --seed pc-gpui/target/debug/examples/seed.exe --out <输出目录>
python3 pc-gpui/scripts/e2e-macos/run_e2e.py --app <mini-todo.app> --seed pc-gpui/target/debug/examples/seed --out <输出目录> --work <工作目录>
```

- Windows debug 构建的 exe 在运行时按**构建机**上 cargo registry 的绝对路径编译着色器，只能在构建它的那台机器上运行；分发一律用 release 构建。
  macOS 的着色器源码嵌在二进制里（`runtime_shaders`），debug 构建也能拷到别的 Mac 上跑；但系统通知等要 bundle id，得用 `bundle.sh` 包成 `.app` 再启动。
- Linux / Windows 上编不了整个 app 的 mac 目标（gpui-pre-apple 的 build.rs 按宿主机生成着色器源码，bundled SQLite 等 C 编译要 Apple SDK）：
  mac 的 GPUI 胶水只有 CI 的 macOS 作业能编译，所以 mac 改动要推分支跑 CI（`.trellis/spec/pc-gpui/verification.md` §7）。
- 安装包与便携 zip、dmg 只由 `release.yml` 构建（NSIS 脚本 `pc-gpui/scripts/windows-installer.nsi`、mac 打包脚本 `pc-gpui/scripts/macos/`，
  本地编译方法见 `.trellis/spec/pc-gpui/release.md`）。

debug 专用环境变量（**只在 debug 构建读取，release 一律忽略**；与正在运行的安装版并存做实测时必须带上，详见
`.trellis/spec/backend/desktop-fences.md` §3.6、`pc-gpui/crates/app/README.md` §5 与 `pc-gpui/scripts/README.md`）：

| 变量 | 作用 |
|---|---|
| `MINI_TODO_DATA_DIR` | 数据目录（库 / 图片 / 备份 / 图标缓存）、日志（`<dir>\logs`）都改到该目录；不启用单实例、不刷新开机自启注册表项；托盘提示"Mini Todo（开发版）" |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录改为该目录（代替 `%USERPROFILE%\MiniTodo收纳箱`） |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容非空时替代屏幕组合 ID，用于模拟显示器组合变化 |
| `MINI_TODO_DEBUG_OPEN` | 加载完数据后额外打开的窗口（`main`、`editor:<id>[:view\|:edit]`、`settings[:<页>]`、`fence:new` 等，格式见 `pc-gpui/scripts/README.md`） |
| `MINI_TODO_DEBUG_LATEST_VERSION` | 版本检查不联网，直接当作 GitHub 最新版本（如 `v2.6.0`） |
| `MINI_TODO_DEBUG_DIALOG_PATH` | 导出 / 导入的文件对话框不弹出，直接返回该路径 |
| `MINI_TODO_DEBUG_WINDOWS_UI` | `1` 时 Linux / macOS 上也显示 Windows 专有的设置项（截图对照用） |
| `MINI_TODO_DEBUG_FENCE_STATE` | `onedrive` / `drag-hover`：收纳箱直接显示 OneDrive 提示面板 / 拖入悬停框 |

### CI 与发布（`.github/workflows/`）

- `release.yml`：推 `v*` tag → 先建 **draft** release（tag 含 `-` 自动标 pre-release）→ Windows 作业构建 pc-gpui release 版并打 NSIS 安装包
  `mini-todo_<版本>_x64-setup.exe` + 便携包 `mini-todo_<版本>_x64-portable.zip`、macOS 作业（`macos-15` arm64，一次构建 aarch64 + x86_64 →
  `lipo` → `bundle.sh` 组装 `.app` 并整包 ad-hoc 签名 → `make-dmg.sh`）出 `mini-todo_<版本>_universal.dmg`、cloud 作业构建 `minitodo-cloud`
  （linux x86_64 / aarch64）→ publish 作业上传 `SHA256SUMS.txt` 并把 draft 转正。Windows / cloud 失败时不发布；**macOS 失败不挡发布**
  （mac 是预览平台：照常转正，只出 warning，之后 Re-run failed jobs 或 `workflow_dispatch` 补建 dmg）。`workflow_dispatch`（tag + platforms：
  `windows` / `macos` / `cloud-x86_64` / `cloud-aarch64`）给已有 release 补建产物。tag 必须等于 `v<pc-gpui 工作区版本>`，否则直接失败
- `cache-warm.yml`：在 main 上预热 release 用的 Rust 缓存（`release.yml` 只读缓存；`gpui-windows` / `gpui-macos` / `cloud` 三个作业与 release 逐字对齐）
- `pc-gpui-ci.yml`：Windows fmt / clippy / test + release 构建（上传 `mini-todo-gpui-windows-x64` 产物），Linux clippy / test，
  macOS clippy / test + universal release 构建并组装 `.app`（上传 `mini-todo-gpui-macos-universal` 产物）
- `pc-gpui-e2e-windows.yml`：Windows e2e（debug 套件 + release 冒烟 / 单实例）；手动触发，或推送到 `claude/**` 分支且改动 `pc-gpui/**` 时运行
- `pc-gpui-e2e-macos.yml`：macOS 编译门 + e2e（`checks` clippy / test、`smoke` debug `.app` 的 M 组、`release-smoke` release `.app` 不隔离的 P 组）；
  触发条件同 Windows e2e。mac 代码没有别的编译门，改了就推分支看它
- `cloud-ci.yml`：fmt / clippy / test + Skill 单测
- **版本号只有一处**：`pc-gpui/Cargo.toml` 的 `[workspace.package] version`（改完 `Cargo.lock` 同步），当前 `3.0.1-pre`。cloud 版本在
  `cloud/Cargo.toml`，与 tag 无关。发布步骤与安装器契约见 `.trellis/spec/pc-gpui/release.md`

## 注意事项

1. **目标平台**：Windows 10/11（x64）；macOS 11+（universal，预览版，未经真机验收）；Linux 可编译运行（CI 与 e2e 用），不提供 Linux 桌面包。
   嵌入桌面、桌面收纳箱等 Win32 特性在 macOS / Linux 上不可用
2. **运行环境**：开发只需 Rust stable；Windows release 构建另需 Windows SDK（`fxc.exe`）；macOS 构建需 Xcode Command Line Tools（clang + SDK，
   不需要 Metal 工具链）。用户运行不需要 WebView2
3. **图标使用**：仅使用 Element Plus Icons，禁止 emoji
4. **GPUI 约定**：改 `pc-gpui/crates/app` 前读 `.trellis/spec/pc-gpui/gpui-conventions.md`（后台执行、App 借用期间不同步调 Win32、
   拖动窗口、z 序与弹层、`img(PathBuf)`、theme token 唯一来源、`ui/` 控件复用、debug 开关只在 debug 构建生效；macOS 的 AppKit 主线程、
   窗口号、显示器本地坐标、红绿灯、字体别名、mac-check 见 §2.15）；
   `gpui-kit` 精确钉 `=0.7.3`，不要随手升级；改了界面跑 `scripts/regress.py` 并逐张看对照图
5. **Serde 命名**：Rust 模型使用 `#[serde(rename_all = "camelCase")]`（同步文档、导出文件、cloud API 都是驼峰）
6. **进程管理**：Windows 平台需要外部子进程时使用 `taskkill` 终止子进程树
7. **日志**：Rust 侧用 `log::info!` / `warn!` / `error!`（app `services::logging` 落盘），不要用 `println!` / `eprintln!`
   （release 是 Windows 子系统程序，没有控制台；macOS 从访达 / Dock 启动同样看不到标准输出）
8. **设置读写**：一律经 `db::settings_kv`（只在值变化时更新 `updated_at`，否则会干扰设置同步）
9. **安全边界**：本地图片只按文件名映射到本机 images 目录（`util/image_ref.rs`），Markdown 里存 `minitodo-image://<name>`；
   链接只放行 http(s) / mailto / 本地文件（`util/file_link.rs`，拒绝 UNC）；图片文件名经 `is_safe_image_name` 校验；
   收纳箱图标只从 `fence-icons/` 缓存加载
10. **时间**：见"时间格式约定"，只用 `db::time` 与 `util/datetime.rs` 解析与格式化
11. **兼容名字不要改**：数据目录、日志目录、单实例 / 开机自启 / AppUserModelID / 安装身份、mac 的 bundle id / `.app` 名 / socket /
    LaunchAgent 沿用旧版（见「兼容旧版沿用的名字」）；
    数据结构改动按 `.trellis/spec/pc-gpui/data-compat.md` 考虑 cloud 与仍在用旧版本的设备
12. **带 owner HWND 的收纳箱移动**（`fence::delete`、`fence::items::{move_in, restore, move_to}`）绝不在 UI 主线程调用（系统对话框会死锁）

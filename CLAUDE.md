# Mini-Todo 项目指南

本文档用于介绍 Mini-Todo 项目，帮助 AI 助手快速了解项目结构和开发规范。

## 项目简介

Mini-Todo 是一款 Windows / macOS 桌面待办事项管理应用，定位为简洁、聚焦的本地待办工具，支持多个待办清单（每个清单一个独立的桌面窗口）、子任务、
四象限、独立的日历窗口、重复提醒、系统通知与 WebDAV 云同步，另有收纳桌面图标的桌面收纳箱（Windows；macOS 预览）。**macOS 版是预览版**：只在 GitHub 的 macOS runner 上自动化验证过，
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
| 业务核心 | `mini-todo-core`（`crates/core`） | rusqlite 0.32 bundled（SQLite）、reqwest 0.12 blocking（WebDAV / 节假日）、windows 0.58；macOS 收纳箱文件层用 libc / objc2 / objc2-foundation / objc2-app-kit（与 app 同版本）；不依赖 GPUI / Tauri / tokio，函数全部阻塞 |
| 农历 | tyme4rs | lunar-javascript 同一作者的 Rust 库，结果与旧前端逐日比对（`tests/fixtures/lunar_fixture.json`） |
| 平台集成 | windows 0.62（与 gpui 同版本）、tray-icon、notify-rust、auto-launch、rfd、embed-resource；macOS：objc2 0.6 / objc2-app-kit 0.3（与 gpui-pre-macos 同版本） | 托盘 / 菜单栏图标、Toast、开机自启、带过滤器的文件对话框（rfd 仅 Windows）、exe 图标与版本信息；macOS 的 NSWindow / NSScreen 调用（含收纳箱桌面层、取色面板 `NSColorPanel`）、系统通知走 GPUI `show_system_notification`（notify-rust 不在 mac 上编译） |
| 安装包 | NSIS（`pc-gpui/scripts/windows-installer.nsi`）+ 便携 zip；macOS universal dmg（`pc-gpui/scripts/macos/`） | 由 `release.yml` 构建：Windows x64 + macOS universal（整包 ad-hoc 签名，不公证） |
| 测试 | cargo test、`scripts/regress.py`（Xvfb 截图对照 + 冒烟）、`scripts/e2e-linux/`、`scripts/e2e-windows/`、`scripts/e2e-macos/`、`scripts/mac-check/`（mac 专属代码的本地交叉检查） | 见 `.trellis/spec/pc-gpui/verification.md` |

## 项目结构

> 项目按平台拆分子目录。`pc-gpui/` 为 Windows / macOS 桌面端（GPUI），`cloud/` 为云端 API，未来可平行加入 `mobile/`、`web/`。

```
mini-todo/
├── docs/                                   # 共享文档（跨平台）
│   ├── 开发文档/                            # 开发相关文档
│   ├── images/                             # README 截图（pc-gpui/scripts/intro-video 生成）
│   └── video/                              # 功能介绍视频 mini-todo-intro.mp4（同上；README 内嵌的是它上传到 GitHub 后的附件链接）
├── pc-gpui/                                # PC 端（Cargo workspace；详见各 crate README）
│   ├── Cargo.toml                          # [workspace] members = crates/core、crates/app；[workspace.package] version 是
│   │                                       # 版本号唯一来源（当前 3.1.0-pre）；release thin LTO + strip
│   ├── Cargo.lock                          # 提交；CI 一律 --locked
│   ├── crates/
│   │   ├── core/                           # mini-todo-core：业务核心（不依赖 GPUI / Tauri / tokio，全部阻塞函数）
│   │   │   ├── README.md                   # 公开 API、线程约束、与旧 Tauri 命令的对照
│   │   │   ├── src/
│   │   │   │   ├── db/                     # 连接 / 迁移前备份（connection、backup）、迁移 v1~v31（migrations）、模型（models）、
│   │   │   │   │                           # settings 读写助手（settings_kv）、变更计数 / 墓碑 / 合并（sync_store）、时间（time）、路径（paths）
│   │   │   │   ├── services/               # webdav.rs（条件 GET/PUT、PROPFIND）、secret.rs（DPAPI）、fence_fs/（收纳箱文件层；
│   │   │   │   │                           # Windows 走 shell.rs / sta.rs，macOS 走 mac.rs）
│   │   │   │   ├── todo.rs / data.rs       # 待办 / 子任务 CRUD（含移到清单）、图片保存；导入导出、AppSettings 读写（唯一来源）
│   │   │   │   ├── lists.rs                # 待办清单 CRUD、名称校验、删除时待办移到默认清单、有效清单兜底（effective_list_id）
│   │   │   │   ├── window_prefs.rs         # 清单 / 日历窗口的设备本地属性（隐藏、日 / 夜、视图、展开全部、日历筛选）与按屏幕组合的布局
│   │   │   │   ├── ids.rs                  # 新清单 id（毫秒 × 1000 + 随机，与 cloud `api/ids.rs` 同一算法）
│   │   │   │   ├── sync/                   # sync::run(db, SyncMode) -> SyncReport（测试在 sync/tests.rs）
│   │   │   │   ├── settings.rs             # 单项设置、运行时偏好、屏幕组合 ID、screen_configs
│   │   │   │   ├── holiday.rs / reminder.rs  # 节假日（blocking）/ 提醒调度纯逻辑与通知窗口槽位、定位
│   │   │   │   ├── window_logic.rs         # 贴边隐藏状态机、needs_reattach、lacks_tool_window_style 等纯函数
│   │   │   │   ├── lunar.rs                # 农历（tyme4rs）
│   │   │   │   └── fence/                  # 收纳箱元数据 / 几何 / 屏幕组合布局（mod.rs）与内容对账、移入移回（items.rs）
│   │   │   ├── examples/seed.rs            # 把导出格式 4.0 / 4.1 的 JSON 导入隔离数据目录（截图 / e2e 用）
│   │   │   └── tests/fixtures/             # 农历对照数据（gen_lunar.cjs 用 lunar-javascript 生成）
│   │   └── app/                            # mini-todo：GPUI 界面、窗口、平台集成（lib mini_todo + bin mini-todo）
│   │       ├── README.md                   # 模块地图、约定、接口表、调试开关（维护者必读）
│   │       ├── build.rs                    # Windows：嵌 exe 图标（资源 ID 1）+ 版本信息；manifest 由 gpui 嵌，不要重复（其它目标直接 return）
│   │       ├── assets/                     # icon.ico；icon.icns（mac bundle 图标，取自 2.5.0）、tray-template.png（mac 菜单栏模板图标）；
│   │       │                               # icons/ep/*.svg（Element Plus）、icons/app/*.svg（自定义）；
│   │       │                               # changelog.md（应用内更新日志，include_str! 嵌入，写法见文件头）
│   │       └── src/
│   │           ├── lib.rs / main.rs        # run()：日志 → 单实例 → 数据库 → Application(QuitMode::Explicit) → 清单 / 日历窗口 → 后台服务
│   │           ├── theme.rs                # 颜色 / 字体 / 圆角 / 间距 / 阴影 token 的唯一来源（按窗口分节）
│   │           ├── state.rs                # AppState（Entity + Global）：跨窗口共享状态（待办、清单、窗口属性的内存副本）与待办 / 清单 / 设置写操作
│   │           ├── db.rs                   # Db 全局（Arc<Database>）、spawn_db（阻塞调用放后台）
│   │           ├── assets.rs               # EpIcon / AppIcon 嵌入表
│   │           ├── util/                   # datetime、image_ref（minitodo-image://）、file_link（链接白名单）、editor_shortcuts、version（版本比较）
│   │           ├── windows/                # WindowManager（mod.rs，含 DebugOpen）+ list_window/（清单窗口：标题栏、清单名改名 / 删除、
│   │           │                           # 列表、四象限、行右键 row_menu）、calendar_window/（日历窗口：网格、清单筛选 filter）、
│   │           │                           # block.rs（BlockKey、清单 / 日历窗口的自绘移动与缩放 BlockDrag）、drag.rs（与收纳箱共用的自绘拖动）、
│   │           │                           # snap.rs（吸附纯逻辑）、editor/、subtask、completed、changelog（更新日志）、notification、settings/、fence/、
│   │           │                           # gallery（控件样例，仅调试）
│   │           ├── ui/                     # Element Plus 风格共享控件（button、input、ep_*、message、message_box、markdown*…）
│   │           ├── services/               # 托盘 / 菜单栏图标、单实例、开机自启、同步编排、自动同步、提醒调度、系统通知、
│   │           │                           # 清单 / 日历窗口生命周期（block_windows）、窗口模式（window_mode/：按窗口的状态机、几何持久化）、
│   │           │                           # 吸附组（snap）、整个应用的提示（notice）、收纳箱、版本检查、更新日志（changelog）、日志；
│   │           │                           # mac_menu.rs（macOS 应用菜单与 Dock 唤起）
│   │           └── platform/               # GPUI 没有的窗口能力：windows.rs（Win32）、macos/（mod.rs GPUI 胶水 + ns.rs AppKit 原始调用，含收纳箱桌面层）、
│   │                                       # mac_geometry.rs（mac 坐标换算，全平台编译带单测）、fallback.rs（Linux）、embed.rs、subclass.rs…
│   └── scripts/
│       ├── README.md                       # regress.py / shot.sh / MINI_TODO_DEBUG_OPEN 说明，文末「macOS：打包、e2e 与交叉检查」
│       ├── regress.py                      # Xvfb 一键回归：场景截图 + 并排对照图 + 端到端冒烟
│       ├── shot.sh                         # 单个场景截图 / xdotool 交互
│       ├── sample-data.json                # 样例数据（导出格式 4.0）
│       ├── intro-video/                    # 功能介绍视频与 README 截图的生成（capture.py 真实应用截图 / 录屏 → video.html 时间轴 → render.cjs 逐帧渲染）
│       ├── windows-installer.nsi           # NSIS 安装器（安装身份沿用旧 Tauri 版，见 .trellis/spec/pc-gpui/release.md）
│       ├── macos/                          # bundle.sh（组装 mini-todo.app + 整包 ad-hoc 签名）、make-dmg.sh（拖放安装 dmg）、gen-tray-icon.py
│       ├── mac-check/                      # 不属于 workspace 的小 crate：把不依赖 GPUI 的 mac 源文件（含 core 的 fence_fs/mac.rs）#[path] 进来，在 Linux / Windows 上
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
- 创建、编辑、删除待办事项。新建入口：清单窗口右下角「+」FAB（锁定时不显示）、清单窗口有焦点时 Ctrl+N（macOS ⌘N，GPUI `secondary-n`，
  锁定与否都行，清单名改名中不响应）、双击清单窗口的空白处（列表：行以外的空白 / 空状态；四象限：双击某个象限里行以外的地方并预选该象限，
  颜色同在表单里点了它；网格外的空白不预选象限）——都建到本清单；双击日历的某天（见「视图模式」）；托盘「添加待办项」（默认清单）。
  双击在按下时判定（`click_count >= 2`）：待办行 / 日历的待办条 / FAB 拦下第二下，双击它们只是打开；拖拽排序中不触发。
  这些入口都走 `editor::open_new(cx, parent, NewTodoPreset { list_id, quadrant, start_time })`，预设算作表单的初始基线（没改过按 Esc 不弹确认）
- 描述字段支持 Markdown（存于 `description`）：编辑为"源码 + 预览"——平时显示渲染结果，单击切到源码编辑，失焦回到渲染；
  粘贴普通文字 / Markdown 源码原样插入；渲染支持 GFM 表格 / 任务清单 / 删除线 / 自动链接。编辑模式另有源码 / 预览分栏的放大编辑弹窗
  （`windows/editor/desc_dialog.rs`，「最大化」联动窗口最大化）
- Markdown 中的图片统一存为 `minitodo-image://<文件名>`（`crates/app/src/util/image_ref.rs`），渲染时映射成本机 images 目录下的文件
  （`img(PathBuf)`）；兼容旧数据里的 `http://asset.localhost/<绝对路径>` / `asset://localhost/...`（按文件名映射到本机，修复跨设备裂图，
  保存时换回规范形式）。粘贴 / 拖入图片 → 后台 `core::todo::save_image(bytes, ext)`：扩展名白名单 png/jpg/jpeg/webp/gif/bmp、单张 ≤20MB、
  文件名由后端生成，插入 `![<文件名>](minitodo-image://<name>)`（原图宽于 240 时插入 `![<文件名>|240](…)`，默认按 240 宽显示；
  `editor_theme::MD_IMAGE_INSERT_WIDTH`）；单击图片打开预览（编辑 / 查看模式都是，编辑模式下不切到源码）。
  Windows 上粘贴直接读系统剪贴板（`platform/clipboard.rs`：PNG > JPEG > GIF > WebP > 位图，也认 `image/png` 等 MIME 名格式），
  剪贴板位图在后台转成 PNG 再保存，复制的文件里有图片时只插入文件；没有可用图片也没有文字时提示。
  描述框与放大编辑弹窗的源码框共用 `ui/markdown_editor.rs` 的 `paste_images`（弹窗里粘贴的图片确定后才写回描述）
- 图片显示尺寸存在 alt 后缀里（Obsidian 写法）：`![image.png|300](…)` 宽 300、高按原图比例，`![image.png|300x200](…)` 宽高都指定；
  旧版本 / 其它设备忽略后缀按原始尺寸显示。编辑模式的渲染结果里悬停图片显示右边 / 底边 / 右下角手柄（`ui/markdown.rs` 的
  `resize_decorations`）：右边只改宽、底边只改高、右下角等比（按住 Shift 自由），松手后重写这张图片的源码，双击手柄恢复原始尺寸；
  放大编辑弹窗的预览同样可以拖（改写弹窗左边的源码草稿，确定才写回）；两处共用 `ui/markdown_editor.rs` 的 `ImageResizer`。
  引用式图片与表格 / 链接里的图片没有手柄
- 链接点击走协议白名单（`util/file_link.rs`）：http/https/mailto 交给系统程序，`file:///` 在资源管理器（macOS 访达）定位（拒绝 UNC），
  其余（javascript:/data: 等）一律拦截；只读排版普通单击即打开，编辑排版下 Ctrl/Cmd+单击
- 编辑类窗口支持 Esc 关闭（有未保存修改时确认）、Ctrl/Cmd+Enter 保存；保存有防重入
- 只读详情模式：从列表 / 四象限 / 日历 / 已完成视图点击待办默认进入（`editor::open(.., Some(id), EditorMode::View)`），
  左侧简化展示标题、Markdown 渲染描述、通知状态、优先级、所属清单，右侧子任务面板功能照常，[编辑] 按钮原地切换
- 支持一级子任务（含 Markdown 详情、图片上传；可从 .md/.txt 文件或文件夹批量导入）
- 四象限分类（重要紧急 / 重要不紧急 / 紧急不重要 / 不紧急不重要）
- 自定义颜色标识、完成状态标记、拖拽排序（列表 / 四象限）、置顶
- 开始/截止时间

### 待办清单（v31）
- 每个待办属于一个清单（`todos.list_id`），每个清单一个独立的清单窗口（见「窗口一览」）。默认清单 id 1「待办清单」：可改名、不可删除；
  归属未知（旧版本写的待办，`list_id` 为 NULL）或指向不存在的清单时显示在默认清单（读侧兜底 core `lists::effective_list_id`，记录不改写）
- 新建：托盘「新增待办清单」（自动取名"清单 / 清单 2…"，`state::next_list_name`）→ 光标所在显示器上找空位开 380×600 的窗口并直接进入改名；
  清单 id 用 core `ids::new_id`（毫秒 × 1000 + 随机，与 cloud 同一算法，两台设备离线各建一个不撞号）；新清单排最后（`sort_order`，第一版没有调整顺序的界面）
- 名称：去首尾空白、1~50 字、不区分大小写不重名（同步造成的重名容忍）；双击清单名就地改名（锁定中也可以；Enter / 失焦提交、Esc 放弃，
  空名 / 重名提示并恢复原名）；右键清单名「重命名 / 删除清单」（默认清单的删除置灰）。实现 `windows/list_window/name.rs`
- 删除：确认框 `确定删除清单「X」吗？N 项待办将移到「待办清单」。` → core `lists::delete`（单事务：其下待办移到默认清单并刷新 `updated_at` /
  `list_updated_at`、删清单、写清单墓碑、删它的窗口记录、从日历筛选移出）→ 关掉它的窗口
- 待办在清单间移动：编辑表单「四象限」下的「清单」下拉（新建预选打开方的清单：FAB / Ctrl+N / 双击空白 = 本清单，日历双击某天 = 筛选恰好
  只选了一个清单时是它、否则默认清单，托盘「添加待办项」= 默认清单；编辑已有待办时
  **只有用户真的改了选择**才在 `UpdateTodoRequest` 带 `list_id`，否则归属未知的待办会被写成 1）；列表 / 四象限的待办行右键「移动到 ▸」
  （子菜单列出其它清单，只有默认清单时置灰并提示"没有其它清单"；`windows/list_window/row_menu.rs`，`AppState::move_todo_to_list` 先改本地副本、
  失败回滚）。不做跨窗口拖动
- 清单窗口里新建（FAB / Ctrl+N / 双击空白）建到本清单；「已完成」只看本清单（恢复后回原清单）；日历显示所有清单的待办（可按清单筛选，见「视图模式」）
- 清单定义与删除随 WebDAV 同步（顶层 `todoLists` / `listTombstones`）；同步来的新清单（本机没有窗口记录）自动开窗，同步删除的清单自动关窗

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
- 列表 / 四象限按清单窗口各自切换（标题栏按钮，存在该窗口的 `window_prefs.view_mode`），只显示本清单（有效清单）的待办
- **列表视图**：按排序/优先级展示
- **四象限视图**：拖拽分类
- **日历窗口**（单例，设置 → 常规「展示日历」/ 托盘「显示日历」勾选，两者同一个设备本地状态 = 日历窗口的 `hidden`）：按日期展示所有清单的待办
  （含农历、节气、传统节日、法定节假日与调休；跨天条用待办自己的颜色，点击打开只读详情）。**双击某天**（待办条以外的地方，含灰色的上 / 下月格子；
  锁定与否都行）新建待办：只预填开始时间 = 那天 00:00（截止、提醒留空；日历按开始时间放格子，保存后就在那天），清单 = 筛选恰好只选了一个且它还在时
  建到它、否则默认清单（`calendar_window::logic::new_todo_list_id`）。标题栏：月份导航、今天、清单筛选、日 / 夜、锁、设置。
  **清单筛选**（漏斗按钮下拉多选，`windows/calendar_window/filter.rs`，存 `window_prefs.calendar_filter`：`NULL` = 全部，否则清单 id 数组）：
  「全部清单」包含以后新建的清单；从全部取消一个 = 其余清单；勾满或一个不剩回到全部；选了具体清单时新清单不勾；被删除的清单自动移出；
  兜底到默认清单的待办按默认清单参与筛选；不是全部时按钮显示主色。第一次显示且当前屏幕组合没有布局时贴默认清单窗口右侧、与它等高
  （`calendar_window::logic::first_rect`）

### 数据导入导出
- 导出版本：`4.1`（`pc-gpui/crates/core/src/data.rs` 的 `EXPORT_VERSION`）：顶层 `todoLists`（含默认清单），归属已知的待办带
  `listId` / `listUpdatedAt`（未知不写）；清单墓碑与窗口属性不导出
- 导出为 ZIP 压缩包（内含 `data.json`），默认文件名 `mini-todo-backup-YYYY-MM-DD.zip`
- 导入兼容 v3.0、v4.0、v4.1（按 `PK` 魔数区分 ZIP / JSON，直接 JSON 也支持）
  - v3.0 是历史导出格式（含已移除的 AI Agent 字段），反序列化时通过 serde 的未知字段忽略与 `#[serde(default)]` 静默跳过
  - 4.1：清单同样按"恢复备份"语义（保留 id、`updated_at` 刷新；备份里没有的清单写清单墓碑并删除；默认清单始终保留、只取备份里的名字）；
    备份里缺 `listId` 的待办恢复成未知；已知归属的 `listUpdatedAt` 刷新为导入时刻
  - 3.0 / 4.0（没有 `todoLists`）：现有清单不动，待办写成**具体的**默认清单 1
  - 旧版本导入 4.1：`todoLists` 被忽略、`listId` 被丢弃（`import_json` 不校验 version）
- 导入是"恢复备份"语义：导入前自动 `VACUUM INTO` 备份到 `backups/`；保留原 id；导入后消失的记录写墓碑，
  被导入记录的 `updated_at` 刷新为当前时间（保证恢复结果在同步中胜出）

### WebDAV 云同步
- 智能同步 `sync::run(db, SyncMode::Smart { full })`：先 GET 远端并逐条合并（LWW + 墓碑），本地有变化才条件 PUT；标题栏同步按钮与设置页
  「立即同步」传 `full: true`（无条件 GET），自动同步传 `false`（每第 10 轮也做一次无条件 GET）。设置页另有带二次确认的
  「用云端覆盖本地」（`SyncMode::ForcePull`）、「用本地覆盖云端」（`SyncMode::ForcePush`）。不再有整库二选一的冲突对话框
- 同步范围：todos / subtasks / 墓碑 / 待办清单与清单墓碑 / 10 个应用设置项（按 `settingsUpdatedAt` 做 LWW，窗口位置与尺寸不从远端应用；
  其中 `is_fixed` / `text_theme` / `view_mode` / `show_calendar` 3.1 起只作迁移 v31 的初始值，不再驱动界面、也不再写入）/ 图片。
  清单 / 日历窗口的属性与布局是设备本地的，不同步
- 自动同步可选（按间隔轮询，失败段只提示一次）；同步在后台线程执行，不阻塞界面；同一时刻只允许一个同步（`"同步正在进行中"`）
- gzip 压缩传输；**数据本身不加密**（传输安全取决于 https，WebDAV 服务商可读取内容）；
  WebDAV 密码在 Windows 上用 DPAPI 加密保存，macOS / Linux 上明文存在本机数据库里（与 2.5.0 一致；mac 设置页有提示，钥匙串要开发者签名，暂不做），
  界面只拿到 `has_password`，http:// 地址会提示明文风险

### 窗口特殊功能
- 下面的模式**按窗口**作用于每个清单窗口与日历窗口（`services/window_mode/`：每个窗口一份 `ModeState` 状态机，`BlockKey::{List(id), Calendar}`）；
  锁定、日 / 夜按窗口各自保存，嵌入桌面 / 贴边自动隐藏 / 唤起置顶 / 界面底色 / 背景透明度是全局设置，作用于每个（锁定的）窗口
- **普通模式**：可拖动、可缩放——与收纳箱一样自绘（标题栏 `windows::block::BlockDrag`、窗口内 8 个缩放手柄，建窗 `resizable: false`，
  所以没有 Aero Snap、标题栏双击不最大化），与其它清单 / 日历窗口和收纳箱互相吸附（见「桌面收纳箱」的吸附对齐）；进任务栏 / Alt+Tab
  （同一 AppUserModelID，Windows 合并成一个任务栏图标）。窗口是透明的，界面自己画圆角底色：浅色白底，深色用设置里的「界面底色」+
  「背景透明度」（透出桌面）
- **固定模式**（每个窗口标题栏的锁按钮，`window_mode::toggle_fixed(cx, key)`；托盘不再有「固定模式」）：
  - 固定在用户指定位置：不可拖动、不可缩放（自绘拖动与手柄不挂；`platform::subclass` 的 `FLAG_LOCK_GEOMETRY` 拦截命中测试）
  - 清单窗口不显示右下角「+」FAB（含嵌入桌面），新建用 Ctrl+N / ⌘N 或双击空白处（见「待办管理」）；空状态文案随之只提示后两者
  - 不在任务栏、不参与 Alt+Tab（`WS_EX_TOOLWINDOW`）；被 Win+D / Win+M 最小化后由 200ms 轮询立即还原
  - 可选贴边自动隐藏、唤起时置顶（设置 → 常规）；经典固定时没有圆角与描边
- **固定模式时嵌入桌面**（设置 → 常规「固定模式时，嵌入桌面中」，仅 Windows 显示，Win11 24H2+ 保证 Win+D 免疫）：
  - 不是第三种模式，而是固定模式的一个全局开关：开启后进入固定模式即走嵌入路径，关闭则是经典固定；当前已固定时切换开关立即生效
    （`window_mode` 集中观察 `AppState.fixed_embed_desktop` 变化 → `window_mode::on_embed_preference_changed`，对所有锁定的窗口生效）
  - 窗口仍是顶层窗口：owner=Progman（`GWLP_HWNDPARENT`）+ 永远置底（`SetWindowSubclass` 处理 `WM_WINDOWPOSCHANGING`，`FLAG_BOTTOM`）+
    去掉 `WS_MINIMIZEBOX` + `WS_EX_TOOLWINDOW`，位于桌面图标之上、所有应用窗口之下；Win+D / Win+M / 任务栏"显示桌面"时随 Progman 一起被抬起
  - 透明背景 / 圆角 / DPI 与普通模式零差异（不做 SetParent 子窗口化）；不可拖拽、不可缩放；贴边隐藏与唤起置顶不生效
  - 200ms 轮询检测 Explorer 重启与样式被改（`platform::embed::needs_reattach`），自动重挂
  - 持久化：settings `fixed_embed_desktop`（v27）；每个窗口是否固定按屏幕组合记在 `window_layouts.is_fixed`（v31；
    `settings.is_fixed` / `screen_configs.is_fixed` 只是旧键，前者只作迁移初始值）
  - 实现：`crates/app/src/services/window_mode/`（`mod.rs` 按窗口的运行时 / `apply_mode` / 200ms 轮询，`machine.rs` 纯状态机 `ModeState` /
    `TargetMode`，`store.rs` 几何与锁定的持久化）、`platform/embed.rs`、`platform/subclass.rs`；
    纯逻辑在 core `window_logic.rs`；契约见 `.trellis/spec/backend/window-modes.md`
  - 非 Windows：开关不显示；`TargetMode::for_state` 只在 Windows 上选嵌入，同步来的开关不会让固定失败
- **macOS 上的固定模式**（2.5.0 mac 版的简化语义，同一套按窗口的 `window_mode` 状态机与 200ms 轮询）：`setMovable:NO`（建窗就不带 `Resizable`，
  自绘拖动与手柄锁定时不挂）、被最小化（⌘M）立即还原、托盘 / 贴边唤起临时置顶（`NSFloatingWindowLevel`）、贴边隐藏用 `NSEvent.mouseLocation` 全局光标
  （往有菜单栏的屏幕顶边藏会被 AppKit 挡回，状态回滚为显示；贴边隐藏中被 Dock / ⌘Tab 激活时按托盘左键唤起所有窗口，`window_mode::on_activated`）；
  没有"不进任务栏 / ⌘Tab"（程序坞图标与 ⌘Tab 是整个应用的），没有嵌入桌面。契约见 `.trellis/spec/backend/window-modes.md`「Scenario: macOS」
- **多屏幕位置记忆**：每个清单 / 日历窗口按屏幕组合记住位置、尺寸（客户区物理像素）与是否固定（`window_layouts`，v31；升级时默认清单窗口继承原主窗口
  在各组合下的记录，原来开着日历的按左 40% / 右 60% 切开）；`screen_configs` 只由默认清单窗口保存时登记组合（设置 → 屏幕配置按它列出，每行汇总这个组合
  保存了几个清单窗口、日历、几个收纳箱的位置与锁定数；删除一个组合连带删除它下面所有清单 / 日历 / 收纳箱的布局，当前组合不能删）。
  托盘「重置位置」（`window_mode::reset_all`）：所有清单窗口（含隐藏的、没开过窗的）与日历窗口从主显示器左上（以前主窗口的重置位置）起向右下错开
  32 逻辑像素层叠（清单 380×600、日历 596×600；超出显示器从头再来），全部解锁、取消隐藏（日历只在「显示日历」开着时出现）；先把全部布局写进当前组合
  （`store::save_all`）再开没开过的窗，所以它们直接出现在重置后的位置。
  macOS 的物理像素口径见 `platform/mac_geometry.rs`（全局点坐标 × 所有屏的最大缩放；屏幕组合 ID 按各屏自身缩放，与 2.5.0 一致）

### 桌面收纳箱（Windows；macOS 预览）
- 托盘 / 菜单栏图标「新增收纳箱」在光标所在显示器工作区的右上角找空位新建（默认 4 列 × 3 行、标题"收纳箱 / 收纳箱 2…"）。每个收纳箱是一个
  无系统标题栏的透明 GPUI 窗口（`WindowKind::Fence`，登记 key `fence-{id}`，`crates/app/src/windows/fence/`）
- **Windows：始终嵌入桌面层**：与清单 / 日历窗口的嵌入共用 `platform::embed`（owner=Progman + 永远置底 + `WS_EX_TOOLWINDOW` + 无 `WS_MINIMIZEBOX`），
  Win+D 后可见、不进任务栏 / Alt+Tab、不盖住应用窗口；建窗参数一次给齐（`resizable: false`、`minimizable: false`、先隐藏建好、
  定位并嵌入后再显示）；200ms 守护在 Explorer 重启后自动重挂（`services/fences.rs`）
- **macOS：桌面图标层之上的不激活面板**（不走 `platform::embed`）：建成 GPUI `WindowKind::PopUp`（NSPanel + `NonactivatingPanel`），先隐藏建好 →
  `setFrame:` 摆到库里的几何 → `Hwnd::make_desktop_layer`（`ns.rs`：层级 `CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`，
  collectionBehavior 整体换成 `CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenNone` = `0x251`、`canHide = NO`、
  不进「窗口」菜单、无显示动画）→ `orderFrontRegardless`；日志自报实际属性（`[fence] 收纳箱窗口 #… 已放到桌面层：level=… behavior=0x…`，
  e2e 据此核对）。单击、拖动、拖入、打开项目都不激活应用；⌘H / 别的应用「隐藏其他」不隐藏它、不计入 Dock 唤起的"有可见窗口"；⌘M / ⌘W 对它无效；
  收纳箱窗口**绝不调** `set_topmost`（会把层级改回 0 / 3）。双击标题改名是唯一激活应用的操作（面板单击不一定成为 key 窗口）：先 `cx.activate(true)`
  再 `window.activate_window()`，150 / 500 / 1000ms 复查、不是 key 就重新拿回，开始后 1.2 秒内的失去 key / 输入框失焦不当作提交，宽限期结束时
  仍不是 key 才提交；改名期间菜单栏切到 mini-todo（`windows/fence/mod.rs` 的 `begin_mac_rename_focus`）。
  没有 Explorer 那样的重挂守护（层级是全局排序，未在真机验证 Finder 重启后的表现）
- **真收纳**：外部拖入（`ui::external_drop`）的文件 / 快捷方式 / 文件夹从桌面移到收纳根目录下的 `<收纳箱名>` 文件夹（Windows
  `%USERPROFILE%\MiniTodo收纳箱\`、macOS `~/MiniTodo收纳箱/`），重名加 ` (2)` 绝不覆盖（包 `X.app` → `X (2).app`）；
  Windows 经 `IFileOperation`（公共桌面项弹 UAC），macOS 逐项 `renamex_np(RENAME_EXCL)`（内核原子判重，撞名换下一个后缀名重试；跨卷才交给
  `NSFileManager`，同样不覆盖；**不用** `std::fs::rename`，它会静默覆盖）。**文件夹是内容的 source of truth**（`notify` 监听，
  资源管理器 / 访达里增删随之刷新），数据库只存元数据与项目的排序 / 原始路径 / 隐藏标记。不允许整体收纳用户桌面 / 公共桌面文件夹本身
- 双击打开（Windows `ShellExecuteExW`；macOS `NSWorkspace`，先拦下失效的替身 / 符号链接）、自绘右键菜单（打开 / 打开所在位置（macOS「在访达中显示」）/
  隐藏或取消隐藏 / 移回桌面 / 移动到…）、箱内拖动排序、Ctrl+滚轮图标大小（macOS 另认 ⌘+滚轮）、
  双击标题改名（收纳文件夹同步改名，失败则保留旧文件夹名并提示）、折叠（悬停 / 拖文件经过时临时展开）、
  「固定」= 锁定位置和大小（仍可拖入、折叠、改名）、每个收纳箱独立的背景色 / 透明度（「自定义颜色」：Windows `ChooseColorW`，macOS 系统取色面板
  `NSColorPanel`，非模态、每 150ms 读一次，面板关掉时生效，打开时会把应用切到前台）
- **隐藏图标**（v30，只影响收纳箱的显示，不改文件属性、不移动文件）：以 `.` 开头的项目自动隐藏（固定开启，不能单独取消隐藏；
  拖入时提示一次）；右键「隐藏」/「取消隐藏」写 `fence_items.hidden`（`fence::items::set_hidden`）；「更多」→「显示隐藏的图标」
  （每个收纳箱独立，`fences.show_hidden`）打开时隐藏项半透明显示，关闭时不显示，全部隐藏时空状态显示"N 个项目已隐藏"。
  对账保留隐藏标记（含只改大小写的重命名），跨箱移动不携带；带 Windows 隐藏 / 系统属性（macOS `UF_HIDDEN`，即 `chflags hidden`）的文件
  维持原样永不显示、不收纳；删除收纳箱时隐藏项一并移回
- **系统元数据文件**（所有平台）：`desktop.ini`、`Thumbs.db`、`.DS_Store`、`Icon\r`、`.localized` 不显示、不收纳、删除收纳箱时不算剩余
  （文件夹只剩它们时连同它们一起删掉）
- 移动 / 缩放：两者是同一套自绘拖动（不进系统拖动循环，否则中途改不了位置；标题栏也不用 `drag_region()`，否则双击改名收不到）：标题栏按下
  只记光标（屏幕物理坐标），移动超过 3px 才读客户区起点、开始跟手（GPUI `on_drag_move`，光标出了窗口也收得到；3px 以内松手的双击照常改名，
  不用 `on_click`），缩放用窗口内 8 个手柄；每帧最多摆一次窗
  （移动只改位置，Windows `SetWindowPos`、macOS `setFrame:`）。**吸附对齐**（纯逻辑 `windows/snap.rs`）：收纳箱、清单窗口、日历窗口是同一个**吸附组**，
  双向吸附、规则相同（清单 / 日历窗口的自绘拖动在 `windows/block.rs`，与收纳箱共用 `windows/drag.rs`）。靠近其它显示中的组员（含锁定的，折叠的收纳箱按标题栏；
  最小化的、收纳箱"全部隐藏"中、清单 / 日历窗口隐藏或贴边藏起时不算；只看光标所在显示器）±10 逻辑像素时，移动吸同名边（left↔left、right↔right、
  top↔top、bottom↔bottom），缩放吸被拖那条边的同名边或等宽 / 等高（对边不动、不小于最小尺寸）；横纵向独立取最近（同距离时收纳箱优先）；
  被对齐的**目标**在对应边上画 2px `#409EFF` 高亮线（跨窗口状态在吸附组服务 `services::snap`），松手 / 脱离即消失；**按住 Alt（macOS Option）临时不吸附**，没有设置项。只有用户拖动 / 缩放结束（松手后直接落库一次，存吸附后的几何）与托盘重置才
  `fence::commit_geometry` / `reset_positions` 落库，系统挪窗不落库。几何一律是客户区的物理像素（macOS 是全局点 × 统一系数，见「多屏幕位置记忆」）
- 删除收纳箱：项目全部移回原位置（原目录不存在则回桌面）、空文件夹删除后才删记录并关窗口；有项目移不回去时删除中止、
  整个收纳箱保留并提示原因
- 按屏幕组合记忆位置（`fence_layouts`，ID 与 `screen_configs.config_id` 同格式，都由 core `settings::screen_config_id` 计算），约 2 秒检测一次
  屏幕组合变化；设置里删除屏幕配置时连带删除对应布局
- 托盘 / 菜单栏图标「显示/隐藏所有收纳箱」「重置收纳箱位置」作为兜底入口；首次收纳"由云同步管理的桌面项目"时提示一次（Windows：桌面被 OneDrive
  重定向；macOS：项目移动前由 iCloud 云盘管理（`NSURLIsUbiquitousItemKey`），移出就会从 iCloud 删除；两者共用 `fence_onedrive_notice_shown`）
- macOS 文件层的隐私权限（TCC）：拖放即授权被拖的项目、在桌面里新建项目不需要同意，但**列举** `~/Desktop` / `~/Documents` / `~/Downloads`
  会弹「文件与文件夹」授权框（ad-hoc 签名每次升级都会再弹），所以 mac 上规划移动时不列举收纳根目录以外的目标目录，撞名交给 `RENAME_EXCL`。
  授权框会阻塞调用线程（移动本来就在后台线程）；`bundle.sh` 的 Info.plist 带 `NSDesktopFolderUsageDescription` 等用途说明（不是身份键）
- 关键实现：core `fence/`、`services/fence_fs/`（macOS 后端 `mac.rs`）；app `services/fences.rs`、`windows/fence/`、`platform/embed.rs`（Windows）、
  `platform/macos/ns.rs`（`make_desktop_layer`、取色面板）；契约与易错点见 `.trellis/spec/backend/desktop-fences.md`
- 已知限制：Win10 / Win11 ≤23H2 上 Win+D 可能盖住收纳箱；macOS 版未经真机验收（见「已知偏差与限制」6）

### 系统功能
- 托盘（Windows）/ 菜单栏图标（macOS，单色模板图标 `assets/tray-template.png`，随菜单栏深浅色着色）：左键单击唤起所有清单窗口（含隐藏的、
  还没开过窗的）与日历窗口（「显示日历」开着时），300ms 去抖（`window_mode::bring_all_to_front`，每个窗口按自己的锁定 / 贴边状态走唤起置顶规则）；
  右键菜单：添加待办项（建到默认清单）、新增待办清单、显示日历（勾选）、新增收纳箱、显示/隐藏所有收纳箱、重置收纳箱位置、重置位置、打开设置、
  开机自启动（勾选）、退出（`services/tray.rs`）
- 关闭清单 / 日历窗口（Alt+F4、macOS ⌘W）= 隐藏（`block_windows::request_close`：保存几何、记住 `hidden`、不注销；关日历 = 关掉「显示日历」），
  不删清单、不退出；隐藏状态重启后保持，默认清单窗口例外（启动时总是显示）。Windows 上所有窗口都关了也不退出（与 macOS 一致），
  只有托盘「退出」/ ⌘Q 退出；Linux（无托盘）关窗是真的关，最后一个窗口关掉即退出
- 整个应用的提示（自动同步失败、加载待办失败、导入成功、托盘新建清单失败）经 `services::notice` 显示在看得见的清单 / 日历窗口里
  （当前激活的 → 发起操作的 → 最近激活过的 → 默认清单）；都看不见时先存着（最多 3 条），下次有清单 / 日历窗口被激活时补显示，同步成功时撤回未显示的同步失败
- 单实例：再次启动时把参数转交给已运行的实例并唤起所有清单窗口（同托盘左键；开机自启拉起的不唤起）；Windows 命名互斥量 + `WM_COPYDATA`，macOS Unix socket
- 开机自启（Windows 注册表 Run 项 / macOS LaunchAgent）、版本更新检查（GitHub `releases/latest`，默认清单窗口标题栏版本号旁红点；不会提示预发布版）
- 运行日志落盘（2MB 轮转，保留 1 份旧日志）

### 更新日志
- 内容是随程序嵌入的 `pc-gpui/crates/app/assets/changelog.md`（`include_str!`）：每个版本一个二级标题 `## <版本号>（YYYY-MM-DD）`，下面是面向普通用户的
  简短中文要点（Markdown，可有 `###` 小标题；不写下载、安装放行、云端组件；禁止 emoji）；`## 未发布` 放已合入还没发版的改动，应用里不显示，
  发版改版本号时把它改成版本号。解析容错（未知标题 / 空段落 / 重复版本 / 代码块里的 `##` 都忽略），按版本号从新到旧显示。
  单测 `current_version_has_a_changelog_section`：工作区版本在更新日志里没有段落时 CI 失败
- **升级后第一次运行时弹出**（`services::changelog::check_on_startup`，`lib.rs` 加载完数据后）：本机记录 settings `last_run_version`（设备本地，见「当前同步覆盖范围」），
  判定是纯函数 `decide`：记录 = 当前版本什么都不做；没有记录时按 core `Database::schema_version_at_open`（本次启动迁移前的库版本）区分——0（新装）只写记录，
  `< 31`（3.0.x 及更早）显示所有 `<=` 当前版本的段落，`>= 31` 按上次是 3.1.0-pre（显示 `(3.1.0-pre, 当前]`）；记录较旧显示 `(记录, 当前]`；降级或区间里没有段落只写记录。
  弹出（窗口打开成功）或判定不弹时把记录更新为当前版本；以默认清单窗口为打开方（模态、不设 owner，同托盘「打开设置」）
- 设置 → 关于「更新日志」随时查看全部版本（`<=` 当前，以设置窗口为打开方的模态子窗口）
- 窗口 `windows/changelog.rs`：520×600 可缩放，自绘标题栏 + 可滚动的 Markdown 只读渲染（每个版本一个标题：版本号 + 日期）+ 底部「知道了」，Esc 关闭；
  自动弹出时开头一句"已更新到 x"
- 测试环境：debug 构建 + `MINI_TODO_DATA_DIR` 时**不自动判定、不写记录**（e2e / regress / 介绍视频常先 seed 再启动 = 已有数据库、没有记录），
  `MINI_TODO_DEBUG_CHANGELOG=auto` 恢复；`MINI_TODO_DEBUG_OPEN=changelog` 直接打开窗口（全部版本）。release 一律按正常规则
  （CI 的 release 冒烟 / P 组都在一次性虚拟机的全新数据目录上跑 = 新装，不弹）

### macOS（预览）
- 能力对齐 2.5.0 mac 版（再加上 3.0 的全部界面与功能），另有桌面收纳箱（2.5.0 没有，见「桌面收纳箱」）与 3.1 的多清单 / 日历窗口（没有 mac 专属代码，
  走共用路径，没在 Mac 上跑过）；清单 / 日历窗口没有嵌入桌面；**未经真机验收**，
  只靠 CI 的 macOS runner 自动化验证（`pc-gpui-e2e-macos.yml`）与用户反馈
- 身份与 2.5.0 mac 版相同（bundle id、`.app` 名、数据目录、日志、单实例 socket、LaunchAgent，见「兼容旧版沿用的名字」）；
  Dock 图标照常显示（不设 `LSUIElement`，GPUI 启动时会把激活策略重设为 Regular，要隐藏只能用 `set_activation_policy`）；
  Info.plist 另带 `NS{Desktop,Documents,Downloads}FolderUsageDescription`（收纳箱万一触发「文件与文件夹」授权框时的中文说明，不是身份键）
- 窗口：所有窗口仍是自绘标题栏，系统红绿灯建窗后隐藏（`Hwnd::prepare_after_open`，改 style mask 后再隐藏一次），
  `WindowSpec` 设 `app_owns_titlebar_drag`；运行时摆窗 / 显示隐藏 / 置顶 / 几何锁定 / 收纳箱桌面层 / 取色面板在 `platform/macos/ns.rs`
  （objc2 调 NSWindow / NSScreen / NSColorPanel）
- 应用菜单（`services/mac_menu.rs`，GPUI 不会自动建）：Mini Todo（关于 / 设置… ⌘, / 服务 / 隐藏 ⌘H / 隐藏其他 ⌥⌘H / 全部显示 / 退出 ⌘Q）、
  编辑（撤销 / 重做 / 剪切 / 拷贝 / 粘贴 / 全选，系统文件面板里的 ⌘C / ⌘V 靠它）、窗口（最小化 ⌘M / 关闭窗口 ⌘W；收纳箱是活动窗口时两者都不做）
- 关窗口（含清单 / 日历窗口、⌘W，后者只是 `orderOut:` 隐藏）不退出进程，菜单栏图标常驻；点 Dock 图标 / 在访达里再次打开时同托盘左键
  唤起所有清单窗口（`on_reopen` → `window_mode::bring_all_to_front`；GPUI 只在没有可见窗口时回调，所以有清单 / 日历窗口显示着时点 Dock 只激活应用、
  不带出隐藏的清单窗口；收纳箱是 NSPanel，按 Apple 文档不算可见窗口，所以有收纳箱时也能唤起——e2e MF07）；
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
    `fence-icons/`（收纳箱图标 PNG 缓存，启动对账后清理过期项）、`ui-state.json`（3.0.x 的全局「展开全部子任务」；
    3.1 起首次启动迁进默认清单窗口的 `window_prefs.expand_all` 后删掉这个键，`state::migrate_legacy_ui_state`）
  - 这个目录同时是 Windows 安装包的默认安装目录（与旧 Tauri 版一致）；安装器卸载只删自己装的程序文件，绝不递归删除该目录
  - 日志：`%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`；macOS `~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`
    （没有 `logs` 子目录）（app `services::logging`，2MB 轮转；目录名 / 位置都为兼容旧版沿用）
  - 收纳箱文件：`%USERPROFILE%\MiniTodo收纳箱\<收纳箱文件夹>\`；macOS `~/MiniTodo收纳箱/<收纳箱文件夹>/`（3.0.3-pre 起，2.5.0 mac 版没有）
    （用户可见的真实文件夹，有收纳箱后才创建；不在数据目录里，卸载 / 删库都不会丢文件）
- **迁移版本**：当前 v1~v31，通过 `pc-gpui/crates/core/src/db/migrations.rs` 的迁移表管理（`LATEST_VERSION`）；有待执行迁移时先备份，
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
  - v31：多待办清单（可重跑）。`todo_lists(id, name, sort_order, created_at, updated_at)` 接 `local_seq` 触发器（参与同步；id 不用
    AUTOINCREMENT、名称不加唯一约束）；默认清单 id 1「待办清单」、`2000-01-01 00:00:00`（两台设备各自迁移出的 id 1 不互相抢）；
    `todos.list_id INTEGER`（**可空**，NULL = 归属未知、不等于默认清单，存量待办为 NULL；不加外键）与 `todos.list_updated_at TEXT`
    （可空，`list_id` 自己的版本，只有显式设置清单时才写）；`list_tombstones(entity_id, deleted_at)`（不计数、永不过期）；作废
    `webdav_remote_etag`（升级后第一次同步无条件 GET）；设备本地的 `window_prefs`（每窗口 `window_key` = `list:<id>` / `calendar`：
    `hidden` / `dark` / `view_mode` / `expand_all` / `calendar_filter`）与 `window_layouts`（每 (窗口, 屏幕组合)：`x / y / width / height /
    is_fixed`，随 `window_prefs` 级联删除），不接 `local_seq`；种子：`list:1` 继承原主窗口（`text_theme` / `view_mode` / 各 `screen_configs`），
    `calendar` 隐藏 = `!show_calendar`，开着日历时把各组合的矩形按左 40% / 右 60% 切给 `list:1` / `calendar`。旧版本打开 v31 库只记警告照常运行
    （按显式列 INSERT 的待办是"未知"）

#### 主要数据表

| 表名 | 说明 |
|------|------|
| `todos` | 待办事项（含重复提醒字段、所属清单 `list_id` 与其版本 `list_updated_at`） |
| `subtasks` | 子任务（标题 + Markdown 内容 + 完成态） |
| `todo_lists` | 待办清单（名称、顺序；参与同步，默认清单 id 1 不可删除） |
| `settings` | 应用设置（键值对，带 `updated_at`，只经 `db::settings_kv` 写入） |
| `screen_configs` | 屏幕配置（屏幕组合的登记，由默认清单窗口保存时写入） |
| `window_prefs` | 清单 / 日历窗口的设备本地属性（隐藏、日 / 夜、视图、展开全部、日历筛选） |
| `window_layouts` | 清单 / 日历窗口按屏幕组合的位置、尺寸与锁定 `(window_key, screen_config_id)` |
| `sync_meta` | 同步计数器（`local_seq` / `synced_seq`），由触发器维护 |
| `tombstones` | 待办 / 子任务删除墓碑 `(entity_type, entity_id, deleted_at)`，保留 30 天 |
| `list_tombstones` | 清单删除墓碑 `(entity_id, deleted_at)`，永不过期（同步文档里是独立顶层键 `listTombstones`） |
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

- **导出版本号**：当前 `4.1`（`pc-gpui/crates/core/src/data.rs` → `EXPORT_VERSION`；兼作同步文档的 `version`，旧 PC 与 cloud 都不校验它）
- **关键文件**（均在 `pc-gpui/crates/core/src/` 下）：
  - 模型定义：`db/models.rs` → `Todo`、`SubTask`、`TodoList`、`ListTombstone`、`ExportData`、`AppSettings`
  - 清单：`lists.rs`（CRUD、`effective_list_id`）；窗口属性（设备本地）：`window_prefs.rs`；新清单 id：`ids.rs`
  - 设置读写（唯一来源）：`data.rs` → `read_app_settings` / `write_app_settings`；
    参与同步的键 `SYNCED_SETTING_KEYS` 与设置版本 `settings_version` 在 `db/settings_kv.rs`
  - 导入导出：`data.rs` → `export_json` / `import_json` / `export_to_file` / `import_from_file`（仅手动导入导出使用）
  - WebDAV 同步：`sync/mod.rs` → `SyncData`、`run`、`run_sync_blocking`、`build_sync_doc`；
    记录读写/合并/墓碑：`db/sync_store.rs`（`merge_remote`、`load_todos_with_subtasks`；清单 `merge_remote_lists`、`force_pull_replace_lists`、
    `force_push_prepare_lists`；归属 `ListAssignment`）
  - cloud 写入校验：`cloud/src/model.rs`（新增记录字段必须同步加入 `TODO_FIELDS` / `SUBTASK_FIELDS`，清单字段 `LIST_FIELDS`，否则 AI 写入会被 400）
  - 界面侧没有单独的前端类型：`crates/app` 直接用 core 的 Rust 模型

#### sync-data.json.gz 结构

```jsonc
{
  "version": "4.1", "deviceId": "…", "updatedAt": "…",        // 元信息
  "todos": [ /* Todo，嵌套 subtasks；归属已知时带 "listId": <清单 id> 与 "listUpdatedAt": "YYYY-MM-DD HH:MM:SS"
               （缺失 / null = 未知，不等于 1；未知时两个键都不写） */ ],
  "todoLists": [ { "id": 1, "name": "待办清单", "sortOrder": 0, "createdAt": "…", "updatedAt": "…" } ],  // 全部未删除清单（含 id 1）
  "listTombstones": [ { "entityId": 1731000000000123, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ],          // 清单墓碑，永不过期
  "settings": { /* AppSettings */ }, "settingsUpdatedAt": "YYYY-MM-DD HH:MM:SS",
  "images": ["<安全文件名>"],
  "tombstones": [ { "entityType": "todo" | "subtask", "entityId": 123, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ]
  // 其它顶层键：所有写入方（PC / cloud）必须原样保留。清单墓碑不进 tombstones：旧版本丢弃未知 entityType 后按本地表重建它，
  // 会把清单墓碑抹掉；todoLists / listTombstones 作为新顶层键，旧版本经 extra 原样带回
}
```

#### 合并规则（PC `sync_store::merge_remote` 与 cloud `sync/merge.rs` 一致）

1. 记录级 LWW：比较规范化后的 `updatedAt`，**平局保留本地**
2. 墓碑：同 `(entityType, entityId)` 且 `deletedAt >= updatedAt` 时删除/压制记录；todo 墓碑连带子任务；
   删除后又被编辑（`updatedAt > deletedAt`）的记录保留
3. 只在一侧存在的记录保留（并集），**不做"缺席即删除"**（cloud 仅对没有 `tombstones` 键的旧版文档、且自身不 dirty 时保留旧行为）
4. 墓碑取并集、同键取较大 `deletedAt`，待办 / 子任务墓碑保留 30 天，清单墓碑永不过期
5. 清单与待办同构（LWW 平局保留本地、`deletedAt >= updatedAt` 压制、并集、墓碑取较大值；PC `merge_remote_lists`、cloud `merge::merge_lists`）；
   远端没有 `todoLists` 键 = 没有清单信息，本地清单一条不删；默认清单不可删除，`entityId = 1` 的清单墓碑一律忽略；收到清单墓碑不改写待办
   （读侧兜底进默认清单），被删清单的窗口记录同一事务里清理
6. 待办的归属（`listId`）按 `(listId, listUpdatedAt)` 做**字段级** LWW，与记录的 `updatedAt` 无关：记录级结果照旧，结果记录存在时归属取所有可见版本
   （本地合并前的——在应用墓碑之前读出——与远端的，哪怕被墓碑压制）里最新的：未知最旧，已知没有版本次旧，有版本的按时间，一样新取较大的 `listId`；
   采纳不改 `updatedAt`、不额外上传；合并后远端那一版的归属比本机旧时补传一次；畸形记录不参与
7. 设置：远端 `settingsUpdatedAt` 比本地同步键的 `max(updated_at)` 新才应用；`windowPosition` / `windowSize` 不从远端应用；
   从未同步过的新设备直接采用远端设置
8. 远端记录反序列化失败只跳过并计数（`recordsSkipped`），不当作删除；远端文档整体解析失败直接报错、不上传

#### 当前同步覆盖范围

| 数据 | 是否同步 | 说明 |
|------|---------|------|
| `todos`（全字段） | 是 | 含重复提醒字段、`listId` / `listUpdatedAt`（归属未知时都不写） |
| `subtasks`（全字段） | 是 | 标题 + Markdown 内容 + 完成态 |
| `tombstones` | 是 | 删除双向传播，保留 30 天 |
| `todo_lists` | 是 | 清单名称 / 顺序，顶层 `todoLists`；默认清单 id 1 不可删除 |
| `list_tombstones` | 是 | 独立顶层键 `listTombstones`（不进 `tombstones`，旧版本经 extra 原样带回），永不过期 |
| `settings`（部分） | 是 | `SYNCED_SETTING_KEYS` 的 10 项（含 `is_fixed` / `fixed_embed_desktop`），按 `settingsUpdatedAt` LWW；不含 WebDAV 配置；窗口位置/尺寸只随导出备份。`is_fixed` / `text_theme` / `view_mode` / `show_calendar` 3.1 起只作迁移初始值（保留在同步键里，旧设备照常读写） |
| `images`（文件） | 是 | 先传图片再传 sync-data；一次 PROPFIND 列远端清单；文件名经 `is_safe_image_name` 校验 |
| `screen_configs` | 否 | 设备特定的屏幕配置 |
| `window_prefs` / `window_layouts` | 否 | 清单 / 日历窗口的设备本地属性与布局（同 `screen_configs`）；也不进导入导出 |
| `fences` / `fence_items` / `fence_layouts` | 否 | 桌面收纳箱：路径与屏幕坐标是设备相关数据，性质同 `screen_configs`；也不进导入导出 |
| settings `fence_onedrive_notice_shown` | 否 | 设备本地提示标记（Windows 的 OneDrive 与 macOS 的 iCloud 提示共用），有意不进 `AppSettings` / `SYNCED_SETTING_KEYS` |
| settings `last_run_version` | 否 | 本机上次运行的程序版本（更新日志弹窗用；没有迁移，键不存在 = 没记录过），有意不进 `AppSettings` / `SYNCED_SETTING_KEYS`、不导出 |
| `sync_meta` / `migrations` | 否 | 本机状态 / 结构性表 |

#### 向后兼容

- 旧 v3.0 备份内的 `agent_configs` / `workflow_steps` / `task_dependencies` / `prompt_templates` /
  `agent_executions` 等字段，以及 todo/subtask 上的 agent/调度/工作流字段，在 v4.0 反序列化时
  通过 serde 的"未知字段忽略"机制自动跳过，不会报错。
- `SyncData` 新字段一律 `#[serde(default)]`；未知顶层键经 `#[serde(flatten)] extra` 原样带回上传。
- 其它设备上可能还在跑旧版本（≤ 2.5.0 的 Tauri 版或更早的 3.x）：新增字段 / 设置键在它们那里会被丢掉，见 `.trellis/spec/pc-gpui/data-compat.md`。
  多清单：旧设备上传的待办全部没有 `listId`，新设备把缺失当作**未知**、沿用各端看到的最新归属（不会被挤回默认清单）；`todoLists` /
  `listTombstones` 经旧版本的 extra 原样带回，清单不丢、已删清单不复活；**≤ 2.4.x 的写入方**（没有 extra / tombstones）会抹掉清单信息，不支持混用。

#### 维护检查清单

当新增数据库迁移时，请检查（路径均在 `pc-gpui/crates/core/src/` 下，另注明的除外）：
1. 新增的 **settings 键值** 是否已加入 `AppSettings`（含 `#[serde(default)]`）、`data.rs` 的 `read_app_settings` / `write_app_settings`；
   需要跨设备同步的再加入 `SYNCED_SETTING_KEYS`（设备相关的不要加）；需要界面的再改 `crates/app`（`state.rs`、`windows/settings/`）。
   例外：纯设备本地的提示 / 状态键（如 v29 的 `fence_onedrive_notice_shown`、更新日志的 `last_run_version`）有意不进 `AppSettings`，
   登记在 `.trellis/spec/backend/sync-protocol.md` §3.6
2. 所有 settings 写入走 `db::settings_kv::set_setting`（禁止 `INSERT OR REPLACE`，否则每次写都会刷新同步版本）
3. 新增的 **数据表** 是否需要同步：需要则加触发器维护 `local_seq`、在 `db/sync_store.rs` 中实现读写/合并/墓碑，
   并纳入 `sync/mod.rs` 的 `build_sync_doc`（显式顶层键，不放进 `extra`；需要删除语义的用独立墓碑键）与 `doc_differs`、
   cloud 的 `sync/doc.rs`（顶层键、`RECORD_KEYS`）、`sync/merge.rs`（参照 `todo_lists`）；设备本地的表登记到 `sync-protocol.md` §3.6
4. 新增的 **记录字段**：`db/models.rs`（`#[serde(default)]`）、`db/sync_store.rs` 的行读写、`data.rs` 的导入导出、
   cloud `src/model.rs` 允许字段与类型；旧版本会丢掉不认识的字段，"缺失"要区分成**未知**而不是默认值（参照 `listId` / `listUpdatedAt`）
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
  → 主题 →（macOS）应用菜单 → UiLayer / Db / WindowManager / AppState → 整个应用的提示（notice）→ 单实例唤起监听
  → 清单 / 日历窗口（`block_windows::start`：迁移 ui-state → 同步读清单、窗口属性、运行时偏好、屏幕组合 → 默认清单窗口（总是显示）
    →「显示日历」开着时开日历 → 其它没隐藏的清单窗口（隐藏的启动时不建）；按当前屏幕组合恢复几何与锁定 → 观察清单列表、5 秒变更轮询、检查更新）
  → 托盘 / 菜单栏图标 → 自动同步定时器 → 提醒调度 → 恢复收纳箱窗口 → 开机自启自愈 → 加载设置与待办
  → 更新日志（`services::changelog::check_on_startup`：升级后第一次运行时以默认清单窗口为打开方弹出）→（debug）MINI_TODO_DEBUG_OPEN
```

### 跨窗口状态与通信

旧版的 Tauri invoke / emit 在 GPUI 版里都变成同进程的直接调用：

| 旧版（Tauri / WebView） | GPUI 版 |
|---|---|
| Pinia `todoStore` / `appStore` | `state::AppState`（`Entity` + Global），各窗口 `cx.observe`；写操作经 `AppState` 方法在后台执行。清单列表与窗口属性（`window_prefs`）的内存副本也在这里，清单列表每替换一次 `lists_revision` 加一 |
| `invoke` 命令 / `spawn_blocking` | `db::spawn_db(cx, \|db\| core::…)` 放后台，`WeakEntity::update` 回前台 |
| `sync-completed` | `services::sync::run_sync` → `AppState::apply_sync_report`（本地数据变了 → 重新拉待办与清单；`settings_applied` → 重读设置）；清单变了 → `lists_revision` → `block_windows::reconcile` 开 / 关清单窗口 |
| （无） | 整个应用的提示：`services::notice`（挑看得见的清单 / 日历窗口显示，都看不见时暂存） |
| （无） | 吸附组：`services::snap`（收纳箱、清单、日历窗口登记；吸附场景与高亮线的跨窗口状态） |
| `data-imported` / `app-settings-changed` / `todo-font-changed` | 直接改 `AppState`，所有窗口随之刷新 |
| `subtask-memory-ready` / `-init` / `-save` | 子任务窗口直接读写新建待办窗口的 `Entity<DraftSubtasks>`（`subtask::open_draft`） |
| `fence-items-changed` | `fence::items::FenceRefresh`：目录监听回调（`fence-watch` 线程）经 channel 回前台 → 对应收纳箱窗口 |
| 托盘事件 | tray-icon 回调（Windows 窗口过程 / macOS 主线程的 AppKit 事件）只把事件投进 channel → 前台执行动作（`services::tray`）；左键 / 单实例 / Dock 唤起都是 `window_mode::bring_all_to_front` |
| 独立 WebView 窗口（label） | GPUI 窗口：`windows::WindowManager` 统一登记 / 模态子窗口 / 遮罩 / owner / 关闭后刷新 / 居中 |
| tao 窗口能力 + `commands/window.rs` Win32 | `platform/`（`windows.rs` Win32、`macos/` AppKit、`fallback.rs` Linux、`embed.rs` 嵌入桌面、`subclass.rs` 置底与几何锁定） |
| Tauri 默认 App 菜单（mac） | `services::mac_menu`（`set_menus` + 全局 action + 快捷键；`on_reopen` 唤起所有清单窗口） |

### 窗口一览（`windows::WindowManager`，`WindowKind`）

| 窗口 | 入口 | 说明 |
|---|---|---|
| 清单窗口 | `services::block_windows`（`show_list` / `create_list` / `reconcile`；`windows/list_window/`） | 每个清单一个，`WindowKind::List`、key `list:<清单 id>`（= `window_prefs` 的 key）；380×600，最小 320×400，透明，自绘移动与缩放（不可系统缩放、没有 Aero Snap / 双击最大化）；系统标题：默认清单窗口固定 `Mini Todo`（e2e 按它找窗口），其它 = 清单名；关闭 = 隐藏 |
| 日历窗口 | `block_windows::set_calendar_visible`（`windows/calendar_window/`） | 单例，`WindowKind::Calendar`、key `calendar`；最小 420×480，第一次显示贴默认清单窗口右侧、等高；系统标题 `日历`；关闭 = 隐藏（关掉「显示日历」） |
| 待办详情 / 编辑 | `editor::open(cx, parent, todo_id, EditorMode::{View, Edit})`；清单 / 日历窗口新建用 `editor::open_new(cx, parent, NewTodoPreset)`（FAB / Ctrl+N / 双击空白 / 双击象限 / 双击日历某天） | 1080×600；`todo_id = None` 为新建（「清单」下拉预选默认清单 / 预设的清单；预设还可带象限、开始时间，算作初始基线） |
| 子任务 | `subtask::open` / `subtask::open_draft` | 800×750，查看 / 编辑 |
| 已完成 | `completed::open(cx, parent, list_id)` | 460×550，只列本清单（有效清单）的已完成；搜索、恢复、删除 |
| 更新日志 | `changelog::open(cx, parent, sections, upgraded)` / `changelog::open_all(cx, parent)` | `WindowKind::Changelog`，520×600、可缩放（最小 420×400）；升级后自动弹出（打开方默认清单窗口、不设 owner，只显示升级区间）/ 设置 → 关于（打开方设置窗口，全部版本）；Esc / 「知道了」关闭 |
| 设置 | `settings::open(cx, parent, SettingsTab::{General, Appearance, Data, Screen, About})` | 680×560；常规 / 外观 / 数据与同步 / 屏幕配置 / 关于 |
| 应用内通知 | `notification::open` | 320×120，透明、`WindowKind::PopUp`、不抢焦点 |
| 收纳箱 | `services::fences`（`fence::open` 等转发） | 每个收纳箱一个窗口，key `fence-{id}`；Windows 始终嵌入桌面层，macOS 是桌面图标层之上的不激活面板（`PopUp`）；不可关闭（删除走 `close_deleted`） |
| 控件样例 | `gallery::open` | 仅调试（`MINI_TODO_DEBUG_OPEN=gallery`） |

打开方是清单 / 日历窗口、已完成、编辑窗口、设置窗口（关于页打开的更新日志）时，子窗口是模态的（同一时刻一个，打开方显示遮罩，Windows 上以打开方为 owner；macOS 没有 owner 关系）；
打开方是任一清单 / 日历窗口时，子窗口关闭后自动 `reload_todos`，设置窗口关闭后再重读设置并重启自动同步。托盘「添加待办项」/「打开设置」、
应用菜单「设置…」以默认清单窗口为打开方；设置里的「深色主题」切的是打开它的那个窗口。

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
  └── 点击待办 → editor::open(.., Some(id), EditorMode::View)（新建：FAB / Ctrl+N / 双击空白 → editor::open_new(.., 本清单 [+ 象限])；
       日历双击某天 → open_new(.., 筛选定的清单 + 开始时间)；托盘「添加待办项」→ None = 默认清单，直接编辑）
       └── 编辑窗口：只读详情（标题 + MD 渲染 + 通知状态 + 优先级 + 所属清单）⇄ [编辑] 原地切换表单（含「清单」下拉）
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

### 清单 / 日历窗口刷新

- 整个应用一个 5s 轮询（`services::block_windows`）：`AppState::refresh_if_changed`（读 `todo::change_seq` = `sync_meta.local_seq`，变化时才
  `fetch_todos`，待办与清单一起拉）；子窗口关闭、同步完成也走同一个单飞 `fetch_todos`（拉取期间有本地写操作则丢弃并补拉）；任一清单 / 日历窗口
  开着模态子窗口时后台刷新跳过（`WindowManager::any_block_has_modal_child`）。清单列表变了（`lists_revision`）→ `block_windows::reconcile`：
  已删除清单的窗口关掉（默认清单永不关）；没开窗的清单里，本机没有窗口记录（同步 / cloud 新建）或记录是显示中的开窗——同步来的新清单在主显示器
  工作区从右上角找不与显示中的吸附组窗口重叠的空位（同新建收纳箱的算法），380×600、未锁定、浅色、列表视图

### WebDAV 同步流程

```
services::sync::run_sync(cx, SyncMode)（后台线程；AppState.syncing 期间标题栏图标旋转）
  └── core sync::run(db, mode)（单同步互斥，"同步正在进行中"）
        ├── GET sync-data（有基准 ETag 时 If-None-Match；本地有未上传变更 / 手动同步 / 每第 10 轮时无条件 GET）
        │     ├── 304 → 远端未变
        │     ├── 200 → 解析（失败直接报错）→ 单事务逐条合并（LWW + 墓碑 + 设置 LWW；清单 todoLists / listTombstones 同一事务；
        │     │         待办归属按 listUpdatedAt 字段级合并）→ 下载缺失图片
        │     └── 404 → 首次上传
        ├── 需要上传？（local_seq ≠ synced_seq / 设置更新 / 远端缺本地记录、墓碑、清单或清单墓碑 / 远端那一版的归属比本机旧）
        │     ├── 先上传缺失图片（一次 PROPFIND Depth:1 列清单）
        │     └── 条件 PUT：已知 ETag 时 If-Match（去 W/），从不发 If-Unmodified-Since
        │           412 → 等 1.1s → 重新 GET → 合并 → PUT（最多 4 次）
        └── 成功：基准取 PUT 响应 ETag → HEAD → PROPFIND；记录 synced_seq；返回 SyncReport
  └── AppState::apply_sync_report（本地数据变了（含清单）→ 重新拉待办与清单 → lists_revision → 开 / 关清单窗口；settings_applied → 重读设置与运行时偏好）
SyncMode::ForcePull  让本地等于远端（删除本地独有记录 / 清单，不写墓碑；默认清单保留；远端缺 listId 时保留本地已知的归属）
SyncMode::ForcePush  让远端等于本地（为远端独有记录 / 清单写墓碑后上传；先采纳远端已知的归属）
```

WebDAV 服务端差异（实测，见 `.trellis/spec/backend/sync-protocol.md`）：Apache mod_dav 的 `If-Unmodified-Since`
对未改动文件也会 412、写入后 1 秒内 ETag 为弱；nginx/Caddy 忽略 PUT 前置条件、PROPFIND 不返回 getetag、
`If-Modified-Since` 按整秒比较。所以两端只用 `If-Match` / `If-None-Match`，并且写前总是先 GET 合并。

### 桌面收纳箱流程

```
托盘 / 菜单栏图标「新增收纳箱」→ services::fences::create_fence
  └── 后台 fence::create（先建文件夹 → INSERT fences → 记当前屏幕组合布局）
       → open_fence_window（先隐藏建窗 → 按客户区定位 → Windows：TOOLWINDOW + 嵌入桌面层 / macOS：PopUp 面板 make_desktop_layer
         → 显示；失败 rollback_create）
       → ensure_watcher；200ms 守护（Windows：Explorer 重启重挂；每 10 tick 检测屏幕组合）
拖入 → 后台 fence::items::move_in（先登记 begin_moving →（macOS）判定 iCloud 项目 → IFileOperation / renamex_np(RENAME_EXCL)
       → upsert fence_items，搬运 / 记录原始路径）→ FenceRefresh
目录变化 → notify（300ms 去抖）→ fence-watch 线程里数据库锁内列举对账 → FenceRefresh 经 channel → 对应收纳箱窗口
删除 → 后台 fence::delete → restore_all → 文件夹已清空才删记录 + close_deleted 关窗，否则整个收纳箱保留
```

### 已知偏差与限制

1. **Markdown 编辑**：旧版 Milkdown 所见即所得无法在 GPUI 中 1:1 还原，编辑模式改为"Markdown 源码 + 渲染预览"（放大编辑弹窗是源码 / 预览分栏）；
   只读渲染、图片粘贴 / 拖入上传、`minitodo-image://` 引用、链接白名单规则不变。
2. **中文换行**：GPUI 的换行器不做避头尾，全角标点（`，。、；：！？）`）可能落到行首，多行中文的折行位置与浏览器可能差一个字。
3. GPUI 没有文字阴影、多色标渐变、位图变换等，个别装饰用近似实现（见 spec `gpui-conventions.md` §2.13）。
4. **Linux 仅用于 CI、截图回归与 e2e**：没有托盘 / 单实例 / 开机自启 / 嵌入桌面 / 收纳箱真实文件操作，最后一个窗口关闭即退出，设置里的字体下拉为空。
   Windows 专有能力（嵌入桌面、收纳箱的 Shell 实现、DPAPI、Toast、托盘、DirectComposition 透明）在 Linux 上只做交叉 clippy，靠 CI 的 Windows e2e 与用户实机验收；
   mac 专属代码见 6。
5. Win10 / Win11 ≤23H2 上 Win+D 可能盖住嵌入桌面的清单 / 日历窗口与收纳箱（显示桌面抬起的是 `WorkerW` 而不是 Progman）。
6. **macOS 是预览版，未经真机验收**：维护者没有 Mac，mac 代码只能在 macOS runner 上编译、跑 e2e（runner 屏幕 1x、单显示器、半虚拟化 GPU），
   Retina、多显示器 / 混合缩放、Spaces / 全屏、阴影与圆角观感、Intel 机型（x86_64 切片只在 arm64 runner 上交叉编译、`lipo` 合并，
   CI 没有运行过它）都靠用户反馈。另外：
   - 只有 ad-hoc 签名、不公证：首次打开要放行（`xattr -cr` 或「隐私与安全性 → 仍要打开」），每次升级都要重新放行
   - 系统通知拿不到投递结果（授权被拒也视为已投递），`cargo run` 的裸二进制收不到通知；runner 上测不了通知横幅与授权
   - 清单 / 日历窗口没有嵌入桌面，固定模式没有"不进 Dock / ⌘Tab"；固定模式在 Spaces / 全屏下的行为（`collectionBehavior`）没有处理
   - 多清单 / 日历窗口（3.1）没有新增 mac 专属代码，共用路径没在 Mac 上跑过：⌘W 隐藏（`orderOut:`）、全部隐藏后点 Dock 带出隐藏的清单窗口、
     新清单窗口进入改名时能否拿到 key、隐藏状态下 `setFrame:` 后再显示（「重置位置」）、清单 / 日历窗口的自绘拖动、缩放与吸附、
     ⌘N 与双击空白 / 日历某天新建（应用菜单里没有「新建待办」项，⌘N 只靠清单窗口的 key context）；
     有清单 / 日历窗口显示着时点 Dock 只激活应用、不带出隐藏的清单窗口（GPUI 只在没有可见窗口时回调 `on_reopen`），只能用菜单栏图标左键
   - 收纳箱（3.0.3-pre 起）只在 runner 上跑过 e2e（MF 组：桌面层层级、监听、真实拖入、移回、删除、⌘H、Dock 唤起、改名、拖动落库、菜单栏三项、
     重启恢复），全部在临时目录里、没碰过真实 `~/Desktop`。改名（MF08）在 runner 上用粘贴 + 回车验证（系统输入流的 ⌘V / 回车；runner 上逐字注入的
     Unicode 按键本来就进不了 GPUI 输入框，普通窗口也一样）。没验证过：真实桌面的隐私授权（TCC）流程、iCloud「桌面与文稿」、跨卷移动、
     Spaces / 台前调度 / 调度中心 /「显示桌面」/「点按墙纸显示桌面」下的表现、Retina / 多显示器、收纳箱改名时的中文输入法、
     打开 / 在访达中显示（e2e 不测，会拉起别的应用）、收纳箱的吸附对齐与高亮、按住 Option 不吸附（MF09 只有一个收纳箱；吸附只在 Windows e2e
     F13–F15 与单测里验证）
   - 收纳箱的小偏差：访达拖入时光标带绿色"+"（GPUI 对外部拖入一律回报"拷贝"），实际执行的是移动；「自定义颜色」的取色面板会把应用切到前台
   - 混合缩放（Retina 笔记本 + 1x 外接屏）时窗口位置的物理像素口径与 2.5.0 不同，2.5.0 记下的位置可能恢复不到（设备本地数据，只丢一次）
   - 文件对话框没有扩展名过滤器；WebDAV 密码明文；最低 macOS 11（2.5.0 写的是 10.13）
7. **清单 / 日历窗口是自绘移动与缩放**（建窗不可缩放）：没有 Aero Snap、标题栏双击不最大化、Win+↑ 不最大化；缩放热区是可见边缘内侧 5px（角 10px）
   的手柄，有模态子窗口时遮罩挡住手柄、不能缩放；Linux 上拖标题栏不移动窗口（GPUI 在 Linux 上不能程序化移动窗口）。第一版不做跨窗口拖动待办、
   跨清单汇总 / 四象限、清单颜色、清单排序界面、逐个唤起清单的子菜单；旧版本设备（≤ 3.0.x）不显示清单，它新建的待办在各端都显示在默认清单

## cloud/ 子项目（云端 API）

`cloud/` 是一个独立的 Rust crate（**不在** `pc-gpui/` 的 Cargo workspace 里），部署到 VPS 上为
AI / Claude Code Skill 提供 mini-todo 数据的 HTTP REST 访问能力。部署、配置、Skill 见 `cloud/README.md`。
与 PC 的契约（和上文"时间格式约定""合并规则"是同一套）：

- **共用同一个 WebDAV `sync-data.json.gz` 通道**。云端是 WebDAV 客户端，不依赖 PC 在线
- **WebDAV 是 source of truth**，云端 SQLite 是缓存；未推送的本地改动（`meta.dirty`）推送成功前不会被拉取覆盖
- **时间**：`YYYY-MM-DD HH:MM:SS`，按 `config.toml` 的 `timezone`（IANA，须与 PC 一致）每次重新换算墙钟（DST 安全）
- **写冲突**：写前先 GET 合并，条件 PUT 只用 `If-Match`（去 `W/`）；412 等待后重新 GET → 合并 → PUT
- **字段契约**：SQLite 是 KV-style（`todos` / `todo_lists` 等表都是 `(id, data_json, updated_at)`），但 API 写入只接受 PC 模型里的字段
  （`src/model.rs`，以 `pc-gpui/crates/core/src/db/models.rs` 为准），未知字段 / 类型错误 → 400；合并远端时未知字段原样保留（可能是更新版 PC 加的新字段）
- **墓碑**：DELETE 写 `tombstones`，随 sync-data 双向传播，保留 30 天
- **清单**：`todo_lists` + `list_tombstones(entity_id, deleted_at)`（清单记录的未知字段原样保留）；同步文档顶层 `todoLists` / `listTombstones`，
  合并规则与 todo 同构、与 PC `sync_store` 一致（远端没有 `todoLists` 键时本地清单一条都不删）；默认清单 id 1 在建库与每次合并时按固定初始值补建
  （"待办清单"、`sortOrder` 0、`2000-01-01 00:00:00`），不可删除，`entityId = 1` 的清单墓碑一律忽略；**清单墓碑永不过期**；REST 写入的 `listId`
  必须指向未删除的清单；`DELETE /lists/:id` 单事务把其下待办移到默认清单（刷新 `updatedAt` 与 `listUpdatedAt`）并写清单墓碑；`GET /todos` 的
  `listId=` / `list=` 按有效清单过滤；清单不分配短码
- **待办的清单归属**：`listId` 缺失或 `null` = 未知，只在显示 / 过滤时算默认清单，cloud 从不替它补值；`listUpdatedAt` 由服务端维护（客户端传入的忽略，
  只在 `POST`、`PATCH` 改了 `listId`、删除清单移进默认清单时刷新），合并时 `(listId, listUpdatedAt)` 做字段级 LWW（规则同上文「合并规则」第 6 条，
  采纳不改 `updatedAt`、不标脏）；合并后远端那一版的归属比本地旧 → 补传一次；上传时未知不写两个键
- **升级**：从不认识清单的旧版 cloud 升级后第一次启动作废一次基准 ETag（meta 标记 `lists_base_reset_v1`），保证第一次 GET 是全量并把清单合并进来
- **短码**：`todo_seq` 表给每个 todo 分配 `C{seq}`，API 路径与 CLI 都可用；只存在于 cloud，不进同步文档

```
cloud/
├── src/main.rs               # 启动：config → SQLite → 首次 pull（spawn_blocking）→ worker → axum；停机限时补推
├── src/config.rs             # config.toml；缺必填字段报错退出
├── src/time.rs               # 规范时间格式、宽松解析、按 IANA 时区换算
├── src/model.rs              # 记录字段契约（todo / subtask / 清单）：写入校验、别名映射、派生 priority、清单名称规则、归属比较
├── src/db/                   # todos / subtasks / todo_lists / settings / meta / tombstones / list_tombstones / todo_seq；normalize.rs 修存量数据
├── src/sync/webdav.rs        # GET（If-None-Match）/ PUT（If-Match）/ HEAD / PROPFIND / MKCOL
├── src/sync/doc.rs           # sync-data 文档读写（todoLists / listTombstones；保留未知顶层键）
├── src/sync/merge.rs         # 与 PC 一致的记录 LWW + 墓碑 + 清单 + 归属 + 设置合并
├── src/sync/{pull,push}.rs   # 拉取合并；写前合并 + 条件 PUT + 412 重试；图片推送
├── src/sync/worker.rs        # pull 轮询（每 10 轮一次无条件全量）、push 去抖 1.5s（最长 10s）、失败指数退避
├── src/sync/images.rs        # 图片镜像（缺什么下什么，失败退避重试）
├── src/sync/mock_dav.rs      # 测试用进程内 WebDAV，可按 Apache / nginx 实测语义模拟
├── src/api/                  # axum：Bearer 鉴权、X-Sync-Status 头、5xx 不外泄内部细节；lists.rs 是清单 CRUD，ids.rs 是新 id 算法
├── deploy/                   # minitodo-cloud.service（systemd 沙箱加固）、Caddyfile.example
└── skill/minitodo/           # Claude Code Skill：SKILL.md + minitodo.py（含单测，lists 子命令与 --list）+ install.{sh,ps1}
```

REST API（全部需要 `Authorization: Bearer <api_key>`，`:id` 可用 `C{seq}` 短码）：

| Method | Path | 说明 |
|---|---|---|
| GET | `/health` | `{status, sync, lastPullAt}` |
| GET | `/todos?...` | 列表；query: `completed` / `priority` / `quadrant` / `dueDateBefore` / `dueDateAfter` / `startDate` / `q` / `listId` / `list`（按有效清单）/ `sort=±field` / `limit` / `offset` / `withSubtasks` |
| GET | `/todos/:id?withSubtasks=true` | 详情；默认嵌套 subtasks |
| POST | `/todos` | 创建；必填 `title`，其余字段按契约校验 |
| PATCH | `/todos/:id` | merge 更新；未提及字段保留 |
| DELETE | `/todos/:id` | 删除（连带 subtasks，写墓碑） |
| POST | `/todos/:id/subtasks` | 创建子任务；必填 `title` |
| PATCH | `/subtasks/:id` | merge 更新子任务（`parentId` 不可改） |
| DELETE | `/subtasks/:id` | 删除子任务 |
| GET | `/lists` | 全部清单 `[{id, name, sortOrder, createdAt, updatedAt}]`，按 `(sortOrder, id)`；默认清单 id 1 总在 |
| GET | `/lists/:id` | 单个清单；`:id` 是数字 id 或 URL 编码的清单名（先按 id、再按名称不区分大小写匹配；重名 409、找不到 404） |
| POST | `/lists` | 新建；必填 `name`，可选 `sortOrder`（缺省排最后）→ 201；名称空 / 超过 50 个字 400，重名 409 |
| PATCH | `/lists/:id` | 改 `name` / `sortOrder`；内容没变时不刷新 `updatedAt` |
| DELETE | `/lists/:id` | 其下待办移到默认清单 + 写清单墓碑；200 `{deleted, name, movedTodos, movedTo}`；默认清单 400 |
| GET | `/images/:name` | 返回图片 bytes |
| POST | `/images` | multipart 上传，`file` 字段；返回 `{name}` |
| POST | `/sync` | 手动 pull + push；全部成功 200、部分失败 207，返回 `{pull, push, pullError?, pushError?}` |
| POST | `/sync/pull` | 仅从 WebDAV 拉取 |
| POST | `/sync/push` | 仅推送到 WebDAV |

todo 可写字段以 `src/model.rs` 的 `TODO_FIELDS` 为准（`title` / `description` / `color` / `quadrant` / `notifyAt` /
`startTime` / `endTime` / `notifyBefore` / `notified` / `completed` / `sortOrder` / 重复提醒 5 个字段 / `listId`）。`listId`：整数 ≥ 1，
必须是未删除的清单，否则 400 `清单不存在`；`POST` 不带时为 1；`PATCH` 不带时不变（缺 `listId` 的旧记录是未知、不补值）。别名：
`dueDate`→`endTime`（仅日期补 23:59:00）、`priority`（high/medium/low）→`color`（#EF4444 / #F59E0B / #10B981）、
`notes`→`description`；`quadrant` 接受 1-4 或 `urgent_important` / `important_not_urgent` 等别名。`id` / `createdAt` /
`updatedAt` / `listUpdatedAt` 等服务端字段写入时忽略。响应附派生字段 `priority`（自定义颜色为 null）与 `seq`，派生字段不入库。

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
python3 scripts/intro-video/capture.py --no-build && node scripts/intro-video/render.cjs   # 重新生成介绍视频（README 截图加 --still，见其 README）

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
| `MINI_TODO_DATA_DIR` | 数据目录（库 / 图片 / 备份 / 图标缓存）、日志（`<dir>\logs`）都改到该目录；不启用单实例、不刷新开机自启注册表项；不自动弹更新日志、不写本机版本记录；托盘提示"Mini Todo（开发版）" |
| `MINI_TODO_FENCES_ROOT` | 收纳根目录改为该目录（代替 `%USERPROFILE%\MiniTodo收纳箱` / macOS `~/MiniTodo收纳箱`） |
| `MINI_TODO_DESKTOP_DIR` | "用户桌面"改为该目录（收纳箱移回的兜底目标、"不能收纳桌面本身"的保护、云同步提示的"来自桌面"判断）；mac e2e 用它避开真实 `~/Desktop` 的隐私授权框 |
| `MINI_TODO_SCREEN_ID_FILE` | 文件内容非空时替代屏幕组合 ID，用于模拟显示器组合变化 |
| `MINI_TODO_DEBUG_OPEN` | 加载完数据后额外打开的窗口（`main` = 默认清单窗口、`list:<id>` / `list:new`、`calendar`、`editor:<id>[:view\|:edit]`、`settings[:<页>]`、`changelog`（更新日志，全部版本）、`fence:new` 等，格式见 `pc-gpui/scripts/README.md`） |
| `MINI_TODO_DEBUG_CHANGELOG` | `auto`：数据隔离（`MINI_TODO_DATA_DIR`）下也按正常规则判定启动时是否弹出更新日志、写本机记录 `last_run_version`（默认隔离时既不弹也不写） |
| `MINI_TODO_DEBUG_LATEST_VERSION` | 版本检查不联网，直接当作 GitHub 最新版本（如 `v2.6.0`） |
| `MINI_TODO_DEBUG_DIALOG_PATH` | 导出 / 导入的文件对话框不弹出，直接返回该路径 |
| `MINI_TODO_DEBUG_WINDOWS_UI` | `1` 时 Linux / macOS 上也显示 Windows 专有的设置项（截图对照用） |
| `MINI_TODO_DEBUG_FENCE_STATE` | `onedrive` / `drag-hover`：收纳箱直接显示云同步提示面板（Windows OneDrive / macOS iCloud 文案）/ 拖入悬停框 |

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
- `pc-gpui-e2e-macos.yml`：macOS 编译门 + e2e（`checks` clippy / test、`smoke` debug `.app` 的 M 组与收纳箱 MF 组、`release-smoke` release `.app` 不隔离的 P 组）；
  触发条件同 Windows e2e。mac 代码没有别的编译门，改了就推分支看它
- `cloud-ci.yml`：fmt / clippy / test + Skill 单测
- **版本号只有一处**：`pc-gpui/Cargo.toml` 的 `[workspace.package] version`（改完 `Cargo.lock` 同步），当前 `3.1.0-pre`。cloud 版本在
  `cloud/Cargo.toml`，与 tag 无关。发布步骤与安装器契约见 `.trellis/spec/pc-gpui/release.md`
- 改版本号时同步改应用内更新日志 `pc-gpui/crates/app/assets/changelog.md`：把 `## 未发布` 改成 `## <版本号>（日期）`（或补写该版本的段落）；
  忘了写时单测 `current_version_has_a_changelog_section` 让 `cargo test` / CI 失败

## 注意事项

1. **目标平台**：Windows 10/11（x64）；macOS 11+（universal，预览版，未经真机验收）；Linux 可编译运行（CI 与 e2e 用），不提供 Linux 桌面包。
   清单 / 日历窗口嵌入桌面只在 Windows 上有；桌面收纳箱 Windows 与 macOS（预览）都有，Linux 上没有真实文件操作（只用于截图 / 冒烟）
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
12. **带 owner HWND 的收纳箱移动**（`fence::delete`、`fence::items::{move_in, restore, move_to}`）绝不在 UI 主线程调用（Windows 系统对话框会死锁；
    macOS 上隐私授权框、iCloud 下载、跨卷复制同样会阻塞调用线程）

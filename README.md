# Mini Todo

一款简洁高效的跨平台桌面待办事项管理应用，基于 Tauri 2 + Vue 3 + TypeScript 开发，支持 Windows / macOS。

> **新版预览：GPUI 版（不依赖 WebView2）**。仓库里的 `pc-gpui/` 是用 Rust + [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) 重写的 Windows 桌面端，
> 界面、功能与现有版本一致，数据完全互通，不再需要 WebView2 Runtime。目前处于实机验收阶段，获取、构建与切换方式见
> [GPUI 版（预览）](#gpui-版预览)。

![列表模式](docs/images/list.png)
![四象限模式](docs/images/grid.png)
![添加/编辑模式](docs/images/add.png)

## 功能特性

### 待办管理
- 以列表模式/四象限模式展示待办项（支持子任务）
- 待办描述使用 Markdown 编辑器（Milkdown），支持粘贴/拖拽上传图片
- 描述支持粘贴 Markdown 源码自动解析为富文本、GFM 语法（表格/任务清单/删除线），并提供源码/预览分栏的放大编辑弹窗（可最大化）
- 点击待办默认进入只读详情：简化排版突出内容，展示标题、Markdown 渲染的描述、通知状态与优先级，可一键切换到编辑模式
- 子任务支持标题 + 内容分离，内容使用 Markdown 编辑器（Milkdown）
- 子任务支持粘贴/拖拽上传图片，图片点击可预览
- 子任务列表双击可快捷内联编辑标题
- 支持日历展示，以及自定义颜色
- 列表/四象限模式都拖拽排序
- 系统提醒

### 云同步（WebDAV）
- 支持通过 WebDAV 协议进行云端数据同步（兼容坚果云、NextCloud、Apache / nginx 自建等）
- 同步数据包含待办事项、子任务、图片与部分应用设置
- 逐条合并：多台设备（以及云端 API）同时修改不同待办时都会保留，同一条待办以最后一次修改为准；
  删除会同步到其它设备，不会被"复活"
- 支持定时自动同步 + 手动一键同步；设置页另有"用云端覆盖本地 / 用本地覆盖云端"（带二次确认）
- 同步在后台线程执行，不会卡住界面；同步数据采用 Gzip 压缩
- 数据本身不加密，建议使用 https 的 WebDAV 地址；Windows 上 WebDAV 密码用系统 DPAPI 加密保存
- 升级提示：多台设备共用一个 WebDAV 时，请把所有 PC 端与云端 API 一起升级（PC 2.5.0 / cloud 0.3.0 起）。
  旧版本仍是"整库覆盖"式同步，混用期间旧版本那一侧仍可能丢失尚未同步的改动

### 日历模式
- 月视图日历
- 支持农历与法定节假日显示
- 支持调休上班日标记（节假日数据源）

#### 数据来源
- 法定节假日与调休上班日：NateScarlet/holiday-cn（Tauri 后端拉取 JSON）
  - https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json
- 农历/节气/传统节日：lunar-javascript
  - https://github.com/6tail/lunar-javascript

### 窗口特殊功能
- **普通模式**：浅色主题，可拖拽移动、调整大小
- **固定模式**：
  - 透明背景，融入桌面
  - macOS 下通过 Tauri 私有 API 穿透桌面，暗色主题会叠加一层半透明灰度以保证文字可读性
  - 忽略 Win+D（显示桌面，存在bug，触发后需要点击任意一个窗口，才会出来）
- 固定模式背景样式暂不支持自定义配置
- 多屏幕位置记忆（重启后根据显示器配置，保持窗口位置）

### 系统功能
- 系统托盘图标（支持双击快速添加待办项）
- 开机自启动
- 版本更新检查
- 数据导入/导出（导入与数据库升级前自动备份到 `backups/`，保留最近 5 份）
- 运行日志写入本地日志文件，便于排查问题
- WebDAV 云同步配置（设置页面内集成）

### 补充
- 因为我没有MacOS和Linux桌面，所以大概率上会存在效果偏差
- 如有相关电脑也愿意进行维护的，欢迎提交PR

## 安装

前往 [Releases](https://github.com/dreamlonglll/mini-todo/releases) 页面下载最新版本。

### Windows
- 下载 `.msi` 或 `.exe` 安装包
- 运行安装程序完成安装
- 便携版 `mini-todo-portable-windows.zip` 解压即用，但需要系统已安装
  [WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)。
  Win11 已内置；部分 Win10 / LTSC 环境需手动安装一次，否则启动会提示
  `Could not find the WebView2 Runtime`（安装包无此问题，会自动安装运行时）

#### macOS 安装提示

下载 `.dmg` 后双击打开，如果系统弹出 **"Mini Todo" is damaged and can't be opened. You should move it to the Bin**（已损坏，移到废纸篓），这并不是文件真的损坏 —— 而是 Release 产物没有 Apple Developer ID 签名，被 Gatekeeper 因 quarantine 标记拒绝。

把 `.app` 拖入 `/Applications` 后，在终端执行一次即可解除限制：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Mini\ Todo.app
```

执行后重新双击即可正常打开。如果后续升级版本再次出现相同提示，重新执行一次命令即可。


## GPUI 版（预览）

`pc-gpui/` 是 Windows 桌面端的 Rust + GPUI 重写版：

- **不依赖 WebView2**：单个 `mini-todo.exe`，界面由 GPUI 直接用 DirectX 11 绘制，不会再遇到 `Could not find the WebView2 Runtime`。
- **元素、样式、功能与现有版本一致**：列表 / 四象限 / 日历、子任务、提醒与重复提醒、WebDAV 同步、固定模式与嵌入桌面、桌面收纳箱、托盘、开机自启等。
- **数据完全互通**：与现有版本使用同一个数据目录（`%LOCALAPPDATA%\mini-todo\`）、同一个数据库、同一份 WebDAV 同步数据、同一导出格式，
  可以随时来回切换，多台设备新旧版本混用、与云端 API 一起同步也没有问题。
- 目前处于**实机验收**阶段：CI 会在 Windows 虚拟机上跑端到端测试，但固定模式、嵌入桌面、桌面收纳箱、托盘、系统通知等仍需要在真实桌面环境中验收；
  验收通过后将替换现有版本。

已知差异：

- 描述 / 子任务的 Markdown 编辑由"所见即所得"改为"**源码 + 实时预览**"（放大编辑弹窗仍是源码 / 预览分栏）；渲染效果、图片粘贴 / 拖入上传、
  链接打开规则不变。
- 多行中文的换行位置可能与旧版差一个字（逗号、句号等全角标点偶尔会出现在行首）。
- 暂时只有单个 exe，没有安装包；目前只面向 Windows 10 / 11。

### 下载试用

GitHub Actions 的 [PC (GPUI) CI](https://github.com/dreamlonglll/mini-todo/actions/workflows/pc-gpui-ci.yml) 每次构建都会产出 release 版 exe：
打开最近一次成功的运行 → 页面底部 **Artifacts** → 下载 `mini-todo-gpui-windows-x64`（zip，内含 `mini-todo.exe`；需要登录 GitHub，
产物默认保留 90 天）。解压后放到任意目录双击运行即可。

### 自己构建

需要在 **Windows** 上构建：

1. 安装 [Rust](https://rustup.rs/)（最新 stable，默认的 MSVC 工具链）。
2. 安装 Visual Studio 2022 生成工具，勾选"使用 C++ 的桌面开发"（提供 MSVC 链接器与 **Windows SDK**）。release 构建会用 Windows SDK 里的
   `fxc.exe` 预编译着色器；找不到时可以用环境变量 `GPUI_FXC_PATH` 指向它（如
   `C:\Program Files (x86)\Windows Kits\10\bin\<版本>\x64\fxc.exe`）。
3. 构建：

   ```powershell
   cd pc-gpui
   cargo build --release -p mini-todo
   # 产物：pc-gpui\target\release\mini-todo.exe
   ```

> 请用 **release** 构建分发或拷到别的电脑。debug 构建（`cargo build` / `cargo run`）的 exe 不内嵌着色器，运行时要按构建机上
> Rust 依赖缓存（cargo registry）里的绝对路径读取着色器源码现场编译，拷到其它机器上会因为找不到这些文件而无法启动。
> Linux 只用于 CI 与自动化测试，不能交叉构建出可用的 Windows 版。

### 与现有版本共存 / 切换

- **不要同时运行两个版本**。两版共用同一个单实例标识，会互相识别：一版正在运行时再启动另一版，新启动的那个会直接退出并把已运行的窗口唤到前台。
  所以切换时先在托盘菜单里"退出"当前版本，再启动另一个版本。
- 切换不需要导入导出：数据、设置、窗口位置、WebDAV 配置（包括已保存的密码）都会直接沿用。首次切换前建议在设置里"导出数据"留一份备份。
- 开机自启沿用同一个启动项：开启了开机自启时，最后一次运行的版本会把启动项更新成自己的路径。想让新版开机自启，用新版启动一次即可；
  切回旧版同理。
- 两版写入同一个日志文件（`%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`），反馈问题时附上即可。


## 开发

### 环境要求
- Node.js 22.12+（vitest / eslint 的要求）
- Rust 最新 stable
- Windows 10/11 或 macOS（Linux 可编译运行，用于 CI 与端到端测试）

> Tauri 版 PC 端代码位于 `pc/` 子目录，下面的命令都在 `pc/` 下执行；GPUI 版见下文 [GPUI 版（`pc-gpui/`）](#gpui-版pc-gpui)。

### 安装依赖

```bash
cd pc
# 安装前端依赖
npm install
```

### 开发模式

```bash
cd pc
npm run tauri dev
```

### 构建生产版本

```bash
cd pc
npm run tauri build
```

### 检查与测试

```bash
cd pc
npm run typecheck && npm run lint && npm run test   # 前端类型检查 / lint / vitest 单测
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
```

端到端验证（Linux，真实应用 + WebDAV + 云端 API）见 [`pc/scripts/e2e-linux/README.md`](pc/scripts/e2e-linux/README.md)。

### GPUI 版（`pc-gpui/`）

只需要 Rust stable，所有命令在 `pc-gpui/` 下执行：

```bash
cd pc-gpui
cargo run -p mini-todo                                   # debug 运行（只能在本机运行，见上文）
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p mini-todo                       # release（仅 Windows，需要 Windows SDK）
```

- 与已安装的正式版同时调试时，用 `MINI_TODO_DATA_DIR=<临时目录>` 启动 debug 构建（隔离数据、日志，不占用单实例与开机自启项）。
- Linux 上可以编译运行（需要 `libxkbcommon-dev libwayland-dev libvulkan-dev libfontconfig-dev` 等，完整列表见
  [`.github/workflows/pc-gpui-ci.yml`](.github/workflows/pc-gpui-ci.yml)），用于截图回归与端到端测试：
  [`pc-gpui/scripts/README.md`](pc-gpui/scripts/README.md)。
- 代码结构与约定：[`pc-gpui/crates/app/README.md`](pc-gpui/crates/app/README.md)（界面 / 窗口 / 平台）、
  [`pc-gpui/crates/core/README.md`](pc-gpui/crates/core/README.md)（业务核心）。

## 技术栈

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| 前端框架 | Vue 3 + TypeScript | 组合式 API，类型安全 |
| UI 组件库 | Element Plus | 企业级 UI 组件库 |
| Markdown 编辑器 | Milkdown | 基于 ProseMirror 的插件化 WYSIWYG 编辑器 |
| 状态管理 | Pinia | Vue 官方推荐状态管理 |
| 桌面框架 | Tauri 2.x | 轻量级跨平台桌面框架 |
| 后端语言 | Rust | 高性能，内存安全 |
| 异步运行时 | Tokio | 异步任务调度和进程管理 |
| 数据库 | SQLite | 轻量级本地数据库 |
| 云同步协议 | WebDAV | 兼容坚果云、NextCloud 等 |
| 云端 API | Rust + Axum | 独立 HTTP 服务，供 AI Skill / cron 读写待办 |

GPUI 版（`pc-gpui/`）：

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| UI 框架 | GPUI（gpui-kit 0.7.3：gpui-pre + gpui-component） | Zed 编辑器的 GPU 加速 UI 框架，不依赖 WebView |
| 界面控件 | 自绘 Element Plus 风格控件 | 外观与现有版本一致 |
| 业务核心 | Rust（`mini-todo-core`） | 数据库、同步、导入导出、提醒、收纳箱文件层，与现有版本同源 |
| 农历 | tyme4rs | 与 lunar-javascript 同一作者 |

## 项目结构

> 项目按平台拆分子目录，`pc/` 为 Windows 桌面端（Tauri），`pc-gpui/` 为 Windows 桌面端的 GPUI 重写版（将取代 `pc/`），
> `cloud/` 为云端 HTTP API（Rust + Axum），未来可平行加入 `mobile/`、`web/`。

```
mini-todo/
├── pc/                          # PC 端（Tauri 2.x + Vue 3）
│   ├── src/                     # Vue 前端源码
│   │   ├── components/          # Vue 组件
│   │   ├── stores/              # Pinia 状态管理
│   │   ├── types/               # TypeScript 类型定义
│   │   ├── views/               # 页面视图（含独立 WebView 窗口）
│   │   ├── utils/               # 工具函数
│   │   └── styles/              # 样式文件
│   ├── src-tauri/               # Tauri/Rust 后端源码
│   │   ├── src/
│   │   │   ├── commands/        # Tauri 命令（前后端桥接）
│   │   │   ├── db/              # 数据库层（SQLite + 迁移）
│   │   │   └── services/        # 业务服务层（通知 / WebDAV）
│   │   └── tauri.conf.json      # Tauri 配置
│   ├── public/                  # 公共静态资源
│   ├── package.json
│   ├── vite.config.ts
│   └── tsconfig.json
├── pc-gpui/                     # PC 端 GPUI 重写版（Rust Cargo workspace）
│   ├── crates/core/             # 业务核心（数据库 / 同步 / 导入导出 / 提醒 / 收纳箱文件层）
│   ├── crates/app/              # GPUI 界面、窗口与平台集成（托盘、通知、开机自启……）
│   └── scripts/                 # 截图回归与端到端测试脚本
├── cloud/                       # 云端 HTTP API（Rust + Axum，独立 crate）
│                                # 通过 WebDAV 与 PC 端共用同一份数据；详见 cloud/README.md
└── docs/                        # 共享文档
```

## 云端 API（可选）

如需在 PC 之外通过 AI（如 Claude Code Skill）读写自己的待办，可在 VPS 上部署 `cloud/`
子项目。它通过 WebDAV 与 PC 端共用 `sync-data.json.gz` 通道，因此不要求 PC 在线。

部署与 REST API 详情见 [`cloud/README.md`](cloud/README.md)。

### Claude Code Skill

安装配套 Skill 后，Claude Code 可直接查看/创建/更新待办：

```bash
# Linux/macOS
bash cloud/skill/minitodo/install.sh
# Windows PowerShell
.\cloud\skill\minitodo\install.ps1
```

脚本会把 `SKILL.md` / `minitodo.py` / `config.example.toml` 拷贝到
`~/.claude/skills/minitodo/`，并提示编辑 `config.toml` 填入云端 `endpoint` 与 `api_key`。

完成后即可使用：

```bash
python ~/.claude/skills/minitodo/minitodo.py today --json    # 今日待办
python ~/.claude/skills/minitodo/minitodo.py add "买菜"       # 新增
python ~/.claude/skills/minitodo/minitodo.py done C3          # 标记完成
python ~/.claude/skills/minitodo/minitodo.py sync             # 手动同步
```

Skill 使用规则见 [`cloud/skill/minitodo/SKILL.md`](cloud/skill/minitodo/SKILL.md)。

### openclaw 临期提醒

通过 [openclaw](https://github.com/anthropics/openclaw) 的 cron 机制，可以让 AI agent
定时检查临期待办并推送到 Slack / Telegram / Discord 等 IM channel：

```bash
# 1. 安装 skill 到 openclaw workspace
bash cloud/skill/minitodo/install.sh --target openclaw

# 2. 编辑 config.toml 填入 endpoint + api_key
nano ~/.openclaw/workspace/skills/minitodo/config.toml

# 3. 注册 cron（每天早 8 点检查临期，推送到 default channel）
openclaw cron add \
  --name minitodo-due-soon \
  --cron "0 8 * * *" \
  --tz "Asia/Shanghai" \
  --session isolated \
  --message "<见 openclaw.md §7 的完整 prompt>" \
  --announce
```

cron 唤起的 agent 会自行拉取所有未完成 todo、按时间锚判断哪些临期/逾期、
组织格式后推送。推送效果示例：

```
mini-todo 临期提醒｜2026-05-14 08:00
已逾期（1）：
  - #C5 [高] 写报告 (05-12 18:00，已逾期 14 小时)
未来 24h 到期（2）：
  - #C7 [中] 买菜 (05-14 10:00，2 小时后)
  - #C2 [低] 周例会 (05-14 14:00，6 小时后｜每周的周三)
```

完整安装步骤、cron message 模板、偏好配置和故障排查见
[`cloud/openclaw.md`](cloud/openclaw.md)。

## 许可证

MIT License

## 致谢

- [Tauri](https://tauri.app/) - 构建更小、更快、更安全的桌面应用
- [Vue.js](https://vuejs.org/) - 渐进式 JavaScript 框架
- [Element Plus](https://element-plus.org/) - Vue 3 组件库
- [Milkdown](https://milkdown.dev/) - 插件化 Markdown 编辑器
- [lunar-javascript](https://github.com/6tail/lunar-javascript) - 农历/节气计算
- [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) / [gpui-kit](https://github.com/longbridge/gpui-kit)（含 gpui-component） - GPUI 版的 UI 框架与组件
- [tyme4rs](https://github.com/6tail/tyme4rs) - GPUI 版的农历/节气计算
- [NateScarlet/holiday-cn](https://github.com/NateScarlet/holiday-cn) - 中国法定节假日数据

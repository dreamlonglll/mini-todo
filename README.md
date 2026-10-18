# Mini Todo

一款简洁高效的 Windows / macOS 桌面待办事项管理应用，基于 Rust + [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) 开发，
**不需要 WebView2**：Windows 上是单个 `mini-todo.exe`，macOS 上是 Apple 芯片 / Intel 通用的 `mini-todo.app`。

> 3.0 起桌面端由 Tauri + Vue 版换成了 GPUI 重写版：界面、功能与 2.x 一致，数据完全互通，安装包会原地升级 2.x 并保留全部数据。
> 提供 Windows x64 版本；3.0.2-pre 起重新提供 macOS 版（Apple 芯片 / Intel 通用，**预览版**：只在 CI 上自动化验证过，未经真机验收，
> 欢迎反馈）。不提供 Linux 桌面包。

![列表模式](docs/images/list.png)
![四象限模式](docs/images/grid.png)
![添加/编辑模式](docs/images/add.png)

> 截图来自较早的版本（左上为深色半透明主题，右下为浅色主题），标题栏按钮等细节与当前版本略有不同。

## 功能特性

### 待办管理
- 以列表模式 / 四象限模式展示待办项（支持子任务），标题栏一键切换；两种模式都支持拖拽排序，四象限之间拖动即可改分类
- 优先级（高 / 中 / 低）与自定义颜色、开始 / 截止时间；未完成的待办可一键置顶（移到列表最前）
- 待办描述支持 Markdown（GFM 表格 / 任务清单 / 删除线 / 自动链接），支持粘贴 / 拖入上传图片，图片点击可预览
- 编辑描述时为"Markdown 源码 + 实时预览"，另有源码 / 预览分栏的放大编辑弹窗（可最大化）
- 点击待办默认进入只读详情：简化排版突出内容，展示标题、Markdown 渲染的描述、通知状态与优先级，可一键切换到编辑模式
- 子任务支持标题 + 内容分离，内容同样是 Markdown，可粘贴 / 拖入图片；可从 .md / .txt 文件或文件夹批量导入
- 子任务列表双击可快捷内联编辑标题；标题栏可一键展开 / 收起全部子任务
- 「已完成」窗口：搜索已完成的待办，恢复为未完成或删除
- 编辑窗口按 Esc 关闭（有未保存的修改时先确认），Ctrl+Enter（macOS ⌘Enter）保存
- Markdown 里的链接：http / https / mailto 交给系统默认程序，`file:///` 本地文件在资源管理器（macOS 访达）中定位，其它协议一律不打开；
  只读详情里单击打开，编辑时 Ctrl / ⌘ + 单击

### 提醒
- 到点提醒可选系统通知（Windows 上是 Toast、macOS 上是通知中心）或应用内通知（屏幕右下角的小窗口，多条自下而上叠放，不抢焦点），
  在设置 → 常规「通知方式」里切换
- 准时提醒，或提前 5 / 15 / 30 分钟、1 小时，也可自定义提前时间
- 重复提醒：按天 / 周（可指定周几）/ 月（每月几号）重复，可设间隔；触发后自动推进到下一次，遇到没有该日期的月份时取当月最后一天
- 应用启动时补发关闭期间错过的重复提醒

### 云同步（WebDAV）
- 支持通过 WebDAV 协议进行云端数据同步（兼容坚果云、NextCloud、Apache / nginx 自建等），在设置 → 数据与同步里配置，可先「测试连接」
- 同步数据包含待办事项、子任务、图片与部分应用设置（窗口位置、WebDAV 配置等设备相关的设置不同步）
- 逐条合并：多台设备（以及云端 API）同时修改不同待办时都会保留，同一条待办以最后一次修改为准；
  删除会同步到其它设备，不会被"复活"
- 支持定时自动同步 + 手动一键同步；设置页另有"用云端覆盖本地 / 用本地覆盖云端"（带二次确认）
- 同步在后台线程执行，不会卡住界面；同步数据采用 Gzip 压缩
- 数据本身不加密，建议使用 https 的 WebDAV 地址（填 http:// 地址时设置页会提示明文传输的风险）；WebDAV 密码在 Windows 上用系统 DPAPI 加密保存，
  macOS 上以明文保存在本机数据库中（与 2.5.0 mac 版相同）
- 升级提示：多台设备共用一个 WebDAV 时，请把所有 PC 端与云端 API 一起升级（PC 2.5.0 / cloud 0.3.0 起）。
  更旧的版本仍是"整库覆盖"式同步，混用期间旧版本那一侧仍可能丢失尚未同步的改动

### 日历模式
- 月视图日历，在设置 → 常规「展示日历」里开启（主窗口随之加宽）
- 有开始 / 截止时间的待办显示为跨天横条，点击打开只读详情
- 支持农历、节气、传统节日与法定节假日显示
- 支持调休上班日标记（节假日数据源）

#### 数据来源
- 法定节假日与调休上班日：NateScarlet/holiday-cn（联网拉取 JSON，失败时使用本地缓存）
  - https://raw.githubusercontent.com/NateScarlet/holiday-cn/master/{year}.json
- 农历 / 节气 / 传统节日：[tyme4rs](https://github.com/6tail/tyme4rs)（结果与 [lunar-javascript](https://github.com/6tail/lunar-javascript) 逐日比对一致）

### 外观
- 浅色 / 深色主题，标题栏一键切换；深色主题可自定义界面底色与背景透明度（半透明融入桌面）
- 待办字体与字号（12~20）可在设置 → 外观里调整

### 窗口特殊功能
- **普通模式**：可拖拽移动、调整大小
- **固定模式**（标题栏锁形按钮或托盘菜单切换）：
  - 固定在当前位置，不可拖动、不可缩放，不占任务栏、不参与 Alt+Tab；被 Win+D / Win+M 最小化后自动还原
  - 可选贴边自动隐藏、唤起时置顶（设置 → 常规）
  - 可选"固定模式时，嵌入桌面中"：窗口嵌在桌面图标之上、其它窗口之下，按 Win+D 显示桌面时不会被隐藏（需要 Windows 11 24H2 及以上）
  - macOS 上：不可拖动、不可缩放、被最小化后立即还原，可选贴边自动隐藏与唤起时置顶；没有"不占程序坞"与嵌入桌面
- 多屏幕位置记忆：按显示器组合分别记住主窗口的位置、尺寸与是否固定，重启后自动恢复；设置 → 屏幕配置里可查看、删除已保存的组合；
  托盘「重置位置」把主窗口放回主显示器左上区域

### 桌面收纳箱（Windows；macOS 预览）
- 托盘 / 菜单栏图标「新增收纳箱」在桌面上创建收纳区域，把桌面上的文件 / 快捷方式 / 文件夹拖进去即可收纳
- 收纳是真实移动：文件放在 `%USERPROFILE%\MiniTodo收纳箱\<收纳箱名>\`（macOS：`~/MiniTodo收纳箱/<收纳箱名>/`）下，重名自动加 ` (2)`，
  绝不覆盖；在资源管理器（macOS：访达）里直接增删这个文件夹的内容，收纳箱会随之刷新
- 删除收纳箱时项目全部移回原位置（原位置已不存在时移回桌面）；有项目移不回去时删除中止，收纳箱原样保留并提示原因
- 双击打开、右键菜单（打开所在位置（macOS：在访达中显示）/ 隐藏 / 移回桌面 / 移动到其它收纳箱）、箱内拖动排序、Ctrl+滚轮调图标大小
  （macOS 也可以 ⌘+滚轮）、双击标题改名、折叠、固定、自定义外观
- 吸附对齐：拖动收纳箱时，边靠近其它收纳箱的同一条边（约 10 像素内）就自动对齐；拖边框调大小时，被拖的边同样会对齐，
  宽度 / 高度接近其它收纳箱时会变成一样大；被对齐的收纳箱在那条边上亮一条蓝线提示。按住 Alt（macOS：Option）拖动可暂时不吸附
- 隐藏图标：以 `.` 开头的文件自动隐藏，也可以右键隐藏单个图标；「更多」→「显示隐藏的图标」可按收纳箱切换（隐藏只影响收纳箱的显示，不改文件属性）
- 收纳箱始终在桌面层：Windows 上嵌在桌面中，Win+D 后仍可见、不占任务栏；macOS 上在桌面图标之上、其它窗口之下，每个桌面空间都能看到，
  ⌘H 不会隐藏它；按显示器组合记忆位置；托盘 / 菜单栏图标「显示/隐藏所有收纳箱」「重置收纳箱位置」可作兜底
- 桌面由 OneDrive（Windows）或 iCloud「桌面与文稿」（macOS）同步时，第一次收纳会提示：移出桌面也会让其他设备的桌面少掉这些项目
- 已知限制：Windows 10 与 Windows 11 23H2 及更早版本上，Win+D 可能会盖住收纳箱；macOS 版的限制见下文「macOS」一节

### 系统功能
- 系统托盘图标 / macOS 菜单栏图标：单击唤起主窗口；右键菜单有添加待办项、新增收纳箱 / 显示或隐藏所有收纳箱 / 重置收纳箱位置、
  固定模式、重置位置、打开设置、开机自启动、退出
- macOS：应用菜单与常用快捷键（⌘, 设置、⌘H 隐藏、⌘Q 退出、⌘M 最小化、⌘W 关闭窗口、编辑菜单）；关掉窗口不退出，点程序坞图标重新唤起主窗口
- 开机自启动（Windows 注册表启动项 / macOS LaunchAgent）
- 版本更新检查：有新的正式版时标题栏版本号旁显示红点，也可以在设置 → 关于里手动检查
- 数据导入 / 导出：导出为 ZIP 备份（默认文件名 `mini-todo-backup-YYYY-MM-DD.zip`）；导入支持本应用导出的 ZIP / JSON 备份（包括 2.x 旧版导出的），
  **导入会用备份内容覆盖当前的全部待办**；导入与数据库升级前自动备份到 `backups/`（保留最近 5 份）
- 运行日志写入本地日志文件，便于排查问题
- 设置窗口分为 常规 / 外观 / 数据与同步 / 屏幕配置 / 关于 五页

## 安装

前往 [Releases](https://github.com/dreamlonglll/mini-todo/releases) 页面下载（页面上附有各版本的更新说明）。注意预发布版（版本号带 `-pre`）
不会出现在应用内的更新提示里，需要到 Releases 页面手动下载。每个版本附带 `SHA256SUMS.txt`，可用于校验下载文件。

### Windows（Windows 10 / 11 x64）

- **安装包** `mini-todo_<版本>_x64-setup.exe`：双击安装，默认装到 `%LOCALAPPDATA%\mini-todo`，可选创建桌面快捷方式。
  已安装 2.x（Tauri 版）时会先卸载旧版再安装，待办、设置、图片都会保留；卸载只删除程序文件，不会删除数据
- **便携版** `mini-todo_<版本>_x64-portable.zip`：解压后直接运行 `mini-todo.exe`
- 不需要安装 WebView2 Runtime

数据位置（安装版、便携版、2.x 旧版共用同一份）：

- 数据库、图片、备份：`%LOCALAPPDATA%\mini-todo\`
- 日志：`%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（目录名沿用旧版，反馈问题时附上即可）
- 桌面收纳箱里的文件：`%USERPROFILE%\MiniTodo收纳箱\`

### macOS（预览版，macOS 11 及以上，Apple 芯片 / Intel 通用）

> macOS 版只在 GitHub Actions 的 macOS 虚拟机上做过自动化测试，**未经真机验收**。遇到问题请到
> [Issues](https://github.com/dreamlonglll/mini-todo/issues) 反馈，最好附上截图与日志（位置见下）。

1. 下载 `mini-todo_<版本>_universal.dmg`，打开后把 **mini-todo** 拖进「应用程序」文件夹。
   装过 2.5.0 mac 版的直接覆盖即可，待办、设置、图片都保留（两者是同一个应用、共用同一份数据）。
2. 本应用没有 Apple 开发者签名（只有 ad-hoc 签名），第一次打开会被系统拦下，二选一放行：
   - **方法一（推荐，终端执行一条命令）**：`xattr -cr /Applications/mini-todo.app`，之后正常双击打开
   - **方法二（不用终端）**：先双击打开一次，在弹框里点「完成」；然后打开「系统设置 → 隐私与安全性」，在「安全性」一栏点「仍要打开」，
     输入登录密码确认（这个按钮只在尝试打开后约一小时内出现）。macOS 15 起「右键 → 打开」已经不能绕过
   - 如果提示「已损坏，无法打开」，只能用方法一
3. 每次升级到新版本后需要重新放行一次。请先拖进「应用程序」再打开：直接在 dmg 里或「下载」文件夹里打开时，系统会把应用放到临时位置运行，
   这时无法开启开机自启

数据位置（与 2.5.0 mac 版共用同一份）：

- 数据库、图片、备份：`~/Library/Application Support/mini-todo/`
- 日志：`~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log`
- 桌面收纳箱里的文件：`~/MiniTodo收纳箱/`

与 Windows 版的差异：主窗口没有"嵌入桌面"；固定模式不会把应用从程序坞 / ⌘Tab 里藏起来；导入导出的文件对话框没有扩展名过滤；
WebDAV 密码明文保存在本机数据库中；系统通知第一次弹出时会请求通知权限。

桌面收纳箱是 macOS 版新加的功能，同样只在虚拟机上测过（没测过真实桌面、iCloud 同步的桌面、多个桌面空间 / 台前调度、多显示器与 Retina 屏、改名时的中文输入法）。
点收纳箱不会把 mini-todo 切到前台；只有双击标题改名时会切到前台（菜单栏随之变成 mini-todo），以便接收键盘输入。
正常使用不需要额外授权；如果系统弹出"mini-todo 想访问「桌面」文件夹中的文件"，点「允许」即可（也可以在「系统设置 → 隐私与安全性 →
文件与文件夹」里打开），被拒绝时收纳 / 移回会失败并提示原因。由于没有开发者签名，升级后系统可能会再问一次。

### 注意事项

- 同一时间只运行一个 Mini Todo。再次启动（包括启动另一份便携版或 2.x 旧版）时，新启动的那个会直接退出并把已运行的窗口唤到前台；
  想切换版本，先在托盘菜单（macOS：菜单栏图标或 ⌘Q）里"退出"当前版本
- 开启了开机自启时，最后一次运行的那个程序会把启动项更新成自己的路径
- 从 2.x 升级的已知差异：描述 / 子任务的 Markdown 编辑由"所见即所得"改为"源码 + 实时预览"（渲染效果、图片粘贴 / 拖入上传、链接打开规则不变）；
  多行中文的换行位置可能与旧版差一个字（逗号、句号等全角标点偶尔会出现在行首）

## 开发

### 环境要求
- Rust 最新 stable
- 在 Windows 上构建 release 版需要安装 Visual Studio 2022 生成工具，勾选"使用 C++ 的桌面开发"（提供 MSVC 链接器与 **Windows SDK**）。
  release 构建会用 Windows SDK 里的 `fxc.exe` 预编译着色器；找不到时可以用环境变量 `GPUI_FXC_PATH` 指向它（如
  `C:\Program Files (x86)\Windows Kits\10\bin\<版本>\x64\fxc.exe`）
- 在 macOS 上构建需要 Xcode Command Line Tools（`xcode-select --install`，提供 clang 与 macOS SDK）；着色器在运行时编译，不需要 Metal 工具链。
  打 universal 包另需 `rustup target add aarch64-apple-darwin x86_64-apple-darwin`，命令见
  [`pc-gpui/scripts/README.md`](pc-gpui/scripts/README.md) 的「macOS：打包、e2e 与交叉检查」。Windows / Linux 上编不了 macOS 版
- Linux 上可以编译运行（需要 `libxkbcommon-dev libwayland-dev libvulkan-dev libfontconfig-dev` 等，完整列表见
  [`.github/workflows/pc-gpui-ci.yml`](.github/workflows/pc-gpui-ci.yml)），只用于 CI、截图回归与端到端测试

PC 端代码在 `pc-gpui/`（Cargo workspace），所有命令在 `pc-gpui/` 下执行：

```bash
cd pc-gpui
cargo run -p mini-todo                                   # debug 运行
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p mini-todo-core                             # 只测业务核心（不编译 GPUI，较快）

# Windows release（需要 Windows SDK）→ target\release\mini-todo.exe
cargo build --release -p mini-todo --locked

# macOS universal release（只能在 Mac 上）→ 组装 dist/mini-todo.app（ad-hoc 签名）；打 dmg 见 scripts/README.md
cargo build --release -p mini-todo --locked --target aarch64-apple-darwin --target x86_64-apple-darwin
bash scripts/macos/bundle.sh target/aarch64-apple-darwin/release/mini-todo target/x86_64-apple-darwin/release/mini-todo dist
```

在 Linux 上开发时，可以交叉检查各平台专有的代码（只做类型检查与 lint，不链接）：

```bash
# Windows 目标：覆盖全部 cfg(windows) 代码；需要 rustup target add x86_64-pc-windows-gnu 与 mingw gcc
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
# macOS 目标：只覆盖不依赖 GPUI 的 mac 源文件；需要 rustup target add aarch64-apple-darwin
cargo clippy --manifest-path scripts/mac-check/Cargo.toml --target aarch64-apple-darwin -- -D warnings
```

mac 的 GPUI 胶水代码在 Linux / Windows 上编不了，只能靠 CI 的 macOS 作业：向 `main` 提 PR 时 `pc-gpui-ci.yml` 会在 macOS 上跑 clippy / test
并构建 universal 包；macOS 与 Windows 的端到端测试（`pc-gpui-e2e-macos.yml` / `pc-gpui-e2e-windows.yml`）可在 Actions 页面手动触发。

- 与已安装的正式版同时调试时，用 `MINI_TODO_DATA_DIR=<临时目录>` 启动 debug 构建（隔离数据、日志，不占用单实例与开机自启项）。
  其它只在 debug 构建生效的调试开关（如 `MINI_TODO_DEBUG_OPEN` 启动后直接打开指定窗口）见
  [`pc-gpui/scripts/README.md`](pc-gpui/scripts/README.md)。
- 请用 **release** 构建分发或拷到别的电脑。Windows 的 debug 构建 exe 不内嵌着色器，运行时要按构建机上 Rust 依赖缓存（cargo registry）里的绝对路径
  读取着色器源码现场编译，拷到其它机器上会因为找不到这些文件而无法启动。Linux 不能交叉构建出可用的 Windows 版。
- 安装包、便携包与 macOS dmg 由 GitHub Actions 的 `release.yml` 在推送 `v*` tag 时构建（NSIS 脚本 `pc-gpui/scripts/windows-installer.nsi`，
  mac 打包脚本 `pc-gpui/scripts/macos/`）。
- 截图回归与端到端测试：[`pc-gpui/scripts/README.md`](pc-gpui/scripts/README.md)、
  [`pc-gpui/scripts/e2e-linux/README.md`](pc-gpui/scripts/e2e-linux/README.md)（Linux，真实应用 + WebDAV + 云端 API）、
  [`pc-gpui/scripts/e2e-windows/README.md`](pc-gpui/scripts/e2e-windows/README.md)（Windows，真实窗口与输入）、
  [`pc-gpui/scripts/e2e-macos/README.md`](pc-gpui/scripts/e2e-macos/README.md)（macOS，GitHub 的 macOS 虚拟机上跑真实应用）。
- 代码结构与约定：[`pc-gpui/crates/app/README.md`](pc-gpui/crates/app/README.md)（界面 / 窗口 / 平台）、
  [`pc-gpui/crates/core/README.md`](pc-gpui/crates/core/README.md)（业务核心）；项目总览与维护检查清单见 [`CLAUDE.md`](CLAUDE.md)，
  GPUI 约定、数据兼容规则、验证门槛与发布流程见 [`.trellis/spec/`](.trellis/spec/)。
- 云端 API 是独立 crate，在 `cloud/` 下构建与测试：`cargo test --locked`；Skill 单测
  `python3 -m unittest discover -s skill/minitodo -p 'test_*.py'`。

## 技术栈

| 层级 | 技术选型 | 说明 |
|------|----------|------|
| UI 框架 | GPUI（gpui-kit 0.7.3：gpui-pre + gpui-component） | Zed 编辑器的 GPU 加速 UI 框架，Windows 上用 DirectX 11、macOS 上用 Metal 绘制，不依赖 WebView |
| 界面控件 | 自绘 Element Plus 风格控件 | 图标为 Element Plus Icons |
| Markdown | gpui-base TextView + markdown crate | GFM 渲染；编辑为"源码 + 预览" |
| 业务核心 | Rust（`mini-todo-core`） | 数据库、同步、导入导出、提醒、收纳箱文件层 |
| 数据库 | SQLite（rusqlite） | 轻量级本地数据库 |
| 云同步协议 | WebDAV（reqwest） | 兼容坚果云、NextCloud 等 |
| 农历 | tyme4rs | 与 lunar-javascript 同一作者 |
| 平台集成 | windows crate（Win32）、objc2（AppKit）、tray-icon、notify-rust、auto-launch | 窗口模式与嵌入桌面、托盘 / 菜单栏图标、系统通知、开机自启 |
| 安装包 | NSIS / dmg | Windows x64 安装包与便携 zip；macOS universal dmg（ad-hoc 签名） |
| 云端 API | Rust + Axum | 独立 HTTP 服务，供 AI Skill / cron 读写待办 |
| CI / 测试 | GitHub Actions + 本地脚本 | CI 跑 Windows / macOS / Linux 的 lint 与单测、Windows / macOS 端到端测试和发布流水线；Linux 截图回归与端到端测试用 `pc-gpui/scripts/` 在本地运行 |

## 项目结构

> 项目按平台拆分子目录：`pc-gpui/` 为 Windows / macOS 桌面端，`cloud/` 为云端 HTTP API（Rust + Axum），未来可平行加入 `mobile/`、`web/`。

```
mini-todo/
├── pc-gpui/                     # PC 端（Rust Cargo workspace；Cargo.toml 的 [workspace.package] version 是版本号唯一来源）
│   ├── crates/core/             # 业务核心（数据库 / 同步 / 导入导出 / 提醒 / 收纳箱文件层），不依赖 GUI 框架
│   ├── crates/app/              # GPUI 界面、窗口与平台集成（托盘 / 菜单栏图标、通知、开机自启……）
│   └── scripts/                 # 截图回归、端到端测试（Linux / Windows / macOS）、Windows 安装器脚本、macOS 打包脚本
├── cloud/                       # 云端 HTTP API（Rust + Axum，独立 crate）
│   │                            # 通过 WebDAV 与 PC 端共用同一份数据；详见 cloud/README.md
│   └── skill/minitodo/          # Claude Code / openclaw Skill（SKILL.md + minitodo.py + 安装脚本）
├── docs/
│   ├── 开发文档/                 # 项目概述、技术架构、功能需求、UI 设计规范、数据库设计等
│   └── images/                  # README 截图
├── .github/
│   ├── workflows/               # CI、端到端测试、发布与缓存预热
│   └── release-notes/           # 各版本的发布说明（<tag>.md，发布时作为 Release 正文）
├── .trellis/spec/               # 开发规范（GPUI 约定、数据兼容、同步协议、窗口模式、收纳箱、验证与发布）
└── CLAUDE.md / AGENTS.md        # 项目指南 / AI 助手的工作流说明
```

## 云端 API（可选）

如需在 PC 之外通过 AI（如 Claude Code Skill）读写自己的待办，可在 VPS 上部署 `cloud/`
子项目。它通过 WebDAV 与 PC 端共用 `sync-data.json.gz` 通道，因此不要求 PC 在线。
每个版本的 Release 里附带 `minitodo-cloud` 的 Linux 二进制（x86_64 / aarch64）。

部署与 REST API 详情见 [`cloud/README.md`](cloud/README.md)。

### Claude Code Skill

安装配套 Skill 后，Claude Code 可直接查看/创建/更新待办（需要 Python 3.10+ 与 `requests`，Python 3.10 另需 `tomli`）：

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

通过 [openclaw](https://github.com/openclaw/openclaw) 的 cron 机制，可以让 AI agent
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

- [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui) / [gpui-kit](https://github.com/longbridge/gpui-kit)（含 gpui-component） - UI 框架与组件
- [Element Plus Icons](https://element-plus.org/) - 图标
- [tyme4rs](https://github.com/6tail/tyme4rs) / [lunar-javascript](https://github.com/6tail/lunar-javascript) - 农历/节气计算
- [NateScarlet/holiday-cn](https://github.com/NateScarlet/holiday-cn) - 中国法定节假日数据
- 2.x 及以前的版本基于 [Tauri](https://tauri.app/)、[Vue.js](https://vuejs.org/)、[Element Plus](https://element-plus.org/)、[Milkdown](https://milkdown.dev/) 构建

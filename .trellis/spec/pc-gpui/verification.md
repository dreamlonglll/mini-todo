# 验证（pc-gpui）

> pc-gpui 的验证入口与"改什么必须跑什么"。命令默认在 `pc-gpui/` 下执行（e2e 另注明）。
> 工具细节以各自 README 为准：`pc-gpui/scripts/README.md`（`regress.py` / `shot.sh` / `MINI_TODO_DEBUG_OPEN`）、
> `pc-gpui/scripts/e2e-linux/README.md`、`pc-gpui/scripts/e2e-windows/README.md`、`pc-gpui/scripts/e2e-macos/README.md`。
> 本机（Claude Code 云端容器）是 Linux：Windows 专有行为只能交叉 clippy + CI 上的 Windows e2e + 用户实机验收；
> macOS 行为只能 `scripts/mac-check` 交叉检查（只覆盖不依赖 GPUI 的 mac 源文件）+ CI 上的 macOS 作业，维护者没有 Mac，**没有真机验收**（§7）。

---

## 1. 改什么必须跑什么

| 改动 | 必跑 |
|---|---|
| 任何改动 | §2 fmt / clippy / test（Linux） |
| `cfg(windows)` 代码：`platform/`、`services/{tray,single_instance,autostart,notify,window_mode,fences}`、`build.rs`；core 的 `services/secret.rs`、`services/fence_fs/`、`settings::system_fonts` | + §3 Windows 交叉 clippy |
| 可见界面（窗口布局、控件、`theme.rs`、文案、字体） | + §4 `regress.py`：相关场景 `--only`，逐张看 `compare/`；改共享控件 / `theme.rs` 时跑全部场景并看控件样例（`gallery`） |
| 跨窗口流程（新建 / 编辑 / 子任务 / 设置即时生效 / 导入导出 / 已完成 / 提醒 / 收纳箱新建改名） | + `regress.py --smoke` |
| core 的同步、导入导出、迁移、提醒调度；单实例、日志 | + §5 Linux e2e（F / S / L 组；同步改动跑 S 组 Apache + nginx），并按 [data-compat.md](./data-compat.md) 确认 cloud 侧也改了 |
| 版本号、`release.yml` / `cache-warm.yml`、NSIS 安装器、`scripts/macos/{bundle.sh,make-dmg.sh}` | 见 [release.md](./release.md)；改 mac 打包脚本另跑 §7.2 的 CI（`pc-gpui-ci.yml` 的 macos 作业或 mac e2e 会调用 `bundle.sh`） |
| Windows 窗口行为（窗口模式、嵌入桌面、透明、通知窗口定位、收纳箱、托盘、开机自启、单实例、DPAPI、Toast、release 构建） | + §6 Windows e2e（CI 或本机），交付说明里列出需要用户实机验收的项 |
| 不依赖 GPUI 的 mac 源文件：`platform/macos/ns.rs`、`services/autostart.rs`、`services/single_instance/unix_socket.rs`、`services/tray/mac_icon.rs`（`scripts/mac-check/src/lib.rs` 收录的那些） | + §7.1 `mac-check` 交叉检查（本机就能跑） |
| `cfg(target_os = "macos")` 代码与 mac 行为：`platform/{macos/,mac_geometry.rs,mod.rs}`、`windows/mod.rs`（建窗 / 红绿灯）、`services/{mac_menu,tray,single_instance,autostart,notify,logging,scheduler,window_mode}` 的 mac 分支、`theme.rs` 的 mac 字体、`lib.rs`、`crates/app/Cargo.toml` 的 mac 依赖 | + §7.1（能覆盖的部分）+ §7.2 推 `claude/**` 分支跑 `pc-gpui-e2e-macos.yml`（mac 代码唯一的编译门），交付说明里写明哪些只在 runner 上验证过 |
| 共享代码里会影响 mac 的改动（新窗口种类 / `WindowSpec`、新快捷键、托盘菜单、文件对话框、`cfg(not(windows))` 分支、测试里的临时目录路径） | + §7.2：mac 上 `cfg(not(windows))` 落到 mac 而不是 Linux，测试路径有 `/var` → `/private/var` 陷阱（[gpui-conventions.md §2.15](./gpui-conventions.md)） |
| 升级 `gpui-kit` | 以上全部（[gpui-conventions.md §3](./gpui-conventions.md)） |

---

## 2. fmt / clippy / 单测

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
# 只测 core（不编译 GPUI，快）
cargo clippy -p mini-todo-core --all-targets -- -D warnings
cargo test -p mini-todo-core
```

- CI（`.github/workflows/pc-gpui-ci.yml`，main / PR）在 Windows 与 Linux 上都跑 fmt / clippy / test（clippy 与 test 带 `--locked`），Windows job 另做
  `cargo build --release -p mini-todo --locked` 并上传 `mini-todo.exe`（产物 `mini-todo-gpui-windows-x64`）；macOS job 跑 clippy / test，
  再做 universal release 构建 + `bundle.sh`，上传 `mini-todo-gpui-macos-universal`（`ditto` 打的 zip）。fmt 与平台无关，mac 不重复跑。
- Linux 构建依赖（Ubuntu 24.04）：`libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libvulkan-dev
  libfontconfig-dev libfreetype-dev libssl-dev libzstd-dev libasound2-dev pkg-config clang`（与 CI 相同）。
- 测试放哪：
  - core：随旧 Tauri 版代码迁移来的全部测试（`sync_store` 合并、迁移、时间、WebDAV 互操作（`tiny_http` 模拟 Apache / nginx）、
    `fence_fs` 纯逻辑……）+ 新增的 `lunar`（逐日比对 `tests/fixtures/lunar_fixture.json`）。新业务逻辑优先写在 core 并带单测。
  - app：纯逻辑放在 `util/`、`windows/*/logic.rs`、`services/*` 等不依赖窗口的函数里并就地 `#[cfg(test)]`（旧 vitest 用例已移植到
    `util/*.rs`）。没有使用 GPUI 的 headless 测试（`gpui-kit/test-support` 会扰动 `Cargo.lock`，见 gpui-api.md §C1）——界面用 §4 验证。
  - app 测试需要内存库时依赖 `mini-todo-core` 的 feature `test-util`（`Database::new_in_memory()`，dev-dependencies 已开）。
  - Windows 专属测试（`fence_fs` 的 shell / sta / 图标、`secret` 的 DPAPI 往返）只在 Windows 上编译运行（CI Windows job 覆盖）。
  - mac 的纯逻辑尽量写成全平台编译、在 Linux 上就能跑的单测：坐标换算 `platform/mac_geometry.rs`、单实例协议
    `services/single_instance/unix_socket.rs`（`cfg(any(target_os = "macos", all(test, unix)))`）、应用菜单结构 `services/mac_menu.rs`、
    菜单栏模板图标（`tray.rs` 的测试）、App Translocation 判定（`autostart.rs`）、字体下拉过滤（`settings/appearance.rs`）。
    依赖临时目录路径的测试在 mac 上要两边 `canonicalize` 后再比较（`/var` → `/private/var`，[gpui-conventions.md §2.15](./gpui-conventions.md)）。
- 重新生成农历对照数据（在仓库根目录执行；lunar-javascript 1.7.7 装在临时目录，不要装进仓库）：
  `npm i --prefix /tmp/lunar-js lunar-javascript@1.7.7`，再 `NODE_PATH=/tmp/lunar-js/node_modules node pc-gpui/crates/core/tests/fixtures/gen_lunar.cjs`
  （PowerShell 写法见脚本文件头）。

---

## 3. Windows 交叉 clippy

```bash
rustup target add x86_64-pc-windows-gnu          # 一次性
apt-get install -y gcc-mingw-w64-x86-64          # 一次性：bundled SQLite 等 C 依赖的构建脚本要用 mingw gcc
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings
```

- 覆盖全部 `cfg(windows)` 代码（Win32 平台层、托盘、单实例、开机自启、Toast、DPAPI、收纳箱 Shell 操作、DirectWrite 字体枚举）。
- **只做 check / clippy，不要交叉链接、不要交叉 release**：GPUI 的 Windows release 构建要在 Windows 主机上用 Windows SDK 的
  `fxc.exe` 预编译着色器；debug 构建的 exe 在运行时按**构建机**上 cargo registry 的绝对路径编译着色器，拷到别的机器跑不起来。
- 没装 `x86_64-w64-mingw32-windres` 时 `build.rs` 只给 warning（exe 不嵌图标 / 版本信息），不影响检查。
- CI 的 Windows job 跑的是原生 clippy，交叉 clippy 是给 Linux 上开发时用的门槛。

---

## 4. 视觉回归：`regress.py` 与参考截图

```bash
scripts/regress.py --list                                   # 场景表
scripts/regress.py --ref <参考截图目录> --out /tmp/regress    # 全部 scene + interaction，生成对照图
scripts/regress.py --only 'settings-*' --before /tmp/regress-before --out /tmp/regress-after --no-build
scripts/regress.py --smoke --no-build                       # 端到端冒烟，按数据库断言 PASS / FAIL（失败退出码 1）
scripts/shot.sh --name main-list --window "Mini Todo" /tmp/shots   # 单个场景 / 临时交互
scripts/shot.sh --open main,gallery --window 控件样例 /tmp/shots    # ui/ 控件样例
```

- 依赖（Ubuntu）：`xvfb xdotool imagemagick mesa-vulkan-drivers fonts-noto-cjk fonts-liberation`。只依赖 Python 3 标准库。
- 每个场景是一次全新进程：`seed` 导入 `scripts/sample-data.json` → 按场景改库 → 以 `MINI_TODO_DATA_DIR` / `MINI_TODO_FENCES_ROOT` /
  `MINI_TODO_DEBUG_OPEN` 启动 debug 构建 → 交互 → 按窗口截图（与参考截图同名）。网络被断开（节假日走缓存、版本检查失败），结果确定。
- 输出：`<out>/<场景>.png`、`<out>/compare/<场景>.png`（左：参考图｜中：`--before`｜右：本次）、`<out>/logs/<场景>.log`，
  冒烟另有 `<out>/results.json`。默认输出目录 `pc-gpui/target/regress`。
- **基线用 `--before`**（改动前跑一遍的输出）做自身回归——旧 Tauri 版删除后这是唯一能随时生成的基线。
- `--ref` / 环境变量 `MINI_TODO_REF_DIR`（冒号分隔多个目录）是**可选**的：重写期间由旧 Vue 前端在 Playwright Chromium 里按真实窗口
  逻辑尺寸渲染的参考截图（时间冻结在 2026-10-15 10:00 Asia/Shanghai，1920×1080 @100%，Windows UA），在仓库外生成、不在仓库里，
  旧前端删除后无法再从仓库重新生成（见 `crates/app/README.md` §1）。
- 看对照图时：透明窗口（主窗口 / 通知 / 收纳箱）圆角外在 Xvfb 下是黑色，忽略四角；日历 / 提醒相关场景与真实时间有关，只比较布局；
  Linux 字体是 Liberation Sans + Noto Sans CJK SC 代替 Segoe UI + 微软雅黑（与参考截图相同）；中文折行位置可能差一个字（GPUI 不做
  避头尾，见 [gpui-conventions.md §2.13](./gpui-conventions.md)）。
- 界面调整后要同步改脚本开头的 `SCENES` / `SMOKE` / `CALLS`（坐标与场景）；新窗口 / 新状态加场景，并在 `MINI_TODO_DEBUG_OPEN`
  里加入口（`windows/mod.rs` 的 `DebugOpen`）。
- **菜单 / 列表里加减一项，后面所有点击坐标都会错位**：交互场景按写死的逻辑坐标点菜单项，插入一项会让后面的场景点到别的项、
  截出错误的画面而不报错。改菜单后 grep `regress.py` 里打开同一菜单的全部场景，按行高重新推算坐标并在注释里写明各项的 y 范围。
  例：收纳箱「更多」菜单加「显示隐藏的图标」后 `fence-delete-confirm` 从 y 133 改为 171；右键菜单加「隐藏」后 `fence-move-to`
  从 (190, 190) 改为 (190, 219)（10-16 fence-hidden-items）。
- 只覆盖 Linux 后端：DirectComposition 透明、嵌入桌面、托盘、Toast 等 Windows 行为看 §6。

---

## 5. Linux e2e（`pc-gpui/scripts/e2e-linux/`）

```bash
cd pc-gpui && cargo build -p mini-todo && cargo build -p mini-todo-core --example seed
cd ../cloud && cargo build                                    # cloud/target/debug/minitodo-cloud
cd .. && python3 pc-gpui/scripts/e2e-linux/run_e2e.py pc-gpui/target/debug/mini-todo cloud/target/debug/minitodo-cloud [前缀 ...]
```

- 真实的 debug 构建在 Xvfb + lavapipe 上黑盒运行（xdotool 输入、tesseract OCR 读界面），断言数据库、WebDAV 上的
  `sync-data.json.gz`、cloud REST、Skill CLI、应用日志与截图。用例组：`F`（本地功能：新建 / 编辑 / 子任务 / 完成 / 删除 / 四象限拖拽 /
  时间格式 / 5s 轮询 / 提醒补发与应用内通知 / 单实例 / 日志 / 导入导出 / 主题与日历）、`S`（同步，Apache + nginx 两种 WebDAV 各一遍，
  PC ↔ cloud ↔ Skill）、`L`（生命周期：旧版本数据库升级迁移等）。前缀过滤如 `F`、`S04[nginx]`。
- 依赖、WebDAV 服务准备（`MT_E2E_DAV_ROOT`）与环境变量见 `pc-gpui/scripts/e2e-linux/README.md`；结果在 `$MT_E2E_WORK/run/`
  （默认 `/tmp/mini-todo-gpui-e2e/run/results.txt`、`shots/`、各进程日志）。
- 何时必跑：改 core 的同步 / 合并 / 时间 / 墓碑 / 导入导出 / 迁移 / 提醒，或 app 的同步编排（`services/sync.rs`、`auto_sync.rs`）、
  单实例、日志。同步改动必须 Apache 与 nginx 两种服务器都 PASS（`backend/sync-protocol.md` §6）。

---

## 6. Windows e2e（`pc-gpui/scripts/e2e-windows/`）

- CI：`.github/workflows/pc-gpui-e2e-windows.yml`（`workflow_dispatch`，以及推送到 `claude/**` 分支且改动 `pc-gpui/**` 时）在 GitHub 托管的
  `windows-latest` 上运行（无 GPU 时 GPUI 走 D3D11 WARP）：
  - `e2e-debug`：debug 构建 + `--suite debug`——W（各窗口外观 / 尺寸 / 透明）、S（普通 → 经典固定 → 嵌入桌面、Z 序、Win+D、
    Explorer 重启重挂）、N（应用内通知样式与叠放）、I（新建 / 编辑 / 完成 / 删除、深色主题、开机自启、托盘、系统通知）、
    D（本地 WebDAV：测试连接、DPAPI 密文、同步）、F（收纳箱：嵌入、监听、改名、真实 OLE 拖入、Win+D、Explorer 重启）。
  - `release-smoke`：release 构建（验证 `fxc.exe` 着色器预编译）+ `--suite release`——P（不隔离：真实数据 / 日志位置、release 忽略
    debug 开关、单实例唤起、关主窗口退出）。
  - 失败详情与关键截图以注解输出，截图 / 日志作为产物上传。
- 本机（自己的 Windows 机器）：见 `pc-gpui/scripts/e2e-windows/README.md`。要点：屏幕不锁定、输入法切英文、测试期间不动鼠标键盘；
  不加 `--allow-real-profile` 时跳过不隔离的 P 组（会用真实数据目录、与安装版冲突）；S06 / F06 会重启 `explorer.exe`，
  I06 会写入再删除与安装版共用的 `HKCU\…\Run\mini-todo`。遵守 `guides/desktop-window-e2e-guide.md` 的"并存"与"输入安全"规则。
- 何时必跑：改 `platform/`、`services/window_mode.rs`、`services/fences.rs`、`windows/fence/`、`windows/notification.rs`、托盘 / 单实例 /
  开机自启 / Toast / DPAPI，或 `build.rs` / release profile。

---

## 7. macOS（`scripts/mac-check/`、`pc-gpui/scripts/e2e-macos/`）

维护者没有 Mac：mac 代码只能"本地交叉检查能覆盖的部分 + 推分支让 CI 的 macOS runner 编译、测试、跑 e2e"，失败日志回灌后再改。
整个 app 的 mac 目标在 Linux / Windows 上**连 `cargo check` 都过不了**：gpui-pre-apple 的 build.rs 按**宿主机** cfg 生成
`stitched_shaders.metal`（非 mac 宿主什么也不生成，随后 `include_str!` 失败），bundled SQLite 等构建脚本要 Apple SDK。
在非 mac 上能做的只有依赖解析（`cargo tree --locked --target aarch64-apple-darwin …`，改 mac 依赖后更新 `Cargo.lock`）和 §7.1。

### 7.1 本地交叉检查：`scripts/mac-check`

```bash
rustup target add aarch64-apple-darwin           # 一次性
cargo check  --manifest-path scripts/mac-check/Cargo.toml --target aarch64-apple-darwin
cargo clippy --manifest-path scripts/mac-check/Cargo.toml --target aarch64-apple-darwin -- -D warnings
```

- 是什么：不属于 workspace 的小 crate（有自己的 `Cargo.lock`），`src/lib.rs` 用 `#[path]` 把 app 里**不依赖 GPUI** 的 mac 源文件原样编进来，
  做类型检查 + clippy。Linux 与 Windows 上都能跑，不需要 Apple SDK。
- 覆盖：`platform/macos/ns.rs`（objc2 调 NSWindow / NSScreen / NSEvent 的全部 AppKit 调用）、`services/autostart.rs`（auto-launch 的 LaunchAgent）、
  `services/single_instance/unix_socket.rs`、`services/tray/mac_icon.rs`（tray-icon / image）——即对 objc2 / objc2-app-kit / tray-icon / auto-launch
  的 API 用法。
- 不覆盖：GPUI 胶水（`platform/macos/mod.rs`、`platform/mod.rs` 与 `windows/mod.rs` 的 mac 分支、`lib.rs` 的 `cfg(target_os = "macos")` 几行、
  `services/{tray,single_instance,notify,logging,scheduler}.rs` 的 mac `imp` / 函数）；也不链接、不运行、不跑测试。这些只有 §7.2。
- 已在 §2 里检查的 mac 代码：全平台编译的 `platform/mac_geometry.rs`（Linux 上跑单测）、`services/mac_menu.rs`（Linux clippy 检查它对 GPUI API 的用法）、
  `unix_socket.rs`（Linux 单测）；`theme.rs` / `settings/appearance.rs` / `settings/data.rs` 用 `cfg!(target_os = "macos")`，两个分支在所有平台都参与编译。
- 维护：新增不依赖 GPUI 的 mac 源文件就在 `mac-check/src/lib.rs` 加一行 `#[path]` 模块；`mac-check/Cargo.toml` 的依赖版本与特性必须和
  `crates/app/Cargo.toml` 的 mac 依赖**逐项一致**（版本钉成 `pc-gpui/Cargo.lock` 里的），否则这里能过、app 里不一定能过。
- 何时必跑：改了上面收录的文件或 mac 依赖。推分支前先跑，省一轮 CI。

### 7.2 CI：`pc-gpui-e2e-macos.yml`（mac 代码唯一的编译门与运行时反馈）

- 触发：`workflow_dispatch`，以及推送到 `claude/**` 分支且改动 `pc-gpui/**` 或本 workflow 时。runner `macos-15`（arm64，Apple Paravirtual GPU /
  Metal；已登录的图形会话；镜像给 `/bin/bash`、`osascript` 与 runner agent 预授权了屏幕录制、辅助功能、AppleEvents、PostEvent）。
- 作业：
  - `checks`：`cargo clippy --workspace --all-targets --locked -- -D warnings` + `cargo test --workspace --locked`（clippy 失败也照跑测试）
  - `smoke`：debug 构建 + seed → `bundle.sh target/debug/mini-todo -` 包成单架构 `.app`（bundle id 才有效）→ `lsregister` 登记 →
    `set_display.py` 尝试把显示器调到 1920×1080（失败不致命，runner 默认 1024×768pt、1x）→ `run_e2e.py --suite smoke --strict-traffic-lights`：
    M01 主窗口（尺寸、截图有内容、OCR 认出待办标题、bundle id、红绿灯已隐藏）、M02 设置、M03 只读详情、M04 AppleScript quit、M05 ⌘Q、
    M06 App 菜单、M07 菜单栏图标菜单（无收纳箱项）、M08 ⌘W 后 `open <app>` 唤起、M09 FAB 新建待办落库、M10 固定模式拖不动 / 改尺寸无效 / `is_fixed` 落库
  - `release-smoke`：release 构建 → `.app` → `run_e2e.py --suite release --allow-real-profile`（一次性 VM，不隔离）：P01 真实数据库迁移到
    `LATEST_VERSION`、日志在 `~/Library/Logs/com.tauri-app.mini-todo/`、单实例 socket；P02 二次启动 5s 内退出码 0 且唤起已运行实例、`--autostart` 静默；
    P03 LaunchAgent plist（`Label` / `ProgramArguments`）；P04 ⌘Q 退出、socket 被删
- 用例细节、判定口径（尺寸与屏幕可见区域取小、"截图确有内容"、Vision OCR 模糊匹配、前台确认后才发按键）与本地（Mac 上）运行方法见
  `pc-gpui/scripts/e2e-macos/README.md`。
- 另一道门：main / PR 上 `pc-gpui-ci.yml` 的 macos 作业（clippy / test + **universal** release 构建 + `bundle.sh`）。e2e 只构建 host 架构，
  x86_64 交叉编译、`lipo` 合并与 universal 签名靠它和 `release.yml` 覆盖；**x86_64 切片目前没有在 CI 上运行过**（可以在 arm64 runner 上
  `arch -x86_64` 走 Rosetta 跑，尚未接入）。
- 何时必跑：§1 表里的 mac 行。交付前看 Job Summary 的 PASS / FAIL 表，并逐张看截图（尤其 `M01-topleft-x3`：红绿灯是否隐藏、
  `M01-main`：字体与布局）。mac 字体与 Linux 不同，不能拿 `regress.py` 的 Linux 截图当基线。

### 7.3 取回结果（产物下载不可用时）

```bash
gh run list --workflow 'PC (GPUI) e2e (macOS)' --limit 3
gh api repos/<owner>/<repo>/actions/runs/<run_id>/jobs --jq '.jobs[] | {name, id, conclusion}'
gh api repos/<owner>/<repo>/actions/jobs/<job_id>/logs > job.log         # 整个 job 的原始日志
grep -E '\[(PASS|FAIL|SKIP)\]|TOTAL:| BAD ' job.log
gh api repos/<owner>/<repo>/check-runs/<job_id>/annotations --paginate    # 注解：E2E SUMMARY / E2E FAIL <用例> / IMG <名字> <k>/<n>
```

- 注解 API 能取到时：截图注解按标题 `IMG <名字> <k>/<n>` 排序拼接，base64 解码就是 PNG（`e2e-windows/emit_annotations.py` 文件头）。
- **注解也取不到时，从 job 日志还原截图**：日志里注解只剩正文——`::notice title=IMG …::<base64>` 记成 `##[notice]<base64>`（warning 同理），标题丢了。
  每张图的第一个分片都以 PNG 文件头的 base64 `iVBORw0KGgo` 开头，所以：按日志顺序拼接所有正文是纯 base64 的 `##[notice]` / `##[warning]` 行
  （排除 `E2E SUMMARY` 之类的文字注解），在每个 `iVBORw0KGgo` 处切开，逐段 base64 解码就是一张 PNG。顺序 = `emit/*.png` 按文件名排序，
  名字对照运行步骤里 `emit <名字>.png: …` 那几行：

  ```python
  import base64, re, sys
  log = open(sys.argv[1], encoding="utf-8", errors="replace").read()
  blob = "".join(re.findall(r"##\[(?:notice|warning)\]([A-Za-z0-9+/=]+)\s*$", log, re.M))
  for i, part in enumerate(blob.split("iVBORw0KGgo")[1:]):
      open(f"img-{i:02d}.png", "wb").write(base64.b64decode("iVBORw0KGgo" + part))
  ```

- 一个 job 最多 50 条注解（失败最多 11 条，截图两个步骤各 15 个分片），放不下的分片在步骤日志里注明"还有 N 个分片没有发出"。

### 7.4 runner 上测不了的（交付说明与发布说明写明"未经真机验收"）

- 系统通知横幅与授权弹框（镜像卸载了通知中心 UI，授权要人点）：只能断言不崩溃、日志有记录；应用内通知窗口能测但目前没有 mac 用例
- Gatekeeper 首次打开与放行流程、App Translocation；钥匙串弹窗（目前不用钥匙串）
- Retina（runner 是 1x）、多显示器 / 混合缩放、Spaces / 全屏、真实 GPU 上的阴影 / 圆角 / 透明与深色主题观感
- 原生 Intel 机型（见 §7.2）；重启 / 重新登录后开机自启是否真的拉起；贴边隐藏（目前没有 mac 用例）
- 从 2.5.0 mac 版覆盖升级的真实流程（数据目录、LaunchAgent 路径、通知授权沿用）——只能按代码与 2.5.0 实物推断

---

## 8. 交付前需要用户在 Windows 上实机验收的项

Linux 与 CI 虚拟机覆盖不到真实环境的部分，交付说明里逐项列出：

- 高 DPI / 多显示器混合缩放下的窗口位置、尺寸、屏幕组合记忆与收纳箱折叠高度
- 固定模式 / 固定 + 嵌入桌面在真实桌面（壁纸、图标、Win11 24H2+ 与 Win10 差异）上的表现；Win+D / Win+M / 任务栏"显示桌面"
- 透明窗口与深色主题的观感、DWM 圆角
- 桌面收纳箱真实拖入（含公共桌面 UAC、OneDrive 重定向桌面）、移回、删除保留语义
- 托盘菜单、开机自启（注册表项与旧版共用）、系统通知（Toast，AppUserModelID 与旧版一致）
- 从旧 Tauri 版（2.5.0）升级：安装包原地升级（[release.md](./release.md) §5）、数据目录沿用、单实例互认（运行中的一版会被唤起、
  新启动的一版退出）、与仍在用旧版的设备经 WebDAV 混用

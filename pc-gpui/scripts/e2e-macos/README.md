# macOS e2e（`pc-gpui/scripts/e2e-macos/`）

在 GitHub 托管的 macOS runner（`macos-15`，arm64，Apple Paravirtual GPU / Metal）上黑盒运行**真实的** GPUI 应用：
按 pid 枚举窗口（`CGWindowListCopyWindowInfo`）→ `screencapture -l` 截窗口 → Vision OCR 断言界面文字 → AX 读菜单 / 点菜单项 →
CGEvent 真实键鼠输入 → 读 SQLite / 文件系统断言。没有 Mac 真机时，这是 mac 代码唯一的运行时反馈。

CI：`.github/workflows/pc-gpui-e2e-macos.yml`（`workflow_dispatch`，以及推送到 `claude/**` 分支且改动 `pc-gpui/**` 时）：

| 作业 | 内容 |
|---|---|
| `checks` | clippy（`-D warnings`）+ `cargo test`（mac 专属代码的编译门） |
| `smoke` | debug 构建 → `bundle.sh` 包成 .app → `run_e2e.py --suite smoke --strict-traffic-lights`（M 组，产物 `pc-gpui-e2e-macos`） |
| `release-smoke` | release 构建 → .app → `run_e2e.py --suite release --allow-real-profile`（P 组，产物 `pc-gpui-e2e-macos-release`） |

| 文件 | 作用 |
|---|---|
| `run_e2e.py` | 用例与入口（用例表见文件头与下文） |
| `maclib.py` | Quartz 窗口枚举、截图与"截图确有内容"判定、Vision OCR 与模糊匹配、屏幕可见区域、CGEvent 键鼠（含 Unicode 输入）、前台应用、AX（应用菜单、菜单栏状态项、窗口位置 / 尺寸）、应用进程、用例框架 |
| `set_display.py` | 开跑前尝试把主显示器调到 1920×1080（`brew install displayplacer` 后 `displayplacer list` 挑模式切换；失败不致命） |
| `../e2e-windows/emit_annotations.py` | 直接复用：把 `results.json` 的失败与 `emit/*.png` 小图以注解输出 |

## 用例

### smoke（debug 构建，每个用例隔离数据目录）

| 用例 | 断言 | 信息性输出 |
|---|---|---|
| M01 主窗口 | 约 380×600pt、截图有内容、OCR 认出 `sample-data.json` 的某个未完成待办标题、bundle id = `com.tauri-app.mini-todo`；`--strict-traffic-lights` 时 AX 判定红绿灯已隐藏 | 红绿灯（AX + 左上角像素）、应用菜单栏顶层菜单、菜单栏状态项、整屏截图 |
| M02 设置窗口（`MINI_TODO_DEBUG_OPEN=settings`） | 约 680×560pt、截图有内容、OCR 认出「常规 / 外观 / 数据与同步 / 屏幕配置 / 关于」之一 | |
| M03 只读详情（`editor:1:view`） | 约 1080×600pt（屏幕放不下时按可见区域）、截图有内容、OCR 认出待办 #1 的标题 | |
| M04 退出 | `tell application id "com.tauri-app.mini-todo" to quit`（不行再 `NSRunningApplication.terminate()`）后 15s 内退出、退出码 0、没有残留窗口、无 panic | 退出方式与用时 |
| M05 ⌘Q | 先切到前台并确认前台进程是本应用（打印前台应用），经系统输入流发 ⌘Q → 10s 内退出、退出码 0 | 切前台的方式 |
| M06 App 菜单 | AX 读菜单栏 1：应用菜单有「退出… / 设置… / 隐藏…」，「编辑」有剪切 / 拷贝 / 粘贴 / 全选，「窗口」有最小化 / 关闭窗口（文案取自 `services/mac_menu.rs`）；AX 点「设置…」→ 设置窗口出现、OCR 认出「常规」 | 全部菜单项（含勾选 / 禁用） |
| M07 菜单栏图标 | 状态项菜单有「添加待办项 / 固定模式 / 重置位置 / 打开设置 / 开机自启动 / 退出」、**没有**收纳箱三项（`services/tray.rs`）；「打开设置」→ 设置窗口；「退出」→ 退出码 0 | 菜单打开方式（AXPress / 右键） |
| M08 ⌘W 与 Dock 唤起 | 前台时 ⌘W → 主窗口消失、进程仍在；`open <app>`（LaunchServices reopen → `on_reopen`）→ 主窗口重新出现、没有另起进程 | |
| M09 新建待办 | 点 FAB（主窗口右下角，中心距右 / 下边 44pt）→ 新建窗口 → `CGEventKeyboardSetUnicodeString` 逐字输入"E2E 新建待办"（OCR 认不出时退到 `pbcopy` + ⌘A ⌘V）→ ⌘Enter → 窗口关闭、库里有该标题、`created_at` / `updated_at` 是 `YYYY-MM-DD HH:MM:SS` → 主窗口 OCR 认出 | 输入方式 |
| M10 固定模式 | 普通模式拖标题栏能移动窗口 → 菜单栏图标「固定模式」→ `settings.is_fixed = 'true'`、拖不动、AX 改尺寸无效（普通模式下能改才判）→ 再点取消 → `'false'`、又能拖 | AX 位置是否可设、screen_configs、菜单勾选 |

### release（release 构建，`--allow-real-profile`，不隔离、真实目录；P01~P04 共用一个实例）

| 用例 | 断言 |
|---|---|
| P01 | 主窗口出现、截图有内容、bundle id；`~/Library/Application Support/mini-todo/data.db` 存在、`migrations` 的最大版本 = core 的 `LATEST_VERSION`（从 `crates/core/src/db/migrations.rs` 读）；`~/Library/Logs/com.tauri-app.mini-todo/mini-todo.log` 有启动记录；`/tmp/com_tauri_app_mini_todo_si.sock` 是 socket |
| P02 单实例 | 先把 Finder 切到前台 → 再次执行可执行文件 → 5s 内退出、退出码 0；第一个实例窗口数不变、被唤起到前台；`--autostart` 再执行 → 也 5s 内退出码 0、窗口数不变（是否被唤起只打印） |
| P03 开机自启 | 菜单栏图标「开机自启动」→ `~/Library/LaunchAgents/mini-todo.plist` 出现、`plutil -lint` 通过、`Label = mini-todo`、`ProgramArguments = [可执行文件绝对路径, "--autostart"]` → 再点删除 |
| P04 退出 | ⌘Q → 退出码 0、socket 文件被删、日志无 panic |

### 通用约定

- 用例互相独立（release 组共用实例，某个用例把它弄没了后面的用例重新启动）；每个用例有超时（SIGALRM），结束时杀掉本用例启动的进程
  （进程组 SIGTERM → SIGKILL），全部结束后按 exe 路径 `pgrep` 兜底；单个用例失败不影响后面的用例；任一 FAIL 时退出码为 1。
- **尺寸**：期望尺寸与主屏可见区域（`NSScreen.visibleFrame`）取小再比（±40pt）——runner 默认 1024×768pt，AppKit 把 1080 宽的窗口夹到 1024。
  屏幕尺寸、可见区域、各窗口的期望尺寸打印在"环境"分组里。
- **截图确有内容**：不透明像素 ≥ 50%、颜色数 ≥ 16、非主色像素 ≥ 0.1%，挡掉"没有屏幕录制权限，只截到壁纸 / 空白"的假成功（runner-images#8782）；
  界面上有没有字由 OCR 断言。
- **OCR**：`VNRecognizeTextRequest`（accurate、`zh-Hans` + `en-US`），小图先放大 2 倍；比较时去空白、统一全角标点，完整子串匹配，
  否则最长公共子串 ≥ 60%。数据是异步加载的，不满足时每 2 秒重截重试。
- **键盘**：先 `bring_to_front`（`NSRunningApplication.activateWithOptions` → System Events `set frontmost` → 点窗口顶部空白处），
  确认前台进程（AX 系统级 `AXFocusedApplication`）是本应用后经 `CGEventPost(kCGHIDEventTap)` 发——与真实按键同一路径；
  切不到前台时退到 `CGEventPostToPid` 并记 BAD。GPUI 按键码换算快捷键、按事件自带的字符 `insertText`，所以 Unicode 输入能打中文。
- **菜单栏图标**：AX `AXExtrasMenuBar` 的第一个子元素。先 `AXPress`（NSStatusItem 设了 menu，AppKit 弹出它；弹菜单期间应用不回 AX
  请求，所以超时 1.5s、返回 -25204 是正常的），读不到菜单再真实右键点它（tray-icon 的视图只在右键按下时弹菜单）；菜单项用 `AXPress`。
- **拖窗口**：`CGEvent` 左键在标题栏上沿（y+6pt，按钮之上的留白，属于拖动区）按下 → 分步拖动 → 抬起，按 CGWindowList 前后的位置判断。

## 输出（`--out`）

- `<用例>-*.png`：窗口截图（像素 = 逻辑点 × 缩放），`M01-screen.png` 整屏，`M01-topleft-x3.png` 主窗口左上角放大 3 倍（看红绿灯），
  `M09-editor-typed.png`、`M10-fixed.png` 等过程截图
- `<用例>-*-ocr.txt`：OCR 结果；`logs/<tag>-stdout.txt`、`logs/<tag>-mini-todo.log`（release 组的 tag 为 `P-<n>`；另有 `real-profile/`）
- `results.json`、`summary.md`（也追加到 `$GITHUB_STEP_SUMMARY`）、`fail-<用例>.txt`（该用例的完整输出）、`fail-<用例>.png`（失败时的整屏）
- `emit/*.png`：缩小、量化后的小图（M01 左上角 → M01 主窗口 → 整屏 → M02 → M03 → M09 → 失败截图；release 组 P01），由
  `emit_annotations.py images` 分片成注解

## 取回结果（产物下载不可用时）

```bash
gh run list --workflow 'PC (GPUI) e2e (macOS)' --limit 3
gh run view <run_id> --json jobs --jq '.jobs[] | {name, databaseId, conclusion}'
gh run view --job <job_id> --log | grep -E '\[(PASS|FAIL|SKIP)\]|TOTAL:| BAD '
# 注解：E2E SUMMARY / E2E FAIL <用例>，以及 IMG <名字> <k>/<n>（按标题排序拼接分片后 base64 解码就是 PNG）
gh api repos/<owner>/<repo>/check-runs/<job_id>/annotations --paginate
```

一个 job 最多 50 条注解：失败最多 11 条，截图两个步骤各发 15 个分片（每片 3900 字符）；放不下的会在步骤日志里注明"还有 N 个分片没有发出"。

## 在 Mac 上本地运行

```bash
cd pc-gpui
cargo build -p mini-todo && cargo build -p mini-todo-core --example seed
bash scripts/macos/bundle.sh target/debug/mini-todo - /tmp/mt-bundle
python3 -m pip install pyobjc-framework-Quartz pyobjc-framework-Vision pyobjc-framework-ApplicationServices pillow
python3 -u scripts/e2e-macos/run_e2e.py --app /tmp/mt-bundle/mini-todo.app --seed target/debug/examples/seed \
  --out /tmp/mt-e2e-out --work /tmp/mt-e2e-work [--only M01,M06]
```

- 终端（或运行脚本的程序）需要「屏幕录制」与「辅助功能」权限（系统设置 → 隐私与安全性），否则截图只有壁纸、窗口标题读不到、AX 报 -25211。
- smoke 组会**接管鼠标键盘**（M05、M08~M10 发真实按键、点击、拖动），跑的时候别动鼠标键盘；M04 的 AppleScript `quit` 第一次会弹自动化授权。
- smoke 组全部在隔离数据目录里运行，不碰真实数据；**release 组（`--suite release --allow-real-profile`）会读写真实数据目录、日志、
  单实例 socket 与 `~/Library/LaunchAgents/mini-todo.plist`，本机不要跑**。`--wallpaper RRGGBB` 会真的换掉桌面壁纸，本机不要加；
  `set_display.py` 会改显示器分辨率。

## 扩展

- 新用例：在 `run_e2e.py` 的 `SMOKE` / `RELEASE` 里加一项（`(用例号, 标题, 函数, 超时)`）；窗口尺寸与标题取自 `crates/app/src/windows/**`
  的 `WindowSpec`，菜单 / 界面文案取自源码，布局坐标按逻辑点从源码常量算（如 FAB = `theme.rs` 的 `FAB_INSET` + `FAB_SIZE / 2`）。
- 要在 x86_64 切片上跑：从 universal .app（`pc-gpui-ci.yml` 的产物或 release 的 dmg）启动时用 `arch -x86_64`（Rosetta）。

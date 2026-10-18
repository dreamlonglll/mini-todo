# macOS e2e（`pc-gpui/scripts/e2e-macos/`）

在 GitHub 托管的 macOS runner（`macos-15`，arm64，Apple Paravirtual GPU / Metal）上黑盒运行**真实的** GPUI 应用：
按 pid 枚举窗口（`CGWindowListCopyWindowInfo`）→ `screencapture -l` 截窗口 → Vision OCR 断言界面文字 → AX 读菜单 / 点菜单项 →
CGEvent 真实键鼠输入 → 读 SQLite / 文件系统断言。没有 Mac 真机时，这是 mac 代码唯一的运行时反馈。

CI：`.github/workflows/pc-gpui-e2e-macos.yml`（`workflow_dispatch`，以及推送到 `claude/**` 分支且改动 `pc-gpui/**` 时）：

| 作业 | 内容 |
|---|---|
| `checks` | clippy（`-D warnings`）+ `cargo test`（mac 专属代码的编译门） |
| `smoke` | debug 构建 → `bundle.sh` 包成 .app → `run_e2e.py --suite smoke --strict-traffic-lights`（M 组 + 收纳箱 MF 组，产物 `pc-gpui-e2e-macos`） |
| `release-smoke` | release 构建 → .app → `run_e2e.py --suite release --allow-real-profile`（P 组，产物 `pc-gpui-e2e-macos-release`） |

| 文件 | 作用 |
|---|---|
| `run_e2e.py` | 用例与入口（用例表见文件头与下文） |
| `maclib.py` | Quartz 窗口枚举（可含桌面元素）、截图与"截图确有内容"判定、截图差异、Vision OCR 与模糊匹配、屏幕可见区域与统一系数、桌面图标层层级、CGEvent 键鼠（含 Unicode 输入、双击、文件拖放）、前台应用、AX（应用菜单、菜单栏状态项、窗口位置 / 尺寸）、应用进程（含 NSRunningApplication 隐藏 / 取消隐藏）、用例框架 |
| `drag_source.py` | 收纳箱 MF03 的文件拖放源：pyobjc 小窗口，按下即以给定文件开始系统拖放（`NSDraggingItem` + 文件 NSURL；`--mode legacy` 为旧式 `NSFilenamesPboardType`，诊断用），stdout 每行一个 JSON 事件 |
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
| M07 菜单栏图标 | 状态项菜单有「添加待办项 / 固定模式 / 重置位置 / 打开设置 / 开机自启动 / 退出」与收纳箱三项「新增收纳箱 / 显示/隐藏所有收纳箱 / 重置收纳箱位置」（`services/tray.rs`）；「打开设置」→ 设置窗口；「退出」→ 退出码 0 | 菜单打开方式（AXPress / 右键） |
| M08 ⌘W 与 Dock 唤起 | 前台时 ⌘W → 主窗口消失、进程仍在；`open <app>`（LaunchServices reopen → `on_reopen`）→ 主窗口重新出现、没有另起进程 | |
| M09 新建待办 | 点 FAB（主窗口右下角，中心距右 / 下边 44pt）→ 新建窗口 → `CGEventKeyboardSetUnicodeString` 逐字输入"E2E 新建待办"（OCR 认不出时退到 `pbcopy` + ⌘A ⌘V）→ ⌘Enter → 窗口关闭、库里有该标题、`created_at` / `updated_at` 是 `YYYY-MM-DD HH:MM:SS` → 主窗口 OCR 认出 | 输入方式 |
| M10 固定模式 | 普通模式拖标题栏能移动窗口 → 菜单栏图标「固定模式」→ `settings.is_fixed = 'true'`、拖不动、AX 改尺寸无效（普通模式下能改才判）→ 再点取消 → `'false'`、又能拖 | AX 位置是否可设、screen_configs、菜单勾选 |

### smoke：桌面收纳箱（MF 组）

macOS 的收纳箱是桌面图标层之上一层、不激活应用的面板（GPUI `PopUp`，`kCGWindowLayer` = `CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`，
在所有普通窗口之下）。每个用例在 `<work>/<用例>-fences`（`MINI_TODO_FENCES_ROOT`）、`<work>/<用例>-desktop`（`MINI_TODO_DESKTOP_DIR`，
"移回桌面"的兜底目标）、`<work>/<用例>-src`（拖入的来源 / 移回的原目录）里操作，**绝不碰**真实的 `~/Desktop` / `~/Documents` / `~/Downloads`
（隐私授权框在 runner 上没人点，后台移动线程会一直卡住）。MF04 / MF05 / MF10 启动前直接写库预置收纳箱与项目（`seed_fence`），不依赖拖入。

| 用例 | 断言 | 信息性输出 |
|---|---|---|
| MF01 新建（`main,fence:new`） | 本进程恰好一个负层级窗口、层级 = 桌面图标层 + 1、排在所有 layer ≥ 0 的窗口之后；368×336pt、右上角离工作区右 / 上边 24pt、整个在 `visibleFrame` 内；`fences` 一行「收纳箱」、几何 = 窗口 × 统一系数、`fence_layouts` 一行、收纳文件夹存在；日志自报 `level`、`behavior` 含 `0x251`（CanJoinAllSpaces / Stationary / IgnoresCycle / FullScreenNone）且不含 `0x10e`、`canHide=false`、`panel=true nonactivating=true`、`hidesOnDeactivate=false`、`excludedFromWindowsMenu=true`；截图有内容、OCR 认出「收纳箱」或「把桌面图标拖到这里」 | 含桌面元素的窗口列表、Finder 桌面图标层窗口、`kCGWindowName` |
| MF02 目录监听 | 往收纳文件夹写 `watch.txt` → 15s 内 `fence_items` 有行且 `original_path` 为空；收纳箱截图与写入前相比 > 0.5% 像素变化 | OCR 是否认出 `watch.txt` |
| MF03 真实拖入 | `drag_source.py`（`NSDraggingItem` / 文件 NSURL）窗口里按下 → CGEvent 慢速拖到收纳箱内容区中心松开 → `a.txt` 离开原目录、进收纳文件夹、内容不变、`original_path` = 原路径；同名的另一个 `a.txt` 再拖入 → `a (2).txt`，原有的 `a.txt` 不被覆盖 | 拖放源事件（按下 / 剪贴板类型 / 结束时目标接受的操作）；第一次没生效时用 `--mode legacy`（旧式 `NSFilenamesPboardType`）再拖一次作诊断（不改变失败结论） |
| MF04 移回 | 预置 `r1.txt`（原始路径在 `-src`）与 `d1.txt`（无原始路径）；右键第一格 → 右键菜单「移回桌面」（OCR 找文字，找不到按布局常量）→ `r1.txt` 回 `-src`、行删掉；再对 `d1.txt` → 回 `-desktop`；`fence_items` 清空 | 收纳箱 OCR |
| MF05 删除收纳箱 | 预置 `k1.txt`（原始路径）、`k2.txt`（无）与 `.DS_Store`；「更多」→「删除收纳箱」→「删除」（OCR 找按钮，找不到依次试布局常量）→ 窗口关闭、`fences` 清空、`k1` 回 `-src`、`k2` 回 `-desktop`、收纳文件夹删掉、`.DS_Store` 没被搬到桌面 | 确认面板的项目数（应为 2） |
| MF06 ⌘H | 前台时 ⌘H → 主窗口不在屏幕上、收纳箱仍在；Finder 在前台时点收纳箱 → 前台不是本应用、主窗口仍隐藏（不激活应用的面板）；`NSRunningApplication.unhide()` → 主窗口回来；`hide()`（别的应用发起的隐藏）→ 收纳箱仍在 | `isHidden`、前台应用 |
| MF07 Dock 唤起 | 有收纳箱时 ⌘W → 主窗口关闭、收纳箱仍在；`open <app>` → 主窗口重新出现、没有另起进程、收纳箱没多出来 | |
| MF08 改名 | Finder 在前台时双击收纳箱标题（改名会激活应用、面板成为 key）→ 同 M09：`CGEventKeyboardSetUnicodeString` 逐字输入「E2E 收纳」，OCR 认不出时退到 `pbcopy` + ⌘A ⌘V（HID）→ 回车（HID）→ `fences.title` / `folder_name` 改名、收纳文件夹改名、层级不变；PASS 详情写明用了哪种输入方式。失败时再用 `CGEventPostToPid` 试一次，说明是"按键没路由到面板"还是"面板没进入改名"（不改变失败结论） | 双击 / 改名后的前台应用（预期是本应用）、`kCGWindowName`、OCR |
| MF09 拖动 | 标题栏 (120, 16) 按下拖 (-160, +90)（自绘拖动：超过 3pt 后窗口每帧 `setFrameOrigin:` 跟手，不走 `performWindowDragWithEvent:`；只有一个收纳箱，没有吸附目标）→ 窗口位移约等于拖动距离（±30pt）、层级不变（拖动不激活应用、不改层级）；`fences.x/y` 变了且 = 窗口位置 × 统一系数，当前组合的 `fence_layouts` 同时更新 | `fence_layouts` |
| MF10 菜单栏图标三项 | 预置左上角的收纳箱按库里的位置打开；「新增收纳箱」→ 第二个「收纳箱 2」在右上角、桌面层、自报属性正确；「显示/隐藏所有收纳箱」→ 都不在屏幕上，再点 → 都回来且仍在桌面层；「重置收纳箱位置」→ 右上角并排（间距 16pt）并落库；「退出」后不带 `MINI_TODO_DEBUG_OPEN` 重启 → 两个收纳箱在原位恢复、自报属性正确 | |

收纳箱用例的约定：
- **窗口枚举**一律不带 `kCGWindowListExcludeDesktopElements`（`M.windows_of(pid, layer=None, include_desktop=True)`）：Apple 没写它按什么判定"桌面元素"。
  收纳箱窗口 = 本进程负层级的窗口，或日志自报过的窗口号（层级不对时也找得到，交给层级断言判失败）。
- **collectionBehavior / canHide** 在 CGWindowList / AX 里读不到：核对应用日志 `[fence] 收纳箱窗口 #<窗口号> 已放到桌面层：level=… behavior=0x… canHide=…`
  （`services/fences.rs` 的 `embed_after_open`，`platform/macos/ns.rs` 的 `describe_window`）。
- **主窗口挡着收纳箱**：主窗口（layer 0）盖着的地方点不到收纳箱、也拖不进。1024×768 的 runner 上居中的主窗口与右上角的收纳箱重叠，
  点 / 拖收纳箱之前先把主窗口拖到收纳箱左边（`clear_main_off`）。
- **窗口内坐标**按 `windows/fence/{mod,grid,panels}.rs` 的布局常量（与 `../regress.py` 的 fence 场景同一套）；菜单项 / 按钮先用 OCR（截图铺黑底）找文字，找不到再用常量。
- 不覆盖：Spaces / 调度中心 / 台前调度 / 「点按墙纸显示桌面」、Retina 与多显示器、中文输入法（runner 测不了）；打开 / 在访达中显示（会拉起别的应用）；
  从真实 `~/Desktop` 拖入的隐私授权流程。

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
  切不到前台时退到 `CGEventPostToPid` 并记 BAD。**文字输入用粘贴**：runner 上经 HID 发的 `CGEventKeyboardSetUnicodeString` 逐字注入
  （虚拟键码 0，`maclib.type_text`）进不了 GPUI 的文字输入框（M09 普通窗口、MF08 收纳箱都一样；同样的事件 `CGEventPostToPid` 能进），
  所以逐字输入后 OCR 认不出就退到 `pbcopy` + ⌘A ⌘V（HID）。`CGEventPostToPid` 只作诊断：它绕过前台应用 / key 窗口的路由。
- **菜单栏图标**：AX `AXExtrasMenuBar` 的第一个子元素。先 `AXPress`（NSStatusItem 设了 menu，AppKit 弹出它；弹菜单期间应用不回 AX
  请求，所以超时 1.5s、返回 -25204 是正常的），读不到菜单再真实右键点它（tray-icon 的视图只在右键按下时弹菜单）；菜单项用 `AXPress`。
- **拖窗口**：`CGEvent` 左键在标题栏上沿（y+6pt，按钮之上的留白，属于拖动区）按下 → 分步拖动 → 抬起，按 CGWindowList 前后的位置判断。

## 输出（`--out`）

- `<用例>-*.png`：窗口截图（像素 = 逻辑点 × 缩放），`M01-screen.png` 整屏，`M01-topleft-x3.png` 主窗口左上角放大 3 倍（看红绿灯），
  `M09-editor-typed.png`、`M10-fixed.png`、`MF01-fence.png`、`MF03-after-drop.png`、`MF04-r1-menu.png`、`MF05-confirm.png` 等过程截图
  （收纳箱是半透明窗口，截图带 alpha）
- `<用例>-*-ocr.txt`：OCR 结果；`logs/<tag>-stdout.txt`、`logs/<tag>-mini-todo.log`（release 组的 tag 为 `P-<n>`；另有 `real-profile/`）
- `results.json`、`summary.md`（也追加到 `$GITHUB_STEP_SUMMARY`）、`fail-<用例>.txt`（该用例的完整输出）、`fail-<用例>.png`（失败时的整屏）
- `emit/*.png`：缩小、量化后的小图（M01 左上角 → M01 主窗口 → 整屏 → M02 → M03 → M09 → MF01 收纳箱 → 失败截图（MF03 拖入没生效、
  MF08 改名没落库时另有收纳箱截图）；release 组 P01；收纳箱铺在壁纸色上），由 `emit_annotations.py images` 分片成注解

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
- smoke 组会**接管鼠标键盘**（M05、M08~M10、MF 组发真实按键、点击、拖动），跑的时候别动鼠标键盘；M04 的 AppleScript `quit` 第一次会弹自动化授权。
  MF 组只在 `--work` 下的临时目录里移动文件，但会在桌面层建收纳箱窗口（用例结束随进程消失）、MF03 会起一个小的拖放源窗口；
  MF06 会隐藏 / 取消隐藏被测应用并把访达切到前台。
- smoke 组全部在隔离数据目录里运行，不碰真实数据；**release 组（`--suite release --allow-real-profile`）会读写真实数据目录、日志、
  单实例 socket 与 `~/Library/LaunchAgents/mini-todo.plist`，本机不要跑**。`--wallpaper RRGGBB` 会真的换掉桌面壁纸，本机不要加；
  `set_display.py` 会改显示器分辨率。

## 扩展

- 新用例：在 `run_e2e.py` 的 `SMOKE` / `RELEASE` 里加一项（`(用例号, 标题, 函数, 超时)`）；窗口尺寸与标题取自 `crates/app/src/windows/**`
  的 `WindowSpec`，菜单 / 界面文案取自源码，布局坐标按逻辑点从源码常量算（如 FAB = `theme.rs` 的 `FAB_INSET` + `FAB_SIZE / 2`）。
- 要在 x86_64 切片上跑：从 universal .app（`pc-gpui-ci.yml` 的产物或 release 的 dmg）启动时用 `arch -x86_64`（Rosetta）。

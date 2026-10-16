# macOS 冒烟 e2e（`pc-gpui/scripts/e2e-macos/`）

在 GitHub 托管的 macOS runner（`macos-15`，arm64，Apple Paravirtual GPU / Metal）上黑盒运行**真实的** GPUI 应用：
按 pid 枚举窗口（`CGWindowListCopyWindowInfo`）→ `screencapture -l` 截窗口 → Vision OCR 断言界面文字 → 退出。
没有 Mac 真机时，这是 mac 代码唯一的运行时反馈（编译门是同一个 workflow 的 `checks` 作业）。

CI：`.github/workflows/pc-gpui-e2e-macos.yml`（`workflow_dispatch`，以及推送到 `claude/**` 分支且改动 `pc-gpui/**` 时）。

| 文件 | 作用 |
|---|---|
| `run_e2e.py` | 用例与入口（用例表见文件头） |
| `maclib.py` | Quartz 窗口枚举、截图与"截图确有内容"判定、Vision OCR 与模糊匹配、AX 探测（红绿灯 / 应用菜单 / 菜单栏状态项）、应用进程、用例框架 |
| `../e2e-windows/emit_annotations.py` | 直接复用：把 `results.json` 的失败与 `emit/*.png` 小图以注解输出 |

## 用例

| 用例 | 断言 | 信息性输出（只打印） |
|---|---|---|
| M01 主窗口 | 约 380×600pt（±40）、截图有内容、OCR 认出 `sample-data.json` 的某个未完成待办标题、进程 bundle id = `com.tauri-app.mini-todo`、无 panic | 红绿灯（AX + 左上角像素）、应用菜单栏顶层菜单、菜单栏状态项（AX `AXExtrasMenuBar` 与本进程 layer 25 窗口）、整屏截图 |
| M02 设置窗口（`MINI_TODO_DEBUG_OPEN=settings`） | 约 680×560pt、截图有内容、OCR 认出「常规 / 外观 / 数据与同步 / 屏幕配置 / 关于」之一 | 左上角像素 |
| M03 只读详情（`editor:1:view`） | 约 1080×600pt、截图有内容、OCR 认出待办 #1 的标题 | |
| M04 退出 | `tell application id "com.tauri-app.mini-todo" to quit`（不行再 `NSRunningApplication.terminate()`）后 15s 内退出、退出码 0、没有残留窗口、无 panic | 退出方式与用时、日志尾部 |
| M05 ⌘Q | AX 读得到 App 菜单（Apple 菜单之外的顶层菜单）时：`CGEventPostToPid` 发 ⌘Q（不经前台应用，不会按到别的程序上；不行再切到前台、确认前台是本进程后经系统输入流发），10s 内退出、退出码 0。还没有 App 菜单时 **SKIP** 并说明 | 顶层菜单、发送方式 |

- 每个用例都是全新进程：`seed` 导入 `../sample-data.json` 到 `<work>/<用例>/` → 以 `MINI_TODO_DATA_DIR`（+ `MINI_TODO_FENCES_ROOT`、
  `MINI_TODO_DEBUG_OPEN`）直接执行 `<.app>/Contents/MacOS/mini-todo`（环境变量能传进去，`NSBundle.mainBundle` 仍是这个 .app，
  bundle id 有效）。debug 开关只在 debug 构建生效，所以 CI 包的是 debug 构建（`bundle.sh target/debug/mini-todo -`）。
- 用例之间互相独立；每个用例有超时（`--case-timeout`，默认 180s，SIGALRM），结束时一定杀掉本用例启动的进程（进程组 SIGTERM → SIGKILL），
  全部结束后再按 exe 路径 `pgrep` 兜底；单个用例失败不影响后面的用例；任一 FAIL 时退出码为 1。
- 截图"确有内容"：不透明像素 ≥ 50%、颜色数 ≥ 16、非主色像素 ≥ 0.1%——挡掉"没有屏幕录制权限，只截到壁纸 / 空白"的假成功
  （runner-images#8782）。界面上有没有字由 OCR 断言（mini-term 出过 mac 包一个字都不画却没人发现的事故）。
- OCR：`VNRecognizeTextRequest`（accurate、`zh-Hans` + `en-US`），小图先放大 2 倍；比较时去空白、统一全角标点，完整子串匹配，
  否则最长公共子串 ≥ 60%（列表里的长标题会被截断）。数据是异步加载的，OCR 不满足时每 2 秒重截重试（30~40 秒）。
- 红绿灯在第一轮只做信息性输出；`--strict-traffic-lights` 时 M01 要求 AX 判定为"隐藏"。

## 输出（`--out`）

- `<用例>-*.png`：窗口截图（像素 = 逻辑点 × 缩放），`M01-screen.png` 整屏，`M01-topleft-x3.png` 主窗口左上角 160×48pt 放大 3 倍（看红绿灯）
- `<用例>-*-ocr.txt`：OCR 结果（坐标是截图像素）；`logs/<用例>-stdout.txt`、`logs/<用例>-mini-todo.log`
- `results.json`、`summary.md`（也追加到 `$GITHUB_STEP_SUMMARY`）、`fail-<用例>.txt`（该用例的完整输出）、`fail-<用例>.png`（失败时的整屏）
- `emit/*.png`：缩小、量化后的小图，按生成顺序编号（M01 左上角放大图 → M01 主窗口 → 整屏 → M02 → M03 → 失败截图），
  由 `emit_annotations.py images` 分片成注解

stdout 里有每个用例的窗口列表（owner pid / layer / bounds / name，本进程的窗口标 `*`）、OCR 识别到的每一行、AX 探测结果、
应用日志与 stdout 的尾部——CI 日志本身就是主要的排查材料。

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
  --out /tmp/mt-e2e-out --work /tmp/mt-e2e-work [--only M01]
```

- 终端（或运行脚本的程序）需要「屏幕录制」与「辅助功能」权限（系统设置 → 隐私与安全性），否则截图只有壁纸、窗口标题读不到、AX 探测报 -25211。
- M04 的 AppleScript `quit` 第一次会弹"想要控制 mini-todo"的自动化授权；拒绝时脚本改用 `NSRunningApplication.terminate()`。
- 全部用例都在隔离数据目录里运行，不碰 `~/Library/Application Support/mini-todo`；`--wallpaper RRGGBB` 会**真的**换掉桌面壁纸，本机不要加。
- 测试期间会有窗口弹出，不需要真实键鼠输入（第一轮只做冒烟，没有点击 / 键入）。

## 扩展

- 新用例：在 `run_e2e.py` 的 `CASES` 里加一项；窗口尺寸与标题取自 `crates/app/src/windows/**` 的 `WindowSpec`，界面文字取自源码文案。
- 交互（新建待办、单实例二次启动、菜单栏图标点击）：CGEvent（`pyobjc-framework-Quartz` 的 `CGEventCreateMouseEvent` /
  `CGEventKeyboardSetUnicodeString`）+ 读 `data.db` 断言，坐标按布局常量（逻辑点）计算，思路与 `../e2e-windows/` 相同；
  菜单栏状态项用 AX（`AXExtrasMenuBar` 的子元素 `AXPress`）。
- release 冒烟（不隔离）：从 `release.yml` 的 dmg 里取 universal .app 拷到 `/Applications`，再用 `arch -x86_64` 跑一遍覆盖 x86_64 切片（Rosetta）。

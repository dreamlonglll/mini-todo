# pc-gpui 验证工具

在 Linux（Xvfb + mesa lavapipe）上运行**真实的** GPUI 应用并截图，用来和参考截图（Vue 版在浏览器里按真实窗口尺寸渲染，
`ref-ui/screens/*.png`）逐一对照。只覆盖 Linux 后端；Windows 专有行为（嵌入桌面、DirectComposition 透明、托盘……）
需要在 Windows 上实机验收。

| 文件 | 作用 |
|---|---|
| `regress.py` | **一键回归**：逐个场景起应用（全新隔离数据目录）→ xdotool 交互 → 按窗口截图（与参考截图同名）→ 生成并排对照图；`--smoke` 跑跨窗口端到端冒烟（按数据库断言 PASS / FAIL） |
| `shot.sh` | 起 Xvfb → 建临时隔离数据目录并导入样例数据 → 以 `MINI_TODO_DATA_DIR` + `MINI_TODO_DEBUG_OPEN` 启动 debug 构建 → 可选 xdotool 交互 → 截图 |
| `sample-data.json` | 样例数据（导出格式 4.0，内容与参考截图 harness 的 `fixtures.mjs` 一致；`settings` 是 harness 的 mock 形状，seed 会转换） |
| `../crates/core/examples/seed.rs` | `cargo run -p mini-todo-core --example seed -- <data-dir> <json>`：把 JSON 导入到指定数据目录 |

依赖（Ubuntu）：`xvfb xdotool imagemagick mesa-vulkan-drivers`，以及 CJK 字体（`fonts-noto-cjk`）与
`fonts-liberation`（界面字体在 Linux 上用 Liberation Sans + Noto Sans CJK SC 代替 Segoe UI + 微软雅黑，与参考截图相同）。

## regress.py

```text
pc-gpui/scripts/regress.py [选项]

  --out DIR          输出目录（默认 pc-gpui/target/regress）
  --ref DIR          参考截图目录（可多次；缺省读环境变量 MINI_TODO_REF_DIR，冒号分隔）。对照图左侧
  --before DIR       上一次的截图目录（可多次，例如改动前跑一遍的输出）。对照图中间
  --group G          scene（与参考截图同名的 48 张）/ interaction（关键交互）/ smoke（端到端冒烟）/ all；
                     缺省 scene + interaction
  --smoke            同 --group smoke
  --only GLOB        只跑名字匹配的场景（可多次，如 --only 'fence-*'）
  --list             列出场景
  --display :NN      Xvfb 显示号（默认 :75）
  --no-build         跳过 cargo build（默认先 build mini-todo 与 seed 示例）
  --keep-data        保留每个场景的数据目录（排查用）
  --tmp DIR          临时数据目录的父目录
```

输出：`<out>/<场景>.png`（按窗口裁剪，名字与参考截图相同：`main-*`、`editor-*`、`subtask-*`、`completed*`、`notification`、
`settings-*`、`fence*`；交互场景另有 `editor-color-panel`、`editor-date-panel`、`settings-bg-color-panel`、`fence-move-to` 等）、
`<out>/compare/<场景>.png`（左：参考图（透明部分铺黑底）｜中：`--before`｜右：本次，中间用品红竖线分隔）、
`<out>/logs/<场景>.log`（应用日志）；冒烟另有 `<out>/results.json`，有失败时退出码为 1。

每个场景都是一次全新的进程：`seed` 导入 `sample-data.json` → 生成子任务示例图与 2026 年节假日缓存 → 按场景改库
（深色主题、四象限、固定模式、空列表、WebDAV 设置与两条屏幕配置、收纳箱及其 6 个样例项目……）→ 以
`MINI_TODO_DATA_DIR` / `MINI_TODO_FENCES_ROOT` / `MINI_TODO_DEBUG_OPEN` 启动 → 停光标 → 交互 → 截图。网络经无效代理断开
（节假日走缓存、版本检查失败，结果确定）。场景表与坐标都在脚本开头的 `SCENES` / `SMOKE` / `CALLS` 里，界面调整后改这里。

冒烟场景（`--smoke`）：

| 场景 | 流程 | 断言 |
|---|---|---|
| `smoke-create-todo` | FAB → 新建窗口 → 输入标题 → Ctrl+Enter | 新建窗口关闭、库里有新待办，主窗口截图 |
| `smoke-edit-todo` | 点第 1 行 → 只读详情 → [编辑] → 改标题 → Ctrl+Enter | 标题写库，主窗口截图 |
| `smoke-subtasks` | 详情里新增子任务、勾选第 2 个 → Esc | 子任务写库、完成数，主窗口计数截图 |
| `smoke-settings-live` | 设置里开深色主题、外观里调字号 | 写库，主窗口即时变化截图 |
| `smoke-export-import` | 导出（`MINI_TODO_DEBUG_DIALOG_PATH`）→ 库里删一条 → 导入并确认 | 导出 ZIP 有 11 条、导入后被删的待办回来，主窗口截图 |
| `smoke-restore-completed` | 已完成窗口里取消完成第 1 条 | 完成数减 1，主窗口截图 |
| `smoke-reminder` | 应用内通知 + 一条下一分钟到期的待办 | 75 秒内弹出通知窗口、`notified = 1` |
| `smoke-fence-new` | `fence:new` → 双击标题改名 | 记录与收纳文件夹都已改名 |

## shot.sh

```text
pc-gpui/scripts/shot.sh [选项] <输出目录>

  --open SPEC        MINI_TODO_DEBUG_OPEN（默认 main），见下文
  --name NAME        截图文件名前缀（默认 shot）
  --window TITLE     只按这个窗口标题裁剪截图（默认每个可见窗口一张）；交互默认也以它为目标窗口
  --data JSON        样例数据（默认 scripts/sample-data.json）
  --data-dir DIR     直接用已有的数据目录（不导入、不删除）
  --no-build         跳过 cargo build
  --display :NN      Xvfb 显示号（默认 :94）
  --screen WxHxD     Xvfb 屏幕（默认 1920x1080x24）
  --wait SEC         窗口出现后再等多久才开始交互 / 截图（默认 4）
  --keep             截完不退出（应用留在 Xvfb 上，回车结束）

  交互（按出现顺序执行，坐标相对当前目标窗口左上角，每步之后等 0.6s）：
  --target TITLE     切换目标窗口（默认 "Mini Todo"）
  --hover X,Y        移动鼠标
  --click X,Y        左键单击        --rclick X,Y   右键单击        --dblclick X,Y   双击
  --key KEYS         xdotool key 语法，如 ctrl+Return、Escape
  --type TEXT        输入文字
  --sleep SEC        等待
```

输出：`<name>-screen.png`（整屏）、`<name>-<窗口>.png`（按窗口几何从整屏裁剪：`main` / `editor` / `subtask` /
`settings` / `completed` / `notification` / `fence`，同类多个时加 `-1`、`-2`）、`<name>-app.log`（应用日志 / 标准输出）。

例：

```bash
# 主窗口列表（对照 ref-ui/screens/main-list.png）
pc-gpui/scripts/shot.sh --name main-list --window "Mini Todo" /tmp/shots
# 悬停第 2 行
pc-gpui/scripts/shot.sh --no-build --name main-list-hover --window "Mini Todo" --hover 120,112 /tmp/shots
# 编辑窗口只读详情 + 设置窗口
pc-gpui/scripts/shot.sh --no-build --open main,editor:1:view,settings:data /tmp/shots
# 点 FAB 打开新建窗口，再按 Esc
pc-gpui/scripts/shot.sh --no-build --click 356,576 --sleep 1 --target 新建待办 --key Escape /tmp/shots
```

注意：
- Xvfb 没有合成器，透明窗口（主窗口、通知、收纳箱）圆角外的透明像素在截图里是黑色，对照时忽略四个角。
- 截图时刻是真实时间（本机没有 `faketime`）；参考截图冻结在 2026-10-15 10:00 Asia/Shanghai，日历 / 提醒时间相关的
  对照要么用 `faketime`，要么只比较布局。脚本固定 `TZ=Asia/Shanghai`。
- 临时数据目录用 `mktemp -d`，结束时删除（`--keep` 时保留，路径会打印出来）。

## MINI_TODO_DEBUG_OPEN

仅 **debug 构建**读取（release 忽略），值为逗号分隔的窗口列表，应用在加载完设置与待办之后依次打开：

| 项 | 打开 |
|---|---|
| `main` | 主窗口（总会打开，写不写都一样） |
| `editor:new`（或 `editor`） | 新建待办 |
| `editor:<todoId>` / `editor:<todoId>:view` / `editor:<todoId>:edit` | 待办详情（只读 / 编辑） |
| `subtask:<subtaskId>` / `subtask:<subtaskId>:view` / `subtask:<subtaskId>:edit` | 子任务窗口 |
| `settings` / `settings:<general\|appearance\|data\|screen\|about>` | 设置窗口（指定左侧菜单） |
| `completed` | 已完成 |
| `notification` / `notification:<todoId>` | 应用内通知（无 id 时用默认文案；有 id 时用该待办的标题与描述） |
| `fence:<fenceId>` | 收纳箱窗口 |
| `fence:new` | 走托盘「新增收纳箱」同一实现新建一个收纳箱（在光标所在显示器右上角找空位，建收纳文件夹）并打开 |
| `gallery` | 控件样例窗口（`ui/` 里的 Element Plus 风格控件，改控件后对照用） |
| `toast:<success\|info\|warning\|error>:<文字>` | 在主窗口里弹一条 ElMessage（3s 后消失，截图用 `--wait 1`） |
| `confirm:<标题>:<内容>` | 在主窗口里弹 ElMessageBox.confirm（`确定` / `取消`） |
| `calendar` | 运行时打开日历（与设置里打开「展示日历」相同：主窗口按旧版公式加宽，左栏 = 打开前的宽度） |

这些窗口**不走模态关系**（主窗口不显示遮罩、没有 owner），只用于截图；无法识别的项记 warning 并跳过。
新增窗口种类时：在 `crates/app/src/windows/mod.rs` 的 `DebugOpen` / `parse_debug_open` / `open_debug` 里加一项，并更新本表。

`MINI_TODO_DEBUG_LATEST_VERSION=v2.6.0`（仅 debug）：版本检查不联网、直接当作 GitHub 最新版本，用于截图更新红点 /
「版本更新」确认框。

其它 debug 专用环境变量（与旧版相同）：`MINI_TODO_DATA_DIR`（数据目录、日志 `<dir>/logs`；不启用单实例、不刷新开机自启项）、
`MINI_TODO_FENCES_ROOT`（收纳根目录）、`MINI_TODO_SCREEN_ID_FILE`（文件内容非空时替代屏幕组合 ID）。

GPUI 版新增的 debug 专用环境变量（release 一律忽略）：

| 变量 | 作用 |
|---|---|
| `MINI_TODO_DEBUG_DIALOG_PATH` | 非空时设置窗口的导出 / 导入文件对话框不弹出，直接返回该路径（导入导出冒烟测试） |
| `MINI_TODO_DEBUG_WINDOWS_UI` | `1` 时 Linux 上也显示 Windows 专有的设置项（「固定模式时，嵌入桌面中」），与按 Windows 渲染的参考截图对照 |
| `MINI_TODO_DEBUG_FENCE_STATE` | `onedrive`：收纳箱窗口打开即显示 OneDrive 提示面板；`drag-hover`：显示外部拖入悬停的虚线框（两者在 Linux 上无法真实触发） |

# GPUI 版端到端验证（Linux）

在 Linux（含 Claude Code 云端容器）里驱动**真实构建的 GPUI 应用**（`pc-gpui` 的 debug 构建），跑通
"界面 ↔ 本地库 ↔ WebDAV ↔ cloud ↔ Skill"整条链路。GPUI 版没有 IPC、也没有无障碍树，所以是**黑盒**测试：

- **操作**：xdotool 在 Xvfb 上点击 / 悬停 / 拖拽 / 滚轮 / 输入 / 快捷键（Ctrl+Enter、Esc、Enter）；
- **断言**：sqlite3 只读打开隔离数据目录里的 `data.db`、WebDAV 上的 `sync-data.json.gz`、cloud REST、
  Skill CLI 输出、应用日志 `<data>/logs/mini-todo.log`，以及界面文字（tesseract OCR，`chi_sim+eng`）与像素颜色；
- **截图**：每个用例的关键画面存到 `$MT_E2E_WORK/run/shots/`，失败时自动存整屏 `FAIL-<用例>-<显示号>.png`。

Windows 专有行为（嵌入桌面、桌面收纳箱、DirectComposition 透明、DPAPI、Toast、托盘、单实例）在这里测不到，
见 Windows e2e（`../e2e-windows/`）。

```
Xvfb :78 ── mini-todo（设备 A，MINI_TODO_DATA_DIR 隔离）──┐  xdotool / OCR / sqlite3
Xvfb :79 ── mini-todo（设备 B，仅 S05 双设备用例）─────────┤
                                                         │ 同步（标题栏按钮 / 设置页 / 自动同步）
              Apache mod_dav :8081 ◄─────────────────────┤ （遵守条件请求）
              nginx dav      :8082 ◄─────────────────────┘ （忽略条件请求）
                    ▲
              minitodo-cloud（REST :8787 / :8797）◄── minitodo.py（Skill CLI）
```

## 一次性准备（Ubuntu 24.04，root）

```bash
bash pc-gpui/scripts/e2e-linux/setup.sh          # 装依赖（Xvfb、xdotool、tesseract-ocr-chi-sim、mesa lavapipe、
                                                 # CJK 字体、apache2 / nginx dav…）并起两个 WebDAV（e2e / e2e-pass）
bash pc-gpui/scripts/e2e-linux/setup.sh --no-install   # 容器重启后只重新拉起 WebDAV
```

不需要 webkit / tauri-driver / selenium（只用 Python 标准库 + `requests`）。

## 构建被测程序

```bash
cd pc-gpui && cargo build -p mini-todo && cargo build -p mini-todo-core --example seed
                                         # → target/debug/mini-todo、target/debug/examples/seed（造样例数据）
cd ../cloud && cargo build               # → target/debug/minitodo-cloud
```

必须是 **debug** 构建：用例依赖只在 debug 下生效的开关（`MINI_TODO_DATA_DIR` 隔离数据目录、
`MINI_TODO_DEBUG_DIALOG_PATH` 让导入 / 导出对话框直接返回路径、`MINI_TODO_DEBUG_LATEST_VERSION` 让版本检查不联网）。

## 运行

```bash
cd pc-gpui/scripts/e2e-linux
python3 run_e2e.py ../../target/debug/mini-todo ../../../cloud/target/debug/minitodo-cloud            # 全量（约 20 分钟）
python3 run_e2e.py <app> <cloud> F                 # 只跑本地组
python3 run_e2e.py <app> <cloud> S[nginx] L01      # 前缀过滤；方括号只匹配该 WebDAV
python3 run_e2e.py <app> <cloud> S03[apache] F06b
```

| 环境变量 | 默认 | 作用 |
|---|---|---|
| `MT_E2E_WORK` | `/tmp/mini-todo-gpui-e2e` | 隔离数据目录、进程输出、截图、`results.txt` 的根 |
| `MT_E2E_DISPLAYS` | `:78,:79` | Xvfb 显示号（第二个只给 S05 的设备 B 用） |
| `MT_E2E_DAV_ROOT` | `/srv/mtodo-e2e` | `setup.sh` 建的 WebDAV 根 |
| `MT_E2E_SEED` | `<app 同目录>/examples/seed` | core 的 seed 示例 |
| `MT_E2E_REF_SHOTS` | 空 | 参考截图目录（Vue 版渲染）；设置后 F00 在结果里附 RMSE 差异度量（只报告，不断言） |

输出：`$MT_E2E_WORK/run/results.txt`（PASS / FAIL / N/A 汇总）、`run/shots/*.png`、`run/app-<会话>.log`（应用标准输出）、
`run/sessions/<会话>/data/`（每台"设备"的数据目录，含 `logs/mini-todo.log`）、`run/cloud-<webdav>/`。
任何用例失败时退出码为 1。

每组用例在自己的隔离数据目录与进程里运行（`Session` = 一台设备，`seed` 预置数据）；同组用例按顺序共享一个应用进程，
但每个用例都能用前缀单独运行（S 组缺配置 / 远端为空时会先补上）。

## 用例

| 组 | 编号 | 覆盖（全部经界面操作，除非注明） |
|---|---|---|
| F 本地 | F00 | 官方样例数据（`scripts/sample-data.json`）启动：380×600、列表 / 四象限渲染（OCR + 截图） |
| | F01 | FAB → 新建窗口 → 输入标题 → Ctrl+Enter 连按三次：库里只多一条、默认颜色 / 象限、时间规范、列表出现 |
| | F02 | 点待办 → 只读详情 → 编辑 → 改标题 → 保存：库与列表更新，`updated_at` 递增 |
| | F02b | 有未保存修改时 Esc →「放弃修改」确认框：Esc 取消留在编辑，点「放弃」关闭且不写库 |
| | F03 | 编辑窗口子任务面板：添加两条、勾选一条 → 库 + 面板 1/2 + 主窗口计数 1/2 |
| | F03b | 标题栏「展开全部子任务」→ 列表里出现子任务标题、写入 `ui-state.json` → 再点收起 |
| | F04 | 悬停操作条「完成」→ 离开列表；标题栏「已完成」窗口里列出；`local_seq` 递增；「恢复为未完成」拿回一条 |
| | F05 | 操作条「删除」+「删除确认」对话框（点按钮）→ 待办与子任务删除、墓碑写入且时间规范 |
| | F06 / F06b | 四象限拖拽改象限（默认色跟随、自定义色保留）；列表拖拽排序与「置顶」（`sort_order`） |
| | F07 | 日期 / 时间选择器键入（开始仅日期、截止、提醒）→ 新建与编辑都存 `YYYY-MM-DD HH:MM:SS` |
| | F08 | 外部直接写库（插入 / 改名）→ 5s 变更计数轮询后列表刷新 |
| | F09a / F09b | 启动补发错过的每日重复提醒并推进；1 分钟内到期的每日提醒弹应用内通知窗口（屏内、不重叠）、`notify_at` +1 天 |
| | F10 | 单实例：**N/A**（GPUI 版只在 Windows 实现；debug 隔离时也关闭） |
| | F11 | 日志落盘 `<data>/logs/mini-todo.log`（启动行、隔离提示、无 panic） |
| | F12 | 设置 → 导出数据（ZIP，含 `data.json` 4.0）→ 新建一条 → 导入数据 + 确认 → id 保留、多出的记录写墓碑、导入前自动备份、主窗口提示 |
| | F13 | 标题栏主题切换（库 + 像素）；设置「展示日历」→ 主窗口加宽、日历渲染、点日历横条打开只读详情 → 关闭复原 |
| | F14 | 空描述框点中部即进入 Markdown 编辑（旧版行为；缺陷 D1 已修复） |
| | F15 | 子任务窗口：悬停「编辑子任务」→ 输入 Markdown → Ctrl+Enter 保存 →「查看子任务」渲染出来 |
| | F16 | 未配置 WebDAV 时点标题栏同步 → 提示「请先在设置中配置 WebDAV 服务器」，不发起同步 |
| S 同步（Apache / nginx 各一遍） | S01 | 设置页填写 WebDAV：错误密码「测试连接」→「连接失败」且不保存；正确密码 →「连接成功」→ 保存配置 → 立即同步（首次上传）→ 再同步为「已是最新」 |
| | S02 | 界面新建 → 标题栏同步按钮推送、提示；再点为「已是最新」；Skill CLI `list` 经 cloud 看到它 |
| | S03 | cloud 写入（Skill `add --priority --due`、REST 带象限别名与 `T` 时间）→ PC 标题栏同步后库与列表出现，别名映射正确 |
| | S04 | 删除双向传播：PC 界面删除 → cloud 404、cloud push 不复活；cloud DELETE → PC 消失；多轮后不复活 |
| | S05 | 双设备（两个应用进程 / 两个 Xvfb）：B 界面配置并拉取、B 改名 → A、A 删除 → B、B 新建 → A，多轮后两端一致 |
| | S06 | 单调版本：上一版来自时钟快 90s 的设备 → 界面编辑后 `updated_at` = 旧值 + 1s；cloud 连续两次 PATCH 严格递增，PC 收敛到最后一版 |
| | S07 | 在描述里键入 `minitodo-image://` 引用（图片文件预先放进 images 目录）→ 上传、cloud 镜像、只读详情按原尺寸渲染；远端恶意文件名被忽略 |
| | S08 | 远端更新的设置按 `settingsUpdatedAt` 应用（窗口位置不应用，界面切回浅色）；未知顶层键经 PC 与 cloud 都保留 |
| | S09 | 设置「高级」→ 用云端覆盖本地 / 用本地覆盖云端（确认框）：强制拉取删除本地独有、强制推送给远端独有写墓碑、cloud 不复活 |
| | S10 | PC（直接写库 + 标题栏同步）与 cloud（REST + push）并发写 4 轮：一条不丢 |
| | S11 | 设置里打开自动同步（间隔 1 分钟，间隔值直接写库）→ 不点按钮也拉到 cloud 新建的待办 |
| L 生命周期 | L01 | v27 旧库（见下）启动：升级前自动备份 `backups/data-v27-*.db`、v28 / v29 迁移、存量时间规范化、数据完好并显示、日志记录 |
| | L02 | WM_DELETE_WINDOW 关闭主窗口 → 进程正常退出（Linux 无托盘）→ 数据与视图模式保留；SIGKILL 后重启库完好 |
| | L03 | 库版本比程序新（v30）→ 记警告、照常运行 |

**L01 的旧库**：先让 core 建出最新库，再按 v28 / v29 迁移的内容逆向剥掉（6 个 `local_seq` 触发器、`sync_meta`、`tombstones`、
三张收纳箱表、两次迁移新增的 settings 键与迁移记录），最后按旧版写法插入 `T` 分隔 / 无秒 / 小数秒 / 仅日期的存量时间。
v1~v27 的表结构与旧版 Tauri 应用是同一份迁移 SQL；不需要构建旧版（旧套件要求一个 v28 之前的旧版二进制）。

## 与旧版套件（`pc/scripts/e2e-linux/`）的差异

| | 旧版（Tauri） | GPUI 版 |
|---|---|---|
| 驱动 | tauri-driver + WebKitWebDriver，直接 `invoke` IPC、读 DOM | 黑盒：xdotool 操作界面，OCR / 像素 / 库 / 日志断言 |
| 数据目录 | 隔离 `HOME` | `MINI_TODO_DATA_DIR`（debug 专用）+ 隔离 `HOME` |
| 写入路径 | 大多直接调 `create_todo` 等命令 | 新建 / 编辑 / 完成 / 删除 / 拖拽 / 设置 / 同步都走界面；只有"模拟其它进程"的场景直接写库（F08、F09b 造到期提醒、S10 PC 侧写入、S11 的 1 分钟间隔、S06 的时钟快版本） |
| 安全用例 | F03 `javascript:` 链接、F04 CSP / asset 协议、F05 图片名穿越 | 不适用（没有 WebView / CSP / asset 协议；链接白名单与图片名校验由 app / core 单测覆盖）；保留"远端恶意图片名被忽略"（S07） |
| 单实例 | L03 | N/A（GPUI 版只在 Windows 实现） |
| 旧库升级 | 需要旧版二进制 | 由新库逆向剥成 v27 结构 |
| 新增 | — | 双设备 S05、自动同步 S11、设置页 WebDAV 配置（含错误密码）/ 强制同步走界面、四象限 / 列表拖拽 / 置顶、子任务窗口、已完成窗口恢复、未保存修改确认、展开全部、日历点击、导入导出走设置页、正常关闭退出 |

## 实现要点（写新用例前看）

- **没有窗口管理器**：键盘焦点用 `xdotool windowfocus`（XSetInputFocus）直接给；`xdotool key/type` 走 XTEST。
- **中文输入**：`xdotool type` 临时改键位会丢第一个 CJK 字符，含非 ASCII 的文字一律走剪贴板（`xclip` + Ctrl+V）。
  用例标题尽量用 ASCII（OCR 也更稳）。`xdotool type` 要加 `--`（文字以 `-` 开头会被当成参数）。
- **`xdotool mousemove` 不要加 `--sync`**：光标已在目标位置时会一直等到超时（每次 ~2s）。
- **OCR**（`harness.OcrResult`）：灰度放大 3 倍（不要 `-normalize`，会把浅色底变灰）；`wait_text` 轮流用 psm 6（整块）、
  psm 11（稀疏）、红色通道（蓝底白字的主按钮）；匹配忽略空白 / 大小写 / 全半角，允许相邻两行拼接（窄列折行）与相似度
  ≥ 0.8 的模糊匹配；删除线文字（已完成）放宽到 0.65。浅灰占位文字（`添加子任务...`）OCR 不可靠，按布局点。
  确认框按钮：取标题下方**最靠下**的一处精确匹配（按钮文字常出现在提示语里）；tesseract 偶尔整行认不出按钮时退回 Enter（确认框约定 Enter = 确认）。
- **ElMessage 提示会盖住标题栏右区按钮**，光标停在提示上它不会消失（与 Element Plus 一样悬停暂停计时）：点标题栏按钮前
  `ui.wait_no_toast` 先把光标移开并等标题栏底部恢复底色。
- **拖拽**：手柄是标题左侧的颜色圆点（标题文字左边 16px），`XDisplay.drag` 按下 → 分步移动 → 停 0.4s → 松开。
- **布局常量**（`ui.py`）：标题栏右区按钮从右往左 设置 / 同步 / 固定 / 主题 / 视图 / 已完成，中心 x = `w-31-36i`、y=27；
  FAB 中心 `(w-44, h-44)`；行操作条 完成 / 置顶 / 删除 在 `w-97 / w-69 / w-41`、行标题那一行；编辑窗口标题输入框 (350,119)、
  描述框中部 (350,245)；子任务行悬停操作 查看 / 编辑 / 删除 在 x=985 / 1011 / 1037（只读模式只有查看，在 1037）；
  设置窗口左侧菜单 x=75、y=85+39i，开关在 x=636。
- **关闭窗口**：`XDisplay.wm_close` 发 WM_DELETE_WINDOW（`xdotool windowclose` 是 XDestroyWindow，不走应用的关闭流程）。
- **同步完成**以日志行 `[sync] 同步完成（…）: <报告>` 为准，提示文字作为附加断言。
- 系统通知需要通知守护进程：本地组切到应用内通知（`notification_type = app`）。

## 已知缺陷与观察

| 编号 | 现象 | 用例 |
|---|---|---|
| D1（已修复） | 编辑模式下描述为空时只有描述框顶部约 10px 能点进源码编辑。原因是 `ui/markdown_editor.rs` 根元素只有 `min_h_full()`，父元素没有确定高度时百分比最小高度不生效；改为在父级 flex 列里 `flex_grow` 撑满 | F14 |
| O1（已修复） | 与文字同一段的图片被画成一行字高；现在含图片的段落按行拆成文字 / 图片片段排版，图片按原尺寸（最大宽度 100%）、与文字垂直居中，与旧版一致。剩余差异：图片旁的文字超过一行时整段先换到下一行；链接 / 标题 / 表格里的图片仍按行高绘制 | S07 |
| O2 | 应用内通知窗口 320×120 时描述行被裁掉（标题下没有剩余高度）；旧版在 Windows 上同样只露出几像素 | F09b 截图 |

## 已知环境差异

- Xvfb 没有合成器：透明窗口（主窗口、通知）圆角外的透明像素在截图里是黑色；深色主题的半透明底在像素断言里按纯黑处理。
- 容器默认 UTC：套件对应用、seed、cloud 统一设置 `TZ=Asia/Shanghai`。
- 截图是真实时间（日历"今天"、提醒时间随运行时刻变化），与参考截图只比较布局。

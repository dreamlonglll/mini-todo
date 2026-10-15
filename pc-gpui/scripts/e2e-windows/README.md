# GPUI 版 Windows 端到端测试

对着**真实的** GPUI 窗口做验证：真实鼠标键盘输入（`SetCursorPos` + `SendInput`）、Win32 只读断言（owner、ex style、
Z 序、`WM_NCHITTEST`、Win+D、Explorer 重启）、截图取色、读 `data.db`、注册表、真实 OLE 拖放。CI 在 GitHub 托管的
`windows-latest` 上跑（`.github/workflows/pc-gpui-e2e-windows.yml`，无 GPU 时 GPUI 走 D3D11 WARP 软件渲染）。

GPUI 窗口没有 DOM / CDP，按钮位置按布局常量（逻辑像素，见 `crates/app/src/windows/**`）点击；会随内容移动的目标
（列表行的颜色圆点、设置页开关、输入框边框、蓝色按钮）先截图按颜色识别再点。

| 文件 | 作用 |
|---|---|
| `run_e2e.py` | 用例与汇总（PASS / FAIL / SKIP、`summary.md`、`results.json`、`GITHUB_STEP_SUMMARY`），任一 FAIL 退出码 1 |
| `winlib.py` | ctypes Win32 封装：窗口枚举与状态、输入、截图取色、应用进程（隔离数据目录）、SQLite、用例汇总 |
| `set-display.ps1` | 分辨率调到 1920×1080（不支持时取最接近的较小模式）、桌面换成纯色 `rgb(150,40,170)`（透明窗口取色对照） |
| `probe-window.ps1` | 探针：普通应用窗口（WinForms），验证嵌入桌面的窗口在它之下 |
| `drag-source.ps1` | OLE 拖放源窗口：`-Mode winforms`（.NET `DataObject` FileDrop）/ `-Mode shell`（`IShellFolder::GetUIObjectOf` 的数据对象） |
| `uia-find.ps1` | UI Automation 找元素矩形（资源管理器窗口里的列表项等） |
| `dav_server.py` | 本地 WebDAV（wsgidav + cheroot，Basic 认证，JSONL 访问日志） |
| `emit_annotations.py` | 把失败详情与 `<out>/emit/*.png` 截图作为 GitHub 注解输出（见下文「取回结果」） |
| `uia-dump.ps1`、`emit-image.ps1` | 诊断：打印窗口的 UIA 树 / 把图片以 base64 打进日志 |

## 用例

| 组 | 内容 | 隔离 |
|---|---|---|
| W01–W12 | `MINI_TODO_DEBUG_OPEN` 打开 main / editor:1:view / editor:1:edit / editor:new / subtask:101:view / settings 五个页 / completed / notification:1：窗口存在、类名 `Zed::Window`、逻辑尺寸、已渲染、截图；主窗口四角透出桌面（DirectComposition 透明） | 是 |
| S01–S07 | 普通模式（APPWINDOW、无 owner、`HTCAPTION` / `HTLEFT`、拖动标题栏移动且落库）→ 点锁按钮经典固定（TOOLWINDOW、`WM_NCHITTEST` 不再是拖动 / 缩放区、真实拖动不动、`is_fixed` 落库）→ `fixed_embed_desktop=true` 重启恢复为嵌入桌面（owner=桌面宿主、TOOLWINDOW、无 MINIMIZEBOX）→ 探针窗口验证 Z 序 → Win+D → 重启 Explorer 自动重挂 → 点锁按钮回普通模式 | 是 |
| N01–N02 | 3 条应用内通知：TOOLWINDOW\|TOPMOST、不抢焦点、右对齐、自下而上叠放（间距 10）；N02 单独检查客户区距工作区底边正好 20 | 是 |
| I00–I08 | FAB 新建 → 输入 → Ctrl+Enter 落库、主窗口刷新；点行 → 只读详情 → 编辑 → 改标题保存；悬停操作条完成 / 删除（确认框）；设置切深色主题（落库、主窗口变半透明深色并透出桌面色）；设置开机自启开 / 关（`HKCU\…\Run\mini-todo`）；托盘图标（`Shell_NotifyIconGetRect`）；系统通知（到期提醒 notified=1、日志无发送失败） | 是 |
| D00–D04 | 本地 WebDAV（Basic 认证）：测试连接（服务器收到带认证的 PROPFIND）、保存配置（`webdav_password` 为 `dpapi:` 密文、库文件无明文）、立即同步（服务器出现 `mini-todo/sync-data.json.gz`、`webdav_last_sync_at` 更新）、标题栏同步 | 是 |
| F01–F11 | `fence:new`：嵌入桌面、收纳文件夹建在 `MINI_TODO_FENCES_ROOT` 下；放文件 → 监听刷新出图标；新实例第一次 OLE 拖入；双击标题改名 → 文件夹同步改名；改名后 / 单击标题后紧接着的拖入；Shell 数据对象拖入；从资源管理器窗口真实拖入；可见区域 = 数据库几何；Win+D 后可见；Explorer 重启后重挂 | 是 |
| P01–P03 | **不隔离**：主窗口、日志在 `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`、库在 `%LOCALAPPDATA%\mini-todo`（release 另验证忽略 `MINI_TODO_DATA_DIR` / `MINI_TODO_DEBUG_OPEN`）；单实例（第二个进程退出、已运行实例被唤起到前台）；WM_CLOSE 关主窗口 → 进程退出 | 否 |

release job 只跑 P 组（加上 release 构建本身：`build.rs` 用 fxc 预编译着色器）。

## 本地运行（自己的 Windows 机器）

前置：Rust（MSVC）、Python 3.10+、`pip install pillow wsgidav cheroot`；屏幕不锁定、前台不是管理员窗口
（否则模拟输入会被丢弃）；**中文输入法切到英文**；测试期间不要动鼠标键盘。

```powershell
cd pc-gpui
cargo build -p mini-todo
cargo build -p mini-todo-core --example seed
# 可选：分辨率 1920x1080 + 纯色桌面（会改你的桌面背景；透明取色用例依赖背景不是纯黑 / 纯白）
# pwsh -NoProfile -File scripts/e2e-windows/set-display.ps1
python scripts/e2e-windows/run_e2e.py --exe target/debug/mini-todo.exe --seed target/debug/examples/seed.exe `
  --out $env:TEMP\mt-e2e-out --only W,S,N,I,D,F
```

- 开始时会把屏幕上其它程序的窗口**最小化**（避免挡住桌面、干扰取色与 Z 序断言）。
- 不加 `--allow-real-profile` 时 P 组跳过：P 组不隔离，会使用真实的 `%LOCALAPPDATA%\mini-todo` 数据目录、与正在运行的
  安装版冲突（单实例会把它唤起）。只在一次性虚拟机 / 没装正式版的机器上加。
- S06 / F06 会 `taskkill explorer.exe` 再拉起；I06 会写入再删除 `HKCU\…\Run\mini-todo`（与安装版共用的值名，
  已开启开机自启的机器上跑完需要重新打开）；F08 会打开一个资源管理器窗口。
- `--only S` 只跑某几组；`--emit-shots F08,N01` 把这些用例的截图另存到 `<out>/emit/`；截图、各实例日志在 `--out` 目录。
- release 冒烟：`cargo build --release -p mini-todo`，然后 `--exe target/release/mini-todo.exe --suite release --allow-real-profile`。

## 取回结果（CI）

汇总表打印在 “Run e2e” 步骤末尾，也写进 Job Summary；截图与日志在产物 `pc-gpui-e2e-windows-debug` / `-release`。
产物下载链接（`*.blob.core.windows.net`）不可达的环境里，可以只用 GitHub API 取回：

- 每个 FAIL 一条 error 注解（标题 `E2E FAIL <用例>`，正文是失败原因 + 用例输出），另有一条 `E2E SUMMARY` warning；
- `<out>/emit/*.png`（`--emit-shots` 指定的截图、第一个失败用例的整屏截图）切成 3900 字符的 base64 分片，作为
  `IMG <名字> <k>/<n>` 的 notice / warning 注解（注解正文上限 4096 字符、每步每级最多 10 条）。

```bash
gh api "repos/<owner>/<repo>/check-runs/<job_id>/annotations?per_page=100" > ann.json
# 按标题 "IMG <名字> k/n" 把同名分片按 k 拼接后 base64 解码即得 PNG
```

# GPUI 版 Windows 端到端测试

对着**真实的** GPUI 窗口做验证：真实鼠标键盘输入（`SetCursorPos` + `SendInput`）、Win32 只读断言（owner、ex style、
Z 序、`WM_NCHITTEST`、Win+D、Explorer 重启）、截图取色、读 `data.db`。CI 在 GitHub 托管的 `windows-latest` 上跑
（`.github/workflows/pc-gpui-e2e-windows.yml`，无 GPU 时 GPUI 走 D3D11 WARP 软件渲染）。

GPUI 窗口没有 DOM / CDP，按钮位置按布局常量（逻辑像素，见 `crates/app/src/windows/**`）点击；会随内容移动的目标
（列表行、开关、蓝色按钮）先截图按颜色识别再点。

| 文件 | 作用 |
|---|---|
| `run_e2e.py` | 用例与汇总（PASS / FAIL / SKIP、`summary.md`、`results.json`、`GITHUB_STEP_SUMMARY`），任一 FAIL 退出码 1 |
| `winlib.py` | ctypes Win32 封装：窗口枚举与状态、输入、截图取色、应用进程（隔离数据目录）、SQLite、用例汇总 |
| `set-display.ps1` | 分辨率调到 1920×1080（不支持时取最接近的较小模式）、桌面换成纯色 `rgb(150,40,170)`（透明窗口取色对照） |
| `probe-window.ps1` | 探针：普通应用窗口（WinForms），验证嵌入桌面的窗口在它之下 |
| `drag-source.ps1` | OLE 拖放源（WinForms `DoDragDrop` FileDrop），验证真实拖入收纳箱 |
| `dav_server.py` | 本地 WebDAV（wsgidav + cheroot，Basic 认证，JSONL 访问日志） |
| `uia-dump.ps1` | 诊断：打印窗口的 UI Automation 树 |
| `emit-image.ps1` | 把图片以 base64 打进日志（产物下载不可用时用） |

## 用例

| 组 | 内容 | 隔离 |
|---|---|---|
| W01–W12 | `MINI_TODO_DEBUG_OPEN` 打开 main / editor:1:view / editor:1:edit / editor:new / subtask:101:view / settings 五个页 / completed / notification:1：窗口存在、类名 `Zed::Window`、逻辑尺寸、已渲染、截图；主窗口四角透出桌面 | 是 |
| S01–S07 | 普通模式（APPWINDOW、无 owner、`HTCAPTION` / `HTLEFT`、拖动标题栏移动且落库）→ 点锁按钮经典固定（TOOLWINDOW、不可拖动缩放、`is_fixed` 落库）→ `fixed_embed_desktop=true` 重启恢复为嵌入桌面（owner=桌面宿主、TOOLWINDOW、无 MINIMIZEBOX）→ 探针窗口验证 Z 序 → Win+D → 重启 Explorer 自动重挂 → 点锁按钮回普通模式 | 是 |
| N01 | 3 条应用内通知：TOOLWINDOW\|TOPMOST、不抢焦点、主显示器工作区右下（边距 20）、自下而上叠放（间距 10） | 是 |
| I00–I08 | FAB 新建 → 输入 → Ctrl+Enter 落库、主窗口刷新；点行 → 只读详情 → 编辑 → 改标题保存；悬停操作条完成 / 删除（确认框）；设置切深色主题（落库、主窗口变半透明深色并透出桌面色）；设置开机自启开 / 关（`HKCU\…\Run\mini-todo`）；托盘图标（`Shell_NotifyIconGetRect`）；系统通知（到期提醒 notified=1、日志无发送失败） | 是 |
| D00–D04 | 本地 WebDAV（Basic 认证）：测试连接（服务器收到带认证的 PROPFIND）、保存配置（`webdav_password` 为 `dpapi:` 密文、库文件无明文）、立即同步（服务器出现 `mini-todo/sync-data.json.gz`、`webdav_last_sync_at` 更新）、标题栏同步 | 是 |
| F01–F06 | `fence:new`：嵌入桌面、收纳文件夹建在 `MINI_TODO_FENCES_ROOT` 下；放文件 → 监听刷新出图标；双击标题改名 → 文件夹同步改名；真实 OLE 拖入 → 文件被移进收纳文件夹；Win+D 后可见；Explorer 重启后重挂 | 是 |
| P01–P03 | **不隔离**：主窗口、日志在 `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`、库在 `%LOCALAPPDATA%\mini-todo`（release 另验证忽略 `MINI_TODO_DATA_DIR` / `MINI_TODO_DEBUG_OPEN`）；单实例（第二个进程退出、已运行实例被唤起到前台）；WM_CLOSE 关主窗口 → 进程退出 | 否 |

## 本地运行（自己的 Windows 机器）

前置：Rust（MSVC）、Python 3.10+、`pip install pillow wsgidav cheroot`；屏幕不锁定、前台不是管理员窗口
（否则模拟输入会被丢弃）；**中文输入法切到英文**；测试期间不要动鼠标键盘。

```powershell
cd pc-gpui
cargo build -p mini-todo
cargo build -p mini-todo-core --example seed
# 可选：分辨率 1920x1080 + 纯色桌面（会改你的桌面背景）
# pwsh -NoProfile -File scripts/e2e-windows/set-display.ps1
python scripts/e2e-windows/run_e2e.py --exe target/debug/mini-todo.exe --seed target/debug/examples/seed.exe `
  --out $env:TEMP\mt-e2e-out --only W,S,N,I,D,F
```

- 不加 `--allow-real-profile` 时 P 组跳过：P 组不隔离，会使用真实的 `%LOCALAPPDATA%\mini-todo` 数据目录、与正在运行的
  安装版冲突（单实例会把它唤起）。只在一次性虚拟机 / 没装正式版的机器上加。
- S06 / F06 会 `taskkill explorer.exe` 再拉起；I06 会写入再删除 `HKCU\…\Run\mini-todo`（与安装版共用的值名，
  已开启开机自启的机器上跑完需要重新打开）。
- `--only S` 只跑某几组；`--emit-shots W01,I05` 把这些用例的截图（缩小的 JPEG）以 base64 打进日志；
  失败截图默认最多打 2 张（`--emit-failures`）。截图、各实例日志在 `--out` 目录。
- release 冒烟：`cargo build --release -p mini-todo`，然后 `--exe target/release/mini-todo.exe --suite release --allow-real-profile`。

# GPUI 版 Windows 端到端测试

对着**真实的** GPUI 窗口做验证：真实鼠标键盘输入（`SetCursorPos` + `SendInput`）、Win32 只读断言（owner、ex style、
Z 序、`WM_NCHITTEST`、Win+D、Explorer 重启）、截图取色、读 `data.db`、注册表、真实 OLE 拖放。CI 在 GitHub 托管的
`windows-latest` 上跑（`.github/workflows/pc-gpui-e2e-windows.yml`，无 GPU 时 GPUI 走 D3D11 WARP 软件渲染）。

GPUI 窗口没有 DOM / CDP，按钮位置按布局常量（逻辑像素，见 `crates/app/src/windows/**`）点击；会随内容移动的目标
（列表行的颜色圆点、设置页开关、输入框边框、蓝色按钮）先截图按颜色识别再点。

| 文件 | 作用 |
|---|---|
| `run_e2e.py` | 用例与汇总（PASS / FAIL / SKIP、`summary.md`、`results.json`、`GITHUB_STEP_SUMMARY`），任一 FAIL 退出码 1 |
| `winlib.py` | ctypes Win32 封装：窗口枚举与状态、输入、截图取色、应用进程（隔离数据目录）、SQLite、托盘菜单命令 id、用例汇总 |
| `selftest.py` | 不碰窗口的纯逻辑自测（托盘命令 id 解析、按清单取未完成待办）：`python selftest.py` |
| `set-display.ps1` | 分辨率调到 1920×1080（不支持时取最接近的较小模式）、桌面换成纯色 `rgb(150,40,170)`（透明窗口取色对照） |
| `probe-window.ps1` | 探针：普通应用窗口（WinForms），验证嵌入桌面的窗口在它之下 |
| `drag-source.ps1` | OLE 拖放源窗口：`-Mode winforms`（.NET `DataObject` FileDrop）/ `-Mode shell`（`IShellFolder::GetUIObjectOf` 的数据对象） |
| `uia-find.ps1` | UI Automation 找元素矩形（资源管理器窗口里的列表项等） |
| `taskbar-button.ps1` | UI Automation 找应用的任务栏按钮矩形：Win11 XAML 任务栏（`Taskbar.TaskListButtonAutomationPeer`，名字 `mini-todo - 1 个运行窗口` / AutomationId `Appid: <AUMID>`）与 Win10 / Server 经典任务栏（`MSTaskListWClass` 下、名字为窗口标题），不匹配通知区托盘图标；文件带 UTF-8 BOM（Windows PowerShell 5.1 按 ANSI 代码页读无 BOM 脚本，中文注释可能吞掉换行） |
| `dav_server.py` | 本地 WebDAV（wsgidav + cheroot，Basic 认证，JSONL 访问日志） |
| `emit_annotations.py` | 把失败详情与 `<out>/emit/*.png` 截图作为 GitHub 注解输出（见下文「取回结果」） |
| `uia-dump.ps1`、`emit-image.ps1` | 诊断：打印窗口的 UIA 树 / 把图片以 base64 打进日志 |

## 用例

| 组 | 内容 | 隔离 |
|---|---|---|
| W01–W14 | `MINI_TODO_DEBUG_OPEN` 打开 main（默认清单窗口）/ editor:1:view / editor:1:edit / editor:new / subtask:101:view / settings 五个页 / completed / notification:1 / calendar（日历窗口，系统标题「日历」596×600）/ list:new（新清单窗口，标题「清单」380×600）：窗口存在、类名 `Zed::Window`、无 `WS_SYSMENU` 且 DWM 标题栏按钮区（`DWMWA_CAPTION_BUTTON_BOUNDS`）宽度为 0、逻辑尺寸、已渲染、截图；默认清单窗口四角透出桌面（DirectComposition 透明） | 是 |
| S01–S08 | 默认清单窗口普通模式（APPWINDOW、无 owner、没有 `WS_THICKFRAME`、标题栏 `WM_NCHITTEST` = `HTCLIENT`、外框外沿不是拖动 / 缩放区；真实拖动标题栏移动（自绘）、拖右边手柄缩放，`window_layouts[list:1]` 与 `screen_configs` 落库；双击标题栏**不**最大化、几何不变）→ 点锁按钮经典固定（TOOLWINDOW、`WM_NCHITTEST` 不是拖动 / 缩放区、真实拖动与拖边缘都不动、`window_layouts.is_fixed = 1`、`settings.is_fixed` 不变）→ `fixed_embed_desktop=true` 重启恢复为嵌入桌面（owner=桌面宿主、TOOLWINDOW、无 MINIMIZEBOX）→ 探针窗口验证 Z 序 → Win+D → 重启 Explorer 自动重挂 → 点锁按钮回普通模式（`is_fixed = 0`、真实拖动又能移动）→ 单击任务栏按钮（`taskbar-button.ps1` 找按钮、真实点击）最小化、再单击还原到前台（找不到按钮 / 按钮合并了多个窗口时 SKIP）。S01 / S02 / S03 / S07 / S08 每次切换后都断言无 `WS_SYSMENU`、标题栏按钮区宽度为 0 | 是 |
| N01–N02 | 3 条应用内通知：TOOLWINDOW\|TOPMOST、无 `WS_SYSMENU` 且标题栏按钮区宽度为 0、不抢焦点、右对齐、自下而上叠放（间距 10）；N02 单独检查客户区距工作区底边正好 20 | 是 |
| I00–I08 | FAB 新建 → 输入 → Ctrl+Enter 落库（`list_id = 1`）、列表刷新；点行 → 只读详情 → 编辑 → 改标题保存；悬停操作条完成 / 删除（确认框）；设置切深色主题（按窗口：`window_prefs[list:1].dark = 1`、`text_theme` 不变，默认清单窗口变半透明深色并透出桌面色），再用标题栏日 / 夜按钮切回；设置开机自启开 / 关（`HKCU\…\Run\mini-todo`）；托盘图标（`Shell_NotifyIconGetRect`）；系统通知（到期提醒 notified=1、日志无发送失败） | 是 |
| D00–D04 | 本地 WebDAV（Basic 认证）：测试连接（服务器收到带认证的 PROPFIND）、保存配置（`webdav_password` 为 `dpapi:` 密文、库文件无明文）、立即同步（服务器出现 `mini-todo/sync-data.json.gz`、`webdav_last_sync_at` 更新）、标题栏同步 | 是 |
| F01–F11 | `fence:new`：嵌入桌面（含无 `WS_SYSMENU`、标题栏按钮区宽度为 0）、收纳文件夹建在 `MINI_TODO_FENCES_ROOT` 下；放文件 → 监听刷新出图标；新实例第一次 OLE 拖入；双击标题改名 → 文件夹同步改名；改名后 / 单击标题后紧接着的拖入；Shell 数据对象拖入；从资源管理器窗口真实拖入；可见区域 = 数据库几何；Win+D 后可见；Explorer 重启后重挂 | 是 |
| F12–F15 | 另起一个实例，库里预置两个收纳箱 A / B（主显示器工作区右侧；默认清单窗口先 WM_CLOSE 隐藏，免得挡住、也不当吸附目标）：F12 拖 B 的标题栏（自绘拖动）→ 窗口跟手、松手后 `fences` 与当前组合的 `fence_layouts` = 客户区；F13 让 B.top 落在 A.top + 6（逻辑像素）→ 吸到 A.top，拖动中 A 的上边截图有 `#409EFF` 高亮、松手后消失；F14 拖到一半按住 Alt → 不吸附、不高亮，松开后照常吸附；F15 拖 B 的下边让高度接近 A → 等高、上边不动，A 的左右两边高亮 | 是 |
| L01–L11 | 待办清单窗口与日历窗口（3.1）：托盘「新增待办清单」→ 新窗口 380×600、普通模式、`todo_lists` 多一行「清单」→ 直接改名（标题与库）；双击清单名改名；待办行右键「移动到 ▸」→ `list_id` 落库、两个窗口各自刷新；拖动落库 `window_layouts`、清单 ↔ 清单吸附（top 落在默认清单 top + 6 → 吸齐，默认清单上边高亮、松手消失）；锁定按窗口（新清单 TOOLWINDOW、`is_fixed = 1`、拖不动，默认清单不变）；两个窗口 WM_CLOSE → 隐藏、进程常驻、`hidden = 1` → 托盘左键全部唤起；托盘「重置位置」→ 隐藏的出现、解锁、层叠错开 32、日历（关着）只写布局在第 3 个位置；删除清单 → 确认 → 窗口销毁、清单墓碑、窗口记录删除、待办回默认清单；托盘「显示日历」→ 596×600 出现在重置写好的位置；锁日历 → 设置里开「嵌入桌面」→ owner=桌面宿主、Win+D 后仍可见；关闭日历 = `hidden = 1`，托盘左键不唤起它 | 是 |
| P01–P03 | **不隔离**：默认清单窗口、日志在 `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`、库在 `%LOCALAPPDATA%\mini-todo`（release 另验证忽略 `MINI_TODO_DATA_DIR` / `MINI_TODO_DEBUG_OPEN`）；单实例（第二个进程退出、已运行实例被唤起到前台）；WM_CLOSE 默认清单窗口 → 只隐藏、进程常驻 → 托盘「退出」→ 进程退出 | 否 |

托盘菜单项（L 组、P03）不弹菜单：往托盘的隐藏窗口（类名 `tray_icon_app`）Post `WM_COMMAND`，命令 id 是 muda 按 `services/tray.rs`
建菜单的顺序从 1000 起分配的（含 `Menu` 本身与分隔线；`winlib.parse_tray_menu_ids` 按源码现算，读不到时用验证过的表）；托盘左键 Post
tray-icon 的回调消息 `WM_USER_TRAYICON`（6002，lParam = `WM_LBUTTONUP`）。两者都不需要托盘图标可见（CI 上图标常在溢出区）。
`python selftest.py` 自测这些不碰窗口的解析逻辑。

release job 只跑 P 组（加上 release 构建本身：`build.rs` 用 fxc 预编译着色器）。

## 本地运行（自己的 Windows 机器）

前置：Rust（MSVC）、Python 3.10+、`pip install pillow wsgidav cheroot`；屏幕不锁定、前台不是管理员窗口
（否则模拟输入会被丢弃）；**键盘布局切到 en-US（Win+Space）**——只把中文输入法切到"英"模式不够：输入法的中 / 英
状态按窗口记，新开的测试窗口仍是中文模式，键入的文字会被篡改（`E2E created todo` → `eEcreatedtodo`、数字被吞）、
之后的点击也可能切不走焦点，I01–I04 / D01–D04 / F03 / F10 会连锁失败（ENV 行的 `kbd=` 应为 `0x04090409`）；
测试期间不要动鼠标键盘。

```powershell
cd pc-gpui
cargo build -p mini-todo
cargo build -p mini-todo-core --example seed
# 可选：分辨率 1920x1080 + 纯色桌面（会改你的桌面背景；透明取色用例依赖背景不是纯黑 / 纯白）
# pwsh -NoProfile -File scripts/e2e-windows/set-display.ps1
python scripts/e2e-windows/run_e2e.py --exe target/debug/mini-todo.exe --seed target/debug/examples/seed.exe `
  --out $env:TEMP\mt-e2e-out --only W,S,N,I,D,F,L
```

- 开始时会把屏幕上其它程序的窗口**最小化**（避免挡住桌面、干扰取色与 Z 序断言）。
- 不加 `--allow-real-profile` 时 P 组跳过：P 组不隔离，会使用真实的 `%LOCALAPPDATA%\mini-todo` 数据目录、与正在运行的
  安装版冲突（单实例会把它唤起）。只在一次性虚拟机 / 没装正式版的机器上加。
- S06 / F06 会 `taskkill explorer.exe` 再拉起；I06 会写入再删除 `HKCU\…\Run\mini-todo`（与安装版共用的值名；
  I 组开始前记下原值、收尾时恢复原样，已开启开机自启的机器上跑也不会丢；不想动它就 `--skip I06`）；F08 会打开一个资源管理器窗口。
- S08 会真实点击任务栏按钮。安装版与开发版的 AppUserModelID 相同，安装版也开着普通模式的主窗口时两者合并成一个任务栏按钮
  （`mini-todo - 2 个运行窗口`，单击弹缩略图而不是最小化），这时 S08 记 SKIP；任务栏自动隐藏、找不到按钮时同样 SKIP。
- **安装版有收纳箱时先把它藏起来再跑 F 组**：测试收纳箱建在光标所在显示器右上角的空位，常与安装版的收纳箱重叠，外部拖入会落进
  安装版的收纳箱（测试文件被移进真实的 `%USERPROFILE%\MiniTodo收纳箱\`），F04 / F08 / F09 / F11 / F05 随之失败。做法见
  `.trellis/spec/guides/desktop-window-e2e-guide.md`「开发版与安装版并存」第 7 条。
- `--only S` 只跑某几组；`--skip I06,F08` 按用例 ID 跳过（记 SKIP）；`--emit-shots F08,N01` 把这些用例的截图另存到
  `<out>/emit/`；截图、各实例日志在 `--out` 目录。
- W01（主窗口四角透出桌面）、I05（深色半透明透出桌面色）拿启动前桌面取样的众数色做对照：桌面是照片壁纸时众数没有意义，
  这两条会 FAIL（不是产品问题，看 `W01-*.png` / `I05-main-dark.png` 能直接看到壁纸透出）；要它们有效就先跑 `set-display.ps1`。
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

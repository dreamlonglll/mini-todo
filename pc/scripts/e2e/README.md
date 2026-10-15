# 主窗口模式 e2e 工具（Windows）

针对固定模式 / 嵌入桌面等"改窗口样式、owner、Z 序"的改动，单元测试看不到 Win32 状态，
只能对着真实窗口断言。这组脚本把常用断言与输入动作固化下来；使用方法与验收项见
`.trellis/spec/backend/window-modes.md` §6 与 `.trellis/spec/guides/desktop-window-e2e-guide.md`。

## 前置

```powershell
cd pc
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'
npm run tauri dev
```

与正在运行的安装版并存时，**必须**带上开发专用数据隔离（仅 debug 构建生效，release 忽略）：

```powershell
$env:MINI_TODO_DATA_DIR = '<临时目录>\data'          # 库 / 图片 / 日志 / WebView2 用户数据全部落到这里，且不注册单实例插件
$env:MINI_TODO_FENCES_ROOT = '<临时目录>\fences'     # 收纳根目录（文件层）
# 可选：模拟"屏幕组合变化"（收纳箱按屏幕组合记忆位置，PRD R5）。文件内容非空时替代真实的屏幕组合 ID，
# 改写文件后约 2 秒内轮询线程按新 ID 走运行时切换路径；清空文件即回到真实 ID。同样只在 debug 构建生效
$env:MINI_TODO_SCREEN_ID_FILE = '<临时目录>\screen-id.txt'
```

用户正在前台操作（例如玩游戏）时不要用 `mouse.ps1` / 真实拖放，模拟输入会打断用户；页面内的点击、拖动、
滚轮、右键可改用 CDP 的 `Input.dispatchMouseEvent`（只作用于该 WebView，不动真实光标），外部文件拖入可用
`plugin:event|emit_to` 给 `fence-{id}` 发 `tauri://drag-drop`（payload `{ paths, position }`）。

脚本只认路径匹配 `target\debug\mini-todo.exe` 的进程，不会碰安装版；`cdp.mjs` 的 9222 端口只属于带了
`--remote-debugging-port` 的开发版 WebView2。实测期间不要切换托盘「开机自启动」（注册表项与安装版共用）。
前台是管理员权限窗口（如部分游戏客户端）或屏幕已锁定时，`mouse.ps1` / `win-d-key` 的模拟输入会被系统丢弃。

`--remote-debugging-port` 让 `cdp.mjs` 能通过 WebView2 的 CDP 直接在页面里 `click()`，
绕开真实鼠标"首次点击只激活窗口"的问题。

## 脚本

| 脚本 | 作用 | 典型用法 |
|---|---|---|
| `e2e.ps1` | 只读 Win32 断言 + Shell 动作。`-Step state\|json\|zorder\|win-d\|win-m\|click\|shot\|restart-explorer\|wait-window\|kill-app` | `pwsh -NoProfile -File e2e.ps1 -Step json` |
| `cdp.mjs` | 在某个 WebView 页面里执行 JS。`node cdp.mjs [--list] [--target '#/settings'] "<js>"`；默认目标是主窗口 | `node cdp.mjs "[...document.querySelectorAll('button[title]')].map(b => b.title)"` |
| `mouse.ps1` | 绝对屏幕坐标（物理像素）的真实鼠标 / 键盘：`-Action click\|dblclick\|drag\|wheel\|keys\|fg\|hit` | `pwsh -NoProfile -File mouse.ps1 -Action click -X -1518 -Y 335` |
| `win.ps1` | 枚举本进程可见顶层窗口：hwnd、rect、客户区屏幕原点、owner、zIndex、是否前台 | `pwsh -NoProfile -File win.ps1 -Title '新建待办'` |
| `dbq.py` | 读 `%LOCALAPPDATA%\mini-todo\data.db`；无参数打印模式设置 / screen_configs / 迁移版本 | `python dbq.py "select key,value from settings where key='is_fixed'"` |
| `fence.ps1` | 桌面收纳箱窗口：`-Step list\|json\|zorder\|win-d\|win-d-key\|win-m\|restart-explorer\|memory\|probe-start\|probe-state\|probe-stop`。列出主窗口 + 全部收纳箱的 owner / ex style / zIndex；Win+D / Win+M 期间逐窗口观察；Explorer 重启后逐个确认重挂；统计开发版进程树（含 WebView2）的私有内存 | `pwsh -NoProfile -File fence.ps1 -Step list` |
| `probe-window.ps1` | 探针：一个普通应用窗口（WinForms），验证收纳箱不盖住应用窗口、两次 Win+D 后应用窗口能恢复；由 `fence.ps1 -Step probe-start/probe-stop` 启停 | `pwsh -NoProfile -File fence.ps1 -Step probe-start -X 2600 -Y 100` |

## 坐标换算

CSS 坐标（CDP `getBoundingClientRect`）→ 屏幕物理坐标：
`screen = clientOrigin(win.ps1) + round(css × devicePixelRatio)`。主窗口无边框，客户区原点 = 窗口左上角；
编辑窗口有 1px 边框，客户区原点 = 窗口左上 + (9, 1)。

## 注意

- `restart-explorer`（`e2e.ps1` 与 `fence.ps1` 都有）会 `Stop-Process explorer`，先征得用户同意。
- `kill-app` 会让 `tauri dev` 连同 vite 一起退出；重启验证要先单独 `npm run dev` 再直接运行
  `src-tauri\target\debug\mini-todo.exe`（dev 构建无 `custom-protocol`，仍从 :1420 加载）。
- 传给 `cdp.mjs` 的 JS 里不要出现 `$` 与转义双引号（PowerShell 会插值 / 转义出错）。
- `SendKeys` 会经过系统输入法，中文输入法下文字会被改写；自动化输入请先切英文。
- 做完把用户的模式切回去（日常用固定模式），测试待办通过 Pinia store 删除，不要直接改库。

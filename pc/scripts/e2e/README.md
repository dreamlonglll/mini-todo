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

## 坐标换算

CSS 坐标（CDP `getBoundingClientRect`）→ 屏幕物理坐标：
`screen = clientOrigin(win.ps1) + round(css × devicePixelRatio)`。主窗口无边框，客户区原点 = 窗口左上角；
编辑窗口有 1px 边框，客户区原点 = 窗口左上 + (9, 1)。

## 注意

- `restart-explorer` 会 `Stop-Process explorer`，先征得用户同意。
- `kill-app` 会让 `tauri dev` 连同 vite 一起退出；重启验证要先单独 `npm run dev` 再直接运行
  `src-tauri\target\debug\mini-todo.exe`（dev 构建无 `custom-protocol`，仍从 :1420 加载）。
- 传给 `cdp.mjs` 的 JS 里不要出现 `$` 与转义双引号（PowerShell 会插值 / 转义出错）。
- `SendKeys` 会经过系统输入法，中文输入法下文字会被改写；自动化输入请先切英文。
- 做完把用户的模式切回去（日常用固定模式），测试待办通过 Pinia store 删除，不要直接改库。

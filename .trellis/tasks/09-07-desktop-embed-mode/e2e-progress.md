# 桌面模式 e2e 记录（2026-09-12 开始，2026-09-13 全部通过）

实现（`trellis-implement`）与质量检查（`trellis-check`）均已完成、门禁全绿（`cargo check` 0 warning /
`cargo test` 30 passed / `npm run build` 通过），代码**未提交**。本文记录 e2e 的环境、手法与全部结果。

## 环境与工具

| 项 | 值 |
|---|---|
| 被测实例 | `pc/src-tauri/target/debug/mini-todo.exe`（`npm run tauri dev` 起的 dev 实例，vite 在 :1420） |
| 启动方式 | 在 `pc/` 下：`$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS='--remote-debugging-port=9222'; npm run tauri dev` |
| 重启验证时的启动方式 | `kill-app` 会让 `tauri dev` 连 vite 一起退出；单独 `npm run dev` 起 vite，再带同一环境变量直接运行 `target\debug\mini-todo.exe`（dev 构建无 `custom-protocol` feature，仍加载 :1420） |
| DOM 驱动 | `research/cdp.mjs`：`node cdp.mjs [--list] [--target '#/editor'] "<js>"`，通过 WebView2 CDP 直接 `click()` 标题栏按钮，绕开"首次点击丢失"。**JS 里不要出现 `$` 与转义双引号**（PowerShell 会插值 / 转义出错）；访问 Pinia 用 `Object.keys(gp).find(k => /pinia/.test(k))` |
| Win32 断言 | `research/e2e.ps1 -Step state\|json\|zorder\|win-d\|win-m\|click\|shot\|restart-explorer\|wait-window\|kill-app` |
| 绝对坐标鼠标/键盘 | `research/mouse.ps1 -Action click\|dblclick\|keys\|wheel\|drag\|fg\|hit -X -Y [-X2 -Y2] [-Delta] [-Text]`（物理像素） |
| 枚举本进程窗口 | `research/win.ps1 [-Title '新建待办']`：hwnd / rect / 客户区屏幕原点 / owner / zIndex / 是否前台 |
| 模式切换循环 | `research/loop.ps1 -Rounds 5 -GapMs 1200`：桌面 → 普通 → 固定 → 桌面（偶数轮桌面 → 固定直切 → 普通 → 桌面），每步断言 Win32 + 前端按钮 + DB |
| 读库 | `research/dbq.py ["<sql>"]`（无参数时打印模式设置 / screen_configs / 迁移版本） |
| CSS → 屏幕坐标 | `screen = clientOrigin + round(css * devicePixelRatio)`；主窗口 dpr 1.25、客户区原点 = 窗口左上角（无边框）；编辑窗口有 1px 边框，客户区原点 = 窗口左上 + (9,1) |
| 数据库 | dev 实例与安装版共用 `%LOCALAPPDATA%\mini-todo\data.db`；备份 `data.db.bak-before-v27` 保留 |
| 标题栏按钮 title | `桌面模式` / `退出桌面模式`；`固定窗口` / `取消固定`；FAB 为 `button.fab-add`；待办操作按钮 `.complete-btn`（hover 后 `.todo-actions` 才 display:flex） |
| 机器 | Win11 25H2 build 26200；副屏 2560x1600@125%（主窗口所在，负坐标），主屏 1920x1080@100% |

## 验收结果（全部通过）

| 验收项 | 结果 |
|---|---|
| 固定模式 → 点「桌面模式」互斥切换 | 前端按钮变为「固定窗口」+「退出桌面模式」；`body.fixed-mode=false`；`data-tauri-drag-region=false`；DB `is_desktop=true / is_fixed=false`，`screen_configs.is_desktop=1 / is_fixed=0` |
| 进入桌面模式的 Win32 状态 | `owner=Progman`、`zIndex=1`（紧贴 Progman 之上）、`WS_MINIMIZEBOX` 去掉、`WS_EX_TOOLWINDOW` 在 / `WS_EX_APPWINDOW` 无、`WS_EX_TOPMOST` 无、DPI 120 不变、矩形不变 |
| Win+D（`Shell.ToggleDesktop`）采样 1.5s | `everIconic=False`、`everHidden=False`、矩形全程不变、`zIndexAfter=1`、窗口中心 `WindowFromPoint` 命中主窗口；截图 `research/e2e-10-win-d-desktop-shown.png`：壁纸透出、圆角保留、无黑/白底 |
| 再次 Win+D 恢复 / Win+M + Undo | 状态与位置不变，仍 z[1]，未最小化、未被盖住 |
| 在任意普通窗口之下 | 在主窗口矩形上 Show 一个普通 WinForms 顶层窗口，`WindowFromPoint` 命中该窗口（盖住了主窗口），Z 序主窗口仍 z[1]；关掉后状态不变 |
| 真实鼠标点击主窗口 | 前台=主窗口，**Z 序仍 z[1]**（tao `ALWAYS_ON_BOTTOM` 钩子压住激活抬升） |
| 从主窗口打开编辑窗口 + 键盘输入 | CDP 点 FAB → `'新建待办'` 窗口 owner=主窗口、z 在主窗口之上；真实鼠标点标题输入框**第 1 次点击**即前台=编辑窗口、`activeElement=INPUT`、`document.hasFocus()=true`；SendKeys 后 value 非空（被系统中文 IME 改写成「饿e'desktop」，IME 行为，与窗口层级无关） |
| 编辑窗口保存后回到主窗口 | 点「创建」后编辑窗口关闭，主窗口成为前台但 Z 序仍 z[1]；新待办出现在列表末尾 |
| 桌面模式下真实鼠标：滚轮 | `.main-content` scrollTop 0 → 76（到底）→ 0 |
| 桌面模式下真实鼠标：拖拽排序 | 拖 `.color-dot`（Sortable forceFallback）把末尾待办拖到第一位，DOM 顺序与 DB `sort_order=0` 一致，未误开编辑窗口 |
| 桌面模式下真实鼠标：勾选完成 | 真实悬停显示 `.todo-actions` → 点 `.complete-btn` → 从未完成列表消失、DB `completed=1` |
| 桌面模式下真实鼠标：点开详情 | 点待办行 → `'待办详情'` 窗口（`#/editor?id=…&mode=view`）owner=主窗口、前台、z 在主窗口之上 |
| 退出桌面模式 → 普通顶层窗口 | `owner=0`、`WS_MINIMIZEBOX` 回来、`WS_EX_TOOLWINDOW` 去掉、`zIndex=13`（普通顶层）、矩形不变；前端按钮「固定窗口」+「桌面模式」；DB 两键均 false |
| 桌面 ⇄ 普通 ⇄ 固定 5 轮循环（含桌面 → 固定直切） | `loop.ps1`：15 步全部 ok，hwnd 不变、进程存活、矩形 15 步全程不变，前端按钮 / DB 每步一致 |
| Explorer 重启自动重挂 | `restart-explorer`：`Stop-Process explorer` 后 Progman 由 `0x10280` 变为 `0x20600`，**主窗口存活**，1.5s 采样时已重挂到新 Progman（`reattached within 0.0s`），z[1]、矩形不变；随后 Win+D 两次采样仍全部免疫。PRD Decision 第 7 条的回退方案不需要 |
| 重启应用后恢复桌面模式且位置正确 | `kill-app` → 直接运行 debug exe → 新进程 6s 后：`owner=Progman`、z[1]、无 MINIMIZEBOX、矩形 `(-1441,-215)-(-32,453)` 与 `screen_configs` 的 `(-1441,-215) 1409x668` 完全一致（负坐标副屏场景），前端按钮为桌面模式；之后 Win+D 两次采样免疫 |
| 任务栏无图标 / Alt+Tab 不出现 | `WS_EX_TOOLWINDOW` + 无 `WS_EX_APPWINDOW` 由 state 证明（与用户日常使用的固定模式同一机制），未另做人工查看 |
| 导出 → 导入、WebDAV 往返保留 `is_desktop` | 单元测试覆盖（`data.rs` 两个 test）；e2e 已确认切换时 `settings.is_desktop` 与 `screen_configs.is_desktop` 实时落库 |
| 托盘勾选与 TitleBar 一致 | 未做自动化（CheckMenuItem 状态不可从外部读取），留人工右键托盘看一眼 |

### 观察备注

* 主窗口矩形在 2026-09-13 18:00 前后从 `(-1428,-221)` 变为 `(-1441,-215)`（13x6 px），发生在 e2e 暂停期间，
  不是模式切换或重启引起（循环 15 步与重启前后矩形均不变）；DB 同步记录了新位置。
* "首次点击丢失"本次没有复现：编辑窗口与主窗口都是第 1 次真实点击即成为前台。
* 编辑窗口中 SendKeys 受系统中文 IME 影响：IME 组合状态未结束时，CDP 直接改 `input.value` +
  `dispatchEvent('input')` 会被 Element Plus 的 `isComposing` 拦截，最终保存的标题是 IME 提交的「饿」。
  自动化输入文字请先切英文输入法，或用 CDP `Input.insertText`。

## 收尾（2026-09-13 18:20 已完成）

1. 测试待办（「饿」，id 1779335661318676）已通过 Pinia `todoStore.deleteTodo` 删除，DB 确认 0 行。
2. 已通过 CDP 切回用户原本的**固定模式**：`settings.is_fixed=true / is_desktop=false`，
   `screen_configs(id=2).is_fixed=1 / is_desktop=0`；Win32 状态 `owner=0`、`WS_EX_TOOLWINDOW` 在、z[13]。
3. dev 实例、单独起的 vite 全部结束，1420 / 9222 端口已释放；e2e 顺手开的资源管理器窗口已关闭。
4. 备份 `data.db.bak-before-v27` 保留（库已带 v27 迁移，对安装版 2.3.9 无影响）。

## 2026-09-13 UX 调整后复验（设置开关「固定模式时，嵌入桌面中」）

用户在上面全部通过后决定：去掉独立「桌面模式」按钮，改成设置里的开关，开启时固定模式即嵌入桌面
（见 PRD "2026-09-13 UX 调整"）。改完后门禁：`cargo check` 0 warning / `cargo test` 30 passed /
`npm run build` 通过；本机库已手动对齐重写后的 v27（备份 `data.db.bak-before-embed-option`：
加 `fixed_embed_desktop` 键、删 `is_desktop` 键与 `screen_configs.is_desktop` 列）。

| 步骤 | 结果 |
|---|---|
| 标题栏按钮 | 只剩「固定窗口」，`Monitor` 按钮已无；托盘无「桌面模式」项 |
| 设置 → 常规出现开关 | 标签「固定模式时，嵌入桌面中」，说明含"需要 Windows 11 24H2 及以上"；`isWindows` 由 UA 判断 |
| 普通模式下打开开关 | DB `fixed_embed_desktop=true`，主窗口保持普通（owner=0、APPWINDOW 在）——只记偏好 |
| 开关开 + 点「固定窗口」 | owner=Progman、z[1]、无 MINIMIZEBOX、TOOLWINDOW；`body.fixed-mode=false`（保留圆角）；DB `is_fixed=true` |
| 上述状态 Win+D 两次 | 未最小化、未隐藏、z[1]、中心命中主窗口 |
| 固定中关掉开关 | 即时切回普通固定：owner=0、MINIMIZEBOX 回来、TOOLWINDOW 保持、z[14]；`body.fixed-mode=true`；按钮仍「取消固定」 |
| 固定中再打开开关 | 即时切回嵌入：owner=Progman、z[1]、无 MINIMIZEBOX |
| 嵌入态点「取消固定」 | 普通顶层窗口：owner=0、APPWINDOW 回来、可最小化、`thickFrame` 回来 |
| 再固定 → 杀进程 → 直接运行 exe | 新进程恢复为嵌入态（owner=Progman、z[1]、无 MINIMIZEBOX），矩形不变，按钮「取消固定」；Win+D 免疫 |

`loop.ps1` 仍按旧的三模式按钮写的，若要再跑循环需改成"固定 ⇄ 普通 + 设置开关"两条轴。

## 之后的 Trellis 流程

e2e 通过 → `trellis-update-spec`（tao `ALWAYS_ON_BOTTOM` 钩子 / owner=Progman 技巧 / CDP + Win32 e2e 手法值得进 spec）
→ 分批提交 → 版本号递增 → `/finish-work`。

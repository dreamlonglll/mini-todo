# 桌面收纳箱 e2e 记录（A 工作线：窗口层）

> 按 `.trellis/spec/guides/desktop-window-e2e-guide.md` 实测。环境、隔离方式见 `research/spike-windows.md` 开头。
> 工具：`pc/scripts/e2e/fence.ps1`（收纳箱窗口断言）、`e2e.ps1`（主窗口断言）、`cdp.mjs`（页面内操作 / 调命令）。
> 开发版始终带 `MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`（会话临时目录），只按 exe 路径
> `target\debug\mini-todo.exe` 定位进程；全程未操作安装版（PID 38132）的进程与窗口，未切换开机自启动。

## 0. 开发专用数据隔离

| 检查 | 结果 |
|---|---|
| 与安装版同时运行（单实例插件未注册，开发版没有被转交退出） | 通过 |
| 库 / 日志落在隔离目录（`<data>\data.db`、`<data>\logs\`），日志首行提示隔离已启用 | 通过 |
| WebView2 用户数据目录 = `<data>\EBWebView\EBWebView`（逐个核对 msedgewebview2 的 `--user-data-dir`） | 通过：与安装版浏览器进程完全分开，CDP 9222 只看得到开发版页面 |
| 自启注册表项不被刷新成开发版 exe（setup 里的自启自愈在隔离时跳过） | 代码路径确认（未触碰托盘自启项） |
| release 构建忽略 `MINI_TODO_DATA_DIR`（`paths::data_dir_override(_, false) == None`） | 单测通过 |

## 1. 收纳箱窗口 Win32 状态（建窗 / 各操作之后）

所有收纳箱在建窗后与整个测试过程中保持：`ownerIsProgman=True, toolWindow=True, appWindow=False,
minimizeBox=False, thickFrame=False, topmost=False, noActivate=False`，DPI 与所在显示器一致（144 / 120）。

| 步骤 | 方式 | 结果 |
|---|---|---|
| 新建（`create_fence`，与托盘「新增收纳箱」同一实现） | CDP 调命令 | 通过：光标所在主屏右上角起从右往左排，不重叠；尺寸 552×504（368×336 DIP @150%）；标题 收纳箱 / 收纳箱 2 … |
| 新建时任务栏是否闪按钮 | UIA 轮询任务栏 4 s | 通过：未出现 |
| 折叠 / 展开（标题栏按钮） | CDP 点 DOM | 通过：折叠后窗口高 48 px（32 DIP），库里 collapsed 持久化、展开高度不变；展开回 504 |
| 悬停临时展开 / 移开收回 | CDP 派发 mouseenter / mouseleave | 通过：150 ms 后展开到 336 CSS px，离开 400 ms 后收回；库里 collapsed 不变 |
| 下方空间不够时向上展开 | 标题栏 y=2000（工作区底 2088） | 通过：展开为 (3252,1584)-(3804,2088)，收回仍在 y=2000 |
| 拖文件经过时临时展开 | CDP 发 `tauri://drag-enter/over/leave/drop` 给 `fence-2` | 通过：enter 展开 + 虚线高亮，leave / drop 300 ms 后收回（真实 OLE 拖动待真实输入复测） |
| 折叠态打开标题栏菜单 / 删除确认 | CDP | 通过：菜单打开时临时展开、菜单完整显示在窗口内，关闭后收回；删除确认面板同理 |
| 固定（锁定） | CDP 点锁按钮 | 通过：库里 locked 持久化，缩放手柄消失；Win32 样式不变（未改 tao flag） |
| 双击标题改名：Enter / Esc / 空标题 / 失焦 / 输入法 Enter | CDP | 通过：Enter 提交（去首尾空白、控制字符换空格），窗口标题同步；Esc 取消；空标题不提交；输入法组合中的 Enter 不提交。失焦提交在页面没有系统焦点时 `input.blur()` 不触发 blur 事件，已补"窗口失去焦点也提交" |
| 外观：预设色 / 透明度 | CDP | 通过：预设色立即生效并落库（#0F2942）；滑块拖动中只预览，松手落库（0.5） |
| 删除收纳箱 | CDP：菜单 → 删除 → 确认 | 通过：确认框写明「收纳箱 N」；窗口销毁、库里记录删除、嵌入登记注销；之后新建会补进空位 |
| 显示 / 隐藏所有收纳箱（与托盘同一实现） | CDP 调 `toggle_fences_visibility` | 通过：全部隐藏（zIndex -1）/ 全部显示，显示后仍是工具窗口；UIA 轮询 3 次切换未见任务栏按钮；ex style 采样见 spike §5 |
| 重置收纳箱位置（与托盘同一实现） | CDP 调 `reset_fence_positions` | 通过：全部排回主屏工作区右上角起的一行并落库；在 125% 副屏的收纳箱挪回 150% 主屏后尺寸、折叠高度正确 |
| 混合 DPI 折叠高度 | 收纳箱挪到 125% 副屏 | 修复后通过：副屏折叠 40 px、回主屏 48 px（第一版按窗口当前缩放计算，错成 48 / 40） |
| 进程重启恢复 | 结束开发版 → 重新启动 exe（vite 单独运行） | 通过：5 个收纳箱位置、尺寸、标题、折叠（48 px）、锁定、背景色 / 透明度全部恢复 |
| 启动夹回可见工作区 | 库里写 (9000, 5000) 再重启 | 通过：显示在主屏右下角 (3288,1584)，库里坐标不变（系统挪窗 / 启动夹回不落库） |
| 不盖住其它应用窗口 | 探针窗口与收纳箱重叠 | 通过：探针 zIndex 远高于收纳箱，探针中心命中探针自己；用户最大化的窗口同样盖住收纳箱 |
| Explorer 重启（第 1 轮，主窗口普通模式 + 5 个收纳箱） | `fence.ps1 -Step restart-explorer` | 通过：全部存活，第一次轮询内全部重挂到新 Progman |
| Win+D 第一次 | 真实 Win+D 按键（探针为普通应用窗口） | 通过：5 个收纳箱 + 嵌入态主窗口 everIconic/everHidden=False、rect 不变、中心命中自己；截图确认显示在桌面上；探针被最小化 |
| Win+D 第二次 | 同上 | 通过：探针恢复原位、原前台窗口重新获得焦点（NoFences #14 副作用未复现）；收纳箱 rect 不变 |
| Win+M | `fence.ps1 -Step win-m` | 通过：收纳箱与嵌入态主窗口不受影响；之后 `UndoMinimizeALL()` 恢复了用户窗口 |
| 真实拖动标题栏移动 | `mouse.ps1 -Action drag` | 通过：松手后落库 (1809,226)，与窗口一致；锁定的收纳箱拖不动、位置不变 |
| 真实缩放（右下角 / 左上角手柄） | `mouse.ps1 -Action drag` | 第一版不通过（`startResizeDragging` 对无 `WS_THICKFRAME` 的窗口无效），改为指针捕获 + `set_fence_bounds` 后通过：652×604 / 回到 552×504 且左上角移动，均落库，嵌入样式不变 |
| 重叠后真实点击下面的收纳箱 | `mouse.ps1 -Action click` + `fence.ps1 -Step zorder` | 通过：被点的升到桌面层组内最上，其余相对顺序不变（无成组重排），普通应用窗口仍在其上 |
| 真实悬停 | 光标移到折叠收纳箱标题栏 / 移入内容区 / 移出 | 通过：展开到 504 px、内容区内保持展开、移出后收回 48 px |
| 真实 OLE 拖文件经过折叠的收纳箱 | WinForms 拖动源（临时文件，`CF_HDROP`） | 通过：离开收回 48 px、再进入展开 504 px，放下后光标移开即收回；源文件未受影响 |
| 真实双击标题改名 / 点桌面失焦提交 | `mouse.ps1 -Action dblclick` / click 桌面 | 通过：输入框获得焦点；点到桌面后标题提交为"真实失焦" |
| 真实点击未激活收纳箱上的折叠按钮 | `mouse.ps1 -Action click` | 通过：第一次点击即生效，折叠持久化 |
| 托盘「新增收纳箱」「显示/隐藏所有收纳箱」「重置收纳箱位置」 | UIA 找到开发版托盘图标（提示"Mini Todo（开发版）"）右键，真实点击菜单项 | 通过：三项位于「添加待办项」之后；新建到空位、全部隐藏 / 显示、全部排回主屏并落库；未点击「开机自启动」，注册表 Run 项仍指向安装版 exe |
| Explorer 重启（第 2 轮，主窗口嵌入 + 5 个收纳箱） | `fence.ps1 -Step restart-explorer` | 通过：收纳箱与主窗口全部存活并在第一次轮询内重挂到新 Progman |

## 2. 主窗口三种模式回归（开发版）

| 模式 | 断言（`e2e.ps1 -Step json`） | 结果 |
|---|---|---|
| 普通 | owner=0, toolWindow=False, appWindow=True, minimizeBox=True | 通过 |
| 固定（经典） | owner=0, toolWindow=True, appWindow=False, minimizeBox=True | 通过 |
| 固定 + 嵌入桌面（`set_fixed_embed_desktop(true)` 后即时切换） | ownerIsProgman=True, zIndex=1, minimizeBox=False, toolWindow=True | 通过；与收纳箱同属 Progman owned 组，进入嵌入时落到 z[1] |
| 嵌入 → 经典固定 → 普通 | 同上表各行 | 通过 |
| 主窗口嵌入 + 收纳箱：Win+D ×2 / Win+M | 真实输入 | 通过（见 §1 Win+D / Win+M 行） |
| 主窗口嵌入 + 收纳箱：Explorer 重启 | `fence.ps1 -Step restart-explorer` | 通过 |
| 主窗口嵌入 + 收纳箱：进程重启 | 结束开发版 → 重新启动 | 第一次**不通过**：主窗口 owner=Progman 但 toolWindow=False / appWindow=True（启动竞态，见 spike §7）；轮询自愈修复后通过（约 200 ms 内恢复工具窗口样式） |
| 经典固定 + 收纳箱：进程重启 | 同上 | 通过（toolWindow=True, owner=0） |

## 3. 未覆盖 / 已知限制

- 渲染首帧白闪（dev 下 main.scss 由脚本注入，可能先显示 index.html 内联的白底）未逐帧录屏确认；release 预计不受影响
- 显示器热插拔（系统挪窗不落库）只验证了"启动夹回不落库"与代码路径（只有用户发起的拖动 / 缩放才落库），未实际拔插显示器
- 前台为管理员权限程序或锁屏时模拟输入会被丢弃，相关步骤需在用户离开、会话解锁时进行（已写进 e2e README）

## 4. 集成阶段（文件层接入 + 项目网格 + R5 按屏幕组合记忆位置）

> 2026-10-15。开发版 `target\debug\mini-todo.exe`，`MINI_TODO_DATA_DIR` / `MINI_TODO_FENCES_ROOT` / `MINI_TODO_SCREEN_ID_FILE`
> 均指向会话临时目录 `scratchpad\e2e-int\`；vite 单独运行；只按 PID / exe 路径操作开发版，安装版（PID 38132）全程未触碰，
> 未切换开机自启动，未重启 Explorer。桌面上只新建 / 操作 `__fence_e2e_` 开头的测试项。
>
> **输入方式说明**：实测期间用户一直在前台操作（`nshm` 游戏 / 资源管理器，`GetLastInputInfo` 空闲 ≈0 ms），为不打断用户，
> 页面内交互一律用 CDP `Input.dispatchMouseEvent` / `Input.dispatchKeyEvent`（只作用于该 WebView，不动真实光标），
> 外部拖入用 `plugin:event|emit_to` 给 `fence-{id}` 发 `tauri://drag-enter` + `tauri://drag-drop`（与 wry 收到 OLE drop 后
> 发出的事件相同，之后的移入走真实的 `IFileOperation`）。开始时做过一次真实 OLE 拖放（WinForms 拖动源），因游戏占用前台 /
> 捕获光标，drop 没有落到收纳箱（拖动源是我们自己的窗口，未影响任何桌面项），此后停止 OS 级模拟输入。
> 截图用 CDP `Page.captureScreenshot`（只截本页面）。

| 步骤 | 方式 | 结果 |
|---|---|---|
| 没有收纳箱时启动 | 启动开发版 | 通过：收纳根目录**未**被创建（监听在有收纳箱之后才开始） |
| 新建收纳箱（与托盘同一实现）×2 | CDP 调 `create_fence` | 通过：先建文件夹再插库，`收纳箱` / `收纳箱 2` 两个文件夹；同时写入当前屏幕组合的 `fence_layouts`；开始监听根目录 |
| 屏幕组合 ID 与前端一致（R5） | CDP 读主窗口 `appStore.currentScreenConfigId`，并在页面里按 `generateScreenConfigId()` 同样算法现算 | 通过：三者均为 `2_3840x2160@150_2560x1600@125`（枚举顺序 DISPLAY1 2560×1600 在前，按面积降序后 3840×2160 在前） |
| 外部拖入 5 项（txt ×2、文件夹、快捷方式、失效快捷方式） | `tauri://drag-drop` → `fence_move_in` | 通过：桌面上 5 项消失，出现在测试收纳根目录 `收纳箱\`；`fence_items` 5 行 original_path 均为桌面原路径（行由监听对账先补、upsert 再填 original_path，正是 upsert 要覆盖的竞态）；无提示（全部成功） |
| 项目网格 | CDP 读 DOM + 截图 | 通过：图标经 `asset.localhost` 从隔离目录 `<data>\fence-icons` 加载（48 × DPR 1.5 → 96 档），`.lnk` 显示名去后缀，悬停 title 为全名；修正一处布局：列间距改为 0，默认尺寸刚好 4 列 |
| 单击选中 / 自绘右键菜单 | CDP 鼠标 | 通过：选中高亮；菜单四项「打开 / 打开所在位置 / 移回桌面 / 移动到…」，夹在窗口内；Esc 关闭 |
| 移回桌面 | 右键 → 移回桌面 | 通过：文件回到桌面、内容不变，行删除，提示"已移回 C:\Users\12197\Desktop" |
| 重名移回不覆盖 | 桌面先新建同名 `__fence_e2e_a.txt`（不同内容）再移回 | 通过：移回的文件变为 `__fence_e2e_a (2).txt`，桌面原有同名文件内容未变 |
| 跨箱移动 | 右键 → 移动到… → 收纳箱 2 | 通过：文件夹移到 `收纳箱 2\`（含内部文件），目标窗口自动刷新，新行搬运了原始路径 |
| 从另一个收纳箱文件夹拖入（锁定状态下） | 收纳箱 3 锁定后，drop `…\常用工具\__fence_e2e_dir` | 通过：锁定不影响拖入；按跨箱处理，original_path 仍为桌面原路径，来源收纳箱的行删除、窗口刷新为空 |
| 资源管理器里放入 / 删除文件 | 直接在收纳文件夹里建 / 删文件 | 通过：约 400 ms 内收纳箱自动刷新；补的行 original_path = NULL；删除后行也删除；其他收纳箱的列表不受影响（事件只发给对应窗口） |
| 运行中删掉收纳文件夹 | 删除空收纳箱的文件夹 | 通过：监听对账按原名重建，日志记录 |
| 箱内拖动排序 | CDP 鼠标拖动（SortableJS force-fallback） | 通过：DOM 顺序与 `fence_items.sort_order` 一致；重启后保留 |
| Ctrl+滚轮图标大小 | CDP `mouseWheel` + Ctrl ×3 | 通过：48 → 72，400 ms 后落库；图标按 128 档重新请求；页面缩放不受影响（DPR 仍 1.5）；重启后保留 |
| 双击打开测试快捷方式 | CDP 双击 `__fence_e2e_launch.lnk`（cmd 写标记文件后退出，最小化启动） | 通过：标记文件生成，没有留下窗口 |
| 双击失效快捷方式 | CDP 双击 `__fence_e2e_broken.lnk` | 通过：立即进入"正在打开"状态（转圈 + title"正在打开「…」…"），随后错误提示"打开「__fence_e2e_broken」失败：快捷方式指向的目标不存在，可能已被移动或卸载"。本机这次 Shell 很快判定失败（目标所在文件夹已删），没有复现 spike 里的约 3 秒 |
| 双击标题改名 | CDP 双击 + `Input.insertText` + Enter | 通过：标题"常用 工具:新"，文件夹同步改为"常用 工具_新"，项目仍显示 |
| 改名时文件夹被占用 | 后台进程以不含 FILE_SHARE_DELETE 打开文件夹里的文件后改名 | 通过：标题照改、`folder_name` 保留旧值，警告"标题已修改，但收纳文件夹没能跟着改名：…正被其他程序占用"；释放后再改名成功 |
| 进程重启 | 按 PID 结束开发版 → 停机期间往收纳文件夹放一个文件 → 重新启动 | 通过：两个收纳箱、标题、排序、图标大小全部恢复；停机期间放入的文件出现在末尾（original_path NULL）；嵌入样式不变（ownerIsProgman / toolWindow） |
| 启动后清理图标缓存 | 日志 | 通过：启动 20 秒后清理 6 个过期缓存（已移走项、旧尺寸） |
| R5 用户缩放写当前组合 | CDP 拖动右下角缩放手柄（+60/+40 CSS） | 通过：`fences` 与当前组合的 `fence_layouts` 同时更新为 642×564 |
| R5 切换到没有记录的组合 | 改写屏幕组合模拟文件 | 通过：约 2 秒内检测到变化；窗口沿用当前位置并夹回可见区域（超出右边界的收纳箱被平移回来），**不落库**（两张表都未变） |
| R5 切换到有记录的组合 / 再切回 | 往库里插入另一组合的记录后来回切换 | 通过：有记录的收纳箱恢复到 (2000,200) 500×450，没有记录的保持原位；切回后恢复到原组合的记录；期间无任何写库 |
| R5 折叠态切换 | 折叠后来回切换 | 通过：两个组合下都是标题栏高度 48 px（目标显示器 150%） |
| R5 启动恢复 | 模拟文件为另一组合时重启 | 通过：有该组合记录的收纳箱用记录，没有的用"最近一次位置" |
| R5 重置位置写当前组合 | CDP 调 `reset_fence_positions`（与托盘同一实现） | 通过：`fences` 与当前组合 `fence_layouts` 同时更新 |
| R5 删除屏幕配置 | CDP 调 `delete_screen_config` | 通过：该组合的 `fence_layouts` 记录一并删除，其他组合不受影响 |
| 删除收纳箱（有项目） | 标题栏菜单 → 删除收纳箱 → 删除 | 通过：确认框"其中的 3 个项目会移回桌面"（实际列举数）；3 项全部回到桌面（含 original_path 为 NULL 的那项）；空文件夹删除；窗口关闭；`fence_items` / `fence_layouts` 级联删除 |
| 删除收纳箱（空） | 同上 | 通过：确认框"收纳箱是空的，收纳文件夹会一并删除"，文件夹删除、窗口关闭 |
| 只有一个收纳箱时的「移动到…」 | 右键 | 通过：禁用，显示"没有其他收纳箱" |
| 折叠态悬停展开 + 项目右键菜单 | CDP | 通过：菜单打开期间鼠标离开也保持展开，关闭菜单后收回 |

### 4.1 未覆盖 / 留给用户手动验证

- **公共桌面拖入的 UAC 取消路径**：会弹安全桌面，无法自动化；分类逻辑有单测。需用户手动：从 `C:\Users\Public\Desktop` 拖一个快捷方式进收纳箱，
  在 UAC / "需要管理员权限" 对话框点取消 → 应提示"已取消 1 项"，文件留在公共桌面
- **首次收纳的 OneDrive 提示**：本机桌面 `C:\Users\12197\Desktop` 未被 OneDrive 重定向，提示路径不会触发；"只提示一次"的标记逻辑有单测
  （`claim_onedrive_notice`），标记存 settings 键 `fence_onedrive_notice_shown`（v29 写默认值，不在同步键 / AppSettings 内）
- **删除收纳箱时有项目移不回去**（2026-10-15 用户决定：删除中止、整个收纳箱保留，提示"N 个项目未能移回桌面，收纳箱已保留"并列出原因）：
  要让 `IFileOperation` 失败只能占用文件或拒绝权限，两者都会弹 Shell 的"文件正在使用 / 需要权限"对话框等待用户选择，无法自动化；
  "是否保留"的判定抽成纯函数 `fence.rs::plan_delete` 并有单测，前端提示文案 `describeDeleteKept` 有单测。需用户手动：
  占用收纳箱里的某个文件（例如用程序打开不放）后删除收纳箱，在"文件正在使用"对话框选跳过 → 收纳箱应保留、只剩该项、提示原因
- 用户一直在前台，以下会打扰用户的步骤未做：双击打开 `.txt`（会拉起 Notepad-- 窗口）、「打开所在位置」（会打开资源管理器窗口）。
  命令本身与快捷方式启动共用同一实现（`ShellExecuteExW` / `SHOpenFolderAndSelectItems`）
- **真实 OLE 拖放未完成**：共尝试两次（WinForms 拖动源）。第一次用户在游戏中，drop 未落到收纳箱；第二次用户空闲约 4 分钟，
  把空收纳箱临时挪到副屏无窗口处（`set_fence_bounds`，不落库，结束后 `set_fence_view` 恢复；DB 未变，顺带确认了程序挪窗不落库），
  但拖动源脚本的辅助线程不是 per-monitor DPI aware，`SetCursorPos(-2300,1200)` 被虚拟化到 (0,1500)，按下落在用户最大化的
  `mini-term` 窗口左边缘，拖动没有开始；随即在原处补发了松开（左键状态由按下恢复为抬起）。为避免再误触用户窗口，停止了 OS 级输入测试。
  wry 的 OLE drop → `tauri://drag-drop` 投递由 A 线实测过（拖文件经过 / 放下），本阶段的拖入逻辑用同名事件 + 真实 `IFileOperation` 验证。
  建议用户手动从桌面拖一个文件进收纳箱确认一次
- release 构建的资源协议 scope（`$LOCALDATA/mini-todo/fence-icons/**`）：实测必须带数据隔离，走的是隔离目录的 `allow_directory`；release 路径只做了配置核对
- 显示器真实插拔（R5）：用模拟文件走了同一条轮询 → 恢复路径；真实拔插时系统挪窗不落库的前提（前端闸门）沿用 A 的实现

### 4.2 删除语义调整后的回归（2026-10-15）

删除时有项目未能移回改为"保留整个收纳箱"（不删记录 / 布局 / 文件夹、不关窗口，已移回项删行并重新对账），
移除了只为旧分支服务的 `close_fence_window` / `fence_reveal_folder` 与"已删除"结果面板。隔离环境快速回归全部移回的路径
（CDP + `tauri://drag-drop`，无 OS 级输入）：新建收纳箱 → 拖入 2 个桌面测试文件 → 删除确认框"其中的 2 个项目会移回桌面" → 删除：
2 个文件回到桌面且内容不变、收纳文件夹删除、窗口关闭、`fences` / `fence_items` / `fence_layouts` 清空。通过。

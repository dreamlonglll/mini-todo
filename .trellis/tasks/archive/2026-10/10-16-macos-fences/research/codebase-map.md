# Research: macOS 收纳箱——代码地图（要改哪里、几何、e2e、mac-check、CI、文档）

- **Query**：在 `pc-gpui/` 里让桌面收纳箱在 macOS 上可用，需要改动的全部位置；mac 几何与收纳箱代码假设的差异；mac e2e 框架与新用例；
  `scripts/mac-check`；CI 与 core 的 mac 依赖；需要更新的文档
- **Scope**：internal（只读源码与 cargo registry 里的 gpui-pre-macos 0.3.9 / objc2-* 源码，未联网）
- **Date**：2026-10-16
- 行号以当前 HEAD `5ba6602` 为准；"建议"一栏是给实现者的候选做法，不是结论

---

## 0. 一句话现状

core 的文件夹 / 列举 / 命名 / 对账 / 监听全平台可用；**移动、打开、定位、图标**四个 Shell 能力在非 Windows 返回
`Err("桌面收纳箱仅支持 Windows")`；app 侧只有两个入口闸门（菜单栏图标三项 `cfg(windows)`、`restore_all` 的 `!cfg!(windows)`），
其余是"非 Windows 分支 = Linux 截图用近似实现"，在 mac 上会走到这些近似分支（只改尺寸不摆位置、不设层级）。
注意：`MINI_TODO_DEBUG_OPEN=fence:new` 在 mac 上**现在就能**建出收纳箱（`create_fence` 没有平台闸门），只是窗口在普通层级、
摆窗 / 文件操作不工作。

---

## 1. 平台闸门与非 Windows 行为清单

### 1.1 app：入口与生命周期

| 位置 | Windows 行为 | mac 现在 | mac 建议 |
|---|---|---|---|
| `crates/app/src/services/tray.rs:212-237` | `#[cfg(windows)]` 往菜单里加「新增收纳箱 / 显示/隐藏所有收纳箱 / 重置收纳箱位置」+ 分隔 | 不加（文件头注释 `tray.rs:3-5` 也写着"只在 Windows 上有"） | 改成 `cfg(any(windows, target_os = "macos"))`；`handle_menu`（`tray.rs:70-72`）与转发层 `mod fences`（`tray.rs:134-149`）本来就全平台 |
| `crates/app/src/lib.rs:85-89` | `windows::fence::restore_all(cx)`（注释"恢复收纳箱窗口（Windows）"） | 调用但 `restore_all` 内部直接 return | 注释改掉；实际闸门在下一行 |
| `crates/app/src/services/fences.rs:408-440` `restore_all` | `prepare_restore` → 逐个 `open_fence_window` → `ensure_watcher` → `items::spawn_startup_reconcile`（`:437`） | `:411` `if !cfg!(windows) { return; }`：不恢复、不对账、不清图标缓存 | 去掉闸门（Linux 仍不应恢复：Linux 没有托盘，`fence:<id>` / `fence:new` 只用于截图，可改成 `cfg!(any(windows, target_os = "macos"))`） |
| `services/fences.rs:42-44` `FenceWindowRecord.hwnd` | 嵌入守护 / 显示隐藏用 | `#[cfg_attr(not(windows), allow(dead_code))]` | mac 上会用到（显示隐藏、设层级），去掉 cfg_attr |
| `services/fences.rs:110-127` `apply_window_rect` | `platform::set_client_rect_physical`（延后执行、按客户区一次摆好） | `#[cfg(not(windows))]` 分支：**只** `window.resize(rect / scale)`，不移动；`scale` 有时是 `MonitorArea.scale`（mac = 统一系数）、有时是 `window.scale_factor()`（缩放会话里的 `dpr`） | mac 走 `platform::set_client_rect_physical`（`platform/macos/mod.rs:83-87` → `raw::set_client_rect` `:271-280` → `setFrame:display:`），即 `cfg(any(windows, target_os = "macos"))`；Linux 保留只改尺寸 |
| `services/fences.rs:159` `spec.show = !cfg!(windows)` | 先隐藏建好、嵌入后再 `SW_SHOWNOACTIVATE` | `show: true`：GPUI 建窗时 `orderFront:`（`gpui-pre-macos/src/window.rs` 建窗末尾 `if focus && show {makeKeyAndOrderFront} else if show {orderFront}`），此时层级是 `NSNormalWindowLevel`，会在普通窗口层闪一下、且应用不在前台时 `orderFront:` 的位置不确定 | mac 也 `show: false`，在 `embed_after_open` 里设好层级 / collectionBehavior 后再 `Hwnd::show()`（mac = `orderFrontRegardless`，`macos/mod.rs:211-214`）。`windows::open_window` 的 `reshow` 只在 `spec.show` 时做（`windows/mod.rs:285`），不冲突 |
| `services/fences.rs:210-239` `embed_after_open` | async 块里 `set_client_rect` → `set_tool_window` → `set_minimizable(false)` → `set_round_corners` → `embed::attach_to_desktop` → `set_always_on_bottom` → 不在"全部隐藏"时 `show()` | `#[cfg(not(windows))]` 空实现 | mac 分支：`set_client_rect(rect)` → 新增 `ns` 调用：`setLevel(kCGDesktopIconWindowLevel + 1)`、`setCollectionBehavior(CanJoinAllSpaces \| Stationary \| IgnoresCycle)`、`setCanHide(false)`、`setExcludedFromWindowsMenu(true)` → 不隐藏时 `show()`。全部在 `cx.spawn` 的 async 块里、`update` 之外（gpui-conventions §2.15"写操作同样要延后"） |
| `services/fences.rs:271-313` `ensure_guard` | 200ms：`#[cfg(windows)]` 块（`:282-306`）重挂 owner / TOOLWINDOW；每 10 tick 屏幕组合检测（`:307-309`，全平台） | 只做屏幕组合检测 | mac 没有 Explorer 重启这回事，可以不加守护；如果担心层级被改（GPUI 运行期不改 level），可在这里低频复核 `level` |
| `services/fences.rs:383-406` `set_all_visible` | `#[cfg(windows)]`：`Hwnd::show` + 重挂 / `Hwnd::hide` | 只改 `runtime.hidden`，**窗口不动** | mac 用 `Hwnd::show` / `hide`（`orderFrontRegardless` / `orderOut:`，`macos/mod.rs:211-218`），即同一段代码去掉嵌入两行，或 `cfg(any(..))` + 嵌入两行仍 `cfg(windows)` |
| `services/fences.rs:316-362` `check_screen_layout` | 读各窗口 `client_rect_physical` + 缩放 → `screen_switch` → `apply_targets` | 读几何可用（mac `client_rect_physical` = frame，`macos/mod.rs:158-160`），但 `apply_targets` → `apply_window_rect` 在 mac 只改尺寸 | 修 `apply_window_rect` 即可；`:337` 兜底缩放用 `window.scale_factor()`，mac 应为 `platform::physical_scale(window)`（只在找不到显示器时用到） |
| `services/fences.rs:474-519` `create_fence` / `:533-563` `reset_positions` / `:443-470` `open_by_id` | 全平台代码 | 能跑；摆窗受 `apply_window_rect` 影响（`reset_positions` 的 `apply_targets`） | 无需平台分支 |
| `crates/app/src/services/mac_menu.rs:125-136` `close_window` | — | ⌘W 对 `WindowKind::Fence` 已经是 no-op（注释"Windows 专有，macOS 上不会出现"） | 改注释 |
| `services/mac_menu.rs:56` ⌘M `Minimize` | — | `with_active_window(.. window.minimize_window())` → GPUI `miniaturize:`（`gpui-pre-macos/src/window.rs:1948-1953`），不看 style mask | 收纳箱是活动窗口时跳过（同 ⌘W 的写法），否则 ⌘M 能把收纳箱缩进程序坞（未在 runner 验证 `miniaturize:` 对无 Miniaturizable 位窗口的行为） |
| `services/mac_menu.rs:102-106` `on_reopen` | — | GPUI 只在 `applicationShouldHandleReopen:hasVisibleWindows:` 的 `hasVisibleWindows == NO` 时回调（`gpui-pre-macos/src/platform.rs:1369-1378`）。**有收纳箱可见时**，主窗口被 ⌘W 关掉后点 Dock 可能不再回调 → 主窗口唤不回来（是否把桌面层 / `canHide = NO` 的窗口算作"可见"未知，需 runner 验证） | 选项：① 接受，靠菜单栏图标唤起；② 在 `ns.rs` 用 `NSNotificationCenter` 观察 `NSApplicationDidBecomeActiveNotification`（要 `block2` + objc2-foundation `NSNotification`/`NSOperation` 特性）自己处理；③ 收纳箱可见时仍由主窗口激活回调处理（主窗口已关时无效）。PRD Open Question |
| `crates/app/src/windows/main_window.rs:774-777` | 主窗口关闭且没有任何窗口时退出（`cfg!(windows)`） | mac 不退出 | 不需要改 |
| `crates/app/src/windows/settings/general.rs:313,361-371` | `MINI_TODO_DEBUG_WINDOWS_UI` 只控制「固定模式时，嵌入桌面中」一行 | — | 与收纳箱无关；**设置页没有任何收纳箱专属的平台闸门**（`settings/screen.rs` 删屏幕配置连带删 `fence_layouts`，全平台） |

### 1.2 app：收纳箱窗口内部（`windows/fence/**`，本身没有 cfg）

| 位置 | 内容 | mac 上的问题 / 建议 |
|---|---|---|
| `windows/fence/mod.rs:573-609` 标题栏移动 | 超过 3 逻辑像素才 `gate.arm` + `platform::begin_window_drag`（`:608`） | mac 的 `begin_window_drag` = `window.start_window_move()`（`platform/macos/mod.rs:71-73`）→ `performWindowDragWithEvent:[NSApp currentEvent]`（`gpui-pre-macos/src/window.rs:2220-2232`）。这里是在 **mouse move** 事件里调用（currentEvent 是 `LeftMouseDragged`，不是 mouse down）。`macos/mod.rs:70` 注释称"鼠标按下 / 移动的事件处理里同步调用"即可，但收纳箱这条路径没在 runner 上跑过——e2e 要覆盖 |
| `windows/fence/mod.rs:612-630` `on_geometry_event` + `logic.rs:75-108` `GeometryCommitGate`（3000ms 窗口，`timing::GEOMETRY_ARM_MS`） | `observe_window_bounds`（`mod.rs:172-174`）每次回调续期，停 300ms 落库 | mac 拖窗期间 `windowDidMove:` 是持续发还是松手才发，未核实；若只在松手发且拖动超过 3 秒，闸门已过期 → 不落库。e2e 断言"拖动后 `fences.x/y` 更新" |
| `windows/fence/mod.rs:633-652` `commit_geometry` | `platform::client_rect_physical`（`:636`） | mac = frame × 统一系数，口径与 `fence_view` 一致，OK |
| `windows/fence/mod.rs:73-82` `ResizeSession`、`:727-739` `cursor_for_resize`、`:741-762` `begin_resize`、`:764-790` `resize_move`、`:792-797` `flush_resize` | 起点客户区 + 光标；`cursor_position_physical()` 有值就当物理像素，否则窗口逻辑坐标 × `dpr` | mac 的 `cursor_position_physical` 有值（`NSEvent.mouseLocation` × 统一系数，`macos/mod.rs:107-110`）→ 走物理分支，正确。但：① `:759` `dpr = window.scale_factor()`，`:779` 最小宽 `round(160 × dpr)`——mac 物理口径应乘 `platform::physical_scale(window)`（混合缩放时 dpr ≠ 统一系数；单一缩放时相等）；② `:794-795` 把 `dpr` 当 `scale` 传给 `apply_window_rect`，mac 改成按物理摆窗后这个参数不再用到；③ `:727` 注释"其它平台用窗口坐标"要改 |
| `windows/fence/mod.rs:390-427` `sync_view` | 折叠 / 展开 → `fence_view` → `apply_window_rect`（`:420`） | 依赖 `apply_window_rect` 修复（现在 mac 折叠只改高度，展开向上长的位移不生效） |
| `windows/fence/mod.rs:921-958` `move_in`、`grid.rs:517-543` `restore_item`、`grid.rs:545-576` `move_item_to`、`panels.rs:71-111` `confirm_delete` | 前台取 `platform::hwnd(window).map(\|h\| h.0)` 作 owner，后台 `spawn_db` 执行 | mac 上 owner = 窗口号（`platform/mod.rs:100-102`），对 NSFileManager 无意义，但签名可原样保留；线程规则（后台执行）对 mac 同样合适（NSFileManager 跨卷移动是阻塞拷贝） |
| `grid.rs:400-426` `open_item`、`grid.rs:461-472` `reveal_item`、`grid.rs:341-390` `pump_icons` | `items::open` / `reveal` / `icon` | 现在 mac 一律 `Err(UNSUPPORTED)`：双击报"打开「…」失败：桌面收纳箱仅支持 Windows"，图标全是占位。`pump_icons` 的 `dpr = window.scale_factor()`（`:345`）用于请求真实像素尺寸的图标，**mac 上就该用 backing scale**，不用改 |
| `grid.rs:654-684` `on_grid_wheel` | `event.modifiers.control` 才调图标大小 | mac 惯例是 ⌘（Control+滚轮在「辅助功能 → 缩放」开启时被系统拿去放大屏幕）。可以 mac 上同时认 `modifiers.platform`；README 的"Ctrl+滚轮"随之改 |
| `grid.rs:952-960` 右键「打开所在位置」、`:962-970`「移回桌面」 | 文案 | mac 惯用"在访达中显示"；文案是否分平台由 PRD 决定 |
| `panels.rs:135-150` `pick_custom_color` → `platform/fence_win.rs:12-18` `choose_color` | Windows：独立线程 `ChooseColorW`（`fence_win.rs:20-87`） | `fence_win.rs:89-103`：返回 `None` 并记日志——点「自定义颜色」没反应。mac 选项：`NSColorPanel`（MainThreadOnly、非模态、靠 target/action 或 `NSColorPanelColorDidChangeNotification` 回传，需要 `define_class!` 或 block2）；或 mac 上隐藏该按钮只留预设色 |
| `panels.rs:343` OneDrive 面板文案"桌面由 OneDrive 同步" | 只在 `onedrive_notice` 为 true 时出现 | mac 上 `desktop_is_onedrive_redirected()` 基本恒 false（见 1.3），面板不会出现；iCloud「桌面与文稿」是否要类似提示由 PRD 决定 |
| `windows/fence/mod.rs:1249-1276` 外部拖入（`ui::external_drop`） | 窗口任何位置松手都收 | gpui-pre-macos 的 `draggingEntered:` / `performDragOperation:` 读 `NSFilenamesPboardType`（`gpui-pre-macos/src/window.rs:3600-3660`），转成同样的 `FileDropEvent`，`external_drop` 不分平台。注意 GPUI 对外部拖入返回 `NSDragOperationCopy`（`:3611`）：访达光标显示绿色"+"，但本应用执行的是移动 |
| `windows/fence/mod.rs:1284-1315` 入口函数 | 注释"仅 Windows"（`:1284`） | 改注释 |
| `windows/fence/logic.rs:242-248` `parent_dir` | 同时认 `\` 和 `/` | mac 路径 OK |

### 1.3 core：`services/fence_fs/**`

| 位置 | Windows | mac 现在 | mac 建议 |
|---|---|---|---|
| `fence_fs/mod.rs:23-25`（文档）、`:38-41`（`shell` / `sta` 只在 Windows 编译）、`:43-46` `UNSUPPORTED` | — | — | 新增 `#[cfg(target_os = "macos")] mod mac;`（或类似）；`UNSUPPORTED` 的 `cfg_attr(windows, allow(dead_code))` 要改成 `cfg_attr(any(windows, target_os = "macos"), ..)`；模块文档"非 Windows 返回 Err"改写 |
| `fence_fs/ops.rs:357-373` `execute` | 新开 STA 线程 + 一个 `IFileOperation` 批量 `MoveItem`（`shell::perform_moves`，`shell.rs:218`），返回 `BatchOutcome { items: Vec<MoveOutcome { hr, new_path, error }>, aborted, perform_error }` | `#[cfg(not(windows))]` → `Err(UNSUPPORTED)` | mac 分支实现同一个 `Executor` 契约（`ops.rs:354` `type Executor = &dyn Fn(&[MoveJob]) -> Result<BatchOutcome, String>`）：逐个 `NSFileManager.moveItemAtURL:toURL:error:`（目标存在时失败、不覆盖；跨卷自动拷贝 + 删除），成功 `hr: Some(0), new_path: Some(dest)`，失败 `hr: Some(<负数>)` + 中文 `error`。`classify`（`ops.rs:140-214`）按文件系统实际状态归类，hr 只要 `>= 0` 表示成功、负数表示失败，与具体码无关。**不要用 `std::fs::rename`**：Unix `rename(2)` 会**覆盖**已存在的同名文件（违反"绝不覆盖"），跨卷 `EXDEV` 直接失败；测试里的 `fake_exec`（`ops.rs:676-723`）用 `fs::rename` 前先 `assert!(!dest.exists())`，只适合测试 |
| `fence_fs/ops.rs:430-448` `move_into_fence` 的 `protected = [user_desktop_dir(), public_desktop_dir()]` | 拒绝整体收纳桌面 / 公共桌面文件夹 | `public_desktop_dir()` mac 为 `None`（`location.rs:74-86`），只保护 `~/Desktop` | 可以不改；mac 没有公共桌面概念 |
| `fence_fs/folder.rs:66-80` `has_hidden_or_system_attribute` | `FILE_ATTRIBUTE_HIDDEN \| SYSTEM` → 列举时永不显示 | 恒 false | mac 对应物是 `st_flags & UF_HIDDEN`（`chflags hidden`，访达不显示）：`std::os::macos::fs::MetadataExt::st_flags()`，`UF_HIDDEN = 0x8000`。是否采用由 PRD 决定（不采用则行为与 Linux 一致） |
| `fence_fs/folder.rs:82-94` `is_hidden_or_system`（移入校验） | 看属性 | 看是否以 `.` 开头 → 点文件在 mac 上移入时 `skipped`（"隐藏或系统文件不收纳"） | 与 spec §3.4 表一致（"其他平台看点开头"），保持即可；若想与 Windows 一样允许 `.env` 移入，要改这里与 spec |
| `fence_fs/naming.rs:28` `SHELL_METADATA_FILES = ["desktop.ini", "thumbs.db"]`、`:216-219` `is_shell_metadata_file` | 不收纳、不显示、删除时不算剩余 | **`.DS_Store` 不在表里**：访达打开过收纳文件夹后会写 `.DS_Store` → ① 列举时作为自动隐藏项出现（全部隐藏时空状态显示"1 个项目已隐藏"，开了「显示隐藏的图标」会半透明显示它）；② `restore_all`（`ops.rs:562-639`）把它当普通项移回桌面（重名时变成 `.DS_Store (2)`）；③ `remove_folder_if_empty_at`（`folder.rs:279-321`）遇到它判"不空" | mac 上把 `.DS_Store`（以及 `.localized`、`Icon\r` 自定义图标文件）加入元数据表；`restore_all` / `remove_folder_if_empty` / 列举都经 `is_shell_metadata_file`，一处生效 |
| `fence_fs/naming.rs:126-134` `split_extension(name, is_dir)`（文件夹不拆扩展名）、`:137-151` 去重后缀 | `docs` → `docs (2)` | `.app` / `.bundle` 等**包**是文件夹：`Safari.app` 重名时变成 `Safari.app (2)`，访达不再当应用 | mac 上对包扩展（或 `NSWorkspace isFilePackageAtPath:`）按文件拆扩展名：`Safari (2).app` |
| `fence_fs/naming.rs:196-213` `is_shortcut_name` / `display_name` | 去掉 `.lnk` / `.url` | mac 的替身（alias，无扩展名）、`.webloc`、`.app` 原样显示（访达默认隐藏 `.app` 扩展名） | 可选：mac 显示名去 `.app`；`is_shortcut`（`FenceEntry.is_shortcut`，只影响显示）是否认 alias / `.webloc` |
| `fence_fs/folder.rs:111-125` `describe_io_error` | `raw_os_error` 32 / 33 / 145 是 Win32 码 | mac 上 32 = `EPIPE`、33 = `EDOM`，会误报"正被其他程序占用"（极少见）；`ENOTEMPTY` 是 66 | 可按平台分开；影响很小 |
| `fence_fs/folder.rs:264-277` `clear_readonly` | 清只读 | no-op | mac 上被锁定（`uchg`）的文件删 / 移会失败并照常报错，可以不改 |
| `fence_fs/open.rs:14-28` `launch_item`、`:34-64` `reveal_in_explorer` | `ShellExecuteExW` / `SHOpenFolderAndSelectItems`（失败退回 `explorer /select`） | `Err(UNSUPPORTED)`（`:24-27`、`:60-63`） | mac：`NSWorkspace.openURL:`（`objc2-app-kit` `NSWorkspace.rs:58`）/ `activateFileViewerSelectingURLs:`（`:107`）；或更省事的 `/usr/bin/open <path>` / `open -R <path>`（`std::process::Command`，不需要 objc2，后台线程可用，失败看退出码）。`objc2-app-kit 0.3.2` 的 `NSWorkspace` **不是** `MainThreadOnly`（`NSWorkspace.rs` 里没有该标记），可在 `spawn_db` 线程调用 |
| `fence_fs/icon.rs:196-209` `render_icon`、`:211-230` `icon_png_in` | `IShellItemImageFactory`（`SIIGBF_ICONONLY`）在常驻 STA 线程 → BGRA → PNG → 原子写入缓存 | `:226-229` `Err(UNSUPPORTED)`：缓存命中逻辑、`file_stamp`（`:54-65`，对 `.app` 等目录也能取 mtime / len）、`encode_png`（`:162-177`）、`write_atomically`（`:180-194`）全平台已在 | mac：`NSWorkspace.iconForFile:`（`NSWorkspace.rs:124`）→ `NSBitmapImageRep`（size_px × size_px、RGBA）+ `NSGraphicsContext graphicsContextWithBitmapImageRep:` 画进去 → 取像素走现成的 `encode_png` + `write_atomically`（或 `representationUsingType:PNG`）。`NSImage` / `NSBitmapImageRep` 在 objc2-app-kit 0.3.2 不是 `MainThreadOnly`（PRD 已记）。`#[cfg_attr(not(windows), allow(dead_code))]`（`:67,82,106,161,179`）要相应调整 |
| `fence_fs/location.rs:44-52` `fence_root_path` | `dirs::home_dir()/MiniTodo收纳箱` | mac 同一表达式 → `~/MiniTodo收纳箱`（家目录根不受 TCC 保护） | 不改 |
| `fence_fs/location.rs:69-71` `user_desktop_dir` | `dirs::desktop_dir()`（已处理重定向） | `~/Desktop`。**TCC**：从 `~/Desktop` 移走 / 移回 `~/Desktop` 属于"桌面文件夹"访问，非沙盒应用首次访问会弹授权框（ad-hoc 签名每次升级 cdhash 变，授权可能要重给）；`bundle.sh` 的 Info.plist（`scripts/macos/bundle.sh:98-122`）没有 `NSDesktopFolderUsageDescription` | 授权框文案可加该键；runner 上没人点授权框 → e2e 的源 / 移回目标要用临时目录，避开 `~/Desktop`（见 §3.4）。拖放本身是否已隐式授权（user intent）未核实 |
| `fence_fs/location.rs:88-117` `desktop_is_onedrive_redirected` / `is_under_onedrive` | OneDrive 环境变量 + 路径段 `onedrive*` | mac 上没有这些环境变量，`~/Desktop` 不含 `onedrive` 段 → 恒 false，不会弹 OneDrive 面板 | iCloud「桌面与文稿」：`~/Desktop` 路径不变、内容在 iCloud 同步，可能的判定依据（`~/Library/Mobile Documents/com~apple~CloudDocs/Desktop` 是否存在等）**未核实** |
| `fence_fs/location.rs:120-136` `normalize_for_compare`（小写比较） | NTFS 不区分大小写 | APFS 默认也不区分大小写；区分大小写的卷上会把两个只差大小写的名字当成同一个（只影响极少数场景） | 不改 |
| `fence_fs/watch.rs:126-150` `watch_root_at`、`:53-75` `folder_of` / `relative_to` | `notify` 递归监听，按根目录前缀算收纳文件夹名 | `notify` 7 在 mac 上默认走 FSEvents，报告**解析过符号链接**的路径；生产根目录 `~/MiniTodo收纳箱` 不是符号链接，OK；测试已在 mac 上 `canonicalize`（`watch.rs:212-216`，注释写着"收纳箱只在 Windows 上用，生产代码不处理"）。如果 `MINI_TODO_FENCES_ROOT` / 根目录经过符号链接（例如 `/var/folders/...`、`/tmp`），`folder_of` 匹配不上 → 不刷新 | mac 上在 `watch_root_at` 里先 `canonicalize(root)`（Windows 不能做，会变 `\\?\`），并改掉测试注释 |
| `fence_fs/ops.rs:1170-1283` `mod real_moves`（`#[cfg(windows)]`） | 临时目录里真实跑 IFileOperation 的往返测试 | 不编译 | 加一份 `#[cfg(target_os = "macos")]` 的同款测试（CI `checks` 作业的 `cargo test --workspace` 会在 mac 上跑它） |

### 1.4 core：`fence/**`

| 位置 | 内容 | mac |
|---|---|---|
| `fence/mod.rs:1053-1105` `delete(db, id, owner_hwnd)` → `fence_fs::restore_all`（`:1067`） | 全部移回后文件夹清空才删记录 | 只依赖 `execute`；`.DS_Store` 问题见 1.3 |
| `fence/items.rs:565-683` `move_in`（跨箱识别 `source_fence_folder` `:527`、OneDrive 提示 `:616-657`）、`:686-718` `restore`、`:721-776` `move_to`、`:779-796` `open` / `reveal` / `icon` | 全平台代码，底层调 fence_fs | 无平台分支；`items.rs:1117`、`fence/mod.rs:1688` 的 `cfg!(windows)` 只是测试里把 `\` 换 `/` |
| `fence/mod.rs:328-334` `fence_view`、`:314-325` `title_scale`、`:670-681` `title_bar_px` / `default_fence_size`、`:757` `create` | 按 `MonitorArea.scale` 算 DIP → 物理 | mac 的 `MonitorArea.scale` 恒为统一系数（`mac_geometry.rs:126-147`），"32 DIP × 统一系数" = 32 点，口径自洽，不用改 |

### 1.5 其它平台层文件

| 位置 | 内容 | mac |
|---|---|---|
| `platform/embed.rs:206-225` 非 Windows 桩 | `desktop_host` None、`attach_to_desktop` false、其余 no-op | 收纳箱 mac 实现不必经过 `embed`（`embed` 的契约是 Progman owner，与 mac 层级模型不同）；建议在 `platform/macos/` 加独立的"桌面层"入口（如 `platform::set_desktop_layer(window/Hwnd)`），`ns.rs` 新增原始调用 |
| `platform/mod.rs:52-71` macOS 文档、`:94-98` 模块声明（`pub mod embed; pub mod fence_win;` 全平台） | 文档写着"no-op 项：…嵌入桌面（`embed`）" | 新增的桌面层能力要写进这段文档 |
| `platform/macos/ns.rs:170-178` `set_floating`（只有 Floating ⇄ Normal） | — | 新增：`set_level(number, level: isize)` 或 `set_desktop_level(number)`、`set_collection_behavior(number, bits)`、`set_can_hide(number, bool)`、`set_excluded_from_windows_menu(number, bool)`；均为 `NSWindow` 方法（objc2-app-kit `NSWindow.rs:1446` `setCollectionBehavior`、`:1084` `setCanHide`、`:756` `setExcludedFromWindowsMenu`；`NSWindowCollectionBehavior::{CanJoinAllSpaces = 1<<0, Stationary = 1<<4, IgnoresCycle = 1<<6}` 在 `:152-165`），已有 `NSWindow` 特性即可。层级值：`objc2-core-graphics 0.3.2` 的 `kCGDesktopIconWindowLevel = kCGDesktopWindowLevel + 20`（头文件常量，`CGWindowLevel.rs:11`）或运行时 `CGWindowLevelForKey(DesktopIconWindowLevelKey)`（`:110`，Apple 文档推荐用函数）——用后者要给 app / mac-check 加 `objc2-core-graphics`（特性 `CGWindowLevel`；该包已在两个 `Cargo.lock` 里，作为 objc2-app-kit 的依赖） |

---

## 2. 几何：mac 口径 vs 收纳箱代码的假设

### 2.1 mac 的三套坐标（`platform/mac_geometry.rs:1-20` 模块文档）

- AppKit：全局点、主屏左下原点、y 向上。
- GPUI mac：逻辑像素 = 点；窗口 `bounds()` 与建窗 `window_bounds` 是**显示器本地**坐标。
- 本 crate"物理像素"（库里存的几何）：全局点（翻成左上原点）× **统一系数** `ScreenLayout::unit()`（所有屏 `backingScaleFactor` 的最大值，`mac_geometry.rs:81-84`）。
  `MonitorInfo.scale` 在 mac 上恒为统一系数（`:126-147`），`PhysRect::to_logical(scale)` 因此得到全局点坐标。
- 屏幕组合 ID 例外：按各屏自己的缩放（`mac_geometry.rs:150-162`，`platform/mod.rs:294-312`），与主窗口 `screen_configs` 同一口径，`fence_layouts` 可直接复用。
- 窗口 frame = 客户区（`FullSizeContentView`），没有 Windows 的不可见边框；`client_rect_physical` = `window_rect_physical`（`macos/mod.rs:158-160`）。
- 逻辑 → 物理用 `platform::physical_scale(window)`（`platform/mod.rs:334-344`），**不要用** `window.scale_factor()`（gpui-conventions §2.15）。

### 2.2 收纳箱代码的几何路径逐项核对

| 路径 | 代码 | mac 结果 |
|---|---|---|
| 显示器工作区 | `services/fences.rs:81-95` `monitor_areas`：`m.work`（mac = `visibleFrame`，去掉菜单栏与程序坞）+ `m.scale`（= 统一系数） | 正确；默认位置在菜单栏下的右上角 |
| 建窗 bounds | `services/fences.rs:141-151`：`bounds = rect / s`，`s` = `fence_view` 返回的缩放 = 统一系数 → **全局点坐标**；`:165-168` `display_id` = 包含 `(x + w/2, y + 4)` 的显示器 | 交给 `windows::open_window`（`windows/mod.rs:220-246`），`:229` 调 `platform::creation_bounds` → `mac_geometry::display_local_bounds`（`mac_geometry.rs:168-195`）换成显示器本地坐标并补显示器。**收纳箱确实经过 `creation_bounds`**，口径正确 |
| 建窗后校正 | `windows/mod.rs:249-258` `client_target`（收纳箱给了 `client_rect`）→ `:273-295` `adjust_native_window`：`prepare_after_open`（隐藏红绿灯、`setHidesOnDeactivate(false)`，`ns.rs:234-240`）→ 延后 `set_client_rect(target)`（mac 已到位就不动，`macos/mod.rs:271-280`） | 正确 |
| `WindowSpec` 其它字段 | `resizable: false` → mac 无 `Resizable` 位（程序 `setFrame:` 不受影响）；`minimizable: false` → 无 `Miniaturizable` 位；`focus: false`；`min_size` 160×32 点（`window_min_size`）；`kind` 默认 `Normal` → `NSNormalWindowLevel` + 鼠标跟踪区 `ActiveAlways`（`gpui-pre-macos/src/window.rs:2499-2519`，应用不在前台时悬停也能触发折叠展开）；`is_movable: true`、`app_owns_titlebar_drag: true`（`windows/mod.rs:199-202`）；`tabbing_identifier` 默认 None → 不会被并成标签页 | `show` 见 1.1 |
| 运行时摆窗 | `apply_window_rect`（`services/fences.rs:112-127`） | **问题 1**：mac 只 `resize`、不移动，且除以的 `scale` 口径不一（`MonitorArea.scale` = 统一系数；缩放会话传入的是 `window.scale_factor()`）。影响折叠 / 展开（`sync_view`）、重置位置、屏幕组合切换、手柄缩放 |
| 缩放手柄 | `windows/fence/mod.rs:741-797` | **问题 2**：光标走物理分支（正确），但最小尺寸 `round(160 × dpr)` 用 `window.scale_factor()`——混合缩放（窗口在 1x 屏、统一系数 2）时最小宽变成 80 点；单一缩放无影响 |
| 拖动落库 | `on_title_mouse_move` → `begin_window_drag`（mac `performWindowDragWithEvent:`）→ `observe_window_bounds` → 闸门 → `commit_geometry` 读 `client_rect_physical` | **问题 3（待验证）**：`performWindowDragWithEvent:` 用的是 move 事件；`windowDidMove:` 的触发频率决定 3 秒闸门是否够用 |
| 屏幕组合检测 | `check_screen_layout`（`services/fences.rs:316-362`），`:337` 兜底 `window.scale_factor()` | 主路径按显示器矩形取 `m.scale`（统一系数），正确；兜底值应换 `physical_scale` |
| 图标请求尺寸 | `grid.rs:345-346` `icon_request_px(icon_size, window.scale_factor())` | 这里要的是真实像素，backing scale 正确 |
| 统一系数随显示器变化 | 统一系数 = 当前所有屏的最大缩放 | **问题 4（边界）**：在只有 1x 屏时记下的 `fences.x/y/width/height`（"最近一次位置"）在接上 Retina 屏后统一系数变 2，若新组合没有 `fence_layouts` 记录，`prepare_restore` 用最近一次位置 → 尺寸 / 位置减半，再被 `fence_view` 夹回工作区。运行中切换走 `screen_switch` 读当前窗口（已按新系数），不受影响。主窗口有同样性质（CLAUDE.md 已知限制"混合缩放"） |
| AppKit 约束 | 带标题栏样式的窗口顶边不能高过菜单栏（gpui-conventions §2.15） | 用户拖动时会被挡住；程序 `setFrame:` 是否被约束未核实；默认位置在 `visibleFrame` 内，不受影响 |

### 2.3 窗口语义（层级 / 空间 / 隐藏 / 激活）

- 目标（PRD Assumptions）：层级 `kCGDesktopIconWindowLevel + 1`（桌面图标之上、所有普通窗口之下），`collectionBehavior = CanJoinAllSpaces | Stationary | IgnoresCycle`
  （所有空间可见、不受调度中心 / 显示桌面影响、不进 ⌘\` 循环），`canHide = NO`（⌘H 的 `[NSApp hide:]` 不隐藏它；`mac_menu.rs:49` `HideApp` → `cx.hide()` → `gpui-pre-macos/src/platform.rs:710-713` `hide:`）。
- GPUI 的 NSWindow 子类 `canBecomeKeyWindow = YES`、`acceptsFirstMouse = YES`（`gpui-pre-macos/src/window.rs:284,440`）：点收纳箱会激活应用。点击后台应用的窗口时，系统是只前置被点窗口还是把主窗口也带上来——**未核实**（需 e2e：Finder 在前时点收纳箱，看主窗口在 CGWindowList 里的前后顺序）。
- GPUI 在名为 `"Window"` 的菜单上 `setWindowsMenu:`（`gpui-pre-macos/src/platform.rs:313-316`）；本项目菜单名是「窗口」，不会触发，窗口列表不会自动出现在菜单里。`setExcludedFromWindowsMenu(true)` 作保险。

---

## 3. mac e2e 框架（`pc-gpui/scripts/e2e-macos/`）

### 3.1 结构

- 文件：`run_e2e.py`（1169 行，用例与入口；文件头 `:1-39` 是用例表）、`maclib.py`（1260 行，工具）、`set_display.py`（尝试 1920×1080）、`README.md`。
  失败注解 / 截图注解复用 `../e2e-windows/emit_annotations.py`。
- 用例表：`run_e2e.py:1012-1023` `SMOKE`（M01–M10，元组 `(用例号, 标题, 函数, 超时秒)`）、`:1024-1029` `RELEASE`（P01–P04，共用一个实例 `rel_app()` `:827`）。
- 每个用例：`start(tag, debug_open)`（`run_e2e.py:114-116`）= `fresh_data(tag)` 用 seed 工具把 `scripts/sample-data.json` 导进 `<work>/<tag>/` → `App(...).start()`。
  `Report.case`（`maclib.py:1187`）负责超时（SIGALRM）、失败截图、`results.json` / `summary.md`。断言用 `Checks.ok / near / done`（`maclib.py:1116-1140`）。
- 启动：`maclib.App.start`（`maclib.py:953-970`）直接执行 `<app>/Contents/MacOS/mini-todo`；环境变量：去掉继承的 `MINI_TODO_*`，设
  `MINI_TODO_DATA_DIR=<work>/<tag>`、**`MINI_TODO_FENCES_ROOT=<work>/<tag>-fences`（已经隔离了收纳根目录）**、`MINI_TODO_DEBUG_OPEN`、`RUST_BACKTRACE=1`，可经 `env=` 追加。
  日志 `<data>/logs/mini-todo.log`（`App.log_path` `:942-946`），`App.q(sql)` 读 `<data>/data.db`（`:1052`）。
- 支持的 `MINI_TODO_DEBUG_OPEN`（`crates/app/src/windows/mod.rs:572-639` `parse_debug_open`）：`main`、`editor[:new|:<id>[:view|:edit]]`、`subtask:<id>[:mode]`、
  `settings[:<页>]`、`completed`、`notification[:<id>]`、**`fence:new`**（`DebugOpen::NewFence` → `fence::tray_create_fence`，`:695-697`，与菜单栏「新增收纳箱」同一实现）、
  **`fence:<id>`**（`fence::open` → `services::fences::open_by_id`，按当前组合的存储几何建窗并开始监听）、`gallery`、`calendar`、`toast:..`、`confirm:..`。
  另有 `MINI_TODO_DEBUG_FENCE_STATE=onedrive|drag-hover`（只改界面状态，`windows/fence/mod.rs:237-247`）、`MINI_TODO_SCREEN_ID_FILE`（模拟屏幕组合变化）。
- **没有**"不经真实拖放就把文件移进收纳箱"的 debug 开关（搜索 `MINI_TODO_DEBUG_*` 只有上面几项）。

### 3.2 可用工具（`maclib.py`）

| 能力 | 函数 |
|---|---|
| 窗口枚举（`CGWindowListCopyWindowInfo`） | `list_windows()` `:133-153`（**选项是 `kCGWindowListOptionOnScreenOnly \| kCGWindowListExcludeDesktopElements`**，`:137`）、`windows_of(pid, layer=0)` `:156-157`（**默认只要 layer 0**）、`dump_windows` `:160-171`；`WinInfo` 含 `number`（= `windowNumber` = `Hwnd`）、`layer`（`kCGWindowLayer`）、全局点 bounds、`alpha`；`App.windows(layer=0)` `:998`、`App.wait_windows` `:1003` |
| 屏幕 | `screen_info()` `:200`（尺寸 + `visibleFrame`）、`expected_size` `:216` |
| 截图 | `capture_window(win, path)` `:238`（`screencapture -l`）、`capture_screen` `:249`、`content_stats` / `has_content` `:259-286` |
| OCR | `ocr(img, path)` `:335`（Vision，zh-Hans + en-US）、`fuzzy_find` `:385`；`run_e2e.py` 的 `ocr_until` `:187` 每 2s 重试 |
| AX | `ax_probe` `:426`、`app_menus` `:751`、`status_item` / `open_status_menu` / `press_status_menu_item` `:762-831`（菜单栏图标菜单）、`ax_window` / `ax_set_xy` `:834-875`、`system_events_set` `:876` |
| CGEvent | `click(x, y, button)` `:603`（左 / 右键）、`drag(x0, y0, x1, y1)` `:613-625`（左键按下 → 分步 `LeftMouseDragged` → 抬起）、`post_key_hid` `:558`、`post_key_to_pid` `:543`、`type_text` `:570`（Unicode）、`set_clipboard` `:584`、`park_cursor` `:628`；键码 `KEY_Q/W/A/V/RETURN/ESCAPE` `:524-525` |
| 前台 | `frontmost_pid` `:648`、`activate_other("Finder")` `:681`、`run_e2e.py` 的 `bring_to_front` `:259`、`send_cmd_key` `:290` |
| 进程 | `App.alive / wait_exit / stop / panics / errors / dump_logs / bundle_id / activate / terminate_via_appkit` `:972-1095`、`kill_all_procs` `:906` |
| 依赖 | pyobjc Quartz / Vision / ApplicationServices + Pillow；AppKit 随 Quartz 的依赖装上（`maclib.py:52-56` 已 `import AppKit`），可以在 Python 里写拖放源 |

### 3.3 现有用例里与收纳箱冲突 / 相关的地方

- **M07**（`run_e2e.py:550-589`）断言菜单栏图标菜单**没有**收纳箱三项（`:562-563`，常量 `TRAY_FENCE_ITEMS` `:77`，注释 `:75`"收纳箱三项只在 Windows 上有"）——要翻转为"有"。README `:34`、verification.md §7.2 同步改。
- **M08**（`:592-630`）⌘W 后 `open <app>` 唤起主窗口：可加一个"有收纳箱可见时"的变体验证 `hasVisibleWindows` 问题（1.1）。
- `main_win(app)`（`:167`）/ `wait_for` 用 `layer=0` 过滤，收纳箱若在负层级不会干扰现有用例。
- `list_windows()` 带 `kCGWindowListExcludeDesktopElements`：Apple 对它的描述是排除"桌面元素（背景图、桌面图标）"；**桌面图标层 + 1 的窗口会不会被排除未核实**。收纳箱用例应另写一个不带该选项的枚举（只用 `kCGWindowListOptionOnScreenOnly`），并 `layer=None`。

### 3.4 建议的收纳箱用例（smoke 组，debug .app）

| 用例 | 操作 | 断言 |
|---|---|---|
| MF1 新建 | `start(tag, "main,fence:new")` | 本进程多出一个窗口：`kCGWindowLayer` < 0 且等于"桌面图标层 + 1"（参照值：`Quartz.CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`；若能枚举到 Finder 的桌面图标窗口，再与它的 layer 比较）；约 368×336pt、右上角在 `visibleFrame` 内（右边距 24pt）；截图有内容、OCR 认出「收纳箱」/「把桌面图标拖到这里」；库里 `fences` 一行、`<work>/<tag>-fences/收纳箱` 目录存在；日志无 panic |
| MF2 监听 | 往收纳文件夹里写 `a.txt` | ≤ 3s 内 OCR 认出 `a.txt`、`fence_items` 有行且 `original_path IS NULL`（FSEvents → 对账 → `FenceRefresh`） |
| MF3 真实拖入 | 源放在 `<work>/<tag>-src/`（避开 `~/Desktop` 的 TCC 授权框）。两种拖放源：① `open <src>` 打开访达窗口，AX 找到项目的位置，`maclib.drag` 拖到收纳箱；② 用 pyobjc 起一个小进程做 `NSDraggingSource`（窗口里 `beginDraggingSessionWithItems:` 放 `NSURL`），CGEvent 在它上面按下拖到收纳箱 | 源文件消失、收纳文件夹里出现、`fence_items.original_path` = 源路径；重名时 ` (2)` 不覆盖 |
| MF4 移回 | 右键项目 → 「移回桌面」（坐标按网格布局常量算，或 OCR 定位菜单项） | 文件回到原目录（MF3 的临时目录）、`fence_items` 行删除 |
| MF5 删除 | 「更多」→「删除收纳箱」→ 确认 | 全部移回原目录、收纳文件夹删除、`fences` 行没了、窗口消失 |
| MF6 ⌘H 不隐藏 | 激活应用后发 ⌘H | 主窗口不在 OnScreen 列表里、收纳箱仍在 |
| MF7 Dock 唤起（有收纳箱时） | ⌘W 关主窗口 → `open <app>` | 主窗口是否重新出现（验证 `hasVisibleWindows`；按 PRD 决定是断言还是只打印） |
| MF8 重启恢复 | 退出后不带 `MINI_TODO_DEBUG_OPEN` 重启 | 收纳箱在原位置重新出现（`restore_all`）、标题 / 折叠 / 锁定保持 |
| MF9 拖动落库 | 在收纳箱标题栏上 `maclib.drag`（先移动 > 3pt 再继续） | 窗口移动、300ms 后 `fences.x/y` 与 `fence_layouts` 更新（验证 `performWindowDragWithEvent:` 与闸门） |
| MF10 菜单栏图标三项 | `press_status_menu_item("显示/隐藏所有收纳箱")` 两次、「重置收纳箱位置」 | 隐藏后不在 OnScreen 列表、再点出现；重置后位置变为主屏右上 |
| （信息性）点击不前置主窗口 | Finder 在前时点收纳箱 | 打印主窗口与 Finder 窗口在 CGWindowList 里的先后 |
| （信息性）显示桌面 | `/System/Applications/Mission Control.app/Contents/MacOS/Mission Control 1` 或 F11 | 收纳箱 bounds 不变（验证 `Stationary`；runner 上是否可触发未知） |

- collectionBehavior / canHide 在 CGWindowList / AX 里读不到：只能用行为断言（MF6、显示桌面）或让应用在 debug 日志里打印实际值再读日志。
- 空间（Spaces）在 runner 上通常只有一个桌面，`CanJoinAllSpaces` 基本测不了。

### 3.5 Linux e2e 的 F 组不是收纳箱

`scripts/e2e-linux/run_e2e.py` 的 F00–F16 是通用功能用例（FAB 新建、详情、子任务、四象限……，`run_e2e.py:133-171,211-`），不涉及收纳箱；
Linux 只在 `harness.py:548` 设 `MINI_TODO_FENCES_ROOT`、`run_e2e.py:1137-1149` 的旧版库兼容检查里删收纳箱表。
收纳箱的 e2e 在 **Windows** 套件：`scripts/e2e-windows/run_e2e.py:1242`（`debug_open="main,fence:new"`）、F01–F11 `:1517-1527`，真实拖放靠 `drag-source.ps1`（.NET `DataObject` / Shell 数据对象 / 资源管理器窗口作拖放源）。
Linux 的视觉回归 `scripts/regress.py:131-273` 有 `fence*` 场景（`open="main,fence:1"`，SQL 改状态、`MINI_TODO_DEBUG_FENCE_STATE`）与 `smoke-fence-new`（`scripts/README.md:65`）。

---

## 4. `scripts/mac-check`

- `scripts/mac-check/Cargo.toml`：不属于 workspace（文件末尾空 `[workspace]`），有自己的 `Cargo.lock`；`[dependencies] log = "0.4"`；
  `[target.'cfg(target_os = "macos")'.dependencies]`：`tray-icon =0.21.3`、`auto-launch =0.5.0`、`image =0.25.10`、`objc2 =0.6.5`、
  `objc2-foundation =0.3.2`（std、objc2-core-foundation、NSArray、NSDictionary、NSGeometry、NSString、NSValue）、
  `objc2-app-kit =0.3.2`（std、objc2-core-foundation、NSApplication、NSButton、NSControl、NSEvent、NSGraphics、NSResponder、NSScreen、NSView、NSWindow）——
  与 `crates/app/Cargo.toml` 的 mac 依赖逐项一致（版本钉成主 `Cargo.lock` 的版本）。
- `scripts/mac-check/src/lib.rs`：`#![cfg(target_os = "macos")]` + `#![allow(dead_code)]`，四个 `#[path]` 模块：
  `crates/app/src/services/autostart.rs`、`services/single_instance/unix_socket.rs`、`services/tray/mac_icon.rs`、`platform/macos/ns.rs`。
  用法：`cargo clippy --manifest-path scripts/mac-check/Cargo.toml --target aarch64-apple-darwin -- -D warnings`（verification.md §7.1）。
- **加 core 的 mac 文件**（如 `crates/core/src/services/fence_fs/mac.rs`）：这个文件必须像 `ns.rs` 一样**自包含**——不引用 `super::` / `crate::`
  （`MoveJob` / `BatchOutcome` 之类的类型留在 `ops.rs`，mac 文件只收发路径、`bool`、`Result<_, String>`、像素缓冲等普通类型），
  然后在 `lib.rs` 加 `#[path = "../../../crates/core/src/services/fence_fs/mac.rs"] mod core_fence_fs_mac;`。
  Cargo.toml 补上该文件用到的特性（例如 objc2-foundation 的 `NSFileManager`、`NSURL`、`NSError`、`NSData`；objc2-app-kit 的 `NSWorkspace`、`NSImage`、
  `NSImageRep`、`NSBitmapImageRep`、`NSGraphicsContext`——特性名在 objc2-foundation 0.3.2 `Cargo.toml:84,99,107,198`、objc2-app-kit 0.3.2 `Cargo.toml:201,705,734,753,1984` 核实过）。
  core 是 edition 2021，mac-check 是 2024：被 `#[path]` 的文件按 mac-check 的 2024 edition 编译，写法要两边都能过。
- **加 `ns.rs` 新函数**：`setLevel` / `setCollectionBehavior` / `setCanHide` / `setExcludedFromWindowsMenu` 都在已开启的 `NSWindow` 特性里，mac-check 无需改依赖；
  若用 `CGWindowLevelForKey`，app 与 mac-check 都要加 `objc2-core-graphics = { version = "0.3.2", features = ["CGWindowLevel", ...] }`（两份 lock 里已有该包）。
- 开新特性可能让 mac-check 的 `Cargo.lock` 里 `objc2-app-kit` 的依赖列表变化（例如 `block2`、`objc2-core-graphics` 是可选依赖，目前 mac-check lock 里
  objc2-app-kit 只依赖 bitflags / objc2 / objc2-core-foundation / objc2-foundation），需要在本机重新生成该 lock（不加 `--locked`）。

---

## 5. CI 与 core 的 mac 依赖

### 5.1 `.github/workflows/pc-gpui-e2e-macos.yml`

- 触发：`workflow_dispatch`；`push` 到 `claude/**` 且改动 `pc-gpui/**` 或本 workflow（spec / CLAUDE.md 单独改不触发）。`concurrency` 按 ref 取消旧运行。
- 三个作业（都在 `macos-15`，arm64）：
  - `checks`（75 min 超时）：`cargo clippy --workspace --all-targets --locked -- -D warnings` + `cargo test --workspace --locked --no-fail-fast`（clippy 失败也跑测试）。core 的 mac 单测（例如新加的真实移动测试）在这里跑。
  - `smoke`（90 min）：debug 构建 app + `seed` example → `bundle.sh target/debug/mini-todo - $RUNNER_TEMP/bundle` → `lsregister` → `set_display.py` → `run_e2e.py --suite smoke --wallpaper 1F6F8B --strict-traffic-lights` → 失败 / 截图注解 → 上传产物 `pc-gpui-e2e-macos`。
  - `release-smoke`（90 min）：release 构建（host 架构）→ .app → `--suite release --allow-real-profile` → 收集真实目录 → 产物 `pc-gpui-e2e-macos-release`。
- 耗时（`gh run view`，2026-10-16 分支 `claude/magical-volta-w2tl2o` 的两次成功运行，缓存命中）：`checks` 约 1–1.5 分钟，`smoke` 约 3 分钟，`release-smoke` 约 4–10 分钟；整轮墙钟 4–10 分钟。新增收纳箱用例会拉长 `smoke`。
- 结果取回：`gh run view --job <id> --log | grep -E '\[(PASS|FAIL|SKIP)\]|TOTAL:| BAD '`；注解 API；从日志按 PNG 头切分还原截图（verification.md §7.3）。

### 5.2 `.github/workflows/pc-gpui-ci.yml` 的 `macos` 作业（`:97-145`）

- 只在 `main` push / 对 `main` 的 PR / 手动触发（`:3-14`），`claude/**` 分支不跑。
- clippy + test + `--target aarch64-apple-darwin --target x86_64-apple-darwin` 的 release 构建 → `bundle.sh` 合成 universal → 上传 `mini-todo-gpui-macos-universal`。x86_64 切片只在这里与 `release.yml` 交叉编译，没有运行过。
- `cache-warm.yml` 的 `gpui-macos` 作业与 `release.yml` 的 mac 作业逐字对齐（`cache-warm.yml:16-25,85-88`），改 mac 构建命令要三处一起改。

### 5.3 core 加 mac 依赖

- 现状：`crates/core/Cargo.toml` 只有 `[target.'cfg(windows)'.dependencies]`（windows 0.58 + windows-core 0.58），没有任何 mac 依赖；`notify = "7"`（mac 默认 FSEvents）、`png = "0.17"` 全平台。
- app 的 mac 依赖（`crates/app/Cargo.toml` `[target.'cfg(target_os = "macos")'.dependencies]`）：`objc2 = "0.6"`、`objc2-foundation = { version = "0.3.2", default-features = false, features = [std, objc2-core-foundation, NSArray, NSDictionary, NSGeometry, NSString, NSValue] }`、
  `objc2-app-kit = { version = "0.3.2", default-features = false, features = [std, objc2-core-foundation, NSApplication, NSButton, NSControl, NSEvent, NSGraphics, NSResponder, NSScreen, NSView, NSWindow] }`；注释要求改动时同步 `scripts/mac-check/Cargo.toml`。
- 主 `Cargo.lock`：`objc2 0.6.5`、`objc2-foundation 0.3.2`、`objc2-app-kit 0.3.2`（其依赖列表已包含 `block2 0.6.2`、`objc2-core-graphics`、`objc2-core-image`、`objc2-core-text`、`objc2-cloud-kit` 等全部可选依赖，因为 GPUI / tray-icon 已经把它们拉进来了）；另有 `objc2 0.5.3` / `objc2-app-kit 0.2.2` 一套旧版本（其它依赖用）。
- 给 core 加 `[target.'cfg(target_os = "macos")'.dependencies]`，写成与 app **相同的版本需求**（`objc2 = "0.6"`、`objc2-foundation = "0.3.2"`、`objc2-app-kit = "0.3.2"`，`default-features = false`）：
  Cargo 按目标统一特性，core 新开的特性也会作用到 app 的同一份 crate，不会多编一份；`Cargo.lock` 只变 `mini-todo-core` 那一项的 `dependencies` 列表（多出 `"objc2 0.6.5"`、`"objc2-app-kit 0.3.2"`、`"objc2-foundation 0.3.2"`），**不新增包**。
  在 Linux 上可以 `cargo tree --locked --target aarch64-apple-darwin -p mini-todo-core` 核对（verification.md §7："在非 mac 上能做的只有依赖解析"）——第一次改依赖时去掉 `--locked` 生成 lock。
- 若 mac 的打开 / 定位用 `/usr/bin/open`、移动用 `libc::renamex_np(RENAME_EXCL)` 之类，则不需要 objc2；但图标（`NSWorkspace iconForFile:`）只能走 AppKit。

---

## 6. 需要随之更新的文档

| 文件 | 位置 / 内容 |
|---|---|
| `CLAUDE.md` | `:8` 项目简介"桌面收纳箱（仅 Windows）"；`:190` 小节标题"桌面收纳箱（仅 Windows）"及其下的 Win32 专有描述（`IFileOperation`、`ShellExecuteExW`、UAC、OneDrive、`%USERPROFILE%`）；`:219` 菜单栏图标菜单"（这三项仅 Windows）"；`:226`「macOS（预览）」"没有收纳箱、嵌入桌面"；`:547` 已知偏差 4（Linux 段，列了收纳箱为 Windows 专有）；`:554` 已知偏差 6"没有嵌入桌面、收纳箱"；`:697` 注意事项 1"桌面收纳箱等 Win32 特性在 macOS / Linux 上不可用"；结构树 `platform/` 一行与 `services/` 说明；「兼容旧版沿用的名字」mac 表可补收纳根目录 `~/MiniTodo收纳箱/` |
| `README.md` | `:58` "### 桌面收纳箱（仅 Windows）" 与 `:59-64`（`%USERPROFILE%`、Ctrl+滚轮、Win+D）；`:66` 托盘"管理收纳箱（仅 Windows）"；macOS 安装 / 放行说明可能要加桌面文件夹授权 |
| `pc-gpui/crates/app/README.md` | `:167` tray"收纳箱三个入口（仅 Windows）"；`:173` fences"200ms 嵌入守护"；`:194` 收纳箱窗口接口；`:256` embed；`:258` `fence_win::choose_color`"其它平台 None"；`:266` `MINI_TODO_FENCES_ROOT`"代替 `%USERPROFILE%\MiniTodo收纳箱`"；`:276` Windows 行为列表 |
| `pc-gpui/crates/core/README.md` | `:63` Windows 专属测试；`:296` `open` / `reveal`"非 Windows 报仅支持 Windows"；fence_fs 段（`:28` 等）若新增 mac 文件 |
| `pc-gpui/scripts/README.md` | `:4` "Windows 专有行为…需要在 Windows 上实机验收"；`:128` `fence:new` 描述；末尾「macOS：打包、e2e 与交叉检查」 |
| `pc-gpui/scripts/e2e-macos/README.md` | `:34` M07"**没有**收纳箱三项"；用例表加 MF 组；`list_windows` 的选项说明 |
| `pc-gpui/scripts/e2e-linux/README.md` | `:11` "Windows 专有行为（…桌面收纳箱…）"（措辞可保持，Linux 仍测不到） |
| `.trellis/spec/backend/desktop-fences.md` | `:30-32` 平台说明（托盘入口 / 启动恢复只在 Windows、mac 没有入口）；`:122` `restore_all`"仅 Windows"；`:126` `apply_window_rect`；§3.1 建窗参数表（`show` 行）与"嵌入后的 Win32 状态"需加 mac 对应表；§3.4 文件层（`.DS_Store`、NSFileManager、TCC）；`:515` 错误矩阵"非 Windows：…`Err(\"桌面收纳箱仅支持 Windows\")`"；§6 测试表（mac 单测与 mac e2e）；"已知限制" |
| `.trellis/spec/pc-gpui/gpui-conventions.md` | §2.15 最后一条（`:292` 起）"Windows 专有的 `platform::*` 在 mac 上是 no-op：…`embed::*`"；如新增桌面层能力、`NSColorPanel`，补到 §2.15；`:190,206` `MINI_TODO_DEBUG_WINDOWS_UI` 不涉及收纳箱 |
| `.trellis/spec/pc-gpui/verification.md` | `:185` §7.2"M07 菜单栏图标菜单（无收纳箱项）"；§7.1 mac-check 覆盖范围（若加入 core mac 文件）；§7.4 runner 上测不了的项（Spaces、TCC 授权框） |
| `.trellis/spec/pc-gpui/index.md` | `:35` "**全部**（仅 Windows；mac 上没有收纳箱入口）" |
| `.trellis/spec/backend/index.md` | `:61` "收纳箱改动：跑 … Windows e2e（F 组）"（加 mac e2e） |
| `.trellis/spec/guides/desktop-window-e2e-guide.md` | 「macOS runner」一节（`:96-131`）补收纳箱的窗口枚举（负层级、`ExcludeDesktopElements`）与拖放源做法 |
| `.trellis/spec/backend/window-modes.md` | `:4,23,314-320` 写着 `platform::embed` 由主窗口与收纳箱共用——mac 收纳箱不走 `embed` 时要说明 |
| `.trellis/spec/pc-gpui/data-compat.md` | `:35` 写 mac 首次打开迁移 v29 / v30"（mac 上不会用到）"一类说法（该行被截断，需看全文） |
| `docs/开发文档/01-项目概述.md` | `:7` "（嵌入桌面与收纳箱仅 Windows）" |
| `docs/开发文档/02-技术架构.md` | `:46` `windows/fence/`"桌面收纳箱（透明、嵌入桌面）"、`:172` 透明窗口一段 |
| `.github/release-notes/v3.0.2-pre.md` | `:43` "新增的是收纳箱用的表，mac 上不会用到"；`:8` "收纳箱隐藏图标（Windows）"。这是已发布 tag 的说明，按 `release.yml` 的约定新版本应另建 `.github/release-notes/<新 tag>.md`，而不是改旧文件 |
| 代码内注释 | `services/tray.rs:3-5,212`、`lib.rs:85`、`services/fences.rs:1-17,42,108-111,271,408`、`windows/fence/mod.rs:727,1284`、`windows/mod.rs:12`（"桌面收纳箱（透明、嵌入桌面；Windows）"）、`platform/mod.rs:70-71`、`services/mac_menu.rs:125`、`fence_fs/mod.rs:23-25`、`fence_fs/watch.rs:212-214`、`platform/fence_win.rs:1-5` |

---

## Caveats / Not Found

- **未核实（需要 macOS runner 实测）**：`kCGWindowListExcludeDesktopElements` 是否排除桌面图标层 + 1 的窗口；`performWindowDragWithEvent:` 用 mouse-dragged 事件是否正常拖窗；
  拖窗期间 `windowDidMove:` 的触发频率（影响 3 秒闸门）；`hasVisibleWindows` 是否把桌面层 / `canHide = NO` 的窗口算作可见（Dock 唤起主窗口）；
  点击后台的收纳箱时主窗口是否被一起前置；⌘M 的 `miniaturize:` 对没有 Miniaturizable 位的窗口是否生效；`Stationary` 在"显示桌面"/ 台前调度下的表现；
  从 `~/Desktop` 拖入时 TCC 是否因"用户拖放"隐式授权；程序 `setFrame:` 是否受菜单栏约束。
- iCloud「桌面与文稿」的检测方法、`kCGDesktopIconWindowLevel` 头文件常量与 `CGWindowLevelForKey` 运行时值是否一致，均未核实。
- 没有现成的"不经真实拖放移入文件"的 debug 开关；mac 上真实拖放源（访达 + AX 定位，或 pyobjc 拖放源进程）都还没写过，可靠性未知。
- `.trellis/spec/pc-gpui/data-compat.md:35` 与若干长行在 grep 输出里被截断，表中只列了位置，没逐字核对全文。
- CI 耗时只取了同一天两次缓存命中的运行，冷缓存会长得多（上个任务调研估计单轮 15–30 分钟）。

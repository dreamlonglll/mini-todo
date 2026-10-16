# macOS 桌面收纳箱

## Goal

用户要求（2026-10-16）："继续 mac 的任务开发，包括收纳箱"，确认"开始"。3.0.2-pre 的 macOS 预览版对齐了 2.5.0 mac 版的能力，但收纳箱
（桌面收纳箱，Windows 3.0 新功能）在 mac 上完全没有：入口、文件操作、桌面层窗口都只有 Windows 实现。目标：把收纳箱移植到 macOS，
行为尽量与 Windows 一致（真收纳：文件从桌面移到可见文件夹 `~/MiniTodo收纳箱/<名称>/`，文件夹是 source of truth，**不丢文件、绝不覆盖**），
在 macOS runner 上可自动化验证。Windows 零回归。

## Research References

- [`research/codebase-map.md`](research/codebase-map.md) — 要改的全部位置（闸门、几何、core 文件层、e2e、mac-check、CI、文档清单 §6）
- [`research/mac-desktop-window.md`](research/mac-desktop-window.md) — 桌面层窗口：层级、collectionBehavior、canHide、PopUp（非激活面板）、Dock 唤起（§7 有 `ns.rs` API 草图）
- [`research/mac-file-ops.md`](research/mac-file-ops.md) — 移动（renamex_np + NSFileManager）、TCC、iCloud、图标、打开 / 定位、垃圾文件、UF_HIDDEN、NSColorPanel（草图均已对 aarch64-apple-darwin 类型检查）

## Decisions（ADR-lite）

**D1 窗口：mac 上收纳箱用 GPUI `WindowKind::PopUp`（非激活 NSPanel），建窗后改成桌面层**
- Context：Windows 的嵌入桌面（Progman owner + 置底）在 mac 上没有对应物；普通 NSWindow 被点击会激活应用（菜单栏切换、⌘H 隐藏中的应用被
  取消隐藏），且会计入 Dock 唤起的 `hasVisibleWindows`，导致关掉主窗口后点 Dock 唤不回来。
- Decision：仅 macOS：`kind = PopUp`、`show = false` → 建窗后（App 未借用时）层级 `CGWindowLevelForKey(kCGDesktopIconWindowLevelKey) + 1`、
  collectionBehavior 整体替换为 `CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenNone`、`canHide = NO`、`excludedFromWindowsMenu = YES`、
  `animationBehavior = None`（`hidesOnDeactivate = NO` 已由 `prepare_new_window` 设好）→ 摆到库里的几何 → 不处于"全部隐藏"时
  `orderFrontRegardless`。收纳箱窗口绝不调用 `set_topmost` / `ns::set_floating` / `setFloatingPanel:`（会改层级）。
  改名开始时先 `window.activate_window()`（非激活面板单击不会成为 key）。Windows / Linux 保持 `Normal`。
- Consequences：点收纳箱不激活应用；NSPanel 不计入 `hasVisibleWindows`（Apple 文档），现有 `on_reopen` 不改。**退路**：若 CI 证明非激活面板
  收不到键盘（改名用例失败）或 Dock 唤起失效，改用 `Normal` + 自己注册 `kAEReopenApplication` Apple Event handler（mac-desktop-window.md §5.3）。
  中文输入法在非激活 key 面板里是否正常未验证（runner 测不了），写进已知限制。

**D2 移动：`renamex_np(RENAME_EXCL)` 为主，EEXIST 顺延改名重试，EXDEV 才交给 `NSFileManager moveItemAtURL`**
- NSFileManager 同卷实现是 lstat + 普通 rename，有覆盖窗口；RENAME_EXCL 由内核原子判重。跨卷 NSFileManager 用 copyfile EXCL，不覆盖。
- 逐项结果映射到现有 `MoveOutcome { hr, new_path, error }`（成功 `hr = Some(0)`、`new_path = 实际路径`；失败 `hr = Some(负数)` + 中文 `error`），
  `classify` 不变。`std::fs::rename` 会覆盖同名文件，**禁止**用于生产移动。
- 错误归类：`EPERM` 且源带 `UF_IMMUTABLE/SF_IMMUTABLE` → "文件已锁定"；其余 `EPERM` → 隐私权限被拒，提示到「系统设置 → 隐私与安全性 →
  文件与文件夹」允许 mini-todo；`EACCES` → 没有权限；`EDEADLK` / iCloud 占位（`ENOENT` 且同目录有 `.名字.icloud`）→ "请先在访达中下载该文件"。

**D3 TCC：正常路径不触发「桌面文件夹」授权框**
- 拖放即授权被拖的项目；在受保护目录新建目录项不需要同意；但**列举**受保护目录会弹框（ad-hoc 签名每次升级都会再弹）。
- mac 上规划阶段**不列举** 收纳根目录以外的目标目录（`existing_names_lower` 对它们返回空集合），撞名交给 D2 的 EEXIST 顺延；收纳根目录内照常列举。
- Info.plist 加 `NSDesktopFolderUsageDescription` / `NSDocumentsFolderUsageDescription` / `NSDownloadsFolderUsageDescription`（万一弹框时有中文说明）；
  身份键不变。
- 新增 debug-only 环境变量 `MINI_TODO_DESKTOP_DIR`（同 `MINI_TODO_FENCES_ROOT`：release 一律不读）替代 `user_desktop_dir()`，让 e2e 的"移回桌面"
  落在临时目录，绝不碰真实 `~/Desktop` / `~/Documents` / `~/Downloads`（runner 上弹框没人点会永久阻塞）。

**D4 iCloud「桌面与文稿」：移动前逐项取 `NSURLIsUbiquitousItemKey`，有则首次提示一次**
- 移出 iCloud 管理的桌面 = 从 iCloud 删除（其它设备也会少这个文件），与 OneDrive 提示同理；共用 `fence_onedrive_notice_shown` 的"只提示一次"。
- 判断必须在移动之前（移走后就不是 ubiquitous 了）。面板文案按平台：Windows 仍是 OneDrive，mac 写 iCloud。

**D5 其它与 Windows 对齐的细节**
- 垃圾文件：`.DS_Store`、`Icon\r`、`.localized` 并入 `SHELL_METADATA_FILES`（全平台）：不显示、不收纳、删除时不算剩余、空文件夹判定时删除它们。
- 隐藏属性：mac 的 `UF_HIDDEN`（`chflags hidden`）= Windows 隐藏属性（永不显示、移入 skipped、删除时一并移回）；**点文件在 mac 上允许移入**
  （移入后按名字自动隐藏，同 Windows 无隐藏属性的 `.env`）。Linux 维持"看点开头"。
- 包（`.app` 等，目录但访达当文件）：去重时按文件拆扩展名 → `Safari (2).app`（不是 `Safari.app (2)`）。
- 图标：`NSWorkspace iconForFile:` 画进自建 N×N `NSBitmapImageRep` → PNG → 现有缓存（`write_atomically`），全程 `autoreleasepool`，后台线程。
- 打开：`NSWorkspace openURL:configuration:completionHandler:`（拿 NSError），打开前 `URLByResolvingAliasFileAtURL` 检查替身失效；
  定位：`activateFileViewerSelectingURLs:`（先 lstat）。右键菜单文案 mac 上「在访达中显示」（Windows 仍「打开所在位置」）。
- 自定义颜色：`NSColorPanel`（MainThreadOnly）：设初始色、`setHidesOnDeactivate(false)`、显示；GPUI 前台每 150ms 轮询，面板关闭时返回最终颜色
  （与 Windows 模态对话框同一个 `Task<Option<(u8,u8,u8)>>` 接口）；换 sRGB 并确认 RGB 模型后再读分量。
- 图标大小：mac 上 ⌘+滚轮（同时保留 Ctrl+滚轮）。
- ⌘M（窗口菜单「最小化」）在收纳箱是活动窗口时跳过（同 ⌘W）。
- 目录监听：mac 上 `watch_root_at` 先 `canonicalize(root)`（FSEvents 报告解析后的路径）。
- 缩放手柄 / 屏幕组合兜底缩放：mac 用 `platform::physical_scale(window)`，不用 `window.scale_factor()`。

## Requirements

### 入口与生命周期（app）
- 菜单栏图标菜单在 mac 上加入「新增收纳箱 / 显示/隐藏所有收纳箱 / 重置收纳箱位置」（`cfg(any(windows, target_os = "macos"))`）
- 启动恢复收纳箱（`services::fences::restore_all`：Windows + macOS；Linux 仍不恢复）、启动对账 / 图标缓存清理照常
- `apply_window_rect` mac 走 `platform::set_client_rect_physical`（摆位置 + 尺寸）；`set_all_visible` mac 用 `Hwnd::show/hide`
- mac 建窗按 D1；`embed_after_open` 的 mac 分支在 `cx.spawn` 的 async 块里、`update` 之外执行
- 新的 `ns.rs` 原始调用（如 `make_desktop_layer(number)`）保持自包含（mac-check 能 `#[path]` 进去）；层级用 `CGWindowLevelForKey`
  （extern "C" 链接 CoreGraphics，或 objc2-core-graphics——优先不引入新包）

### 文件层（core `services/fence_fs`）
- 新增叶子模块 `services/fence_fs/mac.rs`（`cfg(target_os = "macos")`，只依赖 std / libc / objc2 系列，不 `use super::` / `crate::`；
  mac-check 用 `#[path]` 引入）：移动（D2）、图标 PNG、打开、定位、`is_ubiquitous_item`
- `ops::execute` mac 分支接到 mac.rs；规划阶段按 D3 不列举受保护目标目录
- `open.rs` / `icon.rs` mac 分支；`folder.rs` UF_HIDDEN；`naming.rs` 垃圾文件、包扩展名；`location.rs` `MINI_TODO_DESKTOP_DIR`（debug）
- `UNSUPPORTED` 只剩 Linux，文案改为"桌面收纳箱仅支持 Windows 与 macOS"
- core `Cargo.toml` 加 macOS 依赖（objc2 0.6 / objc2-foundation 0.3.2 / objc2-app-kit 0.3.2 / block2 0.6 / libc，与 app 版本一致，不引入 lock 里没有的包）
- `fence::items::move_in` 在移动前按 D4 判定云同步提示

### 验证
- core mac 单测（CI `checks` 作业跑）：RENAME_EXCL 不覆盖 + 撞名顺延、往返移入移回（临时目录）、`.DS_Store` 不阻止删空文件夹、包扩展名去重、
  UF_HIDDEN 不显示、图标输出恰好 N×N 的合法 PNG
- mac e2e（smoke 组，debug `.app`，全部用临时目录，`MINI_TODO_FENCES_ROOT` + `MINI_TODO_DESKTOP_DIR`）：
  - 新建（`main,fence:new`）：本进程多出一个窗口，`kCGWindowLayer` = 桌面图标层 + 1，位于 `visibleFrame` 右上；库里一行、文件夹存在；截图 / OCR 有内容
  - 监听：往收纳文件夹写文件 → 收纳箱里出现、`fence_items` 有行
  - 真实拖入：从临时目录拖文件进收纳箱（pyobjc 拖放源进程或访达窗口）→ 源消失、文件在收纳文件夹、`original_path` 正确
  - 移回、删除收纳箱（全部移回、文件夹删除、窗口消失）
  - ⌘H 后收纳箱仍在屏幕上；点收纳箱后前台应用不变（非激活）
  - 有收纳箱可见时 ⌘W 主窗口 → `open <app>` → 主窗口回来（Dock 唤起）
  - 双击标题改名（键盘输入）→ 落库、文件夹改名
  - 拖标题栏移动 → `fences.x/y` 落库
  - 菜单栏图标三项（显示/隐藏、重置位置）；重启后恢复
  - M07 翻转为"菜单栏图标菜单**有**收纳箱三项"
  - 收纳箱用例用不带 `kCGWindowListExcludeDesktopElements` 的窗口枚举、`layer=None`

## Acceptance Criteria

- [ ] `pc-gpui-e2e-macos.yml` 的 `checks`（clippy / test，含新 mac 单测）、`smoke`（含新收纳箱用例）、`release-smoke` 全绿
- [ ] 本机：`cargo fmt --check`、`cargo clippy --workspace --all-targets --locked -D warnings`、`cargo test --workspace --locked`、
      Windows 交叉 clippy、mac-check 交叉 clippy 全绿
- [ ] `pc-gpui-e2e-windows.yml` 照常通过（Windows 零回归）
- [ ] 规范 / CLAUDE.md / README / crate README / scripts README / 开发文档同步；新的发布说明草稿（不改已发布的 `v3.0.2-pre.md`）

## Definition of Done

- 单测 / e2e 补齐；所有门槛绿；macOS CI 绿
- `desktop-fences.md` 新增 macOS 场景（窗口、文件层、TCC、已知限制）、`gpui-conventions.md` §2.15 / `verification.md` §7 / `window-modes.md` 同步
- 交付说明写明哪些只在 runner 上验证、哪些只能靠用户反馈

## Out of Scope (explicit)

- Spaces / Stage Manager / 「点按墙纸显示桌面」/ Mission Control 的行为验证（runner 测不了，只按 collectionBehavior 设置）
- Retina / 多显示器 / 中文输入法在非激活面板里的真机验证（写进已知限制）
- 真实 `~/Desktop` 的 TCC 授权流程验证（runner 上没人点授权框）
- 主窗口「嵌入桌面」在 mac 上的实现（仍然只有 Windows）
- Developer ID 签名与公证；图片 / 视频缩略图（与 Windows 一致只显示类型图标）
- 打开 / 定位的 e2e（会拉起访达 / 其它应用，runner 上只做单测之外的人工验收）

## Implementation Plan（每步推分支跑 macOS CI）

1. **core 文件层**（agent A）：`fence_fs/mac.rs` + ops / open / icon / folder / naming / location / watch 的 mac 分支、items 的云同步提示、
   core 依赖与 `Cargo.lock`、mac 单测、mac-check 引入 core mac 文件
2. **app 窗口与入口**（agent B，与 A 并行，文件不重叠）：PopUp + 桌面层（`ns.rs` / `platform/macos/mod.rs` / `platform/mod.rs`）、
   `services/fences.rs`、`windows/fence/**`（缩放 / 改名 / 滚轮 / 文案 / 云同步面板文案）、菜单栏图标三项、`lib.rs`、`mac_menu.rs` ⌘M、
   `fence_win.rs` NSColorPanel、`bundle.sh` 用途说明键、mac-check 引入新增 ns 特性
3. **mac e2e**：收纳箱用例 + M07 翻转（依赖 1、2 在 CI 上能跑起来）
4. **文档 / 规范 / 发布说明**；trellis-check；spec 更新

## Technical Notes

- 本机无法编译 mac 目标的 app / core（C 依赖要 Apple SDK）；mac 专属代码只能经 `scripts/mac-check`（叶子文件）本地类型检查，其余靠 CI
- macOS CI：推 `claude/**` 且改动 `pc-gpui/**` 触发 `pc-gpui-e2e-macos.yml`（checks ~1.5 分钟、smoke ~3 分钟、release-smoke 4–10 分钟）；
  同时触发 `pc-gpui-e2e-windows.yml`
- 规范：`.trellis/spec/backend/desktop-fences.md`、`.trellis/spec/pc-gpui/gpui-conventions.md` §2.15、`.trellis/spec/pc-gpui/verification.md` §7

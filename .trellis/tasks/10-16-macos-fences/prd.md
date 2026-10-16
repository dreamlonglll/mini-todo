# macOS 桌面收纳箱

## Goal

用户要求（2026-10-16）："继续 mac 的任务开发，包括收纳箱"。3.0.2-pre 的 macOS 预览版对齐了 2.5.0 mac 版的能力，但收纳箱
（桌面收纳箱，Windows 3.0 新功能）在 mac 上完全没有：入口、文件操作、桌面层窗口都只有 Windows 实现。目标：把收纳箱移植到 macOS，
行为尽量与 Windows 一致（真收纳：文件从桌面移到可见文件夹，文件夹是 source of truth），在 macOS runner 上可自动化验证。

## What I already know

- 上一个任务 `10-16-macos-package` 已合入 main（3.0.2-pre），PRD 把收纳箱列为 Out of Scope
- **现状**（读源码）：
  - 入口：菜单栏图标的三项收纳箱菜单 `cfg(windows)`（`services/tray.rs`）；`services::fences::restore_all` 非 Windows 直接返回
  - core `services/fence_fs`：文件夹管理 / 列举 / 命名 / 监听全平台可用（mac CI 已跑过监听测试，FSEvents 要 canonicalize）；
    移动（`ops::execute`，Windows 走 `IFileOperation`，有 `Executor` 抽象，测试用 `std::fs` 实现）/ 打开 / 定位 / 图标在非 Windows
    返回 `Err("桌面收纳箱仅支持 Windows")`
  - app `windows/fence/**` 没有 cfg（Linux 截图 / 冒烟用）；`services::fences::apply_window_rect` 非 Windows 分支只改尺寸、
    `embed_after_open` 非 Windows 空实现；200ms 守护的嵌入部分 `cfg(windows)`，屏幕组合检测全平台
  - mac 平台层（`platform/macos/{mod,ns}.rs`）已有摆窗 / 显示隐藏 / 置顶层级 / 几何锁定 / 显示器 / 光标；`Hwnd` = 窗口号
  - 收纳箱外观「自定义颜色」`platform::fence_win::choose_color` 非 Windows 返回 `None`
  - v29 / v30 表在 mac 上照样建，不用
- **GPUI（gpui-pre-macos 0.3.9）**：`WindowKind::Normal` = NSWindow（level Normal）；`PopUp` = NSPanel + NonactivatingPanel + PopUp level
  + CanJoinAllSpaces|FullScreenAuxiliary；所有窗口 `canBecomeKeyWindow` = YES、`acceptsFirstMouse` = YES；`show: false` 时不 order front；
  `applicationShouldHandleReopen:hasVisibleWindows:` 只在没有可见窗口时回调 `on_reopen`；没有 app 激活回调
- objc2-app-kit 0.3.2：`NSWorkspace` / `NSImage` / `NSBitmapImageRep` / `NSGraphicsContext` 不是 MainThreadOnly（可在后台线程用）
- 用户没有 Mac：只能靠 CI macOS runner（`pc-gpui-e2e-macos.yml`，推 `claude/**` 触发）与用户反馈；本机只能用 `scripts/mac-check` 对
  不依赖 GPUI 的 mac 源文件做 `--target aarch64-apple-darwin` 类型检查

## Assumptions (temporary)

- 收纳根目录 mac 上 = `~/MiniTodo收纳箱/`（与 Windows `%USERPROFILE%\MiniTodo收纳箱\` 同一表达式 `dirs::home_dir()`）
- 桌面层：窗口层级 = 桌面图标层之上一层（`kCGDesktopIconWindowLevel + 1`），在所有普通窗口之下；collectionBehavior
  `CanJoinAllSpaces | Stationary | IgnoresCycle`；`canHide = NO`（⌘H 不隐藏）；不进「窗口」菜单
- 移动用 NSFileManager（不覆盖、可跨卷）；打开 / 定位用 NSWorkspace；图标 NSWorkspace `iconForFile:` → PNG

## Open Questions

- （调研中）有收纳箱可见时 Dock 唤起主窗口怎么做；点收纳箱是否会把主窗口带到前面（非激活面板？）
- （调研中）桌面文件夹 TCC 授权、iCloud「桌面与文稿」检测是否可行

## Requirements (evolving)

- mac 上菜单栏图标菜单加入「新增收纳箱 / 显示隐藏所有收纳箱 / 重置收纳箱位置」，启动恢复收纳箱
- 收纳箱窗口位于桌面层（桌面图标之上、所有应用窗口之下），所有桌面空间可见，「显示桌面」时不被推开，⌘H 不隐藏
- 拖入 / 移回 / 跨箱移动 / 删除收纳箱（全部移回）、打开、在访达中显示、图标、隐藏图标、改名、折叠、锁定、外观、缩放、拖动、
  屏幕组合记忆：与 Windows 同语义（不丢文件、绝不覆盖）
- Windows 零回归

## Acceptance Criteria (evolving)

- [ ] mac CI（clippy / test）全绿；Windows / Linux CI 不受影响
- [ ] mac e2e：新增收纳箱出现在桌面层（窗口层级、collectionBehavior 断言）、拖入（临时目录）落到收纳文件夹、移回、删除收纳箱
- [ ] Windows `cargo test --workspace --locked`、clippy、Windows 交叉 clippy 照常通过

## Definition of Done

- 单测 / e2e 补齐；fmt / clippy / test / mac-check 绿；macOS CI 绿
- 规范（desktop-fences.md、window-modes / gpui-conventions 的 mac 部分）、CLAUDE.md、README、发布说明更新

## Out of Scope (explicit)

- （待定）

## Technical Notes

- 规范：`.trellis/spec/backend/desktop-fences.md`（收纳箱契约）、`.trellis/spec/pc-gpui/gpui-conventions.md` §2.15（mac 约定）、
  `.trellis/spec/pc-gpui/verification.md`（mac-check / mac e2e）
- 上个任务的调研：`.trellis/tasks/10-16-macos-package/research/*.md`

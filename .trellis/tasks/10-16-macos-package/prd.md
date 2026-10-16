# brainstorm: GPUI 版恢复 macOS 安装包

## Goal

用户要求（2026-10-16，发布 3.0.1-pre 时提出）："依旧需要打 mac 的包"。3.0 起 PC 端只有 GPUI 版，发布流水线只产出 Windows x64
安装包 / 便携版，2.5.0（Tauri 版）提供的 macOS 包（universal dmg）断档。目标：GPUI 版重新提供可用的 macOS 安装包，
数据与身份和 2.5.0 mac 版无缝衔接。

## What I already know

- v2.5.0 的 mac 资产：`mini-todo_2.5.0_universal.dmg`、`mini-todo_universal.app.tar.gz`（未签名，只有链接器自动 ad-hoc 签名）
- **身份（取自 2.5.0 实物 Info.plist）**：bundle id `com.tauri-app.mini-todo`、`mini-todo.app`、可执行文件 `mini-todo`、无 `LSUIElement`、
  最低 10.13；数据目录 `~/Library/Application Support/mini-todo/` = GPUI 的 `dirs::data_local_dir()/mini-todo`（一致）
- **2.5.0 mac 版能力**：菜单栏图标（左键唤起、右键 6 项菜单）、Tauri 默认 App 菜单（⌘Q / ⌘W / ⌘H / 编辑）、单实例
  （`/tmp/com_tauri_app_mini_todo_si.sock`）、开机自启（LaunchAgent `~/Library/LaunchAgents/mini-todo.plist`）、系统通知 + 应用内通知窗口、
  透明主窗口、简化版固定模式（不可缩放 / 不可拖、被最小化即还原、唤起临时置顶）、降级贴边隐藏；无嵌入桌面、无收纳箱；
  字体下拉为空、Dock 图标照常显示、关最后一个窗口即退出、WebDAV 密码明文
- **GPUI 版在 mac 上（读源码推断，未编译运行过）**：所有窗口左上角露出红绿灯；无菜单栏图标、无 App 菜单（⌘Q 不可用）；运行时不能移动窗口；
  Retina 下几何 ×2 / ×1 错位、多屏原点全 (0,0)；界面字体回退 Helvetica、代码块非等宽；固定模式只剩外观；无单实例；
  开机自启走 AppleScript（弹授权框、丢 `--autostart`、不认旧 plist）；通知以 Finder 名义显示；日志路径与旧版不同；屏幕配置 ID 算法不同
- **可行性**：不用升级 GPUI。gpui-pre-macos 0.3.9 已有激活策略、`on_reopen`、`set_menus`、Dock 菜单、`show_system_notification`、
  钥匙串、文件对话框、字体枚举、透明窗口；缺运行时移动 / 置顶层级 / 显示隐藏 / 缩放样式 → 新写 `platform/macos.rs`（objc2 调 NSWindow / NSScreen）。
  `tray-icon` 可沿用（GPUI 在 app 启动完成后才执行 `run` 回调，满足主线程 + run loop 要求），图标改 PNG 模板图标。
  `set_use_launch_agent(true)` 即与旧版同名 plist。GPUI 会把激活策略重设为 Regular（`LSUIElement` 无效，隐藏 Dock 只能用 API）
- **构建**：着色器运行时编译（gpui-kit 强制 `runtime_shaders`），构建期不需要 Metal 编译器；`Cargo.lock` 已含 mac 依赖（`--locked` 可用）；
  **Windows 机器上做不了任何 mac 编译检查**，只能靠 CI
- **打包**：`macos-15`（arm64）编两个架构 → `lipo` → 组装 `mini-todo.app`（Info.plist 照 2.5.0，`CFBundleShortVersionString` 去掉 `-pre`）→
  图标检回 `749405a:pc/src-tauri/icons/icon.icns` → 整包 `codesign --force --sign -` → hdiutil dmg（显式尺寸 + 重试，照 mini-term）；
  不用 `macos-15-intel`（2027-08 下线，且 Metal 多重采样卡死 runner-images#14836）
- **CI 可验证**：macOS runner 有已登录图形会话，bash / osascript 预授权屏幕录制与辅助功能 → 截图、枚举窗口、模拟点击输入、读库断言可自动化；
  测不了：通知横幅、Gatekeeper 首次打开、钥匙串弹窗、原生 Intel、真实开机自启、Retina / 多屏
- **规模**：新增 / 修改约 1500–2500 行，只靠 CI 迭代

## Research References

- [`research/legacy-mac-behavior.md`](research/legacy-mac-behavior.md) — 2.5.0 mac 能力清单、必须沿用的名字表（§2.5）、GPUI 各 cfg 分支在 mac 上的落点（§3）
- [`research/gpui-macos-platform.md`](research/gpui-macos-platform.md) — 各平台能力可行性 / 推荐做法 / 风险 / 工作量，易踩的点
- [`research/macos-packaging-ci.md`](research/macos-packaging-ci.md) — 产物形态、.app / 签名 / dmg、CI 改动 YAML 草稿（§5）、runner 冒烟方案（§6.3）

## Decisions

- 2026-10-16：3.0.1-pre 照常只发 Windows（修复右上角系统 ×）；mac 适配与打包作为本任务，完成后随下一个预发布版发布
- 用户**没有 Mac**：验收只能靠 CI 的 macOS runner 自动化，以及其他用户反馈；方案必须在 runner 上可验证，发布说明写明 mac 版未经真机验收

- 2026-10-16：**一次对齐 2.5.0 mac 版全部能力再发**（含固定模式、贴边隐藏降级版），不分期
- 2026-10-16：**mac 构建失败不挡发布**：Windows + cloud 成功即转正，mac 失败只记警告，修好后 `workflow_dispatch` 补建 dmg
- 签名：ad-hoc（`codesign --force --sign -`），发布说明写放行指引（`xattr -cr` / 系统设置「仍要打开」）；无 Apple 开发者账号，不做公证
- Actions 缓存（现 9.2 / 10 GB）：约一半是过期 key（被新 key 取代的、旧 Tauri `rust-rust-*` / npm、`claude/` 分支 e2e），LRU 淘汰会先清它们；
  加 mac 缓存不需额外处理，必要时手动删过期 key

## 状态（2026-10-16 上午调研结束）

- 需求与计划已整理完，**待用户下午确认**后开始执行；开始前还要用户授权把代码反复推到 `claude/macos-package` 分支跑 CI
  （本机做不了 mac 编译检查；不动 main、不打 tag）
- 下一步：用户确认 → 补 `implement.jsonl` / `check.jsonl` 上下文 → 按 Implementation Plan 第 1 步派 `trellis-implement`

## Requirements

### 身份与数据（与 2.5.0 mac 版一致，见 research/legacy-mac-behavior.md §2.5）
- bundle id `com.tauri-app.mini-todo`、`mini-todo.app`、可执行文件 `mini-todo`；数据目录 `~/Library/Application Support/mini-todo/`（现已一致）
- 日志改到 `~/Library/Logs/com.tauri-app.mini-todo/`（旧版位置）
- 单实例：Unix socket `/tmp/com_tauri_app_mini_todo_si.sock`、消息格式与 tauri-plugin-single-instance 一致，新旧版互认；再次启动转交参数并唤起主窗口
- 开机自启：`auto-launch` `set_use_launch_agent(true)` → `~/Library/LaunchAgents/mini-todo.plist`，带 `--autostart`
- 系统通知归属本 app（GPUI `show_system_notification` 或 notify-rust `set_application(bundle id)`，依调研择一）

### 平台能力（新 `platform/macos.rs` 等；非 Windows 的 Linux 降级行为保持不变）
- 所有窗口不露红绿灯（自绘标题栏）；App 菜单：⌘Q 退出、⌘W 关闭窗口、⌘H 隐藏、编辑菜单（剪切 / 复制 / 粘贴 / 全选 / 撤销）
- 菜单栏图标（PNG 模板图标）：左键唤起主窗口，右键菜单与 Windows 托盘同功能（收纳箱相关项不显示）
- 关闭最后一个窗口不退出（托盘常驻，与 Windows 一致）；"窗口全关即退出"只保留给 Linux（`lib.rs:67-74`）
- 窗口几何：Retina / 多屏正确（物理 ↔ 逻辑换算、显示器枚举与原点）；运行时摆窗、显示 / 隐藏、置顶、激活；屏幕组合 ID
- 固定模式（2.5.0 简化版语义）：不可拖动、不可缩放、被最小化即还原、唤起时临时置顶；贴边隐藏降级版（2.5.0 语义：无全局光标，按鼠标移入移出）
- 字体：mac 界面字体与代码等宽字体映射；设置里字体下拉能列系统字体
- 文件对话框（导入导出）可用；设置项按平台显示 / 隐藏（嵌入桌面、收纳箱等 Windows 专有项在 mac 隐藏）
- WebDAV 密码维持明文（与 2.5.0 一致），设置页提示

### 构建、打包与发布
- `pc-gpui-ci.yml` 新增 macOS 作业：fmt 之外的 build / clippy（-D warnings）/ test（`--locked`）——mac 专属代码唯一的编译门
- `release.yml`：plan 增加平台 key `macos`；`build-macos`（`macos-15`，arm64 + x86_64 → `lipo` → 组装 `.app` → ad-hoc 签名 → hdiutil dmg
  显式尺寸 + 重试 → 挂载回读校验 → `gh release upload --clobber`）；publish 不因 mac 失败而停（mac 失败记警告）；SHA256SUMS 含 dmg
- `cache-warm.yml` 增加与 release 逐字一致的 mac 预热作业
- 资产：`mini-todo_<ver>_universal.dmg`；Info.plist 照 2.5.0（`CFBundleShortVersionString` 去掉 `-pre`、`LSMinimumSystemVersion` 按 GPUI 实际要求）；
  图标检回 `749405a:pc/src-tauri/icons/icon.icns`
- macOS runner 冒烟 e2e（research/macos-packaging-ci.md §6.3）：从打好的 `.app` 启动 → 窗口出现（`CGWindowListCopyWindowInfo`）→
  截图断言确有窗口内容 → 菜单栏图标存在 → 关键交互（新建待办落库）→ 单实例（二次启动不新增进程）→ ⌘Q 退出

### 文档
- `.trellis/spec/pc-gpui/release.md`（mac 作业、资产、签名、非阻塞规则）、`data-compat.md` / CLAUDE.md「兼容旧版沿用的名字」加 mac 列、
  `verification.md`（mac CI 门）、CLAUDE.md 目标平台与 Linux/mac 说明、`crates/app/README.md` 平台表；发布说明模板加 mac 放行指引与"未经真机验收"说明

## Acceptance Criteria

- [ ] `pc-gpui-ci.yml` 的 macOS 作业 build / clippy / test 全绿；Windows / Linux 作业不受影响
- [ ] macOS runner 冒烟全部通过：窗口有内容、无红绿灯（截图左上角）、菜单栏图标存在、新建待办落库、二次启动不新增进程、⌘Q 退出
- [ ] `release.yml`（`workflow_dispatch` 演练或预发布 tag）产出 `mini-todo_<ver>_universal.dmg`：`lipo -info` 含 arm64 + x86_64、
      `codesign --verify --deep` 通过、Info.plist 身份与 2.5.0 一致、进入 SHA256SUMS；mac 作业失败时 release 仍转正
- [ ] Windows 侧零回归：`cargo test --workspace --locked`、clippy、Windows e2e（W / S / N / F）照常通过
- [ ] 文档与发布说明更新

## Out of Scope (explicit)

- 收纳箱、嵌入桌面等 Windows 专有功能
- Developer ID 签名与公证、钥匙串存 WebDAV 密码（维持明文，与 2.5.0 一致）
- 真机验收（用户无 Mac）；通知横幅、Gatekeeper 首次打开、真实开机自启、Retina / 多屏无法在 runner 上断言，发布说明注明

## Implementation Plan（顺序推进，每步在 macOS CI 上验证后再进下一步）

1. **编译门 + 打包骨架**：mac 能编译（cfg 修正）、`pc-gpui-ci.yml` mac 作业、`lib.rs` 退出规则、隐藏红绿灯、App 菜单、
   `release.yml` / `cache-warm.yml` mac 作业（非阻塞）、冒烟 e2e 最小版（启动 + 截图有内容）——尽早暴露 GPUI 在 runner 上能否渲染
2. **窗口几何**：`platform/macos.rs`（Retina / 多屏 / 运行时摆窗 / 显示隐藏 / 置顶）、屏幕组合 ID、字体映射与字体列表
3. **常驻与身份**：菜单栏图标、单实例（socket 互认）、LaunchAgent 自启、系统通知归属、日志路径
4. **固定模式与贴边隐藏降级版**、设置项按平台显示
5. **冒烟 e2e 补全 + 文档 / 规范 / 发布说明模板**；以预发布 tag 或 `workflow_dispatch` 演练出 dmg

迭代方式：本机（Windows）无法做任何 mac 编译检查 → 代码推到工作分支，由 CI 的 macOS 作业编译 / 测试 / 冒烟，失败日志回灌给实现子 agent。

## Technical Notes

- 相关：`pc-gpui/crates/app/src/platform/{mod,fallback}.rs`、`lib.rs:67-74`（窗口全关即退出，要改为仅 Linux）、
  `services/{tray,single_instance,autostart,logging,notification}.rs`、`.github/workflows/{release,cache-warm,pc-gpui-ci}.yml`、
  `.trellis/spec/pc-gpui/release.md`；参考 `D:\Git\mini-term\.github\workflows\release.yml`（mac 打包）

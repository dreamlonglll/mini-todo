# 桌面收纳箱（Desktop Fence）

## Goal

在 mini-todo PC 端（Windows）新增"收纳箱"：用户从托盘右键菜单「新增收纳箱」在桌面上创建一块常驻桌面层的区域，
把桌面上的应用图标**真正收进去**（文件从桌面移走，桌面变干净），双击即可启动；区域支持固定（锁定）、折叠、
重命名等常规操作。定位是轻量的桌面整理能力，不做 Fences 那种注入 Explorer 的重方案。

用户原始需求：
1. 右键右下角托盘 icon，新增选项「新增收纳箱」
2. 点击后在桌面新增一块区域，里面可以放桌面应用的图标
3. 区域支持固定（嵌入到桌面）、折叠、重命名标题等常规操作

## Requirements

### R1 存储模型：真收纳 + 可见收纳目录（2026-10-15 用户决定）

* 图标放进收纳箱 = 原快捷方式 / 文件从桌面**移走**到收纳目录，桌面上消失，只在收纳箱里显示；
  从收纳箱移出或删除收纳箱时**还原回桌面**。**不丢文件是硬约束**
* 每个收纳箱对应一个真实可见的文件夹 `%USERPROFILE%\MiniTodo收纳箱\<收纳箱名>\`
* **文件夹是内容的 source of truth**：收纳箱显示该文件夹内容（`notify` 监听变化），在 Explorer 里往文件夹放 / 删文件，
  收纳箱随之变化；数据库只存收纳箱元数据（标题、文件夹名、位置尺寸、折叠 / 固定 / 外观 / 图标大小、项目排序、项目原始路径）
* 默认细节（未单独询问，按此实现）：
  * 移动走 `IFileOperation`（Shell 变更通知让桌面立即刷新、重名自动改名、权限不足时弹系统"需要管理员权限"对话框）；
    公共桌面 `C:\Users\Public\Desktop` 上的项因此会触发 UAC 确认，用户取消则该项不收纳并提示
  * 记录每项的原始路径；移出 / 删除收纳箱时移回原目录，原目录不存在则回用户桌面（`dirs::desktop_dir()`），重名自动加后缀；
    删除收纳箱后移除已清空的文件夹（有移不回去的文件时的处理见下方"删除收纳箱时有项目未能移回"）
  * 在 Explorer 里直接放进收纳目录的文件没有原始路径，移出时回用户桌面
  * **删除收纳箱时有项目未能移回（2026-10-15 用户决定）**：删除中止、**保留整个收纳箱**（记录、窗口、文件夹都保留），
    已移回的项回到桌面，收纳箱继续显示剩余项并提示原因，用户处理后可再次删除；不产生无收纳箱管理的孤儿文件夹
  * 文件夹名跟随收纳箱标题（非法字符替换、重名加后缀），改标题时同步改文件夹名，失败则保留旧文件夹名并提示
  * 桌面被 OneDrive 重定向时，首次收纳给出一次性提示：移出桌面会同步到其他设备
  * 不移动 `desktop.ini`

### R2 窗口形态：始终在桌面层，固定 = 锁定（2026-10-15 用户决定）

* 收纳箱**始终嵌入桌面层**：owner=Progman + always_on_bottom + `WS_EX_TOOLWINDOW`；Win+D 后可见、不进任务栏、
  不参与 Alt+Tab、不会盖住其他应用窗口；Explorer 重启后自动重挂
* 「固定」= **锁定**位置和大小，与嵌入无关
  * 未固定：拖标题栏移动（按下后移动超过 3px 才开始拖动，避免吞掉双击改名）、拖边缘手柄缩放
    （窗口为 `resizable(false)`，原生缩放无效，改为前端手柄逐帧调用 `set_fence_bounds`，松手后落库）
  * 已固定：不可移动 / 缩放，但**仍可拖入图标、折叠、改名、打开项目**
* 每个收纳箱是一个独立的无边框透明 WebView 窗口（label `fence-{id}`）

### R3 基础操作（行业必备项）

* 托盘「新增收纳箱」：建在光标所在显示器的工作区内，相对已有收纳箱偏移、避免完全重叠；默认尺寸约 4 列 × 3 行图标；
  默认标题"收纳箱"（重名加序号）
* 拖入：从桌面 / 资源管理器拖文件、快捷方式、文件夹进来 → 移入收纳目录
* 打开：双击打开（exe / lnk / url / 文件夹 / 普通文件）；单击选中、悬停高亮；显示名去掉 `.lnk` / `.url` 后缀；
  目标失效的快捷方式打开失败时给提示，不影响其他项
* 项目右键菜单（自绘）：打开 / 打开所在位置 / 移回桌面 / 移动到…（见 R4）
* 箱内拖动排序，排序持久化
* 改名：**双击标题原地编辑**（Enter 确认 / Esc 取消 / 失焦提交），标题栏菜单也提供「重命名」
* 折叠：标题栏按钮卷起到只剩标题栏，再点展开；折叠态持久化；展开时下方空间不够则向上展开
* 固定：标题栏锁按钮切换，状态持久化
* 删除收纳箱：标题栏菜单 → 确认框写明收纳箱名称和"其中的 N 个项目会移回桌面"
* 应用重启后按库恢复全部收纳箱；启动时位置夹回可见工作区（防止显示器变化后跑到屏幕外）
* 非 Windows：托盘不显示收纳箱相关菜单项

### R4 增强项（2026-10-15 用户选定全部纳入首版）

* **托盘兜底入口**：托盘新增「显示/隐藏所有收纳箱」「重置收纳箱位置」（把所有收纳箱排回主显示器工作区内可见位置）
* **外观自定义**：每个收纳箱单独设置背景色和透明度（沿用主窗口 `window_bg_color` / `window_bg_alpha` 的做法），从标题栏菜单进入
* **跨箱移动**：项目右键「移动到…」→ 列出其他收纳箱，选中后文件在收纳目录间移动（保留原始路径记录）
* **图标大小**：收纳箱内 Ctrl+滚轮调整图标大小（每个收纳箱独立，有上下限），持久化
* **悬停临时展开**：折叠的收纳箱在鼠标悬停或拖着文件经过时临时展开，移开后自动收回（不改变持久化的折叠态）

### R5 按屏幕组合记忆位置（2026-10-15 用户追加）

* 每个收纳箱按**屏幕组合**分别记忆位置和尺寸；屏幕组合 ID 与主窗口 `screen_configs` 相同（`{屏幕数}_{宽}x{高}@{缩放%}_…`，
  按像素面积降序，无显示器时 `unknown`），Rust 侧实现须与前端 `appStore.generateScreenConfigId()` 结果完全一致（单测覆盖）
* 折叠 / 固定 / 外观 / 图标大小等状态全局共用，不随屏幕组合变化
* 写入时机：新建收纳箱时、用户拖动 / 缩放结束、托盘「重置收纳箱位置」后，写当前屏幕组合的那一份，同时更新 `fences.x/y/width/height`
  作为"最近一次位置"；显示器变化导致的系统挪窗**不写**
* 启动恢复：有当前屏幕组合的记录就用它，没有就用"最近一次位置"，都夹回可见工作区
* 运行时切换：后台约每 2 秒检测屏幕组合，变化后把各收纳箱恢复到新组合下的记录（没有记录则沿用当前位置并夹回可见区域，不落库）
* 设置里删除某个屏幕配置时，一并删除收纳箱在该组合下的位置记录

## Acceptance Criteria

* [ ] 托盘右键出现「新增收纳箱」，点击后在光标所在显示器出现新收纳箱，同时创建对应收纳文件夹
* [ ] 从桌面拖一个快捷方式进收纳箱：桌面上该图标消失、文件出现在收纳文件夹、收纳箱显示其图标和名称；双击能启动
* [ ] 拖入公共桌面上的快捷方式会弹 UAC 确认；取消则不收纳且有提示
* [ ] 项目右键「移回桌面」：文件回到原目录、桌面重新出现该图标；重名时自动改名，不覆盖
* [ ] 删除收纳箱：确认框含名称和项目数；确认后所有文件移回、空文件夹被删除、窗口关闭
* [ ] 在 Explorer 里往收纳文件夹放入 / 删除文件，收纳箱内容随之更新
* [ ] Win+D / Win+M / 任务栏"显示桌面"后收纳箱仍可见；不在任务栏、不参与 Alt+Tab；不盖住其他应用窗口；
      **第二次 Win+D 后其他应用窗口能正常恢复**（NoFences #14 的已知副作用需验证）
* [ ] 重启 Explorer 后所有收纳箱自动重挂回桌面层
* [ ] 固定后不能移动 / 缩放，但仍可拖入、折叠、改名；未固定时可移动 / 缩放
* [ ] 双击标题改名生效，收纳文件夹同步改名
* [ ] 折叠 / 展开生效；折叠态下悬停或拖文件经过时临时展开，移开收回
* [ ] 箱内拖动排序、Ctrl+滚轮图标大小、背景色 / 透明度、跨箱移动均生效并在重启后保留
* [ ] 托盘「显示/隐藏所有收纳箱」「重置收纳箱位置」生效
* [ ] 重启应用后所有收纳箱的位置、尺寸、标题、折叠 / 固定状态、外观、图标大小、排序全部恢复
* [ ] 两种屏幕组合下分别摆放收纳箱后，切换组合（运行中或重启后）各自恢复到对应位置；新组合下首次出现时位置在可见区域内
* [ ] Rust 计算的屏幕组合 ID 与前端 `generateScreenConfigId()` 一致；设置中删除屏幕配置后对应的收纳箱位置记录被删除

## Definition of Done

* Rust 单元测试覆盖纯逻辑（文件夹名清洗与去重、还原目标路径选择、重名后缀、默认位置计算、工作区夹取、排序合并等）
* `npm run build`（含 vue-tsc）与 `cargo check` 通过
* 桌面窗口行为按 `.trellis/spec/guides/desktop-window-e2e-guide.md` 实测并记录 e2e 结果
* 更新 `CLAUDE.md`（新表、新窗口类型、托盘菜单项、同步覆盖范围表）与 `.trellis/spec/backend/window-modes.md`（多窗口嵌入）

## Technical Approach

* **数据库**：迁移 v29 新增 `fences` 表（id、title、folder_name、x/y/width/height 物理像素、collapsed、locked、bg_color、bg_alpha、
  icon_size、sort_order、created_at/updated_at）与 `fence_items` 表（fence_id、file_name、original_path、sort_order），
  `fence_items` 只是排序和原始路径的附加信息，内容以文件夹为准，启动 / 变化时按文件夹对账
* **窗口**：Rust 侧 async 命令用 `WebviewWindowBuilder` 建窗（transparent、decorations(false)、skip_taskbar、visible(true)、
  always_on_bottom、minimizable(false)、maximizable(false)）；把 `window.rs` 的 `desktop_attach` / `tick_desktop_mode` /
  `reassert_window_mode_state` 泛化成按窗口执行，200ms 轮询线程遍历所有收纳箱窗口做 Explorer 重启重挂；
  （PR0 spike 结论：多个收纳箱共享 owner 时不会被成组重排，无需子类化 `WM_WINDOWPOSCHANGING`）
* **前端**：新路由 `#/fence?id=x` → `FenceView.vue`；标题栏按下并移动超过 3px 后 `startDragging()`（不用 `data-tauri-drag-region`）；
  外部文件拖入走 `onDragDropEvent`；箱内排序用 vuedraggable `force-fallback`
* **文件操作**：`IFileOperation` 移入 / 移回 / 跨箱移动；`notify`（已在 Cargo.toml、未使用）监听收纳根目录
* **图标**：STA 线程 `IShellItemImageFactory::GetImage` → `png` 0.17 编码 → 按路径哈希 + mtime 缓存到
  `%LOCALAPPDATA%\mini-todo\fence-icons\`，前端 `convertFileSrc` 加载；按图标大小和 DPI 取尺寸
* **启动**：`ShellExecuteExW`（`SEE_MASK_FLAG_NO_UI`）；"打开所在位置"用 `explorer /select,`
* **托盘**：`lib.rs` 新增菜单项与事件分发，收纳箱入口仅 `cfg(windows)`

### 实现计划（分 PR）

* **PR0 spike**：多个 WebView2 收纳箱的内存占用；N 个窗口挂 Progman 后 Explorer 重启重挂；第二次 Win+D 其他窗口恢复；
  图标 alpha 是否预乘 / `.lnk` 是否带箭头；开始菜单拖入是否有 `CF_HDROP`
* **PR1 窗口骨架**：v29 迁移、多窗口桌面嵌入泛化、托盘「新增收纳箱」、FenceView 标题栏（移动 / 缩放 / 固定 / 折叠 / 改名 / 删除）、重启恢复
* **PR2 文件层**：收纳目录、`IFileOperation` 移入 / 移回、`notify` 监听与对账、图标提取缓存、双击启动、项目右键菜单、箱内排序
* **PR3 增强项与收尾**：托盘兜底、外观、跨箱移动、图标大小、悬停展开；e2e 实测；CLAUDE.md / spec 更新

## Decision (ADR-lite)

**Context**：同类产品（Fences / 腾讯 / 360）让收纳的图标从桌面消失，靠的是接管 Explorer 桌面视图（需注入 Explorer），
不适合 mini-todo；纯引用模式（NoFences）零风险，但桌面不会变干净。

**Decision**：
1. 真收纳：物理移动文件（TuckPane 模式），每个收纳箱绑定可见文件夹 `%USERPROFILE%\MiniTodo收纳箱\<名称>\`，文件夹为 source of truth
2. 收纳箱始终嵌入桌面层，复用主窗口已有的 owner=Progman 方案，「固定」只表示锁定位置和大小
3. 首版纳入基础操作 + 托盘兜底、外观、跨箱移动、图标大小、悬停展开

**Consequences**：
* 文件路径会改变：依赖桌面路径的脚本或快捷方式会失效（收纳的主要是 `.lnk`，影响有限）
* OneDrive 桌面下移动会同步到其他设备；公共桌面项需要 UAC 确认
* 文件夹为 source of truth，卸载 / 崩溃 / 数据库丢失都不丢文件，最坏情况是用户在 `MiniTodo收纳箱` 文件夹里手动找回
* 每个收纳箱一个 WebView2 渲染进程（同源也不共享）。PR0 实测开发版私有内存：仅主窗口约 208MB，1 / 3 / 5 个收纳箱约 261 / 403 / 440–507MB。
  `--process-per-site` 可把 5 个降到约 336MB，但所有窗口共用一个渲染进程；**2026-10-15 用户决定先不加**，保持窗口间隔离
* Win10 / Win11 ≤23H2 上 Win+D 可能盖住收纳箱（与主窗口嵌入桌面相同的已知限制）

## Out of Scope

* 文件夹映射（Folder Portal：把任意已有文件夹显示成收纳箱）
* 完整的系统 Shell 右键菜单（首版只做自绘的几项）
* 从收纳箱拖出到桌面 / Explorer（OLE 拖放源）——用右键「移回桌面」代替；跨箱拖拽用右键「移动到…」代替
* 双击桌面空白处快速隐藏、按规则自动分类 / 一键整理、标签页、吸附对齐、Peek
* 收纳根目录自定义
* 收纳箱数据进 WebDAV 同步或导入导出（路径和坐标是设备相关数据，性质同 `screen_configs`）
* macOS / Linux 支持

## Technical Notes

* 已查看：`pc/src-tauri/src/lib.rs`（托盘、200ms 轮询线程）、`pc/src-tauri/src/commands/window.rs`（桌面模式 attach / detach / tick / reassert，
  全局 `IS_DESKTOP_MODE` / `DESKTOP_OWNER`，固定取 `main`）、`pc/src-tauri/tauri.conf.json`、`pc/src-tauri/capabilities/default.json`、
  `pc/src-tauri/Cargo.toml`（windows 0.58 当前 features：Win32_Foundation / UI_WindowsAndMessaging / Graphics_Dwm / Graphics_DirectWrite）、
  `services/notification.rs`（Rust 侧建窗先例）、`pc/src/router/index.ts`、`.trellis/spec/backend/window-modes.md`
* 应用目前没有接收系统文件拖放的功能；数据库迁移当前 v28（上游 2.5.0），收纳箱用 v29
* 实现阶段必须遵守的技术约束（来自 tech-feasibility）：
  * 建窗命令必须 async（Windows 上同步命令建窗会死锁）
  * 收纳箱显示 / 隐藏统一走 tao `show()` / `hide()`，建窗时一次性带好 flag，之后少改（tao 不同步 Win32 `ShowWindow` 的可见态）
  * 标题栏不用 `data-tauri-drag-region`（双击会最大化、冲掉嵌入样式）
  * 原生文件拖放与 HTML5 DnD 在 Windows 上互斥：外部拖入用 `onDragDropEvent`，箱内排序用 force-fallback
  * 位置存物理像素；先 set_position 再 set_size；显示器变化引发的系统挪窗不能落库（只在用户拖动结束后保存）
  * windows crate 需新增的 feature 见 `research/tech-feasibility.md` 第 7 节
* 后续派发 subagent 时提示词中需明确：不得在任何外部请求中携带用户邮箱

## Research References

* [`research/comparable-tools.md`](research/comparable-tools.md) — Fences / 腾讯 / 360 文件都留在 Desktop 文件夹、靠接管 Explorer 桌面视图
  隐藏被收纳项；只有 TuckPane 物理移动到真实目录；行业惯例"收纳区永远在桌面层 + 锁定是独立开关，锁定不禁拖入"；必备操作清单见 5.3
* [`research/tech-feasibility.md`](research/tech-feasibility.md) — 6 个主题均可行；主窗口 Progman 嵌入可推广到 N 窗口；
  移入文件的风险（OneDrive、公共桌面权限、重名、崩溃一致性）及"绑定真实文件夹"方案；待 spike 8 项见文档末尾

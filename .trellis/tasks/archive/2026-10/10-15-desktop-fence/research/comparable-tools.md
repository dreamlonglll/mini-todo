# Research: 桌面收纳箱（Desktop Fence）同类产品调研

- **Query**: 同类产品如何实现"桌面收纳箱"（托盘「新增收纳箱」→ 桌面上出现可放应用图标的区域，支持固定/嵌入桌面、折叠、改标题等）。重点：图标存储模型、窗口/Z 序模型、常见操作、新建默认尺寸/位置、用户痛点，以及对 mini-todo 的启示
- **Scope**: mixed（外部为主：官方文档 / 开源源码 / 社区帖；内部只做映射）
- **Date**: 2026-10-15
- **证据等级标注**：[官方] 厂商文档；[源码] 开源仓库代码/设计文档；[社区] 论坛/博客/issue；[推断] 由证据推出、未直接证实

---

## 0. 结论速览

1. **图标存储模型只有四种**，各产品取舍不同：
   - **引用（原图标留在桌面）**：NoFences —— 只存路径列表，桌面上的原图标不动，收纳箱里是"副本显示"。最简单，但用户最常抱怨"放进去了桌面上还在"。
   - **接管 / 过滤真实桌面图标（文件留在 Desktop 文件夹，只是不在桌面上显示）**：Stardock Fences、腾讯桌面整理、360 桌面助手、Pickets、LucidDesk。Fences 跑在 Explorer 进程内；LucidDesk 注入 Explorer 逐项移除视图成员；Pickets 整体隐藏桌面 SysListView32 后自绘全部桌面项。
   - **物理移动到收纳目录**：TuckPane —— 每个收纳窗口就是一个真实目录，删除时询问"整体移回桌面 / 留在原地"。
   - **文件夹映射（Folder Portal）**：Fences（Folder Portals）、腾讯（文件/磁盘映射）、360（映射文件夹）、Pickets、LucidDesk（文件夹面板）、TuckPane（从文件夹创建）都有；NoFences 没有。
2. **窗口模型**：独立进程做 fence 的开源实现普遍是 **顶层无边框窗口 + `WS_EX_TOOLWINDOW` + 压在桌面层**。NoFences 与 LucidDesk 用的正是 `GWLP_HWNDPARENT = Progman(GetShellWindow)` + `HWND_BOTTOM`，**与 mini-todo 主窗口现有的嵌入桌面方案同构**；Pickets 用 `SetWindowPos(insertAfter = WorkerW/Progman)`。Fences 在 Explorer 进程内运行（官方 FAQ："Windows Explorer ... which Fences run under"）。
3. **行业约定的"必备"操作**：拖入、双击打开、移动/缩放、改名、折叠（双击标题卷起 + 悬停展开）、锁定、删除收纳箱（不删文件）、移出单项、系统右键菜单。"双击桌面快速隐藏"、文件夹映射、规则自动分类、标签页、Peek 属于差异化功能。
4. **最集中的痛点**：图标"丢了"/恢复不回来、分辨率或多显示器变化后布局错乱、高 DPI 缩放下异常、Explorer 重启/Windows 更新后失效、Win+D 显示桌面时消失、收纳箱被拖出屏幕后找不回来。

---

## 1. 逐产品调研

### 1.1 Stardock Fences（v3 → v6，商业，Windows 10/11）

版本背景 [官方]：Fences 3（2016-05，高 DPI + 卷起到标题栏）→ Fences 4（2021-12，Peek + Desktop Display，启动时先于桌面加载）→ Fences 5（2023-12，Chameleon）→ Fences 6（2025-05，标签页 + 图标着色）。截至 2026-10 最新为 Fences 6.58（2026-10-14 发布，见 Stardock 帮助中心 changelog 列表）。来源：[Fences History](https://wiki.stardocksoftware.com/index.php?title=Fences_History)

**(1) 图标存储模型：接管真实桌面图标，文件始终留在 Desktop 文件夹**
- 官方定义："automatically placing your shortcuts and icons into resizable shaded areas on your desktop called fences"，与"Folder Portals allow you to mirror any folder to your desktop"并列——普通 fence 装的就是桌面本身的项目。[官方] [产品页](https://www.stardock.com/products/fences/)
- 用户丢了一个 fence 后："the icons that were inside it ... **remain in my "Desktop" file in Windows Explorer, but not on the screen**"；官方助理的处理是恢复布局快照，而不是去找被移走的文件 → 文件从没离开 Desktop 文件夹，Fences 只决定"显示在哪/是否显示"。[社区+官方回复] [Where do missing Fences go?](https://forums.stardock.com/496390/where-do-missing-fences-go-where-do-missing-icons-go)
- Fences 中的图标间距受系统 `HKCU\Control Panel\Desktop\WindowMetrics\IconSpacing` 影响（官方 KB），说明 fence 里显示的是桌面 ListView 的真实图标、沿用桌面图标度量。[官方] [Spacing on Icons has changed](https://support.stardock.com/space/SHC/1423736838)
- 用户反馈"fences will automatically move desktop icons out of the way"（fence 会把桌面图标挤开）、"安装 Fences 后桌面图标无法拖动、卸载后恢复"——都是直接操纵桌面图标位置的副作用。[社区] [Stop Fences from Moving Desktop Icons](https://forums.stardock.com/516530/stop-fences-from-moving-desktop-icons)、[Unable to move icons with Fences Installed](https://forums.stardock.com/494578/unable-to-move-icons-with-fences-installed)
- 删除 fence：鼠标悬停点右上角 X（需先在桌面右键 → 查看中取消"Lock Fences"）。Folder Portal 删除时"The actual folder will not be deleted"。[官方] [Fences UI: Folder Portals](https://support.stardock.com/space/SHC/1459421201)、[社区] [How to delete a fence](https://forums.stardock.com/501645/how-to-delete-a-fence-from-the-desktop)
- 卸载：早年（Fences 1，2009）有"多账户卸载后桌面图标全部消失、文件仍在 Desktop 文件夹"的帖子 [社区] [Missing icons after uninstalling Fences](https://forums.stardock.com/362746/missing-icons-after-uninstalling-fences/page/1)；官方提供 Purge 脚本做彻底清理。[官方] [Fences Support FAQ](https://forums.stardock.com/486084/fences-support-faq)
- **Folder Portal（文件夹映射）**：创建方式三种——按住 ALT 把文件夹拖到桌面；在桌面拖出矩形选"Create Folder Portal Here"；文件夹右键"Show this folder on the desktop"。Portal 内图标**不能手动排序**，只能"Sort By"；Ctrl+双击标题或双击空白处在资源管理器中打开；Ctrl+滚轮调图标大小；支持 Details/List 视图。[官方] [Fences UI: Folder Portals](https://support.stardock.com/space/SHC/1459421201)、[Fences 6 UI: Folder Portals](https://support.stardock.com/space/SHC/2814443832)

**(2) 窗口 / Z 序模型**
- Fences 运行在 Explorer 进程内：官方 FAQ 写明"Windows Explorer crash (which Fences run under)"；Explorer 异常退出后 Fences 弹"Fences has detected that Explorer was not closed properly"并**暂时自我禁用**。[官方] [Support FAQ](https://forums.stardock.com/486084/fences-support-faq)、[官方置顶帖](https://forums.stardock.com/507474/error-fences-has-detected-that-explorer-was-not-closed-properly)
- 具体是子窗口还是 owned 窗口官方未公开。[推断] 既然 fence 内显示的是桌面 ListView 真实图标，fence 的"阴影区域"大概率直接画在桌面窗口层内，因此天然随桌面一起出现在 Win+D 之后。
- Win+D：Fences 4 新增 **Desktop Display**——接管 Win+D，可设为"切换"或"按住显示"，宣称比系统 Win+D 更快。**Peek**（Win+Space）把所有 fence 提到所有窗口之上，再按一次或松开回到桌面层。[官方] [Fences 4 – A Guided Tour](https://www.stardock.com/blog/508751/fences-4--a-guided-tour)、[Fences 6 UI: Peek](https://support.stardock.com/space/SHC/2814443964)
- Windows 11 24H2 改了桌面窗口层级后，Fences 4 的 Peek 失效，官方建议升级 Fences 5。[官方] [Fences 4 Peek Feature on Windows 11 24H2](https://support.stardock.com/space/SHC/2093482017)

**(3) 常见操作** [官方]，来源：[Fences 6 UI: Welcome](https://support.stardock.com/space/SHC/2814443582)、[Roll-Up](https://support.stardock.com/space/SHC/1459159089)、[Fences 6 UI: Rollup](https://support.stardock.com/space/SHC/2814444059)、[Quick-Hide](https://support.stardock.com/space/SHC/1458241576)、[Fences 6 UI: Hide](https://support.stardock.com/space/SHC/2814444006)、[Layout & Snapping](https://support.stardock.com/space/SHC/1459060777)、[Fences 6 UI: Layout](https://support.stardock.com/space/SHC/2814444204)、[Personalization](https://support.stardock.com/space/SHC/2814443712)、[Tabs](https://support.stardock.com/space/SHC/2814444096)

| 操作 | Fences 的做法 |
|---|---|
| 新建 | 在桌面左键拖出矩形 → 菜单选「Create Fence here」/「Create Folder Portal Here」，然后输入标题；首次安装可一键套用默认三分区（程序 / 文件夹 / 文档） |
| 改名 | **鼠标移到标题上单击即可改名**（因为双击标题被卷起占用） |
| 移动 / 缩放 | 拖标题栏移动、拖边缘缩放；可开启吸附对齐、组间距 N 像素、尺寸按图标整数倍（1×1、2×2、3×2…） |
| 卷起（折叠） | **双击标题栏卷起到只剩标题**；**鼠标悬停时展开**，移开后收回；放到屏幕上/下边缘自动卷起（可关）；有标签组时可整组卷起 |
| 锁定 | 桌面右键 → 查看 →「Lock Fences」（全局锁，锁定后不能移动/缩放/删除）；**Fences 5/6 默认隐藏该菜单项**，需导入注册表 `HKCU\Software\Stardock\Fences\Settings\LegacyLockFencesAvailable=1` 才显示（[官方 KB](https://support.stardock.com/space/SHC/1621360668)）；[推断] 官方把它藏起来，可能是因为全局锁常让用户误以为"坏了"（官方 KB 排查"无法移动/缩放"的第一步就是检查它，见 [Cannot move, resize](https://support.stardock.com/space/SHC/1331396688)） |
| 快速隐藏 | **双击桌面空白处隐藏/显示所有图标与 fence**；可指定某个 fence 或单个图标不参与；模式三选一：全部隐藏 / 只隐藏未收纳图标 / 只隐藏 fence；可设"闲置自动隐藏"、"启动时总是显示" |
| 视图 / 排序 | 标题栏左上角汉堡菜单 → View / Sort（名称、类型、日期等）；Portal 支持 Details/List 视图 |
| 外观 | 每个 fence 单独的背景色（色相/亮度/饱和度/透明度）、取色器取壁纸色、标题字体颜色、圆角、描边；Chameleon 融入壁纸；Icon Tint 图标着色；样式可在 fence 间复制粘贴 |
| 标题栏控件 | 可设"鼠标悬停时才显示标题栏按钮"；靠近屏幕顶端时标题栏自动移到底部 |
| 标签页（v6） | 把一个 fence 拖到另一个上合并为标签组；Shift 拖动重排或撕出 |
| 规则 | 按类型 / 名称 / 目标 / 自定义规则把新出现的桌面项自动归到指定 fence |
| 多页 | Desktop Pages：拖屏幕最左/右边缘切换多页 fence |
| 布局备份 | 快照/备份，可"按显示器配置分别保存布局"（5.53 起新装默认开启） |
| 打开项目 | 与系统桌面一致（双击打开，系统右键菜单） |

**(4) 新建默认尺寸/位置**：由用户在桌面上拖出的矩形决定（不是固定默认值）。Fences 6 设置里可选择"如何创建新 fence"（拖拽手势的方式）。[官方] [Fences 6 UI: Welcome](https://support.stardock.com/space/SHC/2814443582)

**(5) 痛点** [社区/官方 KB]
- 图标 / fence "消失"：官方 KB 首先让用户检查是否误触了快速隐藏、是否被放到了另一个 Desktop Page，再用 Automatic Layout 或快照恢复。[官方] [Desktop icons or Fences are missing](https://support.stardock.com/space/SHC/1331822625)
- 多显示器 / 扩展坞：重启后 fence 跑到错误显示器、新建图标被送到错误显示器；官方方案是"按显示器配置保存布局"+ 重启 Explorer + 关掉 DisplayFusion 等多屏软件。[官方] [Fences going to wrong monitor after reboot](https://support.stardock.com/space/SHC/1344503818)、[Shortcuts or icons are being sent to the wrong monitor](https://support.stardock.com/space/SHC/1331494982)
- 分辨率变化后桌面图标被挤乱，用户不得不重排。[社区] [Stop Fences from Moving Desktop Icons](https://forums.stardock.com/516530/stop-fences-from-moving-desktop-icons)
- 高 DPI：Fences 4 在 **≥225% 缩放**下卷起 fence 后原图标仍留在桌面上可见，官方建议"降到 200% + 大图标"。[社区+官方回复] [Closed fence does not remove the icon](https://forums.stardock.com/510353/closed-fence-does-not-remove-the-icon-from-the-desktop)
- fence 标题栏被拖出屏幕后无法移动也无法删除，只能靠快照（且只保留最近 3 份）。[社区] [Remove a Fence from Desktop](https://forums.stardock.com/491487/remove-a-fence-from-desktop)
- Explorer 崩溃 / 重启（驱动安装、其他美化软件）→ Fences 自我禁用；与 WindowBlinds、DisplayFusion 等冲突。[官方] [置顶帖](https://forums.stardock.com/507474/error-fences-has-detected-that-explorer-was-not-closed-properly)
- Office 保存时先写临时文件再改名，生成"新"文件而跑出 fence；Portal 指向网络路径时偶发为空；HDR 下 Portal 改名框空白。[官方] [Office files](https://support.stardock.com/space/SHC/1331232918)、[Folder portals empty](https://support.stardock.com/space/SHC/1332805638)、[HDR issues](https://support.stardock.com/space/SHC/1770422308)

---

### 1.2 NoFences（Twometer/NoFences，开源，C# WinForms）

仓库：<https://github.com/Twometer/NoFences>（692 star，最后推送 2024-04-10，README 只有一句"Didn't want to pay 11€, made my own"）。代码量很小，是"最小可用 fence"的最佳参照。下文行号对应 `main` 分支，约数。

**(1) 图标存储模型：纯引用（只存路径），原文件和桌面原图标都不动**
- `Model/FenceInfo.cs`：`public List<string> Files`——每个 fence 只是一组绝对路径；另存 `Name/PosX/PosY/Width/Height/Locked/CanMinify/TitleHeight`。[源码]
- `Model/FenceManager.cs`：每个 fence 一个目录 `%LOCALAPPDATA%\NoFences\{Guid}\__fence_metadata.xml`（XmlSerializer）；`RemoveFence` 只 `Directory.Delete` 这个元数据目录 → **删除 fence 不碰任何用户文件**。[源码]
- `FenceWindow.cs` `FenceWindow_DragDrop`（约 183-191 行）：拖入时只把 `DataFormats.FileDrop` 的路径加进 `Files`（去重 + 存在性检查），`DragEnter` 里返回 `DragDropEffects.Move`，但没有执行任何文件操作。[源码]
  - 按微软 Shell 拖放约定，源（Explorer）只有在 `DoDragDrop` 返回值 **和** `CFSTR_PERFORMEDDROPEFFECT` 都为 `DROPEFFECT_MOVE` 时才会删除原文件；NoFences 没写后者，所以文件不会被删。[官方] [Handling Shell Data Transfer Scenarios](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios)
- `FenceEntry.FromPath`：文件不存在就返回 null、绘制时跳过（v1.3 修复：原文件改路径后 fence 崩溃并丢失全部项目）。[源码] + [社区] [#3 Moving files - missing reference](https://github.com/Twometer/NoFences/issues/3)
- **后果**：用户最常问"放进 fence 后桌面上的图标还在，怎么去掉？"，社区答案是"桌面右键 → 查看 → 取消『显示桌面图标』"，等于手动把整张桌面交给 fence。[社区] [#69](https://github.com/Twometer/NoFences/issues/69)
- 没有 Folder Portal（[#28 Add a folder as a fence](https://github.com/Twometer/NoFences/issues/28) 仍 open）。

**(2) 窗口 / Z 序模型：顶层无边框窗口 + owner=Progman + 工具窗口 + 压底**
- `Win32/DesktopUtil.cs` `GlueToDesktop`：`FindWindowEx(0,0,"Progman",null)` 后 `SetWindowLong(handle, GWL_HWNDPARENT, progman)`。[源码]
- `Win32/WindowUtil.cs` `HideFromAltTab`：`SetWindowPos(HWND_BOTTOM, NOSIZE|NOMOVE|NOACTIVATE)` + 加 `WS_EX_TOOLWINDOW`。[源码]
- `FenceWindow.WndProc`：拦 `WM_SETFOCUS` 时再次 `SetWindowPos(HWND_BOTTOM)` 防止被激活到前台；拦 `WM_NCCALCSIZE(0x83)` 去边框；拦 `SC_MAXIMIZE`；非锁定时用 `WM_NCHITTEST` 把标题区域映射为 `HTCAPTION`（拖动）、四边四角 10px 映射为缩放热区。[源码]
- 这套方案是社区在 issue 里摸索出来的：最初 fence 在"显示桌面"时消失，作者认为 WinForms 无解；后来改成 `GWL_HWNDPARENT=Progman` 才修好（PR #14）。[社区] [#2](https://github.com/Twometer/NoFences/issues/2)、[#14](https://github.com/Twometer/NoFences/pull/14)
  - **副作用（重要）**：PR #14 测试者反馈"再点一次显示桌面按钮，其他窗口不再恢复"，以及"过一段时间后又会在显示桌面时隐藏，切出任意窗口后恢复"；作者认为"annoying but worth it"。[社区] [#14 评论](https://github.com/Twometer/NoFences/pull/14)
  - 另有"fence 偶发跑到所有窗口之上，约 30 分钟一次，点一下 Chrome 又回去"。[社区] [#55](https://github.com/Twometer/NoFences/issues/55)
- 没有处理 Explorer 重启（代码中无 TaskbarCreated / owner 重挂逻辑）。[源码，未找到相关代码]

**(3) 常见操作**（右键菜单：移除项目、锁定、自动收起 Minify、改名、标题高度、新建 fence、删除 fence）[源码 `FenceWindow.Designer.cs`]
- 改名：菜单 → 弹对话框（`EditDialog`）。
- 折叠：**Minify 选项开启后，鼠标离开即收成标题栏、鼠标进入即展开**（`Minify()` / `FenceWindow_MouseEnter`）；没有"双击标题卷起"。
- 锁定：锁定后禁止拖动/缩放，且**拖入也被禁止**（`DragEnter` 检查 Locked）——曾让用户以为"拖不进去"。[社区] [#3 评论](https://github.com/Twometer/NoFences/issues/3)
- 打开：**双击**打开（`Process.Start`，文件夹走 `explorer.exe`）；单击仅选中，悬停高亮。
- 右键项目：弹**完整系统 Shell 右键菜单**（`Win32/ShellContextMenu.cs`，约 1500 行，取自 CodeProject）；**Shift+右键**才是 fence 自己的菜单。作者承认"Programatically opening the context menu on Windows is a huge pain"。[源码] + [社区] [#23 评论](https://github.com/Twometer/NoFences/issues/23)、[#65 评论](https://github.com/Twometer/NoFences/issues/65)
- 删除 fence：确认框"Really remove this fence?"。
- 其余：图标缩略图（图片文件）、滚轮滚动、亚克力模糊 + 阴影、深色右键菜单。
- 缺失（均为 open issue）：快速隐藏 [#56](https://github.com/Twometer/NoFences/issues/56)、托盘图标/退出 [#25](https://github.com/Twometer/NoFences/issues/25)、开机自启 [#40](https://github.com/Twometer/NoFences/issues/40)、fence 内重排 [#26](https://github.com/Twometer/NoFences/issues/26)/[#60](https://github.com/Twometer/NoFences/issues/60)、调图标大小 [#61](https://github.com/Twometer/NoFences/issues/61)、显示扩展名 [#38](https://github.com/Twometer/NoFences/issues/38)、Win11 新式右键菜单 [#29](https://github.com/Twometer/NoFences/issues/29)。

**(4) 新建默认尺寸/位置**：`FenceManager.CreateFence` 固定 `PosX=100, PosY=250, Width=300, Height=300`；标题高度默认 35（逻辑像素）；图标格 75px 宽。首次启动没有 fence 时自动建一个"First fence"。[源码]
- 用户抱怨：新 fence 总出现在同一位置、与已有 fence 重叠；建议出现在不重叠的位置并置顶；删除确认框应包含 fence 名称以防误删。[社区] [#77](https://github.com/Twometer/NoFences/issues/77)

**(5) 痛点**：显示桌面时消失（已修，见上）、高 DPI 模糊（[#23](https://github.com/Twometer/NoFences/issues/23)、[PR #32](https://github.com/Twometer/NoFences/pull/32)）、文件很多时滚动卡顿（[#70](https://github.com/Twometer/NoFences/issues/70)）、"从 fence 删除"误删真实文件的担忧（[#65](https://github.com/Twometer/NoFences/issues/65)）、杀软误报（[#10](https://github.com/Twometer/NoFences/issues/10)、[#67](https://github.com/Twometer/NoFences/issues/67)）。

---

### 1.3 腾讯桌面整理（DeskGo，原腾讯电脑管家插件，现有独立版 / 微软商店版）

术语：收纳区叫 **"格子"**。来源：[官网](https://guanjia.qq.com/product/zmzl/)、[微软商店](https://apps.microsoft.com/detail/xp9ccdndd9s99t)

**(1) 图标存储模型：文件留在桌面，只做视觉分区** [社区，较可信] + [推断]
- 多篇教程称"文件还是在桌面，只是视觉上被分区管理"，可"恢复桌面 / 停用整理"回到原状。（该 CSDN 文章明显是 SEO 体，可信度一般）[社区] [CSDN 教程](https://blog.csdn.net/2401_89926894/article/details/155032563)
- 布局数据存在 `%APPDATA%\Tencent\DeskGo\` 下的 `ConFile.dat`、`DesktopMgr.lg`、`FencesDataFile.dat`，并自动轮转 `FencesDataFile_temp_x` 备份；进程名 `DesktopMgr`。外接显示器 / 分辨率变化后"整理好的文件错乱"，用户靠备份这三个文件来恢复。→ [推断] 它记录的是桌面项目到格子/坐标的映射，而不是把文件搬走。[社区] [腾讯桌面整理图标错乱恢复](https://jokerbyrant.github.io/2024-08-28/13172/)、[CSDN](https://blog.csdn.net/Asi_welldone/article/details/130840597)
- 有用户卸载后"桌面图标全部消失"，在桌面右键 → 查看 → 勾选「显示桌面图标」后恢复 → [推断] DeskGo 运行时会隐藏系统桌面图标层（类似 Pickets 的"整桌接管"），异常卸载会留下这个状态。[社区] [CSDN](https://blog.csdn.net/haodiaoer/article/details/121470652)
- **文件映射 / 磁盘映射**：把其他文件夹或磁盘镜像为桌面上的一个格子，内容保持同步（即 Folder Portal）。[社区] [v1tx 评测](https://www.v1tx.com/post/tencent-zmzl/)、[官网](https://guanjia.qq.com/product/zmzl/)

**(2) 窗口 / Z 序模型**：未找到公开技术资料。Win11 某次补丁后"打开后电脑会黑屏或者图标不出现"，需结束「桌面整理-主程序」或卸载补丁 → 说明它深度依赖桌面窗口结构，系统更新容易把它打断。[社区] [Microsoft Q&A](https://learn.microsoft.com/zh-cn/answers/questions/4089281/win11)

**(3) 常见操作** [社区] [v1tx 评测](https://www.v1tx.com/post/tencent-zmzl/)、[腾讯云文章](https://cloud.tencent.com/developer/article/2583499)
- 入口：**桌面右键 → 腾讯桌面整理 → 新建格子 / 一键桌面整理**（按应用、文档、文件夹等自动分到格子）。
- 拖动格子移动、调整大小、把桌面文件拖进格子。
- **双击格子标题可修改或删除格子名称**。
- **鼠标悬停到标题栏时出现按钮：锁定格子、收起格子、更改列表排版**。
- 格子可折叠隐藏；扩展名规则自定义分类；附带全盘搜索、壁纸、便签等。

**(4) 默认尺寸/位置**：未找到资料。

**(5) 痛点**：多屏/分辨率切换后图标错乱（最常见，需要手工备份配置文件）；Win11 更新后失效；异常卸载后桌面图标被隐藏。

---

### 1.4 360 桌面助手（360 安全卫士内置 / 独立版，"收纳盒"）

**(1) 图标存储模型：文件留在桌面，收纳盒只是视图** [社区]
- 新建：桌面右键 → 桌面助手 →「新建收纳盒」；收纳盒右上角"三横"菜单 →「**解散收纳盒**」。[社区] [百度经验](https://jingyan.baidu.com/article/915fc4145bd75d51384b2070.html)
- 用户反馈"桌面收纳盒过多后，直接解散，则会**散布桌面**"——解散后文件回到桌面原位，说明文件一直在 Desktop 文件夹里。[社区] [CSDN 意见反馈](https://blog.csdn.net/qq_57484399/article/details/142726500)
- 想恢复原桌面：右上角"三横"→「退出桌面助手」即可。[社区] [百度经验](https://jingyan.baidu.com/article/546ae185279e815049f28caf.html)
- 运行时**桌面右键菜单中的"将图标与网格对齐"被去掉，并强制开启"自动排列图标"**，退出桌面助手后才恢复 → [推断] 直接接管系统桌面 ListView 的排列状态。[社区] [CSDN](https://blog.csdn.net/m0_45047336/article/details/120965173)
- **映射文件夹**：另一种收纳盒，需要手动粘贴磁盘路径来建（即 Folder Portal）；用户希望能"一键把收纳盒转成映射文件夹"（把盒内文件打包进同名文件夹）。[社区] [CSDN 意见反馈](https://blog.csdn.net/qq_57484399/article/details/142726500)

**(2) 窗口 / Z 序模型**：未找到公开资料。

**(3) 常见操作**：随意移动、改变大小；**右上角锁定按钮锁定大小和位置**；增删收纳盒、自定义名称；一键"桌面整理"按类型生成收纳盒。[社区] [果核剥壳](https://www.ghxi.com/360desktop.html)

**(4) 默认尺寸/位置**：未找到资料。

**(5) 痛点** [社区]
- 接扩展显示器后"桌面图标不能显示归集整理框里，全铺开了"，拔掉显示器才恢复。[CSDN](https://blog.csdn.net/weixin_36012215/article/details/118327758)
- "关闭显示器后再点亮，桌面图标会排列混乱"，双屏更严重，重启或开关一个屏幕后便签/图标位置 100% 乱，只能靠桌面备份恢复；官方回复"双屏目前还没有解决"。[360 社区](https://bbs.360.cn/thread-16179670-1-1.html)
- 运行后收纳盒和图标全部变透明（标题提及）。[360 社区](https://bbs.360.cn/thread-16163840-1-1.html)

---

### 1.5 其他开源实现（精选三个，覆盖另外几种存储模型）

| 项目 | 技术栈 | 存储模型 | 窗口/Z 序 | 值得借鉴的点 |
|---|---|---|---|---|
| [Pickets（原 OpenFences）](https://github.com/chrisdfennell/Pickets) | C# WPF，.NET 8 | **整桌接管**："Nothing is copied or moved and no shortcuts are created"；首次把桌面项放进 fence 后，向 `SHELLDLL_DefView` 发 `WM_COMMAND 0x7402`（即"显示桌面图标"开关）**隐藏整个桌面图标层**，所有未收纳项目进入一个 "Desktop" fence 自绘；退出时同步恢复；崩溃后提供 `Pickets.exe --restore-icons` 和开始菜单「Restore desktop icons」 | `SetWindowPos(insertAfter = WorkerW 或 Progman)` 放进"桌面层"+ `WS_EX_TOOLWINDOW`；README 承认"Z-order on the desktop can vary by Windows build" | 右键拖矩形新建；双击标题卷起、悬停或拖文件经过时临时展开；✕ = 关闭（保留内容，可从主窗口重新打开），「Delete Fence…」= 删除（项目回到 Desktop fence，不删磁盘文件）；Ctrl+滚轮调图标大小；「Fit to contents」；吸附 + Alt 自由移动；锁定；Ctrl+Z 撤销 30 步；按显示器配置记忆位置且"never left off-screen"；按显示器 DPI 渲染；系统右键菜单 + 「Remove from fence」；改名改的是真实文件 |
| [LucidDesk](https://github.com/Yuch3nE/luciddesk) | **Rust**，windows-sys，自绘 | **逐项过滤**：DLL 通过 `WH_GETMESSAGE` 钩子进入 Explorer 桌面线程，调用 `IShellFolderView::RemoveObject` 从原生视图移除被收纳的项目；文件不动、不切换自动排列；退出时按原坐标重新加入；会话失效后重连（`docs/development/hybrid-desktop.md`）。README："面板不会搬动你的文件……关闭面板也不会删除原文件" | `SetWindowLongPtrW(GWLP_HWNDPARENT, GetShellWindow())` + `HWND_BOTTOM` + `WS_EX_TOOLWINDOW`（`app/src/pane/window/mod.rs`）；子类化 `WM_WINDOWPOSCHANGING`，给被动重排补 `SWP_NOZORDER`、给主动抬升补 `SWP_NOOWNERZORDER`，防止共享 owner 的多个面板被系统成组重排（`docs/development/pane-drag-order.md`）；过滤会话监听 Explorer 进程退出、在 TaskbarCreated 之前唤醒重连（`crates/luciddesk-explorer/src/filter/explorer.rs`） | **入口就是托盘菜单「新建面板」/「新建文件夹面板…」**；把图标拖回桌面即移出；锁定 = 禁止移动和改名但仍可缩放；折叠 / 自动收起 / 吸附 / 置顶；默认图标格 88×96 DIP，可在 50%–200% 间缩放；列表视图；标签页合并 |
| [TuckPane](https://github.com/ch998244353/TuckPane) | C# / WinUI | **物理移动**：每个收纳窗口对应一个真实目录 `%USERPROFILE%\TuckPane\Windows\Name-ID`，文件直接存在里面；拖放时协商复制/移动/链接；**删除收纳窗口时询问"把整个目录移回桌面"还是"文件留在原处"**；卸载不删任何文件 | 浮动 / 桌面定位两种模式 + 屏幕边缘停靠的 Station | 桌面右键「新建收纳窗」；`--create-organizer` 生成一个**默认收起、3×3 图标布局**的浮动窗格；单击已展开的标题原地改名（Enter 确认 / Esc 取消 / 失焦提交）；可选悬停展开、离开收起、同时只展开一个 |

---

## 2. 横向对比

### 2.1 图标存储模型

| 产品 | 拖入后原文件 | 桌面上原图标 | 删除收纳区 | 卸载/退出 | 文件夹映射 |
|---|---|---|---|---|---|
| Stardock Fences | 留在 Desktop 文件夹 | 由 fence 接管显示（不再出现在 fence 外） | X 删除；Portal 删除不删文件夹 | 早年有卸载后桌面图标"消失"的报告；有 Purge 工具 | ✓ Folder Portals |
| NoFences | 不动 | **仍在桌面（重复显示）** | 只删元数据 | 不影响桌面 | ✗ |
| 腾讯桌面整理 | 留在桌面 [社区] | 由格子接管 [推断] | 未找到资料 | 异常卸载后可能需要手动勾选「显示桌面图标」 | ✓ 文件/磁盘映射 |
| 360 桌面助手 | 留在桌面 | 由收纳盒接管 | 解散 → 图标散回桌面 | 退出即恢复原桌面 | ✓ 映射文件夹 |
| Pickets | 不动 | 整个桌面图标层被隐藏，自绘全部项目 | 项目回到 Desktop fence | 退出即恢复；崩溃后需 `--restore-icons` | ✓ |
| LucidDesk | 不动 | 只移除被收纳的那几项（注入 Explorer） | 关闭面板释放收纳关系 | 退出时按原坐标恢复 | ✓ 文件夹面板 |
| TuckPane | **移动到收纳目录** | 消失（文件已搬走） | 询问：整体移回桌面 / 留在原处 | 不删文件 | ✓ 从文件夹创建 |

### 2.2 窗口 / Z 序

| 产品 | 方式 | Win+D / 显示桌面 |
|---|---|---|
| Fences | 跑在 Explorer 进程内 [官方]；细节未公开 | 随桌面一起显示；v4+ 可用 Desktop Display 接管 Win+D、用 Peek 把 fence 提到最前 |
| NoFences | 顶层窗口，`GWL_HWNDPARENT=Progman` + `HWND_BOTTOM` + `WS_EX_TOOLWINDOW` | 修复后不再消失；但再次显示桌面时其他窗口不恢复、偶发跑到最上层 |
| Pickets | 顶层窗口，`SetWindowPos(insertAfter=WorkerW/Progman)` + `WS_EX_TOOLWINDOW` | 未专门说明；README 称 Z 序随 Windows 版本而异 |
| LucidDesk | 顶层窗口，`GWLP_HWNDPARENT=GetShellWindow()` + `HWND_BOTTOM` + `WS_EX_TOOLWINDOW` + 拦 `WM_WINDOWPOSCHANGING` | 未专门说明 |
| 腾讯 / 360 | 未找到资料 | 未找到资料 |
| **mini-todo 主窗口（现有）** | 顶层窗口，owner=Progman（`GWLP_HWNDPARENT`）+ tao `always_on_bottom` + `WS_EX_TOOLWINDOW` + 200ms 轮询重挂 | Win11 24H2+ 实测免疫；Win10 / ≤23H2 降级 |

### 2.3 常见操作矩阵（✓ 有 / ✗ 无 / ? 未找到资料）

| 操作 | Fences | NoFences | 腾讯 | 360 | Pickets | LucidDesk |
|---|---|---|---|---|---|---|
| 新建入口 | 桌面拖矩形 | fence 右键菜单 | 桌面右键 | 桌面右键 | 桌面右键拖矩形 / 主窗口 | **托盘菜单** |
| 改名 | 单击标题 | 菜单 → 对话框 | 双击标题 | ✓ | 标题右键 / ✎ | ✓（锁定时禁止） |
| 折叠 | 双击标题卷起 + 悬停展开 + 贴边自动卷起 | 鼠标离开自动收起 | 悬停标题出现"收起" | ? | 双击标题 + 悬停展开 | ✓ 折叠 / 自动收起 |
| 锁定 | 全局锁（v5/6 默认隐藏） | 单个 fence 锁 | 单个格子锁 | 单个收纳盒锁 | 单个锁 | 单个锁 |
| 缩放 / 移动 | ✓ + 吸附 + 整数倍 | ✓ | ✓ | ✓ | ✓ + 吸附 | ✓ + 吸附 |
| 删除收纳区 | ✓（X） | ✓（确认框） | ✓ | 解散 | 关闭 / 删除分开 | 关闭 |
| 移出单项 | 拖回桌面 | Shift+右键 → 移除 | ? | ? | 右键 Remove from fence | 拖回桌面 |
| 双击打开 | ✓ | ✓ | ✓ | ✓ | ✓（Enter） | ✓ |
| 系统右键菜单 | ✓ | ✓ | ? | ? | ✓ | ✓ |
| 图标大小 | ✓ | ✗ | ? | ? | Ctrl+滚轮 | 全局 50–200% |
| 排序 | ✓ | ✗（PR 未合） | 列表排版 | ? | ✓ + 手动排序 | ✓ |
| 快速隐藏（双击桌面） | ✓ | ✗ | ? | ? | ✓（可选）+ Ctrl+Alt+H | 快捷键显示所有面板 |
| 颜色 / 透明度 | ✓ 很全 | 固定半透明黑 | ? | ? | ✓ | ✓ |
| 文件夹映射 | ✓ | ✗ | ✓ | ✓ | ✓ | ✓ |
| 按显示器记忆布局 | ✓（5.53 起默认） | ✗ | ✗（痛点） | ✗（痛点） | ✓ | ? |

---

## 3. 新建时的默认尺寸 / 位置

| 产品 | 规则 |
|---|---|
| Fences / Pickets | 用户在桌面上拖出的矩形 = 初始位置和尺寸 |
| NoFences | 固定 (100, 250)，300×300；被抱怨与已有 fence 重叠（#77） |
| TuckPane | 在所选显示器上建一个**默认收起**的浮动窗格，3×3 图标布局，遵循已有的放置规则 |
| LucidDesk | 托盘新建；图标格默认 88×96 DIP（具体初始位置未查到） |
| Pickets「Move to new fence」 | 在当前 fence 旁边新建 |

---

## 4. 痛点汇总（按出现频率）

1. **"图标丢了"恐慌**：fence 消失 / 卸载 / 崩溃后图标不在桌面上（Fences、腾讯、Pickets 需提供恢复命令）；反过来，纯引用模型又被抱怨"桌面上还在"（NoFences #69）。
2. **多显示器 / 分辨率 / 显示器休眠唤醒后布局错乱**：Fences、腾讯、360 都有大量帖子，用户靠手动备份配置文件度日。
3. **收纳区被拖出屏幕外后找不回、删不掉**（Fences 帖子）。
4. **高 DPI**：Fences 4 ≥225% 异常；NoFences 早期模糊。
5. **Explorer 重启 / Windows 更新改桌面层级**：Fences 自我禁用、Fences 4 Peek 在 24H2 失效、腾讯在 Win11 补丁后黑屏/无图标。
6. **显示桌面（Win+D）时消失**，以及修好后的副作用（再次 Win+D 其他窗口不恢复、偶发跑到最上层）——NoFences #2 / #14 / #55。
7. **锁定把拖入也禁掉了**，用户误以为功能坏了（NoFences #3）。
8. **删除确认不够明确**，怕误删（NoFences #77），以及"从收纳区移除会不会删掉真实文件"的担心（#65）。
9. **大量文件时滚动卡顿**（NoFences #70）。

---

## 5. 对 mini-todo 的启示

### 5.1 需要先拍板的产品决策：图标存储模型

| 方案 | 代表 | 原图标 | 复杂度 | 主要风险 | 评价 |
|---|---|---|---|---|---|
| **A 纯引用** | NoFences | 留在桌面（重复显示） | 低：只存路径 | 用户觉得"没收进去" | **MVP 推荐**：零数据风险，删除/卸载不影响任何文件 |
| **A+ 引用 + 可选"隐藏系统桌面图标"开关** | NoFences 用户手动做法、Pickets 的简化版 | 开关打开后整张桌面图标层被隐藏 | 低-中：`WM_COMMAND 0x7402` 发给 `SHELLDLL_DefView` 切换（与"查看 → 显示桌面图标"等价，Pickets 已验证）；退出/崩溃要恢复 | 会把**未收纳**的图标一起藏掉；崩溃后需要托盘/启动参数恢复 | **P1 推荐**：便宜，契合"桌面整洁"的初衷，与"平台专属能力做成设置开关"的约定一致 |
| B 整桌接管（自绘全部桌面项） | Pickets | 全部由程序绘制 | 中-高：要自绘所有桌面项，还要处理新文件、回收站等 | 工作量大 | 不建议首版做 |
| C 逐项过滤（注入 Explorer） | LucidDesk；（推断）Fences / 腾讯 / 360 | 只藏被收纳的项 | 很高：DLL 注入 + `IShellFolderView` + 恢复事务 | Explorer 稳定性、杀软、Windows 更新 | 不建议（违背 mini-todo"轻量待办工具"定位） |
| D 物理移动到收纳目录 | TuckPane | 消失 | 中 | 改变文件路径（依赖桌面路径的快捷方式/脚本会失效）；删除/卸载必须回迁 | 可作为显式的"移动到收纳箱文件夹"可选动作，不宜作为默认 |
| E 给文件加隐藏属性 | 调研对象中未见采用 | 消失（开启"显示隐藏文件"时又会出现） | 低 | 改动用户文件属性；卸载残留会被当成"文件丢了" | 不建议 |

> 建议向用户确认：首版是否接受"原图标仍留在桌面（A）"，配合可选的"隐藏系统桌面图标"开关（A+）。

### 5.2 窗口 / Z 序：直接复用现有嵌入桌面方案，但要从"只管主窗口"改成"管多个窗口"

- 业界开源实现（NoFences、LucidDesk）用的就是 **owner=Progman + `HWND_BOTTOM` + `WS_EX_TOOLWINDOW`**，与 `pc/src-tauri/src/commands/window.rs` 的 `desktop_attach`（1082 行起）同构；Win11 24H2 层级变化的背景见 `.trellis/tasks/archive/2026-09/09-07-desktop-embed-mode/research/win32-desktop-embed.md`。
- 现有实现是**单窗口**的：`IS_DESKTOP_MODE` / `DESKTOP_OWNER` 是全局静态变量，`set_window_desktop_mode` 固定取 `"main"`，`tick_desktop_mode` 只轮询主窗口。收纳箱需要：
  - 每个收纳箱一个独立 WebviewWindow（例如 label `fence-{id}`），透明、无边框、`skipTaskbar`；
  - 把 attach / detach / reassert / tick 泛化成按窗口执行（每个窗口一份 owner 状态），Explorer 重启时所有收纳箱一起重挂；
  - 多个窗口共享同一个 owner 时，系统可能在菜单关闭或激活变化后成组重排它们 → 参考 LucidDesk：子类化 `WM_WINDOWPOSCHANGING`，被动重排补 `SWP_NOZORDER`、主动抬升补 `SWP_NOOWNERZORDER`。
- 需要实测的风险（来自 NoFences #14 / #55）：① 第二次 Win+D 时**其他应用窗口能否恢复**（现有 e2e 只验证了 mini-todo 自己的窗口状态，见 `09-07-desktop-embed-mode/e2e-progress.md` 第 31 行）；② 长时间运行后是否偶发跑到最上层。
- 行业约定是"收纳区永远在桌面层"，"锁定"是另一个独立开关（Fences / NoFences / 腾讯 / 360 / LucidDesk 都是这样）。用户说的"固定（嵌入到桌面中）"可以理解为"嵌入 + 锁定"，也可以拆成两个开关——**需和用户确认**。参考做法：收纳箱默认就嵌入桌面，"锁定"只禁止移动/缩放/改名，**不禁止拖入**（避开 NoFences #3 的误解）。

### 5.3 MVP 功能清单

**必备（缺了会被认为"不能用"）**
1. 托盘「新增收纳箱」：建在**托盘/光标所在显示器**的工作区内，在上一个收纳箱基础上偏移、避开已有收纳箱（避开 NoFences #77 的固定位置问题）；默认尺寸约能放 4 列 × 2–3 行图标（参考 LucidDesk 每格 88×96 DIP、TuckPane 3×3）。
2. 拖入：从桌面或资源管理器拖文件/快捷方式/文件夹进来 → 记录路径，去重；原路径失效时灰显 + 提供"移除"，不能因此崩溃或丢掉其他项（NoFences #3）。
3. 双击打开（exe / lnk / url / 文件夹 / 普通文件），单击选中、悬停高亮——与系统桌面一致。
4. 移动（拖标题栏）、缩放（拖边缘），位置/尺寸持久化；位置**限制在可见工作区内**。
5. 改名：Fences 是"单击标题改名 + 双击标题卷起"，腾讯是"双击标题改名"。二选一并统一，推荐原地编辑（Enter 确认 / Esc 取消 / 失焦提交，参考 TuckPane、LucidDesk）。
6. 折叠/展开：至少提供"卷起到标题栏"（标题栏按钮或双击）；"悬停临时展开"可作为选项。
7. 锁定（单个收纳箱）。
8. 删除收纳箱：确认框里写出**收纳箱名称**，并明确告知"不会删除任何文件"（NoFences #77、Pickets / LucidDesk 的措辞）。
9. 移出单项：项目右键 →「从收纳箱移除」。
10. 嵌入桌面：Win+D 免疫、Explorer 重启自动重挂（复用现有机制）。
11. 托盘兜底：「显示/隐藏所有收纳箱」「重置收纳箱位置」（对应 Fences"标题栏拖出屏幕删不掉"的痛点）。

**锦上添花（可放到后续迭代）**
- 项目右键弹**完整系统 Shell 菜单**（NoFences 为此搬了约 1500 行代码；首版可先只做"打开 / 打开所在位置 / 从收纳箱移除 / 属性"）。
- 收纳箱内拖动排序、跨收纳箱拖动（WebView 之间的 HTML5 拖放不能跨窗口，首版可用右键「移动到…」代替）。
- 图标大小（Ctrl+滚轮）、排序方式、列表视图。
- 每个收纳箱单独的颜色/透明度（可复用 v26 的 `window_bg_color` / `window_bg_alpha` 思路）。
- 双击桌面快速隐藏（需要桌面鼠标钩子 + 判断点在空白处，Pickets 有 `DesktopDoubleClickMonitor`）。
- 文件夹映射（Folder Portal）、按规则自动分类 / 一键整理、吸附对齐、标签页、Peek。
- A+ 的"隐藏系统桌面图标"开关（见 5.1）。
- 按显示器配置分别记忆布局（Fences 5.53 起默认开启；腾讯/360 没做因此被大量吐槽）。

### 5.4 实现层面的注意点

- **拖入不会误删源文件**：wry 的 `IDropTarget`（项目当前用 wry 0.53.5）在 `DragEnter` 返回 `DROPEFFECT_COPY`，`Drop` 不设置 performed effect，因此 Explorer 不会做"未优化移动后删源"（源只有在两个值都是 MOVE 时才删除）。光标会显示"复制"角标，和 Fences 的"移动"体验略有不同。[源码] [wry drag_drop.rs](https://github.com/tauri-apps/wry/blob/dev/src/webview2/drag_drop.rs)、[官方] [Shell 数据传输场景](https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios)（需真机复核）
- **图标**：Rust 侧用 Shell API 取带 alpha 的大图标（如 `IShellItemImageFactory` / `SHGetFileInfo`），按"路径 + 修改时间"缓存后交给前端；显示名去掉 `.lnk` / `.url` 后缀（桌面惯例，Pickets 同样处理）。
- **打开**：`ShellExecuteW("open")`；"打开所在位置"用 `explorer /select,`。
- **改名跟踪**：纯引用模型下，文件在资源管理器里被改名后路径就失效了（NoFences #3）。LucidDesk 用文件 ID 跟踪改名，首版可以只做"失效灰显 + 手动移除"。
- **每个收纳箱一个 WebView2**：多个收纳箱 = 多个 WebView2 实例，内存需要实测；必要时限制数量或做轻量页面。
- **DPI / 多显示器**：位置按物理像素 + 所在显示器保存，启动时校验是否落在可见区域内（Pickets："never left off-screen"）。
- **数据库与同步**：新增表（如 `fences` / `fence_items`）→ 迁移 v28。收纳箱里是**设备相关的本地路径和屏幕坐标**，性质与 `screen_configs` 相同，**建议不进 WebDAV 同步**；是否进入导出 ZIP 需要按 CLAUDE.md 的"维护检查清单"明确决定。

---

## Files Found（内部，供映射引用）

| File Path | Description |
|---|---|
| `pc/src-tauri/src/commands/window.rs` | 现有嵌入桌面实现：`desktop_host`（1056）、`desktop_attach`（1082）、`desktop_detach`（1119）、`tick_desktop_mode`（1144）、`reassert_window_mode_state`（1185）、`set_window_desktop_mode`（1235）；全局静态 `IS_DESKTOP_MODE` / `DESKTOP_OWNER` 只服务主窗口 |
| `pc/src-tauri/src/lib.rs` | 托盘菜单构建（`TrayIconBuilder` 约 168 行、`tray-add-todo` 事件约 192 行），「新增收纳箱」入口加在这里 |
| `.trellis/tasks/archive/2026-09/09-07-desktop-embed-mode/research/win32-desktop-embed.md` | 桌面窗口层级（≤23H2 vs 24H2）、Win+D 机制、owner=Progman 的利弊 |
| `.trellis/tasks/archive/2026-09/09-07-desktop-embed-mode/e2e-progress.md` | Win+D / Win+M / Explorer 重启 / 重启恢复的实测记录（第 30-43 行） |

## External References

- Stardock Fences 产品页：<https://www.stardock.com/products/fences/>
- Fences 发展史：<https://wiki.stardocksoftware.com/index.php?title=Fences_History>
- Fences 4 导览（新建、Peek、Desktop Display、卷起、快速隐藏）：<https://www.stardock.com/blog/508751/fences-4--a-guided-tour>
- Fences 6 UI 文档：Welcome <https://support.stardock.com/space/SHC/2814443582> · Rollup <https://support.stardock.com/space/SHC/2814444059> · Hide <https://support.stardock.com/space/SHC/2814444006> · Folder Portals <https://support.stardock.com/space/SHC/2814443832> · Layout <https://support.stardock.com/space/SHC/2814444204> · Personalization <https://support.stardock.com/space/SHC/2814443712> · Tabs <https://support.stardock.com/space/SHC/2814444096> · Peek <https://support.stardock.com/space/SHC/2814443964>
- Fences 旧版 UI 文档：Roll-Up <https://support.stardock.com/space/SHC/1459159089> · Quick-Hide <https://support.stardock.com/space/SHC/1458241576> · Layout & Snapping <https://support.stardock.com/space/SHC/1459060777> · Folder Portals <https://support.stardock.com/space/SHC/1459421201>
- Fences 故障 KB：<https://support.stardock.com/space/SHC/1331822625> · <https://support.stardock.com/space/SHC/1344503818> · <https://support.stardock.com/space/SHC/1331494982> · <https://support.stardock.com/space/SHC/2093482017> · <https://support.stardock.com/space/SHC/1423736838> · <https://support.stardock.com/space/SHC/1331232918> · <https://support.stardock.com/space/SHC/1332805638> · <https://support.stardock.com/space/SHC/1770422308>
- Fences Support FAQ（"Explorer ... which Fences run under"）：<https://forums.stardock.com/486084/fences-support-faq>
- Fences 社区帖：<https://forums.stardock.com/496390/where-do-missing-fences-go-where-do-missing-icons-go> · <https://forums.stardock.com/507474/error-fences-has-detected-that-explorer-was-not-closed-properly> · <https://forums.stardock.com/510353/closed-fence-does-not-remove-the-icon-from-the-desktop> · <https://forums.stardock.com/516530/stop-fences-from-moving-desktop-icons> · <https://forums.stardock.com/491487/remove-a-fence-from-desktop> · <https://forums.stardock.com/494578/unable-to-move-icons-with-fences-installed>
- NoFences：<https://github.com/Twometer/NoFences>（重点文件 `NoFences/FenceWindow.cs`、`NoFences/Model/FenceManager.cs`、`NoFences/Win32/DesktopUtil.cs`、`NoFences/Win32/WindowUtil.cs`；issue #2 #3 #14 #23 #55 #56 #65 #69 #70 #77）
- Pickets：<https://github.com/chrisdfennell/Pickets>（`Pickets/Services/DesktopHelper.cs`、`DesktopIconManager.cs`、`DesktopTakeover.cs`）
- LucidDesk：<https://github.com/Yuch3nE/luciddesk>（`docs/development/hybrid-desktop.md`、`docs/development/pane-drag-order.md`、`docs/usage.md`、`app/src/pane/window/mod.rs`）
- TuckPane：<https://github.com/ch998244353/TuckPane>
- 腾讯桌面整理：<https://guanjia.qq.com/product/zmzl/> · <https://apps.microsoft.com/detail/xp9ccdndd9s99t> · <https://www.v1tx.com/post/tencent-zmzl/> · <https://jokerbyrant.github.io/2024-08-28/13172/> · <https://blog.csdn.net/Asi_welldone/article/details/130840597> · <https://blog.csdn.net/haodiaoer/article/details/121470652> · <https://learn.microsoft.com/zh-cn/answers/questions/4089281/win11> · <https://cloud.tencent.com/developer/article/2583499>
- 360 桌面助手：<https://jingyan.baidu.com/article/915fc4145bd75d51384b2070.html> · <https://jingyan.baidu.com/article/546ae185279e815049f28caf.html> · <https://www.ghxi.com/360desktop.html> · <https://blog.csdn.net/qq_57484399/article/details/142726500> · <https://blog.csdn.net/m0_45047336/article/details/120965173> · <https://blog.csdn.net/weixin_36012215/article/details/118327758> · <https://bbs.360.cn/thread-16179670-1-1.html>
- 微软 Shell 拖放约定：<https://learn.microsoft.com/en-us/windows/win32/shell/datascenarios>
- wry 拖放实现：<https://github.com/tauri-apps/wry/blob/dev/src/webview2/drag_drop.rs>

## Related Specs

- 未查阅 `.trellis/spec/`（本次只做竞品调研）。实现前应走 `trellis-before-dev` 读取 Rust/前端规范。
- CLAUDE.md「数据导入导出与同步 → 维护检查清单」：新增表/设置项时必须逐项核对。

## Caveats / Not Found

- **Fences 的内部窗口实现**（子窗口还是 owned 窗口、如何隐藏 fence 内的原图标）官方未公开；"在 Explorer 进程内运行"有官方原话，其余是推断。
- **腾讯桌面整理、360 桌面助手**：没有官方技术文档，存储模型来自社区帖和配置文件痕迹（标注了 [推断]）；两者的 Win+D 行为、默认尺寸/位置**未找到资料**。腾讯部分引用的 CSDN 教程可信度一般。
- 没有实际安装运行任何竞品，以上全部来自文档/源码/帖子。
- 搜索引擎（DuckDuckGo）中途被限流，后半程改用 Brave / Bing 和 GitHub API；Stardock 帮助中心通过 Confluence REST API 读取。
- wry 拖放的结论基于 `dev` 分支源码，项目锁定的是 wry 0.53.5，实现预计一致，但需真机复核"从桌面拖入后源文件仍在"。
- 第 5 节的推荐（A / A+ 模型、"锁定"与"嵌入"是否拆开、改名/折叠手势）属于方案建议，需要用户拍板。

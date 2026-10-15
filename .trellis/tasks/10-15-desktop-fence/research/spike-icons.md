# Spike: 收纳箱图标提取（IShellItemImageFactory）

- **日期**：2026-10-15
- **环境**：Windows 11 Home China 10.0.26300，windows crate 0.58，debug 构建
- **代码**：存档于 [`spike-icons-tests.rs`](spike-icons-tests.rs)（`mod spike`，`#[ignore]`，只读图标，不改动任何真实文件）。
  原先位于 `pc/src-tauri/src/services/fence_fs/icon.rs` 末尾，收尾时移出生产代码；复现时需把它放回 `icon.rs` 末尾再运行下列命令
- **运行**（放回 crate 后，在 `pc/src-tauri` 下）：
  - `FENCE_SPIKE_OUT=<输出目录> cargo test --lib spike_icons -- --ignored --nocapture`：
    22 个样本 × 48/64/96 px × 5 种取法，打印 alpha 统计并输出拼图（白 / 深灰 #202020 / 中灰三种底色）
  - `cargo test --lib spike_find_small_icon_exes -- --ignored --nocapture`（`FENCE_SPIKE_SCAN` / `FENCE_SPIKE_BBOX` / `FENCE_SPIKE_SCAN_BIGGER`）：扫描 exe 找小图标程序
  - `FENCE_SPIKE_DUMP="<路径>|<尺寸>" cargo test --lib spike_dump_pixels -- --ignored --nocapture`：打印像素值
  - `cargo test --lib spike_launch_broken_shortcut -- --ignored --nocapture`：启动目标失效的快捷方式
- **样本**（spike 自己在输出目录下生成）：notepad.exe、cmd.exe、指向 notepad 的 `.lnk`、目标不存在的 `.lnk`、指向文件夹的 `.lnk`、
  图标只有 16 / 32 px 的 `.lnk`（自制 .ico）、`.url`、`.txt` / `.docx` / `.pdf` / `.zip` / 未知扩展名、`.jpg`、
  从开始菜单**复制**来的 5 个快捷方式（7-Zip、Access、远程桌面、步骤记录器等）、`C:\Windows`、空文件夹
- **对比的取法**：`SIIGBF_ICONONLY`、`ICONONLY | BIGGERSIZEOK`、`ICONONLY | SCALEUP`、默认（`RESIZETOFIT`，先取缩略图）、最终实现（ICONONLY + 去边框）

## 结论速览

| 问题 | 结论 | 实现中的处理 |
|---|---|---|
| alpha 是否预乘 | **不是**，是直通 alpha | BGRA → RGBA 只换通道，不反预乘；alpha 全 0 时置 255；alpha=0 的像素颜色清零 |
| 转 PNG 后边缘 | 白 / 深灰 / 中灰底上都没有黑边、白边 | 无需额外处理 |
| `.lnk` 是否带箭头 | **不带**（`Notepad.lnk` 与 `notepad.exe` 结果逐像素一致） | 无需处理 |
| 只有小图标的程序 | 按 64 / 96 请求时，Shell 把 48 px 原图放在画布中央，外圈加 2 px 半透明方框 | `strip_small_icon_frame` 识别方框并裁出居中原图，前端 CSS 拉伸 |
| 取图标还是缩略图 | 用 `SIIGBF_ICONONLY` | 与桌面图标观感一致，尺寸恒为正方形，不会被缩略图生成拖慢 |
| 失效快捷方式启动 | `SEE_MASK_FLAG_NO_UI` 下不弹对话框，约 3 s 后返回 `ERROR_CANCELLED` | 失败时读 `.lnk` 目标路径（不 Resolve），目标不存在则提示"快捷方式指向的目标不存在" |

## 1. alpha 是直通的（非预乘）

判定方法：预乘位图里 0 < a < 255 的像素必然满足 `max(r,g,b) <= a`。统计每个样本中违反这一点的半透明像素数：

| 样本（ICONONLY） | 请求 | 半透明像素 | 其中 `max(rgb) > a` |
|---|---|---|---|
| notepad.exe / Notepad.lnk | 48 | 397 | 242 |
| notepad.exe | 96 | 1554 | 967 |
| Example.url | 96 | 1041 | 321 |
| 文件夹 | 96 | 253 | 190 |
| a.docx | 96 | 353 | 217 |

**所有**含半透明像素的样本（全部取法、全部尺寸）都有违反项，没有一个样本"看起来像预乘"。
像素转储也直接印证：小图标边框像素为 `#FFFFFF / α=0x24`（预乘位图里颜色分量不可能大于 alpha）。

研究文档里"图标位图通常是 32bpp 预乘"的担心在本机不成立。实现里**不做反预乘**：
若用"所有像素都满足 rgb ≤ a 就当预乘"的启发式，遇到颜色偏暗的直通 alpha 图标会被误判，半透明边缘被提亮出现白边，风险大于收益。

拼图中所有图标叠加在白、深灰（接近收纳箱默认底色）、中灰三种底色上，边缘都干净，没有预乘误判时常见的黑边 / 白边。

## 2. `.lnk` 不带快捷方式箭头

`IShellItemImageFactory::GetImage` 返回的是目标的图标本身，箭头是资源管理器视图层叠加的 overlay，不在返回位图里：
`Notepad.lnk` 与 `notepad.exe` 在 48 / 64 / 96 px 下统计数字完全相同；开始菜单复制来的快捷方式、指向文件夹的快捷方式也都没有箭头。
**收纳箱不需要做任何去箭头处理。**

目标不存在的 `.lnk`（`Broken.lnk`）显示一个通用的"应用程序窗口"图标，不带损坏标记；是否失效只能在启动失败时得知（见第 6 节）。

## 3. 各类型在 48 / 64 / 96 px 下的效果

| 类型 | 结果 |
|---|---|
| 有大图标资源的 exe / `.lnk`（notepad、cmd、Access、远程桌面、步骤记录器） | 三个尺寸都返回请求尺寸、内容铺满、清晰 |
| 文件夹（`C:\Windows`、空文件夹、指向文件夹的 `.lnk`） | 标准黄色文件夹图标；ICONONLY 下与内容无关（默认取法会给有内容的文件夹加预览） |
| `.url` | 默认浏览器的 URL 文档图标（本机为 Chrome） |
| `.docx` / `.pdf` / 未知扩展名 | 关联程序的文档图标 / 通用空白文档图标，清晰 |
| `.txt` / `.zip`（关联到 Notepad 新图标 / 7-Zip） | 只有 48 px 资源 → 见下文"小图标" |
| `.jpg` | ICONONLY：通用图片图标；默认取法：保持宽高比的缩略图（96×60，非正方形） |

### 只有小图标资源的程序：Shell 加的方框

7-Zip、`.txt`、`.zip` 以及自制的"只有 16 / 32 px 图标"的快捷方式，表现一致：

- 请求 **48**：Shell 把小图标放大到 48×48（16 px 资源放大后明显发糊，alpha 最高只到 254）
- 请求 **64 / 96**：返回请求尺寸的画布，**48 px 原图居中不放大**，画布最外两圈画半透明方框
  （外圈 `#646464 α=0x30`、内圈 `#FFFFFF α=0x24`、第三圈起透明）。多出来的半透明像素数正好等于两圈周长
  （64：496，96：752）。直接显示会在收纳箱里出现一个淡淡的方框，图标也比其他项小一圈
- `SIIGBF_SCALEUP`、`SIIGBF_BIGGERSIZEOK`、默认取法**都不改变**这一行为

处理（`icon.rs::strip_small_icon_frame`）：画布边长 > 48、最外两圈全部是低 alpha（0 < a < 0x80）且第三圈全透明时，
判定为"小图标 + 方框"，按 `ICON_SIZE_BUCKETS` 找能容纳内容的最小档位，裁出画布正中的原图（通常 48×48）写入缓存。
前端用固定 CSS 宽高显示，小图标会被拉伸到格子大小、与其他图标视觉大小一致（略软）。若更想要资源管理器那种"小图标居中"的观感，
前端可以按 PNG 的实际尺寸显示，两种选择 API 都支持。拼图 `sheet-final.png` 是最终效果。

扫描本机 System32、SysWOW64、Program Files、Program Files (x86)、D:\Program Files 共 2242 个 exe：
按 256 请求时没有出现"大画布中央一个小图标但没有方框"的情况；带 `BIGGERSIZEOK` 时也没有返回小于请求尺寸的位图。
即研究文档担心的"大画布小内容"在 `IShellItemImageFactory` 上只以"带方框"的形式出现，已覆盖。

## 4. 取法选择：`SIIGBF_ICONONLY`

- 与桌面"中等 / 大图标"下快捷方式、程序的观感一致；收纳箱收的主要是 `.lnk`，它们没有缩略图
- 返回位图恒为正方形，网格布局简单；缩略图（图片、视频、带预览的文件夹）尺寸不定
- 缩略图生成可能很慢（大图、视频、网络路径、离线文件），ICONONLY 不触发缩略图提供程序
- 代价：收纳进来的图片 / 视频显示通用图标而不是缩略图。如需缩略图，可以在集成阶段对图片类扩展名改用默认取法（`icon.rs::render_icon` 一处）

性能参考（debug 构建）：spike 一共 330 次提取 + PNG 编码约 6 s，单次约 18 ms；命中缓存时只有一次 `metadata` + `is_file`。

## 5. 研究文档第 3.6 节其他问题

- 以物理像素请求：实现按 `ICON_SIZE_BUCKETS = [16, 24, 32, 48, 64, 96, 128, 256]` 向上取档，前端传 `逻辑尺寸 × devicePixelRatio`
- `.lnk` 的 mtime 不随目标程序更新而变：spike 时预留过 `invalidate_icon(path)` 供"刷新图标"使用；首版未做「刷新图标」入口，收尾时已删除
- 提取失败（文件不存在、权限）返回 `Err`，前端显示占位图标

## 6. 失效快捷方式的启动（附带验证）

对目标不存在的 `.lnk` 调 `ShellExecuteExW(SEE_MASK_FLAG_NO_UI | SEE_MASK_NOASYNC)`：

- **不弹**"快捷方式存在问题"对话框
- 约 **3 s** 后失败（Shell 在后台静默尝试解析 / 搜索目标），错误码是 `ERROR_CANCELLED (0x800704C7)`，不是 `FILE_NOT_FOUND`

只按错误码会误报成"已取消"（与用户取消 UAC 同码）。实现里在 `.lnk` 启动失败且错误码为 CANCELLED / FILE_NOT_FOUND / PATH_NOT_FOUND 时，
用 `IShellLinkW::GetPath`（只读快捷方式、不 Resolve、不搜索）取目标路径，目标不存在则返回"快捷方式指向的目标不存在，可能已被移动或卸载"
（spike 复测：3.06 s 后返回该提示）。保留 Shell 的解析过程是有意的：目标被移动时 Shell 的链接跟踪可能找回它。
前端应在这约 3 s 内显示"正在打开"的状态。

## 7. 未覆盖 / 待集成阶段实测

- 高 DPI（150% / 200%）下按 96 / 128 档请求的实际观感（本机拼图在 100% 下查看）
- IDList 型快捷方式（UWP / 商店应用的 `.lnk`）：本机开始菜单前几个快捷方式都是普通文件路径型，未取到样本
- 网络路径、OneDrive 仅联机文件上的提取耗时
- 正常启动 exe / `.url` / 文件夹、打开所在位置：会在用户屏幕上开窗口，没有自动化执行，留给集成 e2e

## 输出文件（会话临时目录，仅供本次查看）

`C:\Users\12197\AppData\Local\Temp\claude\D--Git-mini-todo\2deec5cc-5555-4107-b384-e56f6778f8a9\scratchpad\spike-icons\`：
`sheet-iconOnly.png`（原始结果，可见小图标的方框）、`sheet-final.png`（最终实现）、其余三种取法的拼图与 96 px 单图；
统计日志 `..\spike-icons-log.txt`。

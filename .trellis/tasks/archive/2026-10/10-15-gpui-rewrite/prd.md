# 使用 Rust + gpui-pre 重写 PC 端（去掉 WebView）

## 背景与目标

现有 PC 端 `pc/` 是 Tauri 2 + Vue 3 + Element Plus（WebView2）。用户不想再使用 WebView，要求用
**Rust + gpui-pre** 重写，**元素、样式、功能与现有一致**。

## 关键决策（无需用户拍板的默认选择，均可逆）

| # | 决策 | 理由 |
|---|---|---|
| D1 | 新代码放在新目录 `pc-gpui/`（Cargo workspace），`pc/` 暂时保留不动 | 新版需要在 Windows 上实机验收后再删旧版；两者共用同一数据目录，可直接切换 |
| D2 | UI 栈：`gpui-kit = "=0.7.3"`（re-export `gpui-pre =0.3.9` + `gpui-pre-platform` + `gpui-base` + `gpui-component`） | gpui-pre 是 Zed 每周快照，API 变化快，必须精确钉版本；gpui-component 提供 Input / Checkbox / Switch / Select / Popover / Dialog / Markdown 等，相当于 Element Plus |
| D3 | 拆成两个 crate：`crates/core`（`mini_todo_core`，纯业务、无 GPUI、可单测）+ `crates/app`（二进制 `mini-todo`，GPUI 界面、窗口与平台集成） | 现有 `db/`、`services/`（除通知窗口）与 Tauri 无关，可原样复用并保留原有测试；单测不必编译 GPUI |
| D4 | 数据完全兼容：同一数据目录（`%LOCALAPPDATA%\mini-todo\`）、同一数据库迁移（v1~v29）、同一同步文档（sync-data.json.gz）、同一导出格式（4.0）、同一 settings 键 | 用户可以无缝从旧版切换到新版，旧版 / 新版 / cloud 可以混用同步 |
| D5 | 原来靠 Tauri 事件跨 WebView 通信（`sync-completed`、`data-imported`、`app-settings-changed`、`todo-font-changed`、`subtask-memory-*`、`fence-items-changed` 等）改为同进程 GPUI Global / Entity 通知 | 所有窗口同在一个进程、一个 App 上下文 |
| D6 | 图标沿用 Element Plus Icons 的 SVG（从 `@element-plus/icons-vue` 抽取用到的图标，打包为 GPUI 资源） | 保持"元素一致"；Element Plus Icons 为 MIT 许可 |
| D7 | Markdown：Milkdown 所见即所得无法在 GPUI 中 1:1 还原 → 编辑模式用 gpui-component 多行编辑器（Markdown 源码）+ 渲染预览，只读模式用 Markdown 渲染；放大编辑弹窗保留"源码 / 预览分栏"；图片粘贴 / 拖入上传、`minitodo-image://` 引用、链接白名单规则不变 | 唯一一处无法完全一致的交互，作为已知偏差写入文档 |
| D8 | 平台集成：托盘 `tray-icon`（Windows）、系统通知（Windows Toast）、开机自启（注册表 Run 键，与 Tauri autostart 插件同名同键以兼容已有设置）、单实例（命名互斥量 + 唤起已运行实例）、文件对话框用 GPUI 自带 `prompt_for_paths` / `prompt_for_new_path`、日志沿用 `%LOCALAPPDATA%\com.tauri-app.mini-todo\logs\mini-todo.log`（2MB 轮转） | 与旧版行为与文件位置一致 |
| D9 | Win32 窗口特性（固定模式、固定时嵌入桌面、贴边隐藏 / 唤起、桌面收纳箱嵌入、ex style 处理）从 `commands/window.rs` 移植，基于 GPUI 窗口的原生 HWND 直接调用 Win32；只为对抗 tao `apply_diff` 写的兜底逻辑按 GPUI 实际行为取舍 | 无 tao 后部分兜底不再需要，但 Explorer 重启自愈等仍保留 |

## 范围

需要还原的全部界面与功能（详见 research/ 下各规格文档）：

- 主窗口：自绘标题栏（同步、视图切换、固定、设置、已完成…）、列表视图、四象限视图、日历视图（农历 / 节假日）、
  新建按钮（含"操作模式"变暗）、拖拽排序、子任务展开、5s 变更轮询、自动同步、窗口位置 / 尺寸记忆、深色主题、
  窗口底色与透明度、字体设置
- 待办编辑窗口（只读详情 ⇄ 编辑）、子任务窗口、已完成窗口、应用内通知窗口、设置窗口（全部设置项、WebDAV、导入导出、
  屏幕配置）、桌面收纳箱窗口（Windows）
- 后端：数据库、迁移、同步、WebDAV、通知调度（含重复提醒、补发）、节假日、导入导出、DPAPI、收纳箱文件层
- 平台：托盘菜单、窗口模式、系统通知、开机自启、单实例、日志

## 非目标

- 不改 `cloud/`（云端 API 不变，新 PC 端与之通过同一 WebDAV 通道协作）
- 不改数据格式、同步协议
- 本任务内不删除 `pc/`（待用户在 Windows 上验收后另行删除）

## 已知偏差（必须在交付说明里写清）

1. Markdown 编辑由所见即所得改为"源码 + 预览"（D7）
2. 本容器是 Linux：Windows 专有能力（嵌入桌面、收纳箱、DPAPI、Toast、托盘）只能做交叉 `cargo check` 与代码审查，
   需要用户在 Windows 上实机验收

## 验收标准

- [x] `pc-gpui/` 下 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 在 Linux 通过（app 148 + core 285）
- [x] `clippy` 对 `x86_64-pc-windows-gnu` 目标通过（覆盖全部 `cfg(windows)` 代码）
- [x] core 迁移原有测试全部通过；workspace 特性统一（serde_json/preserve_order）下的不稳定用例已修
- [x] 农历 / 节气 / 节日结果与 lunar-javascript 一致（2024–2027 逐日对照 1461 天）
- [x] Xvfb 下各窗口截图与参考截图逐一对照（`scripts/regress.py`，48 个参考场景 + 交互 + 冒烟）
- [x] 新增 CI workflow（`pc-gpui-ci.yml`：Windows fmt/clippy/test/release 构建并上传产物，Linux clippy/test）
- [x] 更新 CLAUDE.md / README / spec（新增 `.trellis/spec/pc-gpui/`）

## 用户追加要求：Linux 与 Windows 环境的端到端测试

本容器无 KVM，无法本地起 Windows 虚拟机；Windows 环境改用 GitHub Actions `windows-latest`（真实 Windows 虚拟机，
交互桌面，D3D11 WARP 软件渲染）。

- [x] Linux e2e（`pc-gpui/scripts/e2e-linux/`）：Xvfb + xdotool 黑盒驱动真实应用，Apache / nginx 两种 WebDAV、cloud、Skill；
  F 本地 / S 同步 / L 生命周期
- [x] Windows e2e（`pc-gpui/scripts/e2e-windows/` + `pc-gpui-e2e-windows.yml`）：窗口外观与尺寸、透明、普通 / 经典固定 /
  嵌入桌面（owner、ex style、Z 序、Win+D、Explorer 重启重挂）、通知叠放、真实鼠标键盘交互、开机自启注册表、托盘、
  系统通知、DPAPI、WebDAV 同步、收纳箱（真实 OLE 拖放、改名、几何）、单实例、日志位置、release 冒烟
- e2e 发现并已修复的缺陷：窗口按逻辑坐标偏移（GPUI 不可见边框）、改名后首次拖入丢失（GPUI 输入方式判定）、
  收纳箱几何外框 / 客户区不一致、空描述框点击区域、行内图片尺寸、子任务窗口长内容不能滚动

## 仍需用户在真实 Windows 桌面确认

多显示器 / 高 DPI 下的屏幕组合 ID 与旧版记录是否一致；Win10 / Win11 ≤23H2 上嵌入桌面对 Win+D 的表现；
真实 IME 中文输入；Toast 在用户桌面上的显示；与旧版共存时的单实例互认。

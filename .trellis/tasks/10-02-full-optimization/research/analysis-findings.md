# 全项目审查结论（2026-10-02）

> 来源：5 个并行审查（PC 后端 / 窗口模式 / 前端 / cloud / 同步协议）+ 主会话复核与实测。
> 路径相对仓库根。行号以审查当时（commit a6935e4 + cargo fmt）为准，实施前请重新定位。
> 标注「已核实」= 主会话对照代码复核或实测过；「推断」= 代码推导，未运行验证。

## A. 同步链路（PC ↔ WebDAV ↔ cloud）

A1 **本地变更检测失效（已核实，SQLite 实测）** — `pc/src-tauri/src/commands/sync_cmd.rs` `check_local_changes`
用 `updated_at > last_sync_at` 字符串比较；左边 `YYYY-MM-DD HH:MM:SS`，右边 `%Y-%m-%dT%H:%M:%S%:z`。
同一天内空格(0x20) < 'T'(0x54) → 当天修改一律"无变更"。删除、导入、设置修改也都不算变更。
后果：远端较新时走 `sync_apply_remote` → `delete_orphan_todos` 删掉本地当天新建未上传的待办；当天编辑不上传。

A2 **下载即刷新条件 PUT 基准 + 整库二选一（已核实）** — `webdav_download_sync` / `webdav_auto_sync`
在未合并远端时就写 `webdav_last_modified`；随后"保留本地"上传必然通过，整包覆盖远端；云端 pull 孤儿清理再永久删除 AI 新建记录。
"使用云端"= 合并 + 删除所有本地独有记录 + 远端 settings 覆盖本地。自动同步遇冲突只 `console.log` 并永久跳过
（`pc/src/views/MainView.vue` startAutoSync）。首次部署云端后 PC 选"使用云端"会删光本地并重置设置。

A3 **删除无墓碑（已核实）** — PC 硬删除、删除不算变更；云端墓碑只在 push 合并时用，pull 不看
（`cloud/src/sync/pull.rs` merge_into_sqlite），7 天清理；云端墓碑无条件胜过编辑。导致删除复活 / 未同步新建被误删。

A4 **cloud push 后缓存不更新（已核实）** — `cloud/src/sync/push.rs` push_once：合并结果不写回 SQLite，
却把 PUT 后再 GET 拿到的 ETag 存成 `last_etag`；`pull.rs` 带 If-None-Match 拿 304 直接返回 → 缓存陈旧，
AI 在陈旧数据上 PATCH 后以新时间戳整条覆盖 PC 修改（mock WebDAV 复现）。

A5 **cloud 推荐的 Caddy webdav / nginx dav 的 PUT 不检查 If-Unmodified-Since（读上游源码）** —
x/net/webdav handlePut 有 TODO 注释且 O_TRUNC 原地写；nginx dav 只在 200 时检查。条件 PUT + 412 合并在这些服务端上不生效。
SabreDAV(Nextcloud)/Apache 支持。条件写只有秒级精度。建议：If-Match(ETag) 优先、写前总是 GET+合并。

A6 **AI 写入与 PC 模型不兼容（已核实）**
- 时间格式：SKILL 约定 `YYYY-MM-DD HH:MM:SS`；PC 前端写 `T` 格式。
  - `pc/src-tauri/src/services/notification.rs` advance_repeat 只解析 `%Y-%m-%dT%H:%M(:%S)` → 重复提醒每分钟弹一次；
    循环里 `?` 让同轮后续提醒全部跳过（SQL 无 ORDER BY，按 rowid）。
  - `pc/src/components/CalendarView.vue` getTodoStartDate 用 `createdAt.split('T')[0]`，createdAt 是空格格式 → 无开始时间的待办不显示。
  - `pc/src/views/EditorView.vue` parseNotifyAt 等 `split('T')` 把空格格式拼成 `…09:00:00T09:00:00` 再保存。
- 字段：skill `add` 写 `priority` / `dueDate`（`cloud/skill/minitodo/minitodo.py` ~196-206），PC 模型没有 → 往返被剥掉；
  云端 `list --priority` 永远匹配不到 PC 创建的待办（PC 用 color 表示优先级：#EF4444 高 / #F59E0B 中 / #10B981 低）。
- 类型：PATCH 浅合并任意 JSON（`cloud/src/api/todos.rs` merge_json_shallow），如 `quadrant="urgent_important"`、`color=null`
  → PC `merge_remote_into_local` 的 `from_value::<Todo>().ok()` 静默丢弃该条，两端永久分叉。
- 子任务 `parentId` 可被 PATCH 改 → PC 外键失败 → 整个合并事务回滚，此后每次同步失败。
- cloud push 只输出固定 6 个顶层字段（push.rs merge_sync_data 末尾 json!{}），协议扩展字段会被抹掉。

A7 **系统写入污染 LWW 时间戳（已核实）** — `pc/src-tauri/src/commands/todo.rs` reorder_todos/reorder_subtasks
逐行 autocommit 且把所有传入行 `updated_at` 刷成 now；`notification.rs` mark_as_notified / 推进 notify_at 也刷新 updated_at。
整条 LWW 下会压过 AI 刚做的修改。

A8 **cloud 自身缺陷（已核实）**
- 图片队列竞态：`push.rs` push_dirty_images 读队列 → 锁外慢上传 → 用 remaining 整体覆盖 → 上传期间新入队图片丢失。
- `C{seq}` 复用：`cloud/src/db/repo.rs` assign_seq 用 `MAX(seq)+1`，删最大号后复用。
- pull 不看墓碑：已删 todo 被插回缓存并分配新 seq。
- 图片只在启动时镜像一次（`cloud/src/sync/images.rs`），且 PROPFIND 按行解析、每行只取第一个 href，
  x/net/webdav 单行 XML → 镜像 0 个文件。

A9 **请求放大 / 全量负载** — PC 每次上传 3 个 MKCOL + 每张图一个 HEAD（HEAD 出错按不存在重传）；每个周期无条件全量 GET；
sync-data 先于图片上传；图片只增不删。cloud push 1s 固定循环无退避（PUT 持续失败约 43 万请求/天，实测），每次 push 整包 GET 两次，
每 tick 新建 blocking Client。

A10 **settings 回滚** — 设置修改不算本地变更；应用远端时用远端（往往是本机上次上传的旧快照）整体覆盖本地设置；
窗口位置/尺寸等设备相关设置也在同步范围。

A11 **手动导入重编号** — `pc/src-tauri/src/commands/data.rs` import_data_raw 不保留原 id（last_insert_rowid），
保留旧 updated_at → 与远端合并时重复 / 被当孤儿删 / 被远端旧版本覆盖。

A12 **"加密同步"不存在（已核实）** — 只有 gzip；aes-gcm/sha2/rand/machine-uid 在 Cargo.toml 但无引用；
WebDAV 密码明文存 settings 表并经 `get_sync_settings` 原样回传 WebView。

## B. 安全

B1 **只读详情 `javascript:` 链接可执行（已核实）** — `pc/package-lock.json` 锁 @milkdown/preset-commonmark 7.19.0（link 不过滤 href；
7.21.3 发布包已有 sanitizeLinkHref，已对比）；`pc/src/utils/fileLink.ts` handleLinkClick 只接管 `file:///` 与 `http(s)`，
其它协议 return false 走默认导航；只读详情是点开待办的默认模式。
放大面：`pc/src-tauri/tauri.conf.json` `csp: null`、assetProtocol scope `["**"]`（Tauri 2.9 asset 响应带 ACAO，可 fetch 任意本地文件）、
`capabilities/default.json` 所有窗口同权限、`get_sync_settings` 明文返回密码、`save_subtask_image` 可写任意路径。
另：`file://///host/share` 经 extractFilePath 变 UNC 路径交给 revealItemInDir（可能触发 SMB/NTLM，推断）。

B2 **图片文件名路径穿越（已核实）** — `todo.rs` save_subtask_image `dir.join(&file_name)`；`sync_cmd.rs` 下载图片
`images_dir.join(img_name)`（名字来自远端 JSON，无需用户点击）。cloud 已有 `sanitize_filename`（`cloud/src/api/images.rs`）。

B3 **cloud 小项** — 401 响应也带 X-Sync-Status/X-Last-Sync-At 且触发 DB 查询；api_key 只检查长度≥16；无访问日志
（tower-http 未使用）；systemd 缺 CapabilityBoundingSet 等；Caddyfile.example `header_up X-Forwarded-For {remote}` 多余且错误；
install.sh 生成 0644 的 config.toml。

## C. 性能 / 稳定性（PC 后端）

C1 **WebDAV 命令在 UI 线程阻塞（已核实）** — `sync_cmd.rs` 的 webdav_* 全是同步 `fn`（Tauri 2 同步命令在主线程执行），
`services/webdav.rs` 用 reqwest::blocking、30s 总超时、无 connect_timeout。不要只加 `#[tauri::command(async)]`
（blocking reqwest 不能在 tokio worker 上跑），应 async fn + spawn_blocking。导入导出/import_subtasks_from_paths/save_subtask_image 同理。

C2 **应用内通知窗口 DPI（已核实）** — notification.rs send_app_notification 用 `monitor.size()`（物理像素）算坐标，
传给接受逻辑像素的 `.position()`；1080p@125% 完全在屏外；y 用 u32，叠加多个通知下溢（debug panic）；未扣任务栏（应 work_area）。

C3 **无单实例（已核实）** — lib.rs 无 single-instance 插件；重复启动 → 两个调度器每条提醒弹两次、两个同步循环互相 412。

C4 **release 下 eprintln 全丢** — main.rs windows_subsystem；同步删除日志等无从追查。建议 tauri-plugin-log。

C5 **迁移/导入前无备份；DB 初始化失败 `.expect` 静默崩溃（已核实 lib.rs Database::new().expect）**；
`PRAGMA foreign_keys` 在迁移事务内是 no-op（将来重建表会级联删子任务）。

C6 **get_todos / export N+1（已核实）**；主窗口 60s 轮询 + 聚焦 + 关子窗口都全量拉；编辑窗每次子任务操作全库拉。

C7 **事务边界** — reorder 逐条 autocommit；import_subtasks_from_paths 无事务、持 DB 锁读文件、`read_to_string().unwrap_or_default()`
让 GBK 文本静默变空、`is_dir()` 跟随符号链接无深度上限；delete_orphan_todos 手写 BEGIN 出错不 ROLLBACK。

C8 **吞错** — merge `from_value().ok()`；todo.rs/data.rs `filter_map(|t| t.ok())`；远端解析失败仍上传（auto_sync 吞掉下载/解析错误后照常上传）。

C9 **提醒杂项** — `datetime(notify_at, '-' || notify_before || ' minutes')` 在 notify_before 为 NULL 时恒 NULL；
月重复无 repeat_month_day 时用"当前日"导致 31 号漂移到 28 号；calc_next_occurrence 等无单测。

C10 **死代码 / 无用依赖（已核实）** — Cargo.toml: aes-gcm、sha2、rand、machine-uid、notify、libc(unix)、winreg(windows) 无引用；
webdav.rs 4 个 `#[allow(dead_code)]` 函数（get_last_modified 用小写串下标切原串，非 ASCII 可能 panic）；
images 目录路径函数重复（todo.rs / sync_cmd.rs）；window.rs `get_settings` 与 data.rs read_app_settings 重复且前端无调用。

C11 **设置项两份读写已漂移** — data.rs read/write_app_settings 与 window.rs get/save_settings；同步/导入写 top_on_wake / auto_hide 后
后端运行时缓存（TOP_ON_WAKE、AUTO_HIDE_STATE.enabled）不刷新。CLAUDE.md 清单第 9 条提到的 import_json 已不存在。

## D. 窗口模式（window.rs / lib.rs）

D1 **退出普通固定模式置顶残留（已核实）** — set_window_fixed_mode：先 `IS_FIXED_MODE.store(false)` 再
`set_window_always_on_top(false)` → win32_set_topmost 因非固定模式返回 false → 落到 tao 通路，tao flag 本就 false，apply_diff no-op
→ 被唤起过（HWND_TOPMOST）的窗口解锁后一直置顶。set_window_desktop_mode 里退出固定模式的顺序是对的（有注释）。

D2 **贴边唤起区比保持显示区多 WAKE_RANGE_PADDING_PX=40（推断）** — 光标停在 padding 带内会反复收起/唤起，
每次 set_position 触发前端 onMoved → saveWindowState → 多条 settings 写盘。建议滞回 + 单测。

D3 **轮询线程** — tick_auto_hide 在 auto-hide 关闭时也先取几何（多次跨线程阻塞往返）；tick_desktop_mode 每 tick 一次 window_handle 往返
（可缓存 HWND）；普通模式下线程仍在跑。

D4 **托盘点击** — lib.rs 双击检测两个分支都调 bring_main_window_to_front；用 SystemTime 做差，时钟回拨 u64 下溢（debug panic）；
每次点击新建线程撤置顶，较早线程会提前撤掉后一次的置顶。

D5 **非 Windows** — 同步来的 fixed_embed_desktop=true 让 macOS/Linux 点锁时 set_window_desktop_mode 返回 Err 被吞，
store.isFixed=true 但后端未固定；开关在非 Windows 隐藏，用户无法关闭。

D6 **托盘"重置位置"在普通模式下被立刻撤销（推断）** — 后端移窗后 emit tray-reset-window，前端普通模式直接 initSettings()
读到旧 screen_config（onMoved 500ms 防抖未落库）又 setPosition 回去。

D7 多显示器：收起不检查相邻屏、隐藏态不随拓扑变化刷新、用 size() 非 work_area、阈值未乘 scale（中，工作量 M）。

## E. 前端

E1 时间：见 A6；另有 5 处各自实现的时间格式化（TodoItem / CompletedView / EditorView / SettingsView / CalendarView）。

E2 同步 UI：MainView.handleSync 内层 catch 只处理 'cancel'，异常被吞；startAutoSync 先 stop 再 await，重叠调用泄漏定时器；
MainView 与 SettingsView 各有一套冲突流程。

E3 EditorView 保存无防重入（双击创建两条）；新建时先建待办再逐个建子任务，中途失败后重试会重复建待办。

E4 QuadrantView 只把当前象限 ids 传给 reorder_todos → 列表视图顺序被打乱；跨象限移动失败不回滚。

E5 store 的 error/loading 无人读取；子窗口错误只打 console；CompletedView 删除无确认（TodoItem 有）。

E6 全量刷新：见 C6；TodoList / QuadrantView `deep: true` 监听；fetchTodos 无单飞/序号。

E7 包体（实测 vite build + sourcemap）：入口 chunk 1191KB（element-plus 750KB + icons 167KB = 77%），每个窗口都加载；
全量 CSS 364KB；MainView chunk 316KB 中 lunar-javascript 285KB（日历默认隐藏也加载）；
vuedraggable 4.1.0 的 UMD `require("vue")` 拉进完整版 Vue（@vue/compiler-core 70KB + compiler-dom 9KB）。

E8 CalendarView：节假日重试无上限、定时器不清理；跨午夜"今天"高亮不刷新。

E9 子窗口创建代码 5 处重复、拖拽 5 处重复；CompletedView 打开编辑器不设 parent、无模态守卫。

E10 NotificationView 二次 decodeURIComponent，标题含 `%` 时抛错整卡空白。

E11 图片粘贴逐字节拼 base64 + JSON IPC（MarkdownEditor.vue imageUploader）。

E12 Markdown 图片存 `convertFileSrc(本机绝对路径)`，含 Windows 用户名，跨设备裂图；导出 ZIP 不含图片。

E13 死代码：`components/SettingsPanel.vue` 无引用；`utils/fileLink.ts` fileLinkExtension 无引用（marked 只剩 import type）；
`utils/index.ts`；CompletedView 监听无人发的 'todo-updated'；appStore toggleShowCalendar/windowMode 等。

E14 子任务输入 `@keyup.enter` 在中文输入法回车上屏时误触发（推断）；新建待办标题不自动聚焦；无 Esc 关闭 / Ctrl+Enter 保存。

E15 lockfile：pc/package-lock.json 377/473 个 resolved 指向 registry.npmmirror.com，海外 CI / 云端容器 403 或极慢；
改官方源后 npm ci 17s。

## F. cloud 其它

F1 debug 构建启动即 panic：main.rs 在 #[tokio::main] 里同步调用 pull_once → blocking reqwest 在 async 上下文 drop runtime（mock 复现）。
F2 /health 硬编码 healthy；X-Sync-Status 只看 last_pull_at，push 成功也刷新它；push 持续失败仍报 healthy。
F3 SyncLock guard 属于 handler future，客户端断开时提前释放，互斥失效（应 lock_owned 并 move 进 spawn_blocking）。
F4 push 合并 O(N·M)（Vec contains + find）；列表 withSubtasks N+1。
F5 无优雅停机（tokio signal feature 未用）。
F6 rustls 只信任 webpki 根证书，自签 CA 无法配置；http:// 静默接受。
F7 时区偏移只在启动时算一次（DST 区域半年差 1 小时）。
F8 id = 毫秒×1000+随机，冲突时 ON CONFLICT DO UPDATE 静默覆盖；create 注释写"同一事务"实际多条 autocommit。
F9 代码重复：merge_json_shallow 两份、gunzip 两份、远端路径常量三处、content-type 映射两套；thiserror/tower-http 未使用；
sqlite 原始错误与服务端路径直接返回客户端。
F10 skill CLI：10s 超时短于 /sync 内部 30s；POST 无幂等；`today` 用 CLI 主机本地日期；`--due` 帮助写 ISO 8601 但带 T 的值比较失效；
表头 "✓" 在 cp936 下 UnicodeEncodeError；install.sh config.toml 0644。

## G. 工程化

G1 PC CI 只在 Windows 跑 cargo test，无 clippy/fmt；非 Windows cfg 分支直到打 tag 才编译。
G2 测试缺口：重复提醒计算、check_local_changes、reorder、文件名校验、412 重试、PC↔cloud 往返契约；前端无单测。
G3 版本号三处手改（package.json / tauri.conf.json / Cargo.toml）。
G4 文档漂移：CLAUDE.md 数据库位置（实际 %LOCALAPPDATA%）、"加密"、"应用远端走 import_data_raw"、清单第 9 条、
cloud README "重启从 WebDAV 灌满"（实际 SQLite 持久）。

## 已经做得好的（不要改坏）

- cloud：109 个测试 1.7s 全过、clippy -D warnings 零警告；常量时间比较 Bearer；文件名防穿越；dirty generation 清除逻辑。
- PC：迁移逐版本事务 + 回滚测试；导入单事务；SQL 全参数化；网络 I/O 期间不持 DB 锁。
- 窗口：纯判断函数与副作用分离并有单测；对 tao apply_diff 覆写的理解与注释（见 `.trellis/spec/backend/window-modes.md`）。
- 前端：路由全懒加载；MarkdownEditor 代次号处理异步创建竞态；监听器/定时器清理完整；无 v-html；any 仅 4 处。

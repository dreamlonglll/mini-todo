# 全面优化：同步链路、安全、性能与工程化

## Goal

按 2026-10-02 的全项目审查（`research/analysis-findings.md`）修复会静默丢数据的同步缺陷、可升级为本地任意文件读写的安全链路、
UI 线程阻塞与若干正确性 bug，并做一轮性能与工程化优化。用户授权"按你的想法把值得优化的都优化"，不再逐项确认，
但**不改变产品功能边界**（不新增业务功能、不删除用户可见功能），UI 改动仅限同步交互与错误提示。

## 实施分工（6 个实现单元，三条并行链）

| 链 | 单元 | 范围（只改这些路径） |
|---|---|---|
| Rust | **R1 同步核心** | `pc/src-tauri/src/{commands/sync_cmd.rs,commands/data.rs,commands/settings_cmd.rs,commands/todo.rs(仅删除/墓碑相关),services/webdav.rs,services/secret.rs(新),db/**}`、`pc/src-tauri/Cargo.toml`、`lib.rs`(仅命令注册) |
| Rust | **R2 后端杂项 + 窗口** | R1 完成后执行：`services/notification.rs`、`commands/todo.rs`、`commands/window.rs`、`lib.rs`、`main.rs`、`db/connection.rs`、`Cargo.toml`、`capabilities/`(仅插件权限) |
| 前端 | **D1 正确性/安全/同步 UI** | `pc/src/**`、`pc/package.json`、`pc/package-lock.json`、`pc/src-tauri/tauri.conf.json`(security 段)、`pc/src-tauri/capabilities/default.json`(收紧 fs) |
| 前端 | **D2 性能/清理/测试** | D1 完成后：`pc/src/**`、`pc/vite.config.ts`、`pc/package.json`、lockfile、`pc/eslint.config.js`、`pc/tsconfig*.json` |
| 云端 | **E1 同步 worker** | `cloud/src/sync/**`、`cloud/src/db/**`、`cloud/src/main.rs`、`cloud/src/api/{health.rs,headers.rs,sync.rs,mod.rs}`、`cloud/Cargo.toml` |
| 云端 | **E2 API 校验 + Skill + 部署** | E1 完成后：`cloud/src/api/**`、`cloud/src/{util.rs,time.rs,config.rs}`、`cloud/skill/**`、`cloud/deploy/**` |

所有单元：**不得 git commit/push**；不改 `CLAUDE.md` / `README.md` / `cloud/README.md` / `.trellis/spec/**`（主会话统一更新文档）；
`cloud/skill/minitodo/SKILL.md` 由 E2 更新。Rust 改动须 `cargo fmt`。不得禁用/跳过任何已有测试。

## 跨端契约（PC、cloud、前端必须一致）

### K1 时间格式
- 规范存储格式：`YYYY-MM-DD HH:MM:SS`（本地墙钟、无时区），适用于 todo 的 `notifyAt/startTime/endTime/createdAt/updatedAt`、
  subtask 的 `createdAt/updatedAt`、墓碑 `deletedAt`、`settingsUpdatedAt`。
- 读取方必须兼容：`YYYY-MM-DD HH:MM:SS`、`YYYY-MM-DDTHH:MM:SS`、`YYYY-MM-DDTHH:MM`、`YYYY-MM-DD HH:MM`、
  可选小数秒、可选 `Z`/`±HH:MM` 后缀（换算为本地墙钟）、仅日期 `YYYY-MM-DD`。
- 仅日期的默认时刻（与 PC 编辑器一致）：`startTime` → `00:00:00`，`endTime`/`dueDate` → `23:59:00`，`notifyAt` → `09:00:00`。
- 写入方一律输出规范格式。`SyncData.updatedAt`、`webdav_last_sync_at` 属于元信息，可保留 ISO 带偏移，不再用于变更判断。

### K2 sync-data.json.gz 顶层结构
```jsonc
{
  "version": "4.0",
  "deviceId": "…",
  "updatedAt": "…",                 // 元信息
  "todos": [ /* Todo（嵌套 subtasks） */ ],
  "settings": { /* PC AppSettings，cloud 原样透传 */ },
  "settingsUpdatedAt": "YYYY-MM-DD HH:MM:SS",   // 新增，可缺省
  "images": ["name.png"],
  "tombstones": [ { "entityType": "todo" | "subtask", "entityId": 123, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ]  // 新增，可缺省
  // 任何其它顶层键：所有写入方必须原样保留（前向兼容）
}
```
- 墓碑保留期 **30 天**：写出时丢弃 `deletedAt < now-30d` 的墓碑；本地墓碑表同样按 30 天清理。

### K3 合并规则（PC 合并远端、cloud pull、cloud push 构造上传文档，三处一致）
1. 记录级 LWW：比较规范化后的 `updatedAt` 字符串；**平局保持本地版本不动**。
2. 墓碑：存在同 `(entityType, entityId)` 且 `deletedAt >= record.updatedAt` 的墓碑 → 删除/压制该记录；
   todo 的墓碑同时删除其全部子任务。记录 `updatedAt > deletedAt`（删除后又被编辑）→ 记录保留。
3. 仅一侧存在的记录：保留（并集），除非被墓碑压制。**不再做"缺席即删除"的孤儿清理**。
   兼容例外：cloud pull 遇到**没有 `tombstones` 键**的远端文档（旧版 PC 写的）且 cloud 不 dirty 时，可沿用旧的缺席清理。
4. 墓碑集合：并集，同键取较大 `deletedAt`，按保留期过滤。
5. settings：只有 PC 读写。PC 仅当远端 `settingsUpdatedAt` 存在且 **大于** 本地同步设置项的 `max(updated_at)` 时才应用远端 settings；
   `windowPosition`/`windowSize` 永不从远端应用（仍写入导出/上传以兼容旧版）。cloud 原样透传 `settings`/`settingsUpdatedAt`，
   远端缺 settings 时 cloud 写占位对象，但**不写** `settingsUpdatedAt`（PC 因而不会应用占位）。
6. 记录字段：PC 用强类型模型，未知字段会丢；因此 cloud 写入路径必须保证记录只含 K6 定义的字段与类型。
   PC 合并时反序列化失败的记录：跳过、计数、写日志，**绝不**当作删除，**绝不**影响其它记录。

### K4 远端变更检测与条件写（PC 与 cloud 一致）
- 每端保存"基准"= 最近一次**已完整合并或成功写入**的远端版本的 `(ETag, Last-Modified)`。
  **永远不要**把一个内容没有被合并的 GET 结果记为基准（修 A2/A4）。
- GET：有 ETag 用 `If-None-Match`，否则有 LM 用 `If-Modified-Since`；304 = 自基准以来未变。
- PUT：有基准 ETag 用 `If-Match`，否则有基准 LM 用 `If-Unmodified-Since`，远端确认不存在时不带前置条件。
  412 → 重新 GET → 合并 → 重试（≤3 次）。
- 写入前**总是**先 GET（条件 GET）并合并，再条件 PUT；这样即使服务端忽略条件头（Caddy/nginx），丢更新窗口也只剩 GET→PUT 的几秒。
- PUT 成功后：基准取 PUT 响应的 ETag/Last-Modified；都没有时用一次 `PROPFIND Depth:0`（getetag + getlastmodified）获取，**不要整包 GET**。
- PUT 返回 404/409（父目录不存在）时才执行 MKCOL 链再重试；不再每次同步都 MKCOL。

### K5 图片
- 安全文件名：单个普通路径段，匹配 `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`，不含 `..`。不满足的名字在保存、上传、下载、列举时一律跳过并记日志。
  （cloud 现有命名 `img_<millis>_<id>.<ext>` 满足该规则。）
- PC 编辑器上传的图片扩展名白名单：png / jpg / jpeg / webp / gif / bmp（大小写不敏感，存小写）；单张上限 20MB。
- **Markdown 图片引用的规范形式**：`minitodo-image://<name>`。
  - PC 前端渲染时换成 `convertFileSrc(<本机 images 目录>/<name>)`，保存时换回规范形式。
  - 兼容旧数据：`http(s)://asset.localhost/<url 编码的绝对路径>` 与 `asset://localhost/<…>`，若解码后路径的父目录名为 `images`，
    取最后一段文件名视为 `<name>`（渲染时映射到本机 images 目录，保存时写回规范形式）。
  - cloud / AI：通过 `GET /images/<name>` 取图（SKILL.md 说明两种形式及提取文件名的方法）。
- 上传顺序：先传缺失图片，再传 sync-data。远端已有图片清单用**一次** `PROPFIND Depth:1` 获取（健壮解析：任意命名空间前缀、大小写、
  单行或多行 XML 都能取出所有 `href`，URL 解码后取最后一段）；PROPFIND 失败时退化为逐个 HEAD。

### K6 cloud 写入字段契约（E2 实现，PC 模型为准）
- Todo 可写字段：`title`(非空字符串) `description`(字符串|null) `color`(`#RRGGBB`) `quadrant`(1–4 整数；也接受别名
  `urgent_important`=1 `important_not_urgent`=2 `urgent_not_important`=3 `not_urgent_not_important`=4，存整数)
  `notifyAt`/`startTime`/`endTime`(K1 日期时间|null) `notifyBefore`(≥0 整数) `notified`(bool) `completed`(bool) `sortOrder`(整数)
  `repeatEnabled`(bool) `repeatType`(`daily`|`weekly`|`monthly`|null) `repeatInterval`(≥1 整数) `repeatWeekdays`(如 `"1,3,5"`，1–7|null)
  `repeatMonthDay`(1–31|null)。
- 输入别名（入库前归一化）：`dueDate` → `endTime`；`priority`(`high`|`medium`|`low`) → `color`（#EF4444/#F59E0B/#10B981），
  同时给了 `color` 时以 `color` 为准；`notes` → `description`（仅当 description 未给出）。
- 服务端字段 `id/createdAt/updatedAt/seq/subtasks`：写入时忽略。未知字段 → 400，错误信息列出未知字段与允许字段。类型错误 → 400 并指出字段。
- Subtask 可写：`title`(非空) `content`(字符串|null) `completed`(bool) `sortOrder`(整数)；改 `parentId` → 400；`id/createdAt/updatedAt` 忽略。
- 读取响应额外带派生字段 `priority`（由 color 映射，自定义颜色为 null）与 `seq`；派生字段不入库。
  列表过滤 `priority=` 与排序 `sort=priority` 改为按 color 映射。
- 存量数据：启动时一次性把缓存中含 `priority`/`dueDate`/`notes` 或非规范时间格式的记录按上述规则归一化，
  有变化的记录刷新 `updatedAt` 并 mark dirty，让归一化结果传播。

### K7 PC Tauri 命令契约（R1/R2 实现，D1/D2 调用）
- `webdav_sync() -> SyncReport`（async）：K4 流程的"智能同步"，手动按钮、设置页"立即同步"、自动同步定时器共用。
- `webdav_force_pull() -> SyncReport`：让本地等于远端（删除本地独有 todo/subtask，不生成墓碑；应用远端 settings（设备相关键除外）；不检查 settingsUpdatedAt）。远端不存在 → Err。
- `webdav_force_push() -> SyncReport`：让远端等于本地（GET 远端，为远端独有的 todo/subtask 生成墓碑，按 K4 条件 PUT）。
- `SyncReport`（camelCase）：`{ status: "no_changes"|"pulled"|"pushed"|"merged", lastSyncAt: string,
  todosInserted, todosUpdated, todosDeleted, subtasksInserted, subtasksUpdated, subtasksDeleted, recordsSkipped,
  imagesUploaded, imagesDownloaded: number, settingsApplied: boolean }`。
- 同步互斥：同一时刻只允许一个同步（含 force），重入返回 Err("同步正在进行中")。
- 同步成功且本地数据或设置有变化时，后端向所有窗口 emit `sync-completed`（payload = SyncReport）。
- `webdav_test_connection(url, username, password: Option<String>)`：password 为空/None 时用已保存的密码。
- `get_sync_settings()`：`webdavPassword` 恒为 `""`，新增 `hasPassword: boolean`。
- `save_sync_settings(settings)`：`webdavPassword` 为空 → 保留已存密码；新增可选 `clearPassword: boolean` 显式清空。
- 删除命令：`webdav_upload_sync`、`webdav_download_sync`、`webdav_apply_remote`、`webdav_auto_sync`。
- `get_todo(id) -> Todo`（含 subtasks）；`get_change_seq() -> number`（本地 todos/subtasks 任意增删改都会增大的计数）。
- `save_subtask_image`：改为 raw body —— 前端 `invoke('save_subtask_image', bytes: Uint8Array, { headers: { 'x-image-ext': 'png' } })`，
  后端生成文件名 `<millis>_<6位随机>.<ext>`、校验扩展名与大小，返回绝对路径字符串。
- 所有涉及网络或大文件 I/O 的命令（webdav_*、export/import_data_to/from_file、import_subtasks_from_paths、save_subtask_image）
  必须是 `async fn`，阻塞部分放进 `tauri::async_runtime::spawn_blocking`（通过 `AppHandle` 在闭包里取 `State<Database>`）。

## R1 同步核心（PC Rust）

1. **迁移 v28**（`db/migrations.rs`，一个迁移内完成，带测试）：
   - `sync_meta(key TEXT PRIMARY KEY, value INTEGER NOT NULL)`，种子 `local_seq=1`、`synced_seq=0`（升级后首次同步必定执行一次合并上传）。
   - todos / subtasks 的 AFTER INSERT / UPDATE / DELETE 触发器：`local_seq = local_seq + 1`。
   - `tombstones(entity_type TEXT NOT NULL, entity_id INTEGER NOT NULL, deleted_at TEXT NOT NULL, PRIMARY KEY(entity_type, entity_id))`。
   - 规范化存量 `notify_at/start_time/end_time`（`T`→空格，长度 16 补 `:00`），**不刷新 updated_at**。
   - 新 settings 键：`webdav_remote_etag`（''）、`webdav_synced_settings_at`（''）。保留 `webdav_last_modified`。
2. **settings 写入助手** `db/settings_kv.rs`：`get_setting` / `set_setting`，`set_setting` 只在值变化时更新 value 与 updated_at
   （`INSERT … ON CONFLICT(key) DO UPDATE SET … WHERE settings.value IS NOT excluded.value`）。data.rs、sync_cmd.rs、settings_cmd.rs 改用它
   （window.rs 由 R2 改）。`write_app_settings` 包事务由调用方负责。
3. **墓碑**：`delete_todo`（含其全部子任务）、`delete_subtask` 写墓碑（deleted_at = 本地 now 规范格式）。
4. **时间工具** `db/time.rs`（或等价位置）：`now_local()`、`normalize_datetime(&str, DefaultTime) -> Option<String>`、
   `parse_local_datetime(&str) -> Option<NaiveDateTime>`，覆盖 K1 全部输入形态，带单测。R2 的通知与 todo 命令复用。
5. **同步算法**（`sync_cmd.rs` 重写，K3/K4/K7）：
   ```
   lock SYNC_LOCK (try_lock，忙则 Err)
   for attempt in 0..3:
     resp = 条件 GET(sync-data, 基准)
     NotModified → remote_changed=false
     NotFound    → remote=None
     Ok(body)    → 解析失败直接 Err（绝不上传覆盖读不懂的远端）；
                   seq_before = local_seq；dirty = seq_before != synced_seq
                   在单事务内 merge(remote)：记录 LWW + 墓碑 + 下载缺失图片(事务外) + settings 规则
                   记基准 = 本次 GET 的 (etag, lm)
     need_upload = dirty || 本地设置 max(updated_at) > webdav_synced_settings_at
                   || remote 缺本地有的记录/墓碑/较新版本（按 (type,id,updatedAt) 与墓碑集合比较）|| remote=None
     if !need_upload: synced_seq = 当前 local_seq；last_sync_at = now；return report
     先上传缺失图片（K5）
     同一 with_connection 内读 seq_at_export 并导出（todos+subtasks+tombstones+settings+settingsUpdatedAt+remote 的未知顶层键）
     条件 PUT（K4）：Ok → 存基准、synced_seq=seq_at_export、webdav_synced_settings_at=导出时的设置时间、last_sync_at=now；return
                    412 → continue；404/409 → ensure_dir 后重试一次
   Err("多次重试后仍冲突")
   ```
   - merge 内删除 `delete_orphan_todos` 式的缺席清理；墓碑合并按 K3；合并远端子任务时 `parent_id` 以外层 todo 的 id 为准；
     远端 todo 引用的父不存在 / 外键失败的单条记录跳过计数，不回滚整个合并。
   - 所有多语句写入用 `rusqlite::Transaction`（RAII，出错自动回滚），不要手写 BEGIN/COMMIT。
   - 写入远端前对记录时间字段做 K1 规范化；合并远端时同样规范化后再落库。
   - 本地墓碑按 30 天清理（每次同步时）。
6. **force pull / force push**（K7）。
7. **导入**（`import_data_raw`，手动导入）：导入前自动备份（`VACUUM INTO` 到 `backups/` 目录，见 R2 备份工具，若 R2 尚未提供则在 R1 内实现
   `db::backup::snapshot(conn, reason)` 供两处复用，保留最近 5 份）；**保留原 id** 插入；为导入前存在而导入后不存在的记录写墓碑；
   导入记录 `updated_at` 刷成 now（恢复备份应当是权威的）。
8. **webdav.rs**：connect_timeout 10s、总超时 60s；条件 GET/PUT（If-None-Match/If-Modified-Since、If-Match/If-Unmodified-Since）、
   返回 ETag+LM；`propfind_meta`(Depth 0)；`list_names`(Depth 1，健壮 href 解析，带单测：多行 / 单行 / `D:` / `d:` / 无前缀 / URL 编码)；
   删除无用的 `#[allow(dead_code)]` 函数；http:// 地址时 `log`/`eprintln` 警告（R2 接入日志后统一为 log）。
9. **密码**：`services/secret.rs` —— Windows 用 DPAPI（`CryptProtectData`/`CryptUnprotectData`，`windows` crate 增加
   `Win32_Security_Cryptography` feature），存储格式 `dpapi:<base64>`；无前缀视为旧明文，读时直接用、下次保存时加密；
   非 Windows 直接存明文（带注释说明）。`get_sync_settings` 不回传密码（K7）。
10. **图片文件名校验** `is_safe_image_name`（K5），用于同步上传/下载/列举；`save_subtask_image` 由 R2 改造时复用。
11. **测试**（`#[cfg(test)]`，内存库）：v28 迁移（触发器计数、墓碑表、时间规范化）、settings 助手只在值变化时刷新时间、
    merge 的 LWW 平局保持本地、墓碑压制与删除后编辑保留、缺席不删除、坏记录跳过不影响其它记录、settings 应用规则、
    need_upload 判定、href 解析、文件名校验、导入保留 id 并生成墓碑。现有测试中依赖旧语义（孤儿删除等）的要按新契约改写，不得删除覆盖面。

## R2 后端杂项 + 窗口（R1 之后）

1. 通知（`services/notification.rs`）：解析用 R1 时间工具（兼容空格/T）；循环内逐条处理错误（记日志；解析失败的重复提醒标记已通知防刷屏），
   一条失败不影响其它；SQL 用 `COALESCE(notify_before, 0)`；月重复缺 `repeat_month_day` 时取 `notify_at` 的日；
   月末/闰年/跨年/interval>1/周多选的 `calc_next_occurrence` 单测。
   应用内通知窗口：用 `monitor.work_area()` + `scale_factor()` 换算逻辑坐标，i32/f64 计算无下溢，超过可见数量时换列或循环复用槽位；
   提醒写 `notified`/推进 `notify_at` 时保留 `updated_at` 刷新（保证传播），但不改其它字段。
2. `commands/todo.rs`：`create_todo`/`update_todo` 对时间字段 K1 规范化；`get_todos` 与导出去掉 N+1（一次查全部子任务按 parent 分组，`prepare_cached`）；
   新增 `get_todo(id)`、`get_change_seq()`；`reorder_*` 单事务、只更新 sort_order 真正变化的行（仍刷新这些行的 updated_at）；
   `import_subtasks_from_paths`：锁外读文件、UTF-8 → UTF-16(BOM) → GBK（`encoding_rs`）回退、递归深度上限 5、跳过符号链接、单事务插入、
   读失败的文件名汇总报错；`save_subtask_image` 改 raw body（K7），复用 `is_safe_image_name`。
   相关命令改 async + spawn_blocking（K7）。
3. `lib.rs`/`main.rs`/`connection.rs`：
   - `tauri-plugin-single-instance`（第一个注册），二次启动时把主窗口带到前台（复用 window.rs 现有的 bring-to-front 逻辑）。
   - `tauri-plugin-log`：日志写到应用日志目录（文件 + stdout，Info 级，单文件约 2MB 轮转保留 1 份）；全部 `eprintln!/println!` 改 `log` 宏。
   - 备份：有待执行迁移时先 `VACUUM INTO <data dir>/backups/data-v<旧版本>-<时间>.db`（保留最近 5 份），与 R1 导入备份共用工具。
   - 迁移前关闭外键、迁移后 `PRAGMA foreign_key_check` 再开启。
   - DB 初始化失败：不 panic；Windows 弹 `MessageBoxW` 说明数据库路径与错误，写日志后退出码 1；非 Windows 打印到 stderr。
   - 托盘点击：去掉双击分支（两分支相同），用 `Instant` 计时，撤置顶线程改为 generation 计数防止旧线程提前撤销。
   - 删除无用依赖：aes-gcm、sha2、rand、machine-uid、notify、libc、winreg（确认无引用后删），Cargo.lock 随之更新。
4. `commands/window.rs`（先读 `.trellis/spec/backend/window-modes.md`，遵守其顺序规则）：
   - D1：`set_window_fixed_mode(false)` 先撤置顶再 `IS_FIXED_MODE.store(false)`（与 desktop 分支对称）。
   - D2：贴边可见态把"光标在唤起范围（热区 + padding）内"也视为在内（滞回），补纯函数单测。
   - D3：`tick_auto_hide` 未启用时直接返回；setup 时缓存主窗口 HWND，`tick_desktop_mode` 不再跨线程取句柄。
   - D5：非 Windows 上 `set_window_desktop_mode(true)` 回退为普通固定模式并返回 Ok（注释说明）。
   - 新增 `pub fn reload_runtime_prefs(db: &Database)`：从库刷新 TOP_ON_WAKE 与 AUTO_HIDE_STATE.enabled；
     在导入完成、同步应用了远端 settings、save_settings 之后调用（sync_cmd.rs/data.rs 调用点由 R2 补上）。
   - `get_settings` 命令删除（前端无调用，与 data.rs 重复）；`save_settings` 改用 settings_kv 助手并包事务；其余 window.rs 里的
     settings 读写改用助手（布尔读取统一）。
   - **不做** window.rs 模块拆分与多显示器 D7（单独任务）。
5. 测试：通知时间计算、raw 图片保存的文件名/扩展名校验、reorder 只更新变化行、import_subtasks 编码回退、窗口滞回纯函数。

## D1 前端正确性 / 安全 / 同步 UI

1. 安全：
   - `@milkdown/*` 升到 ≥7.21.3 的最新 7.x，**用官方 registry 重建 lockfile**（`npm install --registry=https://registry.npmjs.org/`，
     确保 lockfile 中不再出现 npmmirror 地址）；验证 MarkdownEditor 行为（编辑/只读切换、replaceAll、上传、粘贴 MD）无回归。
   - `utils/fileLink.ts`：点击任何 `<a>` 先判定协议，白名单 http/https（openUrl）、mailto（openUrl）、`file:///`（reveal，拒绝 UNC `\\` 开头），
     其余一律 preventDefault 且不导航；只读与编辑模式都适用（编辑模式下 http(s) 仍需 Ctrl/Cmd+点击）。删除未引用的 `fileLinkExtension` 与 `marked` 依赖。
   - `tauri.conf.json`：`csp` 设为
     `default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' asset: http://asset.localhost blob: data:; font-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost https://api.github.com; object-src 'none'; base-uri 'self'; frame-src 'none'`，
     `devCsp` 设为 null；`assetProtocol.scope` 收窄为 `["$LOCALDATA/mini-todo/images/**"]`。核对所有窗口不需要额外来源（如有遗漏在报告中说明）。
   - `capabilities/default.json`：删除仅被死代码使用的 `fs:allow-write-text-file`（确认无引用）。
2. 时间：新建 `utils/datetime.ts`（解析 K1 全部形态、`toStorage(date, time, kind)` 输出规范空格格式、显示格式化），替换 EditorView、CalendarView
   的 `split('T')` 与 5 处重复格式化（TodoItem / CompletedView / EditorView / SettingsView / CalendarView）。前端写入一律规范格式。
3. 图片引用：新建 `utils/imageRef.ts`（K5 双向转换，含旧 asset URL 兼容），MarkdownEditor 加载时转显示形式、`markdownUpdated` 输出前转规范形式，
   保证 `internalContent` 比较基于规范形式不回环；图片上传改 raw body（K7），用 `get_images_dir` 缓存目录。
4. 同步 UI：MainView 手动同步与自动同步都调 `webdav_sync`；根据 SyncReport 提示（无变化 / 已拉取 / 已推送 / 已合并 + 计数），
   有本地变化时 `fetchTodos`，`settingsApplied` 时 `reloadAppSettings`；错误统一 ElMessage（自动同步连续失败只提示一次）。
   删除冲突对话框。`startAutoSync` 修复重叠调用泄漏（代次号或 clear/set 都在 await 之后）。
   SettingsView：保留服务器配置；按钮改为「立即同步」（主按钮）+「高级：用云端覆盖本地 / 用本地覆盖云端」（二次确认，说明后果）；
   密码框显示"已保存（留空则不修改）"占位，提供清除密码；URL 为 `http://` 时显示明文传输警告。监听后端 `sync-completed` 刷新。
5. 编辑器：保存防重入（saving 状态 + 按钮 loading/disabled）；新建成功后立即切到该 id 的编辑态，子任务创建失败提示且重试不重复建待办；
   取单条待办改用 `get_todo`。
6. CompletedView：删除前确认（与 TodoItem 一致）；打开编辑器传 parent 并加模态守卫（与 MainView 一致）。
7. NotificationView：去掉二次 decodeURIComponent。
8. QuadrantView：象限内拖拽后把新相对顺序合并回全局顺序再整体 reorder；跨象限更新失败回滚本地状态。
9. appStore：非 Windows 平台即使 `fixedEmbedDesktop=true` 也走普通固定模式；托盘"重置位置"在普通模式下先保存当前几何再 initSettings
   （或不再从库恢复几何），修复被撤销的问题。
10. 错误提示：新增 `utils/notify.ts` 的 `notifyError(e, msg)`，store 动作与 EditorView / CompletedView / SubtaskEditorView 的失败路径使用。

## D2 前端性能 / 清理 / 测试（D1 之后）

1. Element Plus 按需引入：`unplugin-vue-components` + `ElementPlusResolver`（组件与样式），`unplugin-element-plus` 或等价方式处理
   `ElMessage`/`ElMessageBox`/`ElLoading` 等函数式 API 的样式；去掉 `app.use(ElementPlus)`、全量 CSS 与全部图标的全局注册，
   各 SFC 显式 import 用到的图标；SettingsView 菜单图标存组件引用而非字符串。用脚本核对模板中不再有未解析组件。
   同步更新 `.trellis/spec/frontend/component-guidelines.md` 中"icons globally registered"的约定（文档由主会话改，你在报告中列出需要改的点）。
2. `CalendarView` 用 `defineAsyncComponent` 懒加载（日历默认隐藏，lunar-javascript 不再进主窗口首包）。
3. vuedraggable 的 `require("vue")` 拉入完整版 Vue：在 vite 配置中把 `vue` 解析到 `vue/dist/vue.runtime.esm-bundler.js`（或等价方案），
   确认构建产物不再包含 `@vue/compiler-core`，拖拽功能代码路径不变。
4. 刷新策略：主窗口轮询改为每 5s 调 `get_change_seq`，变化才 `fetchTodos`；`fetchTodos` 单飞 + 序号丢弃过期响应；
   去掉 TodoList / QuadrantView 的 `deep: true`；EditorView 子任务操作后只刷新当前待办（`get_todo`）。
5. CalendarView：节假日重试设上限并在卸载/切月时清理定时器、用序号丢弃过期结果；"今天"在午夜与窗口聚焦时刷新。
6. 交互：子任务输入改 `@keydown.enter` 并判断 `isComposing`/keyCode 229；新建待办标题自动聚焦；编辑类子窗口支持 Esc 关闭、Ctrl+Enter 保存
   （不与现有 main.ts 的快捷键拦截冲突）。
7. 清理死代码：`components/SettingsPanel.vue`、`utils/index.ts`（若无引用）、CompletedView 的 `'todo-updated'` 监听、appStore 中无引用的
   `exportData/importData/toggleShowCalendar/windowMode` 等（逐个确认无引用再删）。
8. 单测：引入 vitest（devDependency + `npm run test` 脚本），覆盖 `utils/datetime.ts`、`utils/imageRef.ts`、fileLink 协议判定。

## E1 cloud 同步 worker

1. push 成功后，在同一把同步锁内把合并后的文档写回 SQLite（按 K3 应用），再记录基准（K4）；ETag/LM 取自 PUT 响应，缺失时 PROPFIND Depth 0，
   不再整包 GET；PUT 优先 `If-Match`。
2. 墓碑双向传播（K2/K3）：push 输出 `tombstones`（本地 ∪ 远端，30 天保留），pull 应用远端墓碑并写入本地墓碑表；pull 跳过被墓碑压制的记录；
   墓碑按时间规则而非无条件胜出；本地墓碑清理期改 30 天。
3. push 构造文档时以远端文档为底，原样保留未知顶层键与 `settings`/`settingsUpdatedAt`，只覆盖 `todos/images/tombstones/updatedAt/version/deviceId`。
4. pull：仅在远端无 `tombstones` 键且非 dirty 时做旧的缺席清理（K3-3 兼容例外）。
5. `dirty_images` 竞态：上传后在同一 `with_conn` 内重新读取队列，只移除成功上传的名字（或改为表 + 逐条 DELETE）。
6. `C{seq}` 改单调高水位（meta `seq_hwm`），删除最大号后不复用；补测试。
7. 图片：每次 pull 拿到 200 后在后台补下缺失图片；PROPFIND 解析改健壮（与 K5 同规则，单行 XML 可用），补测试。
8. worker：全局复用一个 blocking Client（connect_timeout 10s、按文件大小放宽总超时）；连续失败指数退避 + 抖动（上限 5 分钟），
   状态变化时才打 warn；merge 用 HashMap/HashSet（去掉 O(N·M)）；连续写入去抖 1–2s。
9. 启动：首次 pull 放进 `spawn_blocking`（修 debug 构建 panic）；SyncLock 用 owned guard 并 move 进 spawn_blocking 闭包；
   优雅停机（SIGTERM/SIGINT → 停止接收、等待在途请求、限时补推一次）。
10. 可观测性：meta 记录 `last_push_ok_at`、`last_push_error`、`dirty_since`、图片队列长度；`/health` 与 `X-Sync-Status` 取 pull/push 中较差者，
    降级时 /health 返回 503 并带出这些字段；401 响应不注入同步头、不查库。
11. 时区：每次取时间都 `Utc::now().with_timezone(&tz)`，不缓存偏移（DST）。
12. 测试：用一个进程内 mock WebDAV（如基于 axum/hyper 的测试服务器或 tiny_http）覆盖 push 成功写回、412 重试、墓碑传播、未知顶层键保留、
    图片队列竞态、seq 高水位、pull 墓碑。保持 `cargo fmt`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked` 全绿。

## E2 cloud API 校验 + Skill + 部署（E1 之后）

1. K6 全部：校验、别名映射、派生 priority、过滤/排序映射、存量归一化、时间 K1 规范化。
2. id：create 时普通 INSERT，冲突重新生成；整个写操作包事务；修正 ids.rs 注释中的数量级错误。
3. 错误：不把 sqlite 原始错误与服务端文件路径返回给客户端（记日志，返回通用信息）。
4. 安全/部署：api_key < 32 字符时启动 warn（不拒绝，兼容旧部署）；`tower-http` TraceLayer 访问日志（脱敏 Authorization）；
   `rustls-tls-native-roots` 与 webpki 根同时启用，并支持可选配置 `webdav_ca_file`；非回环 http:// WebDAV 启动 warn；
   systemd unit 补齐加固（CapabilityBoundingSet=、PrivateDevices、ProtectKernel*、ProtectControlGroups、RestrictAddressFamilies、
   RestrictNamespaces、LockPersonality、MemoryDenyWriteExecute、SystemCallFilter=@system-service、UMask=0077 等，确认不影响网络与数据目录写入）；
   删除 Caddyfile.example 的 `header_up X-Forwarded-For {remote}`；删除未使用的 thiserror（若仍未使用）。
5. 重复代码：merge_json_shallow、gunzip、远端路径常量、content-type 映射各保留一份。
6. Skill（`cloud/skill/minitodo/`）：
   - `minitodo.py`：stdout/stderr reconfigure 为 utf-8；`--due` 写 `endTime` 且规范为 `YYYY-MM-DD HH:MM:SS`（仅日期 → 23:59:00）；
     `--priority` 照常发送（服务端映射）；`update` 复用 quadrant 别名映射；sync 命令单独更长超时（≥90s）；GET 失败带退避重试 2 次；
     `today` 的日期按配置 `timezone`（缺省本机）计算；`--due` 帮助文案改为规范格式。
   - `install.sh`/`install.ps1`：生成的 config 权限收紧（chmod 600 / ACL 仅当前用户）。
   - `SKILL.md`：更新字段表（priority 为派生字段、dueDate→endTime、时间格式、图片引用 `minitodo-image://<name>` 与旧 asset URL 的文件名提取方式、
     未知字段 400）。
7. 测试：校验/映射/派生字段/过滤排序/存量归一化/错误不泄露路径；`python -m py_compile`。

## Acceptance Criteria

- [x] `pc/src-tauri`：`cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`（Linux）、`cargo test --locked` 全绿；
      `cargo check --target x86_64-pc-windows-gnu` 通过（Windows cfg 代码可编译）。
- [x] `pc`：`npm run typecheck`、`npm run lint`（0 error）、`npm run build`、`npm run test` 通过；lockfile 无 npmmirror 地址。
- [x] `cloud`：`cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked` 全绿；`python -m py_compile` 通过。
- [x] 跨端契约测试（主会话做）：cloud 产出的 sync-data 能被 PC 反序列化并合并；PC 导出的 sync-data 被 cloud pull 正确处理（含墓碑、未知顶层键）。
- [x] 每个单元在报告中列出：改了什么、对应的审查条目编号、未做的项与原因、需要主会话更新的文档点。

验收记录（2026-10-02，最终 HEAD b73b6ea）：pc/src-tauri fmt / clippy（Linux + x86_64-pc-windows-gnu）/ 161 测试通过；
pc 前端 typecheck / lint / vitest 86 / build 通过；cloud fmt / clippy / 264 测试 + Skill 17 测试通过；
Linux e2e 37/37 通过（含 Apache / nginx 双服务端的跨端契约用例），见 `research/e2e-results.md`。

## Out of Scope（本轮不做，记录原因）

- window.rs 模块拆分、多显示器 D7、EditorView/SettingsView 大规模拆分、appStore 设置工厂化、`src/api/` 全面数据层：纯结构性重构，
  无法在本环境运行 Windows UI 验证，单独立项。
- 按窗口拆分 capabilities / AppManifest 自定义命令权限：需要逐窗口梳理 API，风险较高，CSP 与 asset scope 已先行收口。
- tauri-plugin-updater：需要用户提供签名私钥与发布端配置。
- 字段级合并 / HLC / 全局唯一 ID / 分片或每设备一个文件的同步协议：协议级大改，先以墓碑 + 基准 + 写前合并止血。
- 端到端加密：cloud 需要读取明文，需与用户确认密钥管理方案。

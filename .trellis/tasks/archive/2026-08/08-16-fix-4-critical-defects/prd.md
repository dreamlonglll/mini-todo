# 修复 4 个 bug 级缺陷（导入事务 / 通知计数 / 云端竞态 / 迁移事务)

## Goal

2026-08-16 的全仓代码审查发现 4 个 bug 级缺陷（均已逐一读码验证，非猜测）：两个可导致用户数据永久丢失、一个可导致 debug 构建 panic / release 通知窗口错位、一个可导致应用"启动即失败"死循环。本任务逐项修复并补测试，不做任何顺带重构。

## Requirements

### R1. `import_data_raw` 包进单一事务

- 位置：`pc/src-tauri/src/commands/data.rs:177-223`
- 现状：先 `DELETE FROM subtasks` / `DELETE FROM todos` 再循环 INSERT，全程 autocommit。中途任一 INSERT 失败（磁盘满、IO 错误）→ 旧数据已删、新数据半截、无法回滚。手动导入与 WebDAV 应用远端**共用**此路径。
- 修法：整个导入体包进 `BEGIN IMMEDIATE ... COMMIT`，失败 `ROLLBACK`。注意 `with_connection` 只提供 `&Connection`（不可变引用，见 `db/connection.rs:47-53`），不能用 `conn.transaction()`（需 `&mut`），采用手动 `execute("BEGIN IMMEDIATE")` 或 `unchecked_transaction()`，与 `sync_cmd.rs` merge 路径（`:578` 附近）的现有模式保持一致。
- 附带收益：逐行 autocommit 变单次提交，大备份导入的 N 次 fsync 变 1 次。

### R2. 通知计数器双重递减修复

- 位置：`pc/src-tauri/src/services/notification.rs:227-246`
- 现状：窗口 `Destroyed` 事件里先 `ACTIVE_NOTIFICATIONS.fetch_sub`（:241）再 emit `notification-closed-{label}`（:242-243），该事件恰好触发 :228-233 注册的 listener 再 `fetch_sub` 一次 → 每关一个窗口计数减 2，从 0 下溢到 `u32::MAX`；后续通知的 y 坐标计算（`screen_height - ... - active_count * 130`）debug 构建算术溢出 panic，release 构建窗口位置错乱。
- 修法：只保留 `Destroyed` 事件中的一处递减，删除 `notification-closed-{label}` listener 及对应 emit（同时消除"listener 每条通知注册一次、从不 unlisten"的泄漏）；递减处用 `fetch_update` + `saturating_sub` 兜底，杜绝任何路径下溢。
- 验证前先 grep 前端确认无人依赖 `notification-closed-*` 事件（若有依赖则保留 emit、只删 Rust 侧 listener）。

### R3. cloud 端 pull/push dirty 竞态修复

- 位置：`cloud/src/sync/push.rs:47-70`（push 开始前即置 dirty=false）、`cloud/src/sync/pull.rs:175-176, 224-227`（merge 事务内读 dirty=false 即执行孤儿清理）
- 现状：push tick 先置 `dirty=false` 再做慢速网络 GET/merge/PUT；窗口期内 60s pull tick 并发触发时，孤儿清理会把"本地新建但尚未推送"的记录当孤儿删掉，随后 push 的快照里已无此记录，PUT 后**永久丢失**。同根变体：dirty=false 落盘后进程崩溃，重启 initial pull 同样误删。pull/push 是独立 `spawn_blocking`，仅有单条 SQL 级互斥；`POST /sync` 端点会加剧并发。
- 修法：
  1. pull / push / `POST /sync`（含 `/sync/pull`、`/sync/push`）共享一把 `tokio::sync::Mutex`，把整个同步操作串行化；
  2. dirty 语义改为"PUT 成功后才清"：push 开始时记录一个 generation 计数（或读取 dirty 时间戳），PUT 成功后仅当 generation 未变时清 dirty，保留"处理中新增的 dirty 不被覆盖"的原 CAS 意图；
  3. 连带修 `cloud/src/db/repo.rs:34-41` `get_meta` 吞错问题（`.optional().ok().flatten()` 把 DB 错误与"键不存在"混为 None）：改返回 `rusqlite::Result<Option<String>>`，pull 读 dirty 失败时**跳过孤儿清理**而非视为非 dirty——该缺陷与竞态叠加放大丢数据风险，属同一故障面。

### R4. 数据库迁移逐个包事务

- 位置：`pc/src-tauri/src/db/migrations.rs:21-149`
- 现状：26 个迁移全是"裸执行迁移体 + 单独 INSERT 版本号"，均 autocommit。多语句迁移（如 v23 三段 `execute_batch`）中途失败 → 前面 DDL 已生效但版本号未记录 → 下次启动重跑同一迁移在已变更的 schema 上再次报错，用户陷入"启动即失败"死循环且无法自救（SQLite DDL 可回滚，本可避免）。
- 修法：每个 `migration_vN(conn)` + 对应版本号 INSERT 包成一个事务（迁移失败则整体回滚，数据库停留在上一版本，下次启动可安全重试）。保持 26 个 if 块结构不变，只加事务包裹——**表驱动重构不在本任务范围**。

## Acceptance Criteria

- [ ] R1：新增单测——内存库先有数据，用会在中途失败的导入 JSON 触发失败后，原有 todos/subtasks 完好无损；正常导入路径回归通过
- [ ] R2：关闭一个通知窗口计数恰好减 1；计数为 0 时再触发关闭不下溢（saturating 兜底）；不再有随通知累积的 listener
- [ ] R3：新增测试覆盖竞态场景——push 进行中（dirty 已进入处理）时 pull 的孤儿清理不删除本地新建记录；`get_meta` 读 dirty 失败时孤儿清理被跳过；cloud 现有 1497 行集成测试全绿
- [ ] R4：新增单测——人为构造一个中途失败的迁移，版本号不推进且 schema 无半截变更，重试可继续
- [ ] `cd pc/src-tauri && cargo test` 与 `cd cloud && cargo test` 全部通过，`cargo check` 无新增警告

## Definition of Done

- 上述测试全部落地并通过
- 提交按缺陷拆分（每个 R 一个 commit），commit message 遵循 git-message-format skill 的中文规范
- 无行为外的顺带改动（重构、格式化、死代码清理一概不做)
- CLAUDE.md 无需更新（无 schema / 导出格式 / 同步字段变更）

## Technical Approach

四项修复相互独立，建议实现顺序 R1 → R4 → R2 → R3（前两个同为"包事务"最简单，R2 中等，R3 涉及并发语义与 worker 结构最复杂、测试成本最高）。

关键约束：
- PC 端 `Database::with_connection` 只给 `&Connection`（`db/connection.rs:47-53`），事务用手动 `BEGIN IMMEDIATE` / `unchecked_transaction()`，勿改 `with_connection` 签名（影响面太大）
- cloud 端 pull/push worker 均为 `spawn_blocking` 内同步代码，互斥锁在 async 外层（worker loop / axum handler）获取后再进入 blocking 段，避免在 blocking 线程里 block_on

## Decision (ADR-lite)

**Context**：R2 有两种修法——删 Destroyed 里的 emit+保留 listener，或删 listener+保留 Destroyed 递减。
**Decision**：保留 `Destroyed` 事件递减、删除 listener 与 emit（若 grep 确认前端无依赖）。
**Consequences**：一并消除 listener 泄漏；`Destroyed` 是窗口生命周期的权威信号，语义最准。

## Out of Scope

- 审查报告第二节（重复代码 / 巨石文件拆分）、第三节（性能打磨，含 N+1、deep watcher、client 复用）、第四节（死代码清理）的全部条目
- WebDAV 同步命令阻塞主线程问题（`reqwest::blocking`，体验级非 bug 级）
- migrations.rs 表驱动重构、settings 统一读写模块

## Technical Notes

- 缺陷来源：2026-08-16 三路并行代码审查（前端 / pc Rust / cloud），4 项均已在主会话逐一读码复核
- 参考的现有正确示范：`sync_cmd.rs:578` 附近 merge 的 `BEGIN IMMEDIATE` 用法；`connection.rs:57-59` 毒锁恢复
- cloud 端注意：`POST /sync`、`/sync/pull`、`/sync/push` 三个端点已用 `spawn_blocking`（`api/sync.rs:28, 66, 80`），加互斥后语义为"排队等待"而非"拒绝并发"

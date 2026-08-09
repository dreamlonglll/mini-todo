# fix: sync 路径改用 per-record merge，消除 id 漂移导致的 cloud 重复

## Goal

PC 端 sync 下载路径（`webdav_auto_sync`、`webdav_apply_remote`）误用了为手动文件导入设计的 `import_data_raw`（全清重建、丢弃原始 id），导致每次 sync 后 todo id 漂移。Cloud 端 pull 只增不删，旧 id 记录永远堆积，产生重复。

方案：sync 路径改走已有的 `merge_remote_into_local`（per-record LWW、保留原始 id），并在 merge 后加孤儿清理。`import_data_raw` 保持原样只用于手动文件导入。

## Requirements

### R1: PC 端 sync 下载改走 merge + 孤儿清理

- `webdav_auto_sync`（remote_is_newer && !has_local_changes 分支）和 `webdav_apply_remote` 改为：
  1. 调用 `merge_remote_into_local` 合并 todos/subtasks
  2. 删除"本地有但远端没有"的 todos + 对应 subtasks（孤儿清理）
  3. 从远端 SyncData 解析 `AppSettings` 并写入（`write_app_settings`）
- `merge_remote_into_local` 本身不改（412 冲突路径仍保留"本地有远端无→不删"语义）
- `import_data_raw` 不改，仅供手动文件导入使用

### R2: Cloud 端 pull 加孤儿清理

- `merge_into_sqlite` 结束后，删除本地 id 不在远端 id 集合内的 todos + subtasks
- 当 `meta.dirty == "true"` 时跳过清理（保护 cloud API 本地新建但还没 push 的记录）
- 删除的 todo 连带清理 `todo_seq` 表

### R3: MergeStats 扩展

- 给 PC 端 `MergeStats` 加 `todos_deleted` / `subtasks_deleted` 字段

## Technical Approach

### PC 端改动

| 文件 | 改动 |
|------|------|
| `commands/data.rs` | `write_app_settings` 改 `pub(crate)` |
| `commands/sync_cmd.rs` | 新增 `delete_orphan_todos` 函数；`webdav_apply_remote` 和 `webdav_auto_sync` 改调 merge + delete + write_settings |
| `commands/sync_cmd.rs` | `MergeStats` 加 deleted 字段 |

### Cloud 端改动

| 文件 | 改动 |
|------|------|
| `sync/pull.rs` | `merge_into_sqlite` 末尾加孤儿清理（dirty 时跳过）|
| `db/repo.rs` | 新增 `delete_todos_not_in_set` / `delete_subtasks_not_in_set` |

### 不改的文件

- `commands/data.rs::import_data_raw` — 手动导入保持全清重建
- `commands/sync_cmd.rs::merge_remote_into_local` — 412 路径保持"本地有远端无→不删"

## Acceptance Criteria

- [ ] PC sync 下载后 todo id 与远端一致（不再漂移）
- [ ] PC sync 下载后本地不存在远端已删除的 todo
- [ ] PC 412 冲突路径仍保留本地新增记录（不误删）
- [ ] Cloud pull 后不存在远端已消失的孤儿记录
- [ ] Cloud dirty 状态下 pull 不删除本地新建记录
- [ ] `cargo check` / `cargo test` green（PC + Cloud）

## Out of Scope

- 清理 cloud SQLite 中已积累的历史重复数据（运维手动处理）
- 修改 `import_data_raw`（手动导入场景保持原样）
- 修改 cloud push worker

# 交接：PC core 实现契约修订 §13（2026-11-09 暂停，原代理上下文已满，由新代理接手）

**工作位置**：worktree `D:\Git\mini-todo\.claude\worktrees\agent-a752448658028fe46`，分支 `worktree-agent-a752448658028fe46`。它已经快进到 `feat/multi-list` 的 `3453b32`。**只在这个目录里工作**：所有命令都先 `cd` 到这里，用 Edit / Write 改文件时用这里的绝对路径。不要碰主检出 `D:\Git\mini-todo`，另一个代理正在那里做第 4 步。

## 必读
- `D:\Git\mini-todo\.trellis\tasks\11-09-multi-list-windows\contract.md` 的 §13（取代 §12 的第 2、3、8 条）
- 同目录的 `notes-core.md`（core 第 1 步与 §12 的落点）
- 独立审查的结论（为什么要有 §13）：
  - F1：`listId` 跟着整条记录的版本走，过时副本会把旧归属沿用进新版本
  - F2：PC 在记录循环之前就执行了 `apply_tombstones_to_local`，丢掉了本该沿用的归属
  - F3：降级到 3.0.6 后，旧程序只改 `updated_at`，留下陈旧的 `list_id`
  - F4：被墓碑压制的远端版本，两端对它是否参与归属采纳的处理不一致
  - F5：清单墓碑 30 天过期后，已删的清单会复活
  - F6：畸形记录不该参与归属合并

## 未提交的半成品
- 已修改：`db/migrations.rs`、`db/models.rs`、`db/sync_store.rs`、`db/time.rs`
- 原代理停在"更新 `sync_store` 模块说明，然后改 `sync/mod.rs`"
- 先用 `git diff` 读懂这些改动，再接着做

## 剩余范围（以 contract §13 为准）
1. **v31 迁移**：`todos.list_updated_at` 可为 NULL，存量为 NULL
2. **模型**：`Todo.list_updated_at: Option<String>`，serde 名 `listUpdatedAt`，`#[serde(default)]`，未知时不写出
3. **写入**：只有显式设置清单时才刷新（superseding 规则）：新建、`move_to_list`、`UpdateTodoRequest.list_id` 为 `Some` 且值有变化、`lists::delete` 把待办移进默认清单。涉及 `todo.rs`、`lists.rs`
4. **合并**：记录级结果不变；只要结果记录存在，它的归属就取所有可见版本里最新的那个
   - 可见版本 = 本地合并前的版本（要在应用墓碑**之前**读出）+ 远端版本（即使被压制）
   - 新旧顺序：未知 < 已知但没有时间 < 按时间比较
   - 时间相同、值不同时，取较大的 `listId`
   - 采纳时不改 `updated_at`，也不额外上传
5. **补传**（`sync/mod.rs`）：合并后，远端那一版的归属比本地旧，就上传
6. **清单墓碑永不过期**：读、写、清理三处都去掉 30 天截止；todo 墓碑仍是 30 天
7. **导入**（`data.rs`）：3.0 / 4.0 备份 → `listId = 1`、`listUpdatedAt = 导入时刻`；4.1 备份里有归属的，把 `listUpdatedAt` 刷新为导入时刻
8. **畸形记录**：`listId` 类型错误、或记录的 `updatedAt` 无法解析时，不参与归属合并
9. **测试**：
   - F1 两种顺序都要收敛到 A 的移动
   - F2
   - F3：模拟旧程序只改 `updated_at`、不改 `list_id`，升级后远端更新的归属能胜出
   - F4
   - 时间相同时按确定性规则选出结果
   - 清单墓碑超过 30 天仍保留
   - 按新规则调整 §12 原有的测试
10. **app**：只做能编译的最小改动（`Todo` 字面量加字段之类）
11. 文档要点追加到 `notes-core.md`

## 约束
- 门槛（在这个 worktree 的 `pc-gpui/` 下）：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`
- 中文提交，`<type>: 摘要` 后接 `- ` 列表，中间不空行，用 heredoc 写
- 不 push，不跑 e2e

## 交付说明
新增的提交；§13 各条落在哪里；自行做的决定；门槛结果。

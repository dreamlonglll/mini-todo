# 交接：cloud 实现契约修订 §13（2026-11-09 暂停；原代理上下文已满，由新代理接手）

**工作位置**：worktree `D:\Git\mini-todo\.claude\worktrees\agent-a756b44451e920407`，分支 `worktree-agent-a756b44451e920407`，已快进到
`feat/multi-list` 的 `3453b32`。**只在这个目录里工作**：命令先 `cd` 到这里，Edit / Write 用这里的绝对路径；不要碰主检出 `D:\Git\mini-todo`。

## 必读
- `D:\Git\mini-todo\.trellis\tasks\11-09-multi-list-windows\contract.md` §13（取代 §12 的第 2、3、8 条）
- 同目录 `notes-cloud.md`（cloud 第 2 步与 §12 的落点）
- 独立审查在 cloud 副本上复现问题用的 scratch 用例（可改成正式用例）：
  `C:\Users\12197\AppData\Local\Temp\claude\e7dbbc3c0a5e587f\cd5b6110-4a67-4cba-850e-45e888d8cd10\scratchpad\cloudcopy\src\sync\review_tests.rs`
  （若该临时目录已不在，按下面的场景自己写）
- 审查要点：F1 cloud 先收到旧设备 O 的上传时，会把自己陈旧的 5 补传出去，吞掉 PC-A 的「移动到 6」或永久分叉；F4 被墓碑压制的远端版本
  两端处理不一致；F5 清单墓碑 30 天过期后已删清单复活；F6 cloud 会从畸形记录（listId 为 "5"、updatedAt 无法解析）采纳 listId

## 未提交的半成品
已改：`db/repo.rs`、`model.rs`、`sync/doc.rs`、`sync/merge.rs`、`sync/push.rs`；原代理停在"push.rs（不清理清单墓碑）与 doc.rs（上传时不截止清单墓碑、归属键）"。
先 `git diff` 读懂再接着做。

## 剩余范围（以 contract §13 为准）
1. `listUpdatedAt` 由服务端维护：客户端传入的忽略，响应照常返回
2. 只在显式设置清单时刷新（superseding）：`POST` 新建、`PATCH` 带 listId 且值变了、`DELETE /lists` 把待办移进默认清单（`api/`、`lists.rs`）
3. 合并：记录级结果不变；结果记录存在时，归属取全部可见版本（本地合并前版本、远端版本，含被压制的）里最新的；顺序：未知 < 已知无时间 < 按时间；
   同时间不同值取较大 listId；采纳不改 `updatedAt`、不标脏
4. 补传（`sync/pull.rs` 的远端落后判定）：合并后远端那一版的归属比本地旧 → 补传
5. 清单墓碑永不过期（读、写、清理、上传四处；todo 墓碑仍 30 天）
6. 畸形记录（listId 类型错误、记录 updatedAt 无法解析）不参与归属合并
7. 存量已知 listId、没有 listUpdatedAt 的记录按"已知无时间"处理，不补时间
8. 测试：F1 两种顺序都收敛到 A 的移动、F4、同时间确定性、清单墓碑超过 30 天仍保留、F6；mock_dav 场景两轮往返稳定不来回推；PC 模型副本加 `list_updated_at`
9. `cloud/README.md`、Skill 的 `SKILL.md` 补 `listUpdatedAt` 说明；`notes-cloud.md` 追加 §13 落点

## 约束
- 门槛（在该 worktree 的 `cloud/` 下）：`cargo fmt -- --check`、`cargo clippy --all-targets --locked -- -D warnings`、`cargo test --locked`、
  `python -m unittest discover -s skill/minitodo -p 'test_*.py'`
- 中文提交（`<type>: 摘要` + `- ` 列表，不空行，heredoc）；不 push

## 交付说明
新增提交、§13 各条落点、自行决定的地方、门槛结果。

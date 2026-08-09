# fix: P0~P2 健康度修复

## Goal

基于 2026-08-09 全仓扫描结果，修复 P0（真 bug）、P1（数据安全隐患）、P2（工程化/安全加固/文档漂移）三档问题。

## Requirements

### P0-1: minitodo.py `--json` 位置 bug

- `--json` 目前只在顶层 parser，放在子命令后直接 argparse 报错（exit 2）
- 修法：给各 subparser 也加 `--json`（父子均可接受），CLI 前置/后置写法都能用
- 文档不改写法（后置写法修完即合法）

### P0-2: minitodo.py `cmd_today` 时间边界 `T` 分隔符

- `minitodo.py:160-172` 用 `YYYY-MM-DDTHH:MM:SS` 拼 `dueDateAfter/Before`，服务端存空格分隔，字典序比较下 `' ' < 'T'`
- 症状：「今天到期」恒为空、今天的待办被归入逾期
- 修法：`T` → 空格

### P1-3: PC `delete_orphan_todos` 日志 + 单测

- 删除孤儿时输出日志（删除条数），`MergeStats` 的 deleted 字段不再被静默吞掉
- 给 `sync_cmd.rs` 的 merge/孤儿清理逻辑补第一批 PC 端单元测试（内存 SQLite）

### P1-4: 两端 Mutex 中毒修复

- PC `db/connection.rs:43,51`：`lock().unwrap()` → `unwrap_or_else(|e| e.into_inner())`
- cloud `db/mod.rs:48`：同样处理

### P1-5: cloud 测试补齐 + PC CI

- cloud `/sync` 三路由集成测试（成功/部分失败路径按可行性）
- cloud pull 孤儿清理测试：dirty=false 删除、dirty=true 跳过
- `.github/workflows/build.yml` 给 PC 加 `cargo test`

### P2-6: 前端 ESLint + Prettier

- ESLint flat config（vue + ts）+ Prettier 配置 + `lint`/`format`/`typecheck` scripts
- 只修 lint 报出的真错误，不做全仓格式化（避免大 diff 混入功能修复）

### P2-7: cloud 安全加固

- Bearer 比对改常数时间
- 32 MiB body limit 只给 `/images` 路由，其余恢复默认 2MB
- SVG 从上传白名单移除（存量 .svg GET 时以 `application/octet-stream` + attachment 返回）

### P2-8: 文档漂移清理

- `cloud/README.md`：REST 表补 `/sync` 三路由、CLI 列表补 `sync`、补 seq 短码说明
- 根 README：提醒示例改用 `#C{seq}`
- 临期窗口统一为 24h（SKILL.md 的 12h 改为 24h，与 openclaw.md 默认值/根 README 对齐）
- 新增 `cloud/skill/minitodo/install.ps1`（根 README 和 install.sh 都引用了它但文件不存在）
- `cloud/src/sync/pull.rs:9` 过期 TODO 注释、`webdav.rs`/`images.rs` 的过期 PR2 注释清理
- 清理真死代码：`repo::get_setting`/`list_todos`/`count_todos`、`time::now_local_string_in_tz` 等 `#[allow(dead_code)]`（保留测试用的 `has_tombstone`）
- openclaw.md `stale` 阈值描述与代码对齐（`pull_interval*2` stale、300s offline）

### 附带：Trellis 台账回填

- 已核实代码落地的任务标记完成：05-14-cloud-sync-api、05-14-fix-sync-id-drift、05-14-cloud-todo-seq、05-14-duedatebefore-duedate

## Out of Scope

- P3：拆大文件（EditorView.vue / window.rs / SettingsView.vue）、依赖升级
- 全仓 Prettier 格式化（另起 commit）
- PC 端 merge 之外的测试补齐
- magic bytes 校验、rate limit、请求日志中间件

## Acceptance Criteria

- [x] `minitodo.py today --json` / `minitodo.py --json today` 都能跑（12 种写法组合验证通过）
- [x] `cmd_today` 边界串用空格分隔（且下界兼容纯日期锚、overdue 上界取昨天 23:59:59）
- [x] PC / cloud `cargo test` green（PC 0 → 8，cloud 92 → 102）
- [x] cloud `/sync`（5 个）与孤儿清理（5 个）测试覆盖
- [x] `npm run lint`（0 error）/ `npm run typecheck` / `npm run build` green
- [x] 文档与代码行为一致（README /sync + seq、SKILL 24h、openclaw stale 阈值、install.ps1）
- [x] cloud `cargo fmt --check` + `cargo clippy -D warnings` 通过（CI 严格模式）

# feat: cloud sync API + skill/openclaw 文档更新

## Goal

Cloud 端新增 `POST /sync` 接口，让 skill / cron / AI agent 能手动触发 WebDAV 同步（pull + push），确保读写操作前后数据是最新的。同时更新 SKILL.md 和 openclaw.md 文档。

## Requirements

- `POST /sync` — 阻塞式触发 pull_once + push_tick，返回同步结果
- `POST /sync/pull` — 仅 pull
- `POST /sync/push` — 仅 push
- CLI: `minitodo.py sync [pull|push]` 子命令
- SKILL.md: 加 sync 子命令文档 + HTTP API 表更新
- openclaw.md: 提及 sync 接口用途

## Out of Scope

- 修改 pull/push worker 的后台循环逻辑

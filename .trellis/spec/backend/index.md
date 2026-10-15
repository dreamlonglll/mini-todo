# Backend (Rust / Tauri) Development Guidelines

> Code-specs for `pc/src-tauri/`. Each file is an executable contract: signatures, persisted keys,
> ordering rules, error matrix, required tests.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Window Modes](./window-modes.md) | Normal / fixed / fixed+embedded-in-desktop: atomics, Tauri commands, Win32 ordering rules, persistence, Explorer-restart recovery, e2e harness | Active |
| [Sync Protocol](./sync-protocol.md) | PC ↔ WebDAV ↔ cloud: document shape, time format, monotonic `updated_at`, record LWW + tombstones, settings LWW, WebDAV precondition rules measured on Apache / nginx | Active |
| [Desktop Fences](./desktop-fences.md) | 桌面收纳箱：始终嵌入桌面层的附属窗口（建窗标志、`EMBEDDED_WINDOWS` 登记与 200ms 轮询、移动 / 缩放 / 几何落库）、19 个命令与 `fence-items-changed`、v29 三张表、文件层（文件夹为 source of truth、对账、`FenceActivity`、删除保留语义）、按屏幕组合记忆位置、debug 专用环境变量、asset scope | Active |

---

## Pre-Development Checklist

Before touching `pc/src-tauri/src/commands/window.rs` or anything that changes the main window's
style, owner, Z-order or minimizability:

- [ ] Read [Window Modes](./window-modes.md) — especially "Ordering rules" and "Wrong vs Correct"

改桌面收纳箱（`commands/fence.rs`、`commands/fence_items.rs`、`services/fence_fs/`、`window.rs` 的嵌入附属窗口 /
`desktop_attach_hwnd`、`FenceView.vue` / `FenceItemGrid.vue`、`fences` / `fence_items` / `fence_layouts` 表）之前：

- [ ] 读 [Desktop Fences](./desktop-fences.md) —— 尤其 §3.1（建窗标志与可做的窗口操作）、§3.4（对账、upsert、登记与锁顺序、删除保留）、§7
- [ ] 会移动用户文件的改动：只在隔离环境（`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`）和 `__fence_e2e_` 测试文件上验证

Before touching sync, import/export, or any write path of `todos` / `subtasks` / `settings`
(PC commands or cloud REST handlers):

- [ ] Read [Sync Protocol](./sync-protocol.md) — §3.2 (time + monotonic `updated_at`) and §3.4 (WebDAV rules)
- [ ] Settings writes go through `db::settings_kv` only; synced keys live in `db::settings_kv::SYNCED_SETTING_KEYS`
- [ ] Any new `settings` key / table / record field: walk the "维护检查清单" in `CLAUDE.md`
  (models → data.rs → sync_store → cloud `src/model.rs`)

General:

- [ ] Commands doing network / file / bulk SQL work are `async` and run the blocking part in `spawn_blocking`
- [ ] Use `log::{info,warn,error}!` (persisted by tauri-plugin-log), not `println!`
- [ ] Any new `screen_configs` column: `SCREEN_CONFIG_COLUMNS` + `screen_config_from_row` + `save_screen_config`
- [ ] Any new Tauri command: register in `lib.rs` `use commands::{…}` **and** `generate_handler![…]`
- [ ] Commands that act on the main window resolve it with `app_handle.get_webview_window("main")`, never `window: Window`

## Quality Check

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green (`pc/src-tauri`, and `cloud/` when touched)
- Sync changes: run `pc/scripts/e2e-linux/run_e2e.py` S-suites (Apache + nginx) before claiming done
- Window-mode changes: run the e2e harness in [Window Modes §6](./window-modes.md#6-tests-required) before claiming done
- 收纳箱改动：跑 [Desktop Fences §6](./desktop-fences.md) 的单测与 e2e（含 `npm run test` 的 `utils/fence.test.ts`）

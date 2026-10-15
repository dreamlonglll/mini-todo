# Backend (Rust / Tauri) Development Guidelines

> Code-specs for `pc/src-tauri/`. Each file is an executable contract: signatures, persisted keys,
> ordering rules, error matrix, required tests.
>
> 同样适用于 GPUI 版的业务核心 `pc-gpui/crates/core`（`mini-todo-core`，由 `pc/src-tauri/src` 复制而来：`db/`、`services/` 原样，
> `commands/*.rs` 去掉 Tauri 外壳）。窗口 / 平台部分在 `pc-gpui/crates/app`，按 [pc-gpui 规范](../pc-gpui/index.md) 实现同样的语义契约；
> 文件对应与两边一起改的规则见 [pc-gpui/core-parity.md](../pc-gpui/core-parity.md)。

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

改 `pc-gpui/crates/core`（或改 `pc/src-tauri` 里 core 也有副本的部分）：

- [ ] 同时遵守 [pc-gpui 规范](../pc-gpui/index.md)：读 [Core Parity](../pc-gpui/core-parity.md)——数据库迁移、settings 键、记录字段、
      同步 / 合并 / 时间 / 墓碑规则、导出格式**两边一起改**（同号同 SQL），直到 `pc/` 删除；core 保持阻塞函数、不引入 GPUI / Tauri / tokio
- [ ] 两边 `cargo test` 都绿（`pc/src-tauri` 与 `pc-gpui/`），`db/`、`services/{webdav,secret}.rs`、`services/fence_fs/` 用 `diff` 自查一致

General:

- [ ] Commands doing network / file / bulk SQL work are `async` and run the blocking part in `spawn_blocking`
  （pc-gpui：core 函数保持阻塞，由 app 用 `db::spawn_db` / `background_executor` 放后台，见 [GPUI Conventions §2.1](../pc-gpui/gpui-conventions.md)）
- [ ] Use `log::{info,warn,error}!` (persisted by tauri-plugin-log), not `println!`
- [ ] Any new `screen_configs` column: `SCREEN_CONFIG_COLUMNS` + `screen_config_from_row` + `save_screen_config`
- [ ] Any new Tauri command: register in `lib.rs` `use commands::{…}` **and** `generate_handler![…]`
- [ ] Commands that act on the main window resolve it with `app_handle.get_webview_window("main")`, never `window: Window`

## Quality Check

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` green (`pc/src-tauri`, and `cloud/` when touched)
  ——core 有副本的改动另外在 `pc-gpui/` 下跑 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、
  `cargo test --workspace`（门槛见 [pc-gpui/verification.md](../pc-gpui/verification.md)）
- Sync changes: run `pc/scripts/e2e-linux/run_e2e.py` S-suites (Apache + nginx) before claiming done
  （改了 core 的同步再跑 `pc-gpui/scripts/e2e-linux/run_e2e.py` 的 S 组）
- Window-mode changes: run the e2e harness in [Window Modes §6](./window-modes.md#6-tests-required) before claiming done
- 收纳箱改动：跑 [Desktop Fences §6](./desktop-fences.md) 的单测与 e2e（含 `npm run test` 的 `utils/fence.test.ts`）

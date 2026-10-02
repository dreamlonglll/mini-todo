# Backend (Rust / Tauri) Development Guidelines

> Code-specs for `pc/src-tauri/`. Each file is an executable contract: signatures, persisted keys,
> ordering rules, error matrix, required tests.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Window Modes](./window-modes.md) | Normal / fixed / fixed+embedded-in-desktop: atomics, Tauri commands, Win32 ordering rules, persistence, Explorer-restart recovery, e2e harness | Active |
| [Sync Protocol](./sync-protocol.md) | PC ↔ WebDAV ↔ cloud: document shape, time format, monotonic `updated_at`, record LWW + tombstones, settings LWW, WebDAV precondition rules measured on Apache / nginx | Active |

---

## Pre-Development Checklist

Before touching `pc/src-tauri/src/commands/window.rs` or anything that changes the main window's
style, owner, Z-order or minimizability:

- [ ] Read [Window Modes](./window-modes.md) — especially "Ordering rules" and "Wrong vs Correct"

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

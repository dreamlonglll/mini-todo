# Backend (Rust / Tauri) Development Guidelines

> Code-specs for `pc/src-tauri/`. Each file is an executable contract: signatures, persisted keys,
> ordering rules, error matrix, required tests.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Window Modes](./window-modes.md) | Normal / fixed / fixed+embedded-in-desktop: atomics, Tauri commands, Win32 ordering rules, persistence, Explorer-restart recovery, e2e harness | Active |

---

## Pre-Development Checklist

Before touching `pc/src-tauri/src/commands/window.rs` or anything that changes the main window's
style, owner, Z-order or minimizability:

- [ ] Read [Window Modes](./window-modes.md) — especially "Ordering rules" and "Wrong vs Correct"
- [ ] Any new `settings` key: walk the 12-item "维护检查清单" in `CLAUDE.md` (models → data.rs → sync)
- [ ] Any new `screen_configs` column: `SCREEN_CONFIG_COLUMNS` + `screen_config_from_row` + `save_screen_config`
- [ ] Any new Tauri command: register in `lib.rs` `use commands::{…}` **and** `generate_handler![…]`
- [ ] Commands that act on the main window resolve it with `app_handle.get_webview_window("main")`, never `window: Window`

## Quality Check

- `cargo check` with 0 warnings, `cargo test` green (`pc/src-tauri`)
- Window-mode changes: run the e2e harness in [Window Modes §6](./window-modes.md#6-tests-required) before claiming done

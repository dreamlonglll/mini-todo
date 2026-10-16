# Backend Development Guidelines（PC 业务与平台层契约）

> Code-specs for the PC data / platform contracts. Each file is an executable contract: signatures, persisted keys,
> ordering rules, error matrix, required tests.
>
> 3.0 起 PC 端只有 GPUI 版 `pc-gpui/`（旧 Tauri 版 `pc/` 已删除，见 git 历史）：业务核心在 `pc-gpui/crates/core`（`mini-todo-core`，
> 阻塞函数，不依赖 GPUI / Tauri / tokio），窗口 / 平台部分在 `pc-gpui/crates/app`。GPUI 的写法约定见 [pc-gpui 规范](../pc-gpui/index.md)，
> 数据结构改动对 cloud 与已安装旧版本的影响见 [pc-gpui/data-compat.md](../pc-gpui/data-compat.md)。

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Window Modes](./window-modes.md) | 主窗口普通 / 经典固定 / 固定 + 嵌入桌面：模式状态表、`services::window_mode` 的切换与 200ms 轮询、`platform::embed` / `subclass`、持久化键、Explorer 重启自愈、e2e | Active |
| [Sync Protocol](./sync-protocol.md) | PC ↔ WebDAV ↔ cloud: document shape, time format, monotonic `updated_at`, record LWW + tombstones, settings LWW, WebDAV precondition rules measured on Apache / nginx | Active |
| [Desktop Fences](./desktop-fences.md) | 桌面收纳箱：始终嵌入桌面层的窗口（建窗参数、嵌入守护、移动 / 缩放 / 几何落库）、core `fence` / `fence::items` 接口与 `FenceRefresh`、v29 三张表与 v30 隐藏图标两列（点开头自动隐藏、右键隐藏、显示隐藏的图标）、文件层（文件夹为 source of truth、对账、`FenceActivity`、删除保留语义）、按屏幕组合记忆位置、debug 专用环境变量 | Active |

---

## Pre-Development Checklist

改主窗口的样式、owner、Z 序、可最小化或几何锁定（`pc-gpui/crates/app/src/services/window_mode.rs`、`platform/embed.rs`、
`platform/subclass.rs`、`platform/windows.rs`）之前：

- [ ] Read [Window Modes](./window-modes.md) — especially §3 "Ordering rules" and "Wrong vs Correct"

改桌面收纳箱（core `fence/`、`services/fence_fs/`；app `services/fences.rs`、`windows/fence/`、`platform/embed.rs`；`fences` /
`fence_items` / `fence_layouts` 表）之前：

- [ ] 读 [Desktop Fences](./desktop-fences.md) —— 尤其 §3.1（建窗参数与可做的窗口操作）、§3.4（对账、upsert、登记与锁顺序、删除保留）、§7
- [ ] 会移动用户文件的改动：只在隔离环境（`MINI_TODO_DATA_DIR` + `MINI_TODO_FENCES_ROOT`）和 `__fence_e2e_` 测试文件上验证

Before touching sync, import/export, or any write path of `todos` / `subtasks` / `settings`
(core functions or cloud REST handlers):

- [ ] Read [Sync Protocol](./sync-protocol.md) — §3.2 (time + monotonic `updated_at`) and §3.4 (WebDAV rules)
- [ ] Settings writes go through `db::settings_kv` only; synced keys live in `db::settings_kv::SYNCED_SETTING_KEYS`
- [ ] Any new `settings` key / table / record field: walk the "维护检查清单" in `CLAUDE.md`
  (models → data.rs → sync_store → cloud `src/model.rs`) and [pc-gpui/data-compat.md](../pc-gpui/data-compat.md)

General:

- [ ] core 函数保持阻塞；网络 / 文件 / 批量 SQL 由 app 用 `db::spawn_db` / `background_executor` 放后台（[GPUI Conventions §2.1](../pc-gpui/gpui-conventions.md)），
      UI 主线程上不做阻塞调用
- [ ] 带 `owner_hwnd` 的收纳箱移动（`fence::delete`、`fence::items::{move_in, restore, move_to}`）绝不在 UI 主线程调用
- [ ] Use `log::{info,warn,error}!`（app 的 `services::logging` 落盘），not `println!`
- [ ] Any new `screen_configs` column: core `settings.rs` 的 `SCREEN_CONFIG_COLUMNS` + `screen_config_from_row` + `save_screen_config`
- [ ] 窗口代码里不直接写 Win32：新能力加在 `crates/app/src/platform/`（Windows 实现 + `fallback.rs`），写操作遵守 App 借用规则
      （[GPUI Conventions §2.2](../pc-gpui/gpui-conventions.md)）

## Quality Check

- 在 `pc-gpui/` 下 `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、
  `cargo test --workspace --locked` 全绿（门槛见 [pc-gpui/verification.md](../pc-gpui/verification.md)）；改了 `cloud/` 再跑 cloud 的
  `cargo test --locked`
- Sync changes: run `pc-gpui/scripts/e2e-linux/run_e2e.py` S-suites (Apache + nginx) before claiming done
- Window-mode changes: run the e2e in [Window Modes §6](./window-modes.md#6-tests-required) before claiming done
- 收纳箱改动：跑 [Desktop Fences §6](./desktop-fences.md) 的单测与 Windows e2e（F 组）

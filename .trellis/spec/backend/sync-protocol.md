# Sync Protocol (PC ↔ WebDAV ↔ cloud)

> Code-spec for the shared `sync-data.json.gz` channel. Two independent writers implement it:
> PC (`pc-gpui/crates/core/src/sync/mod.rs` + `db/sync_store.rs` + `db/time.rs` + `db/settings_kv.rs` + `services/webdav.rs`) and cloud
> (`cloud/src/sync/*`). Every rule below must hold on **both** sides — a rule implemented on one side
> only silently loses or resurrects data on the other.
> Established in task `10-02-full-optimization` (2026-10-02, on the old Tauri PC app whose sync code core copied verbatim);
> server behaviour measured against Apache 2.4 mod_dav and nginx 1.24 dav/dav_ext, then verified end-to-end with the real app.
> Current e2e: `pc-gpui/scripts/e2e-linux/` S-suites, run once per server.
>
> 旧 Tauri 版（已删除）的 `webdav_sync` / `webdav_force_pull` / `webdav_force_push` 三个命令在 core 里合并为 `sync::run(db, SyncMode)`，
> `sync-completed` 事件改为返回值 `SyncReport`（app 的 `services::sync::run_sync` → `AppState::apply_sync_report`）。仍在用旧版本的设备
> 也是这份文档的写入方，改文档形状前读 [pc-gpui/data-compat.md](../pc-gpui/data-compat.md)。

---

## Scenario: Record-level sync through one WebDAV file

### 1. Scope / Trigger

- Applies to any change to: the sync document shape, record fields, timestamps written to
  `updated_at` / `deleted_at`, tombstones, settings that sync, WebDAV request headers, the 412 retry
  loop, or the "base version" bookkeeping.
- Also applies to every **local write path** that touches `todos` / `subtasks` / `todo_lists` (core `todo` / `lists` functions,
  import, cloud REST handlers): they feed the LWW comparison. Only writes that **explicitly set a list** (create, move to list,
  editing with a changed list, deleting a list) may touch `listId` / `listUpdatedAt`.

### 2. Signatures

```rust
// PC — pc-gpui/crates/core/src/sync/mod.rs (blocking; reqwest::blocking, never inside a tokio runtime; one sync at a time)
pub enum SyncMode { Smart { full: bool }, ForcePull, ForcePush }
pub fn run(db: &Database, mode: SyncMode) -> Result<SyncReport, String>;
//   Smart { full: true }:  title-bar button / Settings "立即同步" — unconditional first GET
//   Smart { full: false }: auto sync (every FULL_GET_EVERY-th round is unconditional anyway)
//   ForcePull: local := remote;  ForcePush: remote := local
// re-entry while a sync runs -> Err("同步正在进行中"); not configured -> Err("未配置 WebDAV 服务器")
// SyncReport (camelCase): status no_changes|pulled|pushed|merged, lastSyncAt,
//   todos/subtasks Inserted/Updated/Deleted, listsInserted/Updated/Deleted, recordsSkipped, imagesUploaded/Downloaded, settingsApplied
// SyncReport::local_changed() (incl. lists) / lists_changed() / settings_applied tell the app what to reload:
//   app services::sync::run_sync (background) -> AppState::apply_sync_report (refetch todos + lists / reload settings)
//   adopting only a todo's list assignment (§3.3 rule 9) also counts as todosUpdated
// fn remote_assignments_stale(doc, remote) -> bool   // §3.4 self-heal: the remote's list assignment is older than ours

// PC — db/time.rs
pub const CANONICAL_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub fn normalize_datetime(s: &str, default: DefaultTime) -> Option<String>;
pub fn superseding(now: &str, previous: &str) -> String;     // max(now, previous + 1s)
pub const SQL_SET_UPDATED_AT: &str;                          // same rule inside UPDATE statements
pub const SQL_SET_LIST_UPDATED_AT: &str;                     // list_updated_at, same rule; pair with `WHERE list_id IS NOT ?`
pub const SQL_SET_LIST_UPDATED_AT_IF_MOVED: &str;            // multi-field UPDATE: bumps only when the anonymous `?` (new list_id) differs

// PC — db/sync_store.rs
pub const TOMBSTONE_RETENTION_DAYS: i64 = 30;               // todo / subtask tombstones only; list tombstones never expire
pub fn merge_remote(..) -> Result<MergeStats>;               // one transaction (todos, subtasks, tombstones, list assignments)
pub fn merge_remote_lists(tx, Option<&[RemoteList]>, &[ListTombstone]) -> ..;   // None = remote has no `todoLists` key
pub fn force_pull_replace_lists(tx, Option<&[RemoteList]>, &[ListTombstone]) -> ..;
pub fn force_push_prepare_lists(tx, Option<&[Value]>, &[ListTombstone], now) -> ..;
pub fn parse_list_tombstone(v: &Value) -> Option<ListTombstone>;    // entityId 1 dropped
pub fn record_list_tombstone(conn, list_id, deleted_at) -> Result<()>; // never stores id 1
pub fn load_list_tombstones(conn) -> Result<Vec<ListTombstone>>;     // all of them (no cutoff)
pub struct ListAssignment;   // (listId, listUpdatedAt): new / of / from_value / is_newer_than / newest (§3.3 rule 9)
pub(crate) fn deletion_time(now: &str, record_updated_at: &str) -> String; // max(now, record.updatedAt)

// cloud — src/time.rs
pub fn bump_updated_at(now: &str, previous: &[&str], tz: Tz) -> String;   // == superseding
pub fn deletion_time(now: &str, record_times: &[&str], tz: Tz) -> String; // == PC deletion_time

// cloud — lists
// sync::doc::{KEY_TODO_LISTS, KEY_LIST_TOMBSTONES, ListTombstone, strip_record_keys}
// sync::merge::apply_remote_doc (lists in merge_lists); model::is_pristine_default_list
// model::{Ownership, ownership_of, newest_ownership, apply_ownership}  // == PC ListAssignment
```

Persisted PC settings keys (device-local, never synced): `webdav_remote_etag`, `webdav_last_modified`
(recorded only), `webdav_synced_settings_at`, `webdav_last_sync_at`, `webdav_device_id`. Change
counters live in `sync_meta(local_seq, synced_seq)`, maintained by triggers on `todos` / `subtasks` / `todo_lists`
(`list_tombstones`, like `tombstones`, has no trigger). Migration v31 clears `webdav_remote_etag` once.
其他设备本地数据（屏幕配置、桌面收纳箱、`fence_onedrive_notice_shown`、`last_run_version`）见 §3.6。
cloud keeps the equivalent in `meta` (`dirty`, `base_etag`, `base_last_modified`, `remote_envelope`, one-shot `lists_base_reset_v1`).

### 3. Contracts

#### 3.1 Document (`/mini-todo/sync-data.json.gz`, gzip JSON)

```jsonc
{
  "version": "4.1", "deviceId": "…", "updatedAt": "<ISO with offset, metadata only>",
  "todos": [ { /* Todo, camelCase, nested "subtasks": [...]; "listId": 5, "listUpdatedAt": "YYYY-MM-DD HH:MM:SS" */ } ],
  "todoLists": [ { "id": 1, "name": "待办清单", "sortOrder": 0, "createdAt": "…", "updatedAt": "…" } ],
  "listTombstones": [ { "entityId": 1731000000000123, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ],
  "settings": { /* AppSettings */ }, "settingsUpdatedAt": "YYYY-MM-DD HH:MM:SS",   // optional
  "images": ["<safe file name>"],
  "tombstones": [ { "entityType": "todo" | "subtask", "entityId": 123, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ]
  // unknown top-level keys: every writer carries them through unchanged
}
```

- `entityId` is an integer (PC ids are `i64`; cloud-created ids and new list ids on both sides are `millis * 1000 + rand(0..999)`,
  always < 2^53 so consumers that parse JSON numbers as doubles — the old Tauri JS frontend did — keep them exact;
  PC `ids.rs` and cloud `api/ids.rs` share the constants).
- Tombstones older than 30 days are dropped on write and purged locally. **List tombstones never expire.**
- `version` is `EXPORT_VERSION` (`"4.1"`); neither old PC nor cloud checks it (cloud passes it through).
- 多清单（任务 `11-09-multi-list-windows`，契约在该任务的 `contract.md`）：
  - todo 的 `listId`（整数）：**缺失 / `null` = 未知**（写入方不认识清单），不等于默认清单 1；类型错误按无法识别的记录跳过。
    `listUpdatedAt`（规范时间）是 `listId` 自己的版本；缺失 / 无法识别 = 已知但没有版本；非字符串当作缺失、不让整条记录作废。
    归属未知时两个键都不写
  - `todoLists`：每次上传都写出全部未删除的清单（含 id 1）；`listTombstones`：全部清单墓碑（永不过期）。两者是 `SyncData` 的显式字段，
    不放进 `extra`
  - 清单墓碑**不进** `tombstones`：v2.5.0 / 3.0.x 的 `parse_tombstone` 丢弃未知 `entityType`，上传时又按本地表重建 `tombstones`，
    旧设备同步一次就会抹掉 `"list"` 墓碑，未同步到删除的新设备随后按并集把清单复活；新顶层键经旧版本的 `#[serde(flatten)] extra` 原样带回
  - 默认清单 id 1 固定初始值 `"待办清单"` / `sortOrder` 0 / `2000-01-01 00:00:00`（PC 迁移 v31 建，cloud 惰性补建），两端各自建出的不互相抢

#### 3.2 Time (K1)

- Stored and synced record times are `YYYY-MM-DD HH:MM:SS`, local wall clock, no zone.
  Readers accept space/`T`, missing seconds, fractional seconds, `Z`/`±HH:MM` (converted to local),
  date-only (start 00:00:00, end 23:59:00, notify 09:00:00).
- cloud converts with `config.timezone` on **every** call (no cached offset — DST).
- **Monotonic versions**: a local write to an existing record sets
  `updated_at = max(now, previous_updated_at + 1s)`. Without it two edits in the same second share a
  timestamp, and "tie keeps local" makes the other side discard the second edit forever (found by
  e2e S05 on nginx). The same applies when the previous version came from a device whose clock runs
  ahead: plain `now` would lose to the version it replaces.
- A tombstone's `deletedAt = max(now, record.updatedAt)` (only parseable times count), so a delete
  always suppresses the version it deleted.
- Synced settings move as one block: writing any `SYNCED_SETTING_KEYS` key (PC `db/settings_kv.rs`)
  stamps `max(now, settings_version + 1s)`, so `settingsUpdatedAt` also strictly increases.
- Unparseable or missing `updatedAt` compares as the **oldest** value on both sides (PC
  `sync_store::value_updated_at`, cloud `canon_ts`) — a garbage string must never win LWW or be
  copied into a new version.

#### 3.3 Merge (identical on PC `merge_remote` and cloud `sync/merge.rs`)

1. Record LWW on normalized `updatedAt`; **ties keep the local version**; unparseable/missing
   `updatedAt` is the oldest possible value.
2. A tombstone with `deletedAt >= record.updatedAt` deletes/suppresses the record; a todo tombstone
   takes its subtasks with it. A record edited after deletion (`updatedAt > deletedAt`) survives.
3. Records present on one side only are kept (union). **No "absent means deleted".**
   Legacy exception (cloud only): a remote document without a `tombstones` key, while cloud is not
   dirty, may still prune records missing from it.
4. Tombstones: union, max `deletedAt` per key, 30-day retention.
5. Settings (PC only): apply remote `settings` iff remote `settingsUpdatedAt` > local
   `max(updated_at)` over `SYNCED_SETTING_KEYS`, or this device never synced with this remote.
   `windowPosition` / `windowSize` are never applied from remote. cloud passes settings through and
   never invents `settingsUpdatedAt`.
6. A remote record that fails to deserialize is skipped and counted (`recordsSkipped`) — never treated
   as a deletion. A remote document that fails to parse as a whole aborts the sync **without upload**.
7. Subtasks merged under a remote todo take the outer todo's id as `parentId`.
8. **Lists** (PC `merge_remote_lists`, cloud `merge::merge_lists`) merge like todos: LWW (ties keep local), `deletedAt >= updatedAt`
   suppresses, union, list tombstones union with max `deletedAt` — but **never expire**. A remote without a `todoLists` key carries no
   list information: no local list is deleted. The default list (id 1) cannot be deleted; `entityId = 1` list tombstones are ignored
   on both sides (not stored, not counted as skipped). Receiving a list tombstone does **not** rewrite todos (readers fall back to the
   default list via `effective_list_id`). PC deletes the window records (`window_prefs` / `window_layouts`) of a deleted list in the
   same transaction. Bad list records are skipped and counted.
9. **List assignment** (契约 §12 + §13): a todo's `(listId, listUpdatedAt)` is merged **field-level**, independent of the record's
   `updatedAt`. The record-level outcome is unchanged; if the resulting record exists, its assignment is the newest among all visible
   versions — the local pre-merge version (PC reads it with `load_assignments` **before** applying tombstones) and the remote one, even
   when that remote version is suppressed by a tombstone. Order: unknown < known without version < known with version (by time);
   equally new but different → the larger `listId`. Adopting an assignment does not touch `updatedAt`, does not mark cloud dirty,
   and does not trigger a PC upload (PC `set_assignment` writes two columns; `synced_seq` takes the post-merge `local_seq`).
   Malformed records (wrong `listId` type, unparseable `updatedAt`) take no part. PC skips such a record entirely; cloud stores the
   body but takes the assignment from the other versions — both converge.

#### 3.4 WebDAV requests (measured — see Design Decisions)

| Step | Rule |
|---|---|
| Read | `If-None-Match: <base ETag>` only when nothing local is pending and a base ETag exists; otherwise unconditional GET. Manual sync and every 10th round use an unconditional GET (see "Full GET"). **Never `If-Modified-Since`.** |
| Write | Always GET + merge first. PUT with `If-Match: "<opaque>"` (strip `W/`) when a base ETag exists, else no precondition. **Never `If-Unmodified-Since`.** |
| 412 | Wait ≥ 1.1 s, then unconditional GET → merge → PUT. At most 4 attempts (PC) / 3 attempts (cloud) per sync. |
| 404/409 on PUT | MKCOL the parent chain once, retry. No MKCOL on every sync. |
| New base | ETag from the PUT response → HEAD → PROPFIND Depth 0 `getetag`. If the probed size ≠ uploaded size, record no base (next sync does a full GET). Never record a base for content that was not merged. |
| Images | Upload missing images **before** the document; list remote images with one PROPFIND Depth 1 (any namespace prefix, single- or multi-line XML); names must pass `is_safe_image_name` (`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`, no `..`). |
| Self-heal | After merging, if the remote document lacks local records/tombstones (a server that ignores preconditions let someone overwrite us), upload again even when nothing changed locally. Same for lists / list tombstones — except a **pristine default list** (id 1, `updatedAt <= 2000-01-01 00:00:00`), which never counts on either side (PC compares a remote without `todoLists` against that seed state; cloud `is_pristine_default_list`). Also upload once when the remote's assignment of some todo is older than ours (remote `listId` missing while known locally; same time but different `listId` → the side holding the larger one) — PC `remote_assignments_stale`, cloud `remote_lacks_local_ownership`. cloud skips both comparisons for legacy documents without `tombstones`. |
| First sync after upgrade | PC migration v31 clears `webdav_remote_etag`; cloud's `lists_base_reset_v1` drops its base once. The first GET is unconditional, so stored todos (all "unknown" after the upgrade) learn their lists right away. |
| Full GET (both) | Unconditional GET on every 10th round — cloud pull loop (`FULL_PULL_EVERY`), PC smart sync (`FULL_GET_EVERY`, process-wide counter) — and on every manual sync (cloud `/sync`, `/sync/pull`; PC `SyncMode::Smart { full: true }` from the title-bar button and Settings "立即同步"): nginx ETags have 1-second resolution and hide same-second, same-size rewrites behind 304. |

#### 3.5 Force operations (PC)

- `SyncMode::ForcePull` (Settings「用云端覆盖本地」, with confirmation): local := remote; deletes local-only records
  **without** tombstones; applies remote settings (device keys excluded) regardless of `settingsUpdatedAt`. Missing remote → `Err`.
  Lists: local lists := remote (local-only lists deleted without tombstones, the default list always kept), local list tombstones
  replaced by the remote's; a remote **without** `todoLists` leaves lists and list tombstones alone. A remote todo without `listId`
  keeps the locally known assignment (both columns); a known remote assignment is copied with its version.
- `SyncMode::ForcePush` (Settings「用本地覆盖云端」, with confirmation): remote := local; writes tombstones for remote-only
  records (otherwise cloud's next push merges them straight back), bumps local versions with the monotonic rule, conditional PUT.
  Lists: list tombstones for remote-only lists (not id 1); local lists the remote would beat (newer version / tombstone) get a
  superseding `updated_at`. Assignments: unknown locally → adopt the remote's; remote newer → local version pushed to
  `superseding(now, remote version)`.

#### 3.6 设备本地数据（不同步）

以下数据只对本机有意义（路径、屏幕坐标、本机提示状态），**不进**同步文档、`ExportData` / `SyncData`、导入导出，cloud 也不知道它们：

| 数据 | 引入 | 说明 |
|---|---|---|
| `screen_configs` 表 | 早期 | 屏幕组合的登记（3.1 起只由默认清单窗口保存时写入；窗口自己的几何与锁定在 `window_layouts`） |
| `window_prefs` 表 | v31 | 清单窗口 `list:<id>` / 日历窗口 `calendar` 的隐藏 / 深色 / 视图 / 展开全部 / 日历清单筛选 |
| `window_layouts` 表 | v31 | 每 (窗口, 屏幕组合) 的客户区物理像素几何与锁定（随 `window_prefs` 级联删除） |
| `fences` 表 | v29（v30 加 `show_hidden`） | 桌面收纳箱元数据（标题、收纳文件夹名、物理像素几何、折叠 / 锁定 / 外观 / 图标大小 /「显示隐藏的图标」） |
| `fence_items` 表 | v29（v30 加 `hidden`） | 收纳箱项目的排序、原始路径与隐藏标记（内容以本机收纳文件夹为准） |
| `fence_layouts` 表 | v29 | 收纳箱按屏幕组合记忆的位置 |
| settings `fence_onedrive_notice_shown` | v29 | 首次收纳时的云同步一次性提示是否已显示（Windows OneDrive；macOS 的 iCloud 提示共用这个键，键名不变） |
| settings `last_run_version` | 3.1 更新日志（**没有迁移**，键不存在 = 没记录过） | 本机上次运行的程序版本（core `settings::LAST_RUN_VERSION_KEY`，`last_run_version` / `set_last_run_version`）：app 的更新日志弹窗据此判断"升级后第一次运行"、显示哪些版本（`services::changelog`）。每台电脑各自升级、各自弹一次，所以不同步 |
| settings `webdav_remote_etag` / `webdav_last_modified` / `webdav_synced_settings_at` / `webdav_last_sync_at` / `webdav_device_id` | 多个版本 | 同步自身的本机状态（见 §2），WebDAV 连接配置同样不同步 |

规则：

- 三张收纳箱表与 `window_prefs` / `window_layouts` **不接** `sync_meta.local_seq` 的变更计数触发器、不写 `tombstones`：写它们不能让同步认为
  "有本地变更"去上传。清单被删除（本机 / 同步 / 强制拉取 / 导入）时它的窗口记录在同一事务里删除；删屏幕配置连带删该组合的 `fence_layouts` 与
  `window_layouts`（`settings::delete_screen_config`）。
- 同步键 `is_fixed` / `text_theme` / `view_mode` / `show_calendar` 仍在 `SYNCED_SETTING_KEYS`（旧设备照常读写），但 3.1 起只作迁移 v31 的
  初始值，界面改读写上面两张设备本地表。
- 设备本地的 settings 键不进 `AppSettings`、不进 `db::settings_kv::SYNCED_SETTING_KEYS`，`read_app_settings` / `write_app_settings`
  不读写它们，因此导入备份、强制拉取（`SyncMode::ForcePull`）应用远端设置时也不会覆盖它们。`fence_onedrive_notice_shown` 与 `last_run_version`
  是对 `CLAUDE.md`「维护检查清单」第 1 条的**有意偏离**（前者原因见 [desktop-fences.md §3.3](./desktop-fences.md)；后者是本机状态，
  放进 `AppSettings` 会随导入备份 / 强制拉取被改写，导致升级后不弹或重复弹）。
- 新增设备本地数据时：在本表登记，并在 `CLAUDE.md`「当前同步覆盖范围」表里标"否"。
- 测试：`migrations::v29_fence_writes_do_not_count_as_sync_changes`（增删改收纳箱表后 `local_seq` 不变、墓碑数为 0）、
  `migrations::v30_hidden_flags_do_not_count_as_sync_changes`（写 `hidden` / `show_hidden` 不计 `local_seq`）、
  `fence::items` 的 `onedrive_notice_is_claimed_once_and_stays_device_local`（不在 `SYNCED_SETTING_KEYS`，导出的 `AppSettings` JSON 里没有它）、
  `migrations::v31_window_tables_are_device_local_and_layouts_cascade`、`window_prefs::window_prefs_are_device_local`、
  `settings::last_run_version_round_trips_and_stays_device_local`（不在 `SYNCED_SETTING_KEYS`、不改设置版本与 `local_seq`、`AppSettings` 与导出文件里都没有它）。

### 4. Validation & Error Matrix

| Condition | PC | cloud |
|---|---|---|
| Sync already running | `Err("同步正在进行中")` | waits on the async sync mutex (`SyncCtx::run_locked`) |
| Remote document missing (404) | smart sync uploads; force pull errors | push creates it |
| Remote not gzip / not JSON / not an object | error, nothing uploaded | pull error, cache untouched, never uploads over it |
| `todoLists` / `listTombstones` present but not an array (nor null) | whole document unreadable: error, nothing uploaded | same as a bad `todos` |
| List record without integer `id` / unparseable | skipped, `recordsSkipped += 1` | non-integer `id`: skipped and counted; other fields kept verbatim (KV) |
| One bad record | skipped, `recordsSkipped += 1`, logged | no id / not an object: skipped and counted; unknown fields: kept verbatim |
| 412 on every attempt | `Err("同步失败：远端在连续 4 次尝试中都被其它设备同时修改，请稍后重试")` | push fails after 3 attempts, `dirty` stays set, worker retries with backoff |
| Unsafe image name | skipped + logged | skipped + logged; API returns 400 |
| REST write with unknown field / wrong type | — | 400 listing the field and allowed fields |

### 5. Good / Base / Bad Cases

- Good: PC and cloud edit **different** records within the same minute → both edits survive on both
  sides after one sync round each.
- Base: PC edits a record twice within one second, cloud pulled the first version in between → the
  second version has `updatedAt` one second later and wins everywhere.
- Bad (must not happen): a todo created locally today disappears after sync because the remote
  file is "newer" (old whole-file "remote wins" logic, audit A1); a cloud-deleted todo comes back
  after the next PC sync (missing tombstone).

### 6. Tests Required

- PC `cargo test -p mini-todo-core` (in `pc-gpui/`): `db/sync_store.rs` merge/tombstone/settings cases, `db::time`
  parsing + `superseding`, and `sync/tests.rs` — the full sync against an in-process `tiny_http` mock WebDAV in
  Apache and nginx modes (weak ETag window, IUS 412, missing validators, nginx same-second blind spot).
  Lists: `sync_store` `remote_lists_merge_by_lww_and_ties_keep_local`, `missing_list_id_means_unknown_and_never_overrides_a_known_list`,
  `unknown_local_todos_adopt_a_known_remote_list_whoever_wins`, `force_pull_keeps_a_known_list_when_the_remote_does_not_know_it`,
  `force_push_prepare_adopts_known_remote_lists_for_unknown_local_todos`, `list_assignments_order_by_their_own_version`,
  `equally_new_assignments_pick_the_larger_list_id`, `a_stale_copy_cannot_undo_a_newer_move_in_either_order`,
  `tombstones_do_not_hide_assignment_versions`, `a_downgraded_device_learns_the_newer_list_after_upgrading`,
  `malformed_remote_records_take_no_part_in_assignments`; `sync/tests.rs` `lists_and_list_deletions_sync_in_both_directions`,
  `old_devices_keep_list_keys_and_their_edits_keep_the_local_list`, `upgraded_old_device_learns_list_ids_without_uploading`,
  `two_devices_creating_lists_offline_keep_both`, `force_push_and_force_pull_cover_lists`, `a_stale_copy_cannot_undo_a_move_in_either_order`,
  `a_stale_assignment_on_the_remote_is_fixed_by_one_upload`, `list_tombstones_never_expire`.
- cloud `cargo test`: `sync/merge.rs` unit tests, `sync/scenario_tests.rs` on `mock_dav`
  (`MockDav::start_apache` / `start_nginx`), API integration tests incl. "no-op PATCH does not bump
  `updatedAt`". Lists: `merge.rs` list / ownership cases (`stale_ownership_never_beats_a_newer_move_in_either_order`,
  `ownership_from_a_suppressed_remote_version_is_adopted`, `malformed_records_take_no_part_in_ownership`,
  `list_tombstones_take_max_and_never_expire` …), scenario tests that check two round trips settle without another push
  (`stale_cloud_ownership_yields_to_a_newer_move_after_its_repush`, `same_time_ownerships_converge_on_the_larger_list_id` …),
  `api/integration_tests.rs` (`/lists`, `list_updated_at_is_server_maintained`, `pushed_records_deserialize_into_pc_model`).
- e2e (`pc-gpui/scripts/e2e-linux/run_e2e.py`, S-suites once per server, driven through the real GPUI UI): first sync / no change,
  title-bar sync, cloud writes and aliases, deletion both ways without resurrection, two devices, monotonic versions,
  images, settings LWW + unknown top-level key preserved, force pull/push, PC + cloud concurrent writes, auto sync
  (case list in that README).

### 7. Wrong vs Correct

#### Wrong — `If-Unmodified-Since` as the fallback precondition

```rust
// Apache compares IUS against the file's sub-second mtime: 412 even for a file nobody touched
req.header("If-Unmodified-Since", base_last_modified);
```

#### Correct — `If-Match` or nothing; correctness comes from GET + merge before every PUT

```rust
// services/webdav.rs
pub fn choose_put_precondition(base: &RemoteVersion) -> Precondition {
    match base.etag.as_deref().map(opaque_etag) {          // strips W/
        Some(tag) if !tag.is_empty() => Precondition::IfMatch(tag.to_string()),
        _ => Precondition::None, // still safe: the latest GET was merged right before this PUT
    }
}
```

#### Wrong — "now" as the new version time

```sql
UPDATE todos SET title = ?1, updated_at = datetime('now','localtime') WHERE id = ?2
```

#### Correct — strictly newer than the version being replaced

```rust
conn.execute(&format!("UPDATE todos SET title = ?1, {SQL_SET_UPDATED_AT} WHERE id = ?2"), params![title, id])?;
```

---

## Design Decisions

### Design Decision: record-level LWW with tombstones, not whole-file "newest wins"

The old protocol compared the document's `updatedAt` with `last_sync_at` and replaced one side
wholesale, deleting records "absent" from the newer side. Any concurrent write (cloud AI + PC) lost
data (A1). Record-level LWW + tombstones + union makes each record converge independently; the
price is that deletions need tombstones (30-day retention bounds the growth) and that a device
offline for more than 30 days can resurrect records deleted elsewhere — accepted.

### Design Decision: only `If-Match` / `If-None-Match`

Measured (2026-10-02): Apache returns weak ETags for ~1 s after a write and 412s IUS for unchanged
files; PUT responses carry no validators on Apache or nginx; nginx ignores PUT preconditions, has no
`getetag` in PROPFIND and compares IMS in whole seconds. Date-based preconditions are therefore
wrong on one server or useless on the other. Opaque ETags work where supported, and the
"GET + merge before PUT" rule plus self-heal covers servers that ignore preconditions.

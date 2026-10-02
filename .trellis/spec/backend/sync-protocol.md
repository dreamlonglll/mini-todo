# Sync Protocol (PC ↔ WebDAV ↔ cloud)

> Code-spec for the shared `sync-data.json.gz` channel. Two independent writers implement it:
> PC (`pc/src-tauri/src/commands/sync_cmd.rs` + `db/sync_store.rs`) and cloud
> (`cloud/src/sync/*`). Every rule below must hold on **both** sides — a rule implemented on one side
> only silently loses or resurrects data on the other.
> Established in task `10-02-full-optimization` (2026-10-02); server behaviour measured against
> Apache 2.4 mod_dav and nginx 1.24 dav/dav_ext, then verified end-to-end with the real app
> (`pc/scripts/e2e-linux/`, suites S01–S10 run once per server).

---

## Scenario: Record-level sync through one WebDAV file

### 1. Scope / Trigger

- Applies to any change to: the sync document shape, record fields, timestamps written to
  `updated_at` / `deleted_at`, tombstones, settings that sync, WebDAV request headers, the 412 retry
  loop, or the "base version" bookkeeping.
- Also applies to every **local write path** that touches `todos` / `subtasks` (Tauri commands,
  import, cloud REST handlers): they feed the LWW comparison.

### 2. Signatures

```rust
// PC — commands/sync_cmd.rs (all async; blocking work in spawn_blocking; one sync at a time)
#[tauri::command] pub async fn webdav_sync(app: AppHandle) -> Result<SyncReport, String>;       // smart sync
#[tauri::command] pub async fn webdav_force_pull(app: AppHandle) -> Result<SyncReport, String>; // local := remote
#[tauri::command] pub async fn webdav_force_push(app: AppHandle) -> Result<SyncReport, String>; // remote := local
// re-entry while a sync runs -> Err("同步正在进行中")
// SyncReport (camelCase): status no_changes|pulled|pushed|merged, lastSyncAt,
//   todos/subtasks Inserted/Updated/Deleted, recordsSkipped, imagesUploaded/Downloaded, settingsApplied
// emits "sync-completed"(SyncReport) to all windows when local data or settings changed

// PC — db/time.rs
pub const CANONICAL_FORMAT: &str = "%Y-%m-%d %H:%M:%S";
pub fn normalize_datetime(s: &str, default: DefaultTime) -> Option<String>;
pub fn superseding(now: &str, previous: &str) -> String;     // max(now, previous + 1s)
pub const SQL_SET_UPDATED_AT: &str;                          // same rule inside UPDATE statements

// PC — db/sync_store.rs
pub const TOMBSTONE_RETENTION_DAYS: i64 = 30;
pub fn merge_remote(..) -> Result<MergeStats>;               // one transaction
pub(crate) fn deletion_time(now: &str, record_updated_at: &str) -> String; // max(now, record.updatedAt)

// cloud — src/time.rs
pub fn bump_updated_at(now: &str, previous: &[&str], tz: Tz) -> String;   // == superseding
pub fn deletion_time(now: &str, record_times: &[&str], tz: Tz) -> String; // == PC deletion_time
```

Persisted PC settings keys (device-local, never synced): `webdav_remote_etag`, `webdav_last_modified`
(recorded only), `webdav_synced_settings_at`, `webdav_last_sync_at`, `webdav_device_id`. Change
counters live in `sync_meta(local_seq, synced_seq)`, maintained by triggers on `todos` / `subtasks`.
cloud keeps the equivalent in `meta` (`dirty`, `base_etag`, `base_last_modified`, `remote_envelope`).

### 3. Contracts

#### 3.1 Document (`/mini-todo/sync-data.json.gz`, gzip JSON)

```jsonc
{
  "version": "4.0", "deviceId": "…", "updatedAt": "<ISO with offset, metadata only>",
  "todos": [ { /* Todo, camelCase, nested "subtasks": [...] */ } ],
  "settings": { /* AppSettings */ }, "settingsUpdatedAt": "YYYY-MM-DD HH:MM:SS",   // optional
  "images": ["<safe file name>"],
  "tombstones": [ { "entityType": "todo" | "subtask", "entityId": 123, "deletedAt": "YYYY-MM-DD HH:MM:SS" } ]
  // unknown top-level keys: every writer carries them through unchanged
}
```

- `entityId` is an integer (PC ids are `i64`; cloud-created ids are `millis * 1000 + rand(0..999)`,
  always < 2^53 so the JS frontend keeps them exact).
- Tombstones older than 30 days are dropped on write and purged locally.

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

#### 3.3 Merge (identical on PC `merge_remote` and cloud `sync/merge.rs`)

1. Record LWW on normalized `updatedAt`; **ties keep the local version**.
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

#### 3.4 WebDAV requests (measured — see Design Decisions)

| Step | Rule |
|---|---|
| Read | `If-None-Match: <base ETag>` only when nothing local is pending and a base ETag exists; otherwise unconditional GET. **Never `If-Modified-Since`.** |
| Write | Always GET + merge first. PUT with `If-Match: "<opaque>"` (strip `W/`) when a base ETag exists, else no precondition. **Never `If-Unmodified-Since`.** |
| 412 | Wait ≥ 1.1 s, then unconditional GET → merge → PUT. At most 4 attempts (PC) / 3 attempts (cloud) per sync. |
| 404/409 on PUT | MKCOL the parent chain once, retry. No MKCOL on every sync. |
| New base | ETag from the PUT response → HEAD → PROPFIND Depth 0 `getetag`. If the probed size ≠ uploaded size, record no base (next sync does a full GET). Never record a base for content that was not merged. |
| Images | Upload missing images **before** the document; list remote images with one PROPFIND Depth 1 (any namespace prefix, single- or multi-line XML); names must pass `is_safe_image_name` (`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`, no `..`). |
| Self-heal | After merging, if the remote document lacks local records/tombstones (a server that ignores preconditions let someone overwrite us), upload again even when nothing changed locally. |
| Full pull (cloud) | Every 10th pull round is unconditional: nginx ETags have 1-second resolution and hide same-second, same-size rewrites behind 304. |

#### 3.5 Force operations (PC)

- `webdav_force_pull`: local := remote; deletes local-only records **without** tombstones; applies
  remote settings (device keys excluded) regardless of `settingsUpdatedAt`. Missing remote → `Err`.
- `webdav_force_push`: remote := local; writes tombstones for remote-only records (otherwise cloud's
  next push merges them straight back), bumps local versions with the monotonic rule, conditional PUT.

### 4. Validation & Error Matrix

| Condition | PC | cloud |
|---|---|---|
| Sync already running | `Err("同步正在进行中")` | waits on the async sync mutex (`SyncCtx::run_locked`) |
| Remote document missing (404) | smart sync uploads; force pull errors | push creates it |
| Remote not gzip / not JSON / not an object | error, nothing uploaded | pull error, cache untouched, never uploads over it |
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

- PC `cargo test` (`pc/src-tauri`): `sync_store` merge/tombstone/settings cases, `db::time`
  parsing + `superseding`, and the `sync_cmd` interop tests that run the full sync against a
  `tiny_http` mock server in Apache and nginx modes (weak ETag window, IUS 412, missing validators).
- cloud `cargo test`: `sync/merge.rs` unit tests, `sync/scenario_tests.rs` on `mock_dav`
  (`MockDav::start_apache` / `start_nginx`), API integration tests incl. "no-op PATCH does not bump
  `updatedAt`".
- e2e (`pc/scripts/e2e-linux/run_e2e.py`, S01–S10 per server): first sync / no change, A1 same-day
  merge, deletion both ways without resurrection, concurrent edits, cloud validation and aliases,
  images, settings LWW + unknown top-level key preserved, force pull/push, PC + cloud concurrent
  writes, UI sync button.

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

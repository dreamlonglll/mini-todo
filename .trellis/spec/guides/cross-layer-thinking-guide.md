# Cross-Layer Thinking Guide

> **Purpose**: Think through data flow across layers before implementing.

---

## The Problem

**Most bugs happen at layer boundaries**, not within layers.

Common cross-layer bugs:
- API returns format A, frontend expects format B
- Database stores X, service transforms to Y, but loses data
- Multiple layers implement the same logic differently

---

## Before Implementing Cross-Layer Features

### Step 1: Map the Data Flow

Draw out how data moves:

```
Source → Transform → Store → Retrieve → Transform → Display
```

For each arrow, ask:
- What format is the data in?
- What could go wrong?
- Who is responsible for validation?

### Step 2: Identify Boundaries

| Boundary | Common Issues |
|----------|---------------|
| API ↔ Service | Type mismatches, missing fields |
| Service ↔ Database | Format conversions, null handling |
| Backend ↔ Frontend | Serialization, date formats |
| Component ↔ Component | Props shape changes |

### Step 3: Define Contracts

For each boundary:
- What is the exact input format?
- What is the exact output format?
- What errors can occur?

---

## Common Cross-Layer Mistakes

### Mistake 1: Implicit Format Assumptions

**Bad**: Assuming date format without checking

**Good**: Explicit format conversion at boundaries

### Mistake 2: Scattered Validation

**Bad**: Validating the same thing in multiple layers

**Good**: Validate once at the entry point

### Mistake 3: Leaky Abstractions

**Bad**: Component knows about database schema

**Good**: Each layer only knows its neighbors

---

## Checklist for Cross-Layer Features

Before implementation:
- [ ] Mapped the complete data flow
- [ ] Identified all layer boundaries
- [ ] Defined format at each boundary
- [ ] Decided where validation happens

After implementation:
- [ ] Tested with edge cases (null, empty, invalid)
- [ ] Verified error handling at each boundary
- [ ] Checked data survives round-trip

---

## When to Create Flow Documentation

Create detailed flow docs when:
- Feature spans 3+ layers
- Multiple teams are involved
- Data format is complex
- Feature has caused bugs before

---

## Removing a Feature End-to-End

Mirror image of the "adding a field" checklist in `CLAUDE.md`. When deleting a feature that touches multiple layers (DB schema + Rust models + Tauri commands + frontend types + UI), walk every layer or you leave dangling references.

### Layer-by-layer removal checklist

When removing a feature whose data is persisted and synchronized:

- [ ] **DB migration**: write a new HEAD migration that `DROP TABLE` for owned tables and `ALTER TABLE … DROP COLUMN` for fields on retained tables
- [ ] **Drop indices first**: SQLite refuses to drop a column that is part of an index; emit `DROP INDEX IF EXISTS` before `DROP COLUMN`
- [ ] **Rust models** (`pc/src-tauri/src/db/models.rs`): delete struct fields and any deleted-table types; keep the Todo/SubTask structs lean
- [ ] **Tauri commands**: delete the command files entirely AND delete their registration in `lib.rs` `tauri::generate_handler!` (forgetting the registration produces "command not found" only at runtime)
- [ ] **Frontend types** (`pc/src/types/`): delete the matching TS interface fields; delete dedicated type files; remove their re-exports in `pc/src/types/index.ts` and `pc/src/stores/index.ts`
- [ ] **Pinia stores**: delete the store files; verify no other stores import from them
- [ ] **Vue components/views**: delete dedicated files; in mixed files, delete in place (don't refactor) — see "In-place deletion" below
- [ ] **Router**: drop dead routes from `pc/src/router/index.ts`
- [ ] **WebDAV/Export DTOs**: shrink `ExportData` and `SyncData`; rely on serde leniency (see "Backward-compatible deserialization" below)
- [ ] **Cargo.toml**: prune deps that the deleted code was the sole consumer of (e.g. `cron`, `async-trait` if only AgentRunner used it)
- [ ] **CLAUDE.md / docs**: remove "主要数据表" rows, command lists, architecture diagrams, and event names referring to the deleted feature
- [ ] **Spec files** (`.trellis/spec/`): grep the spec dir for store names, view names, type names you deleted — purge stale references the same commit
- [ ] **Version bump**: app version (3 places: `pc/package.json`, `pc/src-tauri/Cargo.toml`, `pc/src-tauri/tauri.conf.json`) and export version (`pc/src-tauri/src/commands/data.rs`)

### Backward-compatible deserialization for export/sync DTOs

**Problem**: We removed fields from `ExportData` / `SyncData` / `Todo` / `SubTask`. Old v3.0 backup JSON and old WebDAV remote data still contain those fields. We don't want to write explicit version branches.

**Solution**: Exploit serde's defaults. We do **not** annotate any structs with `#[serde(deny_unknown_fields)]`, so unknown JSON keys silently pass through during deserialization. For previously-required fields that we're now dropping but might appear nested (e.g. inside an old `SyncData`), we add `#[serde(default)]` so a missing field deserializes to the type's `Default`.

**What this looks like**:

```rust
// In models.rs — no #[serde(deny_unknown_fields)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportData {
    pub version: String,
    pub todos: Vec<Todo>,
    pub subtasks: Vec<SubTask>,
    pub settings: AppSettings,
    // Old agent_configs / workflow_steps / prompt_templates fields silently ignored
}
```

**Why it matters**: Without this, importing a v3.0 backup or syncing from an unmigrated peer would fail with "unknown field" errors. Document this contract with a comment near the struct so future contributors don't add `deny_unknown_fields`.

### In-place deletion (no refactor) for large files

When deleting a feature from a large mixed file (`EditorView.vue` 2086 lines, `SubtaskEditorView.vue` 892 lines), **delete in place** — don't reorder, rename, or extract helpers as part of the same change.

**Why**:
- Keeps the diff readable so reviewers can verify "only deletions, no behavior change to retained features"
- Reduces risk of breaking watchers / nextTick chains / emit ordering
- Makes git blame still useful for retained code

**What "in place" means**:
- Delete the import, the ref, the computed, the watcher, the function, the template branch — all in their original positions
- Clean up newly-unused imports / refs that the deletion makes dead — that's not "refactoring"
- Don't merge two `<script setup>` sections, don't re-sort props, don't rename anything

If after deletion the file's structure feels wrong, file a follow-up task for refactoring; don't blend it with the removal.

---

## Two-Way Sync Across SQLite Replicas via HTTP Blob

Pattern: two writers (e.g. PC desktop + cloud server, or two PC peers) each hold a local SQLite database, and a shared compressed JSON blob on WebDAV/S3/etc. acts as source of truth. Each writer periodically pulls + merges and pushes after local writes. The catch is **concurrent writes** — naive full-replacement PUT will silently lose data.

This pattern is in `pc/src-tauri/src/commands/sync_cmd.rs` (PC) and `cloud/src/sync/{pull,push}.rs` (cloud). The same building blocks apply to any "shared remote blob + per-side local SQLite cache" setup. Below are the four cross-layer traps that **must** be aligned for the pattern to be correct.

### Building blocks

1. **Per-record `updated_at` for LWW merge** — every row has an `updated_at TEXT` column; merge rule = "side with newer `updated_at` wins per record".
2. **Conditional PUT with `If-Unmodified-Since`** — push uses `If-Unmodified-Since: <last-known-Last-Modified>`. Remote 412 means another writer pushed first; download + merge + retry (max ~3).
3. **Tombstone table** — DELETEs alone don't propagate via LWW (the side that still has the record "wins"). Keep a `tombstones(type, id, deleted_at)` table; merges drop incoming records that match a local tombstone. Expire (e.g. 7 days) after a successful push.
4. **Preserve remote id on INSERT during per-record merge** — see "Identity preservation" below.

### Trap 1: Time format alignment

LWW compares `updated_at` strings. **Every writer must produce strings in the same lexicographically-comparable format**, or LWW silently picks the wrong winner.

Mini-todo's PC SQLite uses `datetime('now', 'localtime')` → `"2026-05-13 12:34:56"` (no timezone suffix). The cloud Rust service has no OS-level "localtime", so it explicitly mimics via `chrono::FixedOffset` derived from `config.timezone`:

```rust
// cloud/src/time.rs
pub fn now_local_string(offset: FixedOffset) -> String {
    Utc::now()
        .with_timezone(&offset)
        .format("%Y-%m-%d %H:%M:%S")  // matches PC datetime('now','localtime') byte-for-byte
        .to_string()
}
```

If one writer adds a timezone suffix (`"2026-05-13T12:34:56+08:00"`) while another doesn't, `"…+08:00" > "… 12:34:56"` is false → LWW chooses wrongly.

> **Outer envelope vs per-record timestamps**: a sync blob may have its own "this snapshot was created at X" metadata field, which can safely use ISO 8601 with tz (informational, not compared). The constraint applies only to per-record `updated_at` columns participating in merge.

### Trap 2: `If-Unmodified-Since` is not enough on its own

The conditional PUT **does not** prevent two writers that both pulled the same `Last-Modified` from racing — both pass the precondition, both PUT, the second PUT wins and silently overwrites the first. The 412 only catches the case where one writer's view of the remote is provably stale.

The fix is per-record LWW merge on the 412 path, not just retry. **Retry without merge resends the same lossy snapshot.**

### Trap 3: Identity preservation during per-record merge

When the receiver inserts a record from the sender, it **must** preserve the sender's primary key. Otherwise SQLite AUTOINCREMENT assigns a new id, and the next sync round creates a different record on the sender — id drift, looping forever.

```rust
// pc/src-tauri/src/commands/sync_cmd.rs::merge_remote_into_local
conn.execute(
    "INSERT INTO todos (id, title, ...) VALUES (?1, ?2, ...)",  // id is explicit
    params![remote_todo.id, remote_todo.title, ...],
)?;
```

> **Contrast with full-replacement import**: `webdav_apply_remote` (the non-merge path) drops the table and lets SQLite reassign ids on re-insert. That's fine because the next push then sends those new ids back as authoritative. But the **per-record merge path** (412 retry) must NOT reassign — remote id is authoritative.

### Trap 4: Tombstones for delete propagation

LWW with no tombstone: A deletes record X, B still has X with a newer `updated_at` → next merge, A "loses" and re-creates X. The delete silently fails to propagate.

```rust
// cloud/src/sync/push.rs::merge_sync_data — tombstone consulted before LWW
if todo_tombs.contains(id) {
    continue;  // local deleted this; do not resurrect from remote
}
```

Tombstone schema: `(type TEXT, id TEXT, deleted_at TEXT, PRIMARY KEY (type, id))`. After a successful PUT, sweep tombstones older than N days (mini-todo: 7) so the table doesn't grow forever.

### Per-record conflict matrix

| Local state | Remote state | Action |
|---|---|---|
| present, `local.updated_at >= remote.updated_at` | present | keep local |
| present, `local.updated_at < remote.updated_at` | present | overwrite with remote (all columns, preserve remote `updated_at`) |
| absent | present | INSERT remote (preserve remote id) |
| present (no tombstone) | absent | keep local (treated as locally new) |
| tombstone | present | drop remote, keep tombstone |

### Wrong vs Correct

**Wrong**: full-replacement PUT (works fine until two writers, then silent overwrites)

```rust
let body = export_full_data();
client.put("/sync-data.json.gz", body);  // no precondition, no merge
```

**Correct**: conditional PUT + 412 → merge → retry

```rust
let mut retry = 0;
loop {
    let last_modified = get_setting("webdav_last_modified").filter(|s| !s.is_empty());
    let body = export_full_data();
    match client.upload_bytes(..., body, last_modified.as_deref())? {
        UploadOutcome::Ok(new_last_modified) => {
            if let Some(lm) = new_last_modified { set_setting("webdav_last_modified", lm); }
            break Ok(());
        }
        UploadOutcome::PreconditionFailed => {
            if retry >= MAX_RETRY { return Err("too many conflicts".into()); }
            retry += 1;
            let (remote_bytes, remote_lm) = client.download_bytes(...)?;
            if let Some(lm) = remote_lm { set_setting("webdav_last_modified", lm); }
            let remote: SyncData = decompress(remote_bytes)?;
            merge_remote_into_local(db, &remote)?;  // per-record LWW + tombstone
            // loop: re-export with merged state, retry PUT
        }
    }
}
```

### Tests required (assertion points)

- `merge_keeps_newer`: remote.updated_at > local → local row overwritten
- `merge_keeps_local_when_newer`: local.updated_at > remote → local row untouched
- `merge_tombstone_suppresses_remote`: local tombstone present → remote record dropped, not resurrected
- `merge_inserts_remote_with_explicit_id`: remote-only record → INSERT with remote id, not AUTOINCREMENT
- `merge_runs_in_single_transaction`: any mid-merge SQL error → entire batch rolls back
- `time_format_byte_for_byte_match`: cloud `now_local_string` output matches PC `datetime('now','localtime')` shape exactly (`YYYY-MM-DD HH:MM:SS`, no `T`, no tz suffix)

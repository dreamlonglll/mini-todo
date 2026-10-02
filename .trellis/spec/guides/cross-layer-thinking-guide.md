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

Pattern: two writers (PC desktop + cloud server, or two PCs) each hold a local SQLite database, and a
shared compressed JSON blob on WebDAV acts as source of truth. Each writer pulls + merges
periodically and pushes after local writes. The catch is **concurrent writes** and **servers that
implement HTTP preconditions differently** — naive full-replacement PUT silently loses data.

Implemented in `pc/src-tauri/src/commands/sync_cmd.rs` + `db/sync_store.rs` (PC) and
`cloud/src/sync/*` (cloud). The exact contract (document shape, merge rules, WebDAV header rules,
error matrix, required tests) is **[Sync Protocol](../backend/sync-protocol.md)** — read it before
touching either side. This section explains the traps that made those rules necessary.

### Building blocks

1. **Per-record LWW on `updated_at`**, ties keep the local version, and **strictly increasing
   versions** for local writes: `updated_at = max(now, previous + 1s)`.
2. **Read-merge-write**: every push GETs the remote first, merges, then PUTs with `If-Match` on an
   opaque ETag (or no precondition when none is known). 412 → wait ≥ 1.1 s → GET → merge → PUT.
3. **Tombstones inside the blob** (`tombstones: [{entityType, entityId, deletedAt}]`), union-merged,
   30-day retention, `deletedAt = max(now, record.updatedAt)`.
4. **Union, never "absent means deleted"** — only tombstones delete.
5. **Ids preserved on insert** in both directions.
6. **Self-heal** — after merging, re-upload if the remote lacks local records or tombstones.

### Trap 1: Time format alignment

LWW compares `updated_at` strings, so **every writer must produce the same lexicographically
comparable format**: `YYYY-MM-DD HH:MM:SS`, local wall clock, no zone (PC SQLite
`datetime('now','localtime')`). The cloud has no OS "localtime" and converts from the configured IANA
zone **on every call** — an offset cached at startup is wrong by an hour after a DST switch:

```rust
// cloud/src/time.rs
pub fn now_local_string(tz: chrono_tz::Tz) -> String {
    Utc::now().with_timezone(&tz).format("%Y-%m-%d %H:%M:%S").to_string()
}
```

Readers must still accept `T`, missing seconds, fractions, `Z` / `±HH:MM` and date-only input and
normalize before comparing (`db::time::normalize_datetime`, cloud `time::normalize_datetime`,
frontend `utils/datetime.ts`). A blob-level "snapshot created at" field may use ISO with offset — it
is metadata and never compared.

### Trap 2: Second-resolution timestamps + "ties keep local" lose same-second edits

Edit a record twice within one second: the other writer pulls version 1, version 2 carries the same
timestamp, and "tie keeps local" makes the other writer keep version 1 forever (found by e2e S05 on
nginx). The same happens when the version being replaced came from a device whose clock runs
ahead. Fix: every local write sets `updated_at = max(now, previous + 1s)` (PC `db::time::superseding`
/ `SQL_SET_UPDATED_AT`, cloud `time::bump_updated_at`). cloud additionally skips the bump (and the
push) for a PATCH that changes nothing.

### Trap 3: WebDAV servers disagree on preconditions — date-based ones are unusable

Measured on Apache mod_dav and nginx dav (details in the spec): Apache answers
`If-Unmodified-Since` with 412 even for an untouched file (sub-second mtime vs second-resolution
date) and serves only weak ETags for ~1 s after a write; nginx ignores PUT preconditions entirely,
has no `getetag` in PROPFIND and compares `If-Modified-Since` in whole seconds; neither returns
validators on PUT. Consequences:

- Only `If-Match` / `If-None-Match` with opaque ETags (strip `W/`); never IUS / IMS.
- Correctness may not depend on the precondition: GET + merge right before every PUT, plus self-heal.
- After a PUT, learn the new ETag via PUT response → HEAD → PROPFIND; if the probed size does not
  match what you uploaded, record **no** base rather than a wrong one.
- Never record as "base" a remote version whose content you did not merge — a later `If-None-Match`
  would then answer 304 forever and the content is never merged (old cloud bug A4).
- nginx ETags have one-second resolution: cloud does an unconditional pull every 10th round.

### Trap 4: Identity preservation during per-record merge

When a writer inserts a record from the other side it **must** keep the sender's primary key.
Otherwise AUTOINCREMENT assigns a new id, the next round sends that back as a different record — id
drift, looping forever. Ids are therefore explicit on every insert path (`sync_store::insert_todo_row`
/ `insert_subtask_row`, cloud repo inserts), including import and force pull. Cloud-created ids are
`millis * 1000 + rand(0..999)`, kept below 2^53 so the JS frontend can represent them exactly.

### Trap 5: Tombstones must travel with the data

LWW without tombstones: A deletes X, B still has X → next merge resurrects X. Tombstones kept only
in A's local table are not enough either: B's next push re-adds X to the blob, and A's next pull
inserts it again. So tombstones live in the blob, both sides union them, and a "make remote equal
local" operation (PC `webdav_force_push`) must write tombstones for every remote-only record —
otherwise the other writer merges them straight back.

### Per-record conflict matrix

| Local | Remote | Action |
|---|---|---|
| present, `local.updatedAt >= remote.updatedAt` | present | keep local |
| present, `local.updatedAt < remote.updatedAt` | present | overwrite with remote (all fields, keep remote `updatedAt`) |
| absent | present, no covering tombstone | INSERT remote with its id |
| present | absent | keep local (union) |
| any | tombstone with `deletedAt >= record.updatedAt` (either side) | delete / suppress; a todo tombstone takes its subtasks |
| tombstone | record with `updatedAt > deletedAt` | keep the record (edited after deletion) |

### Wrong vs Correct

**Wrong**: date-based conditional PUT with retry (412s forever on Apache, races silently on nginx)

```rust
match client.upload_bytes(path, body, Some(&last_modified))? {   // If-Unmodified-Since
    UploadOutcome::PreconditionFailed => { download_and_merge()?; /* retry */ }
    UploadOutcome::Ok(lm) => set_setting("webdav_last_modified", lm),
}
```

**Correct**: read-merge-write, opaque ETag precondition, wait out the weak-ETag window on 412

```rust
// sketch of sync_cmd.rs::run_sync_blocking (cloud push.rs has the same shape)
for attempt in 0..MAX_ATTEMPTS {
    let remote = client.get(SYNC_DATA_FILE, if_none_match)?;      // unconditional when dirty
    merge_remote(&tx, &remote)?;                                     // LWW + tombstones + settings
    let doc = build_sync_doc(&conn)?;                                // includes tombstones
    match client.put(SYNC_DATA_FILE, gzip(doc)?, &choose_put_precondition(&base))? {
        PutOutcome::Ok(meta) => { remember_base(meta.or_else(|| head_or_propfind())); return Ok(()) }
        PutOutcome::PreconditionFailed => sleep(CONFLICT_RETRY_DELAY),  // >= 1.1 s
    }
}
```

### Tests required

See [Sync Protocol §6](../backend/sync-protocol.md#6-tests-required): merge unit tests on both sides,
mock-server tests that emulate Apache and nginx behaviour, and the e2e S-suites against real
Apache and nginx.

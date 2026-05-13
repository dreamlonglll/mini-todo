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
- [ ] **Rust models** (`src-tauri/src/db/models.rs`): delete struct fields and any deleted-table types; keep the Todo/SubTask structs lean
- [ ] **Tauri commands**: delete the command files entirely AND delete their registration in `lib.rs` `tauri::generate_handler!` (forgetting the registration produces "command not found" only at runtime)
- [ ] **Frontend types** (`src/types/`): delete the matching TS interface fields; delete dedicated type files; remove their re-exports in `src/types/index.ts` and `src/stores/index.ts`
- [ ] **Pinia stores**: delete the store files; verify no other stores import from them
- [ ] **Vue components/views**: delete dedicated files; in mixed files, delete in place (don't refactor) — see "In-place deletion" below
- [ ] **Router**: drop dead routes from `src/router/index.ts`
- [ ] **WebDAV/Export DTOs**: shrink `ExportData` and `SyncData`; rely on serde leniency (see "Backward-compatible deserialization" below)
- [ ] **Cargo.toml**: prune deps that the deleted code was the sole consumer of (e.g. `cron`, `async-trait` if only AgentRunner used it)
- [ ] **CLAUDE.md / docs**: remove "主要数据表" rows, command lists, architecture diagrams, and event names referring to the deleted feature
- [ ] **Spec files** (`.trellis/spec/`): grep the spec dir for store names, view names, type names you deleted — purge stale references the same commit
- [ ] **Version bump**: app version (3 places: `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json`) and export version (`src-tauri/src/commands/data.rs`)

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

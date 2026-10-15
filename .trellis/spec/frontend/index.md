# Frontend Development Guidelines

> Best practices for frontend development in this project.

---

## Overview

This directory contains guidelines for frontend development. Fill in each file with your project's specific conventions.

> Scope: the Vue 3 frontend of the Tauri app (`pc/src/`). The GPUI rewrite (`pc-gpui/crates/app`) has no WebView / Vue;
> its UI conventions live in [`../pc-gpui/`](../pc-gpui/index.md) (`gpui-conventions.md`). Two rules from
> [State Management](./state-management.md) still hold there semantically: single-flight `fetchTodos` with stale-result
> drop, and skipping background refreshes while a modal child window is open (`AppState::fetch_todos` /
> `refresh_if_changed`, `WindowManager::main_has_modal_child`). Pure helpers in `pc/src/utils/*.ts` have 1:1 Rust ports —
> fix logic bugs in both (mapping in [`../pc-gpui/core-parity.md`](../pc-gpui/core-parity.md) §2.3).

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | To fill |
| [Component Guidelines](./component-guidelines.md) | Component patterns, props, composition | To fill |
| [Hook Guidelines](./hook-guidelines.md) | Custom hooks, data fetching patterns | To fill |
| [State Management](./state-management.md) | Pinia stores, data refresh triggers, modal-skip convention | Active |
| [Quality Guidelines](./quality-guidelines.md) | Code standards, forbidden patterns | Active |
| [Type Safety](./type-safety.md) | Type patterns, validation | To fill |

---

## How to Fill These Guidelines

For each guideline file:

1. Document your project's **actual conventions** (not ideals)
2. Include **code examples** from your codebase
3. List **forbidden patterns** and why
4. Add **common mistakes** your team has made

The goal is to help AI assistants and new team members understand how YOUR project works.

---

**Language**: All documentation should be written in **English**.

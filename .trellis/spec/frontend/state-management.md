# State Management

> How state is managed in this project.

---

## Overview

- **Solution**: Pinia (Vue 3 official state management)
- **Pattern**: Composition API style (`defineStore` with `setup()` syntax)
- **Stores**: `todoStore` (todos/subtasks CRUD), `appStore` (window/theme/settings), `agentStore`, `schedulerStore`

---

## State Categories

| Category | Store | Example |
|----------|-------|---------|
| Domain data | `todoStore` | `todos`, `subtasks` |
| App-level UI | `appStore` | `isDarkTheme`, `isFixed`, `showCalendar` |
| Feature state | `agentStore`, `schedulerStore` | Agent configs, scheduler status |
| Local UI state | Component `ref()` | `isModalOpen`, `isSyncing` |

---

## Data Refresh Triggers

`MainView.vue` is the central refresh orchestrator. All `fetchTodos()` calls flow through it.

| Trigger | Mechanism | Notes |
|---------|-----------|-------|
| App startup | `onMounted` → `fetchTodos()` | Initial load |
| Window focus | `appWindow.onFocusChanged` → `fetchTodos()` | Catches external DB modifications (e.g., scripts writing to SQLite) |
| Polling (60s) | `setInterval` → `fetchTodos()` | Background refresh when window stays in foreground |
| Modal close | `tauri://destroyed` event → `fetchTodos()` | Editor/settings/completed window close |
| Sync complete | `sync-completed` event → `fetchTodos()` | After WebDAV sync applies remote data |
| Auto sync | `webdav_auto_sync` result check → `fetchTodos()` | Only when remote data was applied |
| Settings event | `todo-font-changed` event → `loadTodoFontSettings()` | Real-time cross-window style sync |

### Convention: Skip Refresh During Modal

All background refresh paths (focus, polling) check `isModalOpen` before calling `fetchTodos()`. This prevents list mutations while the user is editing in a child window.

### Don't: Add fetchTodos() in Child Components

Child components (`TodoList`, `TodoItem`, `QuadrantView`) should emit events or call store mutation methods that update local state. Full re-fetches from DB should only happen in `MainView.vue` to avoid redundant queries and race conditions.

---

### Cross-Window Settings Update Pattern

When a setting in the child window (e.g., SettingsView) must take immediate effect on the main window:

1. Child window saves to DB via `invoke()`
2. Child window emits a Tauri event: `await emit('todo-font-changed')`
3. Main window listens for the event in `onMounted` and reloads the setting
4. Main window cleans up the listener in `onUnmounted`

This avoids polling and gives instant feedback. The "modal close" refresh path is still a fallback.

---

## Common Mistakes

### Common Mistake: Forgetting to Refresh After New Data Path

**Symptom**: User performs an action but the list doesn't update.

**Cause**: A new Tauri command or event writes to the database but the corresponding `fetchTodos()` call is missing in `MainView.vue`.

**Prevention**: When adding a new data mutation path (new Tauri command, new event listener, new sync flow), always add a corresponding refresh trigger in `MainView.vue`'s `onMounted` setup.

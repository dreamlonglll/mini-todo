# State Management

> How state is managed in this project.

---

## Overview

- **Solution**: Pinia (Vue 3 official state management)
- **Pattern**: Composition API style (`defineStore` with `setup()` syntax)
- **Stores**: `todoStore` (todos/subtasks CRUD), `appStore` (window/theme/settings)

---

## State Categories

| Category | Store | Example |
|----------|-------|---------|
| Domain data | `todoStore` | `todos`, `subtasks` |
| App-level UI | `appStore` | `isDarkTheme`, `isFixed`, `showCalendar` |
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
| Settings event | `app-settings-changed` event → reload by `key` | Calendar / auto-hide / theme / auto-sync timer / update badge |
| Data imported | `data-imported` event → `fetchTodos()` + `reloadAppSettings()` | Import and remote-apply overwrite the `settings` table too, not just todos |

### Convention: Skip Refresh During Modal

All background refresh paths (focus, polling) check `isModalOpen` before calling `fetchTodos()`. This prevents list mutations while the user is editing in a child window.

### Don't: Add fetchTodos() in Child Components

Child components (`TodoList`, `TodoItem`, `QuadrantView`) should emit events or call store mutation methods that update local state. Full re-fetches from DB should only happen in `MainView.vue` to avoid redundant queries and race conditions.

---

### Cross-Window Settings Update Pattern

**Every child window (SettingsView, EditorView, ...) runs in its own WebView and therefore holds
its own Pinia store instance.** Writing to `appStore` inside SettingsView changes only that
window's copy — the main window sees nothing until it reloads from the database itself.

When a setting in the child window must take immediate effect on the main window:

1. Child window saves to DB via `invoke()`
2. Child window emits a Tauri event with a typed key:
   `await emit('app-settings-changed', { key })` — `key` is `AppSettingKey` in `types/app.ts`
3. Main window listens in `onMounted` and reloads that setting from the DB
4. Main window cleans up the listener in `onUnmounted`

Adding a new setting means touching three places: the `AppSettingKey` union, the emit site in
SettingsView, and the handler branch in MainView. The union makes a typo a compile error (TS2678).

The "modal close" refresh path (`MainView.reloadAppSettings()`) is still a fallback — add the new
`load*()` there too.

Both sides of the pattern matter — a child window must also **load** the real value on mount.
A `computed(() => appStore.isDarkTheme)` in SettingsView reads that window's default `false`
unless `loadDarkTheme()` runs there first.

#### Convention: a setting that changes the main window's live mode is applied by the backend command

**What**: When a settings-window switch must change the main window *right now* (e.g.
`fixed_embed_desktop` flipping a fixed window between classic and embedded), the narrow backend
command (`set_fixed_embed_desktop`) writes the key **and** performs the window change by resolving
`app_handle.get_webview_window("main")`. The `app-settings-changed` handler in MainView only
`load*()`s the value; it never calls `applyFixedMode()` / `applyNormalMode()` in response.

**Why**: The backend already queued Win32 work on the main thread; a second apply from the event
handler races it (see `.trellis/spec/backend/window-modes.md` §3 "Ordering rules"). The main
window still needs the fresh value for `body.fixed-mode` and for the next `saveWindowState()`.

#### Convention: load every key that `saveWindowState()` writes before the first save

**What**: `saveWindowState()` persists `is_fixed`, `top_on_wake`, `fixed_embed_desktop`, background
colour, … from the store's current refs. `initSettings()` therefore calls `loadTopOnWake()`,
`loadFixedEmbedDesktop()`, `loadWindowBackground()` **before** restoring the window and before any
toggle can save.

**Why**: A ref still at its default (`true` for `topOnWake`, `false` for `fixedEmbedDesktop`) would
silently overwrite the user's stored value on the first mode toggle after startup.

#### Convention: platform-only settings rows use a UA check in the view

**What**: `const isWindows = /windows/i.test(navigator.userAgent)` + `v-if="isWindows"` on the
row (same approach as `applyPlatformClass()` for macOS). The backend command still rejects the
operation on other platforms; the `v-if` is UX, not the safety net.

---

## Common Mistakes

### Common Mistake: Forgetting to Refresh After New Data Path

**Symptom**: User performs an action but the list doesn't update.

**Cause**: A new Tauri command or event writes to the database but the corresponding `fetchTodos()` call is missing in `MainView.vue`.

**Prevention**: When adding a new data mutation path (new Tauri command, new event listener, new sync flow), always add a corresponding refresh trigger in `MainView.vue`'s `onMounted` setup.

### Common Mistake: A Tauri command that resolves `Window` from the caller

**Symptom**: Toggling a setting inside the settings window silently moves/resizes the main window
on next launch, or clears its fixed mode.

**Cause**: A command declared as `pub fn cmd(window: Window)` receives the **calling** window.
`get_window_persist_state` did this, so `saveWindowState()` — invoked from the settings window —
persisted the settings window's geometry as the main window's saved state, plus `is_fixed: false`
from that window's fresh store copy.

**Prevention**:
- Commands that act on the main window must resolve it explicitly:
  `app_handle.get_webview_window("main")`, never `window: Window`.
  Applies to `get_window_persist_state` and `set_window_fixed_mode`.
- Don't let a setting-writer call `saveWindowState()` just to persist its own key. Give it a
  narrow command (e.g. `set_text_theme`) that writes only that key. `saveWindowState()` writes
  position, size **and** `is_fixed` together, so calling it from anywhere but the main window
  corrupts window state.

### Common Mistake: Applying a global CSS class from a non-main window

`body.dark-theme` overrides `--text-primary` to white (`main.scss`). Child windows have their own
hard-coded light backgrounds, so applying it there yields white-on-white text. `applyThemeClass()`
guards with `appWindow.label !== 'main'`.

# Quality Guidelines

> Code quality standards for frontend development.

---

## Overview

<!--
Document your project's quality standards here.

Questions to answer:
- What patterns are forbidden?
- What linting rules do you enforce?
- What are your testing requirements?
- What code review standards apply?
-->

(To be filled by the team)

---

## Forbidden Patterns

### Don't: Compose CSS font-family with a potentially empty variable

**Problem**:
```scss
// Don't do this — when --todo-font-family is empty, produces: font-family: , -apple-system, ...
font-family: var(--todo-font-family), -apple-system, "Segoe UI", sans-serif;
```

**Why it's bad**: If the CSS variable is set to an empty string, the browser renders `font-family: , -apple-system, ...` which is invalid CSS and may break font rendering.

**Instead**: Build the complete font stack in JS and set a single variable:
```typescript
// In store
const fallback = '-apple-system, "Segoe UI", "Microsoft YaHei", sans-serif'
const fontStack = userFont ? `"${userFont}", ${fallback}` : fallback
root.style.setProperty('--todo-font-stack', fontStack)
```
```scss
// In SCSS — use the full stack as the variable value, with fallback inside var()
font-family: var(--todo-font-stack, -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif);
```

### Don't: Attach a new global body class without checking index.html for dead styles

**Problem**: `index.html` had a leftover `body.fixed-mode { background: transparent !important }` block that no JS ever activated. When MainView later started toggling `body.fixed-mode` for border styling, the dead block would have been resurrected, turning the light-theme fixed window fully transparent.

**Why it's bad**: Inline `<style>` in `index.html` is only a first-frame anti-flash fallback — but it still participates in the cascade at equal specificity, and `!important` rules there silently win.

**Instead**: Before introducing any new `body`-level class, grep `index.html` (and all global styles) for that class name and delete dead selectors first.

### Don't: Rely on tray CheckMenuItem auto-toggle for state with a second entry point

**Problem**: muda's `CheckMenuItem` flips its checked display on every click, regardless of whether the underlying operation succeeded, and knows nothing about state changed from elsewhere (settings panel, commands).

**Why it's bad**: With two entry points the display and the real state (e.g. autostart registry) invert: UI shows enabled while the registry entry is gone.

**Instead**: Keep the item handle in a `OnceLock` (see `TRAY_TOGGLE_FIXED` / `TRAY_AUTO_START` in `commands/window.rs`) and explicitly `set_checked(actual_state)` after every state change, reading the actual state back rather than assuming the toggle worked.

### Don't: Put follow-up UI sync calls inside the primary operation's try block

**Problem**:
```typescript
try {
  await enable()                                   // primary op — succeeded
  autoStart.value = value
  await invoke('sync_auto_start_state', ...)        // UI sync — may fail
} catch (e) {
  autoStart.value = !value                          // rolls back a SUCCESSFUL primary op
  ElMessage.error('设置开机自启失败')
}
```

**Why it's bad**: A failure in the cosmetic follow-up reports the already-successful primary operation as failed and rolls back UI state, re-creating the exact state-inversion bug the sync was meant to fix.

**Instead**: Fire the follow-up with an independent `.catch` that only logs:
```typescript
invoke('sync_auto_start_state', { enabled: value }).catch((e) => {
  console.error('Failed to sync tray autostart state:', e)
})
```

### Don't: Add conditional child without updating container v-if

**Problem**:
```vue
<!-- Container only checks subtasks and notifyTime -->
<div v-if="subtaskStats.total > 0 || formattedNotifyTime" class="todo-meta">
  <!-- ...existing children... -->
  <!-- New child added, but container v-if doesn't include its condition -->
  <span v-if="isRepeat"><el-icon><RefreshRight /></el-icon></span>
</div>
```

**Why it's bad**: The new child never renders when it's the only truthy condition, because the container itself is hidden.

**Instead**:
```vue
<!-- Update container condition to include ALL child visibility conditions -->
<div v-if="subtaskStats.total > 0 || formattedNotifyTime || isRepeat" class="todo-meta">
```

---

## Required Patterns

| Concern | Use | Never |
|---|---|---|
| Parsing / formatting todo times | `utils/datetime.ts` (`parseDateTime`, `composeDateTime` / `normalizeDateTime` → storage form `YYYY-MM-DD HH:MM:SS`, `formatDateTime` for display) | `split('T')`, `new Date(str)` on stored strings, hand-built ISO strings |
| Images inside Markdown | store `minitodo-image://<name>`; `toDisplayMarkdown` when loading into the editor, `toStorageMarkdown` before saving (`utils/imageRef.ts`) | persisting `asset.localhost` URLs or absolute paths |
| Link clicks in rendered Markdown | `handleLinkClick` / `preventLinkAuxClick` from `utils/fileLink.ts` (protocol whitelist) | letting the WebView navigate, `window.open(href)` |
| Error toasts | `notifyError(e, '<操作>失败')` from `utils/notify.ts` | bare `ElMessage.error(String(e))` |
| `ElMessage` / `ElMessageBox` | import from `@/plugins/element` (eslint enforces) | `import { ElMessage } from 'element-plus'` |
| Editor-like windows | `bindEditorShortcuts` from `utils/editorShortcuts.ts` (Esc closes with dirty check, Ctrl/Cmd+Enter saves, IME-safe) + a re-entry guard on save | duplicate keydown handlers per view |
| Uploading images | `invoke('save_subtask_image', bytes, { headers: { 'x-image-ext': ext } })` | base64 JSON payloads |
| Refreshing the list | `todoStore.fetchTodos()` / `refreshIfChanged()` | direct `invoke('get_todos')` |

---

## Testing Requirements

- Pure logic lives in `src/utils/*.ts` and gets a sibling `*.test.ts` (vitest, `npm run test`).
  Existing suites: datetime, imageRef, fileLink, editorShortcuts, quadrant.
- `npm run typecheck`, `npm run lint`, `npm run test` and `npx vite build` must pass (CI runs all four).
- UI flows that cross the IPC boundary (sync, images, notifications, refresh) are covered by the
  Linux e2e suite in `pc/scripts/e2e-linux/` — extend it when adding such a flow.

---

## Code Review Checklist

- [ ] Times written to the backend come from `utils/datetime.ts` storage helpers
- [ ] No new `v-html` / raw HTML from user content; Markdown goes through `MarkdownEditor`
- [ ] New `<el-*>` components need no manual import (unplugin), but new function-style APIs are added to `plugins/element.ts` with their CSS
- [ ] `components.d.ts` regenerated and committed when components are added/removed
- [ ] Every `listen()` has its unlisten in `onUnmounted`; every `setInterval` is cleared
- [ ] Background refreshes respect `isModalOpen`
- [ ] No emoji icons; Element Plus Icons imported per SFC

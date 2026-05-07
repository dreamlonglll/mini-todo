# Component Guidelines

> How components are built in this project.

---

## Overview

- Vue 3 Composition API (`<script setup lang="ts">`)
- Element Plus as UI library, icons globally registered
- Styles in `src/styles/main.scss` (not scoped), components use empty `<style scoped>` as placeholder
- Props typed via `defineProps<T>()`, emits via `defineEmits<T>()`

---

## Event Bubbling in Clickable Containers

TodoItem and similar list items have a root-level `@click` for navigation (e.g., open editor). Any interactive sub-element inside must use `@click.stop` to prevent unintended navigation.

```vue
<!-- Root element has @click="handleEdit" -->
<div class="todo-item" @click="handleEdit">
  <!-- Sub-element must stop propagation -->
  <span class="subtask-count" @click.stop="toggleExpand">...</span>
</div>
```

**Why**: Without `.stop`, clicking the sub-element triggers both the sub-action AND the parent navigation, confusing users.

---

## Common Mistakes

### Flex Child Text Overflow Ellipsis

**Symptom**: `text-overflow: ellipsis` doesn't work on a flex child element.

**Cause**: Flex children grow to fit content by default. Without width constraints, overflow never triggers.

**Fix**: Always pair with `flex: 1` and `min-width: 0`:

```scss
// Wrong
.text-element {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

// Correct
.text-element {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
```

**Why**: `min-width: 0` overrides the default `min-width: auto` on flex items, allowing them to shrink below content size.

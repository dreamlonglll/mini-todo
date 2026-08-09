# Component Guidelines

> How components are built in this project.

---

## Overview

- Vue 3 Composition API (`<script setup lang="ts">`)
- Element Plus as UI library, icons globally registered
- Styles in `pc/src/styles/main.scss` (not scoped), components use empty `<style scoped>` as placeholder
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

### Common Mistake: `.ProseMirror code` 样式会命中围栏代码块内的 `code`

**Symptom**: Milkdown/ProseMirror 渲染的围栏代码块显示为"深色块上一条条浅色亮条、文字不可见"。

**Cause**: 行内代码样式 `.ProseMirror code { background: #f1f5f9; padding: 2px 6px; }` 是元素选择器，
同样命中 `<pre><code>` 里嵌套的 `code`。行内元素背景按换行逐行绘制 → 深色 `pre` 内出现逐行浅色条，
且浅色文字叠浅色底不可读。子任务编辑器长期存在此潜伏缺陷，直到待办描述里粘贴含代码块的内容才暴露。

**Fix**: 在 `pre` 规则后追加重置：

```css
.markdown-editor-container :deep(.ProseMirror pre code) {
  background: transparent;
  padding: 0;
  border-radius: 0;
  font-size: inherit;
  color: inherit;
}
```

**Prevention**: 给 Markdown 渲染容器写 `code` 样式时，永远同时写 `pre code` 重置；
验收时用包含围栏代码块的样例文档过一遍。统一样式已收敛在 `pc/src/components/MarkdownEditor.vue`，
新增 Markdown 展示场景应复用该组件而不是自写样式。

### Common Mistake: 向 Milkdown WYSIWYG 粘贴 Markdown 源码会被转义存库

**Symptom**: 用户粘贴 `## 标题`、`**加粗**` 等 MD 源码后保存，DB 里存的是 `\## 标题`、`\*\*加粗\*\*`；
只读模式"如实渲染"转义文本，看起来像"渲染失效、显示源码"。

**Cause**: 不注册 clipboard 插件时，Milkdown 把粘贴文本当字面纯文本插入文档；
序列化回 Markdown 时为保住字面语义给所有语法符号加反斜杠转义。渲染链路本身无 bug，根因在输入路径。

**Fix**: 编辑模式注册 `@milkdown/kit/plugin/clipboard`（`builder.use(clipboard)`），
粘贴的 MD 源码会被解析为富文本，序列化结果即干净 Markdown。已在 `MarkdownEditor.vue` 编辑模式分支注册。

**Prevention**: 新建任何 Milkdown 编辑入口时检查插件三件套：`listener`（回写）、`upload`（图片）、
`clipboard`(粘贴解析)。排查"渲染像源码"问题时先查 DB 原文是否含反斜杠转义，再怀疑渲染层。
存量已转义数据不会被此修复救回，需重新粘贴或单独数据修复。

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

---

## Required Patterns

<!-- Patterns that must always be used -->

(To be filled by the team)

---

## Testing Requirements

<!-- What level of testing is expected -->

(To be filled by the team)

---

## Code Review Checklist

<!-- What reviewers should check -->

(To be filled by the team)

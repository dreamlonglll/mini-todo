# Desktop Window E2E Thinking Guide

> **Purpose**: Before claiming a main-window mode / style / Z-order change "works", run the checks
> below. Unit tests cannot see Win32 state; the only proof is the live window.

---

## When to Use

- [ ] You changed anything under `IS_FIXED_MODE` / `IS_DESKTOP_MODE` handling in `commands/window.rs`
- [ ] You added a tao call (`set_resizable`, `set_minimizable`, `set_always_on_top/bottom`) anywhere on the main window
- [ ] You touched `apply_fixed_ex_style`, owner (`GWLP_HWNDPARENT`), `SetWindowPos`, or the 200 ms polling thread
- [ ] You changed how `screen_configs` / `is_fixed` / `fixed_embed_desktop` are restored at startup

---

## Checklist

1. **Start the dev build with CDP enabled** — `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`
   so `pc/scripts/e2e/cdp.mjs` can click title-bar buttons without the "first click lost" problem.
2. **Assert the Win32 state, not the UI** — `e2e.ps1 -Step json`; compare every field against the
   mode table in `.trellis/spec/backend/window-modes.md` §3 (owner, ex style, minimize box, zIndex, rect, DPI).
3. **Show Desktop twice** (`-Step win-d`), **Minimize All** (`-Step win-m`): never iconic, never hidden, rect unchanged, centre hit is still us.
4. **Something must be able to cover it** — show a plain top-level window over the rect; it must win `WindowFromPoint`.
5. **Real input, not synthetic** — `mouse.ps1` at `clientOrigin + css × devicePixelRatio`; verify with CDP DOM reads.
   Editor windows opened from the main window must take foreground on the first click and accept keys.
6. **Explorer restart** (`-Step restart-explorer`, kills `explorer.exe`) — window alive, re-attached, Win+D still immune.
7. **Process restart** — kill the exe, start it again (vite separately), state and rect restored from `screen_configs`.
8. **Loop the transitions** several times — same HWND, process alive, rect unchanged after every step.
9. **Put the user's mode back** before you leave (they run fixed mode daily) and delete any test todos through the store, not raw SQL.

---

## Gotchas Seen

- `SendKeys` goes through the user's IME: text arrives transformed and Element Plus ignores
  `input` events while composing. Switch to an English layout or use CDP `Input.insertText`.
- PowerShell parses `0xFFFFFFFF` as `-1`; wheel deltas need the explicit `+ 2^32` wrap (done in `mouse.ps1`).
- Do not put `$` or escaped double quotes in the JS you pass to `cdp.mjs` from PowerShell.
- The window rect can move a few pixels while you are paused and the user pokes at it — snapshot `rect` at the start of a loop, not from memory.

→ Executable details: `.trellis/spec/backend/window-modes.md` §6, scripts in `pc/scripts/e2e/`.

# 修复点待办「完成」偶发打开详情

## 问题

主窗口待办清单（列表 / 四象限共用 `render_row`）的悬停操作条上，点「完成」偶尔不标记完成，而是打开只读详情窗口。

## 根因（已用真实程序复现）

1. 按下「完成」：操作条没有 `occlude`，「完成」按钮和下面的行都记下了这次按下（行有 `on_click`，
   `pending_mouse_down` 被置上；gpui-pre 0.3.9 `elements/div.rs:2961-2966`）。
2. 按住期间收到任意一条 `WM_MOUSEMOVE`（位移为 0 也算）：GPUI 判定有未结束按下的元素"未悬停"
   （`div.rs:3170`），行的 `on_hover(false)` 清空 `hovered_row`（`todo_item.rs:97`）。
3. 下一帧操作条因 `hovered_row != id` 不再渲染（`todo_item.rs:49`、`:74`），「完成」按钮连同它的 MouseUp 监听一起消失。
4. 松手：只有行的点击成立，`open_todo` 打开详情，「完成」里的 `stop_propagation` 没有机会执行。

复现数据（debug 构建、隔离数据目录、真实输入，每组 5 次）：

| 按住「完成」时的操作 | 打开详情 |
|---|---|
| 不动（80ms / 400ms） | 0/5 |
| 移动后立刻松开（中间没画帧） | 0/5 |
| 移动 1px，隔 8ms 松开 | 2/5 |
| 移动 1px，隔 16 / 33 / 70ms 松开 | 5/5 |
| 位移为 0 的移动消息，隔 70ms 松开 | 5/5 |

## 要做的

在 `pc-gpui/crates/app/src/windows/main_window/todo_item.rs` 的 `render_actions` 里，给 `todo-actions` 容器加：

```rust
.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
```

并在这里写明为什么要拦（按下传给行的后果）。

为什么放在容器上：GPUI 倒序派发鼠标监听（`window.rs:5958`），子元素先于父元素收到事件，所以「完成」等按钮仍能记下按下，
容器拦下后行不会记下这次按下，`on_hover` 不会报 false，操作条不会被卸载。

## 不做的

- 不用 `occlude`：会让行失去悬停，操作条闪烁消失（现有注释已说明）。
- 不处理"第一下落库后立刻点第二下、下一行操作条还没画出来"的窄窗口（隔 100ms 以上不出现），也不处理连点两下完成两条待办：
  这两点已告知用户，用户没有要求修。
- 不改已完成窗口（`completed.rs`）：它的按钮一直挂载（只调 `opacity`），不受影响。
- 不新增依赖，不给 `gpui-kit` 开 `test-support`。

## 验证约束

**用户明确要求：不跑 e2e，除非用户要求。** 包括 `scripts/e2e-*`、`regress.py`、任何启动真实窗口或模拟鼠标键盘输入的脚本。

只跑（在 `pc-gpui/` 下）：

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## 验收标准

- [x] 修复已写入，注释说明了原因
- [x] fmt / clippy / test 全绿
- [x] 按下「完成」后，行不再记下这次按下（代码层面成立；实际点击效果由用户手动确认：按住「完成」轻轻挪动鼠标，
      操作条不消失，松手标记完成，不打开详情）

# 贴边收起残留条露出列表内容

## 背景（issue #9 用户反馈）

固定模式贴边收起后，窗口只在屏幕边缘留 4 物理像素的"把手"条
（`pc/src-tauri/src/commands/window.rs` 的 `HIDDEN_VISIBLE_STRIP_PX = 4`）。
用户截图显示：贴顶收起后，这条 4px 里是列表最后一个待办项被裁切的残影，视觉上很脏。

## 根因

- `.app-container`（`pc/src/styles/main.scss`）底部除 1px 描边外无留白，
  `.main-body` → `.left-panel` → `.main-content` 一路 flex 撑满，
  滚动视口 `.main-content` 的底边直接贴住窗口底边界。
- `.main-content` 现有的 76px 底部 padding 在**滚动内容内部**（给悬浮按钮占位），
  只有滚到最底才露空白；列表溢出停在中间滚动位置时，待办项正好裁切在视口底边＝窗口底边。
- 收起后露出的 4px 落在裁切内容上。

## 方案

在滚动视口**外部**留间距：给 `.main-content` 加 `margin-bottom: 6px`。
无论滚动到哪，窗口最底部像素恒为背景色（固定模式即 v26 `window_bg_color` 半透明底），
收起后的把手条呈纯色。

- 4px 是物理像素，缩放 ≥100% 时 6 CSS px 足够覆盖并留余量。
- 其余三个贴边方向本就露出纯色（顶＝标题栏背景，左右＝16px 水平 padding），不需要改。

## 验收

- 列表溢出、停在任意滚动位置，贴顶收起后残留条为纯色，无内容残影。
- 正常展开状态下布局无可感知变化（仅底部多 6px 背景色）。
- `npm run build`（vue-tsc + vite）通过。

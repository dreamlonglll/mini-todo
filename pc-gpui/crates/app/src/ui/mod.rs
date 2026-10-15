//! Element Plus 外观的共享控件（数值照抄 `research/ui-main-window.md` §12、`ui-editor-windows.md` §1.2）。
//!
//! | 模块 | 内容 |
//! |---|---|
//! | [`icon`] | Element Plus / 应用图标（`svg()` 单色着色）、转圈动画 |
//! | [`button`] | `el-button`：default / primary / success / warning / danger / info，plain / text / link / circle，small，disabled，loading |
//! | [`tooltip`] | 原生 `title` 风格（跟随鼠标）与 Element Plus 深色 tooltip（带箭头，上方弹出） |
//! | [`message`] | `ElMessage`：窗口内顶部居中叠放、3s 自动消失、四种类型 |
//! | [`message_box`] | `ElMessageBox.confirm`：遮罩、标题 / 图标 / 内容 / 取消 + 确认、Esc / 遮罩 / × 取消、Enter 确认 |
//! | [`input`] | `el-input` 外观的单行输入（包装 gpui-component `InputState`） |
//! | [`title_bar`] | 通用自绘标题栏（拖动区 + 标题 + 关闭按钮）与 `drag_region` |
//! | [`scrollbar`] | 细滚动条（非覆盖式，主列表 6px） |
//! | [`layers`] | 每个窗口 Root 上的浮层插件（message / message_box 挂在这里，窗口视图不用管） |
//! | [`ep_popper`] | 浅色弹层（`el-popper is-light`）：`Anchor` 量取触发元素、`popper()` 定位 / 翻转 / 点外关闭、`frame()` 外框与箭头 |
//! | [`ep_select`] | `el-select`：泛型值（默认 `SharedString`）、可过滤 / 可清空 / small / 禁用、键盘导航、虚拟滚动 |
//! | [`ep_dropdown`] | `el-dropdown-menu` 菜单面板（图标、分隔线、`max-height`），配合 `popper()` 使用 |
//! | [`ep_switch`] | `el-switch`（过渡动画、disabled / loading） |
//! | [`ep_checkbox`] | `el-checkbox` |
//! | [`ep_color_picker`] | `el-color-picker size="small"`：SV 面板 + 色相条 + 可选预设色 + hex 输入 + Clear / OK |
//! | [`ep_radio`] | `el-radio-button` 组 |
//! | [`ep_slider`] | `el-slider`（input / change 两种回调） |
//! | [`ep_input_number`] | `el-input-number`（带后缀） |
//! | [`ep_date_picker`] / [`ep_time_picker`] | `el-date-picker` / `el-time-select` 面板 |
//! | [`markdown`] / [`markdown_editor`] / [`markdown_image_viewer`] | Markdown 渲染、源码 + 预览编辑器（图片上传、链接白名单）、图片预览 |
//!
//! 新控件放在 `ui/` 下单独文件，外观数值照抄 UI 规格；同一种 Element Plus 控件只保留一个实现。

pub mod button;
pub mod ep_checkbox;
pub mod ep_color_picker;
pub mod ep_date_picker;
pub mod ep_dropdown;
pub mod ep_input_number;
pub mod ep_popper;
pub mod ep_radio;
pub mod ep_select;
pub mod ep_slider;
pub mod ep_switch;
pub mod ep_time_picker;
pub mod icon;
pub mod input;
pub mod layers;
pub mod markdown;
pub mod markdown_editor;
pub mod markdown_image_viewer;
pub mod message;
pub mod message_box;
pub mod scrollbar;
pub mod title_bar;
pub mod tooltip;

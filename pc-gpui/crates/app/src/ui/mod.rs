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
//! | [`layers`] | 每个窗口 Root 上的浮层插件（message / message_box 挂在这里，窗口视图不用管） |
//!
//! 各窗口需要的其它控件（日期 / 时间 / 颜色选择、下拉、开关、滑块、复选框…）由对应窗口实现者在 `ui/` 下新建文件。
//!
//! W3（设置 / 收纳箱）新增：[`ep_switch`]（el-switch）、[`ep_slider`]（el-slider，input / change 两种回调）、
//! [`ep_radio`]（el-radio-button 组）、[`ep_select`]（el-select，可过滤 / 可清空，虚拟滚动）、
//! [`ep_color_picker`]（el-color-picker small）、[`ep_dropdown`]（el-dropdown-menu 面板）、[`ep_popper`]（浅色弹层外观与箭头）。

pub mod button;
pub mod ep_color_picker;
pub mod ep_dropdown;
pub mod ep_popper;
pub mod ep_radio;
pub mod ep_select;
pub mod ep_slider;
pub mod ep_switch;
pub mod icon;
pub mod input;
pub mod layers;
pub mod message;
pub mod message_box;
pub mod title_bar;
pub mod tooltip;

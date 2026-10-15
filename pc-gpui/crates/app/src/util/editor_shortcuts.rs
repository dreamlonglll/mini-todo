//! 编辑类窗口（待办编辑 / 子任务编辑）的键盘快捷键：Esc 关闭、Ctrl/Cmd+Enter 保存
//! （旧 `utils/editorShortcuts.ts` 的 1:1 移植）。
//!
//! GPUI 映射：窗口根元素 `key_context("MiniTodoEditor")`，用动作 [`EditorClose`] / [`EditorSubmit`] 绑定按键
//! （[`init`] 注册一次）。规则与旧版相同：
//! - 输入法组合输入中的按键属于输入法（GPUI 在 Windows 上不会把 `VK_PROCESSKEY` 当按键派发）；
//! - 被内层控件消费的按键不再响应（单行输入框的 Esc / Enter 会继续冒泡；多行编辑器的 Enter 会插入换行，
//!   所以 `MiniTodoEditor > Input` 上下文里额外把 Ctrl+Enter 绑定到保存，抢在换行之前）；
//! - 有弹出层打开时（下拉、日期 / 时间 / 颜色面板、对话框、确认框、图片预览），Esc 先交给弹出层关闭——
//!   由窗口自己的动作处理函数判断（[`resolve_editor_shortcut`] 只管按键本身）。

use gpui_kit::{App, KeyBinding, Keystroke, actions};

actions!(mini_todo_editor, [EditorClose, EditorSubmit]);

/// 编辑类窗口根元素的 key context
pub const EDITOR_CONTEXT: &str = "MiniTodoEditor";

/// 注册快捷键（`services::scheduler::start` 之前、开编辑窗口之前调用一次即可；重复调用无害）
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", EditorClose, Some(EDITOR_CONTEXT)),
        KeyBinding::new("secondary-enter", EditorSubmit, Some(EDITOR_CONTEXT)),
        // 多行编辑器把 Ctrl+Enter 当换行；同一深度后注册的绑定优先，保存抢在换行之前
        KeyBinding::new(
            "secondary-enter",
            EditorSubmit,
            Some("MiniTodoEditor > Input"),
        ),
    ]);
}

/// 快捷键
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorShortcut {
    /// Esc：关闭窗口（有未保存的修改时可先确认）
    Close,
    /// Ctrl/Cmd+Enter：保存
    Submit,
}

/// 按键（旧 `KeyboardEvent` 用到的字段）
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShortcutKey {
    /// DOM `key`：`Escape` / `Enter` / …
    pub key: String,
    pub ctrl: bool,
    pub meta: bool,
    pub alt: bool,
    pub shift: bool,
    pub is_composing: bool,
    pub key_code: u32,
}

impl ShortcutKey {
    /// GPUI 按键 → DOM 风格（`escape` → `Escape`，`enter` → `Enter`；`platform` 键即 Cmd / Win）
    pub fn from_keystroke(keystroke: &Keystroke) -> Self {
        let key = match keystroke.key.as_str() {
            "escape" => "Escape".to_string(),
            "enter" => "Enter".to_string(),
            other => other.to_string(),
        };
        Self {
            key,
            ctrl: keystroke.modifiers.control,
            meta: keystroke.modifiers.platform,
            alt: keystroke.modifiers.alt,
            shift: keystroke.modifiers.shift,
            is_composing: false,
            key_code: 0,
        }
    }
}

/// 输入法组合输入中（部分 WebView 只给出 keyCode 229 而不置 isComposing）
pub fn is_ime_composing(is_composing: bool, key_code: u32) -> bool {
    is_composing || key_code == 229
}

/// 按键 → 快捷键（纯函数）；不是快捷键返回 `None`
pub fn resolve_editor_shortcut(e: &ShortcutKey) -> Option<EditorShortcut> {
    if is_ime_composing(e.is_composing, e.key_code) || e.alt || e.shift {
        return None;
    }
    let modifier = e.ctrl || e.meta;
    match e.key.as_str() {
        "Escape" if !modifier => Some(EditorShortcut::Close),
        "Enter" if modifier => Some(EditorShortcut::Submit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    //! 原 `editorShortcuts.test.ts`
    use super::*;

    fn key(key: &str) -> ShortcutKey {
        ShortcutKey {
            key: key.into(),
            ..Default::default()
        }
    }

    #[test]
    fn maps_esc_to_close_and_ctrl_enter_to_submit() {
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                key_code: 27,
                ..key("Escape")
            }),
            Some(EditorShortcut::Close)
        );
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                ctrl: true,
                key_code: 13,
                ..key("Enter")
            }),
            Some(EditorShortcut::Submit)
        );
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                meta: true,
                key_code: 13,
                ..key("Enter")
            }),
            Some(EditorShortcut::Submit)
        );
    }

    #[test]
    fn ignores_plain_enter_and_modified_esc() {
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                key_code: 13,
                ..key("Enter")
            }),
            None
        );
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                ctrl: true,
                ..key("Escape")
            }),
            None
        );
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                ctrl: true,
                shift: true,
                ..key("Enter")
            }),
            None
        );
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                ctrl: true,
                alt: true,
                ..key("Enter")
            }),
            None
        );
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                ctrl: true,
                ..key("s")
            }),
            None
        );
    }

    #[test]
    fn leaves_keys_to_the_ime_while_composing() {
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                is_composing: true,
                ..key("Escape")
            }),
            None
        );
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                ctrl: true,
                is_composing: true,
                ..key("Enter")
            }),
            None
        );
        // 部分 WebView 组合输入时只给 keyCode 229
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey {
                ctrl: true,
                key_code: 229,
                ..key("Enter")
            }),
            None
        );
    }

    #[test]
    fn detects_composition_from_either_signal() {
        assert!(is_ime_composing(true, 13));
        assert!(is_ime_composing(false, 229));
        assert!(!is_ime_composing(false, 13));
    }

    #[test]
    fn maps_gpui_keystrokes() {
        let k = Keystroke::parse("ctrl-enter").unwrap();
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey::from_keystroke(&k)),
            Some(EditorShortcut::Submit)
        );
        let k = Keystroke::parse("escape").unwrap();
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey::from_keystroke(&k)),
            Some(EditorShortcut::Close)
        );
        let k = Keystroke::parse("shift-escape").unwrap();
        assert_eq!(
            resolve_editor_shortcut(&ShortcutKey::from_keystroke(&k)),
            None
        );
    }
}

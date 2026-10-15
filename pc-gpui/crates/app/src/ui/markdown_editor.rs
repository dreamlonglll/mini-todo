//! Markdown 编辑器（旧 `MarkdownEditor.vue` 编辑模式的替代，PRD D7）。
//!
//! Milkdown 的所见即所得在 GPUI 里无法 1:1 还原，这里改成"渲染 ⇄ 源码"原地切换：
//! - 平时显示渲染结果（与只读排版同一套 [`MarkdownView`]，看起来与旧版编辑器一致）；
//! - 单击（或拖入图片）切到 Markdown 源码编辑（等宽 13px / 行高 1.6），失焦后回到渲染结果；
//!   Ctrl/Cmd+单击链接按白名单打开（与旧版编辑排版一致），点图片打开预览；
//! - 粘贴 / 拖入图片：按旧版规则校验（只认图片、扩展名白名单、单张 ≤20MB，同样的警告文案）→ 后台
//!   `core::todo::save_image` → 在光标处插入 `![<文件名>](minitodo-image://<name>)`（剪贴板位图没有文件名，用
//!   `image.<ext>`，与浏览器粘贴截图时的 `image.png` 一致）；粘贴普通文字 / Markdown 源码原样插入。
//!
//! 值一律是**存储形式**（`minitodo-image://<name>`）：[`MarkdownEditor::value`] 每次都过一遍
//! `to_storage_markdown`（顺带把旧 asset URL 规范化）。外部赋值不发事件。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::{
    AppContext as _, ClipboardEntry, ClipboardItem, Context, Entity, EventEmitter, ExternalPaths,
    Focusable as _, ImageFormat, InteractiveElement as _, IntoElement, MouseButton,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, WeakEntity, Window, div,
    prelude::FluentBuilder as _, px, relative,
};

use crate::theme::{self, editor_theme, hex};
use crate::ui::markdown::{ImageClickHandler, MarkdownView};
use crate::ui::message::{self, MessageKind};
use crate::util::image_ref::{self, MAX_IMAGE_BYTES};

/// 多行编辑框（`Textarea` xsmall 尺寸）自带的左右内边距；外层容器的内边距要减掉它
pub const TEXTAREA_INSET_X: f32 = 4.;

/// 内容变化（读 [`MarkdownEditor::value`]）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkdownEditorEvent {
    Changed,
}

/// 浏览器会把 `File.type` 设成 `image/*` 的扩展名（非图片文件静默忽略，图片但不在白名单的给出警告）
const IMAGE_LIKE_EXTENSIONS: [&str; 15] = [
    "png", "jpg", "jpeg", "jpe", "jfif", "webp", "gif", "bmp", "svg", "tif", "tiff", "ico", "avif",
    "heic", "heif",
];

/// 一张待上传的图片
struct Upload {
    /// alt 与提示里用的名字
    name: String,
    /// 白名单扩展名；`None` = 不支持的格式
    ext: Option<String>,
    source: UploadSource,
}

enum UploadSource {
    Bytes(Vec<u8>),
    File(PathBuf),
}

/// 剪贴板位图格式 → 扩展名
fn clipboard_extension(format: ImageFormat) -> Result<&'static str, &'static str> {
    match format {
        ImageFormat::Png => Ok("png"),
        ImageFormat::Jpeg => Ok("jpg"),
        ImageFormat::Webp => Ok("webp"),
        ImageFormat::Gif => Ok("gif"),
        ImageFormat::Bmp => Ok("bmp"),
        ImageFormat::Svg => Err("image/svg+xml"),
        ImageFormat::Tiff => Err("image/tiff"),
        ImageFormat::Ico => Err("image/x-icon"),
        ImageFormat::Pnm => Err("image/x-portable-anymap"),
    }
}

/// 文件 → 待上传图片；不是图片返回 `None`（静默忽略）
fn upload_from_path(path: &Path) -> Option<Upload> {
    let name = path.file_name()?.to_string_lossy().to_string();
    let lower = name.to_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e.to_string())?;
    if !IMAGE_LIKE_EXTENSIONS.contains(&ext.as_str()) {
        return None;
    }
    Some(Upload {
        ext: image_ref::resolve_image_extension(&name, ""),
        name,
        source: UploadSource::File(path.to_path_buf()),
    })
}

/// `![alt](minitodo-image://name)`（alt 里的方括号与反斜杠转义）
pub fn image_markdown(alt: &str, file_name: &str) -> String {
    let alt: String = alt
        .chars()
        .flat_map(|c| match c {
            '[' | ']' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    format!("![{alt}]({})", image_ref::to_image_ref(file_name))
}

pub struct MarkdownEditor {
    id: SharedString,
    source: Entity<TextareaState>,
    images_dir: Arc<PathBuf>,
    editing: bool,
    readonly: bool,
    /// 源码区内边距（上下, 左右）
    padding: (f32, f32),
    /// 源码区自动增高的行数范围（`None` = 撑满父元素）
    rows: Option<(usize, usize)>,
    on_image_click: Option<ImageClickHandler>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<MarkdownEditorEvent> for MarkdownEditor {}

impl MarkdownEditor {
    /// `rows`：源码区按内容在 [min, max] 行之间自动增高（描述框）；`None` 时撑满父元素（子任务内容区）
    pub fn new(
        id: impl Into<SharedString>,
        images_dir: Arc<PathBuf>,
        rows: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let source = cx.new(|cx| {
            let state = TextareaState::new(window, cx).soft_wrap(true);
            match rows {
                Some((min, max)) => state.auto_grow(min, max),
                None => state,
            }
        });
        let subscriptions = vec![cx.subscribe_in(
            &source,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => cx.emit(MarkdownEditorEvent::Changed),
                InputEvent::Blur => {
                    if this.editing {
                        this.editing = false;
                        cx.notify();
                    }
                }
                InputEvent::Focus | InputEvent::PressEnter { .. } => {}
            },
        )];
        Self {
            id: id.into(),
            source,
            images_dir,
            editing: false,
            readonly: false,
            padding: (8., 12.),
            rows,
            on_image_click: None,
            _subscriptions: subscriptions,
        }
    }

    /// 源码区 / 渲染区内边距（上下, 左右）：描述框 8 12，子任务内容区 12 16
    pub fn set_padding(&mut self, vertical: f32, horizontal: f32) {
        self.padding = (vertical, horizontal);
    }

    pub fn set_on_image_click(&mut self, handler: ImageClickHandler) {
        self.on_image_click = Some(handler);
    }

    /// 只读：只显示渲染结果（子任务查看模式）
    pub fn set_readonly(&mut self, readonly: bool, cx: &mut Context<Self>) {
        self.readonly = readonly;
        if readonly {
            self.editing = false;
        }
        cx.notify();
    }

    /// 存储形式的内容
    pub fn value(&self, cx: &gpui_kit::App) -> String {
        image_ref::to_storage_markdown(&self.source.read(cx).value())
    }

    /// 外部赋值（存储形式；不发事件）
    pub fn set_value(&mut self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let value = image_ref::to_storage_markdown(value);
        self.source
            .update(cx, |state, cx| state.set_value(value, window, cx));
        cx.notify();
    }

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// 切到源码编辑并聚焦
    pub fn start_editing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.readonly {
            return;
        }
        self.editing = true;
        self.source.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// 粘贴：剪贴板里有图片 / 文件时接管（返回 true），否则交给默认的文字粘贴
    fn handle_paste(
        &mut self,
        item: &ClipboardItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut uploads = Vec::new();
        let mut has_files = false;
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(image) => {
                    has_files = true;
                    match clipboard_extension(image.format) {
                        Ok(ext) => uploads.push(Upload {
                            name: format!("image.{ext}"),
                            ext: Some(ext.to_string()),
                            source: UploadSource::Bytes(image.bytes.clone()),
                        }),
                        Err(mime) => uploads.push(Upload {
                            name: mime.to_string(),
                            ext: None,
                            source: UploadSource::Bytes(Vec::new()),
                        }),
                    }
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    has_files = true;
                    uploads.extend(paths.paths().iter().filter_map(|p| upload_from_path(p)));
                }
                ClipboardEntry::String(_) => {}
            }
        }
        if !has_files {
            return false;
        }
        self.upload(uploads, window, cx);
        true
    }

    fn handle_drop(&mut self, paths: &ExternalPaths, window: &mut Window, cx: &mut Context<Self>) {
        if self.readonly {
            return;
        }
        let uploads: Vec<Upload> = paths
            .paths()
            .iter()
            .filter_map(|p| upload_from_path(p))
            .collect();
        if uploads.is_empty() {
            return;
        }
        self.start_editing(window, cx);
        self.upload(uploads, window, cx);
    }

    /// 逐张校验并后台保存；每张完成后在光标处插入引用（同一批并行，单张失败不影响其它）
    fn upload(&mut self, uploads: Vec<Upload>, window: &mut Window, cx: &mut Context<Self>) {
        for upload in uploads {
            let Some(ext) = upload.ext.clone() else {
                message::warning(
                    window,
                    cx,
                    format!(
                        "不支持的图片格式：{}（支持 png / jpg / webp / gif / bmp）",
                        upload.name
                    ),
                );
                continue;
            };
            let size = match &upload.source {
                UploadSource::Bytes(bytes) => bytes.len() as u64,
                UploadSource::File(path) => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            };
            if size > MAX_IMAGE_BYTES as u64 {
                message::warning(
                    window,
                    cx,
                    format!("图片超过 20MB，未插入：{}", upload.name),
                );
                continue;
            }
            let name = upload.name.clone();
            let source = upload.source;
            let task = cx.background_executor().spawn(async move {
                let bytes = match source {
                    UploadSource::Bytes(bytes) => bytes,
                    UploadSource::File(path) => {
                        std::fs::read(&path).map_err(|e| format!("读取图片失败: {e}"))?
                    }
                };
                mini_todo_core::todo::save_image(&bytes, &ext)
            });
            let handle = window.window_handle();
            cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
                match task.await {
                    Ok(path) => {
                        let Some(file_name) =
                            path.file_name().map(|n| n.to_string_lossy().to_string())
                        else {
                            return;
                        };
                        let text = image_markdown(&name, &file_name);
                        let _ = this.update_in(cx, |this, window, cx| {
                            this.source
                                .update(cx, |state, cx| state.insert(text, window, cx));
                            cx.emit(MarkdownEditorEvent::Changed);
                            cx.notify();
                        });
                    }
                    Err(e) => {
                        log::error!("[markdown] 图片保存失败: {e}");
                        let _ = cx.update(|_, cx| {
                            message::show_in(
                                handle,
                                cx,
                                MessageKind::Error,
                                message::error_text("图片保存失败", &e),
                            )
                        });
                    }
                }
            })
            .detach();
        }
    }
}

impl Render for MarkdownEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (pad_y, pad_x) = self.padding;
        let editing = self.editing && !self.readonly;
        let body = if editing {
            let weak = cx.entity().downgrade();
            let textarea = Textarea::new(&self.source)
                .appearance(false)
                .bordered(false)
                .xsmall()
                .text_size(px(13.))
                .line_height(relative(1.6))
                .w_full()
                .when(self.rows.is_none(), |this| this.h_full())
                .on_paste(move |item, window, cx| {
                    weak.update(cx, |this, cx| this.handle_paste(item, window, cx))
                        .unwrap_or(false)
                });
            div()
                .w_full()
                .when(self.rows.is_none(), |this| this.size_full())
                .px(px(pad_x - TEXTAREA_INSET_X))
                .py(px(pad_y))
                .font(editor_theme::mono_font())
                .text_size(px(13.))
                .line_height(relative(1.6))
                .text_color(hex(editor_theme::SLATE_700))
                .child(textarea)
                .into_any_element()
        } else {
            let source = SharedString::from(self.source.read(cx).value().to_string());
            let mut view = MarkdownView::new(
                SharedString::from(format!("{}-preview", self.id)),
                source,
                self.images_dir.clone(),
            )
            .readonly(self.readonly)
            .selectable(self.readonly);
            if let Some(handler) = self.on_image_click.clone() {
                view = view.on_image_click(move |paths, index, window, cx| {
                    handler(paths, index, window, cx)
                });
            }
            div()
                .w_full()
                .px(px(pad_x))
                .py(px(pad_y))
                .child(view)
                .into_any_element()
        };
        let focused = self
            .source
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        div()
            .id(SharedString::from(format!("{}-root", self.id)))
            .w_full()
            .min_h_full()
            .when(self.rows.is_none(), |this| this.size_full())
            .flex()
            .flex_col()
            .when(!self.readonly && !editing, |this| {
                this.cursor_text().on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, event: &gpui_kit::MouseUpEvent, window, cx| {
                        // Ctrl/Cmd+单击留给链接
                        if event.modifiers.control || event.modifiers.platform {
                            return;
                        }
                        this.start_editing(window, cx);
                    }),
                )
            })
            .when(!self.readonly, |this| {
                this.drag_over::<ExternalPaths>(|style, _, _, _| style)
                    .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                        this.handle_drop(paths, window, cx)
                    }))
            })
            .when(focused && editing, |this| {
                this.text_color(theme::light().text_primary)
            })
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_image_markdown_with_escaped_alt() {
        assert_eq!(
            image_markdown("image.png", "1700_abc123.png"),
            "![image.png](minitodo-image://1700_abc123.png)"
        );
        assert_eq!(
            image_markdown("a[1].png", "x.png"),
            "![a\\[1\\].png](minitodo-image://x.png)"
        );
    }

    #[test]
    fn classifies_dropped_files() {
        assert!(upload_from_path(Path::new("/tmp/notes.txt")).is_none());
        let png = upload_from_path(Path::new("/tmp/Shot.PNG")).unwrap();
        assert_eq!(png.ext.as_deref(), Some("png"));
        assert_eq!(png.name, "Shot.PNG");
        let tiff = upload_from_path(Path::new("/tmp/scan.tiff")).unwrap();
        assert_eq!(tiff.ext, None);
        assert_eq!(clipboard_extension(ImageFormat::Jpeg), Ok("jpg"));
        assert!(clipboard_extension(ImageFormat::Svg).is_err());
    }
}

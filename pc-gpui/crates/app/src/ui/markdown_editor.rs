//! Markdown 编辑器（旧 `MarkdownEditor.vue` 编辑模式的替代，PRD D7）。
//!
//! Milkdown 的所见即所得在 GPUI 里无法 1:1 还原，这里改成"渲染 ⇄ 源码"原地切换：
//! - 平时显示渲染结果（与只读排版同一套 [`MarkdownView`]，看起来与旧版编辑器一致）；
//! - 单击（或拖入图片）切到 Markdown 源码编辑（等宽 13px / 行高 1.6），失焦后回到渲染结果——鼠标按下引起的失焦
//!   等松手再切：渲染结果通常比源码高，按下就切会把下方的控件挤走（这次单击落空），松手还可能落在渲染结果上；
//!   Ctrl/Cmd+单击链接按白名单打开（与旧版编辑排版一致），单击图片打开预览（不切到源码）；
//! - 渲染结果里悬停图片显示调整大小的手柄（[`MarkdownView::image_resize`]）：拖动时按实时尺寸显示，松手后把尺寸写进
//!   这张图片的 alt 后缀（`![image.png|300](…)`，宽高比改变时 `|300x200`），双击手柄还原原始尺寸；改写源码与打字一样
//!   发 [`MarkdownEditorEvent::Changed`]、可在源码里撤销；
//! - 粘贴 / 拖入图片：按旧版规则校验（只认图片、扩展名白名单、单张 ≤20MB，同样的警告文案）→ 后台
//!   `core::todo::save_image` → 在光标处插入 `![<文件名>](minitodo-image://<name>)`（剪贴板位图没有文件名，用
//!   `image.<ext>`，与浏览器粘贴截图时的 `image.png` 一致）；原图宽于 [`editor_theme::MD_IMAGE_INSERT_WIDTH`]（240）时
//!   alt 带上 `|240`，默认按 240 宽显示（读不到原图尺寸、或原图更窄时不带，按原始尺寸）；粘贴普通文字 / Markdown 源码原样插入。
//!   粘贴在 `Paste` 动作的捕获阶段处理（先于输入框）：Windows 上直接读系统剪贴板（`platform::clipboard`：PNG 优先，
//!   GPUI 不认的 `image/png` 等格式也认），剪贴板位图在后台转成 PNG 再保存；复制的文件里有图片时只插入这些文件。
//!   既没有可用的图片、也没有文字时给出提示（不再静默失败）。
//!
//! 值一律是**存储形式**（`minitodo-image://<name>`）：[`MarkdownEditor::value`] 每次都过一遍
//! `to_storage_markdown`（顺带把旧 asset URL 规范化）。外部赋值不发事件。

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::Sizable as _;
use gpui_kit::component::input::{self, InputEvent, Textarea, TextareaState};
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, ExternalPaths, Focusable as _,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Pixels, Point, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _, px, relative,
};

use crate::platform::clipboard::{self, ClipboardContent, ClipboardImage};
use crate::theme::{self, editor_theme, hex};
use crate::ui::external_drop::{DropScope, ExternalDrop};
use crate::ui::markdown::{
    self, BlockImage, ImageClickHandler, ImageResize, ImageResizeEvent, ImageSize, MarkdownView,
    ResizeHandle,
};
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
#[derive(Debug, PartialEq, Eq)]
struct Upload {
    /// alt 与提示里用的名字
    name: String,
    /// 白名单扩展名；`None` = 不支持的格式
    ext: Option<String>,
    source: UploadSource,
}

#[derive(Debug, PartialEq, Eq)]
enum UploadSource {
    Bytes(Vec<u8>),
    File(PathBuf),
    /// 剪贴板里的位图（BMP 文件）：后台转成 PNG 再保存（大小限制按转换后的 PNG 算）
    Bitmap(Vec<u8>),
}

/// 剪贴板图片 → 待上传图片（位图存成 PNG：`image.png`）
fn upload_from_clipboard(image: ClipboardImage) -> Upload {
    match image {
        ClipboardImage::Encoded { bytes, ext } => Upload {
            name: format!("image.{ext}"),
            ext: Some(ext.to_string()),
            source: UploadSource::Bytes(bytes),
        },
        ClipboardImage::Bitmap(bytes) => Upload {
            name: "image.png".to_string(),
            ext: Some("png".to_string()),
            source: UploadSource::Bitmap(bytes),
        },
        ClipboardImage::Unsupported(mime) => Upload {
            name: mime.to_string(),
            ext: None,
            source: UploadSource::Bytes(Vec::new()),
        },
    }
}

/// 一次粘贴怎么处理
#[derive(Debug, PartialEq, Eq)]
enum PastePlan {
    /// 上传这些图片
    Upload(Vec<Upload>),
    /// 交给输入框的默认文字粘贴
    Text,
    /// 复制的文件里没有图片
    NoImageFiles,
    /// 剪贴板里有东西，但既没有可用的图片也没有文字
    Nothing,
}

/// 复制的文件里有图片时只插入这些文件（聊天软件常同时放缓存文件与位图，避免插两张）；否则用剪贴板图片；
/// 都没有时有文字就交给默认粘贴
fn paste_plan(content: ClipboardContent) -> PastePlan {
    let files: Vec<Upload> = content
        .files
        .iter()
        .filter_map(|path| upload_from_path(path))
        .collect();
    if !files.is_empty() {
        return PastePlan::Upload(files);
    }
    if let Some(image) = content.image {
        return PastePlan::Upload(vec![upload_from_clipboard(image)]);
    }
    if content.has_text {
        PastePlan::Text
    } else if !content.files.is_empty() {
        PastePlan::NoImageFiles
    } else if content.formats.is_empty() {
        // 剪贴板是空的：交给默认粘贴（什么都不做）
        PastePlan::Text
    } else {
        PastePlan::Nothing
    }
}

/// 粘贴图片（在 `input::Paste` 动作的捕获阶段调用，先于输入框）：剪贴板里有图片 / 图片文件时接管并上传，每张保存完插入
/// `source` 的光标处，返回 true；否则返回 false，交给输入框的默认文字粘贴。Markdown 编辑器与描述放大编辑弹窗的源码框共用。
///
/// Windows 上直接读系统剪贴板（`platform::clipboard`），读不到时用 GPUI 的结果
pub fn paste_images(source: &Entity<TextareaState>, window: &mut Window, cx: &mut App) -> bool {
    let content = match clipboard::read() {
        Some(content) => content,
        None => match cx.read_from_clipboard() {
            Some(item) => clipboard::from_item(&item),
            None => return false,
        },
    };
    let formats = content.formats.clone();
    match paste_plan(content) {
        PastePlan::Upload(uploads) => {
            log::info!(
                "[markdown] 粘贴图片 {:?}（剪贴板格式 {formats:?}）",
                uploads.iter().map(|u| u.name.as_str()).collect::<Vec<_>>()
            );
            upload_into(source, uploads, window, cx);
            true
        }
        PastePlan::Text => false,
        PastePlan::NoImageFiles => {
            message::warning(
                window,
                cx,
                "复制的文件不是图片，只能粘贴图片（支持 png / jpg / webp / gif / bmp）",
            );
            true
        }
        PastePlan::Nothing => {
            log::warn!("[markdown] 剪贴板里没有可粘贴的图片或文字（格式 {formats:?}）");
            message::warning(window, cx, "剪贴板里没有可粘贴的图片或文字");
            true
        }
    }
}

/// 逐张校验并后台保存；每张完成后在 `source` 的光标处插入引用（插入照常发 `InputEvent::Change`；同一批并行，
/// 单张失败不影响其它；输入框已销毁——窗口关了——就丢掉）
fn upload_into(
    source: &Entity<TextareaState>,
    uploads: Vec<Upload>,
    window: &mut Window,
    cx: &mut App,
) {
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
        // 位图的大小按转换后的 PNG 算（在后台检查，见下）
        let size = match &upload.source {
            UploadSource::Bytes(bytes) => bytes.len() as u64,
            UploadSource::File(path) => std::fs::metadata(path).map(|m| m.len()).unwrap_or(0),
            UploadSource::Bitmap(_) => 0,
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
        let bytes_source = upload.source;
        let task = cx.background_executor().spawn(async move {
            let bytes = match bytes_source {
                UploadSource::Bytes(bytes) => bytes,
                UploadSource::File(path) => {
                    std::fs::read(&path).map_err(|e| format!("读取图片失败: {e}"))?
                }
                UploadSource::Bitmap(bmp) => clipboard::bitmap_to_png(&bmp)?,
            };
            if bytes.len() > MAX_IMAGE_BYTES {
                return Err("图片超过 20MB，未插入".to_string());
            }
            let width = insert_width(encoded_dimensions(&bytes));
            mini_todo_core::todo::save_image(&bytes, &ext).map(|path| (path, width))
        });
        let handle = window.window_handle();
        let target = source.downgrade();
        window
            .spawn(cx, async move |cx| match task.await {
                Ok((path, width)) => {
                    let Some(file_name) = path.file_name().map(|n| n.to_string_lossy().to_string())
                    else {
                        return;
                    };
                    let text = image_markdown(&name, &file_name, width);
                    let _ = cx.update(|window, cx| {
                        if let Some(target) = target.upgrade() {
                            target.update(cx, |state, cx| state.insert(text, window, cx));
                        }
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
            })
            .detach();
    }
}

/// 拖动手柄调整图片大小：手势状态 + 松手后改写源码。Markdown 编辑器的渲染结果与描述放大编辑弹窗的预览共用
/// （`windows/editor/desc_dialog.rs`）
#[derive(Default)]
pub struct ImageResizer {
    session: Option<ResizeSession>,
}

/// [`ImageResizer::handle`] 的结果
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeOutcome {
    /// 与当前手势无关（别的图片的松手、尺寸没变的移动……）
    Ignored,
    /// 拖动中尺寸变了：调用方重绘
    Redraw,
    /// 按下 / 松开了手柄（调用方重绘）；`rewritten` = 改写了源码，调用方当作内容变化处理
    Gesture { rewritten: bool },
}

impl ImageResizer {
    /// 丢掉进行中的手势（切到源码编辑、变成只读时）
    pub fn cancel(&mut self) {
        self.session = None;
    }

    /// 每次渲染时调用：记下本帧交给图片框的尺寸（下一次拖动事件里的图片框大小就是按它排出来的），
    /// 返回交给 [`MarkdownView::image_resize`] 的参数
    pub fn prepare(&mut self, handler: markdown::ImageResizeHandler) -> ImageResize {
        if let Some(session) = self.session.as_mut() {
            session.rendered = session.current;
        }
        ImageResize {
            active: self
                .session
                .as_ref()
                .and_then(|s| s.current.map(|(w, h)| (s.key, w, h))),
            handler,
        }
    }

    /// 处理一次手势事件。`source` 是源码输入框；`rendered` 是渲染结果所用的源码（图片的源码位置按它算，
    /// 与输入框里的文字对不上时不改写——放大编辑弹窗的预览有 300ms 防抖）
    pub fn handle(
        &mut self,
        event: ImageResizeEvent,
        source: &Entity<TextareaState>,
        rendered: &str,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) -> ResizeOutcome {
        match event {
            ImageResizeEvent::Start {
                image,
                handle,
                position,
                natural,
            } => {
                let Some(key) = image.span.as_ref().map(|span| span.range.start) else {
                    return ResizeOutcome::Ignored;
                };
                self.session = Some(ResizeSession {
                    image,
                    key,
                    handle,
                    natural,
                    start_position: position,
                    start_size: None,
                    current: None,
                    rendered: None,
                    max_width: None,
                });
                ResizeOutcome::Gesture { rewritten: false }
            }
            ImageResizeEvent::Move {
                key,
                position,
                frame,
                free,
            } => {
                let Some(session) = self.session.as_mut().filter(|s| s.key == key) else {
                    return ResizeOutcome::Ignored;
                };
                let frame = (f32::from(frame.width), f32::from(frame.height));
                let start = *session.start_size.get_or_insert(frame);
                // 上一帧请求的宽度比图片框实际排出来的宽：被容器宽度限制（max-width 100%），之后不再往宽里拉
                if let Some((width, _)) = session.rendered
                    && frame.0 + 0.5 < width
                {
                    session.max_width = Some(session.max_width.map_or(frame.0, |m| m.min(frame.0)));
                }
                let delta = (
                    f32::from(position.x - session.start_position.x),
                    f32::from(position.y - session.start_position.y),
                );
                let size =
                    markdown::resize_by(session.handle, start, delta, free, session.max_width);
                if session.current == Some(size) {
                    return ResizeOutcome::Ignored;
                }
                session.current = Some(size);
                ResizeOutcome::Redraw
            }
            ImageResizeEvent::Container { key, width } => {
                if let Some(session) = self.session.as_mut().filter(|s| s.key == key)
                    && width > 0.
                {
                    session.max_width = Some(session.max_width.map_or(width, |m| m.min(width)));
                }
                ResizeOutcome::Ignored
            }
            ImageResizeEvent::End { key } => {
                let Some(session) = self.session.take_if(|s| s.key == key) else {
                    return ResizeOutcome::Ignored;
                };
                let rewritten = match session.current {
                    Some((width, height)) => {
                        let size = markdown::committed_size(width, height, session.natural);
                        session.image.size != Some(size)
                            && rewrite_image(
                                source,
                                rendered,
                                &session.image,
                                Some(size),
                                window,
                                cx,
                            )
                    }
                    None => false,
                };
                ResizeOutcome::Gesture { rewritten }
            }
            ImageResizeEvent::Reset { image } => {
                self.session = None;
                let rewritten = image.size.is_some()
                    && rewrite_image(source, rendered, &image, None, window, cx);
                ResizeOutcome::Gesture { rewritten }
            }
        }
    }
}

/// 把一张图片的源码换成带新尺寸的写法（`None` = 原始尺寸），返回是否改写了。改写前核对输入框里那段文字与渲染时的一致
/// 且确实是图片；用"选中这段 + 替换"，源码里可以撤销
fn rewrite_image(
    source: &Entity<TextareaState>,
    rendered: &str,
    image: &BlockImage,
    size: Option<ImageSize>,
    window: &mut Window,
    cx: &mut gpui_kit::App,
) -> bool {
    let Some(span) = image.span.as_ref() else {
        return false;
    };
    let range = span.range.clone();
    let text = source.read(cx).value();
    let (Some(current), Some(expected)) = (text.get(range.clone()), rendered.get(range.clone()))
    else {
        log::warn!("[markdown] 调整图片大小：源码位置 {range:?} 已失效");
        return false;
    };
    if current != expected || !current.starts_with("![") {
        log::warn!("[markdown] 调整图片大小：源码位置 {range:?} 的内容已变，不改写");
        return false;
    }
    let replacement = markdown::image_source(image, size);
    if current == replacement {
        return false;
    }
    source.update(cx, |state, cx| {
        state.set_selected_range(range, cx);
        state.replace(replacement, window, cx);
    });
    true
}

/// 拖动手柄调整图片大小的会话
struct ResizeSession {
    image: BlockImage,
    /// 图片在源码里的起点（[`markdown::ImageSpan::range`]）
    key: usize,
    handle: ResizeHandle,
    natural: Option<(u32, u32)>,
    start_position: Point<Pixels>,
    /// 第一次移动时图片框的实际大小
    start_size: Option<(f32, f32)>,
    /// 当前请求的显示尺寸（还没移动过时 `None`）
    current: Option<(f32, f32)>,
    /// 最近一次渲染时交给图片框的尺寸。拖动事件里的图片框大小是上一帧按它排出来的；鼠标移动比重绘快，
    /// 两帧之间会来好几次移动，所以判断"被容器宽度限制"只能拿它比，不能拿最新的 `current` 比
    /// （否则放大时第二次移动就把还没重绘的旧宽度当成上限，一次拖动只能放大一点）
    rendered: Option<(f32, f32)>,
    /// 宽度上限：图片所在行的宽度（[`ImageResizeEvent::Container`]，拖动的第一次移动就报上来）；兜底再看图片框是否比
    /// 渲染时请求的窄（被容器宽度限制，max-width 100%）
    max_width: Option<f32>,
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

/// 已编码图片的像素尺寸（只读文件头；格式不认识 / 读不出时 `None`）
fn encoded_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()
}

/// 新插入图片的默认显示宽度：原图宽于 [`editor_theme::MD_IMAGE_INSERT_WIDTH`] 时缩到它（高按原图比例），
/// 否则 `None`（不写后缀，按原始尺寸；小图不放大）
fn insert_width(natural: Option<(u32, u32)>) -> Option<u32> {
    let limit = editor_theme::MD_IMAGE_INSERT_WIDTH;
    natural.filter(|(w, h)| *w > limit && *h > 0).map(|_| limit)
}

/// `![alt](minitodo-image://name)`，`width` 有值时 alt 带尺寸后缀 `|宽`（alt 里的方括号与反斜杠转义）
pub fn image_markdown(alt: &str, file_name: &str, width: Option<u32>) -> String {
    let mut alt: String = alt
        .chars()
        .flat_map(|c| match c {
            '[' | ']' | '\\' => vec!['\\', c],
            c => vec![c],
        })
        .collect();
    if let Some(width) = width {
        alt.push('|');
        alt.push_str(&width.to_string());
    }
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
    /// 源码区自动增高的行数范围（`None` = 撑满父元素）。父元素须是 flex 列：编辑器根元素按 `flex: 1 0 auto` 撑满它
    rows: Option<(usize, usize)>,
    on_image_click: Option<ImageClickHandler>,
    /// 拖入图片（不用 GPUI `on_drop`：刚打过字的键盘模态下它收不到松手，见 `ui::external_drop`）
    external_drop: ExternalDrop,
    /// 拖动手柄调整图片大小
    image_resizer: ImageResizer,
    /// 源码编辑中按下了鼠标左键、还没松开：这期间的失焦先不切回渲染结果，松手时再看（[`Self::finish_press`]）
    press_pending: bool,
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
                InputEvent::Change => {
                    // 粘贴 / 拖入的图片在后台保存完才插入：渲染结果要跟着刷新
                    cx.emit(MarkdownEditorEvent::Changed);
                    cx.notify();
                }
                InputEvent::Blur => {
                    // 鼠标按下引起的失焦等松手再切（见 `finish_press`）；键盘 / 程序移走焦点立即切
                    if this.editing && !this.press_pending {
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
            external_drop: ExternalDrop::new(),
            image_resizer: ImageResizer::default(),
            press_pending: false,
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
            self.press_pending = false;
            self.image_resizer.cancel();
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
        self.image_resizer.cancel();
        self.editing = true;
        self.press_pending = false;
        self.source.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// 源码编辑中按下的左键松开了（框内框外都算）：焦点已经不在源码框（按下时被别处拿走）就切回渲染结果。
    /// 在松手的捕获阶段执行，这次单击按的仍是当前这一帧的布局（点颜色 / 四象限照常生效），下一帧才换成渲染结果
    fn finish_press(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !std::mem::take(&mut self.press_pending) || !self.editing {
            return;
        }
        let focused = self
            .source
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        if !focused {
            self.editing = false;
            cx.notify();
        }
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
        upload_into(&self.source, uploads, window, cx);
    }

    /// 调整图片大小的手势（渲染结果里的手柄）。渲染结果的源码就是输入框里的文字（手势期间不能打字）
    fn on_image_resize(
        &mut self,
        event: ImageResizeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.readonly || self.editing {
            self.image_resizer.cancel();
            return;
        }
        let rendered = self.source.read(cx).value();
        match self
            .image_resizer
            .handle(event, &self.source, &rendered, window, cx)
        {
            ResizeOutcome::Ignored => {}
            ResizeOutcome::Redraw => cx.notify(),
            ResizeOutcome::Gesture { rewritten } => {
                // 手柄的按下拦住了冒泡，根元素收不到这次按下，松手落在渲染结果上也不算单击、不切到源码编辑
                if rewritten {
                    cx.emit(MarkdownEditorEvent::Changed);
                }
                cx.notify();
            }
        }
    }
}

impl Render for MarkdownEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (pad_y, pad_x) = self.padding;
        let editing = self.editing && !self.readonly;
        let body = if editing {
            let textarea = Textarea::new(&self.source)
                .appearance(false)
                .bordered(false)
                .xsmall()
                .text_size(px(13.))
                .line_height(relative(1.6))
                .w_full()
                .when(self.rows.is_none(), |this| this.h_full());
            div()
                .w_full()
                // 粘贴在捕获阶段先于输入框处理：GPUI 读不出剪贴板时（只有它不认的图片格式）输入框的粘贴钩子根本不会被调用
                .capture_action(cx.listener(|this, _: &input::Paste, window, cx| {
                    if paste_images(&this.source, window, cx) {
                        cx.stop_propagation();
                    }
                }))
                // 撑满根元素（根元素已撑满内容框），源码框在框内滚动
                .when(self.rows.is_none(), |this| this.flex_1().min_h_0())
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
            if !self.readonly {
                let weak = cx.entity().downgrade();
                view = view.image_resize(self.image_resizer.prepare(Rc::new(
                    move |event, window, cx| {
                        let _ = weak.update(cx, |this, cx| this.on_image_resize(event, window, cx));
                    },
                )));
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
        // 根元素 `flex: 1 0 auto`：撑满父元素（描述框 / 子任务内容框里的 flex 列）的剩余高度，内容更高时随内容增高。
        // 这样点框内任意位置（空描述、内容下方的空白）都落在根元素上、进入源码编辑，与旧版 `.ProseMirror { min-height }`
        // 一致。不能用 `min_h_full`：描述框只有 min / max 高度，百分比最小高度不生效，点击区会缩成内容高度（缺陷 D1）
        let root = div()
            .id(SharedString::from(format!("{}-root", self.id)))
            .w_full()
            .flex_grow(1.)
            .flex_shrink_0()
            .flex()
            .flex_col()
            // 渲染结果：单击（按下、松开都在根元素上）切到源码编辑。不能只看松手：从框外按下、源码切成更高的渲染结果后，
            // 松手会落在渲染结果上，被当成单击又切回源码（点描述框下方的空白会闪一下又回到源码编辑）。
            // 图片手柄的按下、图片的单击都拦住了冒泡，到不了这里
            .when(!self.readonly && !editing, |this| {
                this.cursor_text().on_click(cx.listener(
                    |this, event: &gpui_kit::ClickEvent, window, cx| {
                        // Ctrl/Cmd+单击留给链接
                        let modifiers = event.modifiers();
                        if modifiers.control || modifiers.platform {
                            return;
                        }
                        this.start_editing(window, cx);
                    },
                ))
            })
            // 源码编辑：记下左键按下（框内框外都算），失焦等松手再切回渲染结果（`finish_press`）
            .when(editing, |this| {
                let press = |this: &mut Self, event: &gpui_kit::MouseDownEvent| {
                    if event.button == MouseButton::Left {
                        this.press_pending = true;
                    }
                };
                this.capture_any_mouse_down(
                    cx.listener(move |this, event, _, _| press(this, event)),
                )
                .on_mouse_down_out(cx.listener(move |this, event, _, _| press(this, event)))
                .capture_any_mouse_up(
                    cx.listener(|this, _, window, cx| this.finish_press(window, cx)),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| this.finish_press(window, cx)),
                )
            })
            .when(!self.readonly, |this| {
                this.drag_over::<ExternalPaths>(|style, _, _, _| style)
            })
            .when(focused && editing, |this| {
                this.text_color(theme::light().text_primary)
            })
            .child(body);
        if self.readonly {
            return root;
        }
        self.external_drop.attach(
            root,
            DropScope::Element,
            cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.handle_drop(paths, window, cx)
            }),
            |_, _, _| {},
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_image_markdown_with_escaped_alt() {
        assert_eq!(
            image_markdown("image.png", "1700_abc123.png", None),
            "![image.png](minitodo-image://1700_abc123.png)"
        );
        assert_eq!(
            image_markdown("a[1].png", "x.png", None),
            "![a\\[1\\].png](minitodo-image://x.png)"
        );
        assert_eq!(
            image_markdown("image.png", "1700_abc123.png", Some(240)),
            "![image.png|240](minitodo-image://1700_abc123.png)"
        );
    }

    /// 插入的尺寸后缀能被渲染侧按同样的规则解析回来（alt 里本来就有 `|` 时取最后一段）
    #[test]
    fn inserted_size_suffix_round_trips() {
        for (alt, expected_alt) in [("image.png", "image.png"), ("a|b.png", "a|b.png")] {
            let text = image_markdown(alt, "x.png", Some(240));
            let blocks = markdown::parse_blocks(&text);
            let markdown::Block::Images(images) = &blocks[0] else {
                panic!("应解析成图片段落: {blocks:?}");
            };
            assert_eq!(images[0].alt, expected_alt);
            assert_eq!(
                images[0].size,
                Some(ImageSize {
                    width: 240,
                    height: None
                })
            );
        }
    }

    #[test]
    fn insert_width_only_shrinks_wide_images() {
        assert_eq!(insert_width(Some((1920, 1080))), Some(240));
        assert_eq!(insert_width(Some((241, 10))), Some(240));
        // 不比默认宽度宽：按原始尺寸，不放大
        assert_eq!(insert_width(Some((240, 500))), None);
        assert_eq!(insert_width(Some((100, 100))), None);
        // 读不到尺寸 / 异常尺寸：按原始尺寸
        assert_eq!(insert_width(None), None);
        assert_eq!(insert_width(Some((1000, 0))), None);
    }

    #[test]
    fn reads_encoded_dimensions_from_header() {
        let mut png = Vec::new();
        image::RgbImage::new(300, 120)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert_eq!(encoded_dimensions(&png), Some((300, 120)));
        assert_eq!(encoded_dimensions(b"not an image"), None);
    }

    #[test]
    fn classifies_dropped_files() {
        assert!(upload_from_path(Path::new("/tmp/notes.txt")).is_none());
        let png = upload_from_path(Path::new("/tmp/Shot.PNG")).unwrap();
        assert_eq!(png.ext.as_deref(), Some("png"));
        assert_eq!(png.name, "Shot.PNG");
        let tiff = upload_from_path(Path::new("/tmp/scan.tiff")).unwrap();
        assert_eq!(tiff.ext, None);
    }

    fn content(
        files: &[&str],
        image: Option<ClipboardImage>,
        has_text: bool,
        formats: &[&str],
    ) -> ClipboardContent {
        ClipboardContent {
            files: files.iter().map(PathBuf::from).collect(),
            image,
            has_text,
            formats: formats.iter().map(|f| f.to_string()).collect(),
        }
    }

    #[test]
    fn paste_plan_prefers_image_files_then_clipboard_images_then_text() {
        let png = || ClipboardImage::Encoded {
            bytes: vec![1, 2],
            ext: "png",
        };
        // 文件里有图片：只插入文件（不重复插入同时放进剪贴板的位图）
        let PastePlan::Upload(uploads) = paste_plan(content(
            &["/tmp/a.png", "/tmp/b.txt"],
            Some(png()),
            true,
            &["x"],
        )) else {
            panic!("应上传文件")
        };
        assert_eq!(uploads.len(), 1);
        assert_eq!(uploads[0].name, "a.png");
        // 文件都不是图片：用剪贴板图片
        let PastePlan::Upload(uploads) =
            paste_plan(content(&["/tmp/cache"], Some(png()), false, &["x"]))
        else {
            panic!("应上传剪贴板图片")
        };
        assert_eq!(uploads[0].name, "image.png");
        // 位图存成 PNG
        let PastePlan::Upload(uploads) = paste_plan(content(
            &[],
            Some(ClipboardImage::Bitmap(vec![b'B', b'M'])),
            true,
            &["x"],
        )) else {
            panic!("应上传位图")
        };
        assert_eq!(uploads[0].ext.as_deref(), Some("png"));
        assert_eq!(uploads[0].source, UploadSource::Bitmap(vec![b'B', b'M']));
        // 不支持的格式：给出警告（ext = None）
        let PastePlan::Upload(uploads) = paste_plan(content(
            &[],
            Some(ClipboardImage::Unsupported("image/tiff")),
            false,
            &["x"],
        )) else {
            panic!("应提示不支持")
        };
        assert_eq!(uploads[0].ext, None);
        // 没有图片
        assert_eq!(
            paste_plan(content(&[], None, true, &["x"])),
            PastePlan::Text
        );
        assert_eq!(
            paste_plan(content(&["/tmp/a.txt"], None, false, &["x"])),
            PastePlan::NoImageFiles
        );
        assert_eq!(
            paste_plan(content(&[], None, false, &["49161:HTML Format"])),
            PastePlan::Nothing
        );
        assert_eq!(paste_plan(content(&[], None, false, &[])), PastePlan::Text);
    }
}

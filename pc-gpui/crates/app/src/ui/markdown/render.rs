//! Markdown GPUI 视图与块级/图片/手柄渲染。

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{TextView, TextViewStyle};
use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, BoxShadow, Context, CursorStyle, DragMoveEvent, Empty,
    FontWeight, HighlightStyle, Hsla, ImageSource, InteractiveElement as _, IntoElement,
    MouseButton, MouseDownEvent, ObjectFit, ParentElement as _, Pixels, Point, Render, RenderOnce,
    SharedString, SharedUri, Size, StatefulInteractiveElement as _, StyleRefinement, Styled,
    StyledImage as _, Window, div, img, prelude::FluentBuilder as _, px, relative, rems,
};

use crate::theme::{self, editor_theme, hex};
use crate::ui::tooltip::native;
use crate::util::{file_link, image_ref};

use super::image::{ResizeHandle, block_image_paths, display_size, image_size};
use super::parse::{escape_text, parsed};
use super::{
    Block, BlockImage, ColumnAlign, InlinePart, MdList, MdTable, heading_metrics, item_margins,
};

/// 点击图片：(文档里全部可预览的图片（[`block_image_paths`]）, 被点的下标)
pub type ImageClickHandler = Rc<dyn Fn(Vec<PathBuf>, usize, &mut Window, &mut App)>;

/// 调整图片大小的手势（一张图片用 `key` = [`super::ImageSpan::range`] 的起点标识）
pub enum ImageResizeEvent {
    /// 按下手柄
    Start {
        image: BlockImage,
        handle: ResizeHandle,
        position: Point<Pixels>,
        /// 原图尺寸（读不到时 `None`）
        natural: Option<(u32, u32)>,
    },
    /// 拖动中：`frame` 是图片框当前的实际大小（被容器宽度限制时比请求的小）；`free` = 按住 Shift
    Move {
        key: usize,
        position: Point<Pixels>,
        frame: Size<Pixels>,
        free: bool,
    },
    /// 拖动中：图片所在行的宽度（图片 `max-width: 100%` 的基准，即能拉到的最大宽度）
    Container { key: usize, width: f32 },
    /// 松手（也可能来自没按下手柄的那几张图片：按 `key` 与当前会话核对）
    End { key: usize },
    /// 双击手柄：还原原始尺寸
    Reset { image: BlockImage },
}

pub type ImageResizeHandler = Rc<dyn Fn(ImageResizeEvent, &mut Window, &mut App)>;

/// 编辑排版里可调整图片大小
#[derive(Clone)]
pub struct ImageResize {
    /// 正在调整的图片（`key`）与实时显示尺寸
    pub active: Option<(usize, f32, f32)>,
    pub handler: ImageResizeHandler,
}

/// 拖动手柄时显示在光标处的视图（什么都不画）
struct EmptyDragView;

impl Render for EmptyDragView {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

/// 只读 Markdown 视图
#[derive(IntoElement)]
pub struct MarkdownView {
    id: SharedString,
    source: SharedString,
    images_dir: Arc<PathBuf>,
    readonly: bool,
    selectable: bool,
    on_image_click: Option<ImageClickHandler>,
    image_resize: Option<ImageResize>,
    style: StyleRefinement,
}

/// 渲染上下文（一次渲染内共享）
#[derive(Clone)]
struct Ctx {
    base_id: SharedString,
    images_dir: Arc<PathBuf>,
    readonly: bool,
    selectable: bool,
    image_paths: Rc<Vec<PathBuf>>,
    on_image_click: Option<ImageClickHandler>,
    image_resize: Option<ImageResize>,
}

impl MarkdownView {
    /// `id` 在窗口内唯一且稳定（每个块的行内 TextView 由它派生状态键）
    pub fn new(
        id: impl Into<SharedString>,
        source: impl Into<SharedString>,
        images_dir: Arc<PathBuf>,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            images_dir,
            readonly: true,
            selectable: true,
            on_image_click: None,
            image_resize: None,
            style: StyleRefinement::default(),
        }
    }

    /// 链接点击语义：`true`（默认）只读排版普通单击即打开；`false` 编辑排版只有 Ctrl/Cmd+单击才打开
    pub fn readonly(mut self, readonly: bool) -> Self {
        self.readonly = readonly;
        self
    }

    /// 文字是否可选择（默认 true）
    pub fn selectable(mut self, selectable: bool) -> Self {
        self.selectable = selectable;
        self
    }

    /// 点击图片（打开预览）
    pub fn on_image_click(
        mut self,
        handler: impl Fn(Vec<PathBuf>, usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_image_click = Some(Rc::new(handler));
        self
    }

    /// 编辑排版：内联图片悬停时显示调整大小的手柄（见 [`ImageResizeEvent`]）
    pub fn image_resize(mut self, resize: ImageResize) -> Self {
        self.image_resize = Some(resize);
        self
    }
}

impl Styled for MarkdownView {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for MarkdownView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let blocks = parsed(&self.source);
        let ctx = Ctx {
            base_id: self.id.clone(),
            image_paths: Rc::new(block_image_paths(&blocks, &self.images_dir)),
            images_dir: self.images_dir,
            readonly: self.readonly,
            selectable: self.selectable,
            on_image_click: self.on_image_click,
            image_resize: self.image_resize,
        };
        let mut image_counter = 0usize;
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .text_color(hex(editor_theme::MD_TEXT))
            .text_size(px(14.))
            .refine_style(&self.style)
            .children(render_blocks(&blocks, &ctx, "b", false, &mut image_counter))
    }
}

/// 一串块：相邻块的外边距取较大值；`inner`（列表项、引用里）时首尾外边距透出到外层，不画在这里
fn render_blocks(
    blocks: &[Block],
    ctx: &Ctx,
    path: &str,
    inner: bool,
    image_counter: &mut usize,
) -> Vec<AnyElement> {
    let mut out = Vec::with_capacity(blocks.len());
    let mut prev_bottom: Option<f32> = None;
    for (ix, block) in blocks.iter().enumerate() {
        let (top, bottom) = block.margins();
        let gap = match prev_bottom {
            Some(prev) => prev.max(top),
            None if inner => 0.,
            None => top,
        };
        let element = render_block(block, ctx, &format!("{path}{ix}"), image_counter);
        out.push(div().w_full().pt(px(gap)).child(element).into_any_element());
        prev_bottom = Some(bottom);
    }
    if !inner && let Some(bottom) = prev_bottom {
        out.push(div().h(px(bottom)).into_any_element());
    }
    out
}

fn text_view_style(foreground: Hsla) -> TextViewStyle {
    let accent = hex(editor_theme::MD_ACCENT);
    TextViewStyle::default()
        .with_foreground(foreground)
        .with_muted_foreground(hex(editor_theme::MD_QUOTE_TEXT))
        .with_link(foreground)
        .with_selection(theme::rgba_css(59, 130, 246, 0.3))
        .with_code_background(hex(editor_theme::MD_CODE_BG))
        .with_border(hex(editor_theme::MD_RULE))
        .with_paragraph_gap(rems(0.))
        .with_inline_code(HighlightStyle {
            color: Some(accent),
            background_color: Some(hex(editor_theme::MD_CODE_BG)),
            ..Default::default()
        })
}

/// 一段行内内容（嵌套的 TextView，链接走白名单、图片映射到本机）
fn inline_view(ctx: &Ctx, path: &str, markdown: &str, foreground: Hsla) -> TextView {
    let images_dir = ctx.images_dir.clone();
    let readonly = ctx.readonly;
    TextView::markdown(
        SharedString::from(format!("{}/{path}", ctx.base_id)),
        SharedString::from(markdown.to_string()),
    )
    .style(text_view_style(foreground))
    .selectable(ctx.selectable)
    .image_source(move |uri: &SharedUri| {
        match image_ref::resolve_image_path(uri.as_ref(), &images_dir) {
            Some(path) => ImageSource::from(path),
            None => ImageSource::from(uri.clone()),
        }
    })
    .on_link_click(move |url, event, _window, cx| {
        // 中键 / 右键：旧版一律 preventDefault，不导航
        if !event.standard_click() {
            return;
        }
        let modifiers = event.modifiers();
        file_link::handle_link_click(
            url.as_ref(),
            readonly,
            modifiers.control || modifiers.platform,
            cx,
        );
    })
}

fn render_block(block: &Block, ctx: &Ctx, path: &str, image_counter: &mut usize) -> AnyElement {
    let text = hex(editor_theme::MD_TEXT);
    match block {
        Block::Paragraph(markdown) => div()
            .w_full()
            .text_size(px(editor_theme::MD_P_SIZE))
            .line_height(px(editor_theme::MD_P_LINE))
            .child(inline_view(ctx, path, markdown, text))
            .into_any_element(),
        Block::RawText(raw) => {
            let mut escaped = String::new();
            escape_text(raw, &mut escaped);
            div()
                .w_full()
                .text_size(px(editor_theme::MD_P_SIZE))
                .line_height(px(editor_theme::MD_P_LINE))
                .child(inline_view(ctx, path, &escaped, text))
                .into_any_element()
        }
        Block::EmptyLine => div()
            .w_full()
            .h(px(editor_theme::MD_P_LINE))
            .into_any_element(),
        Block::Heading(level, markdown) => {
            let (size, line, _, _) = heading_metrics(*level);
            let weight = match level {
                1 => FontWeight::BOLD,
                2 => FontWeight::SEMIBOLD,
                // CSS 500：Segoe UI / 微软雅黑没有 500，实际渲染为 Regular
                _ => theme::WEIGHT_MEDIUM,
            };
            div()
                .w_full()
                .text_size(px(size))
                .line_height(relative(line))
                .font_weight(weight)
                .child(inline_view(ctx, path, markdown, text))
                .into_any_element()
        }
        Block::Quote(children) => {
            let quote_text = hex(editor_theme::MD_QUOTE_TEXT);
            let inner = ctx.clone();
            div()
                .w_full()
                .flex()
                .flex_col()
                .border_l(px(3.))
                .border_color(hex(editor_theme::MD_QUOTE_BORDER))
                .pl(px(12.))
                .font(editor_theme::serif_font())
                .text_color(quote_text)
                .children(render_quote_children(
                    children,
                    &inner,
                    &format!("{path}q"),
                    image_counter,
                ))
                .into_any_element()
        }
        Block::List(list) => render_list(list, ctx, path, image_counter),
        Block::Code(code) => div()
            .w_full()
            .px(px(16.))
            .py(px(12.))
            .rounded(px(8.))
            .bg(hex(editor_theme::MD_PRE_BG))
            .text_color(hex(editor_theme::MD_PRE_TEXT))
            .font(editor_theme::mono_font())
            .text_size(px(editor_theme::MD_PRE_SIZE))
            .line_height(px(editor_theme::MD_PRE_LINE))
            .child(
                gpui_kit::base::SelectableText::new(
                    SharedString::from(format!("{}/{path}", ctx.base_id)),
                    SharedString::from(code.clone()),
                )
                .selection_color(theme::rgba_css(255, 255, 255, 0.25)),
            )
            .into_any_element(),
        Block::Table(table) => render_table(table, ctx, path),
        Block::Rule => div()
            .w_full()
            .h(px(1.))
            .bg(hex(editor_theme::MD_RULE))
            .into_any_element(),
        Block::Images(images) => render_images(images, ctx, path, image_counter),
        Block::Mixed(parts) => render_mixed(parts, ctx, path, text, image_counter),
    }
}

/// 引用里的块：与普通块相同，但段落字色是引用色
fn render_quote_children(
    blocks: &[Block],
    ctx: &Ctx,
    path: &str,
    image_counter: &mut usize,
) -> Vec<AnyElement> {
    let quote_text = hex(editor_theme::MD_QUOTE_TEXT);
    let mut out = Vec::with_capacity(blocks.len());
    let mut prev_bottom: Option<f32> = None;
    for (ix, block) in blocks.iter().enumerate() {
        let (top, bottom) = block.margins();
        let gap = prev_bottom.map_or(0., |prev| prev.max(top));
        let child_path = format!("{path}{ix}");
        let element = match block {
            Block::Paragraph(markdown) => div()
                .w_full()
                .text_size(px(editor_theme::MD_P_SIZE))
                .line_height(px(editor_theme::MD_P_LINE))
                .child(inline_view(ctx, &child_path, markdown, quote_text))
                .into_any_element(),
            Block::Mixed(parts) => render_mixed(parts, ctx, &child_path, quote_text, image_counter),
            other => render_block(other, ctx, &child_path, image_counter),
        };
        out.push(div().w_full().pt(px(gap)).child(element).into_any_element());
        prev_bottom = Some(bottom);
    }
    out
}

fn render_list(list: &MdList, ctx: &Ctx, path: &str, image_counter: &mut usize) -> AnyElement {
    let accent = hex(editor_theme::MD_ACCENT);
    let mut items = Vec::with_capacity(list.items.len());
    let mut prev_bottom: Option<f32> = None;
    for (ix, item) in list.items.iter().enumerate() {
        let (top, bottom) = item_margins(item);
        let gap = prev_bottom.map_or(0., |prev| prev.max(top));
        let item_path = format!("{path}l{ix}.");
        let line = item.blocks.first().map_or(editor_theme::MD_P_LINE, |b| {
            b.first_line_height(&ctx.images_dir)
        });
        let marker: AnyElement = match item.checked {
            Some(checked) => task_box(checked).into_any_element(),
            None if list.ordered => div()
                .absolute()
                .top_0()
                .left(px(-editor_theme::MD_LIST_INDENT))
                .w(px(editor_theme::MD_LIST_INDENT - 4.))
                .h(px(line))
                .flex()
                .items_center()
                .justify_end()
                .text_size(px(editor_theme::MD_MARKER_SIZE))
                .text_color(accent)
                .whitespace_nowrap()
                .child(format!("{}.", list.start as usize + ix))
                .into_any_element(),
            None => div()
                .absolute()
                .top_0()
                .left(px(-editor_theme::MD_LIST_INDENT))
                .w(px(editor_theme::MD_LIST_INDENT - 14. + 2.5))
                .h(px(line))
                .flex()
                .items_center()
                .justify_end()
                .child(div().size(px(5.)).rounded_full().bg(accent))
                .into_any_element(),
        };
        items.push(
            div()
                .w_full()
                .pt(px(gap))
                .child(
                    div()
                        .relative()
                        .w_full()
                        .flex()
                        .flex_col()
                        .child(marker)
                        .children(render_blocks(
                            &item.blocks,
                            ctx,
                            &item_path,
                            true,
                            image_counter,
                        )),
                )
                .into_any_element(),
        );
        prev_bottom = Some(bottom);
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .pl(px(editor_theme::MD_LIST_INDENT))
        .children(items)
        .into_any_element()
}

/// 任务项方框：13×13、1.5px `#94A3B8` 边、圆角 3、白底；勾选时底 / 边 `#3B82F6` + 白色对勾。
/// 位置 `left: -20px; top: 0.4em`（li 14px → 5.6）
fn task_box(checked: bool) -> impl IntoElement {
    let blue = hex(editor_theme::MD_QUOTE_BORDER);
    div()
        .absolute()
        .left(px(-20.))
        .top(px(5.6))
        .size(px(13.))
        .rounded(px(3.))
        .border(px(1.5))
        .border_color(if checked {
            blue
        } else {
            hex(editor_theme::MD_TASK_BORDER)
        })
        .bg(if checked { blue } else { hex(0xFFFFFF) })
        .when(checked, |this| {
            this.flex()
                .items_center()
                .justify_center()
                .child(crate::ui::icon::icon(
                    crate::assets::EpIcon::Check,
                    9.,
                    hex(0xFFFFFF),
                ))
        })
}

fn render_table(table: &MdTable, ctx: &Ctx, path: &str) -> AnyElement {
    let columns = table.rows.iter().map(Vec::len).max().unwrap_or(0).max(1);
    let row_count = table.rows.len();
    let radius = px(7.);
    let rows = table.rows.iter().enumerate().map(|(r, row)| {
        let header = r == 0;
        let last_row = r + 1 == row_count;
        let cells = (0..columns).map(|c| {
            let markdown = row.get(c).map(String::as_str).unwrap_or("");
            let align = table.aligns.get(c).copied().unwrap_or_default();
            let bg = if header {
                hex(editor_theme::MD_TH_BG)
            } else if c % 2 == 0 {
                hex(editor_theme::MD_TD_ODD_BG)
            } else {
                hex(0xFFFFFF)
            };
            let last_col = c + 1 == columns;
            div()
                .flex_1()
                .flex_basis(px(0.))
                .min_w_0()
                .px(px(24.))
                .py(px(12.))
                .bg(bg)
                .when(!last_col, |this| {
                    this.border_r_1().border_color(hex(editor_theme::MD_RULE))
                })
                .when(header && c == 0, |this| this.rounded_tl(radius))
                .when(header && last_col, |this| this.rounded_tr(radius))
                .when(last_row && c == 0, |this| this.rounded_bl(radius))
                .when(last_row && last_col, |this| this.rounded_br(radius))
                .text_size(px(editor_theme::MD_P_SIZE))
                .line_height(px(editor_theme::MD_P_LINE))
                .when(header, |this| this.font_weight(theme::WEIGHT_SEMIBOLD))
                .map(|this| match align {
                    ColumnAlign::Left => this,
                    ColumnAlign::Center => this.text_center(),
                    ColumnAlign::Right => this.text_right(),
                })
                .child(inline_view(
                    ctx,
                    &format!("{path}t{r}.{c}"),
                    markdown,
                    hex(editor_theme::MD_TEXT),
                ))
        });
        div()
            .w_full()
            .flex()
            .when(!last_row, |this| {
                this.border_b_1().border_color(hex(editor_theme::MD_RULE))
            })
            .children(cells)
    });
    div()
        .w_full()
        .flex()
        .flex_col()
        .rounded(px(8.))
        .border_1()
        .border_color(hex(0xFFFFFF))
        .bg(hex(0xFFFFFF))
        .shadow(md_shadow())
        .children(rows)
        .into_any_element()
}

/// theme-nord `shadow-md`：`0 4px 6px -1px rgba(0,0,0,.1), 0 2px 4px -2px rgba(0,0,0,.1)`
fn md_shadow() -> Vec<BoxShadow> {
    vec![
        theme::shadow(0., 4., 6., -1., theme::rgba_css(0, 0, 0, 0.1)),
        theme::shadow(0., 2., 4., -2., theme::rgba_css(0, 0, 0, 0.1)),
    ]
}

/// 一张自己排版的图片：显示尺寸（[`display_size`]）、最大宽度 100%（按宽高比缩小）、圆角 6、阴影；悬停变淡，
/// 单击打开预览（不冒泡：编辑排版里点图片只打开预览、不切到源码）。编辑排版（`ctx.image_resize`）里内联图片悬停时
/// 显示描边与调整大小的手柄（[`resize_decorations`]）。本机图片按出现顺序编号（`image_counter`，与
/// [`block_image_paths`] 顺序一致）
fn image_frame(image: &BlockImage, ctx: &Ctx, id: &str, image_counter: &mut usize) -> AnyElement {
    let local = image_ref::resolve_image_path(&image.url, &ctx.images_dir);
    let index = local.as_ref().map(|_| {
        let index = *image_counter;
        *image_counter += 1;
        index
    });
    let source: ImageSource = match &local {
        Some(path) => ImageSource::from(path.clone()),
        None => ImageSource::from(image.url.clone()),
    };
    let natural = local.as_deref().and_then(image_size);
    let frame_id = format!("{}/{id}", ctx.base_id);
    let resize = ctx
        .image_resize
        .clone()
        .zip(image.span.as_ref().map(|span| span.range.start));
    // 正在拖动手柄的这张图片按实时尺寸显示
    let live = resize
        .as_ref()
        .and_then(|(resize, key)| resize.active.filter(|(k, _, _)| k == key))
        .map(|(_, w, h)| (w, h));
    let shown = live
        .map(|(w, h)| (w, Some(h)))
        .or_else(|| display_size(image.size, natural));
    let alt = SharedString::from(image.alt.clone());
    // 图片铺满图片框、按框的宽高拉伸（`|宽x高` 与原图比例不同时就是要改变比例）。宽高比必须显式给：GPUI 的 img 没设时
    // 用原图的比例，taffy 按它算出的自动最小高度会让图片撑出图片框、压住下一段（比原图更宽更矮的 `|300x100`）
    let picture = img(source)
        .size_full()
        .object_fit(ObjectFit::Fill)
        .map(|this| match shown {
            Some((w, Some(h))) if w > 0. && h > 0. => this.aspect_ratio(w / h),
            _ => this,
        })
        .rounded(px(6.))
        .with_fallback(move || {
            div()
                .text_size(px(14.))
                .text_color(theme::light().text_tertiary)
                .child(alt.clone())
                .into_any_element()
        });
    let mut frame = div()
        .id(SharedString::from(frame_id.clone()))
        .relative()
        .flex_none()
        .max_w_full()
        .rounded(px(6.))
        .shadow(md_shadow())
        .cursor(CursorStyle::PointingHand)
        .hover(|s| {
            s.opacity(0.85).shadow(vec![theme::shadow(
                0.,
                2.,
                8.,
                0.,
                theme::rgba_css(0, 0, 0, 0.15),
            )])
        })
        .map(|this| match shown {
            Some((w, Some(h))) if w > 0. && h > 0. => this.w(px(w)).aspect_ratio(w / h),
            Some((w, None)) if w > 0. => this.w(px(w)),
            _ => this,
        })
        .child(picture);
    if let Some((resize, key)) = resize {
        let owner = ctx.base_id.clone();
        let handler = resize.handler.clone();
        let group = SharedString::from(format!("{frame_id}-g"));
        frame = frame
            .group(group.clone())
            // 拖动手柄时每次移动都报给编辑器（捕获阶段、不看悬停；图片框的大小用来判断是否被容器宽度限制）
            .on_drag_move::<ImageResizeDrag>(
                move |event: &DragMoveEvent<ImageResizeDrag>, window, cx| {
                    let drag = event.drag(cx);
                    if drag.key != key || drag.owner != owner {
                        return;
                    }
                    if event.event.pressed_button != Some(MouseButton::Left) {
                        // 没收到松手（捕获被拿走）
                        cx.stop_active_drag(window);
                        handler(ImageResizeEvent::End { key }, window, cx);
                        return;
                    }
                    handler(
                        ImageResizeEvent::Move {
                            key,
                            position: event.event.position,
                            frame: event.bounds.size,
                            free: event.event.modifiers.shift,
                        },
                        window,
                        cx,
                    );
                },
            )
            .children(resize_decorations(
                image,
                key,
                natural,
                ctx,
                &resize.handler,
                &group,
                &frame_id,
                live,
            ));
    }
    match (ctx.on_image_click.clone(), index) {
        (Some(handler), Some(index)) => {
            let paths = ctx.image_paths.clone();
            frame
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    handler(paths.to_vec(), index, window, cx)
                })
                .into_any_element()
        }
        _ => frame.into_any_element(),
    }
}

/// 拖动手柄时 GPUI 拖动的载荷：哪个 Markdown 视图（`base_id`）里的哪张图片
#[derive(Clone)]
struct ImageResizeDrag {
    owner: SharedString,
    key: usize,
}

/// 编辑排版里一张内联图片的描边、三个手柄（右边 / 底边 / 右下角）与拖动中的尺寸标签。
/// 描边与手柄平时透明、悬停图片时显示，拖动中一直显示。手柄按下不冒泡（图片框不会把这次按下当成单击，
/// 编辑器根元素也不会切到源码）；双击手柄还原原始尺寸
#[allow(clippy::too_many_arguments)]
fn resize_decorations(
    image: &BlockImage,
    key: usize,
    natural: Option<(u32, u32)>,
    ctx: &Ctx,
    handler: &ImageResizeHandler,
    group: &SharedString,
    frame_id: &str,
    live: Option<(f32, f32)>,
) -> Vec<AnyElement> {
    let accent = hex(editor_theme::MD_IMAGE_HANDLE);
    let active = live.is_some();
    let reveal = |this: gpui_kit::Div| {
        this.when(!active, |this| {
            this.opacity(0.)
                .group_hover(group.clone(), |s| s.opacity(1.))
        })
    };
    let mut out = Vec::with_capacity(5);
    out.push(
        reveal(
            div()
                .absolute()
                .inset_0()
                .rounded(px(6.))
                .border(px(1.5))
                .border_color(accent),
        )
        .into_any_element(),
    );
    let edge = editor_theme::MD_IMAGE_EDGE_HIT;
    let corner = editor_theme::MD_IMAGE_CORNER_HIT;
    for handle in ResizeHandle::ALL {
        let base = div()
            .id(SharedString::from(format!("{frame_id}-h{}", handle.key())))
            .absolute()
            .flex()
            .items_center()
            .justify_center();
        let (base, mark, cursor) = match handle {
            ResizeHandle::Right => (
                base.top_0().bottom(px(corner)).right_0().w(px(edge)),
                div()
                    .w(px(4.))
                    .h(px(editor_theme::MD_IMAGE_PILL))
                    .max_h_full()
                    .rounded_full()
                    .bg(accent),
                CursorStyle::ResizeLeftRight,
            ),
            ResizeHandle::Bottom => (
                base.left_0().right(px(corner)).bottom_0().h(px(edge)),
                div()
                    .h(px(4.))
                    .w(px(editor_theme::MD_IMAGE_PILL))
                    .max_w_full()
                    .rounded_full()
                    .bg(accent),
                CursorStyle::ResizeUpDown,
            ),
            ResizeHandle::Corner => (
                base.right_0().bottom_0().size(px(corner)),
                div()
                    .size(px(10.))
                    .rounded(px(2.))
                    .border(px(1.5))
                    .border_color(accent)
                    .bg(hex(0xFFFFFF)),
                CursorStyle::ResizeUpLeftDownRight,
            ),
        };
        let down = handler.clone();
        let up = handler.clone();
        let up_out = handler.clone();
        let pressed = image.clone();
        out.push(
            base.cursor(cursor)
                .tooltip(native("拖动调整大小，双击恢复原始大小"))
                .on_mouse_down(
                    MouseButton::Left,
                    move |event: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        let event = if event.click_count >= 2 {
                            ImageResizeEvent::Reset {
                                image: pressed.clone(),
                            }
                        } else {
                            ImageResizeEvent::Start {
                                image: pressed.clone(),
                                handle,
                                position: event.position,
                                natural,
                            }
                        };
                        down(event, window, cx);
                    },
                )
                .on_drag(
                    ImageResizeDrag {
                        owner: ctx.base_id.clone(),
                        key,
                    },
                    |_, _, _, cx| cx.new(|_| EmptyDragView),
                )
                .on_mouse_up(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    up(ImageResizeEvent::End { key }, window, cx);
                })
                .on_mouse_up_out(MouseButton::Left, move |_, window, cx| {
                    up_out(ImageResizeEvent::End { key }, window, cx)
                })
                .child(reveal(mark))
                .into_any_element(),
        );
    }
    if let Some((w, h)) = live {
        out.push(
            div()
                .absolute()
                .top(px(6.))
                .left(px(6.))
                .px(px(6.))
                .rounded(px(4.))
                .bg(theme::hexa(editor_theme::MD_IMAGE_BADGE_BG))
                .text_color(hex(0xFFFFFF))
                .text_size(px(12.))
                .line_height(px(18.))
                .whitespace_nowrap()
                .child(format!("{} × {}", w.round(), h.round()))
                .into_any_element(),
        );
    }
    out
}

/// 一行图片 / 文字：放不下时换行；各项垂直居中（旧版 img 是 `inline-block` + Tailwind preflight 的
/// `vertical-align: middle`，行高被图片撑开、文字落在图片高度中间）。
///
/// 编辑排版里拖动这一行里某张图片（`keys` = 行内可调整图片的源码起点）的手柄时，把行宽报给编辑器
/// （[`ImageResizeEvent::Container`]）：行宽就是图片 `max-width: 100%` 的基准、不随图片大小变化，拖动的第一次移动就能拿到
fn inline_row(items: Vec<AnyElement>, ctx: &Ctx, keys: Vec<usize>) -> gpui_kit::Div {
    let row = div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .children(items);
    match ctx.image_resize.as_ref() {
        Some(resize) if !keys.is_empty() => {
            let handler = resize.handler.clone();
            let owner = ctx.base_id.clone();
            row.on_drag_move::<ImageResizeDrag>(
                move |event: &DragMoveEvent<ImageResizeDrag>, window, cx| {
                    let drag = event.drag(cx);
                    if drag.owner != owner || !keys.contains(&drag.key) {
                        return;
                    }
                    let key = drag.key;
                    handler(
                        ImageResizeEvent::Container {
                            key,
                            width: f32::from(event.bounds.size.width),
                        },
                        window,
                        cx,
                    );
                },
            )
        }
        _ => row,
    }
}

/// 一张图片能在界面上调整大小时的标识（源码起点）
fn resize_key(image: &BlockImage, ctx: &Ctx) -> Option<usize> {
    ctx.image_resize.as_ref()?;
    image.span.as_ref().map(|span| span.range.start)
}

fn render_images(
    images: &[BlockImage],
    ctx: &Ctx,
    path: &str,
    image_counter: &mut usize,
) -> AnyElement {
    let elements = images
        .iter()
        .enumerate()
        .map(|(ix, image)| image_frame(image, ctx, &format!("{path}i{ix}"), image_counter))
        .collect();
    let keys = images
        .iter()
        .filter_map(|image| resize_key(image, ctx))
        .collect();
    inline_row(elements, ctx, keys).into_any_element()
}

/// 含图片又含文字的段落（[`Block::Mixed`]）：按顶层硬换行分行，每行里文字段（TextView，按内容宽度、放不下时
/// 占满一行折行）与图片（同块级图片）依次排开、放不下就换行、垂直居中。
///
/// 与旧版的差异：旧版是真正的行内排版——图片前后的文字会填满图片所在行的剩余宽度再折到下一行；这里文字段是
/// 一整块，长到一行放不下时独占一行（图片排到它下面一行）
fn render_mixed(
    parts: &[InlinePart],
    ctx: &Ctx,
    path: &str,
    color: Hsla,
    image_counter: &mut usize,
) -> AnyElement {
    // 每行：(元素, 行内可调整图片的 key)
    let mut rows: Vec<(Vec<AnyElement>, Vec<usize>)> = vec![(Vec::new(), Vec::new())];
    for (ix, part) in parts.iter().enumerate() {
        match part {
            InlinePart::Break => rows.push((Vec::new(), Vec::new())),
            InlinePart::Text(markdown) => {
                let text = div()
                    .min_w_0()
                    .child(inline_view(ctx, &format!("{path}s{ix}"), markdown, color))
                    .into_any_element();
                rows.last_mut().expect("至少一行").0.push(text);
            }
            InlinePart::Image(image) => {
                let frame = image_frame(image, ctx, &format!("{path}i{ix}"), image_counter);
                let row = rows.last_mut().expect("至少一行");
                row.0.push(frame);
                row.1.extend(resize_key(image, ctx));
            }
        }
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .text_size(px(editor_theme::MD_P_SIZE))
        .line_height(px(editor_theme::MD_P_LINE))
        // 空行（连续硬换行）也有一行高
        .children(
            rows.into_iter().map(|(items, keys)| {
                inline_row(items, ctx, keys).min_h(px(editor_theme::MD_P_LINE))
            }),
        )
        .into_any_element()
}

//! Markdown 只读渲染（旧 `MarkdownEditor.vue` 的只读排版：Milkdown + theme-nord + scoped 覆盖）。
//!
//! 排版分两层：
//! - **块级**（段落、标题、列表、任务项、引用、代码块、表格、分隔线、图片、`<br />` 空行）由本模块
//!   按 `ui-editor-windows.md` §5.5 的实际生效值自己排（外边距按 CSS 规则折叠：相邻块取较大值，列表项 / 引用
//!   的首尾子块外边距透出到外层；Markdown 根是 flex 子项，首块上边距与末块下边距留在容器内）；
//! - **行内**（粗体、斜体、删除线、行内代码、链接、硬换行）交给 gpui-base 的 `TextView`：每个块的行内
//!   内容由 mdast 重新序列化成"全标点转义"的 Markdown（与原文的分隔符规则等价，块语法不会被误识别），
//!   保留文字选择 / 复制。
//!
//! 图片：`minitodo-image://<name>` 与旧 asset URL 映射到本机 images 目录（`util::image_ref`）；按原始尺寸显示
//! （max-width 100%，超宽按宽高比缩小）、圆角 6、阴影，点击打开预览（[`MarkdownView::on_image_click`]）。
//! 段落顶层的图片都由本模块排：只有图片的段落是 [`Block::Images`]；与文字同段的拆成"文字段 / 图片 / 文字段"
//! （[`Block::Mixed`]），同一行里依次排开、放不下换行、垂直居中——旧版 img 是 `inline-block` + `vertical-align:
//! middle`，行高被图片撑开（TextView 自己画行内图片时会把它缩成一行字高，不能交给它）。
//! 链接：一律走 `util::file_link` 白名单（必须接管 `on_link_click`，否则 TextView 默认直接 `open_url`）。
//!
//! 与旧版的差异：行内代码的细节由 TextView 绘制（代码字号 0.875em、内边距 4px，旧版 0.9em / 6px）；与图片同段的
//! 文字段是一整块（长到一行放不下时独占一行，旧版会接着图片所在行的剩余宽度排）；链接 / 标题 / 表格里的图片仍由 TextView
//! 按一行字高绘制且不能点开预览；选择只在单个块内（不能跨段落拖选）；任务项方框只显示（旧版同样不能点击切换）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::base::{TextView, TextViewStyle};
use gpui_kit::component::StyledExt as _;
use gpui_kit::{
    AnyElement, App, BoxShadow, CursorStyle, FontWeight, HighlightStyle, Hsla, ImageSource,
    InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce, SharedString, SharedUri,
    StatefulInteractiveElement as _, StyleRefinement, Styled, StyledImage as _, Window, div, img,
    prelude::FluentBuilder as _, px, relative, rems,
};
use markdown::mdast::{self, AlignKind, Node};

use crate::theme::{self, editor_theme, hex};
use crate::util::file_link;
use crate::util::image_ref;

// ---------------------------------------------------------------------------
// 块模型
// ---------------------------------------------------------------------------

/// 列对齐（GFM 对齐行；默认左）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColumnAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// 自己排版的一张图片（独占段落的，或从含文字的段落里拆出来的）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockImage {
    pub url: String,
    pub alt: String,
}

/// 含图片又含文字的段落里的一段（见 [`Block::Mixed`]）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlinePart {
    /// 行内 Markdown；与图片相邻一侧的空白换成一个不换行空格（单独排版的文字段首尾空白会被 Markdown 解析吃掉）
    Text(String),
    Image(BlockImage),
    /// 段落顶层的硬换行：另起一行
    Break,
}

/// 列表
#[derive(Debug, Clone, PartialEq)]
pub struct MdList {
    pub ordered: bool,
    pub start: u32,
    pub items: Vec<MdItem>,
}

/// 列表项（`checked` 为任务项）
#[derive(Debug, Clone, PartialEq)]
pub struct MdItem {
    pub checked: Option<bool>,
    pub blocks: Vec<Block>,
}

/// 表格（`rows[0]` 为表头；单元格是行内 Markdown）
#[derive(Debug, Clone, PartialEq)]
pub struct MdTable {
    pub aligns: Vec<ColumnAlign>,
    pub rows: Vec<Vec<String>>,
}

/// 块
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// 段落（行内 Markdown）
    Paragraph(String),
    /// 只含图片的段落
    Images(Vec<BlockImage>),
    /// 含图片又含文字的段落：图片拆出来按原始尺寸排（TextView 会把行内图片缩成一行字高），文字段交给 TextView
    Mixed(Vec<InlinePart>),
    /// Milkdown 保存空段落用的独占一行 `<br />`：显示为一个空行
    EmptyLine,
    Heading(u8, String),
    Quote(Vec<Block>),
    List(MdList),
    Code(String),
    Table(MdTable),
    Rule,
    /// 其它 HTML 块：原样显示为文字
    RawText(String),
}

impl Block {
    /// 折叠后的 (上外边距, 下外边距)
    fn margins(&self) -> (f32, f32) {
        match self {
            Block::Paragraph(_)
            | Block::Images(_)
            | Block::Mixed(_)
            | Block::EmptyLine
            | Block::RawText(_) => (editor_theme::MD_P_MARGIN, editor_theme::MD_P_MARGIN),
            Block::Heading(level, _) => {
                let (_, _, top, bottom) = heading_metrics(*level);
                (top, bottom)
            }
            Block::Quote(children) => {
                let (first, last) = edge_margins(children);
                (
                    editor_theme::MD_QUOTE_MARGIN.max(first),
                    editor_theme::MD_QUOTE_MARGIN.max(last),
                )
            }
            Block::List(list) => {
                let first = list.items.first().map_or(0., |item| item_margins(item).0);
                let last = list.items.last().map_or(0., |item| item_margins(item).1);
                (
                    editor_theme::MD_LIST_MARGIN.max(first),
                    editor_theme::MD_LIST_MARGIN.max(last),
                )
            }
            Block::Code(_) => (0., 0.),
            Block::Table(_) => (editor_theme::MD_TABLE_MARGIN, editor_theme::MD_TABLE_MARGIN),
            Block::Rule => (editor_theme::MD_HR_MARGIN, editor_theme::MD_HR_MARGIN),
        }
    }

    /// 首行行高（列表标记按它垂直居中）。图片所在的行被图片撑高（按原始高度估算；超宽被缩小的图片会偏高）
    fn first_line_height(&self, images_dir: &Path) -> f32 {
        let tallest = |images: &mut dyn Iterator<Item = &BlockImage>| {
            images
                .filter_map(|image| image_ref::resolve_image_path(&image.url, images_dir))
                .filter_map(|path| image_size(&path))
                .map(|(_, h)| h as f32)
                .fold(editor_theme::MD_P_LINE, f32::max)
        };
        match self {
            Block::Heading(level, _) => {
                let (size, line, _, _) = heading_metrics(*level);
                size * line
            }
            Block::Code(_) => editor_theme::MD_PRE_LINE + 24.,
            Block::Quote(children) => children
                .first()
                .map_or(editor_theme::MD_P_LINE, |b| b.first_line_height(images_dir)),
            Block::List(list) => list
                .items
                .first()
                .and_then(|item| item.blocks.first())
                .map_or(editor_theme::MD_P_LINE, |b| b.first_line_height(images_dir)),
            Block::Images(images) => tallest(&mut images.iter()),
            Block::Mixed(parts) => tallest(
                &mut parts
                    .iter()
                    .take_while(|part| !matches!(part, InlinePart::Break))
                    .filter_map(|part| match part {
                        InlinePart::Image(image) => Some(image),
                        _ => None,
                    }),
            ),
            _ => editor_theme::MD_P_LINE,
        }
    }
}

/// (字号, 行高倍数, 上外边距, 下外边距)
fn heading_metrics(level: u8) -> (f32, f32, f32, f32) {
    editor_theme::MD_HEADINGS[usize::from(level.clamp(1, 6)) - 1]
}

/// 子块的首块上边距、末块下边距（透出到外层）
fn edge_margins(blocks: &[Block]) -> (f32, f32) {
    (
        blocks.first().map_or(0., |b| b.margins().0),
        blocks.last().map_or(0., |b| b.margins().1),
    )
}

/// 列表项的 (上, 下) 外边距：li 自身 `margin-bottom: 4px` 与首尾子块折叠
fn item_margins(item: &MdItem) -> (f32, f32) {
    let (first, last) = edge_margins(&item.blocks);
    (first, editor_theme::MD_LI_MARGIN.max(last))
}

// ---------------------------------------------------------------------------
// 解析：mdast → 块模型
// ---------------------------------------------------------------------------

type Definitions = HashMap<String, (String, Option<String>)>;

/// 解析 Markdown 源码为块（GFM：表格、任务清单、删除线、自动链接）
pub fn parse_blocks(source: &str) -> Vec<Block> {
    let Ok(root) = markdown::to_mdast(source, &markdown::ParseOptions::gfm()) else {
        return vec![Block::RawText(source.to_string())];
    };
    let mut defs = Definitions::new();
    collect_definitions(&root, &mut defs);
    match &root {
        Node::Root(root) => convert_blocks(&root.children, &defs),
        other => convert_blocks(std::slice::from_ref(other), &defs),
    }
}

fn collect_definitions(node: &Node, defs: &mut Definitions) {
    if let Node::Definition(def) = node {
        defs.entry(def.identifier.clone())
            .or_insert_with(|| (def.url.clone(), def.title.clone()));
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_definitions(child, defs);
        }
    }
}

fn is_br_html(value: &str) -> bool {
    let v = value.trim().to_ascii_lowercase();
    let Some(rest) = v.strip_prefix("<br") else {
        return false;
    };
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('/').unwrap_or(rest).trim_start();
    rest == ">"
}

fn convert_blocks(nodes: &[Node], defs: &Definitions) -> Vec<Block> {
    let mut out = Vec::new();
    for node in nodes {
        match node {
            Node::Paragraph(p) => out.push(convert_paragraph(&p.children, defs)),
            Node::Heading(h) => out.push(Block::Heading(
                h.depth.clamp(1, 6),
                inline_markdown(&h.children, defs),
            )),
            Node::Blockquote(q) => out.push(Block::Quote(convert_blocks(&q.children, defs))),
            Node::List(list) => out.push(Block::List(MdList {
                ordered: list.ordered,
                start: list.start.unwrap_or(1),
                items: list
                    .children
                    .iter()
                    .filter_map(|child| match child {
                        Node::ListItem(item) => Some(MdItem {
                            checked: item.checked,
                            blocks: convert_blocks(&item.children, defs),
                        }),
                        _ => None,
                    })
                    .collect(),
            })),
            Node::Code(code) => out.push(Block::Code(code.value.clone())),
            Node::Math(math) => out.push(Block::Code(math.value.clone())),
            Node::Table(table) => out.push(convert_table(table, defs)),
            Node::ThematicBreak(_) => out.push(Block::Rule),
            Node::Html(html) => {
                if is_br_html(&html.value) {
                    out.push(Block::EmptyLine);
                } else {
                    out.push(Block::RawText(html.value.clone()));
                }
            }
            Node::FootnoteDefinition(def) => out.extend(convert_blocks(&def.children, defs)),
            Node::Definition(_) | Node::Yaml(_) | Node::Toml(_) => {}
            other => {
                if let Some(children) = other.children() {
                    out.extend(convert_blocks(children, defs));
                }
            }
        }
    }
    out
}

fn convert_paragraph(children: &[Node], defs: &Definitions) -> Block {
    // 只有 `<br />`：空行
    let meaningful: Vec<&Node> = children
        .iter()
        .filter(|n| !matches!(n, Node::Text(t) if t.value.trim().is_empty()))
        .collect();
    if meaningful.len() == 1
        && let Node::Html(html) = meaningful[0]
        && is_br_html(&html.value)
    {
        return Block::EmptyLine;
    }
    // 只有图片（中间允许空白 / 换行）：块级图片
    let mut images = Vec::new();
    let only_images = !meaningful.is_empty()
        && meaningful.iter().all(|n| match resolved_image(n, defs) {
            Some(image) => {
                images.push(image);
                true
            }
            None => matches!(n, Node::Break(_)),
        });
    if only_images && !images.is_empty() {
        return Block::Images(images);
    }
    if let Some(parts) = mixed_parts(children, defs) {
        return Block::Mixed(parts);
    }
    Block::Paragraph(inline_markdown(children, defs))
}

/// 段落顶层的图片（引用式图片要有定义；没有定义的按文字显示）
fn resolved_image(node: &Node, defs: &Definitions) -> Option<BlockImage> {
    match node {
        Node::Image(image) => Some(BlockImage {
            url: image.url.clone(),
            alt: image.alt.clone(),
        }),
        Node::ImageReference(r) => defs.get(&r.identifier).map(|(url, _)| BlockImage {
            url: url.clone(),
            alt: r.alt.clone(),
        }),
        _ => None,
    }
}

/// 不换行空格：文字段与图片之间的空格（旧版 `white-space: break-spaces` 照常显示这个空格）
const NBSP: char = '\u{a0}';

/// 含图片又含文字的段落 → 文字段 / 图片 / 硬换行；没有顶层图片返回 `None`
fn mixed_parts(children: &[Node], defs: &Definitions) -> Option<Vec<InlinePart>> {
    if !children.iter().any(|n| resolved_image(n, defs).is_some()) {
        return None;
    }
    let mut parts = Vec::new();
    let mut run: Vec<&Node> = Vec::new();
    for node in children {
        if let Some(image) = resolved_image(node, defs) {
            flush_text(&mut run, true, defs, &mut parts);
            parts.push(InlinePart::Image(image));
        } else if matches!(node, Node::Break(_)) {
            flush_text(&mut run, false, defs, &mut parts);
            parts.push(InlinePart::Break);
        } else {
            run.push(node);
        }
    }
    flush_text(&mut run, false, defs, &mut parts);
    Some(parts)
}

/// 一段连续的行内节点 → 文字段。首尾空白去掉；与图片相邻的一侧保留成一个不换行空格。
/// 只有空白时：夹在两张图片之间才保留（一个空格宽），否则丢掉
fn flush_text(
    run: &mut Vec<&Node>,
    before_image: bool,
    defs: &Definitions,
    parts: &mut Vec<InlinePart>,
) {
    if run.is_empty() {
        return;
    }
    let mut markdown = String::new();
    for node in run.drain(..) {
        write_inline(node, defs, &mut markdown);
    }
    let after_image = matches!(parts.last(), Some(InlinePart::Image(_)));
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        if after_image && before_image && !markdown.is_empty() {
            parts.push(InlinePart::Text(NBSP.to_string()));
        }
        return;
    }
    let mut text = String::with_capacity(trimmed.len() + 4);
    if after_image && markdown.starts_with(|c: char| c.is_whitespace()) {
        text.push(NBSP);
    }
    text.push_str(trimmed);
    if before_image && markdown.ends_with(|c: char| c.is_whitespace()) {
        text.push(NBSP);
    }
    parts.push(InlinePart::Text(text));
}

fn convert_table(table: &mdast::Table, defs: &Definitions) -> Block {
    let aligns = table
        .align
        .iter()
        .map(|a| match a {
            AlignKind::Center => ColumnAlign::Center,
            AlignKind::Right => ColumnAlign::Right,
            _ => ColumnAlign::Left,
        })
        .collect();
    let rows = table
        .children
        .iter()
        .filter_map(|row| match row {
            Node::TableRow(row) => Some(
                row.children
                    .iter()
                    .map(|cell| match cell {
                        Node::TableCell(cell) => inline_markdown(&cell.children, defs),
                        _ => String::new(),
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect();
    Block::Table(MdTable { aligns, rows })
}

/// 行内节点 → 等价的 Markdown（文字里的 ASCII 标点全部反斜杠转义：块语法不会被误识别，
/// 分隔符左右的"是否标点"不变，强调 / 删除线的成立条件与原文一致）
fn inline_markdown(nodes: &[Node], defs: &Definitions) -> String {
    let mut out = String::new();
    for node in nodes {
        write_inline(node, defs, &mut out);
    }
    out
}

fn escape_text(text: &str, out: &mut String) {
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
}

fn write_children(nodes: &[Node], defs: &Definitions, out: &mut String) {
    for node in nodes {
        write_inline(node, defs, out);
    }
}

fn link_destination(url: &str) -> String {
    if url.is_empty()
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '(' | ')' | '<' | '>'))
    {
        let inner: String = url
            .chars()
            .map(|c| match c {
                '<' => "%3C".to_string(),
                '>' => "%3E".to_string(),
                '\n' | '\r' => "%0A".to_string(),
                c => c.to_string(),
            })
            .collect();
        format!("<{inner}>")
    } else {
        url.to_string()
    }
}

fn link_title(title: &Option<String>) -> String {
    match title {
        Some(t) if !t.is_empty() => {
            format!(" \"{}\"", t.replace('\\', "\\\\").replace('"', "\\\""))
        }
        _ => String::new(),
    }
}

fn code_span(value: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in value.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    let pad = value.starts_with('`')
        || value.ends_with('`')
        || (value.starts_with(' ') && value.ends_with(' ') && !value.trim().is_empty());
    if pad {
        format!("{fence} {value} {fence}")
    } else {
        format!("{fence}{value}{fence}")
    }
}

fn write_inline(node: &Node, defs: &Definitions, out: &mut String) {
    match node {
        Node::Text(text) => escape_text(&text.value, out),
        Node::Strong(strong) => {
            out.push_str("**");
            write_children(&strong.children, defs, out);
            out.push_str("**");
        }
        Node::Emphasis(em) => {
            out.push('*');
            write_children(&em.children, defs, out);
            out.push('*');
        }
        Node::Delete(del) => {
            out.push_str("~~");
            write_children(&del.children, defs, out);
            out.push_str("~~");
        }
        Node::InlineCode(code) => out.push_str(&code_span(&code.value)),
        Node::InlineMath(math) => escape_text(&format!("${}$", math.value), out),
        Node::Break(_) => out.push_str("\\\n"),
        Node::Link(link) => {
            out.push('[');
            write_children(&link.children, defs, out);
            out.push_str("](");
            out.push_str(&link_destination(&link.url));
            out.push_str(&link_title(&link.title));
            out.push(')');
        }
        Node::LinkReference(r) => match defs.get(&r.identifier) {
            Some((url, title)) => {
                out.push('[');
                write_children(&r.children, defs, out);
                out.push_str("](");
                out.push_str(&link_destination(url));
                out.push_str(&link_title(title));
                out.push(')');
            }
            None => {
                out.push_str("\\[");
                write_children(&r.children, defs, out);
                out.push_str("\\]");
            }
        },
        Node::Image(image) => {
            out.push_str("![");
            escape_text(&image.alt, out);
            out.push_str("](");
            out.push_str(&link_destination(&image.url));
            out.push_str(&link_title(&image.title));
            out.push(')');
        }
        Node::ImageReference(r) => match defs.get(&r.identifier) {
            Some((url, title)) => {
                out.push_str("![");
                escape_text(&r.alt, out);
                out.push_str("](");
                out.push_str(&link_destination(url));
                out.push_str(&link_title(title));
                out.push(')');
            }
            None => escape_text(&format!("![{}]", r.alt), out),
        },
        Node::Html(html) => out.push_str(&html.value),
        Node::FootnoteReference(f) => escape_text(&format!("[^{}]", f.identifier), out),
        other => {
            if let Some(children) = other.children() {
                write_children(children, defs, out);
            }
        }
    }
}

thread_local! {
    /// 解析缓存（同一段源码在每次重绘时复用；只保留最近若干条）
    static PARSE_CACHE: RefCell<Vec<(SharedString, Rc<Vec<Block>>)>> = const { RefCell::new(Vec::new()) };
    /// 本机图片的原始尺寸（图片文件名唯一且不会被改写，读一次文件头即可）
    static IMAGE_SIZES: RefCell<HashMap<PathBuf, Option<(u32, u32)>>> = RefCell::new(HashMap::new());
}

const PARSE_CACHE_SIZE: usize = 24;

/// 带缓存的 [`parse_blocks`]
pub fn parsed(source: &SharedString) -> Rc<Vec<Block>> {
    PARSE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(pos) = cache.iter().position(|(s, _)| s == source) {
            let entry = cache.remove(pos);
            let blocks = entry.1.clone();
            cache.push(entry);
            return blocks;
        }
        let blocks = Rc::new(parse_blocks(source));
        cache.push((source.clone(), blocks.clone()));
        if cache.len() > PARSE_CACHE_SIZE {
            cache.remove(0);
        }
        blocks
    })
}

/// 图片原始尺寸（读文件头；读不到返回 `None`）
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    IMAGE_SIZES.with(|sizes| {
        if let Some(size) = sizes.borrow().get(path) {
            return *size;
        }
        let size = image::image_dimensions(path).ok();
        // 文件还没同步下来时不缓存失败结果，下次重绘再试
        if size.is_some() {
            sizes.borrow_mut().insert(path.to_path_buf(), size);
        }
        size
    })
}

/// 文档里全部自己排版的图片（独占段落的与从文字段落里拆出来的；表格里的不算）的本机路径（按文档顺序；预览翻页用）
pub fn block_image_paths(blocks: &[Block], images_dir: &Path) -> Vec<PathBuf> {
    fn walk(blocks: &[Block], dir: &Path, out: &mut Vec<PathBuf>) {
        for block in blocks {
            match block {
                Block::Images(images) => {
                    out.extend(
                        images
                            .iter()
                            .filter_map(|image| image_ref::resolve_image_path(&image.url, dir)),
                    );
                }
                Block::Mixed(parts) => {
                    out.extend(parts.iter().filter_map(|part| match part {
                        InlinePart::Image(image) => image_ref::resolve_image_path(&image.url, dir),
                        _ => None,
                    }));
                }
                Block::Quote(children) => walk(children, dir, out),
                Block::List(list) => {
                    for item in &list.items {
                        walk(&item.blocks, dir, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(blocks, images_dir, &mut out);
    out
}

// ---------------------------------------------------------------------------
// 渲染
// ---------------------------------------------------------------------------

/// 点击图片：(文档里全部可预览的图片（[`block_image_paths`]）, 被点的下标)
pub type ImageClickHandler = Rc<dyn Fn(Vec<PathBuf>, usize, &mut Window, &mut App)>;

/// 只读 Markdown 视图
#[derive(IntoElement)]
pub struct MarkdownView {
    id: SharedString,
    source: SharedString,
    images_dir: Arc<PathBuf>,
    readonly: bool,
    selectable: bool,
    on_image_click: Option<ImageClickHandler>,
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

/// 一张自己排版的图片：原始尺寸、最大宽度 100%（按宽高比缩小）、圆角 6、阴影；悬停变淡，点击打开预览。
/// 本机图片按出现顺序编号（`image_counter`，与 [`block_image_paths`] 顺序一致）
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
    let size = local.as_deref().and_then(image_size);
    let alt = SharedString::from(image.alt.clone());
    let picture = img(source)
        .size_full()
        .rounded(px(6.))
        .with_fallback(move || {
            div()
                .text_size(px(14.))
                .text_color(theme::light().text_tertiary)
                .child(alt.clone())
                .into_any_element()
        });
    let frame = div()
        .id(SharedString::from(format!("{}/{id}", ctx.base_id)))
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
        .map(|this| match size {
            Some((w, h)) if w > 0 && h > 0 => {
                this.w(px(w as f32)).aspect_ratio(w as f32 / h as f32)
            }
            _ => this,
        })
        .child(picture);
    match (ctx.on_image_click.clone(), index) {
        (Some(handler), Some(index)) => {
            let paths = ctx.image_paths.clone();
            frame
                .on_click(move |_, window, cx| handler(paths.to_vec(), index, window, cx))
                .into_any_element()
        }
        _ => frame.into_any_element(),
    }
}

/// 一行图片 / 文字：放不下时换行；各项垂直居中（旧版 img 是 `inline-block` + Tailwind preflight 的
/// `vertical-align: middle`，行高被图片撑开、文字落在图片高度中间）
fn inline_row(items: Vec<AnyElement>) -> gpui_kit::Div {
    div()
        .w_full()
        .flex()
        .flex_wrap()
        .items_center()
        .children(items)
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
    inline_row(elements).into_any_element()
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
    let mut rows: Vec<Vec<AnyElement>> = vec![Vec::new()];
    for (ix, part) in parts.iter().enumerate() {
        match part {
            InlinePart::Break => rows.push(Vec::new()),
            InlinePart::Text(markdown) => {
                let text = div()
                    .min_w_0()
                    .child(inline_view(ctx, &format!("{path}s{ix}"), markdown, color))
                    .into_any_element();
                rows.last_mut().expect("至少一行").push(text);
            }
            InlinePart::Image(image) => {
                let frame = image_frame(image, ctx, &format!("{path}i{ix}"), image_counter);
                rows.last_mut().expect("至少一行").push(frame);
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
            rows.into_iter()
                .map(|items| inline_row(items).min_h(px(editor_theme::MD_P_LINE))),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(text: &str) -> Block {
        Block::Paragraph(text.to_string())
    }

    #[test]
    fn parses_the_sample_description_into_blocks() {
        let source = "## 目标\n\n整理 **Q3 OKR** 的完成情况。参考 [上季度复盘](https://example.com/okr/q2)。\n\n- 汇总 *完成度*\n- 例如 `sync-data.json.gz` 超过 ~~5MB~~ 8MB\n\n### 进度表\n\n| KR | 负责人 |\n| --- | :-: |\n| 日活 | 小王 |\n\n- [x] 拉取\n- [ ] 写结论\n\n> 评审 **10 月**\n\n```bash\npython3 a.py\n```\n";
        let blocks = parse_blocks(source);
        assert_eq!(blocks.len(), 8, "{blocks:#?}");
        assert_eq!(blocks[0], Block::Heading(2, "目标".into()));
        assert_eq!(
            blocks[1],
            para("整理 **Q3 OKR** 的完成情况。参考 [上季度复盘](https://example.com/okr/q2)。")
        );
        let Block::List(list) = &blocks[2] else {
            panic!("list")
        };
        assert!(!list.ordered);
        assert_eq!(list.items[0].blocks, vec![para("汇总 *完成度*")]);
        assert_eq!(
            list.items[1].blocks,
            vec![para("例如 `sync-data.json.gz` 超过 ~~5MB~~ 8MB")]
        );
        assert_eq!(blocks[3], Block::Heading(3, "进度表".into()));
        let Block::Table(table) = &blocks[4] else {
            panic!("table")
        };
        assert_eq!(table.aligns, vec![ColumnAlign::Left, ColumnAlign::Center]);
        assert_eq!(table.rows.len(), 2);
        assert_eq!(table.rows[1], vec!["日活".to_string(), "小王".to_string()]);
        let Block::List(tasks) = &blocks[5] else {
            panic!("tasks")
        };
        assert_eq!(tasks.items[0].checked, Some(true));
        assert_eq!(tasks.items[1].checked, Some(false));
        assert_eq!(blocks[6], Block::Quote(vec![para("评审 **10 月**")]));
        assert_eq!(blocks[7], Block::Code("python3 a.py".into()));
    }

    #[test]
    fn escapes_punctuation_so_block_syntax_is_not_reinterpreted() {
        // 标题内容 "1. 背景" 单独解析会变成有序列表；转义后保持文字
        let blocks = parse_blocks("## 1. 背景 #tag\n\n| a |\n| - |\n| - b |\n");
        assert_eq!(blocks[0], Block::Heading(2, "1\\. 背景 \\#tag".into()));
        let Block::Table(table) = &blocks[1] else {
            panic!("table")
        };
        assert_eq!(table.rows[1], vec!["\\- b".to_string()]);
        // 引用里的续行不会带着 "> "
        let blocks = parse_blocks("> 第一行\n> 第二行\n");
        assert_eq!(blocks[0], Block::Quote(vec![para("第一行\n第二行")]));
    }

    fn image(name: &str, alt: &str) -> BlockImage {
        BlockImage {
            url: format!("minitodo-image://{name}"),
            alt: alt.into(),
        }
    }

    fn text(markdown: &str) -> InlinePart {
        InlinePart::Text(markdown.to_string())
    }

    #[test]
    fn standalone_images_and_br_lines_become_blocks() {
        let blocks = parse_blocks(
            "![指标截图](minitodo-image://sample-chart.png)\n\n<br />\n\n文字 ![a](minitodo-image://a.png)\n",
        );
        assert_eq!(
            blocks[0],
            Block::Images(vec![image("sample-chart.png", "指标截图")])
        );
        assert_eq!(blocks[1], Block::EmptyLine);
        assert_eq!(
            blocks[2],
            Block::Mixed(vec![
                text("文字\u{a0}"),
                InlinePart::Image(image("a.png", "a"))
            ])
        );
        assert_eq!(
            block_image_paths(&blocks, Path::new("/d")),
            vec![
                PathBuf::from("/d/sample-chart.png"),
                PathBuf::from("/d/a.png")
            ]
        );
    }

    #[test]
    fn images_next_to_text_are_split_out_of_the_paragraph() {
        // 与图片相邻的空白变成不换行空格；文字段按行内 Markdown 序列化
        let blocks = parse_blocks("pic ![e](minitodo-image://e.png) **after** text\n");
        assert_eq!(
            blocks[0],
            Block::Mixed(vec![
                text("pic\u{a0}"),
                InlinePart::Image(image("e.png", "e")),
                text("\u{a0}**after** text"),
            ])
        );
        // 顶层硬换行另起一行；与图片之间没有空白时不加空格
        let blocks = parse_blocks("第一行\\\n![e](minitodo-image://e.png)\\\n最后一行\n");
        assert_eq!(
            blocks[0],
            Block::Mixed(vec![
                text("第一行"),
                InlinePart::Break,
                InlinePart::Image(image("e.png", "e")),
                InlinePart::Break,
                text("最后一行"),
            ])
        );
        // 两张图片之间的空格保留一个空格宽；引用式图片按定义解析；列表 / 引用里同样拆开
        let blocks = parse_blocks(
            "![a](minitodo-image://a.png) ![b][r] 尾巴\n\n- 项 ![c](minitodo-image://c.png)\n\n[r]: minitodo-image://b.png\n",
        );
        assert_eq!(
            blocks[0],
            Block::Mixed(vec![
                InlinePart::Image(image("a.png", "a")),
                text("\u{a0}"),
                InlinePart::Image(image("b.png", "b")),
                text("\u{a0}尾巴"),
            ])
        );
        let Block::List(list) = &blocks[1] else {
            panic!("list: {blocks:?}")
        };
        assert_eq!(
            list.items[0].blocks,
            vec![Block::Mixed(vec![
                text("项\u{a0}"),
                InlinePart::Image(image("c.png", "c"))
            ])]
        );
        assert_eq!(
            block_image_paths(&blocks, Path::new("/d")),
            ["a.png", "b.png", "c.png"].map(|n| PathBuf::from("/d").join(n))
        );
        // 链接里的图片、没有定义的引用式图片不拆（仍是普通段落）
        let blocks = parse_blocks("看 [![a](minitodo-image://a.png)](https://x.y) 与 ![b][nope]\n");
        assert!(matches!(blocks[0], Block::Paragraph(_)), "{blocks:?}");
    }

    #[test]
    fn first_line_of_an_image_row_is_as_tall_as_its_images() {
        let dir = std::env::temp_dir().join(format!("mt-md-line-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        ::image::RgbImage::new(16, 90)
            .save(dir.join("tall.png"))
            .unwrap();
        let blocks = parse_blocks(
            "前 ![t](minitodo-image://tall.png) 后\n\n文字\\\n![t](minitodo-image://tall.png)\n",
        );
        assert_eq!(blocks[0].first_line_height(&dir), 90.);
        // 图片在第二行：首行仍是一行字高
        assert_eq!(blocks[1].first_line_height(&dir), editor_theme::MD_P_LINE);
        // 读不到的图片按一行字高
        let missing = parse_blocks("![m](minitodo-image://missing.png) x\n");
        assert_eq!(missing[0].first_line_height(&dir), editor_theme::MD_P_LINE);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn inline_serialization_round_trips_through_the_parser() {
        for source in [
            // 段落顶层的图片会拆成 Block::Mixed；链接里的图片仍按行内 Markdown 序列化
            "a **b** *c* ~~d~~ `e` [f](https://x.y/z \"t\") [![g](minitodo-image://g.png)](https://x.y)",
            "代码 `` a`b `` 与 ` `` `",
            "行尾硬换行\\\n下一行",
            "[引用链接][r]\n\n[r]: https://example.com",
            "x*(注)*y 与 a_b_c",
        ] {
            let blocks = parse_blocks(source);
            let Block::Paragraph(first) = &blocks[0] else {
                panic!("{source}: {blocks:?}")
            };
            // 重新解析序列化结果，得到同样的块
            let again = parse_blocks(first);
            assert_eq!(again.len(), 1, "{source} → {first}");
            assert_eq!(&again[0], &blocks[0], "{source} → {first}");
        }
    }

    #[test]
    fn margins_collapse_like_css() {
        let list = Block::List(MdList {
            ordered: false,
            start: 1,
            items: vec![MdItem {
                checked: None,
                blocks: vec![para("x")],
            }],
        });
        // ul 0.4em(5.6) 与 li 内段落 6.4 折叠
        assert_eq!(list.margins(), (6.4, 6.4));
        assert_eq!(Block::Heading(2, "t".into()).margins(), (14.4, 7.2));
        assert_eq!(Block::Quote(vec![para("x")]).margins(), (7., 7.));
        assert_eq!(Block::Code("x".into()).margins(), (0., 0.));
    }

    #[test]
    fn br_html_detection() {
        assert!(is_br_html("<br />"));
        assert!(is_br_html("<br>"));
        assert!(is_br_html(" <BR/> "));
        assert!(!is_br_html("<br class=\"x\">"));
        assert!(!is_br_html("<b>"));
    }
}

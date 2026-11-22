//! Markdown 解析（mdast → 块模型）与解析缓存。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::SharedString;
use markdown::mdast::{self, AlignKind, Node};

use super::image::split_size_suffix;
use super::{Block, BlockImage, ColumnAlign, ImageSpan, InlinePart, MdItem, MdList, MdTable};

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

pub(super) fn is_br_html(value: &str) -> bool {
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
        Node::Image(image) => {
            let (alt, size) = split_size_suffix(&image.alt);
            Some(BlockImage {
                url: image.url.clone(),
                alt: alt.to_string(),
                size,
                span: image.position.as_ref().map(|p| ImageSpan {
                    range: p.start.offset..p.end.offset,
                    title: image.title.clone(),
                }),
            })
        }
        Node::ImageReference(r) => defs.get(&r.identifier).map(|(url, _)| {
            let (alt, size) = split_size_suffix(&r.alt);
            BlockImage {
                url: url.clone(),
                alt: alt.to_string(),
                size,
                span: None,
            }
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

pub(super) fn escape_text(text: &str, out: &mut String) {
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

pub(super) fn link_destination(url: &str) -> String {
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

pub(super) fn link_title(title: &Option<String>) -> String {
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

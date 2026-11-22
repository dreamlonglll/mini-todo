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
//! 图片：`minitodo-image://<name>` 与旧 asset URL 映射到本机 images 目录（`util::image_ref`）；默认按原始尺寸显示
//! （max-width 100%，超宽按宽高比缩小）、圆角 6、阴影，单击打开预览（[`MarkdownView::on_image_click`]，不再冒泡给编辑器）。
//! 显示尺寸写在 alt 的后缀里（Obsidian 写法）：`![alt|300](url)` 宽 300、高按原图比例，`![alt|300x200](url)` 宽高都指定；
//! 不认识这个后缀的旧版本只会忽略它（按原始尺寸显示）。编辑排版（[`MarkdownView::image_resize`]）在悬停的图片上显示
//! 右边 / 底边 / 右下角三个手柄，拖动调整大小、双击还原原始尺寸，松手后由编辑器重写这张图片的源码（[`image_source`]）。
//! 段落顶层的图片都由本模块排：只有图片的段落是 [`Block::Images`]；与文字同段的拆成"文字段 / 图片 / 文字段"
//! （[`Block::Mixed`]），同一行里依次排开、放不下换行、垂直居中——旧版 img 是 `inline-block` + `vertical-align:
//! middle`，行高被图片撑开（TextView 自己画行内图片时会把它缩成一行字高，不能交给它）。
//! 链接：一律走 `util::file_link` 白名单（必须接管 `on_link_click`，否则 TextView 默认直接 `open_url`）。
//!
//! 与旧版的差异：行内代码的细节由 TextView 绘制（代码字号 0.875em、内边距 4px，旧版 0.9em / 6px）；与图片同段的
//! 文字段是一整块（长到一行放不下时独占一行，旧版会接着图片所在行的剩余宽度排）；链接 / 标题 / 表格里的图片仍由 TextView
//! 按一行字高绘制且不能点开预览；选择只在单个块内（不能跨段落拖选）；任务项方框只显示（旧版同样不能点击切换）。

use std::ops::Range;
use std::path::Path;

use crate::theme::editor_theme;
use crate::util::image_ref;

mod image;
mod parse;
mod render;

pub use image::{
    ResizeHandle, block_image_paths, committed_size, display_size, image_size, image_source,
    resize_by, split_size_suffix,
};
pub use parse::{parse_blocks, parsed};
pub use render::{
    ImageClickHandler, ImageResize, ImageResizeEvent, ImageResizeHandler, MarkdownView,
};

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
    /// 去掉尺寸后缀（`|300` / `|300x200`）的 alt
    pub alt: String,
    /// alt 后缀里的显示尺寸；`None` = 原始尺寸
    pub size: Option<ImageSize>,
    /// 内联图片 `![alt](url "title")` 在源码里的位置；引用式图片没有（不能在界面上调整大小）
    pub span: Option<ImageSpan>,
}

/// 图片的显示尺寸（逻辑像素），存在 alt 的后缀里
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageSize {
    pub width: u32,
    /// `None`：按原图宽高比
    pub height: Option<u32>,
}

/// 内联图片在源码里的字节范围与标题（调整大小时整段重写）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSpan {
    pub range: Range<usize>,
    pub title: Option<String>,
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
    pub(super) fn margins(&self) -> (f32, f32) {
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

    /// 首行行高（列表标记按它垂直居中）。图片所在的行被图片撑高（按显示高度估算；超宽被缩小的图片会偏高）
    pub(super) fn first_line_height(&self, images_dir: &Path) -> f32 {
        let tallest = |images: &mut dyn Iterator<Item = &BlockImage>| {
            images
                .filter_map(|image| {
                    let natural = image_ref::resolve_image_path(&image.url, images_dir)
                        .and_then(|path| image_size(&path));
                    display_size(image.size, natural).and_then(|(_, h)| h)
                })
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
pub(super) fn heading_metrics(level: u8) -> (f32, f32, f32, f32) {
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
pub(super) fn item_margins(item: &MdItem) -> (f32, f32) {
    let (first, last) = edge_margins(&item.blocks);
    (first, editor_theme::MD_LI_MARGIN.max(last))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::parse::is_br_html;
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
            size: None,
            span: None,
        }
    }

    fn text(markdown: &str) -> InlinePart {
        InlinePart::Text(markdown.to_string())
    }

    /// 去掉图片的源码位置（按内容比较块时用）
    fn strip_spans(blocks: Vec<Block>) -> Vec<Block> {
        fn image(mut image: BlockImage) -> BlockImage {
            image.span = None;
            image
        }
        blocks
            .into_iter()
            .map(|block| match block {
                Block::Images(images) => Block::Images(images.into_iter().map(image).collect()),
                Block::Mixed(parts) => Block::Mixed(
                    parts
                        .into_iter()
                        .map(|part| match part {
                            InlinePart::Image(i) => InlinePart::Image(image(i)),
                            other => other,
                        })
                        .collect(),
                ),
                Block::Quote(children) => Block::Quote(strip_spans(children)),
                Block::List(mut list) => {
                    for item in &mut list.items {
                        item.blocks = strip_spans(std::mem::take(&mut item.blocks));
                    }
                    Block::List(list)
                }
                other => other,
            })
            .collect()
    }

    fn parse(source: &str) -> Vec<Block> {
        strip_spans(parse_blocks(source))
    }

    /// 第一张自己排版的图片
    fn first_image(blocks: &[Block]) -> BlockImage {
        for block in blocks {
            match block {
                Block::Images(images) => return images[0].clone(),
                Block::Mixed(parts) => {
                    if let Some(image) = parts.iter().find_map(|part| match part {
                        InlinePart::Image(image) => Some(image.clone()),
                        _ => None,
                    }) {
                        return image;
                    }
                }
                _ => {}
            }
        }
        panic!("没有图片：{blocks:?}")
    }

    #[test]
    fn standalone_images_and_br_lines_become_blocks() {
        let blocks = parse(
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
        let blocks = parse("pic ![e](minitodo-image://e.png) **after** text\n");
        assert_eq!(
            blocks[0],
            Block::Mixed(vec![
                text("pic\u{a0}"),
                InlinePart::Image(image("e.png", "e")),
                text("\u{a0}**after** text"),
            ])
        );
        // 顶层硬换行另起一行；与图片之间没有空白时不加空格
        let blocks = parse("第一行\\\n![e](minitodo-image://e.png)\\\n最后一行\n");
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
        let blocks = parse(
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
    fn size_suffix_in_alt() {
        let size = |width, height| Some(ImageSize { width, height });
        assert_eq!(split_size_suffix("截图|300"), ("截图", size(300, None)));
        assert_eq!(split_size_suffix("a |300x200"), ("a", size(300, Some(200))));
        assert_eq!(split_size_suffix("a|300X200"), ("a", size(300, Some(200))));
        assert_eq!(split_size_suffix("|64"), ("", size(64, None)));
        // 只认最后一段；不是合法尺寸时 alt 原样保留
        assert_eq!(split_size_suffix("a|b|12"), ("a|b", size(12, None)));
        for alt in [
            "a|b",
            "a|0",
            "a|300x",
            "a|x200",
            "a|123456",
            "a|-3",
            "a| ",
            "无后缀",
        ] {
            assert_eq!(split_size_suffix(alt), (alt, None), "{alt}");
        }
    }

    #[test]
    fn inline_images_remember_where_they_are_in_the_source() {
        let source = "前文 ![截图|300](minitodo-image://a.png \"标题\") 后\n\n![b|20x10][r]\n\n[r]: minitodo-image://b.png\n";
        let blocks = parse_blocks(source);
        let a = first_image(&blocks);
        assert_eq!(a.alt, "截图");
        assert_eq!(
            a.size,
            Some(ImageSize {
                width: 300,
                height: None
            })
        );
        let span = a.span.clone().expect("内联图片有位置");
        // 字节范围（前面有中文）
        assert_eq!(
            &source[span.range.clone()],
            "![截图|300](minitodo-image://a.png \"标题\")"
        );
        assert_eq!(span.title.as_deref(), Some("标题"));
        // 引用式图片照样认尺寸，但没有位置（不能在界面上调整）
        let b = first_image(&blocks[1..]);
        assert_eq!(
            b.size,
            Some(ImageSize {
                width: 20,
                height: Some(10)
            })
        );
        assert_eq!(b.span, None);
    }

    #[test]
    fn regenerated_image_source_parses_back_to_the_same_image() {
        let source = "x ![a*b* [c]|10](<minitodo-image://a b.png> \"t \\\"q\\\"\") y";
        let image = first_image(&parse_blocks(source));
        assert_eq!(image.alt, "ab [c]");
        for size in [
            None,
            Some(ImageSize {
                width: 320,
                height: None,
            }),
            Some(ImageSize {
                width: 320,
                height: Some(90),
            }),
        ] {
            let text = image_source(&image, size);
            let again = first_image(&parse_blocks(&text));
            assert_eq!(again.alt, image.alt, "{text}");
            assert_eq!(again.url, image.url, "{text}");
            assert_eq!(again.size, size, "{text}");
            assert_eq!(
                again.span.and_then(|s| s.title),
                Some("t \"q\"".to_string()),
                "{text}"
            );
        }
        assert_eq!(
            image_source(&image, None),
            "![ab \\[c\\]](<minitodo-image://a b.png> \"t \\\"q\\\"\")"
        );
        let plain = first_image(&parse_blocks("![image.png](minitodo-image://1_a.png)"));
        assert_eq!(
            image_source(
                &plain,
                Some(ImageSize {
                    width: 240,
                    height: None
                })
            ),
            "![image.png|240](minitodo-image://1_a.png)"
        );
    }

    #[test]
    fn display_size_prefers_the_suffix() {
        let natural = Some((400, 200));
        assert_eq!(display_size(None, natural), Some((400., Some(200.))));
        assert_eq!(display_size(None, None), None);
        let width_only = Some(ImageSize {
            width: 100,
            height: None,
        });
        assert_eq!(display_size(width_only, natural), Some((100., Some(50.))));
        // 读不到原图：只知道宽
        assert_eq!(display_size(width_only, None), Some((100., None)));
        let both = Some(ImageSize {
            width: 100,
            height: Some(80),
        });
        assert_eq!(display_size(both, natural), Some((100., Some(80.))));
    }

    #[test]
    fn resize_math() {
        let min = editor_theme::MD_IMAGE_MIN_SIZE;
        let start = (200., 100.);
        // 右边 / 底边只改一边
        assert_eq!(
            resize_by(ResizeHandle::Right, start, (-50., 30.), false, None),
            (150., 100.)
        );
        assert_eq!(
            resize_by(ResizeHandle::Bottom, start, (-50., 30.), false, None),
            (200., 130.)
        );
        // 角：等比，横纵里变化大的那个说了算
        assert_eq!(
            resize_by(ResizeHandle::Corner, start, (-100., 5.), false, None),
            (100., 50.)
        );
        assert_eq!(
            resize_by(ResizeHandle::Corner, start, (2., 40.), false, None),
            (280., 140.)
        );
        // Shift：宽高各自跟随
        assert_eq!(
            resize_by(ResizeHandle::Corner, start, (10., -20.), true, None),
            (210., 80.)
        );
        // 最小尺寸（等比时短边也不小于最小值）
        assert_eq!(
            resize_by(ResizeHandle::Right, start, (-500., 0.), false, None),
            (min, 100.)
        );
        assert_eq!(
            resize_by(ResizeHandle::Corner, start, (-500., 0.), false, None),
            (min * 2., min)
        );
        // 容器宽度
        assert_eq!(
            resize_by(ResizeHandle::Corner, start, (300., 0.), false, Some(300.)),
            (300., 150.)
        );
        assert_eq!(
            resize_by(ResizeHandle::Right, start, (300., 0.), false, Some(250.)),
            (250., 100.)
        );
    }

    #[test]
    fn committed_size_omits_height_when_proportional() {
        let natural = Some((1920, 1080));
        assert_eq!(
            committed_size(480.2, 270.4, natural),
            ImageSize {
                width: 480,
                height: None
            }
        );
        assert_eq!(
            committed_size(480., 300., natural),
            ImageSize {
                width: 480,
                height: Some(300)
            }
        );
        assert_eq!(
            committed_size(480., 270., None),
            ImageSize {
                width: 480,
                height: Some(270)
            }
        );
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

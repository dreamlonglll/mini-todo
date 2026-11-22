//! Markdown 图片尺寸缓存、显示尺寸后缀（`|宽` / `|宽x高`）、缩放计算与源码重写。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::theme::editor_theme;
use crate::util::image_ref;

use super::parse::{link_destination, link_title};
use super::{Block, BlockImage, ImageSize, InlinePart};

#[derive(Clone, Copy)]
enum ImageSizeEntry {
    Ready((u32, u32)),
    /// 读取失败（文件缺失 / 尚未同步到位）的负缓存时间：短 TTL 内不再于 UI 渲染主线程反复打盘
    Failed(Instant),
}

const IMAGE_SIZE_FAIL_TTL: Duration = Duration::from_secs(5);
const IMAGE_SIZE_CACHE_MAX: usize = 512;

thread_local! {
    /// 本机图片的原始尺寸（图片文件名唯一且不会被改写，读一次文件头即可；缺失路径按短 TTL 负缓存）
    static IMAGE_SIZES: RefCell<HashMap<PathBuf, ImageSizeEntry>> = RefCell::new(HashMap::new());
}

/// 图片原始尺寸（读文件头；读不到返回 `None`；失败按短 TTL 负缓存避免悬停/滚动时每帧同步打盘）
pub fn image_size(path: &Path) -> Option<(u32, u32)> {
    IMAGE_SIZES.with(|sizes| {
        let now = Instant::now();
        if let Some(entry) = sizes.borrow().get(path).copied() {
            match entry {
                ImageSizeEntry::Ready(size) => return Some(size),
                ImageSizeEntry::Failed(at) if now.duration_since(at) < IMAGE_SIZE_FAIL_TTL => {
                    return None;
                }
                ImageSizeEntry::Failed(_) => {}
            }
        }
        let size = ::image::image_dimensions(path).ok();
        let mut map = sizes.borrow_mut();
        if map.len() >= IMAGE_SIZE_CACHE_MAX {
            map.retain(|_, v| matches!(v, ImageSizeEntry::Ready(_)));
            if map.len() >= IMAGE_SIZE_CACHE_MAX {
                map.clear();
            }
        }
        let entry = match size {
            Some(dims) => ImageSizeEntry::Ready(dims),
            None => ImageSizeEntry::Failed(now),
        };
        map.insert(path.to_path_buf(), entry);
        size
    })
}

// ---------------------------------------------------------------------------
// 图片显示尺寸（alt 后缀 `|宽` / `|宽x高`）
// ---------------------------------------------------------------------------

/// 尺寸后缀里一个数的最大位数（99999px）
const SIZE_DIGITS_MAX: usize = 5;

/// 拆出 alt 末尾的尺寸后缀：`截图|300` → (`截图`, 宽 300)，`截图|300x200` → (`截图`, 300×200)。
/// 不是合法后缀（非数字、0、位数太多）时原样返回、没有尺寸
pub fn split_size_suffix(alt: &str) -> (&str, Option<ImageSize>) {
    fn number(text: &str) -> Option<u32> {
        let text = text.trim();
        if text.is_empty()
            || text.len() > SIZE_DIGITS_MAX
            || !text.bytes().all(|b| b.is_ascii_digit())
        {
            return None;
        }
        text.parse().ok().filter(|v| *v > 0)
    }
    let Some((name, suffix)) = alt.rsplit_once('|') else {
        return (alt, None);
    };
    let (width, height) = match suffix.split_once(['x', 'X', '×']) {
        Some((w, h)) => (w, Some(h)),
        None => (suffix, None),
    };
    let Some(width) = number(width) else {
        return (alt, None);
    };
    let height = match height {
        Some(h) => match number(h) {
            Some(h) => Some(h),
            None => return (alt, None),
        },
        None => None,
    };
    (name.trim_end(), Some(ImageSize { width, height }))
}

/// alt 文字的转义：只转义会改变解析结果的字符（方括号、反斜杠、强调 / 删除线 / 代码 / 尖括号 / 实体），
/// 换行换成空格；`|` 不转义（表格里的图片不能调整大小，不会写到表格里）
fn escape_alt(alt: &str) -> String {
    let mut out = String::with_capacity(alt.len() + 4);
    for c in alt.chars() {
        match c {
            '\\' | '[' | ']' | '*' | '_' | '`' | '<' | '>' | '&' | '~' => {
                out.push('\\');
                out.push(c);
            }
            '\n' | '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// 重新生成一张内联图片的源码（`size` 为 `None` 时去掉尺寸后缀，即原始尺寸）：调整 / 还原大小时整段替换
/// [`super::ImageSpan::range`]。alt 里的格式（强调等）会丢掉，只保留文字
pub fn image_source(image: &BlockImage, size: Option<ImageSize>) -> String {
    let mut alt = image.alt.clone();
    if let Some(size) = size {
        alt.push('|');
        alt.push_str(&size.width.to_string());
        if let Some(height) = size.height {
            alt.push('x');
            alt.push_str(&height.to_string());
        }
    }
    let title = image.span.as_ref().and_then(|span| span.title.clone());
    format!(
        "![{}]({}{})",
        escape_alt(&alt),
        link_destination(&image.url),
        link_title(&title)
    )
}

/// 显示尺寸（逻辑像素，尚未按容器宽度缩小）：(宽, 高)。有后缀按后缀（只给宽时高按原图宽高比；读不到原图时高未知），
/// 否则按原图尺寸；都没有时 `None`
pub fn display_size(
    size: Option<ImageSize>,
    natural: Option<(u32, u32)>,
) -> Option<(f32, Option<f32>)> {
    let natural = natural.filter(|(w, h)| *w > 0 && *h > 0);
    match (size, natural) {
        (
            Some(ImageSize {
                width,
                height: Some(height),
            }),
            _,
        ) => Some((width as f32, Some(height as f32))),
        (
            Some(ImageSize {
                width,
                height: None,
            }),
            natural,
        ) => Some((
            width as f32,
            natural.map(|(w, h)| width as f32 * h as f32 / w as f32),
        )),
        (None, Some((w, h))) => Some((w as f32, Some(h as f32))),
        (None, None) => None,
    }
}

/// 调整大小的手柄
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeHandle {
    /// 右下角：等比缩放（按住 Shift 时宽高各自跟随光标）
    Corner,
    /// 右边：只改宽
    Right,
    /// 底边：只改高
    Bottom,
}

impl ResizeHandle {
    pub(super) const ALL: [ResizeHandle; 3] = [
        ResizeHandle::Right,
        ResizeHandle::Bottom,
        ResizeHandle::Corner,
    ];

    pub(super) fn key(self) -> &'static str {
        match self {
            ResizeHandle::Corner => "c",
            ResizeHandle::Right => "r",
            ResizeHandle::Bottom => "b",
        }
    }
}

/// 拖动手柄后的显示尺寸。`start` 是按下时图片框的实际大小，`delta` 是光标位移；不小于 [`editor_theme::MD_IMAGE_MIN_SIZE`]，
/// 宽不超过 `max_width`（图片框被容器宽度限制时）
pub fn resize_by(
    handle: ResizeHandle,
    start: (f32, f32),
    delta: (f32, f32),
    free: bool,
    max_width: Option<f32>,
) -> (f32, f32) {
    let min = editor_theme::MD_IMAGE_MIN_SIZE;
    let (sw, sh) = (start.0.max(1.), start.1.max(1.));
    let (dx, dy) = delta;
    let clamp_w = |w: f32, floor: f32| {
        let w = w.max(floor);
        match max_width {
            Some(max) => w.min(max.max(floor)),
            None => w,
        }
    };
    match handle {
        ResizeHandle::Right => (clamp_w(sw + dx, min), sh),
        ResizeHandle::Bottom => (sw, (sh + dy).max(min)),
        ResizeHandle::Corner if free => (clamp_w(sw + dx, min), (sh + dy).max(min)),
        ResizeHandle::Corner => {
            let aspect = sw / sh;
            // 横向、纵向位移里变化大的那个决定新宽度
            let width = if dx.abs() >= (dy * aspect).abs() {
                sw + dx
            } else {
                (sh + dy) * aspect
            };
            let width = clamp_w(width, min.max(min * aspect));
            (width, width / aspect)
        }
    }
}

/// 写回源码的尺寸：取整；与原图宽高比一致（取整误差不超过 1px）时只写宽
pub fn committed_size(width: f32, height: f32, natural: Option<(u32, u32)>) -> ImageSize {
    let width = width.round().max(1.) as u32;
    let height = height.round().max(1.) as u32;
    let proportional = natural.is_some_and(|(nw, nh)| {
        nw > 0
            && nh > 0
            && (f64::from(width) * f64::from(nh) / f64::from(nw) - f64::from(height)).abs() <= 1.
    });
    ImageSize {
        width,
        height: (!proportional).then_some(height),
    }
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

//! 粘贴用的剪贴板内容：图片 / 文件 / 有没有文字（补 GPUI 读剪贴板的不足）。
//!
//! gpui-pre-windows 0.3.9 的 `read_from_clipboard` 只认 `PNG` / `GIF` / `JFIF` / `image/svg+xml` 与 `CF_DIB`，而且取**枚举到的
//! 第一个**图片格式：
//! - 截图工具多半先放位图，于是剪贴板里明明有 PNG 也拿到未压缩的位图（BMP，宽 × 高 × 4 字节，4K 截图约 33MB），
//!   超过 20MB 上限插不进去，插得进去的也是几 MB 的 `.bmp`（还要同步到 WebDAV）；
//! - 只提供 `image/png` 之类 MIME 名格式的应用它一个都不认，整个剪贴板读成空，粘贴静默失败（只在日志里留一条
//!   `Try to paste with unsupported clipboard format`）。
//!
//! 所以 Windows 上 Markdown 编辑器改用 [`read`]：一次枚举全部格式，按 [`pick_order`] 的优先级（PNG > JPEG > GIF > WebP >
//! BMP 文件 > `CF_DIB`）逐个读取、按文件头校验，取第一个能用的；位图（[`ClipboardImage::Bitmap`]）由调用方在后台用
//! [`bitmap_to_png`] 转成 PNG 再保存。其它平台 [`read`] 返回 `None`，调用方照旧用 GPUI 读到的内容（[`from_item`]）。
//!
//! 剪贴板读取都是同步的（与 GPUI 自己的实现相同）：`GetClipboardData` 遇到延迟渲染的格式会同步等数据所有者（别的进程）
//! 生成数据，不会给本进程的窗口发消息，可以在事件处理里调用。

use std::path::PathBuf;

use gpui_kit::{ClipboardEntry, ClipboardItem, ImageFormat};

/// 剪贴板里的一张图片
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardImage {
    /// 已编码的图片与对应的白名单扩展名（`png` / `jpg` / `gif` / `webp`）
    Encoded { bytes: Vec<u8>, ext: &'static str },
    /// BMP 文件（带 14 字节文件头）：保存前转成 PNG
    Bitmap(Vec<u8>),
    /// 认得但不支持的图片格式（MIME，用于提示）
    Unsupported(&'static str),
}

/// 一次粘贴能用的剪贴板内容
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClipboardContent {
    /// 复制的文件（资源管理器 / 访达里复制的文件，或聊天软件附带的缓存文件）
    pub files: Vec<PathBuf>,
    /// 选出来的图片
    pub image: Option<ClipboardImage>,
    /// 有没有文字
    pub has_text: bool,
    /// 剪贴板里的全部格式（诊断日志用）
    pub formats: Vec<String>,
}

/// 预定义的剪贴板格式（`CF_*`）
#[cfg_attr(not(windows), allow(dead_code))]
mod cf {
    pub const TEXT: u32 = 1;
    pub const DIB: u32 = 8;
    pub const UNICODETEXT: u32 = 13;
    pub const HDROP: u32 = 15;
    pub const DIBV5: u32 = 17;
}

/// 图片格式的种类（按优先级从高到低）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
    /// 带文件头的 BMP（`image/bmp`）
    BmpFile,
    /// `CF_DIB` / `CF_DIBV5`：不带文件头的位图
    Dib,
}

/// 格式 → 图片种类（注册格式按名字，大小写不敏感）
#[cfg_attr(not(windows), allow(dead_code))]
fn kind_of(id: u32, name: &str) -> Option<Kind> {
    if id == cf::DIB || id == cf::DIBV5 {
        return Some(Kind::Dib);
    }
    match name.to_ascii_lowercase().as_str() {
        "png" | "image/png" => Some(Kind::Png),
        "jfif" | "jpeg" | "jpg" | "image/jpeg" | "image/jpg" | "image/pjpeg" => Some(Kind::Jpeg),
        "gif" | "image/gif" => Some(Kind::Gif),
        "webp" | "image/webp" => Some(Kind::Webp),
        "bmp" | "image/bmp" | "image/x-bmp" => Some(Kind::BmpFile),
        _ => None,
    }
}

/// 剪贴板里的图片格式按优先级排好（同种类时 `CF_DIB` 先于 `CF_DIBV5`：系统合成的 `CF_DIB` 一定是
/// `BITMAPINFOHEADER`，解码最稳）
#[cfg_attr(not(windows), allow(dead_code))]
fn pick_order(formats: &[(u32, String)]) -> Vec<(u32, Kind)> {
    let mut picks: Vec<(u32, Kind)> = formats
        .iter()
        .filter_map(|(id, name)| kind_of(*id, name).map(|kind| (*id, kind)))
        .collect();
    picks.sort_by_key(|(id, kind)| (*kind, *id == cf::DIBV5));
    picks
}

/// 读到的字节 → 图片；按文件头校验（声明的格式与内容不符时按实际内容；认不出来返回 `None`，换下一个格式）
#[cfg_attr(not(windows), allow(dead_code))]
fn image_from_bytes(kind: Kind, bytes: Vec<u8>) -> Option<ClipboardImage> {
    if kind == Kind::Dib {
        return dib_to_bmp(&bytes).map(ClipboardImage::Bitmap);
    }
    match image::guess_format(&bytes).ok()? {
        image::ImageFormat::Png => Some(ClipboardImage::Encoded { bytes, ext: "png" }),
        image::ImageFormat::Jpeg => Some(ClipboardImage::Encoded { bytes, ext: "jpg" }),
        image::ImageFormat::Gif => Some(ClipboardImage::Encoded { bytes, ext: "gif" }),
        image::ImageFormat::WebP => Some(ClipboardImage::Encoded { bytes, ext: "webp" }),
        image::ImageFormat::Bmp => Some(ClipboardImage::Bitmap(bytes)),
        _ => None,
    }
}

/// `CF_DIB` / `CF_DIBV5`（BMP 去掉 14 字节文件头）→ BMP 文件：补上文件头，像素数据偏移 = 文件头 + 信息头 +
/// （`BITMAPINFOHEADER` 时）`BI_BITFIELDS` 的 3 个掩码 / `BI_ALPHABITFIELDS` 的 4 个掩码 + 调色板
pub fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let u32_at = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(dib.get(at..at + 4)?.try_into().ok()?))
    };
    let header_size = u32_at(0)?;
    if header_size < 40 || header_size as usize > dib.len() {
        return None;
    }
    let bit_count = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
    let compression = u32_at(16)?;
    let colors_used = u32_at(32)?;
    let masks = match (header_size, compression) {
        (40, 3) => 12,
        (40, 6) => 16,
        _ => 0,
    };
    let palette = if bit_count <= 8 {
        let colors = if colors_used == 0 {
            1u32 << bit_count
        } else {
            colors_used
        };
        colors.checked_mul(4)?
    } else {
        colors_used.checked_mul(4)?
    };
    let offset = 14u32
        .checked_add(header_size)?
        .checked_add(masks)?
        .checked_add(palette)?;
    let file_size = u32::try_from(dib.len()).ok()?.checked_add(14)?;
    if offset > file_size {
        return None;
    }
    let mut bmp = Vec::with_capacity(file_size as usize);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&file_size.to_le_bytes());
    bmp.extend_from_slice(&[0u8; 4]);
    bmp.extend_from_slice(&offset.to_le_bytes());
    bmp.extend_from_slice(dib);
    Some(bmp)
}

/// BMP 文件 → PNG（阻塞：大图解码 / 编码要几百毫秒，放后台调用）。32 位 `BI_RGB` 位图的第 4 个字节按惯例不当透明度
pub fn bitmap_to_png(bmp: &[u8]) -> Result<Vec<u8>, String> {
    let image = image::load_from_memory_with_format(bmp, image::ImageFormat::Bmp)
        .map_err(|e| format!("剪贴板里的位图无法解码：{e}"))?;
    let mut out = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| format!("转换为 PNG 失败：{e}"))?;
    Ok(out.into_inner())
}

/// GPUI 读到的剪贴板内容 → [`ClipboardContent`]（非 Windows 平台，或 Windows 上 [`read`] 打不开剪贴板时）
pub fn from_item(item: &ClipboardItem) -> ClipboardContent {
    let mut content = ClipboardContent::default();
    for entry in item.entries() {
        match entry {
            ClipboardEntry::String(_) => {
                content.has_text = true;
                content.formats.push("text".into());
            }
            ClipboardEntry::ExternalPaths(paths) => {
                content.files.extend(paths.paths().iter().cloned());
                content.formats.push("files".into());
            }
            ClipboardEntry::Image(image) => {
                content.formats.push(format!("image/{:?}", image.format));
                if content.image.is_some() {
                    continue;
                }
                let bytes = image.bytes.clone();
                content.image = Some(match image.format {
                    ImageFormat::Png => ClipboardImage::Encoded { bytes, ext: "png" },
                    ImageFormat::Jpeg => ClipboardImage::Encoded { bytes, ext: "jpg" },
                    ImageFormat::Webp => ClipboardImage::Encoded { bytes, ext: "webp" },
                    ImageFormat::Gif => ClipboardImage::Encoded { bytes, ext: "gif" },
                    ImageFormat::Bmp => ClipboardImage::Bitmap(bytes),
                    ImageFormat::Svg => ClipboardImage::Unsupported("image/svg+xml"),
                    ImageFormat::Tiff => ClipboardImage::Unsupported("image/tiff"),
                    ImageFormat::Ico => ClipboardImage::Unsupported("image/x-icon"),
                    ImageFormat::Pnm => ClipboardImage::Unsupported("image/x-portable-anymap"),
                });
            }
        }
    }
    content
}

/// 直接读系统剪贴板（Windows）；打不开剪贴板时 `None`
#[cfg(windows)]
pub fn read() -> Option<ClipboardContent> {
    win::read()
}

/// 非 Windows：用 GPUI 读到的内容（[`from_item`]）
#[cfg(not(windows))]
pub fn read() -> Option<ClipboardContent> {
    None
}

#[cfg(windows)]
mod win {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::path::PathBuf;

    use windows::Win32::Foundation::HGLOBAL;
    use windows::Win32::System::DataExchange::{
        CloseClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardFormatNameW,
        OpenClipboard,
    };
    use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

    use super::{ClipboardContent, cf, image_from_bytes, pick_order};

    /// 剪贴板里格式数的上限（防御异常的枚举）
    const MAX_FORMATS: usize = 512;

    struct Opened;

    impl Drop for Opened {
        fn drop(&mut self) {
            if let Err(e) = unsafe { CloseClipboard() } {
                log::warn!("[clipboard] 关闭剪贴板失败: {e}");
            }
        }
    }

    pub(super) fn read() -> Option<ClipboardContent> {
        if let Err(e) = unsafe { OpenClipboard(None) } {
            log::warn!("[clipboard] 打开剪贴板失败: {e}");
            return None;
        }
        let _opened = Opened;
        let mut formats = Vec::new();
        let mut id = 0u32;
        while formats.len() < MAX_FORMATS {
            id = unsafe { EnumClipboardFormats(id) };
            if id == 0 {
                break;
            }
            formats.push((id, format_name(id)));
        }
        let has_text = formats
            .iter()
            .any(|(id, _)| *id == cf::UNICODETEXT || *id == cf::TEXT);
        let files = if formats.iter().any(|(id, _)| *id == cf::HDROP) {
            dropped_files()
        } else {
            Vec::new()
        };
        let image = pick_order(&formats)
            .into_iter()
            .find_map(|(id, kind)| image_from_bytes(kind, data(id)?));
        Some(ClipboardContent {
            files,
            image,
            has_text,
            formats: formats
                .into_iter()
                .map(|(id, name)| format!("{id}:{name}"))
                .collect(),
        })
    }

    /// 注册格式的名字；预定义格式没有名字，用 `CF_*` 名
    fn format_name(id: u32) -> String {
        let mut buffer = [0u16; 128];
        let len = unsafe { GetClipboardFormatNameW(id, &mut buffer) };
        if len > 0 {
            return String::from_utf16_lossy(&buffer[..len as usize]);
        }
        match id {
            cf::TEXT => "CF_TEXT",
            2 => "CF_BITMAP",
            3 => "CF_METAFILEPICT",
            7 => "CF_OEMTEXT",
            cf::DIB => "CF_DIB",
            cf::UNICODETEXT => "CF_UNICODETEXT",
            14 => "CF_ENHMETAFILE",
            cf::HDROP => "CF_HDROP",
            16 => "CF_LOCALE",
            cf::DIBV5 => "CF_DIBV5",
            _ => "",
        }
        .to_string()
    }

    /// 一个格式的全部字节（`GlobalSize` 可能比实际数据略大，各解码器都容忍尾部多余字节）
    fn data(id: u32) -> Option<Vec<u8>> {
        let handle = unsafe { GetClipboardData(id) }.ok()?;
        let global = HGLOBAL(handle.0);
        let size = unsafe { GlobalSize(global) };
        let ptr = unsafe { GlobalLock(global) } as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = (size > 0).then(|| unsafe { std::slice::from_raw_parts(ptr, size) }.to_vec());
        let _ = unsafe { GlobalUnlock(global) };
        bytes
    }

    /// `CF_HDROP` 里的文件路径
    fn dropped_files() -> Vec<PathBuf> {
        let Ok(handle) = (unsafe { GetClipboardData(cf::HDROP) }) else {
            return Vec::new();
        };
        let hdrop = HDROP(handle.0);
        let count = unsafe { DragQueryFileW(hdrop, u32::MAX, None) };
        let mut files = Vec::with_capacity(count as usize);
        for index in 0..count {
            let len = unsafe { DragQueryFileW(hdrop, index, None) } as usize;
            let mut buffer = vec![0u16; len + 1];
            let copied = unsafe { DragQueryFileW(hdrop, index, Some(&mut buffer)) } as usize;
            if copied == 0 {
                continue;
            }
            files.push(PathBuf::from(OsString::from_wide(&buffer[..copied])));
        }
        files
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn formats(list: &[(u32, &str)]) -> Vec<(u32, String)> {
        list.iter()
            .map(|(id, name)| (*id, name.to_string()))
            .collect()
    }

    #[test]
    fn prefers_encoded_formats_over_bitmaps() {
        // 截图工具：位图在前、PNG 在后
        let order = pick_order(&formats(&[
            (2, ""),
            (cf::DIB, ""),
            (cf::DIBV5, ""),
            (0xC123, "PNG"),
            (cf::UNICODETEXT, ""),
        ]));
        assert_eq!(
            order,
            vec![
                (0xC123, Kind::Png),
                (cf::DIB, Kind::Dib),
                (cf::DIBV5, Kind::Dib)
            ]
        );
        // MIME 名的格式（GPUI 不认）
        let order = pick_order(&formats(&[(0xC200, "image/jpeg"), (0xC201, "image/png")]));
        assert_eq!(order[0], (0xC201, Kind::Png));
        assert_eq!(order[1], (0xC200, Kind::Jpeg));
        assert!(pick_order(&formats(&[(cf::UNICODETEXT, ""), (0xC300, "HTML Format")])).is_empty());
    }

    fn png_bytes() -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbImage::new(3, 2)
            .write_to(&mut out, image::ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }

    #[test]
    fn validates_bytes_by_their_header() {
        let png = png_bytes();
        assert_eq!(
            image_from_bytes(Kind::Png, png.clone()),
            Some(ClipboardImage::Encoded {
                bytes: png.clone(),
                ext: "png"
            })
        );
        // 声明成 JPEG、实际是 PNG：按实际内容
        assert!(matches!(
            image_from_bytes(Kind::Jpeg, png),
            Some(ClipboardImage::Encoded { ext: "png", .. })
        ));
        assert_eq!(image_from_bytes(Kind::Png, b"not an image".to_vec()), None);
    }

    /// 2×2、32 位 `BI_RGB`、自下而上的 DIB（`BITMAPINFOHEADER`）
    fn dib_32bpp() -> Vec<u8> {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&2i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB
        dib.extend_from_slice(&16u32.to_le_bytes());
        dib.extend_from_slice(&[0u8; 16]); // 分辨率、colors_used、colors_important
        // 像素（BGRA，第 4 字节为 0：截图工具的常见写法）：底行红 / 绿，顶行蓝 / 白
        dib.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0, 255, 0, 0, 0, 255, 255, 255, 0]);
        dib
    }

    #[test]
    fn dib_becomes_an_opaque_png() {
        let bmp = dib_to_bmp(&dib_32bpp()).expect("合法 DIB");
        assert_eq!(&bmp[..2], b"BM");
        assert_eq!(u32::from_le_bytes(bmp[10..14].try_into().unwrap()), 54);
        let png = bitmap_to_png(&bmp).unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        // 自下而上：第一行像素在图片底部；第 4 字节为 0 也不透明
        assert_eq!(decoded.get_pixel(0, 1).0, [255, 0, 0, 255]);
        assert_eq!(decoded.get_pixel(1, 1).0, [0, 255, 0, 255]);
        assert_eq!(decoded.get_pixel(0, 0).0, [0, 0, 255, 255]);
        assert_eq!(decoded.get_pixel(1, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn dib_offsets_cover_masks_and_palettes() {
        let mut bitfields = dib_32bpp();
        bitfields[16..20].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(
            u32::from_le_bytes(dib_to_bmp(&bitfields).unwrap()[10..14].try_into().unwrap()),
            54 + 12
        );
        let mut paletted = dib_32bpp();
        paletted[14..16].copy_from_slice(&8u16.to_le_bytes());
        paletted.extend_from_slice(&[0u8; 1024]);
        assert_eq!(
            u32::from_le_bytes(dib_to_bmp(&paletted).unwrap()[10..14].try_into().unwrap()),
            54 + 1024
        );
        assert_eq!(dib_to_bmp(&[0u8; 12]), None);
        // 调色板超出数据：拒绝
        let mut short = dib_32bpp();
        short[14..16].copy_from_slice(&8u16.to_le_bytes());
        assert_eq!(dib_to_bmp(&short), None);
    }
}

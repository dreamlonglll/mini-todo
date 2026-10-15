//! 图标提取与缓存。
//!
//! 流程：`IShellItemImageFactory::GetImage(SIIGBF_ICONONLY)`（常驻 STA 线程 `fence-icon`）→
//! 32 位 BGRA（直通 alpha）→ 去掉小图标的 Shell 边框 → `png` 编码 →
//! 写入 `<应用数据目录>\fence-icons\`，前端用 `convertFileSrc` 加载。
//!
//! 缓存键 = 路径哈希（FNV-1a 64，路径规范化 + 小写）+ 文件修改时间 + 文件大小 + 像素尺寸档位，
//! 文件变化后键随之变化，旧文件由 [`prune_icon_cache`] 清理。

use super::location::{icon_cache_dir, normalize_for_compare};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// 请求尺寸向上取整到这些档位（物理像素），减少 Ctrl+滚轮 调整图标大小时的重复提取
pub const ICON_SIZE_BUCKETS: [u32; 8] = [16, 24, 32, 48, 64, 96, 128, 256];

/// 物理像素尺寸 → 档位（向上取整，超过 256 按 256）
pub fn icon_bucket(size_px: u32) -> u32 {
    ICON_SIZE_BUCKETS
        .iter()
        .copied()
        .find(|bucket| *bucket >= size_px)
        .unwrap_or(ICON_SIZE_BUCKETS[ICON_SIZE_BUCKETS.len() - 1])
}

/// FNV-1a 64 位哈希：实现简单、跨版本稳定（`DefaultHasher` 不保证跨 Rust 版本稳定）
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 路径哈希（规范化后不区分大小写，`\\?\` 前缀与分隔符写法不影响结果）
pub fn path_hash(path: &Path) -> u64 {
    fnv1a64(normalize_for_compare(path).as_bytes())
}

/// 缓存文件名的前缀部分：`{路径哈希}-{mtime 毫秒}-{大小}-`
fn cache_stamp(path: &Path, mtime_ms: u64, len: u64) -> String {
    format!("{:016x}-{:x}-{:x}-", path_hash(path), mtime_ms, len)
}

/// 缓存文件名：`{路径哈希:016x}-{mtime 毫秒:x}-{大小:x}-{档位}.png`
pub fn icon_cache_key(path: &Path, mtime_ms: u64, len: u64, size_px: u32) -> String {
    format!("{}{}.png", cache_stamp(path, mtime_ms, len), size_px)
}

/// 读取文件的（修改时间毫秒, 大小），用于缓存键
fn file_stamp(path: &Path) -> Result<(u64, u64), String> {
    let meta = fs::metadata(path)
        .map_err(|e| format!("无法读取文件信息: {}", super::folder::describe_io_error(&e)))?;
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0);
    Ok((mtime_ms, meta.len()))
}

/// 32 位 BGRA 位图（自上而下，直通 alpha）
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

/// BGRA（`GetDIBits` 输出）→ RGBA（PNG 需要）。
///
/// * `IShellItemImageFactory::GetImage` 返回的位图是**直通 alpha**（非预乘），颜色原样保留；
///   spike 中所有含半透明像素的样本都存在 `max(r,g,b) > a` 的像素（预乘位图不可能出现），
///   因此不做反预乘（详见 `research/spike-icons.md`）
/// * 所有像素 alpha 都为 0：位图没有 alpha 通道（无透明信息的缩略图 / 老式图标），alpha 全部置 255
/// * alpha 为 0 的像素颜色清零，避免 PNG 里残留无意义的颜色
#[cfg_attr(not(windows), allow(dead_code))] // 非 Windows 不提取图标，只有测试用到
pub(crate) fn bgra_to_rgba(bgra: &[u8]) -> Vec<u8> {
    let (pixels, _) = bgra.as_chunks::<4>();
    let no_alpha = pixels.iter().all(|p| p[3] == 0);
    let mut rgba = Vec::with_capacity(bgra.len());
    for p in pixels {
        let (b, g, r, a) = (p[0], p[1], p[2], p[3]);
        if no_alpha {
            rgba.extend_from_slice(&[r, g, b, 255]);
        } else if a == 0 {
            rgba.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }
    rgba
}

/// 去掉 Shell 给"小图标"加的边框。
///
/// 程序 / 文件类型只有 ≤48 px 的图标资源时，按更大尺寸请求，Shell 会把原尺寸图标放在画布正中，
/// 并在画布最外两圈画一个半透明边框（外圈灰 `#646464` α≈0x30、内圈白 α≈0x24，第三圈起透明）。
/// 直接显示会在收纳箱里多出一个方框。检测到这种形态时，裁出居中的原尺寸图标
/// （按 [`ICON_SIZE_BUCKETS`] 中能容纳内容的最小档位）；不是这种形态返回 `None`。
#[cfg_attr(not(windows), allow(dead_code))] // 非 Windows 不提取图标，只有测试用到
pub(crate) fn strip_small_icon_frame(img: &RawImage) -> Option<RawImage> {
    let size = img.width as usize;
    if img.width != img.height || size <= 48 || img.bgra.len() != size * size * 4 {
        return None;
    }
    let alpha = |x: usize, y: usize| img.bgra[(y * size + x) * 4 + 3];
    let ring_all = |r: usize, pred: &dyn Fn(u8) -> bool| {
        let last = size - 1 - r;
        (r..=last).all(|i| {
            pred(alpha(i, r)) && pred(alpha(i, last)) && pred(alpha(r, i)) && pred(alpha(last, i))
        })
    };
    let faint = |a: u8| a > 0 && a < 0x80;
    if !(ring_all(0, &faint) && ring_all(1, &faint) && ring_all(2, &|a| a == 0)) {
        return None;
    }
    // 边框以内的内容包围盒
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 3..size - 3 {
        for x in 3..size - 3 {
            if alpha(x, y) > 0 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 == usize::MAX {
        return None;
    }
    let native = ICON_SIZE_BUCKETS
        .iter()
        .map(|b| *b as usize)
        .filter(|n| *n <= size - 6)
        .find(|n| {
            let offset = (size - n) / 2;
            x0 >= offset && y0 >= offset && x1 < offset + n && y1 < offset + n
        })
        .unwrap_or(size - 6);
    let offset = (size - native) / 2;
    let mut bgra = Vec::with_capacity(native * native * 4);
    for y in offset..offset + native {
        let start = (y * size + offset) * 4;
        bgra.extend_from_slice(&img.bgra[start..start + native * 4]);
    }
    Some(RawImage {
        width: native as u32,
        height: native as u32,
        bgra,
    })
}

/// RGBA 8 位 → PNG 字节
#[cfg_attr(not(windows), allow(dead_code))] // 非 Windows 不提取图标，只有测试用到
pub(crate) fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| format!("编码图标失败: {}", e))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| format!("编码图标失败: {}", e))?;
    }
    Ok(out)
}

/// 先写临时文件再改名，避免前端读到写了一半的 PNG
#[cfg_attr(not(windows), allow(dead_code))] // 非 Windows 不提取图标，只有测试用到
fn write_atomically(target: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = target.with_extension(format!(
        "tmp{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&tmp, bytes).map_err(|e| format!("写入图标缓存失败: {}", e))?;
    fs::rename(&tmp, target).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("写入图标缓存失败: {}", e)
    })
}

/// 提取并写入缓存（在 `fence-icon` STA 线程上执行）
#[cfg(windows)]
fn render_icon(path: &Path, size_px: u32, target: &Path) -> Result<(), String> {
    use windows::Win32::UI::Shell::SIIGBF_ICONONLY;
    if target.is_file() {
        // 排队期间同一请求已经生成过
        return Ok(());
    }
    // 只取图标、不取缩略图：与桌面"中等图标"的观感一致，也避免缩略图生成拖慢（见 spike-icons.md）
    let image = super::shell::extract_image(path, size_px, SIIGBF_ICONONLY)?;
    let image = strip_small_icon_frame(&image).unwrap_or(image);
    let rgba = bgra_to_rgba(&image.bgra);
    let png = encode_png(image.width, image.height, &rgba)?;
    write_atomically(target, &png)
}

pub(crate) fn icon_png_in(cache_dir: &Path, path: &Path, size_px: u32) -> Result<PathBuf, String> {
    let (mtime_ms, len) = file_stamp(path)?;
    let bucket = icon_bucket(size_px);
    let target = cache_dir.join(icon_cache_key(path, mtime_ms, len, bucket));
    if target.is_file() {
        return Ok(target);
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(cache_dir).map_err(|e| format!("创建图标缓存目录失败: {}", e))?;
        let source = path.to_path_buf();
        let output = target.clone();
        super::sta::icon_worker()?.run(move || render_icon(&source, bucket, &output))??;
        Ok(target)
    }
    #[cfg(not(windows))]
    {
        Err(super::UNSUPPORTED.to_string())
    }
}

/// 取某个文件 / 文件夹 / 快捷方式的图标 PNG 路径（命中缓存直接返回，否则提取后写入缓存）。
///
/// * `size_px`：期望的**物理像素**边长（前端按 `图标逻辑尺寸 × devicePixelRatio` 计算），
///   向上取整到 [`ICON_SIZE_BUCKETS`] 档位。返回的 PNG 是正方形，边长通常等于档位；
///   只有小图标资源（≤48 px）的程序按大档位请求时返回其原尺寸（如 48×48，已去掉 Shell 加的边框），
///   前端统一用 CSS 固定宽高显示即可（会被拉伸到格子大小）
/// * `.lnk` 取目标程序的图标，不带快捷方式小箭头；`.url` 取浏览器 / 网站图标；文件夹取文件夹图标
/// * 文件不存在或提取失败返回 `Err`（前端显示占位图标）；非 Windows 返回 `Err`
/// * 阻塞调用（内部在常驻 STA 线程排队执行），请在 `spawn_blocking` 中调用
pub fn icon_png(path: &str, size_px: u32) -> Result<PathBuf, String> {
    icon_png_in(&icon_cache_dir(), Path::new(path), size_px)
}

fn remove_cached_matching(cache_dir: &Path, keep: impl Fn(&str) -> bool) -> Result<usize, String> {
    let read = match fs::read_dir(cache_dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("读取图标缓存目录失败: {}", e)),
    };
    let mut removed = 0;
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && !keep(&name)
            && fs::remove_file(entry.path()).is_ok()
        {
            removed += 1;
        }
    }
    Ok(removed)
}

pub(crate) fn prune_icon_cache_in<P: AsRef<Path>>(
    cache_dir: &Path,
    live_paths: &[P],
) -> Result<usize, String> {
    let live: HashSet<String> = live_paths
        .iter()
        .filter_map(|p| {
            let path = p.as_ref();
            file_stamp(path)
                .ok()
                .map(|(mtime_ms, len)| cache_stamp(path, mtime_ms, len))
        })
        .collect();
    remove_cached_matching(cache_dir, |name| {
        name.ends_with(".png")
            && name
                .rfind('-')
                .is_some_and(|idx| live.contains(&name[..=idx]))
    })
}

/// 清理图标缓存：只保留 `live_paths`（当前所有收纳箱里的项目完整路径）在**当前**修改时间 / 大小下的缓存，
/// 其余（已移走的项、旧版本、写了一半的临时文件）全部删除。建议应用启动对账完成后调用一次。返回删除的文件数。
pub fn prune_icon_cache<P: AsRef<Path>>(live_paths: &[P]) -> Result<usize, String> {
    prune_icon_cache_in(&icon_cache_dir(), live_paths)
}

#[cfg(test)]
mod tests {
    use super::super::folder::test_support::{touch, TempDir};
    use super::*;

    #[test]
    fn bucket_rounds_up() {
        assert_eq!(icon_bucket(0), 16);
        assert_eq!(icon_bucket(16), 16);
        assert_eq!(icon_bucket(17), 24);
        assert_eq!(icon_bucket(48), 48);
        assert_eq!(icon_bucket(60), 64);
        assert_eq!(icon_bucket(72), 96);
        assert_eq!(icon_bucket(144), 256);
        assert_eq!(icon_bucket(1000), 256);
    }

    #[test]
    fn fnv_known_vectors() {
        // FNV-1a 64 官方测试向量
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn cache_key_is_stable_and_case_insensitive() {
        let a = icon_cache_key(Path::new(r"C:\Users\Me\Desktop\Chrome.lnk"), 1000, 2048, 48);
        let b = icon_cache_key(Path::new(r"c:\users\me\desktop\chrome.LNK"), 1000, 2048, 48);
        let c = icon_cache_key(
            Path::new(r"\\?\C:\Users\Me\Desktop\Chrome.lnk"),
            1000,
            2048,
            48,
        );
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert!(a.ends_with("-3e8-800-48.png"), "{a}");
        assert_eq!(a.len(), 16 + "-3e8-800-48.png".len());
        // 修改时间 / 大小 / 尺寸 / 路径任一变化，键都变
        assert_ne!(
            a,
            icon_cache_key(Path::new(r"C:\Users\Me\Desktop\Chrome.lnk"), 1001, 2048, 48)
        );
        assert_ne!(
            a,
            icon_cache_key(Path::new(r"C:\Users\Me\Desktop\Chrome.lnk"), 1000, 2049, 48)
        );
        assert_ne!(
            a,
            icon_cache_key(Path::new(r"C:\Users\Me\Desktop\Chrome.lnk"), 1000, 2048, 64)
        );
        assert_ne!(
            a,
            icon_cache_key(Path::new(r"C:\Users\Me\Desktop\Edge.lnk"), 1000, 2048, 48)
        );
    }

    #[test]
    fn rgba_conversion_handles_alpha_modes() {
        // 无 alpha 通道：全部不透明
        let rgba = bgra_to_rgba(&[10, 20, 30, 0, 40, 50, 60, 0]);
        assert_eq!(rgba, vec![30, 20, 10, 255, 60, 50, 40, 255]);

        // 直通 alpha：颜色原样保留（即使所有像素都满足 rgb <= a，也不做反预乘），BGR → RGB
        let rgba = bgra_to_rgba(&[40, 40, 40, 128, 0, 0, 255, 255, 0, 0, 0, 0]);
        assert_eq!(&rgba[0..4], &[40, 40, 40, 128]);
        assert_eq!(&rgba[4..8], &[255, 0, 0, 255]);
        assert_eq!(&rgba[8..12], &[0, 0, 0, 0]);

        // 完全透明像素颜色清零
        let rgba = bgra_to_rgba(&[200, 100, 50, 100, 9, 9, 9, 0]);
        assert_eq!(&rgba[0..4], &[50, 100, 200, 100]);
        assert_eq!(&rgba[4..8], &[0, 0, 0, 0]);
    }

    /// 构造 Shell 的"小图标 + 边框"位图：size 画布，最外两圈半透明，中间放 native×native 的不透明内容
    fn framed(size: usize, native: usize, content_inset: usize) -> RawImage {
        let mut bgra = vec![0u8; size * size * 4];
        let set = |bgra: &mut Vec<u8>, x: usize, y: usize, px: [u8; 4]| {
            bgra[(y * size + x) * 4..(y * size + x) * 4 + 4].copy_from_slice(&px);
        };
        for i in 0..size {
            for (r, px) in [(0usize, [100, 100, 100, 0x30]), (1, [255, 255, 255, 0x24])] {
                let last = size - 1 - r;
                if i >= r && i <= last {
                    set(&mut bgra, i, r, px);
                    set(&mut bgra, i, last, px);
                    set(&mut bgra, r, i, px);
                    set(&mut bgra, last, i, px);
                }
            }
        }
        let offset = (size - native) / 2;
        for y in offset + content_inset..offset + native - content_inset {
            for x in offset + content_inset..offset + native - content_inset {
                set(&mut bgra, x, y, [(x % 256) as u8, (y % 256) as u8, 7, 255]);
            }
        }
        RawImage {
            width: size as u32,
            height: size as u32,
            bgra,
        }
    }

    #[test]
    fn strips_shell_frame_around_small_icons() {
        let img = framed(96, 48, 2);
        let cropped = strip_small_icon_frame(&img).expect("应识别出边框");
        assert_eq!((cropped.width, cropped.height), (48, 48));
        // 裁出的是画布正中 48×48 的原图
        let offset = 24;
        for y in 0..48 {
            for x in 0..48 {
                let src = ((y + offset) * 96 + x + offset) * 4;
                let dst = (y * 48 + x) * 4;
                assert_eq!(&cropped.bgra[dst..dst + 4], &img.bgra[src..src + 4]);
            }
        }
        // 64 画布 + 32 原图
        let cropped = strip_small_icon_frame(&framed(64, 32, 1)).unwrap();
        assert_eq!(cropped.width, 32);
        // 内容比 48 大时取能容纳它的档位
        let cropped = strip_small_icon_frame(&framed(96, 64, 0)).unwrap();
        assert_eq!(cropped.width, 64);
    }

    #[test]
    fn leaves_normal_icons_alone() {
        // 没有边框：内容铺满
        let mut img = framed(96, 96, 0);
        assert!(strip_small_icon_frame(&img).is_none());
        // 第三圈不透明（不是"边框 + 空隙"形态）
        img = framed(96, 92, 0);
        assert!(strip_small_icon_frame(&img).is_none());
        // 只有边框没有内容
        let empty = framed(96, 48, 24);
        assert!(strip_small_icon_frame(&empty).is_none());
        // 48 及以下不处理；非正方形不处理
        assert!(strip_small_icon_frame(&framed(48, 32, 0)).is_none());
        let wide = RawImage {
            width: 96,
            height: 60,
            bgra: vec![0; 96 * 60 * 4],
        };
        assert!(strip_small_icon_frame(&wide).is_none());
    }

    #[test]
    fn png_round_trip() {
        let rgba = vec![255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 1, 2, 3, 4];
        let bytes = encode_png(2, 2, &rgba).unwrap();
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let info = reader.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(info.color_type, png::ColorType::Rgba);
        assert_eq!(&buf[..info.buffer_size()], &rgba[..]);
    }

    #[test]
    fn prune_cache() {
        let tmp = TempDir::new("icon-cache");
        let cache = tmp.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let live = tmp.path().join("live.lnk");
        let gone = tmp.path().join("gone.lnk");
        touch(&live);
        let (mtime, len) = file_stamp(&live).unwrap();
        let live48 = icon_cache_key(&live, mtime, len, 48);
        let live96 = icon_cache_key(&live, mtime, len, 96);
        let stale = icon_cache_key(&live, mtime.wrapping_sub(5000), len, 48);
        let gone48 = icon_cache_key(&gone, 1, 1, 48);
        for name in [&live48, &live96, &stale, &gone48] {
            touch(&cache.join(name));
        }
        touch(&cache.join(format!("{live48}.tmp123-0")));

        let removed = prune_icon_cache_in(&cache, &[&live, &gone]).unwrap();
        assert_eq!(removed, 3);
        assert!(cache.join(&live48).is_file());
        assert!(cache.join(&live96).is_file());
        assert!(!cache.join(&stale).exists());
        assert!(!cache.join(&gone48).exists());
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 2);
        // 缓存目录不存在
        assert_eq!(
            prune_icon_cache_in::<&Path>(&tmp.path().join("none"), &[]).unwrap(),
            0
        );
    }

    #[test]
    fn missing_file_is_an_error() {
        let tmp = TempDir::new("icon-missing");
        assert!(icon_png_in(tmp.path(), &tmp.path().join("nope.lnk"), 48).is_err());
    }

    /// 真实提取：系统自带程序与临时文件夹，结果写入临时缓存目录
    #[cfg(windows)]
    #[test]
    fn extracts_real_icons_into_cache() {
        let tmp = TempDir::new("icon-real");
        let cache = tmp.path().join("cache");
        let notepad = PathBuf::from(std::env::var("SystemRoot").unwrap_or("C:\\Windows".into()))
            .join("System32")
            .join("notepad.exe");
        let folder = tmp.path().join("文件夹");
        fs::create_dir_all(&folder).unwrap();
        for (path, size, expect) in [(&notepad, 60, 64), (&folder, 48, 48)] {
            let png_path = icon_png_in(&cache, path, size).unwrap();
            assert!(png_path.starts_with(&cache));
            let decoder = png::Decoder::new(fs::File::open(&png_path).unwrap());
            let reader = decoder.read_info().unwrap();
            assert_eq!(reader.info().width, expect);
            assert_eq!(reader.info().height, expect);
            // 第二次命中缓存，路径相同
            assert_eq!(icon_png_in(&cache, path, size).unwrap(), png_path);
        }
    }
}

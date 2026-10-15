//! Markdown 图片引用（旧 `utils/imageRef.ts` 的 1:1 移植，跨端契约 K5）。
//!
//! 规范形式：`minitodo-image://<name>`，`<name>` 是本机 images 目录下的安全文件名。数据库、导出文件与
//! WebDAV 同步数据里只存规范形式，cloud / AI 通过 `GET /images/<name>` 取图。
//!
//! - 渲染：GPUI 版不再经 asset 协议，`resolve_image_path` 直接把规范引用与旧 asset URL 映射成
//!   `<images 目录>/<name>`（`img(PathBuf)`）；[`to_display_markdown`] / [`to_asset_url`] 只为兼容测试保留。
//! - 保存（[`to_storage_markdown`]）：父目录名为 `images` 的旧 asset URL 一律换回规范形式（幂等）。
//!
//! 兼容旧数据：旧版本直接存 `convertFileSrc(本机绝对路径)`，即
//! `http(s)://asset.localhost/<url 编码的绝对路径>`（Windows）或 `asset://localhost/<…>`（macOS / Linux）。
//! 按文件名映射到本机 images 目录，修复跨设备裂图。
//!
//! 匹配规则（原正则）：
//! ```text
//! minitodo-image://(SAFE_NAME)NAME_END
//! | (?:https?://asset\.localhost|asset://localhost)/PATH_CHAR*?SEP images SEP(SAFE_NAME)NAME_END
//! ```
//! `regex` crate 没有零宽断言，懒惰量词失败后也不会像 JS 那样回溯找下一个 `images` 段，所以这里手写匹配器。

use std::path::{Path, PathBuf};

/// 规范引用前缀
pub const IMAGE_REF_SCHEME: &str = "minitodo-image://";

/// 编辑器允许上传的图片扩展名（大小写不敏感，存小写）
pub const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];

/// 单张图片上限 20MB（后端同样校验）
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;

const SAFE_NAME_MAX: usize = 128;
const ASSET_PREFIXES: [&str; 3] = [
    "http://asset.localhost/",
    "https://asset.localhost/",
    "asset://localhost/",
];

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn is_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')
}

/// 是否为允许落盘 / 同步的安全图片文件名：`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$` 且不含 `..`
pub fn is_safe_image_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= SAFE_NAME_MAX
        && is_name_start(bytes[0])
        && bytes.iter().all(|b| is_name_char(*b))
        && !name.contains("..")
}

/// 规范引用 `minitodo-image://<name>`
pub fn to_image_ref(name: &str) -> String {
    format!("{IMAGE_REF_SCHEME}{name}")
}

/// `SAFE_NAME NAME_END` 从 `at` 开始；成功返回名字结束位置。
/// 名字取最长（≤128）；名字后紧跟名字字符或 `%` 时失败（更短的名字后面也跟着名字字符，同样失败）。
fn match_name(bytes: &[u8], at: usize) -> Option<usize> {
    if at >= bytes.len() || !is_name_start(bytes[at]) {
        return None;
    }
    let mut end = at + 1;
    while end < bytes.len() && end - at < SAFE_NAME_MAX && is_name_char(bytes[end]) {
        end += 1;
    }
    match bytes.get(end) {
        Some(b) if is_name_char(*b) || *b == b'%' => None,
        _ => Some(end),
    }
}

/// `SEP`：`%5C` / `%2F`（大小写不敏感的十六进制）或 `/`；返回长度
fn match_sep(bytes: &[u8], at: usize) -> Option<usize> {
    match bytes.get(at)? {
        b'/' => Some(1),
        b'%' => {
            let hi = *bytes.get(at + 1)?;
            let lo = *bytes.get(at + 2)?;
            match (hi, lo) {
                (b'5', b'C' | b'c') | (b'2', b'F' | b'f') => Some(3),
                _ => None,
            }
        }
        _ => None,
    }
}

/// `PATH_CHAR` 的一个记号：`[A-Za-z0-9\-_.!~*'()/]` | `\(` `\)` | `%XX`；返回长度
fn path_char_len(bytes: &[u8], at: usize) -> Option<usize> {
    let b = *bytes.get(at)?;
    if b.is_ascii_alphanumeric()
        || matches!(
            b,
            b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' | b'/'
        )
    {
        return Some(1);
    }
    match b {
        b'\\' if matches!(bytes.get(at + 1), Some(b'(' | b')')) => Some(2),
        b'%' if bytes.get(at + 1).is_some_and(u8::is_ascii_hexdigit)
            && bytes.get(at + 2).is_some_and(u8::is_ascii_hexdigit) =>
        {
            Some(3)
        }
        _ => None,
    }
}

/// `SEP images SEP SAFE_NAME NAME_END` 从 `at` 开始；成功返回 (名字起点, 名字终点)
fn match_images_tail(bytes: &[u8], at: usize) -> Option<(usize, usize)> {
    let mut i = at + match_sep(bytes, at)?;
    if !bytes.get(i..)?.starts_with(b"images") {
        return None;
    }
    i += "images".len();
    i += match_sep(bytes, i)?;
    let end = match_name(bytes, i)?;
    Some((i, end))
}

/// 在 `at` 处匹配一个图片引用：返回 (整段结束位置, 名字起点, 名字终点)
fn match_ref_at(bytes: &[u8], at: usize) -> Option<(usize, usize, usize)> {
    let rest = &bytes[at..];
    if rest.starts_with(IMAGE_REF_SCHEME.as_bytes()) {
        let start = at + IMAGE_REF_SCHEME.len();
        return match_name(bytes, start).map(|end| (end, start, end));
    }
    let prefix = ASSET_PREFIXES
        .iter()
        .find(|p| rest.starts_with(p.as_bytes()))?;
    // `PATH_CHAR*?`：从最短开始，每次多吃一个记号，直到后面接得上 `SEP images SEP 名字`
    let mut i = at + prefix.len();
    loop {
        if let Some((name_start, name_end)) = match_images_tail(bytes, i) {
            return Some((name_end, name_start, name_end));
        }
        i += path_char_len(bytes, i)?;
    }
}

/// 对每个图片引用取文件名，安全的替换为 `replacer(name)`，否则保留原文（旧 `replaceImageRefs`）
fn replace_image_refs(markdown: &str, replacer: impl Fn(&str) -> String) -> String {
    let bytes = markdown.as_bytes();
    let mut out = String::with_capacity(markdown.len());
    let mut copied = 0;
    let mut i = 0;
    while i < bytes.len() {
        // 匹配只可能从 ASCII 字符开始，非 UTF-8 字符边界不会命中
        if let Some((end, name_start, name_end)) = match_ref_at(bytes, i) {
            let name = &markdown[name_start..name_end];
            out.push_str(&markdown[copied..i]);
            if is_safe_image_name(name) {
                out.push_str(&replacer(name));
            } else {
                out.push_str(&markdown[i..end]);
            }
            copied = end;
            i = end;
        } else {
            i += 1;
        }
    }
    out.push_str(&markdown[copied..]);
    out
}

/// 渲染形式 / 旧数据 → 存储形式：父目录为 images 的 asset URL 换回规范引用（幂等）
pub fn to_storage_markdown(markdown: &str) -> String {
    if markdown.is_empty() {
        return String::new();
    }
    replace_image_refs(markdown, to_image_ref)
}

/// 一个完整的图片地址（Markdown 里的 `![](…)` 目标）指向的本机图片文件名：
/// 规范引用或父目录为 images 的旧 asset URL，且文件名安全；其它返回 `None`
pub fn image_name_of_url(url: &str) -> Option<&str> {
    let bytes = url.as_bytes();
    let (end, name_start, name_end) = match_ref_at(bytes, 0)?;
    let name = &url[name_start..name_end];
    (end == bytes.len() && is_safe_image_name(name)).then_some(name)
}

/// 图片地址 → 本机图片路径（GPUI 渲染用 `img(PathBuf)`）；不是本机图片引用时返回 `None`
pub fn resolve_image_path(url: &str, images_dir: &Path) -> Option<PathBuf> {
    image_name_of_url(url).map(|name| images_dir.join(name))
}

fn join_path(dir: &str, name: &str) -> String {
    let sep = if dir.contains('\\') && !dir.contains('/') {
        '\\'
    } else {
        '/'
    };
    if dir.ends_with('\\') || dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}{sep}{name}")
    }
}

/// JS `encodeURIComponent`：除 `A-Z a-z 0-9 - _ . ! ~ * ' ( )` 外按 UTF-8 百分号编码（大写十六进制）
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(
                b,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// 旧版 Windows `convertFileSrc` + 括号编码（兼容测试用；GPUI 渲染不经 asset 协议）
pub fn to_asset_url(path: &str) -> String {
    format!("http://asset.localhost/{}", encode_uri_component(path))
        .replace('(', "%28")
        .replace(')', "%29")
}

/// 存储形式 → 旧渲染形式（兼容测试用）：`images_dir` 为空时原样返回
pub fn to_display_markdown(markdown: &str, images_dir: Option<&str>) -> String {
    match images_dir {
        Some(dir) if !markdown.is_empty() && !dir.is_empty() => {
            replace_image_refs(markdown, |name| to_asset_url(&join_path(dir, name)))
        }
        _ => markdown.to_string(),
    }
}

/// 推断上传图片的扩展名（小写）：优先 MIME（粘贴的截图文件名往往是通用的 image.png），
/// 其次文件名；不在白名单内返回 `None`
pub fn resolve_image_extension(name: &str, mime: &str) -> Option<String> {
    let from_mime = match mime.to_ascii_lowercase().as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" | "image/pjpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/bmp" | "image/x-ms-bmp" => Some("bmp"),
        _ => None,
    };
    if let Some(ext) = from_mime {
        return Some(ext.to_string());
    }
    let dot = name.rfind('.')?;
    let ext = name[dot + 1..].to_lowercase();
    IMAGE_EXTENSIONS.contains(&ext.as_str()).then_some(ext)
}

#[cfg(test)]
mod tests {
    //! 原 `imageRef.test.ts`（Windows 形态的 convertFileSrc）
    use super::*;

    const WIN_DIR: &str = r"C:\Users\bob\AppData\Local\mini-todo\images";
    const MAC_DIR: &str = "/Users/bob/Library/Application Support/mini-todo/images";

    /// 旧版本存进 Markdown 的图片地址：convertFileSrc(本机绝对路径)
    fn legacy_windows_url(dir: &str, name: &str, origin: &str) -> String {
        format!(
            "{origin}/{}",
            encode_uri_component(&format!("{dir}\\{name}"))
        )
    }

    fn legacy(dir: &str, name: &str) -> String {
        legacy_windows_url(dir, name, "http://asset.localhost")
    }

    #[test]
    fn accepts_plain_single_path_segments() {
        let long = "A".repeat(128);
        for name in [
            "a",
            "img_1.png",
            "1700000000000_ab12cd.png",
            "img_1727-x.JPEG",
            long.as_str(),
        ] {
            assert!(is_safe_image_name(name), "{name}");
        }
    }

    #[test]
    fn rejects_traversal_separators_odd_characters_and_overlong_names() {
        let long = "A".repeat(129);
        for name in [
            "",
            ".hidden.png",
            "-x.png",
            "../x.png",
            "a..b.png",
            "a/b.png",
            r"a\b.png",
            "a b.png",
            "a%2Fb.png",
            "图片.png",
            long.as_str(),
        ] {
            assert!(!is_safe_image_name(name), "{name}");
        }
    }

    #[test]
    fn keeps_canonical_references_unchanged() {
        let md =
            format!("![a]({IMAGE_REF_SCHEME}1700_a.png)\n\ntext ![b](minitodo-image://b.webp)");
        assert_eq!(to_storage_markdown(&md), md);
        assert_eq!(to_storage_markdown(&to_storage_markdown(&md)), md);
    }

    #[test]
    fn rewrites_legacy_windows_asset_urls_whose_parent_is_images() {
        let md = format!("before ![shot]({}) after", legacy(WIN_DIR, "1700_abc.png"));
        assert_eq!(
            to_storage_markdown(&md),
            "before ![shot](minitodo-image://1700_abc.png) after"
        );
    }

    #[test]
    fn handles_https_asset_origins_and_asset_localhost() {
        let https = legacy_windows_url(WIN_DIR, "x.png", "https://asset.localhost");
        assert_eq!(
            to_storage_markdown(&format!("![x]({https})")),
            "![x](minitodo-image://x.png)"
        );
        let mac = format!(
            "asset://localhost/{}",
            encode_uri_component(&format!("{MAC_DIR}/y.gif"))
        );
        assert_eq!(
            to_storage_markdown(&format!("![y]({mac})")),
            "![y](minitodo-image://y.gif)"
        );
        // 未编码的 / 分隔符同样识别
        assert_eq!(
            to_storage_markdown(
                "![z](asset://localhost/home/bob/.local/share/mini-todo/images/z.png)"
            ),
            "![z](minitodo-image://z.png)"
        );
    }

    #[test]
    fn rewrites_every_image_including_other_users_and_devices() {
        let other = legacy(r"D:\Data\alice\mini-todo\images", "2.png");
        let md = format!(
            "![1]({})\n![2]({other})\n![3](minitodo-image://3.png)",
            legacy(WIN_DIR, "1.png")
        );
        assert_eq!(
            to_storage_markdown(&md),
            "![1](minitodo-image://1.png)\n![2](minitodo-image://2.png)\n![3](minitodo-image://3.png)"
        );
    }

    #[test]
    fn leaves_unrelated_or_unsafe_urls_untouched() {
        let cases = [
            // 父目录不是 images
            format!("![p]({})", legacy(r"C:\Users\bob\Pictures", "p.png")),
            // 不是 asset 协议
            "![w](https://example.com/images/w.png)".to_string(),
            // 文件名不安全
            format!(
                "![u](http://asset.localhost/{})",
                encode_uri_component(&format!("{WIN_DIR}\\..\\secret.png"))
            ),
            "![u](minitodo-image://../secret.png)".to_string(),
            // 普通链接与文本
            "[link](https://example.com) and `minitodo-image` in code".to_string(),
        ];
        for md in cases {
            assert_eq!(to_storage_markdown(&md), md, "{md}");
        }
    }

    #[test]
    fn returns_empty_input_as_is() {
        assert_eq!(to_storage_markdown(""), "");
    }

    #[test]
    fn display_maps_canonical_references_to_the_local_images_directory() {
        assert_eq!(
            to_display_markdown("![a](minitodo-image://a.png)", Some(WIN_DIR)),
            format!(
                "![a](http://asset.localhost/{})",
                encode_uri_component(&format!("{WIN_DIR}\\a.png"))
            )
        );
        assert_eq!(
            to_display_markdown("![a](minitodo-image://a.png)", Some(MAC_DIR)),
            format!(
                "![a](http://asset.localhost/{})",
                encode_uri_component(&format!("{MAC_DIR}/a.png"))
            )
        );
    }

    #[test]
    fn display_does_not_double_a_trailing_separator() {
        assert_eq!(
            to_display_markdown(
                "![a](minitodo-image://a.png)",
                Some(&format!("{WIN_DIR}\\"))
            ),
            format!(
                "![a](http://asset.localhost/{})",
                encode_uri_component(&format!("{WIN_DIR}\\a.png"))
            )
        );
    }

    #[test]
    fn display_remaps_legacy_urls_from_another_device() {
        let foreign = legacy(r"D:\Data\alice\mini-todo\images", "f.png");
        assert_eq!(
            to_display_markdown(&format!("![f]({foreign})"), Some(WIN_DIR)),
            format!(
                "![f](http://asset.localhost/{})",
                encode_uri_component(&format!("{WIN_DIR}\\f.png"))
            )
        );
    }

    #[test]
    fn display_is_reversed_by_storage() {
        let md = "# t\n\n![a](minitodo-image://a.png) text ![b](minitodo-image://b.jpg)";
        assert_eq!(
            to_storage_markdown(&to_display_markdown(md, Some(WIN_DIR))),
            md
        );
        assert_eq!(
            to_storage_markdown(&to_display_markdown(md, Some(MAC_DIR))),
            md
        );
    }

    #[test]
    fn display_leaves_content_untouched_without_an_images_directory() {
        let md = "![a](minitodo-image://a.png)";
        assert_eq!(to_display_markdown(md, None), md);
    }

    #[test]
    fn display_never_puts_raw_parentheses_into_the_link_destination() {
        for dir in [
            r"C:\Users\bob(\AppData\Local\mini-todo\images",
            r"C:\Users\a)b\AppData\Local\mini-todo\images",
            r"C:\Program Files (x86)\mini-todo\images",
        ] {
            let md = "![a](minitodo-image://1_a.png) tail";
            let display = to_display_markdown(md, Some(dir));
            let url = display
                .strip_prefix("![a](")
                .and_then(|s| s.strip_suffix(") tail"))
                .unwrap_or_else(|| panic!("{display}"));
            assert!(!url.contains(['(', ')']), "{url}");
            assert_eq!(to_storage_markdown(&display), md);
        }
    }

    #[test]
    fn asset_url_percent_encodes_parentheses() {
        assert_eq!(
            to_asset_url(r"C:\Users\bob(1)\x.png"),
            format!(
                "http://asset.localhost/{}%281%29{}",
                encode_uri_component(r"C:\Users\bob"),
                encode_uri_component(r"\x.png")
            )
        );
    }

    #[test]
    fn accepts_serializer_escaped_legacy_urls_and_angle_destinations() {
        let encoded = encode_uri_component(r"C:\Program Files (x86)\mini-todo\images\1_a.png");
        let escaped = encoded.replace('(', r"\(").replace(')', r"\)");
        assert_eq!(
            to_storage_markdown(&format!("![a](http://asset.localhost/{escaped})")),
            "![a](minitodo-image://1_a.png)"
        );
        assert_eq!(
            to_storage_markdown(&format!("![a](<http://asset.localhost/{encoded}>)")),
            "![a](<minitodo-image://1_a.png>)"
        );
    }

    #[test]
    fn builds_the_canonical_reference() {
        assert_eq!(to_image_ref("a.png"), "minitodo-image://a.png");
    }

    #[test]
    fn resolve_extension_prefers_mime_then_file_name() {
        assert_eq!(
            resolve_image_extension("image.png", "image/jpeg").as_deref(),
            Some("jpg")
        );
        assert_eq!(
            resolve_image_extension("clip", "IMAGE/PNG").as_deref(),
            Some("png")
        );
        assert_eq!(
            resolve_image_extension("x", "image/x-ms-bmp").as_deref(),
            Some("bmp")
        );
        assert_eq!(
            resolve_image_extension("Photo.JPEG", "").as_deref(),
            Some("jpeg")
        );
        assert_eq!(
            resolve_image_extension("a.b.webp", "application/octet-stream").as_deref(),
            Some("webp")
        );
        assert_eq!(resolve_image_extension("vector.svg", "image/svg+xml"), None);
        assert_eq!(resolve_image_extension("noext", ""), None);
        assert_eq!(resolve_image_extension("doc.tiff", "image/tiff"), None);
    }

    #[test]
    fn resolves_image_urls_to_local_paths() {
        let dir = Path::new("/data/images");
        assert_eq!(
            resolve_image_path("minitodo-image://a.png", dir),
            Some(dir.join("a.png"))
        );
        assert_eq!(
            resolve_image_path(&legacy(r"D:\x\images", "b.png"), dir),
            Some(dir.join("b.png"))
        );
        assert_eq!(resolve_image_path("minitodo-image://../a.png", dir), None);
        assert_eq!(
            resolve_image_path("https://example.com/images/a.png", dir),
            None
        );
        // 名字之后还有内容：不是一个完整的引用
        assert_eq!(resolve_image_path("minitodo-image://a.png?x", dir), None);
    }

    #[test]
    fn lazy_path_backtracks_to_a_later_images_segment() {
        // 第一个 images 段后的名字不完整（后跟 %），JS 会继续往后找下一个 images 段
        let md = "![a](asset://localhost/x/images/a%20b/images/c.png)";
        assert_eq!(to_storage_markdown(md), "![a](minitodo-image://c.png)");
    }
}

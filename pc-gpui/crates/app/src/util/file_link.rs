//! Markdown 链接的协议白名单（旧 `utils/fileLink.ts` 的 1:1 移植）。
//!
//! - http / https / mailto：交给系统默认程序（`cx.open_url`，用 WHATWG 规范化后的地址）
//! - `file:///` 本机绝对路径：在资源管理器中定位（`cx.reveal_path`）；UNC（`file://host/share`、
//!   `file:////host`）一律拒绝——交给系统会去连 SMB 共享，可能泄露 NTLM 凭据
//! - 其余（javascript: / data: / vbscript: / blob: / ftp: / 相对地址 / `#anchor` …）：什么都不做
//!
//! 只读排版（详情、预览）普通单击即打开；编辑排版下普通点击留给编辑器，只有 Ctrl/Cmd+单击才打开外部链接
//! （file:/// 单击即定位）。GPUI 版的链接只出现在渲染结果里（编辑器是源码），调用方按只读语义传参。

use gpui_kit::App;

/// 链接的去向
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// http / https / mailto：交给系统默认程序
    External(String),
    /// file:///：在资源管理器中定位
    File(String),
    /// 白名单外：什么都不做
    Blocked,
}

/// 当前平台是否 Windows（决定 file:/// 路径的形态）
pub const IS_WINDOWS: bool = cfg!(windows);

/// JS `decodeURIComponent`：`%` 后必须是两位十六进制，结果必须是合法 UTF-8，否则 `None`
pub fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `file:///` 链接 → 本机绝对路径；不是本机绝对路径时返回 `None`。
/// 只接受三斜杠形式；先去掉查询串与片段再解码（编码过的 `%23` / `%3F` 属于文件名）
pub fn extract_file_path(href: &str, windows: bool) -> Option<String> {
    const PREFIX: &str = "file:///";
    if !href
        .get(..PREFIX.len())
        .is_some_and(|p| p.eq_ignore_ascii_case(PREFIX))
    {
        return None;
    }
    let raw = &href[PREFIX.len()..];
    let raw = raw.split(['?', '#']).next().unwrap_or_default();
    let rest = decode_uri_component(raw)?;
    if windows {
        let path = rest.replace('/', "\\");
        // 必须是盘符开头的绝对路径（顺带排除 \\server\share 与 \\?\ 前缀）
        let b = path.as_bytes();
        let ok = b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\';
        ok.then_some(path)
    } else {
        let path = format!("/{rest}");
        (!path.starts_with("//")).then_some(path)
    }
}

/// 按协议白名单解析链接（纯函数）
pub fn resolve_link_target(href: &str, windows: bool) -> LinkTarget {
    let raw = href.trim();
    if raw.is_empty() {
        return LinkTarget::Blocked;
    }
    // 与浏览器导航同一套解析规则（WHATWG）：大小写、内嵌的制表符 / 换行都会被规范化，
    // "JaVa\tScript:" 之类的写法绕不过去；相对地址没有 base 直接解析失败
    let Ok(url) = url::Url::parse(raw) else {
        return LinkTarget::Blocked;
    };
    match url.scheme() {
        "http" | "https" | "mailto" => LinkTarget::External(url.to_string()),
        "file" => extract_file_path(raw, windows).map_or(LinkTarget::Blocked, LinkTarget::File),
        _ => LinkTarget::Blocked,
    }
}

/// 一次链接点击的处理结果（旧 `handleLinkClick` 的判定部分）
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkAction {
    /// 不消费（编辑排版的普通点击：交给编辑器放光标）
    Ignore,
    /// 用系统默认程序打开
    Open(String),
    /// 在资源管理器中定位
    Reveal(String),
    /// 已消费但什么都不做（白名单外）
    Blocked,
}

/// 旧 `handleLinkClick`：`readonly` = 详情 / 预览排版；`modifier` = Ctrl 或 Cmd 按下
pub fn link_click_action(href: &str, readonly: bool, modifier: bool, windows: bool) -> LinkAction {
    match resolve_link_target(href, windows) {
        LinkTarget::File(path) => LinkAction::Reveal(path),
        LinkTarget::External(url) if readonly || modifier => LinkAction::Open(url),
        LinkTarget::External(_) => LinkAction::Ignore,
        LinkTarget::Blocked => LinkAction::Blocked,
    }
}

/// 执行链接点击（只读语义）；返回是否已消费
pub fn handle_link_click(href: &str, readonly: bool, modifier: bool, cx: &mut App) -> bool {
    match link_click_action(href, readonly, modifier, IS_WINDOWS) {
        LinkAction::Ignore => false,
        LinkAction::Open(url) => {
            cx.open_url(&url);
            true
        }
        LinkAction::Reveal(path) => {
            cx.reveal_path(std::path::Path::new(&path));
            true
        }
        LinkAction::Blocked => {
            log::info!("[link] 已拦截白名单外的链接: {href}");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    //! 原 `fileLink.test.ts`（DOM 相关的"同一事件只处理一次 / 优先 data-href"在 GPUI 版不存在：
    //! TextView 直接给出原始地址、每次点击只回调一次）
    use super::*;

    #[test]
    fn maps_file_urls_to_windows_drive_paths() {
        assert_eq!(
            extract_file_path("file:///C:/Users/bob/notes.txt", true).as_deref(),
            Some(r"C:\Users\bob\notes.txt")
        );
        assert_eq!(
            extract_file_path("FILE:///d:/a%20b/c.md", true).as_deref(),
            Some(r"d:\a b\c.md")
        );
    }

    #[test]
    fn strips_query_and_fragment_before_decoding() {
        assert_eq!(
            extract_file_path("file:///C:/a/b.txt?x=1#frag", true).as_deref(),
            Some(r"C:\a\b.txt")
        );
        // 编码过的 # / ? 属于文件名
        assert_eq!(
            extract_file_path("file:///C:/a/%23b%3F.txt", true).as_deref(),
            Some(r"C:\a\#b?.txt")
        );
    }

    #[test]
    fn rejects_unc_and_non_absolute_windows_paths() {
        for href in [
            "file://server/share/a.txt",
            "file://///server/share/a.txt",
            "file:////server/share/a.txt",
            "file:///relative/a.txt",
            "file:///%5C%5C?%5CC:%5Ca.txt",
        ] {
            assert_eq!(extract_file_path(href, true), None, "{href}");
        }
    }

    #[test]
    fn maps_file_urls_to_posix_paths_and_rejects_network_paths() {
        assert_eq!(
            extract_file_path("file:///home/bob/a%20b.txt", false).as_deref(),
            Some("/home/bob/a b.txt")
        );
        assert_eq!(extract_file_path("file:////server/share", false), None);
    }

    #[test]
    fn rejects_malformed_percent_encoding_and_other_schemes() {
        assert_eq!(extract_file_path("file:///C:/a%E0%A4%A.txt", true), None);
        assert_eq!(extract_file_path("https://example.com/a", true), None);
        assert_eq!(extract_file_path(r"C:\a.txt", true), None);
    }

    #[test]
    fn sends_http_https_mailto_to_the_system_handler() {
        assert_eq!(
            resolve_link_target("https://example.com/a?b=1", IS_WINDOWS),
            LinkTarget::External("https://example.com/a?b=1".into())
        );
        assert_eq!(
            resolve_link_target("HTTP://Example.com", IS_WINDOWS),
            LinkTarget::External("http://example.com/".into())
        );
        assert_eq!(
            resolve_link_target("mailto:bob@example.com", IS_WINDOWS),
            LinkTarget::External("mailto:bob@example.com".into())
        );
        assert_eq!(
            resolve_link_target("  https://example.com/a b  ", IS_WINDOWS),
            LinkTarget::External("https://example.com/a%20b".into())
        );
    }

    #[test]
    fn reveals_local_files_only_for_triple_slash_paths() {
        assert_eq!(
            resolve_link_target("file:///C:/a/b.txt", true),
            LinkTarget::File(r"C:\a\b.txt".into())
        );
        assert_eq!(
            resolve_link_target("file:///home/bob/b.txt", false),
            LinkTarget::File("/home/bob/b.txt".into())
        );
        assert_eq!(
            resolve_link_target("file://server/share/x", true),
            LinkTarget::Blocked
        );
    }

    #[test]
    fn blocks_script_capable_and_unknown_schemes() {
        for href in [
            "javascript:alert(1)",
            "JaVaScRiPt:alert(1)",
            "java\tscript:alert(1)",
            " javascript:alert(1)",
            "java\nscript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "blob:https://example.com/uuid",
            "ftp://example.com/a",
            "tauri://localhost",
            "asset://localhost/C%3A%5Ca.txt",
        ] {
            assert_eq!(
                resolve_link_target(href, IS_WINDOWS),
                LinkTarget::Blocked,
                "{href:?}"
            );
        }
    }

    #[test]
    fn blocks_relative_and_empty_hrefs() {
        for href in ["", "   ", "#anchor", "./a.md", "/abs/path", "example.com"] {
            assert_eq!(
                resolve_link_target(href, IS_WINDOWS),
                LinkTarget::Blocked,
                "{href:?}"
            );
        }
    }

    #[test]
    fn never_follows_a_blocked_link() {
        assert_eq!(
            link_click_action("javascript:alert(1)", true, false, true),
            LinkAction::Blocked
        );
    }

    #[test]
    fn opens_external_links_on_plain_click_in_read_only_mode() {
        assert_eq!(
            link_click_action("https://example.com", true, false, true),
            LinkAction::Open("https://example.com/".into())
        );
    }

    #[test]
    fn requires_ctrl_click_to_open_external_links_while_editing() {
        assert_eq!(
            link_click_action("https://example.com", false, false, true),
            LinkAction::Ignore
        );
        assert_eq!(
            link_click_action("https://example.com", false, true, true),
            LinkAction::Open("https://example.com/".into())
        );
    }

    #[test]
    fn reveals_file_links_on_plain_click_in_either_mode() {
        assert_eq!(
            link_click_action("file:///home/bob/a.txt", false, false, false),
            LinkAction::Reveal("/home/bob/a.txt".into())
        );
    }
}

//! WebDAV ETag 强弱处理与 PROPFIND multistatus XML 解析（PC 与 Cloud 共用）。

use std::borrow::Cow;
use std::collections::HashSet;

/// 解析自 PROPFIND `<response>` 的单个条目
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DavEntry {
    pub href: String,
    pub is_collection: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_length: Option<u64>,
}

/// 是否为弱 ETag（`W/"..."`，RFC 9110 §8.8.3）
pub fn is_weak_etag(etag: &str) -> bool {
    let s = etag.trim();
    s.starts_with("W/") || s.starts_with("w/")
}

/// 去掉弱 ETag 的 `W/` 前缀，返回可放进 `If-Match` 的 opaque-tag
pub fn opaque_etag(etag: &str) -> &str {
    let s = etag.trim();
    s.strip_prefix("W/")
        .or_else(|| s.strip_prefix("w/"))
        .unwrap_or(s)
        .trim_start()
}

/// 供 `If-Match` 使用的强 ETag（去掉 `W/`）；空串返回 `None`
pub fn etag_for_if_match(raw: &str) -> Option<&str> {
    let stripped = opaque_etag(raw);
    (!stripped.is_empty()).then_some(stripped)
}

/// PROPFIND 返回的 getetag 有的不带引号，统一成 HTTP 头里的带引号形式
pub fn normalize_etag(raw: &str) -> String {
    let raw = raw.trim();
    if raw.starts_with('"') || is_weak_etag(raw) {
        raw.to_string()
    } else {
        format!("\"{}\"", raw)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Href,
    ETag,
    LastModified,
    ContentLength,
}

fn field_of(local_name: &str) -> Option<Field> {
    match local_name {
        "href" => Some(Field::Href),
        "getetag" => Some(Field::ETag),
        "getlastmodified" => Some(Field::LastModified),
        "getcontentlength" => Some(Field::ContentLength),
        _ => None,
    }
}

/// 解析 WebDAV multistatus 响应，返回每个 `<response>` 的 href / 是否集合 /
/// getetag / getlastmodified / getcontentlength。
///
/// 只认元素的本地名（去掉任意命名空间前缀、大小写不敏感），支持 XML 注释与 CDATA，
/// 解码预定义实体与数字字符引用（如 `&#34;`）。
pub fn parse_multistatus(xml: &str) -> Vec<DavEntry> {
    let mut entries = Vec::new();
    let mut current: Option<DavEntry> = None;
    let mut capture: Option<(Field, String)> = None;
    let mut rest = xml;

    while let Some(lt) = rest.find('<') {
        if let Some((_, buf)) = capture.as_mut() {
            buf.push_str(&decode_entities(&rest[..lt]));
        }
        let tail = &rest[lt..];
        if let Some(body) = tail.strip_prefix("<!--") {
            match body.find("-->") {
                Some(end) => {
                    rest = &body[end + 3..];
                    continue;
                }
                None => break,
            }
        }
        if let Some(body) = tail.strip_prefix("<![CDATA[") {
            match body.find("]]>") {
                Some(end) => {
                    if let Some((_, buf)) = capture.as_mut() {
                        buf.push_str(&body[..end]);
                    }
                    rest = &body[end + 3..];
                    continue;
                }
                None => break,
            }
        }
        let Some(end) = tag_end(tail) else { break };
        let inner = &tail[1..end];
        rest = &tail[end + 1..];
        if inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        let (closing, inner) = match inner.strip_prefix('/') {
            Some(i) => (true, i),
            None => (false, inner),
        };
        let self_closing = !closing && inner.trim_end().ends_with('/');
        let name = inner
            .trim_end_matches('/')
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name).to_ascii_lowercase();

        if closing {
            if local == "response" {
                if let Some(entry) = current.take() {
                    entries.push(entry);
                }
                capture = None;
            } else if let Some(field) = field_of(&local) {
                let matches = capture.as_ref().is_some_and(|(f, _)| *f == field);
                if let (true, Some((_, buf)), Some(entry)) =
                    (matches, capture.take(), current.as_mut())
                {
                    assign_field(entry, field, buf.trim());
                }
            }
            continue;
        }

        match local.as_str() {
            "response" if !self_closing => {
                current = Some(DavEntry::default());
                capture = None;
            }
            "collection" => {
                if let Some(entry) = current.as_mut() {
                    entry.is_collection = true;
                }
            }
            other => {
                if let (Some(field), false, true) =
                    (field_of(other), self_closing, current.is_some())
                {
                    capture = Some((field, String::new()));
                }
            }
        }
    }
    entries
}

fn assign_field(entry: &mut DavEntry, field: Field, value: &str) {
    if value.is_empty() {
        return;
    }
    match field {
        Field::Href if entry.href.is_empty() => entry.href = value.to_string(),
        Field::ETag if entry.etag.is_none() => entry.etag = Some(value.to_string()),
        Field::LastModified if entry.last_modified.is_none() => {
            entry.last_modified = Some(value.to_string())
        }
        Field::ContentLength if entry.content_length.is_none() => {
            entry.content_length = value.parse().ok()
        }
        _ => {}
    }
}

/// 找到结束当前标签的 `>`（跳过属性值里的引号内容）
fn tag_end(s: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, b) in s.bytes().enumerate().skip(1) {
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => {}
            None => match b {
                b'"' | b'\'' => quote = Some(b),
                b'>' => return Some(i),
                _ => {}
            },
        }
    }
    None
}

/// 解码 XML 预定义实体与数字字符引用；不认识的实体原样保留
pub fn decode_entities(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let decoded = after.find(';').filter(|&semi| semi <= 10).and_then(|semi| {
            let ent = &after[..semi];
            let ch = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    if let Some(hex) = ent.strip_prefix("#x").or_else(|| ent.strip_prefix("#X")) {
                        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
                    } else if let Some(dec) = ent.strip_prefix('#') {
                        dec.parse::<u32>().ok().and_then(char::from_u32)
                    } else {
                        None
                    }
                }
            };
            ch.map(|c| (c, semi))
        });
        match decoded {
            Some((c, semi)) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

fn href_path(href: &str) -> &str {
    let path = match href.find("://") {
        Some(i) => {
            let after = &href[i + 3..];
            match after.find('/') {
                Some(j) => &after[j..],
                None => "/",
            }
        }
        None => href,
    };
    path.split(['?', '#']).next().unwrap_or(path)
}

/// 从 `PROPFIND Depth: 1` 的响应里取出 `dir` 下的文件名（URL 解码后的最后一段），
/// 跳过目录本身与子目录，去重并保持出现顺序。
pub fn file_names_in_listing(xml: &str, dir: &str) -> Vec<String> {
    let dir_tail = dir.trim_matches('/');
    let dir_suffix = format!("/{}", dir_tail);
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for entry in parse_multistatus(xml) {
        if entry.is_collection {
            continue;
        }
        let Ok(decoded) = urlencoding::decode(href_path(&entry.href)) else {
            continue;
        };
        if decoded.ends_with('/') {
            continue;
        }
        let rel = decoded.trim_start_matches('/');
        if rel.is_empty() || rel == dir_tail || decoded.ends_with(&dir_suffix) {
            continue;
        }
        if let Some(name) = decoded.rsplit('/').next().filter(|n| !n.is_empty()) {
            if seen.insert(name.to_string()) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// 解析 PROPFIND Depth 0 元信息；兼容没有 `<response>` 包裹的精简片段（例如 `<d:prop>`）。
pub fn parse_prop_meta(xml: &str) -> DavEntry {
    if let Some(entry) = parse_multistatus(xml).into_iter().next() {
        return entry;
    }
    let wrapped = format!("<response>{}</response>", xml);
    parse_multistatus(&wrapped)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// 同 [`file_names_in_listing`]，但在缺少 `<response>` 包裹的非常规响应中回退解析裸 `<href>`。
pub fn file_names_in_listing_lenient(xml: &str, dir: &str) -> Vec<String> {
    if !parse_multistatus(xml).is_empty() {
        return file_names_in_listing(xml, dir);
    }
    // 非常规服务端：没有 <response> 包裹，把每个 <href>...</href> 视为一条非集合条目
    let mut wrapped = String::with_capacity(xml.len() + 64);
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        let tail = &rest[lt..];
        let Some(end) = tag_end(tail) else { break };
        let inner = &tail[1..end];
        let after_open = &tail[end + 1..];
        let name = inner
            .trim_end_matches('/')
            .split(|c: char| c.is_whitespace())
            .next()
            .unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name);
        if !inner.starts_with('/')
            && !inner.trim_end().ends_with('/')
            && local.eq_ignore_ascii_case("href")
        {
            if let Some(close_lt) = after_open.find("</") {
                let close_tail = &after_open[close_lt..];
                if let Some(close_gt) = close_tail.find('>') {
                    wrapped.push_str("<response><href>");
                    wrapped.push_str(&after_open[..close_lt]);
                    wrapped.push_str("</href></response>");
                    rest = &close_tail[close_gt + 1..];
                    continue;
                }
            }
        }
        rest = after_open;
    }
    if wrapped.is_empty() {
        Vec::new()
    } else {
        file_names_in_listing(&wrapped, dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_helpers_strip_weak_prefix_and_normalize() {
        assert!(is_weak_etag("W/\"abc\""));
        assert!(is_weak_etag("w/\"abc\""));
        assert!(!is_weak_etag("\"abc\""));
        assert_eq!(opaque_etag("W/\"abc\""), "\"abc\"");
        assert_eq!(etag_for_if_match("W/\"1-2\""), Some("\"1-2\""));
        assert_eq!(etag_for_if_match("  "), None);
        assert_eq!(normalize_etag("abc"), "\"abc\"");
        assert_eq!(normalize_etag("\"abc\""), "\"abc\"");
        assert_eq!(normalize_etag("W/\"abc\""), "W/\"abc\"");
    }

    #[test]
    fn parses_multistatus_with_entities_and_collections() {
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
  <D:response>
    <D:href>/dav/mini-todo/images/</D:href>
    <D:propstat><D:prop><D:resourcetype><D:collection/></D:resourcetype></D:prop></D:propstat>
  </D:response>
  <D:response>
    <D:href>/dav/mini-todo/images/a%20b.png</D:href>
    <D:propstat><D:prop>
      <D:getetag>&#34;etag-1&#34;</D:getetag>
      <D:getcontentlength>123</D:getcontentlength>
    </D:prop></D:propstat>
  </D:response>
</D:multistatus>"#;
        let entries = parse_multistatus(xml);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_collection);
        assert_eq!(entries[1].etag.as_deref(), Some("\"etag-1\""));
        assert_eq!(entries[1].content_length, Some(123));
        assert_eq!(
            file_names_in_listing(xml, "/mini-todo/images"),
            vec!["a b.png".to_string()]
        );
    }
}

//! 云端用的 WebDAV 客户端。
//!
//! 与 `pc/src-tauri/src/services/webdav.rs` 共享 API 思路，但额外支持
//! 条件请求需要的 header：
//!
//! * GET 时附 `If-None-Match`（带 ETag），304 → 不变；同时回读 `ETag` 与
//!   `Last-Modified`，作为后续条件 PUT 的依据
//! * PUT 时附 `If-Unmodified-Since`，远端被别人改过会回 412
//!
//! PR1 只用到 GET / PROPFIND / GET file；PUT 的 `if_unmodified_since` 参数留给
//! PR2 push worker。

use std::path::Path;
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, ETAG, IF_NONE_MATCH, IF_UNMODIFIED_SINCE, LAST_MODIFIED};
use reqwest::Method;

/// GET 调用的返回值。404 → `status_code = 404` & `bytes/etag/last_modified` 都为 None。
#[derive(Debug, Clone)]
pub struct WebDavGetResult {
    pub status_code: u16,
    pub bytes: Option<Vec<u8>>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// PUT 调用的返回值。412 (Precondition Failed) 不算 error，调用方根据
/// `status_code` 决定是否触发冲突恢复。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct WebDavPutResult {
    pub status_code: u16,
}

pub struct WebDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

impl WebDavClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow::anyhow!("初始化 reqwest 客户端失败: {}", e))?;
        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            username: username.to_string(),
            password: password.to_string(),
        })
    }

    fn full_url(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        format!("{}/{}", self.base_url, path)
    }

    /// 创建路径上每一级 collection（远端可能已存在，忽略错误）。
    pub fn ensure_dir(&self, path: &str) -> anyhow::Result<()> {
        let parts: Vec<&str> = path
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        let mut current = String::new();
        for part in parts {
            current = format!("{}/{}", current, part);
            let url = self.full_url(&current);
            let _ = self
                .client
                .request(Method::from_bytes(b"MKCOL").unwrap(), &url)
                .basic_auth(&self.username, Some(&self.password))
                .send();
        }
        Ok(())
    }

    /// 条件 GET。`if_none_match` 不为空时附 `If-None-Match` header，远端文件
    /// 未变会返回 304；调用方应据此跳过解码。
    pub fn get(
        &self,
        remote_path: &str,
        if_none_match: Option<&str>,
    ) -> anyhow::Result<WebDavGetResult> {
        let url = self.full_url(remote_path);
        let mut req = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password));
        if let Some(etag) = if_none_match {
            if !etag.is_empty() {
                req = req.header(IF_NONE_MATCH, etag);
            }
        }
        let resp = req
            .send()
            .map_err(|e| anyhow::anyhow!("WebDAV GET {} 失败: {}", remote_path, e))?;
        let status = resp.status().as_u16();

        if status == 304 {
            return Ok(WebDavGetResult {
                status_code: 304,
                bytes: None,
                etag: None,
                last_modified: None,
            });
        }
        if status == 404 {
            return Ok(WebDavGetResult {
                status_code: 404,
                bytes: None,
                etag: None,
                last_modified: None,
            });
        }
        if status != 200 {
            anyhow::bail!("WebDAV GET {} 返回状态 {}", remote_path, status);
        }

        let etag = resp
            .headers()
            .get(ETAG)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let last_modified = resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let bytes = resp
            .bytes()
            .map_err(|e| anyhow::anyhow!("读取 WebDAV 响应体失败: {}", e))?
            .to_vec();

        Ok(WebDavGetResult {
            status_code: 200,
            bytes: Some(bytes),
            etag,
            last_modified,
        })
    }

    /// 条件 PUT。`if_unmodified_since` 非空 → 远端被改过会返回 412。PR2 才会
    /// 真正用到此函数，PR1 保留接口（保持 module surface 稳定）。
    #[allow(dead_code)]
    pub fn put(
        &self,
        remote_path: &str,
        data: &[u8],
        content_type: &str,
        if_unmodified_since: Option<&str>,
    ) -> anyhow::Result<WebDavPutResult> {
        let url = self.full_url(remote_path);
        let mut req = self
            .client
            .put(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header(CONTENT_TYPE, content_type);
        if let Some(lm) = if_unmodified_since {
            if !lm.is_empty() {
                req = req.header(IF_UNMODIFIED_SINCE, lm);
            }
        }
        let resp = req
            .body(data.to_vec())
            .send()
            .map_err(|e| anyhow::anyhow!("WebDAV PUT {} 失败: {}", remote_path, e))?;
        Ok(WebDavPutResult {
            status_code: resp.status().as_u16(),
        })
    }

    /// PROPFIND Depth=1，返回 `remote_path` 下所有"文件名"（不含子目录）。
    pub fn list_files(&self, remote_path: &str) -> anyhow::Result<Vec<String>> {
        let url = self.full_url(remote_path);
        let resp = self
            .client
            .request(Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "1")
            .send()
            .map_err(|e| anyhow::anyhow!("PROPFIND {} 失败: {}", remote_path, e))?;
        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(Vec::new());
        }
        if status != 207 && status != 200 {
            anyhow::bail!("PROPFIND {} 返回状态 {}", remote_path, status);
        }
        let body = resp
            .text()
            .map_err(|e| anyhow::anyhow!("读取 PROPFIND 响应失败: {}", e))?;
        Ok(parse_href_filenames(&body))
    }

    /// 下载单个文件并写入本地路径，返回写入字节数。404 → 写 0 字节并返回 Err。
    pub fn download_to(&self, remote_path: &str, local_path: &Path) -> anyhow::Result<usize> {
        let res = self.get(remote_path, None)?;
        let bytes = res.bytes.ok_or_else(|| {
            anyhow::anyhow!("远端文件 {} 不存在（{}）", remote_path, res.status_code)
        })?;
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("创建目录 {} 失败: {}", parent.display(), e))?;
        }
        std::fs::write(local_path, &bytes)
            .map_err(|e| anyhow::anyhow!("写入 {} 失败: {}", local_path.display(), e))?;
        Ok(bytes.len())
    }
}

/// 解析 PROPFIND XML，抽出每个 `<D:href>` 末尾的文件名；过滤掉目录本身的 href。
fn parse_href_filenames(body: &str) -> Vec<String> {
    let mut names = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        let (start, prefix) = if let Some(pos) = line.find("<D:href>") {
            (pos, "<D:href>")
        } else if let Some(pos) = line.find("<d:href>") {
            (pos, "<d:href>")
        } else {
            continue;
        };
        let rest = &line[start + prefix.len()..];
        let end = match rest.find("</D:href>").or_else(|| rest.find("</d:href>")) {
            Some(e) => e,
            None => continue,
        };
        let href = &rest[..end];
        let decoded = urlencoding::decode(href).unwrap_or_else(|_| href.into());
        let trimmed = decoded.trim_end_matches('/');
        if let Some(name) = trimmed.rsplit('/').next() {
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    // 首个 href 通常是目录自身，移除它
    if !names.is_empty() {
        names.remove(0);
    }
    names
}

#[cfg(test)]
mod tests {
    use super::parse_href_filenames;

    #[test]
    fn parse_nginx_dav_propfind() {
        let xml = r#"<?xml version="1.0"?>
<D:multistatus xmlns:D="DAV:">
<D:response>
  <D:href>/mini-todo/images/</D:href>
</D:response>
<D:response>
  <D:href>/mini-todo/images/foo.png</D:href>
</D:response>
<D:response>
  <D:href>/mini-todo/images/bar.jpg</D:href>
</D:response>
</D:multistatus>"#;
        let files = parse_href_filenames(xml);
        assert_eq!(files, vec!["foo.png", "bar.jpg"]);
    }
}

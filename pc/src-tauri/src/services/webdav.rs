use reqwest::blocking::Client;
use reqwest::header::{CONTENT_TYPE, LAST_MODIFIED};
use std::path::Path;
use std::time::Duration;

pub struct WebDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

/// 上传响应：用于区分成功 vs 条件 PUT 失败（HTTP 412），方便调用方对 412 做重试。
///
/// 成功时附带 server 返回的 `Last-Modified` header（如果存在），便于下一次条件 PUT
/// 使用最新值，避免再额外 GET 一次。
#[derive(Debug, Clone)]
pub enum UploadOutcome {
    Ok(Option<String>),
    PreconditionFailed,
}

/// `download_bytes` 的返回类型：`Some((bytes, last_modified))` 或 `None`（404）。
pub type DownloadOutcome = Option<(Vec<u8>, Option<String>)>;

impl WebDavClient {
    pub fn new(base_url: &str, username: &str, password: &str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        let base_url = base_url.trim_end_matches('/').to_string();

        Self {
            client,
            base_url,
            username: username.to_string(),
            password: password.to_string(),
        }
    }

    fn full_url(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        format!("{}/{}", self.base_url, path)
    }

    pub fn test_connection(&self) -> Result<bool, String> {
        let url = self.full_url("/");
        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "0")
            .send()
            .map_err(|e| format!("连接失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 207 || status == 200 || status == 301 || status == 302 {
            Ok(true)
        } else if status == 401 {
            Err("认证失败，请检查用户名和密码".to_string())
        } else {
            Err(format!("服务器返回状态码: {}", status))
        }
    }

    pub fn ensure_dir(&self, path: &str) -> Result<(), String> {
        let parts: Vec<&str> = path
            .trim_matches('/')
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();

        let mut current = String::new();
        for part in parts {
            current = format!("{}/{}", current, part);
            let url = self.full_url(&current);

            let resp = self
                .client
                .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &url)
                .basic_auth(&self.username, Some(&self.password))
                .send()
                .map_err(|e| format!("创建目录失败: {}", e))?;

            let _status = resp.status().as_u16();
        }
        Ok(())
    }

    pub fn exists(&self, remote_path: &str) -> Result<bool, String> {
        let url = self.full_url(remote_path);
        let resp = self
            .client
            .head(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .map_err(|e| format!("检查文件失败: {}", e))?;
        Ok(resp.status().is_success())
    }

    /// 上传 bytes 到远端。
    ///
    /// 若 `if_unmodified_since` 是 `Some`，附 `If-Unmodified-Since` HTTP header；
    /// server 返回 412 时返回 `UploadOutcome::PreconditionFailed`，**不**作为 Err，
    /// 由调用方决定是否重试（典型场景：拉远端 → per-record merge → 重新 PUT）。
    /// 其它非 2xx 状态仍返回 Err。
    pub fn upload_bytes(
        &self,
        remote_path: &str,
        data: &[u8],
        content_type: &str,
        if_unmodified_since: Option<&str>,
    ) -> Result<UploadOutcome, String> {
        let url = self.full_url(remote_path);

        let mut req = self
            .client
            .put(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header(CONTENT_TYPE, content_type);

        if let Some(value) = if_unmodified_since {
            req = req.header("If-Unmodified-Since", value);
        }

        let resp = req
            .body(data.to_vec())
            .send()
            .map_err(|e| format!("上传失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 200 || status == 201 || status == 204 {
            let last_modified = resp
                .headers()
                .get(LAST_MODIFIED)
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            Ok(UploadOutcome::Ok(last_modified))
        } else if status == 412 {
            Ok(UploadOutcome::PreconditionFailed)
        } else {
            Err(format!("上传失败，状态码: {}", status))
        }
    }

    #[allow(dead_code)]
    pub fn upload_text(&self, remote_path: &str, text: &str) -> Result<(), String> {
        self.upload_bytes(
            remote_path,
            text.as_bytes(),
            "application/json; charset=utf-8",
            None,
        )
        .map(|_| ())
    }

    pub fn upload_file(&self, remote_path: &str, local_path: &Path) -> Result<(), String> {
        let data = std::fs::read(local_path).map_err(|e| format!("读取文件失败: {}", e))?;

        let ext = local_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("bin");

        let content_type = match ext {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            _ => "application/octet-stream",
        };

        self.upload_bytes(remote_path, &data, content_type, None)
            .map(|_| ())
    }

    #[allow(dead_code)]
    pub fn download_text(&self, remote_path: &str) -> Result<Option<String>, String> {
        let url = self.full_url(remote_path);

        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .map_err(|e| format!("下载失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(None);
        }
        if status != 200 {
            return Err(format!("下载失败，状态码: {}", status));
        }

        let text = resp.text().map_err(|e| format!("读取响应失败: {}", e))?;
        Ok(Some(text))
    }

    /// 下载远端 bytes 并附带 server 的 `Last-Modified` header（若返回）。
    ///
    /// 返回值：`Option<(bytes, last_modified_string)>`。404 → `None`；其它失败 → Err。
    /// `Last-Modified` 用于后续 PUT 时附 `If-Unmodified-Since`，避免覆盖并发写入。
    pub fn download_bytes(&self, remote_path: &str) -> Result<DownloadOutcome, String> {
        let url = self.full_url(remote_path);

        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .map_err(|e| format!("下载失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(None);
        }
        if status != 200 {
            return Err(format!("下载失败，状态码: {}", status));
        }

        let last_modified = resp
            .headers()
            .get(LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let bytes = resp.bytes().map_err(|e| format!("读取响应失败: {}", e))?;
        Ok(Some((bytes.to_vec(), last_modified)))
    }

    pub fn download_file(&self, remote_path: &str, local_path: &Path) -> Result<bool, String> {
        let url = self.full_url(remote_path);

        let resp = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .map_err(|e| format!("下载失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(false);
        }
        if status != 200 {
            return Err(format!("下载失败，状态码: {}", status));
        }

        let bytes = resp.bytes().map_err(|e| format!("读取响应失败: {}", e))?;

        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        std::fs::write(local_path, &bytes).map_err(|e| format!("写入文件失败: {}", e))?;

        Ok(true)
    }

    #[allow(dead_code)]
    pub fn list_files(&self, remote_path: &str) -> Result<Vec<String>, String> {
        let url = self.full_url(remote_path);

        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "1")
            .send()
            .map_err(|e| format!("列表失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(Vec::new());
        }
        if status != 207 && status != 200 {
            return Err(format!("列表失败，状态码: {}", status));
        }

        let body = resp.text().map_err(|e| format!("读取响应失败: {}", e))?;

        // Simple XML parsing to extract href values
        let mut files = Vec::new();
        for line in body.lines() {
            let line: &str = line;
            if let Some(start) = line.find("<D:href>").or_else(|| line.find("<d:href>")) {
                let prefix_len: usize = 8;
                let rest = &line[start + prefix_len..];
                if let Some(end) = rest.find("</D:href>").or_else(|| rest.find("</d:href>")) {
                    let href = &rest[..end];
                    let decoded = urlencoding::decode(href).unwrap_or_else(|_| href.into());
                    let name = decoded.trim_end_matches('/');
                    if let Some(file_name) = name.rsplit('/').next() {
                        if !file_name.is_empty() {
                            files.push(file_name.to_string());
                        }
                    }
                }
            }
        }

        // Remove the directory itself (first entry)
        if !files.is_empty() {
            files.remove(0);
        }

        Ok(files)
    }

    #[allow(dead_code)]
    pub fn get_last_modified(&self, remote_path: &str) -> Result<Option<String>, String> {
        let url = self.full_url(remote_path);

        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "0")
            .send()
            .map_err(|e| format!("查询失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 404 {
            return Ok(None);
        }

        let body = resp.text().map_err(|e| format!("读取响应失败: {}", e))?;

        // Extract getlastmodified from XML
        for line in body.lines() {
            let line: &str = line;
            let line_lower = line.to_lowercase();
            if line_lower.contains("getlastmodified") {
                if let Some(start) = line_lower.find('>') {
                    let rest = &line[start + 1..];
                    if let Some(end) = rest.find('<') {
                        return Ok(Some(rest[..end].to_string()));
                    }
                }
            }
        }

        Ok(None)
    }
}

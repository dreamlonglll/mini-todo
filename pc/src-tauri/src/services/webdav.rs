use reqwest::blocking::Client;
use reqwest::header::CONTENT_TYPE;
use std::path::Path;
use std::time::Duration;

pub struct WebDavClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

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

    pub fn upload_bytes(&self, remote_path: &str, data: &[u8], content_type: &str) -> Result<(), String> {
        let url = self.full_url(remote_path);

        let resp = self
            .client
            .put(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header(CONTENT_TYPE, content_type)
            .body(data.to_vec())
            .send()
            .map_err(|e| format!("上传失败: {}", e))?;

        let status = resp.status().as_u16();
        if status == 200 || status == 201 || status == 204 {
            Ok(())
        } else {
            Err(format!("上传失败，状态码: {}", status))
        }
    }

    #[allow(dead_code)]
    pub fn upload_text(&self, remote_path: &str, text: &str) -> Result<(), String> {
        self.upload_bytes(remote_path, text.as_bytes(), "application/json; charset=utf-8")
    }

    pub fn upload_file(&self, remote_path: &str, local_path: &Path) -> Result<(), String> {
        let data = std::fs::read(local_path)
            .map_err(|e| format!("读取文件失败: {}", e))?;

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

        self.upload_bytes(remote_path, &data, content_type)
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

    pub fn download_bytes(&self, remote_path: &str) -> Result<Option<Vec<u8>>, String> {
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

        let bytes = resp.bytes().map_err(|e| format!("读取响应失败: {}", e))?;
        Ok(Some(bytes.to_vec()))
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

        std::fs::write(local_path, &bytes)
            .map_err(|e| format!("写入文件失败: {}", e))?;

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

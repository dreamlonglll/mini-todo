//! `/images/:name` GET + `/images` POST（multipart）。

use std::path::{Component, PathBuf};

use axum::body::Body;
use axum::extract::{Multipart, Path, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use chrono::Utc;
use serde::Serialize;

use super::error::ApiError;
use super::AppState;
use crate::db::repo;

#[derive(Debug, Serialize)]
pub struct UploadResp {
    pub name: String,
}

// =============================================================================
// GET /images/:name
// =============================================================================

pub async fn get_image(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Response, ApiError> {
    let safe = sanitize_filename(&name)
        .ok_or_else(|| ApiError::bad_request(format!("invalid image name: {}", name)))?;
    let full: PathBuf = state.config.images_dir.join(&safe);

    let bytes = match std::fs::read(&full) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::not_found(format!("image {} not found", safe)));
        }
        Err(e) => {
            return Err(ApiError::internal(format!(
                "read {} failed: {}",
                full.display(),
                e
            )));
        }
    };

    let ct = content_type_for(&safe);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, ct)
        .body(Body::from(bytes))
        .map_err(|e| ApiError::internal(format!("build response: {}", e)))
}

// =============================================================================
// POST /images (multipart)
// =============================================================================

pub async fn upload_image(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<UploadResp>, ApiError> {
    // 接受第一个 file 字段（兼容 name="file" / name="image"）
    let mut payload: Option<(String, Vec<u8>)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("multipart: {}", e)))?
    {
        // 仅看 file 字段
        let field_name = field.name().unwrap_or("").to_string();
        if !matches!(field_name.as_str(), "file" | "image" | "") {
            continue;
        }
        let file_name = field.file_name().map(|s| s.to_string());
        let content_type = field.content_type().map(|s| s.to_string());
        let bytes = field
            .bytes()
            .await
            .map_err(|e| ApiError::bad_request(format!("read part: {}", e)))?
            .to_vec();
        if bytes.is_empty() {
            continue;
        }
        // 安全 ext：先从 client filename 尝试推断，再从 Content-Type 推断；
        // 最终通过 sanitize_ext 白名单（仅图片扩展），其它一律 "bin"。
        // 不直接信任 client 提供的 ext 字符串，避免奇怪字符进入服务端文件名。
        let ext = sanitize_ext(
            file_name
                .as_deref()
                .and_then(extension_of)
                .or_else(|| content_type.as_deref().and_then(ext_for_content_type)),
        );
        let name = format!(
            "img_{}_{}.{}",
            Utc::now().timestamp_millis(),
            crate::api::ids::new_id(),
            ext
        );
        payload = Some((name, bytes));
        break;
    }

    let (name, bytes) =
        payload.ok_or_else(|| ApiError::bad_request("missing file part in multipart"))?;

    std::fs::create_dir_all(&state.config.images_dir).map_err(|e| {
        ApiError::internal(format!(
            "create {} failed: {}",
            state.config.images_dir.display(),
            e
        ))
    })?;
    let full = state.config.images_dir.join(&name);
    std::fs::write(&full, &bytes)
        .map_err(|e| ApiError::internal(format!("write {} failed: {}", full.display(), e)))?;

    // 写 dirty_images：JSON 数组形式存进 meta
    state.db.with_conn(|conn| -> rusqlite::Result<()> {
        let raw = repo::get_meta(conn, "dirty_images").unwrap_or_else(|| "[]".to_string());
        let mut arr: Vec<String> = serde_json::from_str(&raw).unwrap_or_default();
        if !arr.iter().any(|n| n == &name) {
            arr.push(name.clone());
        }
        let new_raw = serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string());
        repo::set_meta(conn, "dirty_images", &new_raw)?;
        repo::set_meta(conn, "dirty", "true")?;
        Ok(())
    })?;

    Ok(Json(UploadResp { name }))
}

// =============================================================================
// 工具
// =============================================================================

/// 防 path traversal：拒绝 `..` / 绝对路径 / 含 `/` 或 `\`。
fn sanitize_filename(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.len() > 255 {
        return None;
    }
    let path = std::path::Path::new(raw);
    for c in path.components() {
        match c {
            Component::Normal(_) => {}
            _ => return None,
        }
    }
    let s = path.to_string_lossy().to_string();
    if s.contains('/') || s.contains('\\') {
        return None;
    }
    Some(s)
}

fn extension_of(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
}

/// 把可能不可信的 ext 收紧成图片白名单；非白名单一律 fallback "bin"。
/// 这是 upload 路径的最后一道防线：保证最终文件名不会含非 ASCII 字符 / 路径分隔符
/// / 通配符等。
fn sanitize_ext(ext: Option<String>) -> String {
    match ext.as_deref() {
        Some("png") | Some("jpg") | Some("jpeg") | Some("webp") | Some("gif") | Some("bmp")
        | Some("svg") => ext.unwrap_or_else(|| "bin".to_string()),
        _ => "bin".to_string(),
    }
}

fn ext_for_content_type(ct: &str) -> Option<String> {
    match ct.to_ascii_lowercase().as_str() {
        "image/png" => Some("png".to_string()),
        "image/jpeg" | "image/jpg" => Some("jpg".to_string()),
        "image/webp" => Some("webp".to_string()),
        "image/gif" => Some("gif".to_string()),
        "image/bmp" => Some("bmp".to_string()),
        _ => None,
    }
}

fn content_type_for(name: &str) -> &'static str {
    match extension_of(name).as_deref() {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal() {
        assert_eq!(sanitize_filename(".."), None);
        assert_eq!(sanitize_filename("../foo.png"), None);
        assert_eq!(sanitize_filename("a/b.png"), None);
        assert_eq!(sanitize_filename("a\\b.png"), None);
        assert_eq!(sanitize_filename(""), None);
    }

    #[test]
    fn accepts_simple_names() {
        assert_eq!(sanitize_filename("a.png"), Some("a.png".to_string()));
        assert_eq!(
            sanitize_filename("img_123.jpg"),
            Some("img_123.jpg".to_string())
        );
    }

    #[test]
    fn content_type_lookup() {
        assert_eq!(content_type_for("a.PNG"), "image/png");
        assert_eq!(content_type_for("a.unknown"), "application/octet-stream");
    }

    #[test]
    fn ext_for_ct() {
        assert_eq!(ext_for_content_type("image/png"), Some("png".to_string()));
        assert_eq!(ext_for_content_type("image/JPEG"), Some("jpg".to_string()));
        assert_eq!(ext_for_content_type("text/plain"), None);
    }

    #[test]
    fn sanitize_ext_whitelists_images() {
        assert_eq!(sanitize_ext(Some("png".into())), "png");
        assert_eq!(sanitize_ext(Some("jpeg".into())), "jpeg");
        assert_eq!(sanitize_ext(Some("svg".into())), "svg");
        // 非图片：fallback bin
        assert_eq!(sanitize_ext(Some("exe".into())), "bin");
        assert_eq!(sanitize_ext(Some("中文".into())), "bin");
        assert_eq!(sanitize_ext(Some("..".into())), "bin");
        assert_eq!(sanitize_ext(None), "bin");
    }
}

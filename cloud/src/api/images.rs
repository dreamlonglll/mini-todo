//! `/images/:name` GET + `/images` POST（multipart）。
//!
//! - 文件名一律走 K5 规则（`sync::images::is_safe_image_name`），与同步层同一个判定
//! - 扩展名 ↔ Content-Type 只有一张表（`util`），svg 不在白名单内
//! - 文件读写放进阻塞线程池（单张最大 32 MiB，不能卡住 async worker）；写入先落临时
//!   文件再 rename，半截文件永远不会以正式文件名出现（push / 镜像只认正式文件名）
//! - IO 错误只写服务端日志，不把服务端路径返回给客户端

use std::path::PathBuf;

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
use crate::sync::images::is_safe_image_name;
use crate::util::{
    allowed_image_ext, extension_of, image_content_type, image_ext_for_content_type,
};

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
    if !is_safe_image_name(&name) {
        return Err(ApiError::bad_request(format!(
            "invalid image name: {} (expected a single file name like img_1715000000000_1.png)",
            name
        )));
    }
    let full: PathBuf = state.config.images_dir.join(&name);
    let read = tokio::task::spawn_blocking(move || std::fs::read(&full))
        .await
        .map_err(|e| ApiError::internal("image read task failed", e))?;
    let bytes = match read {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::not_found(format!("image {} not found", name)));
        }
        Err(e) => {
            return Err(ApiError::internal(
                &format!(
                    "read image {}",
                    state.config.images_dir.join(&name).display()
                ),
                e,
            ));
        }
    };

    // nosniff：阻止浏览器把响应嗅探成 HTML/SVG 执行脚本（存储型 XSS 防线）。
    // 存量 .svg（历史上传/PC 同步）额外强制 attachment，只允许下载不允许内联渲染。
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, image_content_type(&name))
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    if extension_of(&name).as_deref() == Some("svg") {
        builder = builder.header(header::CONTENT_DISPOSITION, "attachment");
    }
    builder
        .body(Body::from(bytes))
        .map_err(|e| ApiError::internal("build image response", e))
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
        let name = format!(
            "img_{}_{}.{}",
            Utc::now().timestamp_millis(),
            crate::api::ids::new_id(),
            upload_ext(file_name.as_deref(), content_type.as_deref())
        );
        payload = Some((name, bytes));
        break;
    }

    let (name, bytes) =
        payload.ok_or_else(|| ApiError::bad_request("missing file part in multipart"))?;
    debug_assert!(is_safe_image_name(&name), "{}", name);

    let dir = state.config.images_dir.clone();
    let target = name.clone();
    tokio::task::spawn_blocking(move || write_atomically(&dir, &target, &bytes))
        .await
        .map_err(|e| ApiError::internal("image write task failed", e))?
        .map_err(|e| {
            ApiError::internal(
                &format!("store image in {}", state.config.images_dir.display()),
                e,
            )
        })?;

    // 入 dirty_images 上传队列 + 标脏，同一事务（队列格式与并发约束见 repo::enqueue_dirty_image）
    state.db.with_conn(|conn| -> rusqlite::Result<()> {
        let tx = conn.transaction()?;
        repo::enqueue_dirty_image(&tx, &name)?;
        repo::mark_dirty(&tx)?;
        tx.commit()
    })?;

    Ok(Json(UploadResp { name }))
}

// =============================================================================
// 工具
// =============================================================================

/// 服务端文件名的扩展名：先看客户端文件名的扩展名，再看 Content-Type，都不在图片白名单
/// 里就用 `bin`。不直接信任客户端给的扩展名字符串（白名单保证文件名满足 K5）。
/// svg 故意不在白名单：可内嵌脚本，新上传一律降级 `bin`。
fn upload_ext(file_name: Option<&str>, content_type: Option<&str>) -> &'static str {
    file_name
        .and_then(extension_of)
        .and_then(|e| allowed_image_ext(&e))
        .or_else(|| content_type.and_then(image_ext_for_content_type))
        .unwrap_or("bin")
}

/// 先写同目录临时文件（`.` 开头，K5 规则与 push / 镜像都会跳过）再 rename。
fn write_atomically(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.part", name));
    if let Err(e) = std::fs::write(&tmp, bytes) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, dir.join(name)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_ext_whitelists_images() {
        assert_eq!(upload_ext(Some("a.PNG"), None), "png");
        assert_eq!(upload_ext(Some("a.jpeg"), Some("text/plain")), "jpeg");
        assert_eq!(upload_ext(None, Some("image/jpeg")), "jpg");
        assert_eq!(upload_ext(Some("noext"), Some("image/webp")), "webp");
        // svg 可内嵌脚本，不允许进白名单
        assert_eq!(upload_ext(Some("x.svg"), Some("image/svg+xml")), "bin");
        assert_eq!(upload_ext(Some("danger.exe"), None), "bin");
        assert_eq!(upload_ext(Some("图.中文"), None), "bin");
        assert_eq!(upload_ext(Some(".."), None), "bin");
        assert_eq!(upload_ext(None, None), "bin");
    }

    #[test]
    fn generated_names_satisfy_k5() {
        let name = format!(
            "img_{}_{}.{}",
            Utc::now().timestamp_millis(),
            crate::api::ids::new_id(),
            upload_ext(Some("a.webp"), None)
        );
        assert!(is_safe_image_name(&name), "{}", name);
    }

    #[test]
    fn atomic_write_leaves_no_temp_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("images");
        write_atomically(&dir, "a.png", b"PNG").unwrap();
        assert_eq!(std::fs::read(dir.join("a.png")).unwrap(), b"PNG");
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.png"]);
    }
}

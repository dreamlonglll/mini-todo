//! 图片同步（跨端契约 K5）。
//!
//! - 文件名规则：单个普通路径段，`^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$` 且不含 `..`。
//!   不满足的名字在上传、下载、列举时一律跳过并记日志（远端名字不可信：恶意或被
//!   劫持的 WebDAV 服务端可能给出 `../x` 之类的名字）。
//! - 上传：API 上传的图片进 `meta.dirty_images` 队列，push 时**先**传图片再传
//!   sync-data；上传结束后在同一个连接调用序列里重新读取队列，只移除这次真正处理
//!   掉的名字（修复上传期间新入队图片丢失的竞态，审查 A8）。
//! - 镜像：每次 pull 拿到新文档（200）后由后台 worker 补下缺失图片。远端清单用一次
//!   `PROPFIND Depth: 1` 获取；PROPFIND 失败时退化为按 sync-data 的 `images` 清单逐个下载。

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use tracing::{debug, info, warn};

use crate::db::repo::{self, meta_keys as mk};
use crate::sync::doc::envelope_images;
use crate::sync::webdav::{Precondition, PutOutcome, WebDavClient};
use crate::sync::{SyncCtx, REMOTE_IMAGES_DIR};

/// 图片文件名最大长度（K5）。
pub const MAX_IMAGE_NAME_LEN: usize = 128;

/// K5 安全文件名判定。
pub fn is_safe_image_name(name: &str) -> bool {
    let b = name.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_IMAGE_NAME_LEN
        && b[0].is_ascii_alphanumeric()
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !name.contains("..")
}

/// 本地 images 目录里的合法图片文件名（跳过子目录、下载中的临时文件、非法名）。
pub fn list_local_images(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if is_safe_image_name(name) {
                out.push(name.to_string());
            }
        }
    }
    out.sort();
    out
}

/// 上传用的 Content-Type（WebDAV 存储侧，非对外下发）。
pub fn guess_image_content_type(name: &str) -> &'static str {
    match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("bmp") => "image/bmp",
        Some("svg") => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

/// 一次图片上传的统计。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ImagePushReport {
    pub uploaded: usize,
    pub dropped: usize,
    pub failed: Vec<String>,
}

/// 上传 `meta.dirty_images` 队列里的图片。单张图片上传失败不算 `Err`：记在
/// `report.failed` 里、名字留在队列中等下一轮（由调用方决定是否当作失败退避）。
/// 只有读写本地状态失败 / 客户端初始化失败才返回 `Err`。
pub fn push_dirty_images(ctx: &SyncCtx) -> anyhow::Result<ImagePushReport> {
    let queue = ctx
        .db
        .with_conn(|c| repo::dirty_image_queue(c))
        .map_err(|e| anyhow::anyhow!("读 meta.dirty_images 失败: {}", e))?;
    if queue.is_empty() {
        ctx.db
            .with_conn(|c| repo::delete_meta(c, mk::IMAGE_QUEUE_SINCE))
            .map_err(|e| anyhow::anyhow!("写 meta 失败: {}", e))?;
        return Ok(ImagePushReport::default());
    }
    ctx.db
        .with_conn(|c| {
            repo::set_meta_if_absent(
                c,
                mk::IMAGE_QUEUE_SINCE,
                &chrono::Utc::now().timestamp().to_string(),
            )
        })
        .map_err(|e| anyhow::anyhow!("写 meta 失败: {}", e))?;

    let dav = ctx.dav()?;
    let mut report = ImagePushReport::default();
    let mut done: HashSet<String> = HashSet::new();
    let mut dir_ensured = false;
    for name in &queue {
        if !is_safe_image_name(name) {
            warn!(target: "minitodo_cloud::images", "drop queued image with unsafe name {:?}", name);
            done.insert(name.clone());
            report.dropped += 1;
            continue;
        }
        let local_path = ctx.cfg.images_dir.join(name);
        let bytes = match fs::read(&local_path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                warn!(target: "minitodo_cloud::images", "queued image {} missing locally, drop", name);
                done.insert(name.clone());
                report.dropped += 1;
                continue;
            }
            Err(e) => {
                warn!(target: "minitodo_cloud::images", "read {} failed: {}", local_path.display(), e);
                report.failed.push(name.clone());
                continue;
            }
        };
        let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, name);
        let content_type = guess_image_content_type(name);
        let mut res = dav.put(
            &remote_path,
            &bytes,
            content_type,
            Precondition::Unconditional,
        );
        if matches!(res, Ok(PutOutcome::ParentMissing(_))) && !dir_ensured {
            dir_ensured = true;
            dav.ensure_dir(REMOTE_IMAGES_DIR)?;
            res = dav.put(
                &remote_path,
                &bytes,
                content_type,
                Precondition::Unconditional,
            );
        }
        match res {
            Ok(PutOutcome::Stored(_)) => {
                info!(target: "minitodo_cloud::images", "uploaded image {}", name);
                done.insert(name.clone());
                report.uploaded += 1;
            }
            Ok(other) => {
                warn!(target: "minitodo_cloud::images", "PUT image {} -> {:?}", name, other);
                report.failed.push(name.clone());
            }
            Err(e) => {
                warn!(target: "minitodo_cloud::images", "PUT image {} failed: {:#}", name, e);
                report.failed.push(name.clone());
            }
        }
    }

    // 重新读取当前队列，只移除处理掉的名字：上传期间（不持 DB 锁）新入队的图片保留
    ctx.db
        .with_conn(|c| -> rusqlite::Result<()> {
            let remaining = repo::remove_dirty_images(c, &done)?;
            if remaining.is_empty() {
                repo::delete_meta(c, mk::IMAGE_QUEUE_SINCE)?;
            }
            Ok(())
        })
        .map_err(|e| anyhow::anyhow!("更新 meta.dirty_images 失败: {}", e))?;

    Ok(report)
}

/// 列出远端图片目录里的合法文件名（K5）。
pub fn list_remote_images(dav: &WebDavClient) -> anyhow::Result<Vec<String>> {
    let names = dav.list_names(REMOTE_IMAGES_DIR)?;
    Ok(names
        .into_iter()
        .filter(|name| {
            let ok = is_safe_image_name(name);
            if !ok {
                warn!(target: "minitodo_cloud::images", "skip remote image with unsafe name {:?}", name);
            }
            ok
        })
        .collect())
}

/// 一次镜像的统计。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MirrorReport {
    /// PROPFIND 列举失败的原因；`Some` 表示清单退化为 sync-data 的 images 清单，
    /// worker 会按退避稍后重试（远端可能有清单里没列出的图片）。
    pub listing_error: Option<String>,
    pub remote_count: usize,
    pub downloaded: usize,
    /// 清单里有、但下载时远端已经 404 的数量。
    pub missing: usize,
    pub failed: usize,
}

/// 把远端有、本地没有的图片下载到 `images_dir`。**不持同步锁**：只新建本地缺失的
/// 文件（先写临时文件再 rename），与 push 上传、API 上传互不干扰。
pub fn mirror_once(ctx: &SyncCtx) -> anyhow::Result<MirrorReport> {
    let images_dir = &ctx.cfg.images_dir;
    fs::create_dir_all(images_dir).map_err(|e| {
        anyhow::anyhow!("创建本地 images 目录 {} 失败: {}", images_dir.display(), e)
    })?;
    let dav = ctx.dav()?;

    let mut report = MirrorReport::default();
    let names = match list_remote_images(dav) {
        Ok(names) => names,
        Err(e) => {
            // 日志由 worker 按"状态变化才 warn"统一打
            debug!(
                target: "minitodo_cloud::images",
                "PROPFIND 列举远端图片失败，退化为按 sync-data images 清单下载: {:#}", e
            );
            report.listing_error = Some(format!("{:#}", e));
            let envelope = ctx
                .db
                .with_conn(|c| repo::get_meta(c, mk::REMOTE_ENVELOPE))
                .map_err(|e| anyhow::anyhow!("读 meta.remote_envelope 失败: {}", e))?;
            envelope
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .and_then(|v| v.as_object().map(envelope_images))
                .unwrap_or_default()
        }
    };
    report.remote_count = names.len();

    for name in names {
        let local_path = images_dir.join(&name);
        if local_path.exists() {
            continue;
        }
        let remote_path = format!("{}/{}", REMOTE_IMAGES_DIR, name);
        match dav.download_to(&remote_path, &local_path) {
            Ok(Some(n)) => {
                info!(target: "minitodo_cloud::images", "downloaded {} ({} bytes)", name, n);
                report.downloaded += 1;
            }
            Ok(None) => {
                debug!(target: "minitodo_cloud::images", "remote image {} vanished (404)", name);
                report.missing += 1;
            }
            Err(e) => {
                warn!(target: "minitodo_cloud::images", "download {} failed: {:#}", name, e);
                report.failed += 1;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_image_name_rule() {
        for ok in [
            "a.png",
            "img_1715000000000_1715000000000123.png",
            "1715000000000_abc123.jpg",
            "A-b_c.d.webp",
            &"x".repeat(128),
        ] {
            assert!(is_safe_image_name(ok), "{:?} should be safe", ok);
        }
        for bad in [
            "",
            ".hidden.png",
            "-a.png",
            "_a.png",
            "a..png",
            "../a.png",
            "a/b.png",
            "a\\b.png",
            "a b.png",
            "图.png",
            "a.png\0",
            ".a.png.part",
            &"x".repeat(129),
        ] {
            assert!(!is_safe_image_name(bad), "{:?} should be rejected", bad);
        }
    }

    #[test]
    fn list_local_images_skips_unsafe_dirs_and_temp_files() {
        let tmp = tempfile::TempDir::new().unwrap();
        fs::write(tmp.path().join("a.png"), b"1").unwrap();
        fs::write(tmp.path().join(".b.png.part"), b"1").unwrap();
        fs::write(tmp.path().join("c d.png"), b"1").unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        assert_eq!(list_local_images(tmp.path()), vec!["a.png"]);
        assert!(list_local_images(&tmp.path().join("missing")).is_empty());
    }

    #[test]
    fn guess_content_type_matches_extension() {
        assert_eq!(guess_image_content_type("a.png"), "image/png");
        assert_eq!(guess_image_content_type("b.JPG"), "image/jpeg");
        assert_eq!(
            guess_image_content_type("c.bin"),
            "application/octet-stream"
        );
    }
}

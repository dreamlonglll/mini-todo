//! Push：把云端本地写入推到 WebDAV（跨端契约 K4 / K5）。
//!
//! 流程（整段在同步锁内、阻塞线程里执行）：
//! 1. **先传图片**（`meta.dirty_images` 队列），失败的留在队列里下轮重试
//! 2. sync-data（仅当 `meta.dirty`，或刚传了新图片）：
//!    - 写前**总是**先条件 GET（只用 `If-None-Match`）并按 K3 合并进本地 SQLite
//!      （与 pull 同一实现，合并与记录基准同一事务）。服务端忽略前置条件
//!      （nginx dav / Caddy webdav）时，丢更新窗口也只剩 GET→PUT 的几秒
//!    - 用"合并后的本地状态 + 基准信封"构造文档：未知顶层键、`settings`、
//!      `settingsUpdatedAt` 原样保留
//!    - 条件 PUT：知道基准 ETag 就发 `If-Match: "<opaque>"`（去掉 `W/`），否则不带前置
//!      条件。**从不发 `If-Unmodified-Since`**：Apache 用文件的亚秒级 mtime 和秒级日期
//!      比较，拿文件自己的 Last-Modified 也会 412（e2e 实测）
//!    - 412 → 至少等 1.1 秒（Apache 写入后约 1 秒内 ETag 是弱的，这一秒内 `If-Match`
//!      必然 412）→ 无条件重新 GET → 合并 → 重试（共 ≤3 次）；404/409 → MKCOL 后重试一次
//!    - 成功 → 基准 ETag 依次取自：PUT 响应 → `HEAD`（nginx 只在 HEAD/GET 给 ETag）→
//!      `PROPFIND Depth: 0` 的 getetag；Last-Modified 一并记录但不用于前置条件。
//!      **不整包 GET**；本地缓存此时已经等于刚上传的文档（合并结果先落库再导出），
//!      不会出现"基准指向缓存里没有的内容"（审查 A4）
//!    - dirty 只在 `dirty_generation` 没变时清除（推送窗口期内的新写入留给下一轮）

use std::collections::HashSet;
use std::time::Duration;

use anyhow::Context as _;
use tracing::{debug, info, warn};

use crate::db::repo::{self, meta_keys as mk};
use crate::sync::doc::{self, envelope_images, TOMBSTONE_RETENTION_DAYS};
use crate::sync::images::{self, ImagePushReport};
use crate::sync::pull::{fetch_and_merge, load_base, load_envelope, store_base};
use crate::sync::webdav::{etag_for_if_match, Precondition, PutOutcome, Validators, WebDavClient};
use crate::sync::{gzip, SyncCtx, REMOTE_DIR, SYNC_DATA_FILE};
use crate::time::{days_ago_local_string, now_local_string};
use crate::util::public_error_message;

/// sync-data PUT 最多尝试次数（412 重试上限）。
pub const MAX_PUT_ATTEMPTS: u32 = 3;

/// 412 之后、下一次尝试之前至少等待的时长：Apache 写入后约 1 秒内只给弱 ETag，
/// 这一秒内 `If-Match`（强比较）必然 412；e2e 实测 1.1 秒后同一个值可以匹配。
pub const PRECONDITION_RETRY_DELAY: Duration = Duration::from_millis(1100);

/// 一次 push 的结果。
#[derive(Debug, Default)]
pub struct PushReport {
    pub images: ImagePushReport,
    /// sync-data 是否成功 PUT。
    pub pushed: bool,
    /// sync-data PUT 尝试次数。
    pub attempts: u32,
    /// 是否清除了 dirty（推送窗口期内有新写入时不清）。
    pub dirty_cleared: bool,
}

/// 选择 PUT 的前置条件（K4）。
///
/// - 远端不存在 → 不带条件
/// - 知道基准 ETag（强弱都行）→ `If-Match: "<opaque>"`（去掉 `W/`）
/// - 否则 → 不带条件（**不**退回 `If-Unmodified-Since`，见模块注释）
pub fn select_precondition(remote_exists: bool, base: &Validators) -> Precondition<'_> {
    if !remote_exists {
        return Precondition::Unconditional;
    }
    match base.etag.as_deref().and_then(etag_for_if_match) {
        Some(tag) => Precondition::IfMatch(tag),
        None => Precondition::Unconditional,
    }
}

/// 单次 push。失败时把错误写进 `meta.last_push_error`，成功（含无事可做）时清除它
/// ——`/health` 展示的是"当前还没解决的推送问题"。写进 meta 的是脱敏后的消息
/// （本地存储错误只留上下文，详情只进日志，见 `util::public_error_message`）。
/// 只能在阻塞上下文、同步锁内调用（见 `SyncCtx::run_locked`）。
pub fn push_once(ctx: &SyncCtx) -> anyhow::Result<PushReport> {
    let res = push_inner(ctx);
    let recorded = match &res {
        Ok(_) => ctx
            .db
            .with_conn(|c| repo::delete_meta(c, mk::LAST_PUSH_ERROR)),
        Err(e) => {
            let msg = public_error_message(e);
            ctx.db
                .with_conn(|c| repo::set_meta(c, mk::LAST_PUSH_ERROR, &msg))
        }
    };
    if let Err(db_err) = recorded {
        warn!(target: "minitodo_cloud::push", "记录 last_push_error 失败: {}", db_err);
    }
    res
}

fn push_inner(ctx: &SyncCtx) -> anyhow::Result<PushReport> {
    let mut report = PushReport::default();

    // K5：先传图片，再传 sync-data。图片失败不阻塞记录同步。
    let images_res = images::push_dirty_images(ctx);
    // 这次传上去的图片若在上一版 sync-data 构造时还在队列里，就没进 `images` 清单
    // （旧版 PC 只按清单下载图片），所以哪怕记录没变也要刷新一次 sync-data。
    let uploaded_any = images_res.as_ref().is_ok_and(|r| r.uploaded > 0);

    let (dirty, g0) = ctx
        .db
        .with_conn(|c| -> rusqlite::Result<(bool, i64)> {
            Ok((repo::is_dirty(c)?, repo::get_dirty_generation(c)?))
        })
        .context("读 meta.dirty 失败")?;
    if dirty || uploaded_any {
        push_sync_data(ctx, g0, &mut report)?;
    }

    let images = images_res?;
    if !images.failed.is_empty() {
        // 失败的图片留在队列里；返回错误让 worker 退避重试并记录 last_push_error
        anyhow::bail!(
            "{} 张图片上传失败: {}",
            images.failed.len(),
            images.failed.join(", ")
        );
    }
    report.images = images;
    Ok(report)
}

fn push_sync_data(ctx: &SyncCtx, g0: i64, report: &mut PushReport) -> anyhow::Result<()> {
    let dav = ctx.dav()?;
    let tz = ctx.cfg.timezone;
    let mut force_full = false;

    for attempt in 1..=MAX_PUT_ATTEMPTS {
        report.attempts = attempt;

        // 1) 写前先 GET 并合并（412 之后无条件 GET：保证拿到最新内容与校验器）
        let fetched = fetch_and_merge(ctx, force_full)?;

        // 2) 合并后的本地状态 + 基准信封 → 要上传的文档
        let cutoff = days_ago_local_string(tz, TOMBSTONE_RETENTION_DAYS);
        let local_images = images::list_local_images(&ctx.cfg.images_dir);
        let (base, doc) = ctx
            .db
            .with_conn(|c| -> rusqlite::Result<_> {
                let base = load_base(c)?;
                let envelope = load_envelope(c)?;
                // 还在上传队列里的图片远端可能还没有，不列进清单（远端清单里已有的照列）
                let pending: HashSet<String> = repo::dirty_image_queue(c)?.into_iter().collect();
                let images = doc::normalize_image_list(
                    local_images
                        .into_iter()
                        .filter(|n| !pending.contains(n))
                        .chain(envelope_images(&envelope)),
                );
                let doc = doc::build_outgoing_doc(c, &envelope, images, tz, &cutoff)?;
                Ok((base, doc))
            })
            .context("构造 sync-data 失败")?;
        let payload = gzip(&serde_json::to_vec(&doc)?)?;

        // 3) 条件 PUT
        let pre = select_precondition(fetched.remote_exists, &base);
        let mut outcome = dav.put(SYNC_DATA_FILE, &payload, "application/gzip", pre)?;
        if let PutOutcome::ParentMissing(status) = outcome {
            info!(
                target: "minitodo_cloud::push",
                "PUT sync-data 返回 {}，创建远端目录后重试", status
            );
            dav.ensure_dir(REMOTE_DIR)?;
            outcome = dav.put(SYNC_DATA_FILE, &payload, "application/gzip", pre)?;
        }

        match outcome {
            PutOutcome::Stored(from_put) => {
                let validators = validators_after_put(dav, from_put, payload.len());
                let mut envelope = doc;
                envelope.remove("todos");
                envelope.remove("tombstones");
                let now = now_local_string(tz);
                let cleared = ctx
                    .db
                    .with_conn(|c| -> rusqlite::Result<bool> {
                        let tx = c.transaction()?;
                        store_base(&tx, &validators, &envelope)?;
                        repo::set_meta(&tx, mk::LAST_PUSH_OK_AT, &now)?;
                        repo::delete_meta(&tx, mk::LAST_PUSH_ERROR)?;
                        repo::purge_tombstones_before(&tx, &cutoff)?;
                        let cleared = repo::clear_dirty_if_unchanged(&tx, g0)?;
                        tx.commit()?;
                        Ok(cleared)
                    })
                    .context("PUT 成功后写 meta 失败")?;
                report.pushed = true;
                report.dirty_cleared = cleared;
                info!(
                    target: "minitodo_cloud::push",
                    "push ok ({} bytes, attempt {}, precondition {:?}{})",
                    payload.len(),
                    attempt,
                    pre,
                    if cleared { "" } else { ", new writes pending" }
                );
                return Ok(());
            }
            PutOutcome::PreconditionFailed => {
                info!(
                    target: "minitodo_cloud::push",
                    "PUT sync-data 412（第 {} 次）：远端已被其它写入方修改，或 ETag 仍处于弱 ETag 窗口；{:?} 后重新拉取合并再试",
                    attempt,
                    PRECONDITION_RETRY_DELAY
                );
                force_full = true;
                if attempt < MAX_PUT_ATTEMPTS {
                    std::thread::sleep(PRECONDITION_RETRY_DELAY);
                }
            }
            PutOutcome::ParentMissing(status) => {
                anyhow::bail!(
                    "PUT sync-data 返回 {}：远端目录不存在，MKCOL 后仍失败",
                    status
                )
            }
        }
    }
    anyhow::bail!("多次重试后仍冲突（连续 {} 次 412）", MAX_PUT_ATTEMPTS)
}

/// 长度对不上说明远端在 PUT 之后又被别人覆盖了：不能把那个没合并过的版本记为基准。
fn length_matches(reported: Option<u64>, payload_len: usize) -> bool {
    reported.is_none_or(|n| n == payload_len as u64)
}

/// PUT 成功后确定新的基准校验器（K4）：
/// 1. PUT 响应里有 ETag → 直接用
/// 2. 否则 `HEAD`（nginx：PUT 响应与 PROPFIND 都没有 ETag，只有 HEAD / GET 有）
/// 3. 再不行 `PROPFIND Depth: 0` 的 getetag（Apache 的 PROPFIND 给强 ETag）
///
/// Last-Modified 一并记录（只作信息，不用于前置条件）。**不整包 GET**。HEAD / PROPFIND
/// 报告的长度与刚上传的不一致说明远端已被别人覆盖，此时不记录基准（下次全量拉取合并）
/// ——宁可多一次 GET 也不把没合并过的版本当作基准。
fn validators_after_put(
    dav: &WebDavClient,
    from_put: Validators,
    payload_len: usize,
) -> Validators {
    if from_put.etag.is_some() {
        return from_put;
    }
    let mut last_modified = from_put.last_modified;
    match dav.head(SYNC_DATA_FILE) {
        Ok(Some(info)) => {
            if !length_matches(info.content_length, payload_len) {
                warn!(
                    target: "minitodo_cloud::push",
                    "HEAD 返回的长度 {:?} 与刚上传的 {} 不一致，远端可能已被覆盖；不记录基准",
                    info.content_length,
                    payload_len
                );
                return Validators::default();
            }
            last_modified = info.validators.last_modified.or(last_modified);
            if info.validators.etag.is_some() {
                return Validators {
                    etag: info.validators.etag,
                    last_modified,
                };
            }
            debug!(target: "minitodo_cloud::push", "HEAD 没有 ETag，改用 PROPFIND Depth: 0");
        }
        Ok(None) => {
            warn!(target: "minitodo_cloud::push", "PUT 成功后 HEAD 返回 404；不记录基准");
            return Validators::default();
        }
        Err(e) => {
            debug!(target: "minitodo_cloud::push", "PUT 成功后 HEAD 失败，改用 PROPFIND: {:#}", e);
        }
    }
    match dav.propfind_meta(SYNC_DATA_FILE) {
        Ok(Some(entry)) => {
            if !length_matches(entry.content_length, payload_len) {
                warn!(
                    target: "minitodo_cloud::push",
                    "PROPFIND 返回的长度 {:?} 与刚上传的 {} 不一致，远端可能已被覆盖；不记录基准",
                    entry.content_length,
                    payload_len
                );
                return Validators::default();
            }
            Validators {
                etag: entry.etag,
                last_modified: entry.last_modified.or(last_modified),
            }
        }
        Ok(None) => {
            warn!(target: "minitodo_cloud::push", "PUT 成功后 PROPFIND 返回 404；不记录基准");
            Validators::default()
        }
        Err(e) => {
            warn!(
                target: "minitodo_cloud::push",
                "PUT 成功后 HEAD / PROPFIND 都拿不到 ETag（下次全量拉取）: {:#}", e
            );
            Validators {
                etag: None,
                last_modified,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::Db;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn v(etag: Option<&str>, lm: Option<&str>) -> Validators {
        Validators {
            etag: etag.map(str::to_string),
            last_modified: lm.map(str::to_string),
        }
    }

    const LM: &str = "Fri, 02 Oct 2026 08:00:00 GMT";

    #[test]
    fn precondition_none_when_remote_absent() {
        assert_eq!(
            select_precondition(false, &v(Some("\"a\""), Some(LM))),
            Precondition::Unconditional
        );
    }

    #[test]
    fn precondition_is_if_match_for_any_known_etag() {
        assert_eq!(
            select_precondition(true, &v(Some("\"a\""), Some(LM))),
            Precondition::IfMatch("\"a\"")
        );
        // Apache 一秒内的弱 ETag：去掉 W/ 再发（这一秒内 412，等 1.1 秒后重试即可匹配）
        assert_eq!(
            select_precondition(true, &v(Some("W/\"3-65cdc\""), Some(LM))),
            Precondition::IfMatch("\"3-65cdc\"")
        );
    }

    /// 只有 Last-Modified（或什么都没有）时不带前置条件：绝不发 If-Unmodified-Since。
    #[test]
    fn precondition_never_falls_back_to_last_modified() {
        assert_eq!(
            select_precondition(true, &v(None, Some(LM))),
            Precondition::Unconditional
        );
        assert_eq!(
            select_precondition(true, &v(Some("  "), Some(LM))),
            Precondition::Unconditional
        );
        assert_eq!(
            select_precondition(true, &v(None, None)),
            Precondition::Unconditional
        );
    }

    #[test]
    fn length_check_tolerates_missing_length() {
        assert!(length_matches(None, 10));
        assert!(length_matches(Some(10), 10));
        assert!(!length_matches(Some(11), 10));
    }

    fn fresh_db() -> (Db, TempDir) {
        let tmp = TempDir::new().expect("tempdir");
        let db = Db::open(&tmp.path().join("data.db")).expect("open db");
        (db, tmp)
    }

    fn dirty_flag(db: &Db) -> Option<String> {
        db.with_conn(|conn| repo::get_meta(conn, "dirty"))
            .expect("读 dirty")
    }

    /// 推送窗口期内（g0 已读、dirty 还没清）又有写入 → PUT 成功也不能清 dirty。
    #[test]
    fn clear_dirty_keeps_flag_when_write_lands_during_push() {
        let (db, _tmp) = fresh_db();
        db.with_conn(|conn| repo::mark_dirty(conn)).unwrap();
        let g0 = db
            .with_conn(|conn| repo::get_dirty_generation(conn))
            .unwrap();
        // 慢速 PUT 期间，API 写路径又标了一次脏
        db.with_conn(|conn| repo::mark_dirty(conn)).unwrap();
        assert!(
            !db.with_conn(|c| repo::clear_dirty_if_unchanged(c, g0))
                .unwrap(),
            "generation 变了就不该清 dirty"
        );
        assert_eq!(dirty_flag(&db).as_deref(), Some("true"));
    }

    /// 推送期间无写入 → PUT 成功后正常清 dirty（连同 dirty_since）。
    #[test]
    fn clear_dirty_clears_flag_when_no_write_during_push() {
        let (db, _tmp) = fresh_db();
        db.with_conn(|conn| repo::mark_dirty(conn)).unwrap();
        let g0 = db
            .with_conn(|conn| repo::get_dirty_generation(conn))
            .unwrap();
        assert!(db
            .with_conn(|c| repo::clear_dirty_if_unchanged(c, g0))
            .unwrap());
        assert_eq!(dirty_flag(&db).as_deref(), Some("false"));
        assert_eq!(
            db.with_conn(|c| repo::get_meta(c, mk::DIRTY_SINCE))
                .unwrap(),
            None
        );
    }

    /// WebDAV 不可达 → push 报错、dirty 保持 true、错误写进 last_push_error。
    #[test]
    fn push_once_keeps_dirty_when_webdav_unreachable() {
        let (db, tmp) = fresh_db();
        db.with_conn(|conn| repo::mark_dirty(conn)).unwrap();
        // Config::for_tests 的 webdav_url 指向 127.0.0.1:0，必然连不上
        let cfg = Config::for_tests(
            "test-api-key-1234567890abcdef",
            tmp.path().join("data"),
            tmp.path().join("images"),
        );
        let ctx = SyncCtx::new(Arc::new(cfg), db.clone());
        assert!(push_once(&ctx).is_err(), "WebDAV 不可达时 push 必须报错");
        assert_eq!(dirty_flag(&db).as_deref(), Some("true"));
        let err = db
            .with_conn(|c| repo::get_meta(c, mk::LAST_PUSH_ERROR))
            .unwrap();
        assert!(err.is_some_and(|e| !e.is_empty()));
    }

    /// 不 dirty、图片队列空 → push 是 no-op，不触网。
    #[test]
    fn push_once_is_noop_when_clean() {
        let (db, tmp) = fresh_db();
        let cfg = Config::for_tests(
            "test-api-key-1234567890abcdef",
            tmp.path().join("data"),
            tmp.path().join("images"),
        );
        let ctx = SyncCtx::new(Arc::new(cfg), db);
        let report = push_once(&ctx).expect("clean push must not touch the network");
        assert!(!report.pushed);
        assert_eq!(report.attempts, 0);
    }
}

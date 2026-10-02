//! Push：把云端本地写入推到 WebDAV（跨端契约 K4 / K5）。
//!
//! 流程（整段在同步锁内、阻塞线程里执行）：
//! 1. **先传图片**（`meta.dirty_images` 队列），失败的留在队列里下轮重试
//! 2. sync-data（仅当 `meta.dirty`）：
//!    - 写前**总是**先条件 GET 并按 K3 合并进本地 SQLite（与 pull 同一实现，
//!      合并与记录基准同一事务）。服务端忽略条件头（nginx dav / Caddy webdav）
//!      时，丢更新窗口也只剩 GET→PUT 的几秒
//!    - 用"合并后的本地状态 + 基准信封"构造文档：未知顶层键、`settings`、
//!      `settingsUpdatedAt` 原样保留
//!    - 条件 PUT：基准是强 ETag 才用 `If-Match`（Apache 一秒内的弱 ETag 做
//!      `If-Match` 必然 412）；否则用 `If-Unmodified-Since`；远端不存在不带条件
//!    - 412 → 无条件重新 GET → 合并 → 重试（≤3 次）；404/409 → MKCOL 后重试一次
//!    - 成功 → 基准取 PUT 响应的 ETag/Last-Modified，都没有（Apache）就用一次
//!      `PROPFIND Depth: 0`，**不整包 GET**；本地缓存此时已经等于刚上传的文档
//!      （合并结果先落库再导出），不会出现"基准指向缓存里没有的内容"（审查 A4）
//!    - dirty 只在 `dirty_generation` 没变时清除（推送窗口期内的新写入留给下一轮）

use std::collections::HashSet;

use tracing::{info, warn};

use crate::db::repo::{self, meta_keys as mk};
use crate::sync::doc::{self, envelope_images, TOMBSTONE_RETENTION_DAYS};
use crate::sync::images::{self, ImagePushReport};
use crate::sync::pull::{fetch_and_merge, load_base, load_envelope, store_base};
use crate::sync::webdav::{is_strong_etag, Precondition, PutOutcome, Validators, WebDavClient};
use crate::sync::{gzip, SyncCtx, REMOTE_DIR, SYNC_DATA_FILE};
use crate::time::{days_ago_local_string, now_local_string};

/// sync-data PUT 最多尝试次数（412 重试上限）。
pub const MAX_PUT_ATTEMPTS: u32 = 3;

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
/// - 基准 ETag 是强 ETag（且服务端没被证实错误处理 If-Match）→ `If-Match`
/// - 否则有 Last-Modified → `If-Unmodified-Since`
/// - 什么都没有 → 不带条件
pub fn select_precondition(
    remote_exists: bool,
    base: &Validators,
    if_match_allowed: bool,
) -> Precondition<'_> {
    if !remote_exists {
        return Precondition::Unconditional;
    }
    if if_match_allowed {
        if let Some(etag) = base.etag.as_deref().filter(|e| is_strong_etag(e)) {
            return Precondition::IfMatch(etag.trim());
        }
    }
    match base
        .last_modified
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(lm) => Precondition::IfUnmodifiedSince(lm),
        None => Precondition::Unconditional,
    }
}

/// 单次 push。失败时把错误写进 `meta.last_push_error`，成功（含无事可做）时清除它
/// ——`/health` 展示的是"当前还没解决的推送问题"。
/// 只能在阻塞上下文、同步锁内调用（见 `SyncCtx::run_locked`）。
pub fn push_once(ctx: &SyncCtx) -> anyhow::Result<PushReport> {
    let res = push_inner(ctx);
    let recorded = match &res {
        Ok(_) => ctx
            .db
            .with_conn(|c| repo::delete_meta(c, mk::LAST_PUSH_ERROR)),
        Err(e) => {
            let msg = format!("{:#}", e);
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
        .map_err(|e| anyhow::anyhow!("读 meta.dirty 失败: {}", e))?;
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
    let mut sent_if_match: Option<String> = None;

    for attempt in 1..=MAX_PUT_ATTEMPTS {
        report.attempts = attempt;

        // 1) 写前先（条件）GET 并合并
        let fetched = fetch_and_merge(ctx, force_full)?;
        if let (Some(sent), Some(seen)) =
            (sent_if_match.as_deref(), fetched.fetched_etag.as_deref())
        {
            if sent == seen.trim() {
                // 远端根本没变，服务端却拒绝了 If-Match：它对 If-Match 的处理不可靠
                ctx.mark_if_match_unreliable();
            }
        }

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
            .map_err(|e| anyhow::anyhow!("构造 sync-data 失败: {}", e))?;
        let payload = gzip(&serde_json::to_vec(&doc)?)?;

        // 3) 条件 PUT
        let pre = select_precondition(fetched.remote_exists, &base, !ctx.if_match_unreliable());
        sent_if_match = match pre {
            Precondition::IfMatch(etag) => Some(etag.to_string()),
            _ => None,
        };
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
                    .map_err(|e| anyhow::anyhow!("PUT 成功后写 meta 失败: {}", e))?;
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
                    "PUT sync-data 412（第 {} 次），远端已被其它写入方修改，重新拉取合并后重试",
                    attempt
                );
                force_full = true;
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

/// PUT 响应里的校验器是否足以做下一次条件 PUT：有强 ETag（可用 If-Match）或有
/// Last-Modified（可用 If-Unmodified-Since）。只有弱 ETag 或什么都没有（Apache）时
/// 需要再 PROPFIND 一次。
fn put_validators_sufficient(v: &Validators) -> bool {
    v.etag.as_deref().is_some_and(is_strong_etag) || v.last_modified.is_some()
}

/// PUT 成功后确定新的基准校验器：优先用 PUT 响应头；不够用时（Apache 什么都不给）
/// 用一次 `PROPFIND Depth: 0` 补齐，**不整包 GET**。PROPFIND 报告的长度与刚上传的
/// 不一致说明远端已被别人覆盖，此时不记录基准（下次全量拉取合并）——宁可多一次 GET
/// 也不把没合并过的版本当作基准。
fn validators_after_put(
    dav: &WebDavClient,
    from_put: Validators,
    payload_len: usize,
) -> Validators {
    if put_validators_sufficient(&from_put) {
        return from_put;
    }
    match dav.propfind_meta(SYNC_DATA_FILE) {
        Ok(Some(entry)) => {
            if entry
                .content_length
                .is_some_and(|n| n != payload_len as u64)
            {
                warn!(
                    target: "minitodo_cloud::push",
                    "PROPFIND 返回的长度 {:?} 与刚上传的 {} 不一致，远端可能已被覆盖；不记录基准",
                    entry.content_length,
                    payload_len
                );
                return Validators::default();
            }
            Validators {
                etag: entry.etag.or(from_put.etag),
                last_modified: entry.last_modified.or(from_put.last_modified),
            }
        }
        Ok(None) => {
            warn!(target: "minitodo_cloud::push", "PUT 成功后 PROPFIND 返回 404；不记录基准");
            Validators::default()
        }
        Err(e) => {
            warn!(
                target: "minitodo_cloud::push",
                "PUT 成功后 PROPFIND 失败，不记录基准（下次全量拉取）: {:#}", e
            );
            Validators::default()
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
            select_precondition(false, &v(Some("\"a\""), Some(LM)), true),
            Precondition::Unconditional
        );
    }

    #[test]
    fn precondition_prefers_if_match_for_strong_etag() {
        assert_eq!(
            select_precondition(true, &v(Some("\"a\""), Some(LM)), true),
            Precondition::IfMatch("\"a\"")
        );
        assert_eq!(
            select_precondition(true, &v(Some("\"a\""), None), true),
            Precondition::IfMatch("\"a\"")
        );
    }

    /// Apache 一秒内刚修改的文件给弱 ETag；弱 ETag 做 If-Match 必然 412，必须退回 LM。
    #[test]
    fn precondition_falls_back_to_lm_for_weak_etag() {
        assert_eq!(
            select_precondition(true, &v(Some("W/\"3-65cdc\""), Some(LM)), true),
            Precondition::IfUnmodifiedSince(LM)
        );
        assert_eq!(
            select_precondition(true, &v(Some("W/\"3-65cdc\""), None), true),
            Precondition::Unconditional
        );
    }

    #[test]
    fn put_response_validators_sufficiency() {
        assert!(put_validators_sufficient(&v(Some("\"a\""), None)));
        assert!(put_validators_sufficient(&v(None, Some(LM))));
        assert!(put_validators_sufficient(&v(Some("W/\"a\""), Some(LM))));
        // 只有弱 ETag：下一次 PUT 既不能 If-Match 也没有 LM，必须 PROPFIND 补齐
        assert!(!put_validators_sufficient(&v(Some("W/\"a\""), None)));
        // Apache：PUT 响应什么都没有
        assert!(!put_validators_sufficient(&v(None, None)));
    }

    #[test]
    fn precondition_uses_lm_without_etag_or_when_if_match_unreliable() {
        assert_eq!(
            select_precondition(true, &v(None, Some(LM)), true),
            Precondition::IfUnmodifiedSince(LM)
        );
        assert_eq!(
            select_precondition(true, &v(Some("\"a\""), Some(LM)), false),
            Precondition::IfUnmodifiedSince(LM)
        );
        assert_eq!(
            select_precondition(true, &v(None, Some("  ")), true),
            Precondition::Unconditional
        );
        assert_eq!(
            select_precondition(true, &v(None, None), true),
            Precondition::Unconditional
        );
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

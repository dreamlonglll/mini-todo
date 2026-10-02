//! Pull：条件 GET `/mini-todo/sync-data.json.gz`，按 K3 合并进本地 SQLite。
//!
//! "基准"（跨端契约 K4）= 最近一次**已完整合并或成功写入**的远端版本的
//! `(ETag, Last-Modified)`，连同该版本的"信封"（除记录外的顶层键）一起存在
//! meta 里。只有在合并事务里才会把一次 GET 的校验器记为基准——内容没合并的
//! GET 结果绝不记为基准（旧版 push 把 PUT 后再 GET 的 ETag 记下来、缓存却没
//! 更新，之后 pull 永远 304，即审查 A4）。
//!
//! 条件 GET 只用 `If-None-Match`；**从不用 `If-Modified-Since`**（nginx 按整秒比较，
//! 与基准同一秒内的写入得到 304，e2e 中云端因此漏掉了 PC 的修改）。不知道 ETag 时
//! 无条件 GET。nginx 的 ETag 本身也只有秒级精度（`mtime秒-长度`），同一秒内写入且
//! 长度相同仍可能 304，所以后台 worker 每隔若干轮做一次无条件全量拉取兜底。
//!
//! 合并之后若发现远端**缺少**本地已有的记录 / 墓碑（例如别的写入方在忽略前置条件的
//! 服务端上用旧快照整包覆盖了），pull 会标脏，让 push 把并集重新写回去（与 PC 一致）。
//!
//! - `fetch_and_merge`：pull 与 push 共用的"条件 GET + 合并"一步
//! - `pull_once` / `pull_once_with`：一次 pull（含 seq 回填、远端落后检测、错误记录）
//! - 后台循环见 `sync::worker`

use anyhow::Context as _;
use serde_json::{Map, Value};
use tracing::{debug, info, warn};

use crate::db::repo::{self, meta_keys as mk};
use crate::db::{normalize, Db};
use crate::sync::doc::{RemoteDoc, TOMBSTONE_RETENTION_DAYS};
use crate::sync::merge::{self, MergeStats};
use crate::sync::webdav::{GetOutcome, Validators};
use crate::sync::{gunzip, SyncCtx, SYNC_DATA_FILE};
use crate::time::{days_ago_local_string, now_local_string};
use crate::util::public_error_message;

/// 判定"远端缺少本地墓碑"时忽略离过期不到一天的墓碑：各端按自己的时钟清理过期墓碑，
/// 留一天余量，时钟偏差不会导致每轮都重推。
const TOMBSTONE_RECHECK_MARGIN_DAYS: i64 = 1;

/// 读取基准。没有信封时视为没有基准（304 时无法重建上传文档，必须全量 GET）。
pub(crate) fn load_base(conn: &rusqlite::Connection) -> rusqlite::Result<Validators> {
    if repo::get_meta(conn, mk::REMOTE_ENVELOPE)?.is_none() {
        return Ok(Validators::default());
    }
    Ok(Validators {
        etag: repo::get_meta(conn, mk::BASE_ETAG)?.filter(|s| !s.is_empty()),
        last_modified: repo::get_meta(conn, mk::BASE_LAST_MODIFIED)?.filter(|s| !s.is_empty()),
    })
}

/// 读取基准版本的信封（没有则为空对象）。
pub(crate) fn load_envelope(conn: &rusqlite::Connection) -> rusqlite::Result<Map<String, Value>> {
    Ok(repo::get_meta(conn, mk::REMOTE_ENVELOPE)?
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_default())
}

/// 写入基准校验器。缺失的校验器删除对应键（不留陈旧值）。
pub(crate) fn store_validators(
    conn: &rusqlite::Connection,
    v: &Validators,
) -> rusqlite::Result<()> {
    match v.etag.as_deref() {
        Some(etag) => repo::set_meta(conn, mk::BASE_ETAG, etag)?,
        None => repo::delete_meta(conn, mk::BASE_ETAG)?,
    }
    match v.last_modified.as_deref() {
        Some(lm) => repo::set_meta(conn, mk::BASE_LAST_MODIFIED, lm)?,
        None => repo::delete_meta(conn, mk::BASE_LAST_MODIFIED)?,
    }
    Ok(())
}

/// 写入基准（校验器 + 信封）。
pub(crate) fn store_base(
    conn: &rusqlite::Connection,
    v: &Validators,
    envelope: &Map<String, Value>,
) -> rusqlite::Result<()> {
    store_validators(conn, v)?;
    let raw = serde_json::to_string(envelope).unwrap_or_else(|_| "{}".to_string());
    repo::set_meta(conn, mk::REMOTE_ENVELOPE, &raw)
}

/// `fetch_and_merge` 的结果。
#[derive(Debug, Default)]
pub(crate) struct Fetched {
    /// 远端 sync-data 是否存在（304 / 200 → true，404 → false）。
    pub remote_exists: bool,
    /// 是否拿到并合并了新内容（200）。
    pub changed: bool,
    /// 合并之后远端仍缺少本地已有的内容（记录、较新的版本或墓碑），需要重新推送。
    /// 远端 404 而本地有数据时同样为真。304 时为假（远端就是已合并 / 已写入的基准）。
    pub remote_behind: bool,
    pub stats: MergeStats,
}

/// 条件 GET + 合并。`force_full` 时不带 `If-None-Match`（412 之后、周期性兜底时用，
/// 保证拿到最新内容与校验器）。
///
/// - 304：远端仍是基准版本；响应若带新的校验器（Apache 的弱 ETag 一秒后变强）就刷新基准
/// - 404：远端不存在；清空基准校验器（信封保留，重建文档时还能带上 settings）
/// - 200：解析失败直接报错（绝不上传覆盖读不懂的远端）；合并 + 记录基准在同一事务里提交
pub(crate) fn fetch_and_merge(ctx: &SyncCtx, force_full: bool) -> anyhow::Result<Fetched> {
    let dav = ctx.dav()?;
    let tz = ctx.cfg.timezone;
    let base = ctx
        .db
        .with_conn(|c| load_base(c))
        .context("读同步基准失败")?;
    let if_none_match = if force_full { None } else { base.etag.clone() };

    let outcome = dav.get_conditional(SYNC_DATA_FILE, if_none_match.as_deref())?;
    let now = now_local_string(tz);
    match outcome {
        GetOutcome::NotModified(fresh) => {
            if if_none_match.is_none() {
                anyhow::bail!("WebDAV 对无条件 GET 返回了 304");
            }
            // If-None-Match 得到的 304 证明"当前版本就是基准版本"：采用响应里更新的
            // 校验器（例如 Apache 的弱 ETag 一秒后变强）。
            let refreshed = Validators {
                etag: fresh.etag.or(base.etag),
                last_modified: fresh.last_modified.or(base.last_modified),
            };
            ctx.db
                .with_conn(|c| -> rusqlite::Result<()> {
                    store_validators(c, &refreshed)?;
                    mark_pull_ok(c, &now)
                })
                .context("写 meta 失败")?;
            debug!(target: "minitodo_cloud::pull", "remote unchanged (304)");
            Ok(Fetched {
                remote_exists: true,
                ..Default::default()
            })
        }
        GetOutcome::NotFound => {
            let has_local = ctx
                .db
                .with_conn(|c| -> rusqlite::Result<bool> {
                    store_validators(c, &Validators::default())?;
                    mark_pull_ok(c, &now)?;
                    repo::has_any_sync_state(c)
                })
                .context("写 meta 失败")?;
            debug!(target: "minitodo_cloud::pull", "remote sync-data.json.gz 尚不存在（404）");
            Ok(Fetched {
                remote_behind: has_local,
                ..Default::default()
            })
        }
        GetOutcome::Fetched { body, validators } => {
            let json = gunzip(&body)?;
            let doc = RemoteDoc::parse(&json)?;
            let cutoff = days_ago_local_string(tz, TOMBSTONE_RETENTION_DAYS);
            let recheck_cutoff =
                days_ago_local_string(tz, TOMBSTONE_RETENTION_DAYS - TOMBSTONE_RECHECK_MARGIN_DAYS);
            let settings_snapshot = doc.settings().cloned().unwrap_or(Value::Null).to_string();
            let (stats, remote_behind) = ctx
                .db
                .with_conn(|c| -> rusqlite::Result<(MergeStats, bool)> {
                    let tx = c.transaction()?;
                    let stats = merge::apply_remote_doc(&tx, &doc, tz, &cutoff)?;
                    let behind = merge::remote_lacks_local_state(&tx, &doc, tz, &recheck_cutoff)?;
                    store_base(&tx, &validators, &doc.envelope())?;
                    // settings 整 JSON 存一行，只作调试快照（上传时用的是信封里的 settings）
                    repo::set_setting(&tx, "all", &settings_snapshot)?;
                    mark_pull_ok(&tx, &now)?;
                    tx.commit()?;
                    Ok((stats, behind))
                })
                .context("合并远端 sync-data 失败")?;
            backfill_missing_seq(&ctx.db).context("回填 seq 失败")?;
            ctx.request_image_mirror();
            info!(
                target: "minitodo_cloud::pull",
                "merged remote: todos +{} -{}, subtasks +{} -{}, tombstones +{}, skipped {}{}{}",
                stats.todos_upserted,
                stats.todos_deleted,
                stats.subtasks_upserted,
                stats.subtasks_deleted,
                stats.tombstones_applied,
                stats.records_skipped,
                if stats.legacy_cleanup { " (legacy cleanup)" } else { "" },
                if remote_behind { " (remote is missing local changes)" } else { "" }
            );
            Ok(Fetched {
                remote_exists: true,
                changed: true,
                remote_behind,
                stats,
            })
        }
    }
}

fn mark_pull_ok(conn: &rusqlite::Connection, now: &str) -> rusqlite::Result<()> {
    repo::set_meta(conn, mk::LAST_PULL_AT, now)?;
    repo::delete_meta(conn, mk::LAST_PULL_ERROR)
}

/// 一次 pull 的结果。
#[derive(Debug, Default)]
pub struct PullReport {
    pub remote_exists: bool,
    pub changed: bool,
    /// 远端缺少本地内容，已标脏安排重新推送。
    pub repush_scheduled: bool,
    pub stats: MergeStats,
}

/// 单次 pull（带 `If-None-Match`）。见 `pull_once_with`。
pub fn pull_once(ctx: &SyncCtx) -> anyhow::Result<PullReport> {
    pull_once_with(ctx, false)
}

/// 单次 pull：（条件）GET + 合并 + seq 回填 + 远端落后检测。`force_full` 时无条件 GET。
/// 失败时把（脱敏后的）错误写进 `meta.last_pull_error`。
/// 只能在阻塞上下文、同步锁内调用（见 `SyncCtx::run_locked`）。
pub fn pull_once_with(ctx: &SyncCtx, force_full: bool) -> anyhow::Result<PullReport> {
    let res = fetch_and_merge(ctx, force_full).and_then(|f| {
        // 不管远端是否变化，本地都可能有还没分配 cloud 短码的 todo（例如旧版本遗留），
        // 每次 pull 末尾扫一遍补上；开销 O(N) 且只命中没 seq 的。
        let n = backfill_missing_seq(&ctx.db).context("回填 seq 失败")?;
        if n > 0 {
            info!(target: "minitodo_cloud::pull", "backfilled {} todo seq(s)", n);
        }
        // 存量记录的 K6 归一化：只做一次，且在远端最新内容合并进来之后（见 db::normalize）。
        // 失败不影响这次 pull 的结果，下次 pull 再试。
        match ctx
            .db
            .with_conn(|c| normalize::normalize_cache_once(c, ctx.cfg.timezone))
        {
            Ok(Some(r)) if r.any() => info!(
                target: "minitodo_cloud::pull",
                "normalized cached records (K6): todos repaired {} / reformatted {}, subtasks repaired {} / reformatted {}, corrupt {}",
                r.todos_repaired,
                r.todos_reformatted,
                r.subtasks_repaired,
                r.subtasks_reformatted,
                r.corrupt_rows
            ),
            Ok(_) => {}
            Err(e) => warn!(target: "minitodo_cloud::pull", "归一化存量记录失败，下次 pull 重试: {}", e),
        }
        let mut repush_scheduled = false;
        if f.remote_behind {
            // 只在 pull 里标脏：push 自己合并时远端落后是常态（本地正有待推送的写入），
            // 而它马上就会把并集写回去
            let newly = ctx
                .db
                .with_conn(|c| -> rusqlite::Result<bool> {
                    if repo::is_dirty(c)? {
                        return Ok(false);
                    }
                    repo::mark_dirty(c)?;
                    Ok(true)
                })
                .context("标记重新推送失败")?;
            if newly {
                info!(
                    target: "minitodo_cloud::pull",
                    "remote sync-data is missing local records / tombstones; scheduling a re-push"
                );
            }
            repush_scheduled = true;
        }
        Ok(PullReport {
            remote_exists: f.remote_exists,
            changed: f.changed,
            repush_scheduled,
            stats: f.stats,
        })
    });
    if let Err(e) = &res {
        let msg = public_error_message(e);
        if let Err(db_err) = ctx
            .db
            .with_conn(|c| repo::set_meta(c, mk::LAST_PULL_ERROR, &msg))
        {
            warn!(target: "minitodo_cloud::pull", "记录 last_pull_error 失败: {}", db_err);
        }
    }
    res
}

/// 扫 todos 表，给在 `todo_seq` 中无对应行的 todo 分配 seq。
/// 不修改 data_json / updated_at（避免触发不必要的 dirty 同步），seq 仅本地表持有。
pub(crate) fn backfill_missing_seq(db: &Db) -> rusqlite::Result<usize> {
    db.with_conn(|conn| -> rusqlite::Result<usize> {
        let ids = repo::todo_ids_without_seq(conn)?;
        for id in &ids {
            repo::assign_seq(conn, id)?;
        }
        Ok(ids.len())
    })
}

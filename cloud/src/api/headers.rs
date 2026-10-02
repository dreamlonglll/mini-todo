//! 同步状态计算 + 响应 header 注入。
//!
//! - `X-Sync-Status: healthy | stale | offline`，取 pull 与 push 中**较差**的一个：
//!   * pull：最近一次成功拉取（`meta.last_pull_at`）距今
//!     ≤ `pull_interval * 2` → healthy；≤ 5 分钟 → stale；更久或从未成功 → offline
//!   * push：本地待推送内容（`dirty` / 图片队列）积压时长（`dirty_since` /
//!     `image_queue_since`）≤ 2 分钟 → healthy；≤ 5 分钟 → stale；更久 → offline；
//!     没有积压 → healthy
//! - `X-Last-Sync-At: <meta.last_pull_at 原值>`（本地墙钟，与 PC SQLite 字符串一致）
//! - offline 时额外加 `Warning: 110 - "sync offline"`（RFC 7234）
//!
//! 这个中间件挂在鉴权中间件**内层**：401 直接由鉴权层返回，不注入同步头、不查库。

use axum::extract::{Request, State};
use axum::http::header::HeaderValue;
use axum::http::HeaderName;
use axum::middleware::Next;
use axum::response::Response;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use rusqlite::Connection;
use tracing::warn;

use super::AppState;
use crate::db::repo::{self, meta_keys as mk};
use crate::time::{epoch_to_local_string, local_string_to_utc};

const X_SYNC_STATUS: HeaderName = HeaderName::from_static("x-sync-status");
const X_LAST_SYNC_AT: HeaderName = HeaderName::from_static("x-last-sync-at");

/// 超过这个时长没有成功同步就是 offline。
const OFFLINE_AFTER_SECS: u64 = 300;
/// 本地写入积压超过这个时长就不再算 healthy。
const PUSH_HEALTHY_SECS: u64 = 120;

/// 同步健康等级（可比较：越大越差）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Healthy,
    Stale,
    Offline,
}

impl Level {
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Healthy => "healthy",
            Level::Stale => "stale",
            Level::Offline => "offline",
        }
    }
}

/// 从 meta 读出的原始同步状态。
#[derive(Debug, Default, Clone)]
pub struct SyncSnapshot {
    pub last_pull_at: Option<String>,
    pub last_pull_error: Option<String>,
    pub last_push_ok_at: Option<String>,
    pub last_push_error: Option<String>,
    pub dirty: bool,
    /// UNIX 秒。
    pub dirty_since: Option<i64>,
    pub image_queue_length: usize,
    /// UNIX 秒。
    pub image_queue_since: Option<i64>,
}

/// 判定后的同步状态。
#[derive(Debug, Clone)]
pub struct SyncStatus {
    pub overall: Level,
    pub pull: Level,
    pub push: Level,
    pub snapshot: SyncSnapshot,
    /// `dirty_since` 换算成本地墙钟（展示用）。
    pub dirty_since_local: Option<String>,
}

fn non_empty(v: Option<String>) -> Option<String> {
    v.filter(|s| !s.trim().is_empty())
}

fn meta_epoch(conn: &Connection, key: &str) -> rusqlite::Result<Option<i64>> {
    Ok(repo::get_meta(conn, key)?.and_then(|s| s.trim().parse::<i64>().ok()))
}

/// 一次连接调用读出全部同步状态。
pub fn read_snapshot(conn: &Connection) -> rusqlite::Result<SyncSnapshot> {
    let dirty = repo::is_dirty(conn)?;
    let image_queue_length = repo::dirty_image_queue(conn)?.len();
    Ok(SyncSnapshot {
        last_pull_at: non_empty(repo::get_meta(conn, mk::LAST_PULL_AT)?),
        last_pull_error: non_empty(repo::get_meta(conn, mk::LAST_PULL_ERROR)?),
        last_push_ok_at: non_empty(repo::get_meta(conn, mk::LAST_PUSH_OK_AT)?),
        last_push_error: non_empty(repo::get_meta(conn, mk::LAST_PUSH_ERROR)?),
        dirty,
        dirty_since: if dirty {
            meta_epoch(conn, mk::DIRTY_SINCE)?
        } else {
            None
        },
        image_queue_length,
        image_queue_since: if image_queue_length > 0 {
            meta_epoch(conn, mk::IMAGE_QUEUE_SINCE)?
        } else {
            None
        },
    })
}

fn pull_level(snapshot: &SyncSnapshot, now: DateTime<Utc>, tz: Tz, pull_interval: u64) -> Level {
    let Some(at) = snapshot
        .last_pull_at
        .as_deref()
        .and_then(|s| local_string_to_utc(s, tz))
    else {
        return Level::Offline;
    };
    let age = (now - at).num_seconds().max(0) as u64;
    if age <= pull_interval.saturating_mul(2) {
        Level::Healthy
    } else if age <= OFFLINE_AFTER_SECS {
        Level::Stale
    } else {
        Level::Offline
    }
}

fn push_level(snapshot: &SyncSnapshot, now: DateTime<Utc>) -> Level {
    let pending = snapshot.dirty || snapshot.image_queue_length > 0;
    if !pending {
        return Level::Healthy;
    }
    let since = match (snapshot.dirty_since, snapshot.image_queue_since) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    };
    let Some(since) = since else {
        // 有积压但不知道从什么时候开始（worker 还没观察到图片队列）：按刚开始算
        return Level::Healthy;
    };
    let age = (now.timestamp() - since).max(0) as u64;
    if age <= PUSH_HEALTHY_SECS {
        Level::Healthy
    } else if age <= OFFLINE_AFTER_SECS {
        Level::Stale
    } else {
        Level::Offline
    }
}

/// 纯函数：根据快照与当前时刻判定状态。
pub fn classify(
    snapshot: SyncSnapshot,
    now: DateTime<Utc>,
    tz: Tz,
    pull_interval: u64,
) -> SyncStatus {
    let pull = pull_level(&snapshot, now, tz, pull_interval);
    let push = push_level(&snapshot, now);
    let dirty_since_local = snapshot
        .dirty_since
        .and_then(|secs| epoch_to_local_string(secs, tz));
    SyncStatus {
        overall: pull.max(push),
        pull,
        push,
        snapshot,
        dirty_since_local,
    }
}

pub fn compute_sync_status(state: &AppState) -> SyncStatus {
    // 读失败按"什么都不知道"处理（pull 判 offline），但要留日志——header 注入
    // 不该因为一次 DB 错误静默降级成"看起来只是还没同步过"。
    let snapshot = match state.db.with_conn(|conn| read_snapshot(conn)) {
        Ok(s) => s,
        Err(e) => {
            warn!(target: "minitodo_cloud::api", "读同步状态失败: {}", e);
            SyncSnapshot::default()
        }
    };
    classify(
        snapshot,
        Utc::now(),
        state.config.timezone,
        state.config.pull_interval_secs,
    )
}

pub async fn inject_sync_headers(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let mut resp = next.run(req).await;
    let status = compute_sync_status(&state);

    resp.headers_mut().insert(
        X_SYNC_STATUS,
        HeaderValue::from_static(status.overall.as_str()),
    );
    if let Some(ref last) = status.snapshot.last_pull_at {
        if let Ok(v) = HeaderValue::from_str(last) {
            resp.headers_mut().insert(X_LAST_SYNC_AT, v);
        }
    }
    if status.overall == Level::Offline {
        let v = HeaderValue::from_static(r#"110 - "sync offline""#);
        resp.headers_mut().insert(axum::http::header::WARNING, v);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn tz() -> Tz {
        "Asia/Shanghai".parse().unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 2, 2, 0, 0).unwrap() // 上海 10:00:00
    }

    fn pulled_at(local: &str) -> SyncSnapshot {
        SyncSnapshot {
            last_pull_at: Some(local.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn pull_levels_by_age() {
        let s = classify(pulled_at("2026-10-02 09:59:00"), now(), tz(), 60);
        assert_eq!(s.pull, Level::Healthy);
        let s = classify(pulled_at("2026-10-02 09:57:00"), now(), tz(), 60);
        assert_eq!(s.pull, Level::Stale);
        let s = classify(pulled_at("2026-10-02 09:50:00"), now(), tz(), 60);
        assert_eq!(s.pull, Level::Offline);
        let s = classify(SyncSnapshot::default(), now(), tz(), 60);
        assert_eq!(s.pull, Level::Offline);
        let s = classify(pulled_at("garbage"), now(), tz(), 60);
        assert_eq!(s.pull, Level::Offline);
    }

    #[test]
    fn push_levels_by_backlog_age_and_overall_takes_worse() {
        let base = pulled_at("2026-10-02 09:59:30");
        let mut s = base.clone();
        s.dirty = true;
        s.dirty_since = Some(now().timestamp() - 30);
        let st = classify(s.clone(), now(), tz(), 60);
        assert_eq!((st.push, st.overall), (Level::Healthy, Level::Healthy));

        s.dirty_since = Some(now().timestamp() - 200);
        let st = classify(s.clone(), now(), tz(), 60);
        assert_eq!((st.push, st.overall), (Level::Stale, Level::Stale));

        s.dirty_since = Some(now().timestamp() - 600);
        let st = classify(s, now(), tz(), 60);
        assert_eq!(
            (st.pull, st.push, st.overall),
            (Level::Healthy, Level::Offline, Level::Offline)
        );
        assert!(st.dirty_since_local.is_some());

        // 只有图片积压也算
        let mut img = base;
        img.image_queue_length = 2;
        img.image_queue_since = Some(now().timestamp() - 400);
        assert_eq!(classify(img, now(), tz(), 60).push, Level::Offline);
    }

    #[test]
    fn read_snapshot_reflects_meta() {
        let c = Connection::open_in_memory().unwrap();
        crate::db::schema::init(&c).unwrap();
        let empty = read_snapshot(&c).unwrap();
        assert!(!empty.dirty);
        assert_eq!(empty.image_queue_length, 0);
        repo::mark_dirty(&c).unwrap();
        repo::enqueue_dirty_image(&c, "a.png").unwrap();
        repo::set_meta(&c, mk::LAST_PUSH_ERROR, "boom").unwrap();
        let s = read_snapshot(&c).unwrap();
        assert!(s.dirty);
        assert!(s.dirty_since.is_some());
        assert_eq!(s.image_queue_length, 1);
        assert_eq!(s.last_push_error.as_deref(), Some("boom"));
    }
}

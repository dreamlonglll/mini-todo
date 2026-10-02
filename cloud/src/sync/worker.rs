//! 后台同步 worker。
//!
//! - pull 循环：每 `pull_interval` 秒一次；失败按指数退避 + 抖动拉长间隔（上限 5 分钟）。
//!   平时用 `If-None-Match` 条件拉取，每 `FULL_PULL_EVERY` 轮做一次无条件全量拉取兜底：
//!   nginx 的 ETag 只有秒级精度（`mtime秒-长度`），同一秒内写入且长度相同的修改会被
//!   304 掩盖，全量拉取保证这种修改最多晚一个周期被合并
//! - push 循环：每 500ms 看一眼 `meta.dirty` / 图片队列（只读本地 SQLite，不触网）；
//!   连续写入去抖 1.5s（最长攒 10s），失败指数退避 + 抖动（1s 起，上限 5 分钟）
//! - 图片镜像循环：pull 拿到新文档后被唤醒，补下缺失图片；有下载失败时退避重试
//! - 日志只在状态变化时打 warn（正常 → 失败），持续失败降为 debug，恢复时 info
//! - 停机：收到 shutdown 信号后各循环在下一个等待点退出；`final_push` 在限时内补推一次

use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use tokio::sync::watch;
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::db::repo;
use crate::db::Db;
use crate::sync::{images, pull, push, SyncCtx};

/// 退避上限。
pub const MAX_BACKOFF: Duration = Duration::from_secs(300);
/// push 循环检查本地状态的间隔。
const PUSH_TICK: Duration = Duration::from_millis(500);
/// 最后一次写入之后静默这么久才推送（连续写入合并成一次 PUT）。
const PUSH_DEBOUNCE: Duration = Duration::from_millis(1500);
/// 持续写入时最多攒这么久就推一次。
const PUSH_MAX_DELAY: Duration = Duration::from_secs(10);
/// push 失败退避的起点。
const PUSH_BACKOFF_BASE: Duration = Duration::from_secs(1);
/// 图片镜像失败退避的起点。
const MIRROR_BACKOFF_BASE: Duration = Duration::from_secs(30);
/// 每隔这么多轮成功的 pull 做一次无条件全量拉取（默认间隔 60s → 约 10 分钟一次）。
pub const FULL_PULL_EVERY: u32 = 10;

/// 第 `round` 轮（从 1 起）pull 是否做无条件全量拉取。
pub fn is_full_pull_round(round: u32) -> bool {
    round > 0 && round.is_multiple_of(FULL_PULL_EVERY)
}

/// 指数退避 + 抖动：`base * 2^(failures-1)`，乘以 `[0.8, 1.2)` 的抖动系数，
/// 结果不超过 `max`。`failures == 0` 返回 `base`。`jitter` 取 `[0, 1)`。
pub fn backoff_delay(base: Duration, failures: u32, max: Duration, jitter: f64) -> Duration {
    let exp = failures.saturating_sub(1).min(32) as i32;
    let raw = base.as_secs_f64() * 2f64.powi(exp);
    let factor = 0.8 + 0.4 * jitter.clamp(0.0, 1.0);
    let secs = (raw.min(max.as_secs_f64()) * factor).min(max.as_secs_f64());
    Duration::from_secs_f64(secs.max(0.0))
}

fn jitter() -> f64 {
    rand::thread_rng().gen::<f64>()
}

/// 连续失败计数 + 下次允许尝试的时间。
#[derive(Debug)]
pub(crate) struct Backoff {
    base: Duration,
    max: Duration,
    failures: u32,
    next_at: Option<Instant>,
}

impl Backoff {
    pub(crate) fn new(base: Duration, max: Duration) -> Self {
        Backoff {
            base,
            max,
            failures: 0,
            next_at: None,
        }
    }

    pub(crate) fn ready(&self, now: Instant) -> bool {
        self.next_at.is_none_or(|t| now >= t)
    }

    /// 记一次失败，返回 (连续失败次数, 本次退避时长)。
    pub(crate) fn on_failure(&mut self, now: Instant) -> (u32, Duration) {
        self.failures = self.failures.saturating_add(1);
        let delay = backoff_delay(self.base, self.failures, self.max, jitter());
        self.next_at = Some(now + delay);
        (self.failures, delay)
    }

    /// 记一次成功，返回此前的连续失败次数。
    pub(crate) fn on_success(&mut self) -> u32 {
        let prev = self.failures;
        self.failures = 0;
        self.next_at = None;
        prev
    }
}

/// push 去抖：观察 `dirty_generation`，最后一次变化后静默 `PUSH_DEBOUNCE` 才放行，
/// 但从第一次观察到待推送起最多等 `PUSH_MAX_DELAY`。
#[derive(Debug)]
pub(crate) struct Debounce {
    quiet: Duration,
    max_delay: Duration,
    observed_gen: Option<i64>,
    changed_at: Option<Instant>,
    pending_since: Option<Instant>,
}

impl Debounce {
    pub(crate) fn new(quiet: Duration, max_delay: Duration) -> Self {
        Debounce {
            quiet,
            max_delay,
            observed_gen: None,
            changed_at: None,
            pending_since: None,
        }
    }

    /// 观察一次当前 generation，返回是否可以推送。
    pub(crate) fn observe(&mut self, generation: i64, now: Instant) -> bool {
        if self.observed_gen != Some(generation) {
            self.observed_gen = Some(generation);
            self.changed_at = Some(now);
        }
        let pending_since = *self.pending_since.get_or_insert(now);
        let changed_at = self.changed_at.unwrap_or(now);
        now.duration_since(changed_at) >= self.quiet
            || now.duration_since(pending_since) >= self.max_delay
    }

    /// 没有待推送内容 / 推送成功后重置。
    pub(crate) fn reset(&mut self) {
        self.pending_since = None;
        self.changed_at = None;
    }
}

/// 本地待推送状态（只读 SQLite，不触网）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pending {
    pub dirty: bool,
    pub generation: i64,
    pub image_queue_len: usize,
}

impl Pending {
    pub(crate) fn any(&self) -> bool {
        self.dirty || self.image_queue_len > 0
    }
}

pub(crate) fn read_pending(db: &Db) -> rusqlite::Result<Pending> {
    db.with_conn(|c| {
        Ok(Pending {
            dirty: repo::is_dirty(c)?,
            generation: repo::get_dirty_generation(c)?,
            image_queue_len: repo::dirty_image_queue(c)?.len(),
        })
    })
}

/// 启动全部后台循环，返回它们的 JoinHandle（停机时等待退出用）。
pub fn spawn_workers(
    ctx: Arc<SyncCtx>,
    shutdown: watch::Receiver<bool>,
) -> Vec<tokio::task::JoinHandle<()>> {
    vec![
        tokio::spawn(pull_loop(ctx.clone(), shutdown.clone())),
        tokio::spawn(push_loop(ctx.clone(), shutdown.clone())),
        tokio::spawn(mirror_loop(ctx, shutdown)),
    ]
}

/// 等待 `delay` 或停机信号；返回 `true` 表示应当退出。
async fn sleep_or_shutdown(delay: Duration, shutdown: &mut watch::Receiver<bool>) -> bool {
    if *shutdown.borrow() {
        return true;
    }
    tokio::select! {
        _ = tokio::time::sleep(delay) => *shutdown.borrow(),
        // 发送端被 drop 也视为停机
        res = shutdown.changed() => res.is_err() || *shutdown.borrow(),
    }
}

async fn pull_loop(ctx: Arc<SyncCtx>, mut shutdown: watch::Receiver<bool>) {
    let interval = Duration::from_secs(ctx.cfg.pull_interval_secs);
    let mut backoff = Backoff::new(interval, interval.max(MAX_BACKOFF));
    let mut delay = interval;
    let mut round: u32 = 0;
    loop {
        if sleep_or_shutdown(delay, &mut shutdown).await {
            break;
        }
        round = round.wrapping_add(1);
        let full = is_full_pull_round(round);
        match ctx.run_locked(move |c| pull::pull_once_with(c, full)).await {
            Ok(_) => {
                let prev = backoff.on_success();
                if prev > 0 {
                    info!(target: "minitodo_cloud::pull", "pull recovered after {} consecutive failure(s)", prev);
                }
                delay = interval;
            }
            Err(e) => {
                let (failures, next) = backoff.on_failure(Instant::now());
                if failures == 1 {
                    warn!(target: "minitodo_cloud::pull", "pull failed, backing off: {:#}", e);
                } else {
                    debug!(
                        target: "minitodo_cloud::pull",
                        "pull failed {} times in a row, next try in {:?}: {:#}", failures, next, e
                    );
                }
                delay = next.max(Duration::from_secs(1));
            }
        }
    }
    debug!(target: "minitodo_cloud::pull", "pull loop stopped");
}

async fn push_loop(ctx: Arc<SyncCtx>, mut shutdown: watch::Receiver<bool>) {
    let mut backoff = Backoff::new(PUSH_BACKOFF_BASE, MAX_BACKOFF);
    let mut debounce = Debounce::new(PUSH_DEBOUNCE, PUSH_MAX_DELAY);
    let mut db_error_logged = false;
    loop {
        if sleep_or_shutdown(PUSH_TICK, &mut shutdown).await {
            break;
        }
        let pending = match read_pending(&ctx.db) {
            Ok(p) => {
                db_error_logged = false;
                p
            }
            Err(e) => {
                if !db_error_logged {
                    warn!(target: "minitodo_cloud::push", "读本地待推送状态失败: {}", e);
                    db_error_logged = true;
                }
                continue;
            }
        };
        if !pending.any() {
            debounce.reset();
            let prev = backoff.on_success();
            if prev > 0 {
                info!(target: "minitodo_cloud::push", "nothing left to push after {} failure(s)", prev);
            }
            continue;
        }
        let now = Instant::now();
        if pending.dirty && !debounce.observe(pending.generation, now) {
            continue;
        }
        if !backoff.ready(now) {
            continue;
        }
        match ctx.run_locked(push::push_once).await {
            Ok(_) => {
                debounce.reset();
                let prev = backoff.on_success();
                if prev > 0 {
                    info!(target: "minitodo_cloud::push", "push recovered after {} consecutive failure(s)", prev);
                }
            }
            Err(e) => {
                let (failures, next) = backoff.on_failure(Instant::now());
                if failures == 1 {
                    warn!(target: "minitodo_cloud::push", "push failed, backing off: {:#}", e);
                } else {
                    debug!(
                        target: "minitodo_cloud::push",
                        "push failed {} times in a row, next try in {:?}: {:#}", failures, next, e
                    );
                }
            }
        }
    }
    debug!(target: "minitodo_cloud::push", "push loop stopped");
}

async fn mirror_loop(ctx: Arc<SyncCtx>, mut shutdown: watch::Receiver<bool>) {
    let mut backoff = Backoff::new(MIRROR_BACKOFF_BASE, MAX_BACKOFF);
    let mut retry_at: Option<Instant> = None;
    loop {
        if *shutdown.borrow() {
            break;
        }
        let retry_deadline = retry_at;
        let retry = async move {
            match retry_deadline {
                Some(t) => tokio::time::sleep_until(t).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            _ = ctx.mirror_requested() => {}
            _ = retry => {}
            res = shutdown.changed() => {
                if res.is_err() || *shutdown.borrow() {
                    break;
                }
                continue;
            }
        }
        retry_at = None;
        // 镜像只新建本地缺失的文件，不碰 SQLite 记录，不持同步锁
        let job_ctx = ctx.clone();
        let res = tokio::task::spawn_blocking(move || images::mirror_once(&job_ctx))
            .await
            .map_err(|e| anyhow::anyhow!("图片镜像任务异常退出: {}", e))
            .and_then(|r| r);
        let outcome = res.and_then(|report| {
            if report.downloaded > 0 {
                info!(
                    target: "minitodo_cloud::images",
                    "image mirror: {} new file(s) of {} listed", report.downloaded, report.remote_count
                );
            }
            match (&report.listing_error, report.failed) {
                (None, 0) => Ok(()),
                (Some(e), 0) => Err(anyhow::anyhow!("listing remote images failed: {}", e)),
                (_, failed) => Err(anyhow::anyhow!("{} image download(s) failed", failed)),
            }
        });
        match outcome {
            Ok(()) => {
                let prev = backoff.on_success();
                if prev > 0 {
                    info!(target: "minitodo_cloud::images", "image mirror recovered after {} failure(s)", prev);
                }
            }
            Err(e) => {
                let (failures, next) = backoff.on_failure(Instant::now());
                log_mirror_failure(failures, next, &format!("{:#}", e));
                retry_at = Some(Instant::now() + next);
            }
        }
    }
    debug!(target: "minitodo_cloud::images", "image mirror loop stopped");
}

fn log_mirror_failure(failures: u32, next: Duration, msg: &str) {
    if failures == 1 {
        warn!(target: "minitodo_cloud::images", "image mirror incomplete, retry in {:?}: {}", next, msg);
    } else {
        debug!(
            target: "minitodo_cloud::images",
            "image mirror failed {} times in a row, retry in {:?}: {}", failures, next, msg
        );
    }
}

/// 停机前限时补推一次（有待推送内容时）。超时只是放弃等待，不中断已经在跑的请求。
pub async fn final_push(ctx: &Arc<SyncCtx>, budget: Duration) {
    match read_pending(&ctx.db) {
        Ok(p) if p.any() => {}
        Ok(_) => return,
        Err(e) => {
            warn!(target: "minitodo_cloud::push", "停机前读待推送状态失败: {}", e);
            return;
        }
    }
    info!(target: "minitodo_cloud::push", "flushing pending changes before shutdown (budget {:?})", budget);
    match tokio::time::timeout(budget, ctx.run_locked(push::push_once)).await {
        Ok(Ok(_)) => info!(target: "minitodo_cloud::push", "final push ok"),
        Ok(Err(e)) => warn!(target: "minitodo_cloud::push", "final push failed: {:#}", e),
        Err(_) => warn!(target: "minitodo_cloud::push", "final push timed out after {:?}", budget),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_pull_every_n_rounds() {
        let rounds: Vec<u32> = (0..=25).filter(|r| is_full_pull_round(*r)).collect();
        assert_eq!(rounds, vec![10, 20]);
    }

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        let base = Duration::from_secs(1);
        let max = Duration::from_secs(300);
        // jitter = 0.5 → 系数 1.0
        assert_eq!(backoff_delay(base, 1, max, 0.5), Duration::from_secs(1));
        assert_eq!(backoff_delay(base, 2, max, 0.5), Duration::from_secs(2));
        assert_eq!(backoff_delay(base, 5, max, 0.5), Duration::from_secs(16));
        assert_eq!(backoff_delay(base, 20, max, 0.5), max);
        assert_eq!(backoff_delay(base, u32::MAX, max, 0.99), max);
    }

    #[test]
    fn backoff_jitter_stays_within_bounds() {
        let base = Duration::from_secs(10);
        let max = Duration::from_secs(300);
        let lo = backoff_delay(base, 2, max, 0.0);
        let hi = backoff_delay(base, 2, max, 0.999);
        assert!(
            lo >= Duration::from_secs(16) && lo <= Duration::from_secs(20),
            "{:?}",
            lo
        );
        assert!(
            hi > Duration::from_secs(20) && hi < Duration::from_secs(24),
            "{:?}",
            hi
        );
        // 抖动不会突破上限
        assert!(backoff_delay(base, 9, max, 0.999) <= max);
    }

    #[test]
    fn backoff_state_machine() {
        let mut b = Backoff::new(Duration::from_secs(1), MAX_BACKOFF);
        let t0 = Instant::now();
        assert!(b.ready(t0));
        let (n, d) = b.on_failure(t0);
        assert_eq!(n, 1);
        assert!(!b.ready(t0));
        assert!(b.ready(t0 + d));
        let (n2, _) = b.on_failure(t0);
        assert_eq!(n2, 2);
        assert_eq!(b.on_success(), 2);
        assert!(b.ready(t0));
        assert_eq!(b.on_success(), 0);
    }

    #[test]
    fn debounce_waits_for_quiet_period() {
        let mut d = Debounce::new(Duration::from_millis(1500), Duration::from_secs(10));
        let t0 = Instant::now();
        assert!(!d.observe(1, t0));
        assert!(!d.observe(1, t0 + Duration::from_millis(1000)));
        // 新写入重置静默计时
        assert!(!d.observe(2, t0 + Duration::from_millis(1200)));
        assert!(!d.observe(2, t0 + Duration::from_millis(2000)));
        assert!(d.observe(2, t0 + Duration::from_millis(2700)));
    }

    #[test]
    fn debounce_caps_total_delay_under_continuous_writes() {
        let mut d = Debounce::new(Duration::from_millis(1500), Duration::from_secs(10));
        let t0 = Instant::now();
        let mut released = None;
        for i in 0..30u64 {
            let t = t0 + Duration::from_millis(500 * i);
            if d.observe(i as i64, t) {
                released = Some(t);
                break;
            }
        }
        let released = released.expect("continuous writes must still be pushed");
        assert_eq!(released.duration_since(t0), Duration::from_secs(10));
        d.reset();
        assert!(!d.observe(100, t0 + Duration::from_secs(20)));
    }

    #[tokio::test]
    async fn sleep_or_shutdown_returns_early_on_signal() {
        let (tx, mut rx) = watch::channel(false);
        let handle =
            tokio::spawn(
                async move { sleep_or_shutdown(Duration::from_secs(3600), &mut rx).await },
            );
        tx.send(true).unwrap();
        let stopped = tokio::time::timeout(Duration::from_secs(5), handle)
            .await
            .expect("must return promptly")
            .unwrap();
        assert!(stopped);
    }
}

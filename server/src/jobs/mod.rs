pub mod alias_submitter;
pub mod audit_log_gc;
pub mod backfill_submitter;
pub mod notification_dispatcher;
pub mod qimao_alias_submitter;
pub mod qimao_backfill_submitter;
pub mod qimao_income_notice;
pub mod qimao_rank;
pub mod qimao_token_refresh;
pub mod tomato_income;
pub mod tomato_rank;

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Timelike;
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::db::DbPool;
use crate::services::job_log;

/// Drive a long-running poller with built-in execution logging.
pub async fn poller_loop<F, Fut>(
    name: &'static str,
    interval: Duration,
    pool: Arc<DbPool>,
    work: F,
) where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<usize, String>>,
{
    tracing::info!("{name}: worker starting");
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    loop {
        tick.tick().await;
        let t = Instant::now();
        match work().await {
            Ok(0) => {}
            Ok(n) => {
                let ms = t.elapsed().as_millis() as i64;
                tracing::info!("{name}: processed {n} item(s)");
                job_log::record(&pool, name, n, ms, None).await;
            }
            Err(e) => {
                let ms = t.elapsed().as_millis() as i64;
                tracing::warn!("{name}: round failed: {e}");
                job_log::record(&pool, name, 0, ms, Some(&e)).await;
            }
        }
    }
}

/// Definition of a once-per-day job eligible for compensation.
struct DailyJobDef {
    name: &'static str,
    /// Earliest wall-clock minute-of-day (local) at which the job may run.
    target_hour: u32,
    target_minute: u32,
}

const DAILY_JOBS: &[DailyJobDef] = &[
    DailyJobDef { name: "tomato_rank",  target_hour: 3, target_minute: 0  },
    DailyJobDef { name: "qimao_rank",   target_hour: 3, target_minute: 30 },
    DailyJobDef { name: "audit_log_gc", target_hour: 4, target_minute: 0  },
];

/// Returns true when `job_name` has a successful run recorded in
/// `job_runs` for today (local time).
async fn ran_today(pool: &DbPool, job_name: &str) -> bool {
    let now = chrono::Local::now();
    let today_local = match now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|dt| dt.and_local_timezone(chrono::Local).single())
    {
        Some(t) => t,
        None => return true, // can't determine — skip to be safe
    };
    sqlx::query_scalar(
        "SELECT EXISTS(\
           SELECT 1 FROM job_runs \
           WHERE job_name = $1 AND ran_at >= $2 AND success = true\
         )",
    )
    .bind(job_name)
    .bind(today_local)
    .fetch_one(pool)
    .await
    .unwrap_or(true) // on DB error assume ran, so we don't double-fire
}

/// Single-flight guard for compensation. Prevents two ticks from
/// running concurrently if one tick's daily-job execution exceeds the
/// 30-minute schedule interval (rare, but possible during platform
/// outages or DB latency spikes). Without this, both ticks could see
/// `ran_today=false` and double-fire the same daily job.
static COMPENSATION_RUNNING: AtomicBool = AtomicBool::new(false);

/// RAII guard that resets COMPENSATION_RUNNING to false on drop.
/// Ensures the flag is cleared even on panic or early return.
struct CompensationGuard;
impl Drop for CompensationGuard {
    fn drop(&mut self) {
        COMPENSATION_RUNNING.store(false, Ordering::Release);
    }
}

/// Compensation task — runs every 30 minutes via `tokio-cron-scheduler`.
///
/// For each daily job whose scheduled time has already passed today,
/// checks `job_runs`. If no successful record exists it runs the job
/// immediately. This makes every daily job restart-safe without polling
/// in application code.
async fn run_compensation(pool: Arc<DbPool>, abogus_url: Arc<String>) {
    // Single-flight: bail if a previous tick is still running.
    if COMPENSATION_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        tracing::debug!("compensation: previous tick still running, skip");
        return;
    }
    let _guard = CompensationGuard; // resets flag on drop

    let now = chrono::Local::now();
    let now_min = now.hour() * 60 + now.minute();

    for def in DAILY_JOBS {
        let target_min = def.target_hour * 60 + def.target_minute;
        if now_min < target_min {
            continue;
        }
        if ran_today(&pool, def.name).await {
            continue;
        }

        tracing::info!(
            "compensation: {} missed today — running now at {:02}:{:02}",
            def.name,
            now.hour(),
            now.minute()
        );

        let t = Instant::now();
        let result = match def.name {
            "tomato_rank" => tomato_rank::run(&pool, &abogus_url).await,
            "qimao_rank"  => qimao_rank::run(&pool).await,
            "audit_log_gc" => audit_log_gc::run(&pool).await,
            _ => Err(format!("unknown job: {}", def.name)),
        };

        let ms = t.elapsed().as_millis() as i64;
        match result {
            Ok(()) => {
                tracing::info!("compensation: {} done", def.name);
                job_log::record(&pool, def.name, 1, ms, None).await;
            }
            Err(e) => {
                tracing::error!("compensation: {} failed: {e}", def.name);
                job_log::record(&pool, def.name, 0, ms, Some(&e)).await;
            }
        }
    }
}

/// Spin up the cron scheduler and register all daily + compensation jobs,
/// then spawn all long-running worker tasks.
pub async fn start(pool: DbPool, abogus_url: String) -> Result<JobScheduler, String> {
    let sched = JobScheduler::new()
        .await
        .map_err(|e| format!("scheduler init: {e}"))?;
    let pool = Arc::new(pool);
    let abogus_url = Arc::new(abogus_url);

    // ── Daily jobs ───────────────────────────────────────────────────────────

    // 番茄达人书单 — 每天 03:00 (server local time).
    {
        let pool_c = pool.clone();
        let abogus_c = abogus_url.clone();
        let job = Job::new_async("0 0 3 * * *", move |_id, _sched| {
            let pool = pool_c.clone();
            let abogus = abogus_c.clone();
            Box::pin(async move {
                let t = Instant::now();
                match tomato_rank::run(&pool, &abogus).await {
                    Ok(()) => job_log::record(&pool, "tomato_rank", 1, t.elapsed().as_millis() as i64, None).await,
                    Err(e) => {
                        tracing::error!("tomato_rank: {e}");
                        job_log::record(&pool, "tomato_rank", 0, t.elapsed().as_millis() as i64, Some(&e)).await;
                    }
                }
            })
        })
        .map_err(|e| format!("tomato_rank job: {e}"))?;
        sched.add(job).await.map_err(|e| format!("add tomato_rank: {e}"))?;
    }

    // 七猫达人书单 — 每天 03:30.
    {
        let pool_q = pool.clone();
        let job = Job::new_async("0 30 3 * * *", move |_id, _sched| {
            let pool = pool_q.clone();
            Box::pin(async move {
                let t = Instant::now();
                match qimao_rank::run(&pool).await {
                    Ok(()) => job_log::record(&pool, "qimao_rank", 1, t.elapsed().as_millis() as i64, None).await,
                    Err(e) => {
                        tracing::error!("qimao_rank: {e}");
                        job_log::record(&pool, "qimao_rank", 0, t.elapsed().as_millis() as i64, Some(&e)).await;
                    }
                }
            })
        })
        .map_err(|e| format!("qimao_rank job: {e}"))?;
        sched.add(job).await.map_err(|e| format!("add qimao_rank: {e}"))?;
    }

    // 接口日志 GC — 每天 04:00.
    {
        let pool_gc = pool.clone();
        let job = Job::new_async("0 0 4 * * *", move |_id, _sched| {
            let pool = pool_gc.clone();
            Box::pin(async move {
                let t = Instant::now();
                match audit_log_gc::run(&pool).await {
                    Ok(()) => job_log::record(&pool, "audit_log_gc", 1, t.elapsed().as_millis() as i64, None).await,
                    Err(e) => {
                        tracing::error!("audit_log_gc: {e}");
                        job_log::record(&pool, "audit_log_gc", 0, t.elapsed().as_millis() as i64, Some(&e)).await;
                    }
                }
            })
        })
        .map_err(|e| format!("audit_log_gc job: {e}"))?;
        sched.add(job).await.map_err(|e| format!("add audit_log_gc: {e}"))?;
    }

    // ── Compensation job — 每 30 分钟检查并补跑漏掉的 daily job ────────────
    {
        let pool_comp = pool.clone();
        let abogus_comp = abogus_url.clone();
        let job_comp = Job::new_async("0 */30 * * * *", move |_id, _sched| {
            let pool = pool_comp.clone();
            let abogus = abogus_comp.clone();
            Box::pin(async move {
                run_compensation(pool, abogus).await;
            })
        })
        .map_err(|e| format!("compensation job: {e}"))?;
        sched.add(job_comp).await.map_err(|e| format!("add compensation: {e}"))?;
    }

    // ── 七猫月度收益通知 — 每月 10–20 日,9/13/21 点各跑一次 ───────────────
    // 七猫达人 publishes the monthly income statement as a feed
    // notice between days 10 and 20 of the following month. 33 fires
    // per month per profile; idempotency comes from the
    // `qimao_income_notice` PK so duplicate fires are safe. Not in
    // DAILY_JOBS so it doesn't get the every-30-min compensation
    // sweep — the 3×/day cadence already catches anything that
    // arrives during a brief outage.
    {
        let pool_qmn = pool.clone();
        let job = Job::new_async("0 0 9,13,21 10-20 * *", move |_id, _sched| {
            let pool = pool_qmn.clone();
            Box::pin(async move {
                let t = Instant::now();
                match qimao_income_notice::run(&pool).await {
                    Ok(()) => {
                        job_log::record(
                            &pool,
                            "qimao_income_notice",
                            1,
                            t.elapsed().as_millis() as i64,
                            None,
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::error!("qimao_income_notice: {e}");
                        job_log::record(
                            &pool,
                            "qimao_income_notice",
                            0,
                            t.elapsed().as_millis() as i64,
                            Some(&e),
                        )
                        .await;
                    }
                }
            })
        })
        .map_err(|e| format!("qimao_income_notice job: {e}"))?;
        sched.add(job).await.map_err(|e| format!("add qimao_income_notice: {e}"))?;
    }

    sched.start().await.map_err(|e| format!("start scheduler: {e}"))?;
    tracing::info!(
        "scheduler started: tomato_rank@03:00, qimao_rank@03:30, \
         audit_log_gc@04:00, compensation@*/30min"
    );

    // ── Long-running workers ─────────────────────────────────────────────────

    let pool_a = pool.clone();
    let abogus_a = abogus_url.clone();
    tokio::spawn(async move { alias_submitter::start_worker(pool_a, abogus_a).await });
    tracing::info!("worker started: alias_submitter (poll 2s)");

    let pool_b = pool.clone();
    let abogus_b = abogus_url.clone();
    tokio::spawn(async move { backfill_submitter::start_worker(pool_b, abogus_b).await });
    tracing::info!("worker started: backfill_submitter (poll 30s)");

    let pool_qt = pool.clone();
    tokio::spawn(async move { qimao_token_refresh::start_worker(pool_qt).await });
    tracing::info!("worker started: qimao_token_refresh (poll 30m)");

    let pool_qa = pool.clone();
    tokio::spawn(async move { qimao_alias_submitter::start_worker(pool_qa).await });
    tracing::info!("worker started: qimao_alias_submitter (poll 2s)");

    let pool_qb = pool.clone();
    tokio::spawn(async move { qimao_backfill_submitter::start_worker(pool_qb).await });
    tracing::info!("worker started: qimao_backfill_submitter (poll 30s)");

    let pool_nd = pool.clone();
    tokio::spawn(async move { notification_dispatcher::start_worker(pool_nd).await });
    tracing::info!("worker started: notification_dispatcher (poll 60s)");

    let pool_ti = pool.clone();
    let abogus_ti = abogus_url.clone();
    tokio::spawn(async move { tomato_income::start_worker(pool_ti, abogus_ti).await });
    tracing::info!("worker started: tomato_income (poll 600s, 2-min skew gate)");

    Ok(sched)
}

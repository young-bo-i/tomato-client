//! 定时任务执行记录服务。
//!
//! 每个 worker / cron job 跑完一轮后调用 `record()` 写入
//! `job_runs` 表，失败时只 warn 不 panic（审计日志不能阻塞主流程）。

use crate::db::DbPool;

pub async fn record(
    pool: &DbPool,
    job_name: &str,
    items_processed: usize,
    duration_ms: i64,
    error: Option<&str>,
) {
    let success = error.is_none();
    if let Err(e) = sqlx::query(
        r#"INSERT INTO job_runs
               (job_name, items_processed, duration_ms, success, error_reason)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(job_name)
    .bind(items_processed as i64)
    .bind(duration_ms as i32)
    .bind(success)
    .bind(error)
    .execute(pool)
    .await
    {
        tracing::warn!("job_log::record failed for {job_name}: {e}");
    }
}

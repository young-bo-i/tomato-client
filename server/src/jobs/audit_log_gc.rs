//! Daily GC for time-bounded server tables.
//!
//! Runs at 04:00 daily (see jobs/mod.rs). Tables and retention windows:
//!
//! | Table                   | Retention | Rationale                          |
//! |-------------------------|-----------|------------------------------------|
//! | external_api_responses  | 30 days   | Incident triage / ops queries      |
//! | douyin_videos           | 60 days   | backfill_submitter queries this at 29-day renewal; must outlive that cycle |
//! | tomato_aliases (failed) | 30 days   | Failed/rejected keywords; terminal, never retried |
//! | qimao_aliases  (failed) | 30 days   | Same                               |
//! | job_runs                | 90 days   | Execution history; lightweight     |
//!
//! ## Chunked DELETE
//!
//! Each table is deleted in chunks of `CHUNK_SIZE` rows with a brief
//! pause between batches. Rationale: `external_api_responses` accumulates
//! 20–50k rows per day (after sampling). A single big DELETE would:
//!   * generate a multi-MB WAL entry, triggering checkpoint pressure
//!   * hold a long-running transaction blocking autovacuum on the table
//!   * lock-conflict with concurrent INSERT from the workers (api_log
//!     writes are continuous, even at 04:00)
//!
//! `WHERE id IN (SELECT id ... LIMIT N)` lets Postgres stop the scan
//! after N rows are found. `id` is correlated with `created_at` /
//! `inserted_at` / `ran_at` (all serial inserts), so the seq-scan
//! exits early on the oldest rows.

use std::time::Duration;

use crate::db::DbPool;

const API_LOG_DAYS: i32 = 30;
const DOUYIN_VIDEO_DAYS: i32 = 60;
const FAILED_ALIAS_DAYS: i32 = 30;
const JOB_RUNS_DAYS: i32 = 90;

/// Rows per chunk. Sized so a single batch's DELETE TX is sub-second
/// even on slow disks, and the WAL spike fits comfortably under the
/// default checkpoint window.
const CHUNK_SIZE: i64 = 5_000;

/// Pause between chunks. Lets autovacuum clean up earlier dead rows
/// and gives concurrent INSERTs a chance to acquire the index lock.
const CHUNK_GAP: Duration = Duration::from_millis(200);

pub async fn run(pool: &DbPool) -> Result<(), String> {
    let api = chunked_delete(
        pool,
        "DELETE FROM external_api_responses WHERE id IN (\
            SELECT id FROM external_api_responses \
            WHERE created_at < NOW() - ($1::int * INTERVAL '1 day') \
            LIMIT $2)",
        API_LOG_DAYS,
    )
    .await
    .map_err(|e| format!("gc external_api_responses: {e}"))?;

    let videos = chunked_delete(
        pool,
        "DELETE FROM douyin_videos WHERE id IN (\
            SELECT id FROM douyin_videos \
            WHERE inserted_at < NOW() - ($1::int * INTERVAL '1 day') \
            LIMIT $2)",
        DOUYIN_VIDEO_DAYS,
    )
    .await
    .map_err(|e| format!("gc douyin_videos: {e}"))?;

    // Only clean terminal failed rows — submitted rows must stay for renewal.
    let ta = chunked_delete(
        pool,
        "DELETE FROM tomato_aliases WHERE id IN (\
            SELECT id FROM tomato_aliases \
            WHERE status = 'failed' \
              AND created_at < NOW() - ($1::int * INTERVAL '1 day') \
            LIMIT $2)",
        FAILED_ALIAS_DAYS,
    )
    .await
    .map_err(|e| format!("gc tomato_aliases: {e}"))?;

    let qa = chunked_delete(
        pool,
        "DELETE FROM qimao_aliases WHERE id IN (\
            SELECT id FROM qimao_aliases \
            WHERE status = 'failed' \
              AND created_at < NOW() - ($1::int * INTERVAL '1 day') \
            LIMIT $2)",
        FAILED_ALIAS_DAYS,
    )
    .await
    .map_err(|e| format!("gc qimao_aliases: {e}"))?;

    let jobs = chunked_delete(
        pool,
        "DELETE FROM job_runs WHERE id IN (\
            SELECT id FROM job_runs \
            WHERE ran_at < NOW() - ($1::int * INTERVAL '1 day') \
            LIMIT $2)",
        JOB_RUNS_DAYS,
    )
    .await
    .map_err(|e| format!("gc job_runs: {e}"))?;

    tracing::info!(
        "audit_log_gc: api_log -{} ({}d) | douyin_videos -{} ({}d) | \
         tomato_aliases[failed] -{} ({}d) | qimao_aliases[failed] -{} ({}d) | \
         job_runs -{} ({}d)",
        api, API_LOG_DAYS,
        videos, DOUYIN_VIDEO_DAYS,
        ta, FAILED_ALIAS_DAYS,
        qa, FAILED_ALIAS_DAYS,
        jobs, JOB_RUNS_DAYS,
    );
    Ok(())
}

/// Run `delete_sql` in CHUNK_SIZE-row batches until no rows match.
/// `delete_sql` must accept two parameters: `$1 = days_int`, `$2 = limit`.
/// Returns the total number of rows deleted across all batches.
async fn chunked_delete(
    pool: &DbPool,
    delete_sql: &str,
    days: i32,
) -> Result<u64, sqlx::Error> {
    let mut total: u64 = 0;
    loop {
        let n = sqlx::query(delete_sql)
            .bind(days)
            .bind(CHUNK_SIZE)
            .execute(pool)
            .await?
            .rows_affected();
        total += n;
        if n < CHUNK_SIZE as u64 {
            // Last batch — either we hit zero or the final partial batch.
            break;
        }
        // Pause to let autovacuum and concurrent writers breathe.
        tokio::time::sleep(CHUNK_GAP).await;
    }
    Ok(total)
}

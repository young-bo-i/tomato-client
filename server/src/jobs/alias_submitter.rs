//! Continuous worker that picks up `tomato_aliases` rows in
//! `status='pending'` state and submits them via the 番茄达人 promotion
//! API.
//!
//! Triggered indirectly: when the bulk-ingest endpoint inserts new
//! filtered words, it inserts `pending` rows here too. This worker
//! drains those.
//!
//! Design notes:
//!
//! - **Cookie source**: random pick from any admin profile whose tomato
//!   cookie is currently `is_online=TRUE` (see `services::tomato_cookie`).
//!   Random rather than freshest because traffic should spread evenly
//!   across accounts. One cookie per polling round — different rounds
//!   roll different accounts naturally.
//!
//! - **No per-request throttle**: the user explicitly opted out of
//!   sleep-between-requests. The 2-second poll interval below already
//!   smooths out bursts caused by 50 concurrent ingest browsers.
//!
//! - **Auth-failure handling**: HTTP 401/403 means "this cookie is
//!   dead". We mark the cookie offline (so future rounds skip it) and
//!   leave the alias row in `pending` so it gets retried with a
//!   different cookie next round. The row only goes to `failed` for
//!   non-auth errors (api code != 0, parse, transport).
//!
//! - **Attribution**: each successful or failed update stamps
//!   `submitted_by_profile_id` so the dashboard can break down
//!   per-account counts.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::fanqie_promotion::{
    build_http_client, submit_alias, ENDPOINT_PROMOTION_PLAN_CREATE, SERVICE_NAME,
};
use crate::services::tomato_cookie;

/// Poll cadence. Picked to be small enough that a freshly-enqueued
/// alias is visible to the platform within a few seconds, but large
/// enough that an idle DB scan is essentially free.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How many work-ticks between renewal scans. At POLL_INTERVAL=2s this
/// means renewals are checked every ~2 minutes during active periods.
/// Renewals (30-day TTL) don't benefit from more frequent checks.
const RENEWAL_CHECK_EVERY: u32 = 60;

/// Tick counter for renewal throttling. Only incremented when there is
/// actual pending work (idle rounds return before reaching the check).
static RENEWAL_TICK: AtomicU32 = AtomicU32::new(0);

/// How many `pending` rows we attempt per poll. Sized so one tick can
/// drain a typical ingest batch's worth (~50 rows × 3 alias_types = 150)
/// over a couple of ticks.
const BATCH_SIZE: i64 = 30;

/// Rows processed concurrently within each chunk. Mirrors backfill_submitter
/// and qimao_alias_submitter — overlaps network latency without overwhelming
/// the platform's per-IP rate limits. Same shared cookie across the chunk.
const CONCURRENCY: usize = 2;

/// Per-alias TTL on the platform side. After this much time elapses
/// since the last successful submission, 番茄达人 considers the alias
/// expired and we need to re-call plan/create. Renewal resets the
/// row's submission AND backfill state but preserves
/// `submitted_by_profile_id` (original credit) and
/// `backfill_link_history` (links have their own 29-day TTL that
/// survives the alias being re-issued with a new alias_id).
const ALIAS_RENEWAL_INTERVAL: &str = "30 days";

pub async fn start_worker(pool: Arc<DbPool>, abogus_url: Arc<String>) {
    let p = pool.clone();
    crate::jobs::poller_loop("alias_submitter", POLL_INTERVAL, p, move || {
        let pool = pool.clone();
        let abogus_url = abogus_url.clone();
        async move { process_pending(&pool, &abogus_url).await }
    })
    .await;
}

async fn process_pending(pool: &DbPool, abogus_url: &str) -> Result<usize, String> {
    // Pick by target_profile_id first (new routing), fall back to user_id
    // for legacy rows that have no target set.
    #[derive(sqlx::FromRow)]
    struct Peek { user_id: i32, target_profile_id: Option<uuid::Uuid> }
    let peek: Option<Peek> = sqlx::query_as(
        "SELECT user_id, target_profile_id FROM tomato_aliases WHERE status = 'pending' ORDER BY created_at LIMIT 1"
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("peek: {e}"))?;
    let Some(peek) = peek else { return Ok(0) };
    let user_id = peek.user_id;

    let selected = if let Some(target_pid) = peek.target_profile_id {
        match tomato_cookie::pick_online_for_profile(pool, target_pid).await? {
            Some(s) => s,
            None => return Ok(0), // target profile has no valid cookie yet
        }
    } else {
        match tomato_cookie::pick_random_online_for_user(pool, user_id).await? {
            Some(s) => s,
            None => return Ok(0),
        }
    };

    let tick = RENEWAL_TICK.fetch_add(1, Ordering::Relaxed);
    if tick % RENEWAL_CHECK_EVERY == 0 {
        handle_renewals(pool, abogus_url, &selected, user_id).await;
    }

    let pending: Vec<PendingRow> = sqlx::query_as::<_, PendingRow>(
        r#"SELECT id, book_id, alias_name, alias_type
           FROM tomato_aliases
           WHERE status = 'pending'
             AND user_id = $2
             AND (target_profile_id IS NULL OR target_profile_id = $3)
           ORDER BY created_at
           LIMIT $1"#,
    )
    .bind(BATCH_SIZE)
    .bind(user_id)
    .bind(selected.profile_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("select pending: {e}"))?;

    if pending.is_empty() {
        return Ok(0);
    }

    let http = build_http_client()?;
    let mut done = 0usize;

    'outer: for chunk in pending.chunks(CONCURRENCY) {
        let futs = chunk.iter().map(|row| {
            let pool = pool.clone();
            let http = http.clone();
            let selected = selected.clone();
            let abogus = abogus_url.to_string();
            let row = row.clone();
            async move { handle_row(&pool, &http, &abogus, &selected, &row).await }
        });
        let results = futures_util::future::join_all(futs).await;
        let mut cookie_dead = false;
        for outcome in results {
            match outcome {
                RowOutcome::Ok => done += 1,
                RowOutcome::CookieDead => cookie_dead = true,
            }
        }
        if cookie_dead {
            tracing::info!("alias_submitter: aborted round on auth failure");
            break 'outer;
        }
    }
    Ok(done)
}

enum RowOutcome {
    Ok,
    CookieDead,
}

async fn handle_row(
    pool: &DbPool,
    http: &reqwest_middleware::ClientWithMiddleware,
    abogus_url: &str,
    selected: &tomato_cookie::SelectedCookie,
    row: &PendingRow,
) -> RowOutcome {
    let outcome = submit_alias(
        http,
        abogus_url,
        &selected.cookie_header,
        &row.book_id,
        &row.alias_name,
        row.alias_type,
    )
    .await;
    let request_summary = json!({
        "book_id": row.book_id,
        "alias_name": row.alias_name,
        "alias_type": row.alias_type,
        "alias_row_id": row.id,
        "profile_id": selected.profile_id,
    });
    match outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_PROMOTION_PLAN_CREATE, request_summary)
        .await
    {
        Ok(alias_id) => {
            if let Err(e) = update_submitted(pool, row.id, &alias_id, selected.profile_id).await {
                tracing::warn!("alias_submitter: update_submitted: {e}");
            }
            tracing::info!(
                "alias_submitter: ok profile={} book={} type={} word={} alias_id={}",
                selected.profile_id,
                row.book_id,
                row.alias_type,
                row.alias_name,
                alias_id
            );
            RowOutcome::Ok
        }
        Err(err) if err.is_auth_failure() => {
            // Auth failure → cookie problem, not row problem. Leave row
            // pending so a later round picks a different cookie; mark
            // this cookie offline once and signal the chunk to abort.
            tomato_cookie::mark_offline(
                pool,
                selected.profile_id,
                &format!("alias_submit: {err}"),
            )
            .await
            .ok();
            tracing::warn!(
                "alias_submitter: cookie dead profile={} {err}, will retry batch with different cookie",
                selected.profile_id
            );
            RowOutcome::CookieDead
        }
        Err(crate::services::upstream_error::UpstreamError::ApiCode { code: 10004, .. }) => {
            // Platform internal error — transient, not the row's fault.
            // Leave as pending so the next round retries.
            tracing::warn!(
                "alias_submitter: platform internal error (10004) for word={} type={}, will retry",
                row.alias_name,
                row.alias_type
            );
            RowOutcome::Ok
        }
        Err(err) => {
            let reason = err.to_string();
            if let Err(e) = update_failed(pool, row.id, &reason, selected.profile_id).await {
                tracing::warn!("alias_submitter: update_failed: {e}");
            }
            tracing::warn!(
                "alias_submitter: fail profile={} book={} type={} word={} reason={}",
                selected.profile_id,
                row.book_id,
                row.alias_type,
                row.alias_name,
                reason
            );
            RowOutcome::Ok
        }
    }
}

/// In-place renewal pass for `submitted` aliases that have aged past
/// `ALIAS_RENEWAL_INTERVAL`. We never terminal-fail a renewal attempt:
/// the alias may still be valid on the platform side, in which case the
/// rejection ("你已申请此别名") is normal — we just defer the next try
/// by bumping `submitted_at`. A real success replaces alias_id and
/// resets the per-cycle backfill state so the row re-enters the post
/// pipeline with a fresh post.
async fn handle_renewals(
    pool: &DbPool,
    abogus_url: &str,
    selected: &tomato_cookie::SelectedCookie,
    user_id: i32,
) {
    let candidates: Vec<PendingRow> = match sqlx::query_as::<_, PendingRow>(
        r#"SELECT id, book_id, alias_name, alias_type
           FROM tomato_aliases
           WHERE status = 'submitted'
             AND user_id = $1
             AND submitted_at IS NOT NULL
             AND submitted_at < NOW() - INTERVAL '30 days'
           ORDER BY submitted_at
           LIMIT 10"#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("alias_submitter: renewal query failed: {e}");
            return;
        }
    };
    if candidates.is_empty() {
        return;
    }

    let http = match build_http_client() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("alias_submitter: renewal http client: {e}");
            return;
        }
    };

    for row in candidates {
        let outcome = submit_alias(
            &http,
            abogus_url,
            &selected.cookie_header,
            &row.book_id,
            &row.alias_name,
            row.alias_type,
        )
        .await;
        let request_summary = json!({
            "book_id": row.book_id,
            "alias_name": row.alias_name,
            "alias_type": row.alias_type,
            "alias_row_id": row.id,
            "profile_id": selected.profile_id,
            "renewal": true,
        });
        match outcome
            .audit(pool, SERVICE_NAME, ENDPOINT_PROMOTION_PLAN_CREATE, request_summary)
            .await
        {
            Ok(new_alias_id) => {
                // Real renewal: platform actually let us re-submit, so
                // the previous alias is dead and we have a brand-new
                // alias_id. Reset every per-cycle field so backfill
                // restarts from scratch. Preserve `submitted_by_profile_id`
                // (original credit, per spec) and `backfill_link_history`
                // (per-link 29-day TTL on the platform survives the
                // alias_id change).
                let res = sqlx::query(
                    r#"UPDATE tomato_aliases
                       SET alias_id=$1,
                           submitted_at=NOW(),
                           error_reason=NULL,
                           backfill_status='pending',
                           backfill_attempts=0,
                           backfill_post_link=NULL,
                           backfill_last_attempt_at=NULL,
                           backfill_error_reason=NULL,
                           backfilled_at=NULL,
                           backfilled_by_profile_id=NULL,
                           platform_status=NULL,
                           platform_audit_reason=NULL,
                           platform_status_checked_at=NULL,
                           submitted_by_profile_id=COALESCE(submitted_by_profile_id, $2)
                       WHERE id=$3"#,
                )
                .bind(&new_alias_id)
                .bind(selected.profile_id)
                .bind(row.id)
                .execute(pool)
                .await;
                if let Err(e) = res {
                    tracing::warn!("renewal update {}: {e}", row.id);
                    continue;
                }
                tracing::info!(
                    "alias_submitter: renewal ok alias={} type={} new_alias_id={} (was renewed by profile={})",
                    row.alias_name,
                    row.alias_type,
                    new_alias_id,
                    selected.profile_id
                );
            }
            Err(err) => {
                // Renewal rejected. Most common case: platform still
                // considers the alias active and rejects with
                // "你已申请此别名". Either way we don't want to keep
                // hammering — bump submitted_at to defer the next
                // renewal try by another `ALIAS_RENEWAL_INTERVAL`.
                let reason = err.to_string();
                let res = sqlx::query(
                    r#"UPDATE tomato_aliases
                       SET submitted_at=NOW()
                       WHERE id=$1"#,
                )
                .bind(row.id)
                .execute(pool)
                .await;
                if let Err(e) = res {
                    tracing::warn!("renewal defer {}: {e}", row.id);
                    continue;
                }
                tracing::info!(
                    "alias_submitter: renewal deferred alias={} type={} (reason: {}); next try after {}",
                    row.alias_name,
                    row.alias_type,
                    reason,
                    ALIAS_RENEWAL_INTERVAL
                );
            }
        }
    }
}

async fn update_submitted(
    pool: &DbPool,
    row_id: i64,
    alias_id: &str,
    profile_id: Uuid,
) -> Result<(), String> {
    // COALESCE preserves the ORIGINAL submitter's credit across renewal
    // cycles — the row is only "first submitted" once, even if a
    // different account does the 30-day renewal. Same for `update_failed`
    // below, so failure during renewal still attributes to whoever
    // first owned the row.
    sqlx::query(
        r#"UPDATE tomato_aliases
           SET status='submitted',
               alias_id=$1,
               submitted_at=NOW(),
               submitted_by_profile_id=COALESCE(submitted_by_profile_id, $2)
           WHERE id=$3"#,
    )
    .bind(alias_id)
    .bind(profile_id)
    .bind(row_id)
    .execute(pool)
    .await
    .map_err(|e| format!("update submitted {row_id}: {e}"))?;
    Ok(())
}

async fn update_failed(
    pool: &DbPool,
    row_id: i64,
    reason: &str,
    profile_id: Uuid,
) -> Result<(), String> {
    // Also mark backfill as failed — no alias_id means backfill can never run.
    sqlx::query(
        r#"UPDATE tomato_aliases
           SET status='failed',
               backfill_status='failed',
               backfill_error_reason='alias submission failed',
               error_reason=$1,
               submitted_at=NOW(),
               submitted_by_profile_id=COALESCE(submitted_by_profile_id, $2)
           WHERE id=$3"#,
    )
    .bind(reason)
    .bind(profile_id)
    .bind(row_id)
    .execute(pool)
    .await
    .map_err(|e| format!("update failed {row_id}: {e}"))?;
    Ok(())
}

#[derive(sqlx::FromRow, Clone)]
struct PendingRow {
    id: i64,
    book_id: String,
    alias_name: String,
    alias_type: i32,
}

//! Continuous worker that picks up `qimao_aliases` rows in
//! `status='pending'` state and submits them via the 七猫达人 promotion
//! API.
//!
//! Two-step submission (mirrors the legacy C# `QiMaoSubmitBrushSender`):
//!   1. `keyword_precheck` — platform-side validation. If the response's
//!      `reject_reason` is non-empty, the keyword is permanently
//!      rejected (e.g. blacklisted / similar-to-existing). Mark
//!      `status='failed'` with the reason.
//!   2. `add_keywords` — actual submission. The platform doesn't return
//!      an `alias_id` here; that comes later from `keyword_page`
//!      polling in `qimao_backfill_submitter`.
//!
//! Cookie source: `services::qimao_account::pick_random_active` —
//! same shape as `tomato_cookie::pick_random_online` but reads the
//! account's `qimao_token` (kept fresh by `qimao_token_refresh`).

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::qimao_account;
use crate::services::qimao_promotion::{
    add_keywords, build_http_client, keyword_precheck, ENDPOINT_ADD_KEYWORDS,
    ENDPOINT_KEYWORD_PRECHECK, SERVICE_NAME,
};

/// Poll cadence. Same 2s as tomato's alias_submitter.
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Per-tick batch ceiling. qimao only has one alias per word (no
/// fan-out) so we burn through the queue faster than tomato; cap at
/// the same 30 to keep one tick bounded.
const BATCH_SIZE: i64 = 30;

/// Rows processed concurrently within each chunk. Mirrors the value
/// used by backfill_submitter — enough to overlap network latency
/// without overwhelming the platform's per-IP rate limits.
const CONCURRENCY: usize = 2;

pub async fn start_worker(pool: Arc<DbPool>) {
    let p = pool.clone();
    crate::jobs::poller_loop("qimao_alias_submitter", POLL_INTERVAL, p, move || {
        let pool = pool.clone();
        async move { process_pending(&pool).await }
    })
    .await;
}

async fn process_pending(pool: &DbPool) -> Result<usize, String> {
    #[derive(sqlx::FromRow)]
    struct Peek { user_id: i32, target_profile_id: Option<uuid::Uuid> }
    let peek: Option<Peek> = sqlx::query_as(
        "SELECT user_id, target_profile_id FROM qimao_aliases WHERE status = 'pending' ORDER BY created_at LIMIT 1"
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("peek: {e}"))?;
    let Some(peek) = peek else { return Ok(0) };
    let user_id = peek.user_id;

    let selected = if let Some(target_pid) = peek.target_profile_id {
        match qimao_account::pick_active_for_profile(pool, target_pid).await? {
            Some(s) => s,
            None => return Ok(0),
        }
    } else {
        match qimao_account::pick_random_active_for_user(pool, user_id).await? {
            Some(s) => s,
            None => return Ok(0),
        }
    };

    let pending: Vec<PendingRow> = sqlx::query_as::<_, PendingRow>(
        r#"SELECT id, book_id, book_name, alias_name
           FROM qimao_aliases
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
            let row = row.clone();
            async move { handle_row(&pool, &http, &selected, &row).await }
        });
        let results = futures_util::future::join_all(futs).await;
        let mut token_dead = false;
        for outcome in results {
            match outcome {
                RowOutcome::Ok => done += 1,
                RowOutcome::TokenDead => token_dead = true,
            }
        }
        if token_dead {
            tracing::info!("qimao_alias_submitter: aborted round on auth failure");
            break 'outer;
        }
    }
    Ok(done)
}

enum RowOutcome {
    Ok,
    TokenDead,
}

async fn handle_row(
    pool: &DbPool,
    http: &reqwest_middleware::ClientWithMiddleware,
    selected: &qimao_account::SelectedAccount,
    row: &PendingRow,
) -> RowOutcome {
    // ─── Stage 1: precheck ─────────────────────────────────────────
    let precheck_outcome = keyword_precheck(http, &selected.token, &row.alias_name).await;
    let precheck_request = json!({
        "alias_row_id": row.id,
        "alias_name": row.alias_name,
        "profile_id": selected.profile_id,
    });
    let reject_reason = match precheck_outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_KEYWORD_PRECHECK, precheck_request)
        .await
    {
        Ok(r) => r,
        Err(err) if err.is_auth_failure() => {
            qimao_account::invalidate_token(
                pool,
                selected.profile_id,
                &format!("precheck: {err}"),
            )
            .await
            .ok();
            tracing::warn!(
                "qimao_alias_submitter: token dead profile={} on precheck: {err}",
                selected.profile_id
            );
            return RowOutcome::TokenDead;
        }
        Err(err) => {
            if let Err(e) =
                update_failed(pool, row.id, &format!("precheck error: {err}"), selected.profile_id)
                    .await
            {
                tracing::warn!("qimao_alias_submitter: update_failed: {e}");
            }
            tracing::warn!("qimao_alias_submitter: precheck error name={} {err}", row.alias_name);
            return RowOutcome::Ok;
        }
    };

    if !reject_reason.is_empty() {
        if let Err(e) = update_failed(
            pool,
            row.id,
            &format!("precheck rejected: {reject_reason}"),
            selected.profile_id,
        )
        .await
        {
            tracing::warn!("qimao_alias_submitter: update_failed: {e}");
        }
        tracing::warn!(
            "qimao_alias_submitter: precheck rejected name={} reason={}",
            row.alias_name,
            reject_reason
        );
        return RowOutcome::Ok;
    }

    // ─── Stage 2: add_keywords ─────────────────────────────────────
    let submit_outcome =
        add_keywords(http, &selected.token, row.book_id, &row.book_name, &row.alias_name).await;
    let submit_request = json!({
        "alias_row_id": row.id,
        "alias_name": row.alias_name,
        "book_id": row.book_id,
        "profile_id": selected.profile_id,
    });
    match submit_outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_ADD_KEYWORDS, submit_request)
        .await
    {
        Ok(()) => {
            if let Err(e) = update_submitted(pool, row.id, selected.profile_id).await {
                tracing::warn!("qimao_alias_submitter: update_submitted: {e}");
            }
            tracing::info!(
                "qimao_alias_submitter: ok profile={} book={} word={}",
                selected.profile_id,
                row.book_id,
                row.alias_name
            );
        }
        Err(err) if err.is_auth_failure() => {
            qimao_account::invalidate_token(
                pool,
                selected.profile_id,
                &format!("add_keywords: {err}"),
            )
            .await
            .ok();
            return RowOutcome::TokenDead;
        }
        Err(err) => {
            let reason = err.to_string();
            if let Err(e) = update_failed(pool, row.id, &reason, selected.profile_id).await {
                tracing::warn!("qimao_alias_submitter: update_failed: {e}");
            }
            tracing::warn!("qimao_alias_submitter: fail name={} reason={}", row.alias_name, reason);
        }
    }
    RowOutcome::Ok
}

async fn update_submitted(
    pool: &DbPool,
    row_id: i64,
    profile_id: Uuid,
) -> Result<(), String> {
    sqlx::query(
        r#"UPDATE qimao_aliases
           SET status='submitted',
               submitted_at=NOW(),
               submitted_by_profile_id=COALESCE(submitted_by_profile_id, $1)
           WHERE id=$2"#,
    )
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
    sqlx::query(
        r#"UPDATE qimao_aliases
           SET status='failed',
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
    book_id: i64,
    book_name: String,
    alias_name: String,
}

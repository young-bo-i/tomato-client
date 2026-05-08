//! qimao backfill worker.
//!
//! Two responsibilities collapsed into one worker since the platform's
//! API forces them together:
//!
//!   1. **Resolve `alias_id`**: qimao's `add_keywords` doesn't return
//!      the platform's keyword id. After the alias_submitter marks a
//!      row `submitted`, this worker polls `keyword_page` until the
//!      keyword shows up with `status_text_code` 2/4 (active) and
//!      stamps `alias_id`. Codes "1"=审核中 we wait; anything else
//!      we mark backfill_status='failed' (the keyword never made it
//!      onto the platform — terminal).
//!
//!   2. **Backfill the post link**: once `alias_id` is known, pick a
//!      Douyin link from `douyin_videos` (matching the alias_name's
//!      filtered title or suggest) that we haven't used for this row
//!      before, and POST it via `add_keyword_links`.
//!
//! Lifecycle gates:
//!   * 5-minute soak after `submitted_at` before the first
//!     keyword_page poll (the platform's review starts asynchronously).
//!   * 10-minute cooldown between rounds for a given row, like tomato.
//!   * 30-day age cap (matches C# `QiMaoWriteBackJob.ThrowDay`): if a
//!     row is still `pending` 30 days after `created_at`, mark
//!     `failed` and stop polling.
//!   * 5 post/create attempts before giving up on backfill.
//!
//! Token + auth: same shape as `qimao_alias_submitter`. On 401/403 we
//! invalidate the profile's token and abort the round.

use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use serde_json::{json, Value as JsonValue};
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::qimao_account;
use crate::services::qimao_promotion::{
    add_keyword_links, build_http_client, is_active_status, keyword_page, QimaoKeywordItem,
    ENDPOINT_ADD_KEYWORD_LINKS, ENDPOINT_KEYWORD_PAGE, QIMAO_STATUS_REVIEWING, SERVICE_NAME,
};

const POLL_INTERVAL: Duration = Duration::from_secs(30);
const BATCH_SIZE: i64 = 30;
const CONCURRENCY: usize = 2;
const MAX_BACKFILL_ATTEMPTS: i32 = 5;
/// 30-day age cap for backfill (mirror of `QiMaoWriteBackJob.ThrowDay`).
const EXPIRATION_INTERVAL: &str = "30 days";

pub async fn start_worker(pool: Arc<DbPool>) {
    let p = pool.clone();
    crate::jobs::poller_loop("qimao_backfill_submitter", POLL_INTERVAL, p, move || {
        let pool = pool.clone();
        async move { process_pending(&pool).await }
    })
    .await;
}

async fn process_pending(pool: &DbPool) -> Result<usize, String> {
    let user_id: Option<i32> = sqlx::query_scalar(
        r#"SELECT user_id FROM qimao_aliases
           WHERE status = 'submitted' AND backfill_status = 'pending'
             AND submitted_at IS NOT NULL
             AND submitted_at < NOW() - INTERVAL '5 minutes'
             AND (backfill_last_attempt_at IS NULL
                  OR backfill_last_attempt_at < NOW() - INTERVAL '10 minutes')
           ORDER BY submitted_at LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("peek user_id: {e}"))?;
    let Some(user_id) = user_id else { return Ok(0) };

    let selected = match qimao_account::pick_random_active_for_user(pool, user_id).await? {
        Some(s) => s,
        None => return Ok(0),
    };

    let expired = sqlx::query(
        r#"UPDATE qimao_aliases
           SET backfill_status='failed',
               backfill_error_reason='exceeded 30 days age cap',
               backfill_last_attempt_at=NOW()
           WHERE status='submitted'
             AND backfill_status='pending'
             AND user_id = $1
             AND created_at < NOW() - INTERVAL '30 days'"#,
    )
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| format!("expire query: {e}"))?;
    if expired.rows_affected() > 0 {
        tracing::info!(
            "qimao_backfill_submitter: expired {} stale row(s) (>{} old)",
            expired.rows_affected(),
            EXPIRATION_INTERVAL
        );
    }

    let pending: Vec<PendingRow> = sqlx::query_as::<_, PendingRow>(
        r#"SELECT id, alias_id, alias_name, backfill_attempts, backfill_link_history
           FROM qimao_aliases
           WHERE status = 'submitted'
             AND backfill_status = 'pending'
             AND user_id = $2
             AND submitted_at IS NOT NULL
             AND submitted_at < NOW() - INTERVAL '5 minutes'
             AND (backfill_last_attempt_at IS NULL
                  OR backfill_last_attempt_at < NOW() - INTERVAL '10 minutes')
           ORDER BY submitted_at
           LIMIT $1"#,
    )
    .bind(BATCH_SIZE)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("select pending: {e}"))?;

    if pending.is_empty() {
        return Ok(0);
    }

    let http = build_http_client()?;
    let post_date = Local::now().format("%Y-%m-%d").to_string();
    let lookback_start = Local::now()
        .checked_sub_signed(chrono::Duration::days(35))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| post_date.clone());
    let lookback_end = Local::now()
        .checked_add_signed(chrono::Duration::days(1))
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| post_date.clone());

    let mut done = 0usize;

    'outer: for chunk in pending.chunks(CONCURRENCY) {
        let futs = chunk.iter().map(|row| {
            let pool = pool.clone();
            let http = http.clone();
            let selected = selected.clone();
            let lookback_start = lookback_start.clone();
            let lookback_end = lookback_end.clone();
            let row = row.clone();
            async move {
                handle_row(&pool, &http, &selected, &row, &lookback_start, &lookback_end).await
            }
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
    lookback_start: &str,
    lookback_end: &str,
) -> RowOutcome {
    // ─── Stage A: resolve alias_id if not yet known ────────────────
    let alias_id = match row.alias_id {
        Some(id) => id,
        None => {
            // Need to ask keyword_page where we are. The query
            // returns a list; we filter to exact match on alias_name
            // (the upstream's keyword= param is a contains-style
            // search, not exact).
            let outcome = keyword_page(
                http,
                &selected.token,
                &row.alias_name,
                lookback_start,
                lookback_end,
            )
            .await;
            let request_summary = json!({
                "alias_row_id": row.id,
                "alias_name": row.alias_name,
                "profile_id": selected.profile_id,
            });
            let items = match outcome
                .audit(pool, SERVICE_NAME, ENDPOINT_KEYWORD_PAGE, request_summary)
                .await
            {
                Ok(items) => items,
                Err(err) if err.is_auth_failure() => {
                    qimao_account::invalidate_token(
                        pool,
                        selected.profile_id,
                        &format!("keyword_page: {err}"),
                    )
                    .await
                    .ok();
                    return RowOutcome::TokenDead;
                }
                Err(err) => {
                    bump_cooldown_only(pool, row.id).await;
                    tracing::warn!(
                        "qimao_backfill_submitter: keyword_page failed name={} {err}",
                        row.alias_name
                    );
                    return RowOutcome::Ok;
                }
            };

            // Pick the matching item with the most-favorable status:
            // active (2/4) wins over reviewing (1) wins over anything
            // else. The upstream may return multiple rows per keyword
            // when the same word was submitted multiple times.
            let exact: Vec<&QimaoKeywordItem> = items
                .iter()
                .filter(|i| i.search_keyword == row.alias_name)
                .collect();

            if let Some(active) =
                exact.iter().find(|i| is_active_status(&i.status_text_code))
            {
                persist_alias_id(pool, row.id, (*active).clone()).await;
                tracing::info!(
                    "qimao_backfill_submitter: resolved alias_id={} for word={}",
                    active.id,
                    row.alias_name
                );
                active.id
            } else if exact
                .iter()
                .any(|i| i.status_text_code == QIMAO_STATUS_REVIEWING)
            {
                // Still in review — record platform_status_code, bump
                // cooldown, recheck next round.
                let reviewing = exact
                    .iter()
                    .find(|i| i.status_text_code == QIMAO_STATUS_REVIEWING)
                    .copied()
                    .unwrap();
                persist_platform_status(pool, row.id, reviewing.clone()).await;
                bump_cooldown_only(pool, row.id).await;
                tracing::info!(
                    "qimao_backfill_submitter: alias={} still in review, will recheck",
                    row.alias_name
                );
                return RowOutcome::Ok;
            } else if exact.is_empty() {
                // Not on the platform yet (or never made it). Bump
                // cooldown and retry — the alias_submitter may have
                // queued it but the platform hasn't fully surfaced it
                // yet.
                bump_cooldown_only(pool, row.id).await;
                tracing::info!(
                    "qimao_backfill_submitter: alias={} not yet on platform, will recheck",
                    row.alias_name
                );
                return RowOutcome::Ok;
            } else {
                // Match exists but status is non-active and non-reviewing:
                // terminal. Captures the platform's reject_reason if any.
                let invalid = exact[0].clone();
                let reason = format!(
                    "platform status_text_code={} ({}); reject_reason={}",
                    invalid.status_text_code,
                    invalid.status_text,
                    if invalid.reject_reason.is_empty() {
                        "(none)"
                    } else {
                        &invalid.reject_reason
                    }
                );
                mark_terminal(pool, row.id, invalid, &reason).await;
                tracing::info!(
                    "qimao_backfill_submitter: alias={} terminal status={} reason={}",
                    row.alias_name,
                    exact[0].status_text_code,
                    reason
                );
                return RowOutcome::Ok;
            }
        }
    };

    // ─── Stage B: pick a fresh link + add_keyword_links ────────────
    let history = row.link_history_strings();
    let link = match pick_random_link(pool, &row.alias_name, &history).await {
        Ok(Some(l)) => l,
        Ok(None) if history.is_empty() => {
            sqlx::query(
                r#"UPDATE qimao_aliases
                   SET backfill_status='failed',
                       backfill_error_reason='no source link in douyin_videos',
                       backfill_last_attempt_at=NOW()
                   WHERE id=$1"#,
            )
            .bind(row.id)
            .execute(pool)
            .await
            .ok();
            tracing::warn!(
                "qimao_backfill_submitter: no source link for word={}",
                row.alias_name
            );
            return RowOutcome::Ok;
        }
        Ok(None) => {
            // history non-empty + no fresh link available → wait for
            // a new video to arrive instead of re-submitting a used
            // link. Same shape as tomato's renewal-exhausted path.
            bump_cooldown_only(pool, row.id).await;
            tracing::info!(
                "qimao_backfill_submitter: alias={} all {} link(s) used, awaiting fresh video",
                row.alias_name,
                history.len()
            );
            return RowOutcome::Ok;
        }
        Err(e) => {
            tracing::warn!("qimao_backfill_submitter: pick_random_link: {e}");
            return RowOutcome::Ok;
        }
    };

    let outcome =
        add_keyword_links(http, &selected.token, alias_id, &row.alias_name, &link).await;
    let request_summary = json!({
        "alias_row_id": row.id,
        "alias_id": alias_id,
        "alias_name": row.alias_name,
        "post_link": link,
        "attempt": row.backfill_attempts + 1,
        "profile_id": selected.profile_id,
    });
    match outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_ADD_KEYWORD_LINKS, request_summary)
        .await
    {
        Ok(()) => {
            update_backfill_ok(pool, row.id, &link, selected.profile_id).await;
            tracing::info!(
                "qimao_backfill_submitter: ok alias_id={} word={} link={}",
                alias_id,
                row.alias_name,
                link
            );
            RowOutcome::Ok
        }
        Err(err) if err.is_auth_failure() => {
            qimao_account::invalidate_token(
                pool,
                selected.profile_id,
                &format!("add_keyword_links: {err}"),
            )
            .await
            .ok();
            RowOutcome::TokenDead
        }
        Err(err) => {
            let reason = err.to_string();
            update_backfill_fail(
                pool,
                row.id,
                row.backfill_attempts + 1,
                &link,
                &reason,
                selected.profile_id,
            )
            .await;
            tracing::warn!(
                "qimao_backfill_submitter: fail alias_id={} word={} attempt={}/{} reason={}",
                alias_id,
                row.alias_name,
                row.backfill_attempts + 1,
                MAX_BACKFILL_ATTEMPTS,
                reason
            );
            RowOutcome::Ok
        }
    }
}

// ───────────────────────── DB helpers ─────────────────────────────────

async fn persist_alias_id(pool: &DbPool, row_id: i64, item: QimaoKeywordItem) {
    if let Err(e) = sqlx::query(
        r#"UPDATE qimao_aliases
           SET alias_id=$1,
               platform_status_code=$2,
               platform_reject_reason=$3,
               platform_status_checked_at=NOW()
           WHERE id=$4"#,
    )
    .bind(item.id)
    .bind(&item.status_text_code)
    .bind(&item.reject_reason)
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("persist_alias_id {row_id}: {e}");
    }
}

async fn persist_platform_status(pool: &DbPool, row_id: i64, item: QimaoKeywordItem) {
    if let Err(e) = sqlx::query(
        r#"UPDATE qimao_aliases
           SET platform_status_code=$1,
               platform_reject_reason=$2,
               platform_status_checked_at=NOW()
           WHERE id=$3"#,
    )
    .bind(&item.status_text_code)
    .bind(&item.reject_reason)
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("persist_platform_status {row_id}: {e}");
    }
}

async fn mark_terminal(
    pool: &DbPool,
    row_id: i64,
    item: QimaoKeywordItem,
    reason: &str,
) {
    if let Err(e) = sqlx::query(
        r#"UPDATE qimao_aliases
           SET backfill_status='failed',
               backfill_error_reason=$1,
               backfill_last_attempt_at=NOW(),
               platform_status_code=$2,
               platform_reject_reason=$3,
               platform_status_checked_at=NOW()
           WHERE id=$4"#,
    )
    .bind(reason)
    .bind(&item.status_text_code)
    .bind(&item.reject_reason)
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("mark_terminal {row_id}: {e}");
    }
}

async fn bump_cooldown_only(pool: &DbPool, row_id: i64) {
    if let Err(e) = sqlx::query(
        r#"UPDATE qimao_aliases
           SET backfill_last_attempt_at=NOW()
           WHERE id=$1"#,
    )
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("bump_cooldown_only {row_id}: {e}");
    }
}

async fn update_backfill_ok(pool: &DbPool, row_id: i64, link: &str, profile_id: Uuid) {
    if let Err(e) = sqlx::query(
        r#"UPDATE qimao_aliases
           SET backfill_status='submitted',
               backfilled_at=NOW(),
               backfill_last_attempt_at=NOW(),
               backfill_post_link=$1,
               backfill_link_history=backfill_link_history || to_jsonb($1::text),
               backfill_attempts=backfill_attempts + 1,
               backfill_error_reason=NULL,
               backfilled_by_profile_id=$2
           WHERE id=$3"#,
    )
    .bind(link)
    .bind(profile_id)
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("update_backfill_ok {row_id}: {e}");
    }
}

async fn update_backfill_fail(
    pool: &DbPool,
    row_id: i64,
    next_attempts: i32,
    link: &str,
    reason: &str,
    profile_id: Uuid,
) {
    let terminal = next_attempts >= MAX_BACKFILL_ATTEMPTS;
    let next_status = if terminal { "failed" } else { "pending" };
    if let Err(e) = sqlx::query(
        r#"UPDATE qimao_aliases
           SET backfill_status=$1,
               backfill_attempts=$2,
               backfill_last_attempt_at=NOW(),
               backfill_post_link=$3,
               backfill_error_reason=$4,
               backfilled_by_profile_id=$5
           WHERE id=$6"#,
    )
    .bind(next_status)
    .bind(next_attempts)
    .bind(link)
    .bind(reason)
    .bind(profile_id)
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("update_backfill_fail {row_id}: {e}");
    }
}

async fn pick_random_link(
    pool: &DbPool,
    word: &str,
    exclude_history: &[String],
) -> Result<Option<String>, String> {
    // Fetch up to 50 candidates from each partial index branch without
    // ORDER BY, then pick randomly in Rust. Avoids the O(n log n) sort
    // that `ORDER BY random() LIMIT 1` requires on large result sets.
    let candidates: Vec<String> = sqlx::query_scalar(
        r#"SELECT share_url FROM (
              SELECT share_url FROM douyin_videos
              WHERE title_filtered = $1
                AND share_url IS NOT NULL AND share_url <> ''
                AND NOT (share_url = ANY($2::text[]))
              UNION ALL
              SELECT share_url FROM douyin_videos
              WHERE suggest_word_filtered = $1
                AND share_url IS NOT NULL AND share_url <> ''
                AND NOT (share_url = ANY($2::text[]))
           ) AS s
           LIMIT 50"#,
    )
    .bind(word)
    .bind(exclude_history)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("pick link: {e}"))?;

    use rand::seq::SliceRandom;
    Ok(candidates.choose(&mut rand::thread_rng()).cloned())
}

#[derive(sqlx::FromRow, Clone)]
struct PendingRow {
    id: i64,
    alias_id: Option<i64>,
    alias_name: String,
    backfill_attempts: i32,
    backfill_link_history: JsonValue,
}

impl PendingRow {
    fn link_history_strings(&self) -> Vec<String> {
        self.backfill_link_history
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    }
}

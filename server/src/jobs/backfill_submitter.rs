//! Continuous worker that picks up `tomato_aliases` rows where the
//! alias-create call already succeeded (`status='submitted'`) and the
//! second-leg "post" submission is still pending
//! (`backfill_status='pending'`).
//!
//! Per-row flow each round:
//!   1. Look up the alias's current platform-side review status via
//!      `promotion/plan/list`. The platform UI exposes 6 states; we map
//!      them to actions:
//!        - 1 生效中 / 6 即将失效 → proceed with post/create
//!        - 3 待审核                → not eligible yet; record status,
//!                                     bump cooldown, do NOT bump
//!                                     attempts (failure isn't ours)
//!        - 2 已失效 / 4 审核不通过 / 5 强制失效
//!                                  → terminal; mark backfill_status
//!                                     'failed' with the platform's
//!                                     audit_reason
//!   2. For the proceeding cases, pick a random Douyin link that
//!      produced this filtered word and POST to /promotion/post/create.
//!      Retry budget is 5; after that we mark 'failed' to avoid
//!      burning the upstream's quota on a hopeless alias.
//!
//! Why query status first instead of just calling post/create:
//!   - We were burning the 5-attempt budget on rows that the platform
//!     had already permanently rejected (alias_status=4). Querying
//!     status lets us distinguish "still in review (will become OK)"
//!     from "rejected (never will)".
//!   - It also lets the dashboard show meaningful per-row status
//!     instead of just "pending → maybe → submitted".
//!
//! Cookie source: shared `services::tomato_cookie` helper — random
//! pick of an online admin cookie. On HTTP 401/403 from any of the
//! upstream calls the worker marks that cookie offline and aborts
//! the round; no attempt counter bump (the failure isn't the row's
//! fault), so the row gets a fair retry next round.

use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use serde_json::{json, Value as JsonValue};
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::fanqie_promotion::{
    alias_status_label, build_http_client, query_alias_status, submit_post, AliasStatusInfo,
    ALIAS_STATUS_ACTIVE, ALIAS_STATUS_EXPIRED, ALIAS_STATUS_EXPIRING, ALIAS_STATUS_FORCE_INVALID,
    ALIAS_STATUS_PENDING_REVIEW, ALIAS_STATUS_REJECTED, ENDPOINT_PROMOTION_PLAN_LIST,
    ENDPOINT_PROMOTION_POST_CREATE, SERVICE_NAME,
};
use crate::services::tomato_cookie;

/// Slower than alias_submitter — backfill is on a 10-minute cooldown
/// anyway, so a 30s tick is plenty responsive while keeping the DB
/// scan rate negligible.
const POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Per-tick batch ceiling. Sized to drain a normal day's accumulated
/// aliases (a few hundred over 10-minute cooldowns) without blocking
/// the worker on a single round.
const BATCH_SIZE: i64 = 30;
const CONCURRENCY: usize = 2;

/// Strikeouts before we give up on a row in the post/create stage. The
/// status-check stage doesn't consume from this budget — only actual
/// post/create attempts do.
const MAX_ATTEMPTS: i32 = 5;

/// Posted Douyin links go stale on the platform after ~29 days. After
/// that the alias still exists but the link no longer counts toward
/// the alias's productivity. We re-enter the backfill flow with a
/// fresh link from `douyin_videos`.
///
/// Renewal link selection (distinct from initial backfill):
///   1. If the row has a previous failed link (`backfill_post_link`),
///      reuse it as long as it was captured within RENEWAL_LINK_WINDOW.
///      Rationale: the failure was likely transient; retrying the same
///      link avoids burning a fresh one unnecessarily.
///   2. Otherwise pick a random link from `douyin_videos` captured
///      within RENEWAL_LINK_WINDOW (prefer recent captures over stale
///      ones that may have already expired on Douyin's side).
const RENEWAL_INTERVAL: &str = "29 days";

/// How recently a Douyin link must have been captured to qualify for
/// renewal backfill. Fresh links are far less likely to be 404/expired
/// on Douyin's side than ones captured months ago.
const RENEWAL_LINK_WINDOW: &str = "24 hours";

pub async fn start_worker(pool: Arc<DbPool>, abogus_url: Arc<String>) {
    let p = pool.clone();
    crate::jobs::poller_loop("backfill_submitter", POLL_INTERVAL, p, move || {
        let pool = pool.clone();
        let abogus_url = abogus_url.clone();
        async move { process_pending(&pool, &abogus_url).await }
    })
    .await;
}

async fn process_pending(pool: &DbPool, abogus_url: &str) -> Result<usize, String> {
    let user_id: Option<i32> = sqlx::query_scalar(
        r#"SELECT user_id FROM tomato_aliases
           WHERE status = 'submitted' AND backfill_status = 'pending'
             AND alias_id IS NOT NULL
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

    let selected = match tomato_cookie::pick_random_online_for_user(pool, user_id).await? {
        Some(s) => s,
        None => return Ok(0),
    };

    let promoted = sqlx::query(
        r#"UPDATE tomato_aliases
           SET backfill_status='pending',
               backfill_attempts=0,
               backfill_last_attempt_at=NULL,
               backfill_error_reason=NULL
           WHERE status='submitted'
             AND user_id = $1
             AND backfill_status='submitted'
             AND backfilled_at IS NOT NULL
             AND backfilled_at < NOW() - INTERVAL '29 days'"#,
    )
    .bind(user_id)
    .execute(pool)
    .await
    .map_err(|e| format!("promote expired: {e}"))?;
    if promoted.rows_affected() > 0 {
        tracing::info!(
            "backfill_submitter: promoted {} stale row(s) for renewal (>{} since backfill)",
            promoted.rows_affected(),
            RENEWAL_INTERVAL
        );
    }

    // Eligibility:
    //   * alias-create already succeeded (status='submitted', alias_id present)
    //   * backfill not yet terminal
    //   * 5-minute soak after submit so the platform's review has at
    //     least had a chance to start
    //   * 10-minute cooldown between rounds — applies to both status
    //     checks and post/create attempts uniformly
    let pending: Vec<PendingRow> = sqlx::query_as::<_, PendingRow>(
        r#"SELECT id, alias_id, alias_name, alias_type, backfill_attempts,
                  backfill_link_history, backfill_post_link, backfilled_at
           FROM tomato_aliases
           WHERE status = 'submitted'
             AND backfill_status = 'pending'
             AND user_id = $2
             AND alias_id IS NOT NULL
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
    let mut done = 0usize;

    // Process CONCURRENCY rows at a time. Each chunk runs as concurrent
    // futures in the same task (cooperative I/O parallelism — no extra
    // threads). If any row in a chunk returns CookieDead we stop after
    // the chunk completes so partial results are still committed.
    'outer: for chunk in pending.chunks(CONCURRENCY) {
        let futs = chunk.iter().map(|row| {
            let pool = pool.clone();
            let http = http.clone();
            let selected = selected.clone();
            let abogus = abogus_url.to_string();
            let post_date = post_date.clone();
            let row = row.clone();
            async move { handle_row(&pool, &http, &abogus, &selected, &row, &post_date).await }
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
            break 'outer;
        }
    }
    Ok(done)
}

/// Per-row outcome that controls the loop. We don't propagate Err out
/// of handle_row because each row's failure is recorded in the DB; the
/// loop only needs to know "is this cookie still usable".
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
    post_date: &str,
) -> RowOutcome {
    // ─── Stage 1: refresh platform status ─────────────────────────────
    let outcome = query_alias_status(
        http,
        abogus_url,
        &selected.cookie_header,
        &row.alias_name,
        row.alias_type,
    )
    .await;
    let request_summary = json!({
        "alias_row_id": row.id,
        "alias_name": row.alias_name,
        "alias_type": row.alias_type,
        "profile_id": selected.profile_id,
    });
    let info_opt = match outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_PROMOTION_PLAN_LIST, request_summary)
        .await
    {
        Ok(info) => info,
        Err(err) if err.is_auth_failure() => {
            tomato_cookie::mark_offline(
                pool,
                selected.profile_id,
                &format!("plan_list: {err}"),
            )
            .await
            .ok();
            tracing::warn!(
                "backfill_submitter: cookie dead profile={} on status query: {err}",
                selected.profile_id
            );
            return RowOutcome::CookieDead;
        }
        Err(err) => {
            // Non-auth status-query failure: bump cooldown anchor only
            // so we don't hammer the upstream. No attempts bump (we
            // never tried post/create this round).
            bump_cooldown_only(pool, row.id).await;
            tracing::warn!(
                "backfill_submitter: status query failed alias={} {err}, will retry",
                row.alias_name
            );
            return RowOutcome::Ok;
        }
    };

    let info = match info_opt {
        Some(i) => i,
        None => {
            // Alias not found by the upstream's filter. Treat as
            // terminal — there's no meaningful retry path. (Edge case;
            // shouldn't happen in normal flow.)
            mark_terminal(
                pool,
                row.id,
                None,
                &Vec::<String>::new(),
                "alias not found via plan/list",
            )
            .await;
            tracing::warn!(
                "backfill_submitter: alias not found on platform alias={} type={}",
                row.alias_name,
                row.alias_type
            );
            return RowOutcome::Ok;
        }
    };

    // ─── Stage 2: branch on platform status ───────────────────────────
    match info.alias_status {
        ALIAS_STATUS_ACTIVE | ALIAS_STATUS_EXPIRING => {
            // Eligible — push the link.
            persist_platform_status(pool, row.id, &info).await;
            do_post_create(pool, http, abogus_url, selected, row, post_date).await
        }

        ALIAS_STATUS_PENDING_REVIEW => {
            // Still under review. Record status; cooldown until the
            // next round; don't burn an attempt.
            persist_platform_status(pool, row.id, &info).await;
            bump_cooldown_only(pool, row.id).await;
            tracing::info!(
                "backfill_submitter: alias={} type={} still in review, will recheck",
                row.alias_name,
                row.alias_type
            );
            RowOutcome::Ok
        }

        s @ (ALIAS_STATUS_EXPIRED | ALIAS_STATUS_REJECTED | ALIAS_STATUS_FORCE_INVALID) => {
            // Terminal — no point ever retrying.
            let label = alias_status_label(s);
            let reasons = info.audit_reasons();
            let reason = if reasons.is_empty() {
                format!("platform status {s} ({label})")
            } else {
                format!("platform status {s} ({label}); audit_reason={reasons:?}")
            };
            mark_terminal(pool, row.id, Some(s), reasons, &reason).await;
            tracing::info!(
                "backfill_submitter: alias={} type={} terminal status={} ({}) reason={:?}",
                row.alias_name,
                row.alias_type,
                s,
                label,
                reasons
            );
            RowOutcome::Ok
        }

        other => {
            // Unknown status — store it for visibility, throttle, and
            // wait for a human to investigate.
            persist_platform_status(pool, row.id, &info).await;
            bump_cooldown_only(pool, row.id).await;
            tracing::warn!(
                "backfill_submitter: alias={} type={} unknown platform status={}",
                row.alias_name,
                row.alias_type,
                other
            );
            RowOutcome::Ok
        }
    }
}

/// Run the post/create call against an alias we've confirmed is
/// `ALIAS_STATUS_ACTIVE` or `ALIAS_STATUS_EXPIRING`.
async fn do_post_create(
    pool: &DbPool,
    http: &reqwest_middleware::ClientWithMiddleware,
    abogus_url: &str,
    selected: &tomato_cookie::SelectedCookie,
    row: &PendingRow,
    post_date: &str,
) -> RowOutcome {
    let history = row.link_history_strings();

    let link = if row.is_renewal() {
        // 29-day renewal: only use links captured within RENEWAL_LINK_WINDOW.
        // Priority: retry the previous failed link first (if it's still fresh),
        // then fall back to any fresh link. Avoids burning a new link when the
        // prior failure was transient (network, rate limit).
        match pick_renewal_link(pool, &row.alias_name, &history, row.backfill_post_link.as_deref()).await {
            Ok(Some(l)) => l,
            Ok(None) => {
                // No recent link available yet. Wait for fresh captures.
                bump_cooldown_only(pool, row.id).await;
                tracing::info!(
                    "backfill_submitter: renewal waiting alias={} type={} no link captured within {RENEWAL_LINK_WINDOW}",
                    row.alias_name,
                    row.alias_type,
                );
                return RowOutcome::Ok;
            }
            Err(e) => {
                tracing::warn!("backfill_submitter: pick_renewal_link failed: {e}");
                return RowOutcome::Ok;
            }
        }
    } else {
        // Initial backfill: any available link is fine.
        match pick_random_link(pool, &row.alias_name, &history).await {
            Ok(Some(l)) => l,
            Ok(None) if history.is_empty() => {
                // No link for this word at all — terminal.
                sqlx::query(
                    r#"UPDATE tomato_aliases
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
                    "backfill_submitter: no source link for word={} alias_id={}",
                    row.alias_name,
                    row.alias_id
                );
                return RowOutcome::Ok;
            }
            Ok(None) => {
                bump_cooldown_only(pool, row.id).await;
                tracing::info!(
                    "backfill_submitter: initial backfill waiting alias={} type={} all {} link(s) used",
                    row.alias_name,
                    row.alias_type,
                    history.len()
                );
                return RowOutcome::Ok;
            }
            Err(e) => {
                tracing::warn!("backfill_submitter: pick_random_link failed: {e}");
                return RowOutcome::Ok;
            }
        }
    };

    let outcome = submit_post(
        http,
        abogus_url,
        &selected.cookie_header,
        &row.alias_id,
        row.alias_type,
        &link,
        post_date,
    )
    .await;
    let request_summary = json!({
        "alias_row_id": row.id,
        "alias_id": row.alias_id,
        "alias_type": row.alias_type,
        "alias_name": row.alias_name,
        "post_link": link,
        "post_date": post_date,
        "attempt": row.backfill_attempts + 1,
        "profile_id": selected.profile_id,
    });
    match outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_PROMOTION_POST_CREATE, request_summary)
        .await
    {
        Ok(()) => {
            update_backfill_ok(pool, row.id, &link, selected.profile_id).await;
            tracing::info!(
                "backfill_submitter: ok profile={} alias_id={} word={} link={}",
                selected.profile_id,
                row.alias_id,
                row.alias_name,
                link
            );
            RowOutcome::Ok
        }
        Err(err) if err.is_auth_failure() => {
            tomato_cookie::mark_offline(
                pool,
                selected.profile_id,
                &format!("backfill_post: {err}"),
            )
            .await
            .ok();
            tracing::warn!(
                "backfill_submitter: cookie dead profile={} on post_create: {err}",
                selected.profile_id
            );
            RowOutcome::CookieDead
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
                "backfill_submitter: fail profile={} alias_id={} word={} attempt={}/{} reason={}",
                selected.profile_id,
                row.alias_id,
                row.alias_name,
                row.backfill_attempts + 1,
                MAX_ATTEMPTS,
                reason
            );
            RowOutcome::Ok
        }
    }
}

// ───────────────────────── DB helpers ─────────────────────────────────

async fn persist_platform_status(pool: &DbPool, row_id: i64, info: &AliasStatusInfo) {
    let audit_json = serde_json::to_value(&info.audit_reason).unwrap_or(JsonValue::Null);
    if let Err(e) = sqlx::query(
        r#"UPDATE tomato_aliases
           SET platform_status=$1,
               platform_audit_reason=$2,
               platform_status_checked_at=NOW()
           WHERE id=$3"#,
    )
    .bind(info.alias_status)
    .bind(&audit_json)
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("persist_platform_status {row_id}: {e}");
    }
}

async fn bump_cooldown_only(pool: &DbPool, row_id: i64) {
    if let Err(e) = sqlx::query(
        r#"UPDATE tomato_aliases
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

/// Mark a row terminally failed because of platform-side state (status
/// 2/4/5 or alias-not-found). Captures audit_reason + status into the
/// row in one shot so the dashboard has full context.
async fn mark_terminal(
    pool: &DbPool,
    row_id: i64,
    platform_status: Option<i32>,
    audit_reason: &[String],
    error_reason: &str,
) {
    let audit_json = serde_json::to_value(audit_reason).unwrap_or(JsonValue::Null);
    if let Err(e) = sqlx::query(
        r#"UPDATE tomato_aliases
           SET backfill_status='failed',
               backfill_error_reason=$1,
               backfill_last_attempt_at=NOW(),
               platform_status=$2,
               platform_audit_reason=$3,
               platform_status_checked_at=NOW()
           WHERE id=$4"#,
    )
    .bind(error_reason)
    .bind(platform_status)
    .bind(&audit_json)
    .bind(row_id)
    .execute(pool)
    .await
    {
        tracing::warn!("mark_terminal {row_id}: {e}");
    }
}

async fn update_backfill_ok(pool: &DbPool, row_id: i64, link: &str, profile_id: Uuid) {
    // Append the link to history so the next 29-day renewal cycle
    // prefers a different one. `||` on JSONB concatenates arrays;
    // `to_jsonb()` wraps the string. Idempotent re-appends are fine
    // (we just lose preference uniqueness, not correctness).
    if let Err(e) = sqlx::query(
        r#"UPDATE tomato_aliases
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
    let terminal = next_attempts >= MAX_ATTEMPTS;
    let next_status = if terminal { "failed" } else { "pending" };
    if let Err(e) = sqlx::query(
        r#"UPDATE tomato_aliases
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

/// Pick any `share_url` from `douyin_videos` matching `word` and not
/// in `exclude_history`. Used for the initial backfill path.
///
/// Fetches up to 50 candidates without ORDER BY (index-scan order) and
/// picks randomly in Rust. Avoids the O(n log n) full sort that
/// `ORDER BY random() LIMIT 1` requires when the matching set is large.
async fn pick_random_link(
    pool: &DbPool,
    word: &str,
    exclude_history: &[String],
) -> Result<Option<String>, String> {
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

/// Pick a link for the 29-day renewal path.
///
/// Priority:
///   1. `prefer_link` (the previous failed link) — reuse it if it was
///      captured within RENEWAL_LINK_WINDOW and is not already in
///      `exclude_history` (successfully submitted links). Avoids
///      wasting a fresh link when the prior failure was transient.
///   2. Any random link captured within RENEWAL_LINK_WINDOW that is
///      not in `exclude_history`.
///
/// Returns `Ok(None)` when no link meeting the freshness window exists.
async fn pick_renewal_link(
    pool: &DbPool,
    word: &str,
    exclude_history: &[String],
    prefer_link: Option<&str>,
) -> Result<Option<String>, String> {
    // Step 1: try the preferred (previously failed) link if fresh.
    if let Some(prev) = prefer_link {
        if !prev.is_empty() && !exclude_history.contains(&prev.to_string()) {
            let exists: bool = sqlx::query_scalar(
                r#"SELECT EXISTS (
                      SELECT 1 FROM douyin_videos
                      WHERE share_url = $1
                        AND inserted_at > NOW() - INTERVAL '24 hours'
                   )"#,
            )
            .bind(prev)
            .fetch_one(pool)
            .await
            .map_err(|e| format!("check prefer link: {e}"))?;
            if exists {
                return Ok(Some(prev.to_string()));
            }
        }
    }

    // Step 2: any random fresh link not in history.
    let candidates: Vec<String> = sqlx::query_scalar(
        r#"SELECT share_url FROM (
              SELECT share_url FROM douyin_videos
              WHERE title_filtered = $1
                AND share_url IS NOT NULL AND share_url <> ''
                AND NOT (share_url = ANY($2::text[]))
                AND inserted_at > NOW() - INTERVAL '24 hours'
              UNION ALL
              SELECT share_url FROM douyin_videos
              WHERE suggest_word_filtered = $1
                AND share_url IS NOT NULL AND share_url <> ''
                AND NOT (share_url = ANY($2::text[]))
                AND inserted_at > NOW() - INTERVAL '24 hours'
           ) AS s
           LIMIT 50"#,
    )
    .bind(word)
    .bind(exclude_history)
    .fetch_all(pool)
    .await
    .map_err(|e| format!("pick renewal link: {e}"))?;

    use rand::seq::SliceRandom;
    Ok(candidates.choose(&mut rand::thread_rng()).cloned())
}

#[derive(sqlx::FromRow, Clone)]
struct PendingRow {
    id: i64,
    alias_id: String,
    alias_name: String,
    alias_type: i32,
    backfill_attempts: i32,
    /// JSONB array of links previously submitted successfully. Used to
    /// avoid re-submitting the same link in initial backfill.
    backfill_link_history: JsonValue,
    /// The link used in the most recent (possibly failed) attempt.
    /// Non-null when a prior attempt was made but may have failed.
    /// Used by the renewal path as a preferred retry candidate.
    backfill_post_link: Option<String>,
    /// Non-null for rows that were previously successfully backfilled
    /// and are now in the 29-day renewal cycle. Null for initial backfill.
    backfilled_at: Option<chrono::DateTime<chrono::Local>>,
}

impl PendingRow {
    fn is_renewal(&self) -> bool {
        self.backfilled_at.is_some()
    }

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

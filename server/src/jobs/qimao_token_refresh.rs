//! Long-running worker that keeps every qimao profile's
//! `x-qm-devops-token` fresh by re-calling `/api/v1/user/signin`.
//!
//! Mirrors the legacy C# `RefreshQiMaoTokenJob.cs`:
//!   * Scope: every browser_profile with `kol_platform='qimao'` AND
//!     non-empty credentials.
//!   * Trigger: token is NULL OR `token_refreshed_at < NOW() - 12h`.
//!   * Outcome: on success, persist `qimao_token` + clear last_error;
//!     on failure, persist last_error and leave token alone (the next
//!     sweep retries).
//!
//! Workers (`qimao_rank`, the future qimao alias submitter) just read
//! `qimao_token` directly. They never touch the credential.
//!
//! Scheduling: poll every 30 minutes. The 12h freshness window is much
//! larger than the poll cadence, so even if a sweep is missed for a
//! few hours nothing breaks.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::db::DbPool;
use crate::services::qimao_promotion::{
    build_http_client, signin, ENDPOINT_SIGNIN, SERVICE_NAME,
};

const POLL_INTERVAL: Duration = Duration::from_secs(1800); // 30min
// Refresh window: 12 hours since last refresh. Hardcoded into the SQL
// literal in `fetch_candidates` (was a constant + format!() which
// bypassed the prepared statement plan cache for no benefit since the
// value never varies).

/// Profiles processed concurrently per chunk. Cold start can have ~50
/// profiles all needing refresh at once; serial signin would take
/// 25–50 s and starve other qimao workers waiting for tokens. 2 is
/// the same conservative ceiling used by alias/backfill submitters.
const CONCURRENCY: usize = 2;

pub async fn start_worker(pool: Arc<DbPool>) {
    tracing::info!("qimao_token_refresh: worker starting");
    let mut tick = tokio::time::interval(POLL_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Don't skip the first tick — on cold start we want to refresh
    // immediately (otherwise other workers find no usable token for
    // the first 30 min).
    loop {
        tick.tick().await;
        match sweep(&pool).await {
            Ok(0) => {} // idle, no log spam
            Ok(n) => tracing::info!("qimao_token_refresh: refreshed {n} profile(s)"),
            Err(e) => tracing::warn!("qimao_token_refresh: round failed: {e}"),
        }
    }
}

async fn sweep(pool: &DbPool) -> Result<usize, String> {
    let candidates: Vec<(Uuid, String, String)> = fetch_candidates(pool).await?;
    if candidates.is_empty() {
        return Ok(0);
    }

    let http = build_http_client()?;
    let mut done = 0usize;

    // Concurrent chunks. Each profile is independent (separate qimao
    // account); no shared cookie/token to invalidate, so no early-abort
    // semantics — one failure doesn't affect the rest of the chunk.
    for chunk in candidates.chunks(CONCURRENCY) {
        let futs = chunk.iter().map(|c| {
            let pool = pool.clone();
            let http = http.clone();
            let c = c.clone();
            async move { handle_profile(&pool, &http, c).await }
        });
        let results = futures_util::future::join_all(futs).await;
        for ok in results {
            if ok {
                done += 1;
            }
        }
    }
    Ok(done)
}

/// Refresh one profile's token: signin → persist token or error.
/// Returns true on successful token persisted, false otherwise (errors
/// are already logged in-place).
async fn handle_profile(
    pool: &DbPool,
    http: &reqwest_middleware::ClientWithMiddleware,
    candidate: (Uuid, String, String),
) -> bool {
    let (profile_id, identifier, credential) = candidate;
    let outcome = signin(http, &identifier, &credential).await;
    // Audit the call — useful when an account starts failing. Don't log
    // identifier/credential in the request_summary so the audit table
    // never accumulates plaintext credentials.
    let request_summary = json!({
        "profile_id": profile_id,
        "identifier_suffix": identifier_tail(&identifier),
    });
    match outcome
        .audit(pool, SERVICE_NAME, ENDPOINT_SIGNIN, request_summary)
        .await
    {
        Ok(token) => {
            if let Err(e) = sqlx::query(
                r#"UPDATE browser_profiles
                   SET qimao_token = $1,
                       qimao_token_refreshed_at = NOW(),
                       qimao_token_last_error = NULL
                   WHERE id = $2"#,
            )
            .bind(&token)
            .bind(profile_id)
            .execute(pool)
            .await
            {
                tracing::warn!(
                    "qimao_token_refresh: persist token {profile_id}: {e}"
                );
                return false;
            }
            tracing::info!(
                "qimao_token_refresh: ok profile={profile_id} identifier=…{}",
                identifier_tail(&identifier)
            );
            true
        }
        Err(err) => {
            let reason = err.to_string();
            if let Err(e) = sqlx::query(
                r#"UPDATE browser_profiles
                   SET qimao_token_last_error = $1,
                       qimao_token_refreshed_at = NOW()
                   WHERE id = $2"#,
            )
            .bind(&reason)
            .bind(profile_id)
            .execute(pool)
            .await
            {
                tracing::warn!(
                    "qimao_token_refresh: persist error {profile_id}: {e}"
                );
            }
            tracing::warn!(
                "qimao_token_refresh: signin failed profile={profile_id} reason={reason}"
            );
            false
        }
    }
}

/// Eligible rows: kol_platform='qimao', credentials present, and
/// either no token yet or last refresh older than 12 hours.
async fn fetch_candidates(pool: &DbPool) -> Result<Vec<(Uuid, String, String)>, String> {
    // Static SQL (no format!()) so sqlx + Postgres can cache the
    // prepared statement plan across rounds. The 12h window is the
    // legacy C# refresh cadence and never changes at runtime.
    // Owner-active gate: don't waste sign-in attempts on disabled
     // users' qimao profiles. Tokens stay stale until re-enabled (and
    // will refresh on the next scheduled tick after that).
    let rows = sqlx::query(
        r#"SELECT bp.id, bp.qimao_identifier, bp.qimao_credential
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.kol_platform = 'qimao'
             AND bp.qimao_identifier IS NOT NULL AND bp.qimao_identifier <> ''
             AND bp.qimao_credential IS NOT NULL AND bp.qimao_credential <> ''
             AND u.is_active = TRUE
             AND (
                 bp.qimao_token IS NULL
                 OR bp.qimao_token_refreshed_at IS NULL
                 OR bp.qimao_token_refreshed_at < NOW() - INTERVAL '12 hours'
             )
           ORDER BY bp.qimao_token_refreshed_at NULLS FIRST
           LIMIT 50"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| format!("fetch candidates: {e}"))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let id: Uuid = r.try_get("id").map_err(|e| format!("id col: {e}"))?;
        let identifier: String = r
            .try_get("qimao_identifier")
            .map_err(|e| format!("identifier col: {e}"))?;
        let credential: String = r
            .try_get("qimao_credential")
            .map_err(|e| format!("credential col: {e}"))?;
        out.push((id, identifier, credential));
    }
    Ok(out)
}

/// Return last 4 characters of the identifier for log lines — enough
/// to disambiguate accounts during debugging without leaking the full
/// phone number / email into logs.
fn identifier_tail(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    if n <= 4 {
        return s.to_string();
    }
    chars[n - 4..].iter().collect()
}

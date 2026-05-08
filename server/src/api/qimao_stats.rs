//! Read endpoints for the qimao dashboard. Mirrors `tomato_stats` in
//! shape so the client can reuse the same UI patterns.
//!
//! Two views:
//!
//! - `GET /api/qimao/stats/overview` — global rollup of qimao_aliases.
//! - `GET /api/qimao/stats/accounts` — per-profile breakdown for every
//!   browser profile with `kol_platform='qimao'`. Token health (token
//!   present? last refresh? last error?) lives on the row instead of
//!   the cookie-based `is_online` we use for tomato.

use std::sync::RwLock;
use std::time::{Duration, Instant};

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use once_cell::sync::Lazy;
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::DbPool;
use crate::errors::AppResult;

const OVERVIEW_TTL: Duration = Duration::from_secs(60);
const ACCOUNTS_TTL: Duration = Duration::from_secs(60);

static OVERVIEW_CACHE: Lazy<RwLock<Option<(Instant, OverviewResponse)>>> =
    Lazy::new(|| RwLock::new(None));

static ACCOUNTS_CACHE: Lazy<RwLock<Option<(Instant, Vec<AccountStats>)>>> =
    Lazy::new(|| RwLock::new(None));

/// Global counters across all qimao_aliases rows.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewResponse {
    pub total: i64,
    pub submit_pending: i64,
    pub submit_done: i64,
    pub submit_failed: i64,
    /// `submitted` rows whose alias_id is still NULL — i.e. the
    /// platform hasn't surfaced them via keyword_page yet, OR our
    /// backfill worker hasn't polled them yet. Useful as an indicator
    /// that the backfill worker has work queued.
    pub awaiting_alias_id: i64,
    pub backfill_pending: i64,
    pub backfill_done: i64,
    pub backfill_failed: i64,
}

/// `GET /api/qimao/stats/overview`
///
/// Result is cached for OVERVIEW_TTL (60s). Same rationale as
/// `tomato_stats::overview` — full-table scan on a growing table.
pub async fn overview(pool: web::Data<DbPool>, _: AuthUser) -> AppResult<HttpResponse> {
    // Fast path: return cached value if still fresh.
    if let Ok(guard) = OVERVIEW_CACHE.read() {
        if let Some((ts, ref cached)) = *guard {
            if ts.elapsed() < OVERVIEW_TTL {
                return Ok(HttpResponse::Ok().json(cached));
            }
        }
    }

    // Slow path: fetch from DB, update cache.
    let row: (i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              COUNT(*) AS total,
              COUNT(*) FILTER (WHERE status = 'pending')                                        AS submit_pending,
              COUNT(*) FILTER (WHERE status = 'submitted')                                      AS submit_done,
              COUNT(*) FILTER (WHERE status = 'failed')                                         AS submit_failed,
              COUNT(*) FILTER (WHERE status = 'submitted' AND alias_id IS NULL)                 AS awaiting_alias_id,
              COUNT(*) FILTER (WHERE status = 'submitted' AND backfill_status = 'pending')      AS backfill_pending,
              COUNT(*) FILTER (WHERE backfill_status = 'submitted')                             AS backfill_done,
              COUNT(*) FILTER (WHERE backfill_status = 'failed')                                AS backfill_failed
           FROM qimao_aliases"#,
    )
    .fetch_one(pool.get_ref())
    .await?;

    let resp = OverviewResponse {
        total: row.0,
        submit_pending: row.1,
        submit_done: row.2,
        submit_failed: row.3,
        awaiting_alias_id: row.4,
        backfill_pending: row.5,
        backfill_done: row.6,
        backfill_failed: row.7,
    };

    if let Ok(mut guard) = OVERVIEW_CACHE.write() {
        *guard = Some((Instant::now(), resp.clone()));
    }

    Ok(HttpResponse::Ok().json(resp))
}

/// One row per qimao profile. Counts only work attributed to this
/// profile via `submitted_by_profile_id` / `backfilled_by_profile_id`.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AccountStats {
    pub profile_id: Uuid,
    pub profile_name: String,
    /// Account identifier (phone or email) — masked in the dashboard
    /// to last 4 chars, but returned verbatim so the client can decide
    /// formatting.
    pub qimao_identifier: Option<String>,
    /// `true` when `qimao_token` is non-empty. The qimao_token_refresh
    /// worker keeps it fresh every ~12h; if false, the worker either
    /// hasn't run yet or signin failed.
    pub has_token: bool,
    pub qimao_token_refreshed_at: Option<DateTime<Local>>,
    pub qimao_token_last_error: Option<String>,
    /// Successful alias submissions stamped to this account.
    pub submit_done: i64,
    /// Terminal-failed alias submissions stamped to this account.
    pub submit_failed: i64,
    /// Successful backfills (add_keyword_links) stamped to this account.
    pub backfill_done: i64,
    /// Terminal-failed backfills stamped to this account.
    pub backfill_failed: i64,
    /// Most recent submission timestamp by this account.
    pub last_submitted_at: Option<DateTime<Local>>,
}

/// `GET /api/qimao/stats/accounts`
///
/// Cached for ACCOUNTS_TTL (60s). Same rationale as
/// `tomato_stats::accounts`.
pub async fn accounts(pool: web::Data<DbPool>, _: AuthUser) -> AppResult<HttpResponse> {
    // Fast path: return cached vec if still fresh.
    if let Ok(guard) = ACCOUNTS_CACHE.read() {
        if let Some((ts, ref cached)) = *guard {
            if ts.elapsed() < ACCOUNTS_TTL {
                return Ok(HttpResponse::Ok().json(cached));
            }
        }
    }

    // Slow path: fetch + cache.
    let rows = sqlx::query_as::<_, AccountStats>(
        r#"SELECT
              bp.id                                       AS profile_id,
              bp.name                                     AS profile_name,
              bp.qimao_identifier                         AS qimao_identifier,
              (bp.qimao_token IS NOT NULL AND bp.qimao_token <> '') AS has_token,
              bp.qimao_token_refreshed_at                 AS qimao_token_refreshed_at,
              bp.qimao_token_last_error                   AS qimao_token_last_error,
              COALESCE(s.submit_done, 0)                  AS submit_done,
              COALESCE(s.submit_failed, 0)                AS submit_failed,
              COALESCE(b.backfill_done, 0)                AS backfill_done,
              COALESCE(b.backfill_failed, 0)              AS backfill_failed,
              s.last_submitted_at                         AS last_submitted_at
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           LEFT JOIN (
               SELECT submitted_by_profile_id AS pid,
                      COUNT(*) FILTER (WHERE status = 'submitted')          AS submit_done,
                      COUNT(*) FILTER (WHERE status = 'failed')             AS submit_failed,
                      MAX(submitted_at)                                     AS last_submitted_at
               FROM qimao_aliases
               WHERE submitted_by_profile_id IS NOT NULL
               GROUP BY submitted_by_profile_id
           ) s ON s.pid = bp.id
           LEFT JOIN (
               SELECT backfilled_by_profile_id AS pid,
                      COUNT(*) FILTER (WHERE backfill_status = 'submitted') AS backfill_done,
                      COUNT(*) FILTER (WHERE backfill_status = 'failed')    AS backfill_failed
               FROM qimao_aliases
               WHERE backfilled_by_profile_id IS NOT NULL
               GROUP BY backfilled_by_profile_id
           ) b ON b.pid = bp.id
           WHERE bp.kol_platform = 'qimao'
             AND u.is_active = TRUE
           ORDER BY has_token DESC, bp.name ASC"#,
    )
    .fetch_all(pool.get_ref())
    .await?;

    if let Ok(mut guard) = ACCOUNTS_CACHE.write() {
        *guard = Some((Instant::now(), rows.clone()));
    }

    Ok(HttpResponse::Ok().json(rows))
}

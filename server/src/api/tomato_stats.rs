//! Read endpoints for the tomato dashboard.
//!
//! Two views, deliberately split:
//!
//! - `GET /api/tomato/stats/overview` — global counters across all rows.
//!   "Pending" is a global concept (rows not yet picked up by any
//!   worker), so no per-account filter — the answer wouldn't mean what
//!   the dashboard wants.
//!
//! - `GET /api/tomato/stats/accounts` — per-account row, only counts
//!   work *attributed* to each account (submitted_by_profile_id or
//!   backfilled_by_profile_id). Carries cookie health
//!   (is_online / offline_reason / last_offline_at) so the dashboard
//!   can render an "离线" badge and tell the operator which account
//!   needs re-login.
//!
//! Both endpoints are read-only and authenticated as any logged-in
//! user — currently no admin gate, since the dashboard is the
//! operational surface for the same admins that manage tomato cookies
//! anyway.

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

/// Global rollup. All rows in `tomato_aliases`, regardless of which
/// account submitted them.
#[derive(Debug, Clone, Serialize)]
pub struct OverviewResponse {
    pub total: i64,
    pub submit_pending: i64,
    pub submit_done: i64,
    pub submit_failed: i64,
    /// Eligible-for-backfill subset of submit_done that hasn't been
    /// backfilled yet (status='submitted' AND backfill_status='pending').
    pub backfill_pending: i64,
    pub backfill_done: i64,
    pub backfill_failed: i64,
}

/// `GET /api/tomato/stats/overview` — single-row global counters.
///
/// Result is cached for OVERVIEW_TTL (60s). The full-table COUNT(*)
/// scan becomes expensive as tomato_aliases grows; the dashboard
/// tolerates 60s staleness for operational monitoring.
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
    let row: (i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        r#"SELECT
              COUNT(*) AS total,
              COUNT(*) FILTER (WHERE status = 'pending')                                            AS submit_pending,
              COUNT(*) FILTER (WHERE status = 'submitted')                                          AS submit_done,
              COUNT(*) FILTER (WHERE status = 'failed')                                             AS submit_failed,
              COUNT(*) FILTER (WHERE status = 'submitted' AND backfill_status = 'pending')          AS backfill_pending,
              COUNT(*) FILTER (WHERE backfill_status = 'submitted')                                 AS backfill_done,
              COUNT(*) FILTER (WHERE backfill_status = 'failed')                                    AS backfill_failed
           FROM tomato_aliases"#,
    )
    .fetch_one(pool.get_ref())
    .await?;

    let resp = OverviewResponse {
        total: row.0,
        submit_pending: row.1,
        submit_done: row.2,
        submit_failed: row.3,
        backfill_pending: row.4,
        backfill_done: row.5,
        backfill_failed: row.6,
    };

    if let Ok(mut guard) = OVERVIEW_CACHE.write() {
        *guard = Some((Instant::now(), resp.clone()));
    }

    Ok(HttpResponse::Ok().json(resp))
}

/// One row per tomato account (any user, admin or otherwise) that has
/// cookies stored for `kol.fanqieopen.com`. `*_count` fields count rows
/// the account is *attributed to*; pending rows (no attribution yet)
/// are intentionally excluded — they belong to the global overview.
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct AccountStats {
    pub profile_id: Uuid,
    pub profile_name: String,
    pub is_online: bool,
    pub offline_reason: Option<String>,
    pub last_offline_at: Option<DateTime<Local>>,
    pub cookie_updated_at: DateTime<Local>,

    /// Successful alias submissions stamped to this account.
    pub submit_done: i64,
    /// Terminal-failed alias submissions stamped to this account.
    pub submit_failed: i64,
    /// Successful backfills (post/create) stamped to this account.
    pub backfill_done: i64,
    /// Terminal-failed backfills stamped to this account.
    pub backfill_failed: i64,
    /// Most recent `submitted_at` for any row this account submitted —
    /// so the UI can show "上次活跃" without an extra query.
    pub last_submitted_at: Option<DateTime<Local>>,
}

/// `GET /api/tomato/stats/accounts` — per-account breakdown.
///
/// Cached for ACCOUNTS_TTL (60s). The two GROUP BY subqueries aggregate
/// the entire tomato_aliases table; covering indexes (migration 030)
/// make this index-only but the scan still grows linearly with the
/// 30-day retained row count.
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
              pkc.profile_id                              AS profile_id,
              bp.name                                     AS profile_name,
              pkc.is_online                               AS is_online,
              pkc.offline_reason                          AS offline_reason,
              pkc.last_offline_at                         AS last_offline_at,
              pkc.updated_at                              AS cookie_updated_at,
              COALESCE(s.submit_done, 0)                  AS submit_done,
              COALESCE(s.submit_failed, 0)                AS submit_failed,
              COALESCE(b.backfill_done, 0)                AS backfill_done,
              COALESCE(b.backfill_failed, 0)              AS backfill_failed,
              s.last_submitted_at                         AS last_submitted_at
           FROM platform_kol_cookies pkc
           JOIN browser_profiles bp ON bp.id = pkc.profile_id
           JOIN users u             ON u.id = bp.user_id
           LEFT JOIN (
               SELECT submitted_by_profile_id AS pid,
                      COUNT(*) FILTER (WHERE status = 'submitted')          AS submit_done,
                      COUNT(*) FILTER (WHERE status = 'failed')             AS submit_failed,
                      MAX(submitted_at)                                     AS last_submitted_at
               FROM tomato_aliases
               WHERE submitted_by_profile_id IS NOT NULL
               GROUP BY submitted_by_profile_id
           ) s ON s.pid = pkc.profile_id
           LEFT JOIN (
               SELECT backfilled_by_profile_id AS pid,
                      COUNT(*) FILTER (WHERE backfill_status = 'submitted') AS backfill_done,
                      COUNT(*) FILTER (WHERE backfill_status = 'failed')    AS backfill_failed
               FROM tomato_aliases
               WHERE backfilled_by_profile_id IS NOT NULL
               GROUP BY backfilled_by_profile_id
           ) b ON b.pid = pkc.profile_id
           WHERE pkc.platform = 'tomato'
             AND pkc.domain = 'kol.fanqieopen.com'
             AND u.is_active = TRUE
           ORDER BY pkc.is_online DESC, bp.name ASC"#,
    )
    .fetch_all(pool.get_ref())
    .await?;

    if let Ok(mut guard) = ACCOUNTS_CACHE.write() {
        *guard = Some((Instant::now(), rows.clone()));
    }

    Ok(HttpResponse::Ok().json(rows))
}

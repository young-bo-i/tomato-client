//! Admin-only read endpoints for the income panel.
//!
//! `GET /api/admin/income` — one row per tomato profile that has been
//! polled at least once, joined to `browser_profiles` + `users` so
//! the UI can render account name + owner without follow-up queries.
//!
//! All amounts are 整数分 (cents); the UI divides by 100. The verbose
//! per-task / weekly / monthly breakdowns are returned verbatim as
//! JSONB so the UI can render historical charts.

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::Serialize;
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::AppResult;

/// One row in the admin income panel. Sorted by total_income DESC by
/// default — top earners first — matching the index already on
/// `kol_income.total_income`.
#[derive(Debug, Serialize, FromRow)]
pub struct IncomeRow {
    pub profile_id: Uuid,
    pub profile_name: String,
    pub owner_user_id: i32,
    pub owner_username: String,
    pub owner_role: String,

    /// All in 分 (cents).
    pub total_income: i64,
    pub regular_income: i64,
    pub bonus_income: i64,
    pub current_week_income: i64,
    pub current_month_income: i64,

    /// Upstream's `latest_update_time` converted to local time. NULL
    /// means upstream hasn't computed any income yet for this account.
    pub latest_update_time: Option<DateTime<Local>>,

    /// Verbatim arrays from upstream — the UI iterates these for
    /// historical breakdown.
    pub weekly_income_list: Option<JsonValue>,
    pub monthly_income_list: Option<JsonValue>,
    pub task_income_list: Option<JsonValue>,

    /// Most-recent positive jump we've recorded. `last_diff = 0` means
    /// no forward movement has been observed yet (or the row was just
    /// created with the upstream's already-final total).
    pub last_diff: i64,
    pub last_diff_at: Option<DateTime<Local>>,

    /// When the diff email for `last_diff_at` was successfully sent.
    /// `last_emailed_at < last_diff_at` (or NULL) means the diff is
    /// pending email — usually the previous round's SMTP attempt
    /// failed; check `last_email_error`.
    pub last_emailed_at: Option<DateTime<Local>>,
    /// Most recent SMTP failure reason, verbatim. Cleared on next
    /// successful send. Hover-tooltip in the admin panel.
    pub last_email_error: Option<String>,

    /// Heartbeat — when this row was last refreshed by the poller.
    pub fetched_at: DateTime<Local>,
}

/// `GET /api/admin/income` — list all polled tomato accounts with
/// their latest income snapshot, sorted by total_income DESC.
///
/// Includes inactive owners' rows too (admin still wants to see what
/// they earned before being deactivated). UI can filter client-side
/// if needed.
pub async fn list(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    let rows = sqlx::query_as::<_, IncomeRow>(
        r#"SELECT
              ki.profile_id,
              bp.name                AS profile_name,
              bp.user_id             AS owner_user_id,
              u.username             AS owner_username,
              u.role                 AS owner_role,
              ki.total_income, ki.regular_income, ki.bonus_income,
              ki.current_week_income, ki.current_month_income,
              ki.latest_update_time,
              ki.weekly_income_list, ki.monthly_income_list, ki.task_income_list,
              ki.last_diff, ki.last_diff_at,
              ki.last_emailed_at, ki.last_email_error,
              ki.fetched_at
           FROM kol_income ki
           JOIN browser_profiles bp ON bp.id = ki.profile_id
           JOIN users u             ON u.id = bp.user_id
           ORDER BY ki.total_income DESC, bp.name ASC"#,
    )
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(rows))
}

/// Aggregated overview for the panel header — one row of totals
/// summed across every polled account.
#[derive(Debug, Serialize)]
pub struct IncomeOverview {
    pub account_count: i64,
    pub total_income: i64,
    pub regular_income: i64,
    pub bonus_income: i64,
    pub current_week_income: i64,
    pub current_month_income: i64,
    /// Most recent fetched_at across all rows — proxy for "is the
    /// poller alive".
    pub last_fetched_at: Option<DateTime<Local>>,
}

/// `GET /api/admin/income/overview` — sum-row for the top of the
/// panel. Single query, no caching (the underlying table is small —
/// up to a few hundred rows).
pub async fn overview(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    let row: (i64, i64, i64, i64, i64, i64, Option<DateTime<Local>>) = sqlx::query_as(
        r#"SELECT
              COUNT(*)                         AS account_count,
              COALESCE(SUM(total_income), 0)   AS total_income,
              COALESCE(SUM(regular_income), 0) AS regular_income,
              COALESCE(SUM(bonus_income), 0)   AS bonus_income,
              COALESCE(SUM(current_week_income), 0)  AS current_week_income,
              COALESCE(SUM(current_month_income), 0) AS current_month_income,
              MAX(fetched_at)                  AS last_fetched_at
           FROM kol_income"#,
    )
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(IncomeOverview {
        account_count: row.0,
        total_income: row.1,
        regular_income: row.2,
        bonus_income: row.3,
        current_week_income: row.4,
        current_month_income: row.5,
        last_fetched_at: row.6,
    }))
}

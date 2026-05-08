//! Caller-scoped read endpoints for the 番茄达人 income panel.
//!
//! `GET /api/users/me/income` — every income row for the caller's
//! tomato profiles (admin sees only THEIR own profiles too — the
//! all-users digest is delivered via email, "[管理员速览]").
//!
//! `GET /api/users/me/income/overview` — sum-row across the caller's
//! tomato profiles for the panel header.
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

use crate::auth::AuthUser;
use crate::db::DbPool;
use crate::errors::AppResult;

/// One row in the income panel. Sorted by total_income DESC so the
/// caller's top-earner is on top.
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

    /// Verbatim arrays from upstream — UI iterates these.
    pub weekly_income_list: Option<JsonValue>,
    pub monthly_income_list: Option<JsonValue>,
    pub task_income_list: Option<JsonValue>,

    pub last_diff: i64,
    pub last_diff_at: Option<DateTime<Local>>,

    pub last_emailed_at: Option<DateTime<Local>>,
    pub last_email_error: Option<String>,

    pub fetched_at: DateTime<Local>,
}

/// `GET /api/users/me/income` — list the caller's polled tomato
/// accounts with their latest income snapshot, sorted by total_income
/// DESC. Filter by `bp.user_id = caller`, so admin sees only their
/// own profiles too (cross-user view goes via the [管理员速览] email).
pub async fn list(pool: web::Data<DbPool>, user: AuthUser) -> AppResult<HttpResponse> {
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
           WHERE bp.user_id = $1
           ORDER BY ki.total_income DESC, bp.name ASC"#,
    )
    .bind(user.0.sub)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(rows))
}

/// Aggregated overview — one row of totals summed across the caller's
/// polled accounts.
#[derive(Debug, Serialize)]
pub struct IncomeOverview {
    pub account_count: i64,
    pub total_income: i64,
    pub regular_income: i64,
    pub bonus_income: i64,
    pub current_week_income: i64,
    pub current_month_income: i64,
    pub last_fetched_at: Option<DateTime<Local>>,
}

/// `GET /api/users/me/income/overview` — sum-row for the top of the
/// panel. SUM(BIGINT) returns NUMERIC in Postgres (overflow guard);
/// the explicit `::BIGINT` cast keeps sqlx happy reading into i64.
pub async fn overview(pool: web::Data<DbPool>, user: AuthUser) -> AppResult<HttpResponse> {
    let row: (i64, i64, i64, i64, i64, i64, Option<DateTime<Local>>) = sqlx::query_as(
        r#"SELECT
              COUNT(*)                                            AS account_count,
              COALESCE(SUM(ki.total_income), 0)::BIGINT           AS total_income,
              COALESCE(SUM(ki.regular_income), 0)::BIGINT         AS regular_income,
              COALESCE(SUM(ki.bonus_income), 0)::BIGINT           AS bonus_income,
              COALESCE(SUM(ki.current_week_income), 0)::BIGINT    AS current_week_income,
              COALESCE(SUM(ki.current_month_income), 0)::BIGINT   AS current_month_income,
              MAX(ki.fetched_at)                                  AS last_fetched_at
           FROM kol_income ki
           JOIN browser_profiles bp ON bp.id = ki.profile_id
           WHERE bp.user_id = $1"#,
    )
    .bind(user.0.sub)
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

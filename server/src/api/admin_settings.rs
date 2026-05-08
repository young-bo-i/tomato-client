//! GET/PUT `/api/admin/settings` — singleton admin-tunable runtime
//! settings. Currently exposes `admin_contribution_pct`; mirrors the
//! `admin_settings` table layout.
//!
//! PUT validates `0..=100` (the DB CHECK enforces this too, but
//! returning a 400 with the input value is friendlier than letting
//! sqlx surface a constraint violation). On success we invalidate the
//! `services::admin_settings` cache so router behavior reflects the
//! change without waiting for the 60s TTL.

use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::auth::AdminUser;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};

#[derive(Debug, Serialize)]
pub struct AdminSettingsView {
    pub admin_contribution_pct: i32,
    pub updated_at: DateTime<Local>,
}

/// `GET /api/admin/settings` — fetch the current global settings.
pub async fn get(pool: web::Data<DbPool>, _: AdminUser) -> AppResult<HttpResponse> {
    let row = sqlx::query(
        "SELECT admin_contribution_pct, updated_at FROM admin_settings WHERE id = 1",
    )
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(AdminSettingsView {
        admin_contribution_pct: row.try_get("admin_contribution_pct").unwrap_or(0),
        updated_at: row.try_get("updated_at")?,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateBody {
    pub admin_contribution_pct: i32,
}

/// Allowed contribution buckets. Restricted to a discrete set rather
/// than a free 0..=100 slider because (a) the operator only thinks
/// in "every Nth word" cadences anyway, and (b) every value here
/// divides 100 cleanly so the Bresenham distribution becomes a
/// strict "every N" period (e.g. 20 → exactly every 5th).
const ALLOWED_PCT: &[i32] = &[0, 10, 20, 50, 100];

/// `PUT /api/admin/settings` — update in place. The migration seeds
/// the row, so this is a pure UPDATE (no UPSERT needed).
pub async fn put(
    pool: web::Data<DbPool>,
    body: web::Json<UpdateBody>,
    _: AdminUser,
) -> AppResult<HttpResponse> {
    let pct = body.into_inner().admin_contribution_pct;
    if !ALLOWED_PCT.contains(&pct) {
        return Err(AppError::BadRequest(format!(
            "admin_contribution_pct must be one of {ALLOWED_PCT:?}, got {pct}"
        )));
    }

    let updated = sqlx::query(
        r#"UPDATE admin_settings
           SET admin_contribution_pct = $1, updated_at = NOW()
           WHERE id = 1"#,
    )
    .bind(pct)
    .execute(pool.get_ref())
    .await?
    .rows_affected();

    if updated == 0 {
        // Should be unreachable: migration 002 seeds id=1.
        return Ok(HttpResponse::InternalServerError().json(serde_json::json!({
            "ok": false, "error": "admin_settings row missing"
        })));
    }

    crate::services::admin_settings::invalidate();
    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true })))
}

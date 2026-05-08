//! User self-edit endpoints (anyone authenticated can call these on
//! their own account; admins use `/api/admin/users/:id` for cross-user
//! edits).
//!
//! Currently exposes a single endpoint:
//!
//!   PUT /api/users/me/tier2_contribution
//!
//! Updates the caller's `tier2_contribution_pct` — the rate at which
//! their tier-2 subordinates' words flow up to them. Tier-2 users and
//! admins technically have the column but it has no effect for them
//! (no subordinates), so the API still accepts the write but the
//! caller doesn't see the option in the UI unless `has_subordinates`
//! is true.

use actix_web::{web, HttpResponse};
use serde::Deserialize;

use crate::auth::AuthUser;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};

/// Allowed buckets — same set as admin_contribution_pct so the UX is
/// consistent. Centralized at the API layer because the DB CHECK is
/// the wider 0..=100 envelope.
const ALLOWED_TIER2_PCT: &[i32] = &[0, 10, 20, 50, 100];

#[derive(Debug, Deserialize)]
pub struct UpdateTier2Body {
    pub tier2_contribution_pct: i32,
}

/// `PUT /api/users/me/tier2_contribution` — update the caller's own
/// `tier2_contribution_pct`. Returns `{ ok: true }` on success.
///
/// Auth: any logged-in user. Caller scope = `user.0.sub`; cross-user
/// edits go through `/api/admin/users/:id` (admin-only).
///
/// **Defensive gate**: the caller must currently have at least one
/// tier-2 subordinate. The frontend only shows the team-management
/// nav when `has_subordinates`, but the API enforces it too so a
/// curl-poker can't silently push values into a column that has no
/// routing effect for them. Admins editing a different user's value
/// use `PATCH /admin/users/:id` (no subordinate gate there — admins
/// can configure any tier-1 in advance).
pub async fn update_my_tier2_contribution(
    pool: web::Data<DbPool>,
    user: AuthUser,
    body: web::Json<UpdateTier2Body>,
) -> AppResult<HttpResponse> {
    let pct = body.into_inner().tier2_contribution_pct;
    if !ALLOWED_TIER2_PCT.contains(&pct) {
        return Err(AppError::BadRequest(format!(
            "tier2_contribution_pct must be one of {ALLOWED_TIER2_PCT:?}, got {pct}"
        )));
    }

    // Reject if the caller has no subordinates: the value would be a
    // no-op for routing, and exposing the endpoint as "always 200"
    // implies state changes that don't actually affect anything.
    let has_subs: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM users WHERE parent_user_id = $1)",
    )
    .bind(user.0.sub)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or(false);
    if !has_subs {
        return Err(AppError::Forbidden);
    }

    let updated = sqlx::query(
        r#"UPDATE users
           SET tier2_contribution_pct = $1
           WHERE id = $2"#,
    )
    .bind(pct)
    .bind(user.0.sub)
    .execute(pool.get_ref())
    .await?
    .rows_affected();

    if updated == 0 {
        return Err(AppError::NotFound(format!("user {}", user.0.sub)));
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({ "ok": true })))
}

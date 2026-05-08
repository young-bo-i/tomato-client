//! Pick a usable 七猫达人 token from any browser_profile (any active
//! user — the desktop client is multi-user; admin is just one tier of
//! the submission_router fallback, not a credential-ownership rule).
//! The token is signed-in by `jobs::qimao_token_refresh` and stored
//! on the profile row directly (no separate accounts table — the
//! credential/token columns live alongside the profile so the
//! create-profile dialog is the single source of truth).
//!
//! "Usable" means `qimao_token IS NOT NULL AND <> ''` AND the row's
//! owning user is active. Random selection spreads load across qimao
//! accounts the same way `tomato_cookie::pick_random_online` does for
//! 番茄达人.

use std::sync::Arc;

use sqlx::Row;
use uuid::Uuid;

use crate::db::DbPool;

/// Selected qimao token. `token: Arc<str>` so concurrent worker chunks
/// can clone the SelectedAccount without re-heap-allocating the token
/// string (~50 bytes but cloned 30× per round under load).
#[derive(Debug, Clone)]
pub struct SelectedAccount {
    pub profile_id: Uuid,
    pub token: Arc<str>,
}

/// Pick a random usable qimao account. Returns `Ok(None)` when no
/// profile has a fresh token yet — workers idle in that state instead
/// of failing.
///
/// Use `pick_random_active_for_user` when work must stay within one
/// user's own accounts (alias/backfill submission).
pub async fn pick_random_active(pool: &DbPool) -> Result<Option<SelectedAccount>, String> {
    pick_account(pool, None).await
}

/// Same as `pick_random_active` but restricted to profiles owned by
/// `user_id`. Used by alias/backfill workers for per-user isolation.
pub async fn pick_random_active_for_user(
    pool: &DbPool,
    user_id: i32,
) -> Result<Option<SelectedAccount>, String> {
    pick_account(pool, Some(user_id)).await
}

/// Pick the token for one specific profile.
pub async fn pick_active_for_profile(
    pool: &DbPool,
    profile_id: uuid::Uuid,
) -> Result<Option<SelectedAccount>, String> {
    // Owner-active gate (defense in depth — same reason as tomato_cookie).
    let row = sqlx::query(
        r#"SELECT bp.id AS profile_id, bp.qimao_token AS token
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.id = $1
             AND bp.qimao_token IS NOT NULL
             AND bp.qimao_token <> ''
             AND u.is_active = TRUE
           LIMIT 1"#,
    )
    .bind(profile_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("pick qimao for profile: {e}"))?;

    let Some(row) = row else { return Ok(None) };
    let pid: uuid::Uuid = row.try_get("profile_id").map_err(|e| format!("profile_id: {e}"))?;
    let token: String = row.try_get("token").map_err(|e| format!("token: {e}"))?;
    Ok(Some(SelectedAccount { profile_id: pid, token: Arc::from(token) }))
}

async fn pick_account(
    pool: &DbPool,
    user_id: Option<i32>,
) -> Result<Option<SelectedAccount>, String> {
    // No `u.role = 'admin'` filter — see tomato_cookie::pick_cookie
    // for the rationale. Every active user can hold qimao accounts;
    // the admin pool is the submission_router's fallback tier, not a
    // hard requirement on credential ownership.
    let row = sqlx::query(
        r#"SELECT bp.id AS profile_id, bp.qimao_token AS token
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.kol_platform = 'qimao'
             AND bp.qimao_token IS NOT NULL
             AND bp.qimao_token <> ''
             AND u.is_active = TRUE
             AND ($1::INTEGER IS NULL OR bp.user_id = $1)
           ORDER BY random()
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("pick qimao account: {e}"))?;

    let Some(row) = row else { return Ok(None) };
    let profile_id: Uuid = row
        .try_get("profile_id")
        .map_err(|e| format!("profile_id col: {e}"))?;
    let token: String = row
        .try_get("token")
        .map_err(|e| format!("token col: {e}"))?;
    Ok(Some(SelectedAccount { profile_id, token: Arc::from(token) }))
}

/// Clear a profile's token so the next `qimao_token_refresh` sweep
/// resigns in. Called by workers on confirmed auth failures.
pub async fn invalidate_token(
    pool: &DbPool,
    profile_id: Uuid,
    reason: &str,
) -> Result<(), String> {
    let trimmed: String = reason.chars().take(500).collect();
    sqlx::query(
        r#"UPDATE browser_profiles
           SET qimao_token = NULL,
               qimao_token_last_error = $1
           WHERE id = $2"#,
    )
    .bind(&trimmed)
    .bind(profile_id)
    .execute(pool)
    .await
    .map_err(|e| format!("invalidate token: {e}"))?;
    tracing::warn!("qimao_account: invalidated token for profile {profile_id} ({trimmed})");
    Ok(())
}

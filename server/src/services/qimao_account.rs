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
/// Random active qimao token owned by `user_id`. Used by alias/
/// backfill workers for per-user isolation.
pub async fn pick_random_active_for_user(
    pool: &DbPool,
    user_id: i32,
) -> Result<Option<SelectedAccount>, String> {
    pick_account(pool, Some(user_id)).await
}

/// Pick a random qimao token owned by **any active admin user**. Used
/// by the daily 七猫书籍 rank scraper. Mirrors the rationale on
/// `tomato_cookie::pick_random_online_admin`: platform-global crawls
/// shouldn't burn an arbitrary user's quota. `Ok(None)` when no admin
/// has a usable token — scraper idles.
pub async fn pick_random_active_admin(pool: &DbPool) -> Result<Option<SelectedAccount>, String> {
    let row = sqlx::query(
        r#"SELECT bp.id AS profile_id, bp.qimao_token AS token
           FROM browser_profiles bp
           JOIN users u ON u.id = bp.user_id
           WHERE bp.kol_platform = 'qimao'
             AND bp.qimao_token IS NOT NULL
             AND bp.qimao_token <> ''
             AND u.is_active = TRUE
             AND u.role = 'admin'
           ORDER BY random()
           LIMIT 1"#,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("pick admin qimao account: {e}"))?;

    let Some(row) = row else { return Ok(None) };
    let profile_id: Uuid = row
        .try_get("profile_id")
        .map_err(|e| format!("profile_id col: {e}"))?;
    let token: String = row
        .try_get("token")
        .map_err(|e| format!("token col: {e}"))?;
    Ok(Some(SelectedAccount { profile_id, token: Arc::from(token) }))
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
/// resigns in. Called from the recover_or_offline path when re-signin
/// itself failed; not used directly by workers anymore (they go
/// through `recover_or_offline` for the self-healing path).
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

/// Outcome of `recover_or_offline`. Workers branch on this to decide
/// whether to retry their original API call inline (rare — usually
/// they just continue and the next round naturally picks the new
/// token) and whether to log loudly.
#[derive(Debug)]
pub enum RecoverOutcome {
    /// Token refreshed in-place. The caller's API request can be
    /// retried with the new token, or just bail and let the next
    /// poll round pick it up. We don't auto-retry inline because
    /// most workers process batches and a partial-batch retry is
    /// awkward — letting the next round redo it is simpler.
    Resigned { new_token: Arc<str> },
    /// Re-signin itself failed with a definitive auth error
    /// (credentials wrong / account locked / etc). Token cleared,
    /// `last_error` stamped, AND `platform_kol_cookies.is_online`
    /// flipped to FALSE on the kol.wtzw.com row so
    /// `notification_dispatcher` picks it up next round and emails
    /// the owner. **This is the only path that triggers a user-
    /// facing notification.**
    CredentialsInvalid { error: String },
    /// Re-signin failed for transient reasons (network error, 5xx,
    /// signing service down). Token left as-is so the next round
    /// can try again; no notification fired.
    Transient { error: String },
    /// Profile has no stored credentials so we can't auto-resign
    /// (admin never set qimao_identifier/credential). Treated as a
    /// notification-worthy event so the operator knows to fix it.
    NoCredentials,
}

/// Self-healing entry point for qimao auth failures.
///
/// Decision tree:
///   1. Profile has no `qimao_identifier` or no `qimao_credential`
///      → `NoCredentials`. We flip pkc.is_online so notification
///        fires (operator must set up credentials).
///   2. Profile has credentials → call `signin`.
///      a. Success → write new token + clear last_error + ensure
///         pkc.is_online=TRUE (recovery clears any prior offline).
///         → `Resigned { new_token }`
///      b. Failure with definitive auth error (`AuthFailed` /
///         `ApiCode` / parse error) → clear token, stamp last_error,
///         flip pkc.is_online=FALSE. → `CredentialsInvalid`
///      c. Failure with transient error (`Transport` / `Sign`) →
///         only stamp last_error, leave is_online untouched.
///         → `Transient`
///
/// This function is idempotent in the recovery direction:
/// calling it twice with the same valid credentials just refreshes
/// the token twice. It's NOT idempotent in the failure direction
/// (each call attempts a signin, which costs a request), but the
/// auth-failure path in workers is rare so this is fine.
pub async fn recover_or_offline(
    pool: &DbPool,
    http: &reqwest_middleware::ClientWithMiddleware,
    profile_id: Uuid,
    reason: &str,
) -> RecoverOutcome {
    use crate::services::qimao_promotion;

    // Pull credentials from DB.
    let row: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        r#"SELECT qimao_identifier, qimao_credential
           FROM browser_profiles WHERE id = $1"#,
    )
    .bind(profile_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let (identifier, credential) = match row {
        Some((Some(id), Some(cr))) if !id.is_empty() && !cr.is_empty() => (id, cr),
        _ => {
            // No stored credentials → can't auto-resign.
            tracing::warn!(
                "qimao_account: profile {profile_id} auth failed but no credentials stored; flipping offline ({reason})"
            );
            mark_qimao_offline(pool, profile_id, "no stored credentials for auto-resign").await;
            stamp_last_error(pool, profile_id, "no stored credentials").await;
            return RecoverOutcome::NoCredentials;
        }
    };

    // Try to re-signin.
    let outcome = qimao_promotion::signin(http, &identifier, &credential).await;
    let snap = outcome.snapshot;
    let result = outcome.result;

    // Audit-log the signin attempt regardless of outcome — the
    // existing api_log machinery captures it for forensic review.
    let parse_error = result.as_ref().err().map(|e| e.to_string());
    crate::services::api_log::log_call(
        pool,
        qimao_promotion::SERVICE_NAME,
        qimao_promotion::ENDPOINT_SIGNIN,
        serde_json::json!({ "profile_id": profile_id, "auto_resign": true, "trigger": reason }),
        &snap,
        result.is_ok(),
        parse_error.as_deref(),
    )
    .await;

    match result {
        Ok(new_token) => {
            // Recovery path: write the new token, clear last_error,
            // ensure pkc rows are online (clears any offline_notified_at
            // so a future fresh failure re-notifies).
            let arc_token: Arc<str> = Arc::from(new_token.clone());
            if let Err(e) = sqlx::query(
                r#"UPDATE browser_profiles
                   SET qimao_token = $1,
                       qimao_token_refreshed_at = NOW(),
                       qimao_token_last_error = NULL
                   WHERE id = $2"#,
            )
            .bind(&new_token)
            .bind(profile_id)
            .execute(pool)
            .await
            {
                tracing::warn!(
                    "qimao_account: recover wrote token but UPDATE failed for {profile_id}: {e}"
                );
            }
            mark_qimao_online(pool, profile_id).await;
            tracing::info!(
                "qimao_account: auto-recovered profile {profile_id} (new token via signin); trigger={reason}"
            );
            RecoverOutcome::Resigned { new_token: arc_token }
        }
        Err(err) => {
            let err_msg = err.to_string();
            let is_transient = matches!(
                err,
                crate::services::upstream_error::UpstreamError::Transport(_)
                    | crate::services::upstream_error::UpstreamError::Sign(_)
                    | crate::services::upstream_error::UpstreamError::HttpError { status: 500..=599, .. }
            );
            if is_transient {
                stamp_last_error(pool, profile_id, &format!("signin transient: {err_msg}")).await;
                tracing::info!(
                    "qimao_account: profile {profile_id} signin transient ({err_msg}); leaving online flag — will retry next round"
                );
                RecoverOutcome::Transient { error: err_msg }
            } else {
                // Definitive credential failure — auth error from
                // signin itself, parse error, or 4xx.
                invalidate_token(pool, profile_id, &format!("signin failed: {err_msg}"))
                    .await
                    .ok();
                mark_qimao_offline(
                    pool,
                    profile_id,
                    &format!("auto-resign failed: {err_msg}"),
                )
                .await;
                tracing::warn!(
                    "qimao_account: profile {profile_id} signin failed definitively ({err_msg}); flipped offline + notification will fire"
                );
                RecoverOutcome::CredentialsInvalid { error: err_msg }
            }
        }
    }
}

/// Stamp `qimao_token_last_error` without touching the token itself.
/// Used for transient signin failures so admins can see the latest
/// reason in the dashboard without losing the still-valid token.
async fn stamp_last_error(pool: &DbPool, profile_id: Uuid, reason: &str) {
    let trimmed: String = reason.chars().take(500).collect();
    if let Err(e) = sqlx::query(
        r#"UPDATE browser_profiles
           SET qimao_token_last_error = $1
           WHERE id = $2"#,
    )
    .bind(&trimmed)
    .bind(profile_id)
    .execute(pool)
    .await
    {
        tracing::warn!("qimao_account: stamp_last_error {profile_id}: {e}");
    }
}

/// Flip the qimao profile's `platform_kol_cookies(kol.wtzw.com)`
/// row to `is_online=FALSE` so notification_dispatcher emails the
/// owner. We touch only the kol.wtzw.com row (the API host) and
/// leave dmp.wtzw.com alone so we generate exactly one notification.
async fn mark_qimao_offline(pool: &DbPool, profile_id: Uuid, reason: &str) {
    let trimmed: String = reason.chars().take(500).collect();
    if let Err(e) = sqlx::query(
        r#"UPDATE platform_kol_cookies
           SET is_online = FALSE,
               offline_reason = $1,
               last_offline_at = NOW(),
               offline_notified_at = NULL
           WHERE profile_id = $2
             AND platform = 'qimao'
             AND domain = 'kol.wtzw.com'
             AND is_online = TRUE"#,
    )
    .bind(&trimmed)
    .bind(profile_id)
    .execute(pool)
    .await
    {
        tracing::warn!("qimao_account: mark_qimao_offline {profile_id}: {e}");
    }
}

/// Recovery: flip qimao profile's pkc rows to online. Called when
/// auto-resign succeeds. Touches BOTH domain rows for symmetry with
/// what fresh-cookie-push would do (clears offline_notified_at on
/// both so future offline events re-notify).
async fn mark_qimao_online(pool: &DbPool, profile_id: Uuid) {
    if let Err(e) = sqlx::query(
        r#"UPDATE platform_kol_cookies
           SET is_online = TRUE,
               offline_reason = NULL,
               offline_notified_at = NULL
           WHERE profile_id = $1
             AND platform = 'qimao'
             AND is_online = FALSE"#,
    )
    .bind(profile_id)
    .execute(pool)
    .await
    {
        tracing::warn!("qimao_account: mark_qimao_online {profile_id}: {e}");
    }
}

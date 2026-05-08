//! Shared selection + health bookkeeping for tomato platform cookies.
//!
//! Three workers (`tomato_rank`, `alias_submitter`, `backfill_submitter`)
//! all need to pull a cookie from `platform_kol_cookies` to talk to
//! kol.fanqieopen.com. Without a shared helper they each re-implement
//! the same selection SQL — which made it easy for them to drift on
//! the new requirements (random pick instead of "freshest", filter
//! `is_online=TRUE`, return profile_id alongside the header).
//!
//! Convention: callers that *attribute work* to an account need
//! `pick_random_online()` (returns the profile_id). Callers that just
//! need *some* working cookie can use `pick_any_online()` and discard
//! the id.
//!
//! On HTTP 401/403 from the upstream, callers are expected to invoke
//! `mark_offline()` so the next selection skips this cookie. There is
//! deliberately no automatic "online again after N hours" — only a
//! fresh cookie upsert (manual re-login pushing state up) flips it
//! back, handled in the cookie write path, not here.

use std::sync::Arc;

use serde_json::Value as JsonValue;
use sqlx::Row;
use uuid::Uuid;

use crate::db::DbPool;

pub const PLATFORM: &str = "tomato";
pub const DOMAIN: &str = "kol.fanqieopen.com";

/// One selection result. `profile_id` is what the caller stamps onto
/// per-row work for stats attribution.
///
/// `cookie_header` is `Arc<str>` so it can be cheaply cloned across the
/// worker chunks (each row in a chunk takes its own owned copy of the
/// SelectedCookie via `.clone()`). For a 200–500 byte cookie cloned
/// 30× per round, the heap allocation cost adds up.
#[derive(Debug, Clone)]
pub struct SelectedCookie {
    pub profile_id: Uuid,
    pub cookie_header: Arc<str>,
}

/// Pick a random tomato cookie that's still considered logged in. Random
/// rather than freshest because the user wants traffic spread evenly
/// across multiple accounts, not concentrated on whoever logged in last.
///
/// Returns `Ok(None)` (not an error) when no eligible cookie exists —
/// workers idle in that state instead of failing.
///
/// Use `pick_random_online_for_user` when work must be attributed to a
/// specific user's own cookies (alias/backfill submission).
pub async fn pick_random_online(pool: &DbPool) -> Result<Option<SelectedCookie>, String> {
    pick_cookie(pool, None).await
}

/// Same as `pick_random_online` but restricted to cookies belonging to
/// `user_id`. Used by alias/backfill workers so each user's pending
/// rows are only ever submitted with that user's own platform cookies.
pub async fn pick_random_online_for_user(
    pool: &DbPool,
    user_id: i32,
) -> Result<Option<SelectedCookie>, String> {
    pick_cookie(pool, Some(user_id)).await
}

/// Pick the cookie for one specific profile (used when target_profile_id is set).
pub async fn pick_online_for_profile(
    pool: &DbPool,
    profile_id: uuid::Uuid,
) -> Result<Option<SelectedCookie>, String> {
    let row = sqlx::query(
        r#"SELECT pkc.profile_id, pkc.cookies
           FROM platform_kol_cookies pkc
           WHERE pkc.profile_id = $1
             AND pkc.platform = $2
             AND pkc.domain = $3
             AND pkc.is_online = TRUE
           LIMIT 1"#,
    )
    .bind(profile_id)
    .bind(PLATFORM)
    .bind(DOMAIN)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("pick cookie for profile: {e}"))?;

    let Some(row) = row else { return Ok(None) };
    let pid: uuid::Uuid = row.try_get("profile_id").map_err(|e| format!("profile_id: {e}"))?;
    let cookies: serde_json::Value = row.try_get("cookies").map_err(|e| format!("cookies: {e}"))?;
    match serialize_cookie_header(&cookies)? {
        Some(h) => Ok(Some(SelectedCookie { profile_id: pid, cookie_header: Arc::from(h) })),
        None => Ok(None),
    }
}

async fn pick_cookie(
    pool: &DbPool,
    user_id: Option<i32>,
) -> Result<Option<SelectedCookie>, String> {
    // No `u.role = 'admin'` filter: every active user can hold their
    // own tomato accounts (the admin pool is just one tier of the
    // submission_router fallback, not a hard requirement). The
    // user_id parameter scopes selection to a specific user when set;
    // pass `None` from rank scrapers / one-off ops where any working
    // cookie is fine.
    let row = sqlx::query(
        r#"SELECT pkc.profile_id, pkc.cookies
           FROM platform_kol_cookies pkc
           JOIN browser_profiles bp ON bp.id = pkc.profile_id
           JOIN users u ON u.id = bp.user_id
           WHERE pkc.platform = $1
             AND pkc.domain = $2
             AND pkc.is_online = TRUE
             AND u.is_active = TRUE
             AND ($3::INTEGER IS NULL OR bp.user_id = $3)
           ORDER BY random()
           LIMIT 1"#,
    )
    .bind(PLATFORM)
    .bind(DOMAIN)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("pick cookie: {e}"))?;

    let Some(row) = row else { return Ok(None) };
    let profile_id: Uuid = row
        .try_get("profile_id")
        .map_err(|e| format!("profile_id col: {e}"))?;
    let cookies: JsonValue = row
        .try_get("cookies")
        .map_err(|e| format!("cookies col: {e}"))?;
    let header = serialize_cookie_header(&cookies)?;
    match header {
        Some(h) => Ok(Some(SelectedCookie {
            profile_id,
            cookie_header: Arc::from(h),
        })),
        None => Ok(None),
    }
}

/// Flip one cookie offline. Called from workers on HTTP 401/403. The
/// reason gets surfaced in the dashboard so an operator knows whether
/// to re-login a specific account.
///
/// Reason is truncated to 500 chars on the way in — the column is
/// untyped TEXT but a runaway upstream message shouldn't pollute the
/// row beyond what's diagnostic.
pub async fn mark_offline(
    pool: &DbPool,
    profile_id: Uuid,
    reason: &str,
) -> Result<(), String> {
    let trimmed: String = reason.chars().take(500).collect();
    sqlx::query(
        r#"UPDATE platform_kol_cookies
           SET is_online = FALSE,
               offline_reason = $1,
               last_offline_at = NOW()
           WHERE profile_id = $2
             AND platform = $3
             AND domain = $4"#,
    )
    .bind(&trimmed)
    .bind(profile_id)
    .bind(PLATFORM)
    .bind(DOMAIN)
    .execute(pool)
    .await
    .map_err(|e| format!("mark offline: {e}"))?;
    tracing::warn!(
        "tomato_cookie: marked profile {profile_id} offline ({trimmed})"
    );
    Ok(())
}

/// HTTP statuses we treat as "this cookie is dead, skip it next round".
/// 401/403 are unambiguous auth failures. 5xx and timeouts are upstream
/// hiccups, not our cookie's fault — leave online.
pub fn is_auth_failure_status(status: Option<u16>) -> bool {
    matches!(status, Some(401) | Some(403))
}

fn serialize_cookie_header(cookies: &JsonValue) -> Result<Option<String>, String> {
    let arr = cookies
        .as_array()
        .ok_or_else(|| "cookies not an array".to_string())?;
    let pairs: Vec<String> = arr
        .iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?;
            let value = c.get("value")?.as_str()?;
            if name.is_empty() {
                return None;
            }
            Some(format!("{name}={value}"))
        })
        .collect();
    if pairs.is_empty() {
        return Ok(None);
    }
    Ok(Some(pairs.join("; ")))
}

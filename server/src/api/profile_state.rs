use actix_web::{web, HttpResponse};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};

/// Stored-and-returned state snapshot. `cookies` and `local_storage` are
/// independently optional so the client can push just one without
/// clobbering the other. `local_storage` travels as base64 on the wire
/// (actix-web's JsonConfig doesn't like raw binary in JSON).
#[derive(Debug, Serialize)]
pub struct ProfileStateResponse {
    pub cookies: Option<JsonValue>,
    pub cookies_updated_at: Option<DateTime<Local>>,
    pub local_storage_b64: Option<String>,
    pub local_storage_updated_at: Option<DateTime<Local>>,
    /// Chromium per-profile cookie encryption key (contents of the
    /// `os_crypt_key` file). Plaintext on the wire; must be restored
    /// to the target profile dir BEFORE cookies are injected, otherwise
    /// Chromium will regenerate a different key and fail to decrypt.
    pub os_crypt_key: Option<String>,
    pub os_crypt_key_updated_at: Option<DateTime<Local>>,
}

#[derive(Debug, FromRow)]
struct ProfileStateRow {
    cookies: Option<JsonValue>,
    cookies_updated_at: Option<DateTime<Local>>,
    local_storage: Option<Vec<u8>>,
    local_storage_updated_at: Option<DateTime<Local>>,
    os_crypt_key: Option<String>,
    os_crypt_key_updated_at: Option<DateTime<Local>>,
}

#[derive(Debug, Deserialize)]
pub struct PutProfileStateRequest {
    /// Full cookie list (structured). `None` means don't touch cookies.
    pub cookies: Option<JsonValue>,
    /// Base64-encoded tar.gz of the browser's local-storage directory.
    /// `None` means don't touch local_storage.
    pub local_storage_b64: Option<String>,
    /// Chromium cookie encryption key (os_crypt_key file contents).
    /// `None` means don't touch.
    pub os_crypt_key: Option<String>,
}

/// Confirm the profile belongs to the current user. Returns NotFound
/// (rather than Forbidden) to avoid leaking ownership information.
async fn ensure_owner(
    pool: &DbPool,
    profile_id: Uuid,
    user_id: i32,
) -> AppResult<()> {
    let exists: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM browser_profiles WHERE id = $1 AND user_id = $2",
    )
    .bind(profile_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    exists
        .map(|_| ())
        .ok_or_else(|| AppError::NotFound(format!("profile {profile_id}")))
}

pub async fn get(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
) -> AppResult<HttpResponse> {
    let profile_id = path.into_inner();
    ensure_owner(pool.get_ref(), profile_id, user.0.sub).await?;

    let row: Option<ProfileStateRow> = sqlx::query_as(
        r#"SELECT cookies, cookies_updated_at,
                  local_storage, local_storage_updated_at,
                  os_crypt_key, os_crypt_key_updated_at
           FROM profile_state WHERE profile_id = $1"#,
    )
    .bind(profile_id)
    .fetch_optional(pool.get_ref())
    .await?;

    let response = match row {
        Some(r) => ProfileStateResponse {
            cookies: r.cookies,
            cookies_updated_at: r.cookies_updated_at,
            local_storage_b64: r
                .local_storage
                .as_ref()
                .map(|bytes| base64_encode(bytes)),
            local_storage_updated_at: r.local_storage_updated_at,
            os_crypt_key: r.os_crypt_key,
            os_crypt_key_updated_at: r.os_crypt_key_updated_at,
        },
        None => ProfileStateResponse {
            cookies: None,
            cookies_updated_at: None,
            local_storage_b64: None,
            local_storage_updated_at: None,
            os_crypt_key: None,
            os_crypt_key_updated_at: None,
        },
    };
    Ok(HttpResponse::Ok().json(response))
}

pub async fn put(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
    body: web::Json<PutProfileStateRequest>,
) -> AppResult<HttpResponse> {
    let profile_id = path.into_inner();
    ensure_owner(pool.get_ref(), profile_id, user.0.sub).await?;

    let b = body.into_inner();
    if b.cookies.is_none() && b.local_storage_b64.is_none() && b.os_crypt_key.is_none() {
        return Err(AppError::BadRequest("nothing to update".into()));
    }

    let local_storage_bytes = match b.local_storage_b64 {
        Some(s) => Some(base64_decode(&s).map_err(AppError::BadRequest)?),
        None => None,
    };

    let now = Local::now();

    sqlx::query(
        r#"INSERT INTO profile_state (
              profile_id, cookies, cookies_updated_at,
              local_storage, local_storage_updated_at,
              os_crypt_key, os_crypt_key_updated_at
           ) VALUES ($1, $2, $3, $4, $5, $6, $7)
           ON CONFLICT (profile_id) DO UPDATE SET
              cookies                  = COALESCE(EXCLUDED.cookies, profile_state.cookies),
              cookies_updated_at       = COALESCE(EXCLUDED.cookies_updated_at, profile_state.cookies_updated_at),
              local_storage            = COALESCE(EXCLUDED.local_storage, profile_state.local_storage),
              local_storage_updated_at = COALESCE(EXCLUDED.local_storage_updated_at, profile_state.local_storage_updated_at),
              os_crypt_key             = COALESCE(EXCLUDED.os_crypt_key, profile_state.os_crypt_key),
              os_crypt_key_updated_at  = COALESCE(EXCLUDED.os_crypt_key_updated_at, profile_state.os_crypt_key_updated_at)"#,
    )
    .bind(profile_id)
    .bind(&b.cookies)
    .bind(b.cookies.as_ref().map(|_| now))
    .bind(&local_storage_bytes)
    .bind(local_storage_bytes.as_ref().map(|_| now))
    .bind(&b.os_crypt_key)
    .bind(b.os_crypt_key.as_ref().map(|_| now))
    .execute(pool.get_ref())
    .await?;

    // After upserting the full snapshot, extract per-platform cookie
    // subsets (e.g. tomato → kol.fanqieopen.com) into a side table for
    // downstream automation.
    if let Some(cookies) = b.cookies.as_ref() {
        if let Err(e) = extract_platform_cookies(pool.get_ref(), profile_id, cookies).await {
            tracing::warn!(
                profile = %profile_id,
                "platform cookie extraction failed: {e}"
            );
        }
    }

    Ok(HttpResponse::NoContent().finish())
}

/// Mapping from `kol_platform` value to the cookie target domains we
/// care about. Cookies for these domains get copied into
/// `platform_kol_cookies` for downstream consumers (server-side
/// automation, dashboards, etc.) to pick up without poking at the full
/// state blob.
fn platform_target_domains(platform: &str) -> &'static [&'static str] {
    match platform {
        "tomato" => &["kol.fanqieopen.com"],
        // 七猫达人: API requests go to kol.wtzw.com, but the login flow
        // (and the source of `x-qm-devops-token`) lives on dmp.wtzw.com.
        // Cookies for both domains can be relevant — extract both, the
        // worker will only send what kol.wtzw.com asks for.
        "qimao" => &["kol.wtzw.com", "dmp.wtzw.com"],
        // douyin TBD — add target hostnames here when known.
        _ => &[],
    }
}

/// True if the cookie's `domain` field would cause the cookie to be sent
/// to a request to `target`. Handles the `.example.com` wildcard form
/// in addition to exact match.
fn cookie_applies_to(cookie_domain: &str, target: &str) -> bool {
    let d = cookie_domain.trim_start_matches('.');
    target == d || target.ends_with(&format!(".{d}"))
}

async fn extract_platform_cookies(
    pool: &DbPool,
    profile_id: Uuid,
    cookies: &JsonValue,
) -> Result<(), String> {
    let row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT kol_platform FROM browser_profiles WHERE id = $1")
            .bind(profile_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("lookup platform: {e}"))?;

    let platform = match row.and_then(|r| r.0) {
        Some(p) if !p.is_empty() => p,
        _ => return Ok(()), // no platform set on this profile
    };

    let targets = platform_target_domains(&platform);
    if targets.is_empty() {
        return Ok(());
    }

    let cookie_array = cookies
        .as_array()
        .ok_or_else(|| "cookies is not an array".to_string())?;

    for target in targets {
        let matched: Vec<&JsonValue> = cookie_array
            .iter()
            .filter(|c| {
                c.get("domain")
                    .and_then(JsonValue::as_str)
                    .is_some_and(|d| cookie_applies_to(d, target))
            })
            .collect();

        let payload = serde_json::Value::Array(matched.iter().map(|c| (*c).clone()).collect());

        // A fresh upsert means the user just re-pushed their browser
        // state — implicitly a fresh login. Flip `is_online` back on,
        // clear the prior offline diagnostic AND clear the notified-at
        // flag so the next time this cookie goes offline a fresh
        // email goes out.
        sqlx::query(
            r#"INSERT INTO platform_kol_cookies (profile_id, platform, domain, cookies)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (profile_id, platform, domain)
               DO UPDATE SET cookies = EXCLUDED.cookies,
                             is_online = TRUE,
                             offline_reason = NULL,
                             last_offline_at = NULL,
                             offline_notified_at = NULL"#,
        )
        .bind(profile_id)
        .bind(&platform)
        .bind(*target)
        .bind(&payload)
        .execute(pool)
        .await
        .map_err(|e| format!("upsert {target}: {e}"))?;
    }
    Ok(())
}

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| format!("invalid base64: {e}"))
}

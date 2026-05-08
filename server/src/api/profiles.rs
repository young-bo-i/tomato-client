use actix_web::{web, HttpResponse};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::auth::AuthUser;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::models::browser_profile::{
    BrowserProfile, CreateProfileRequest, UpdateProfileRequest,
};

pub async fn list(pool: web::Data<DbPool>, user: AuthUser) -> AppResult<HttpResponse> {
    let rows = sqlx::query_as::<_, BrowserProfile>(
        r#"SELECT id, user_id, name, browser, version, release_type, proxy_id, vpn_id,
                  group_id, extension_group_id, tags, note, camoufox_config,
                  wayfern_config, sync_mode, encryption_salt, last_sync, last_launch,
                  host_os, ephemeral, proxy_bypass_rules, created_by_id,
                  created_by_email, dns_blocklist, kol_platform,
                  qimao_identifier, qimao_credential, qimao_token,
                  qimao_token_refreshed_at, qimao_token_last_error,
                  created_at, updated_at
           FROM browser_profiles
           WHERE user_id = $1
           ORDER BY created_at ASC"#,
    )
    .bind(user.0.sub)
    .fetch_all(pool.get_ref())
    .await?;
    Ok(HttpResponse::Ok().json(rows))
}

pub async fn get(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let row = sqlx::query_as::<_, BrowserProfile>(
        r#"SELECT id, user_id, name, browser, version, release_type, proxy_id, vpn_id,
                  group_id, extension_group_id, tags, note, camoufox_config,
                  wayfern_config, sync_mode, encryption_salt, last_sync, last_launch,
                  host_os, ephemeral, proxy_bypass_rules, created_by_id,
                  created_by_email, dns_blocklist, kol_platform,
                  qimao_identifier, qimao_credential, qimao_token,
                  qimao_token_refreshed_at, qimao_token_last_error,
                  created_at, updated_at
           FROM browser_profiles
           WHERE id = $1 AND user_id = $2"#,
    )
    .bind(id)
    .bind(user.0.sub)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound(format!("profile {id}")))?;
    Ok(HttpResponse::Ok().json(row))
}

pub async fn create(
    pool: web::Data<DbPool>,
    user: AuthUser,
    body: web::Json<CreateProfileRequest>,
) -> AppResult<HttpResponse> {
    let b = body.into_inner();
    // For douyin profiles we optimistically default `douyin_login_state`
    // to `authenticated` at creation time. Rationale: the user just
    // logged in inside the browser and synced the profile to the
    // server; assuming online keeps the dashboard from showing a
    // misleading "未登录" badge until the collection extension fires.
    // The collection extension will overwrite this with the real state
    // on first run (push_douyin_state from kol-ext via /douyin_state).
    let result = sqlx::query_as::<_, BrowserProfile>(
        r#"INSERT INTO browser_profiles (
              id, user_id, name, browser, version, release_type, proxy_id, vpn_id,
              group_id, extension_group_id, tags, note, camoufox_config,
              wayfern_config, sync_mode, encryption_salt, last_sync, last_launch,
              host_os, ephemeral, proxy_bypass_rules, created_by_id,
              created_by_email, dns_blocklist, kol_platform,
              qimao_identifier, qimao_credential,
              douyin_login_state, douyin_login_state_updated_at
           ) VALUES (
              $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15,
              $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27,
              CASE WHEN $25 = 'douyin' THEN 'authenticated' ELSE NULL END,
              CASE WHEN $25 = 'douyin' THEN NOW()             ELSE NULL END
           )
           RETURNING id, user_id, name, browser, version, release_type, proxy_id, vpn_id,
                    group_id, extension_group_id, tags, note, camoufox_config,
                    wayfern_config, sync_mode, encryption_salt, last_sync, last_launch,
                    host_os, ephemeral, proxy_bypass_rules, created_by_id,
                    created_by_email, dns_blocklist, kol_platform,
                  qimao_identifier, qimao_credential, qimao_token,
                  qimao_token_refreshed_at, qimao_token_last_error,
                  created_at, updated_at"#,
    )
    .bind(b.id)
    .bind(user.0.sub)
    .bind(&b.name)
    .bind(&b.browser)
    .bind(&b.version)
    .bind(&b.release_type)
    .bind(&b.proxy_id)
    .bind(&b.vpn_id)
    .bind(&b.group_id)
    .bind(&b.extension_group_id)
    .bind(&b.tags)
    .bind(&b.note)
    .bind(&b.camoufox_config)
    .bind(&b.wayfern_config)
    .bind(&b.sync_mode)
    .bind(&b.encryption_salt)
    .bind(b.last_sync)
    .bind(b.last_launch)
    .bind(&b.host_os)
    .bind(b.ephemeral)
    .bind(&b.proxy_bypass_rules)
    .bind(&b.created_by_id)
    .bind(&b.created_by_email)
    .bind(&b.dns_blocklist)
    .bind(&b.kol_platform)
    .bind(&b.qimao_identifier)
    .bind(&b.qimao_credential)
    .fetch_one(pool.get_ref())
    .await;

    match result {
        Ok(row) => {
            // Seed default submission config (enabled=true, daily_limit=300)
            // for each (platform, alias_type) relevant to this profile.
            seed_default_submission_config(pool.get_ref(), row.id, row.kol_platform.as_deref())
                .await;
            crate::services::cache::invalidate_submission_config();
            Ok(HttpResponse::Created().json(row))
        }
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(AppError::Conflict(
            format!("profile {} already exists", b.id),
        )),
        Err(e) => Err(AppError::Database(e)),
    }
}

/// Insert per-profile kol_submission_config rows for a newly created
/// profile, using `kol_submission_config_defaults` (admin-managed) as
/// the initial values. Uses ON CONFLICT DO NOTHING so re-creation
/// (e.g. import) is idempotent — existing per-profile rows win, the
/// admin's defaults table only seeds rows that don't already exist.
///
/// Platform mismatches (e.g. defaults table has alias_type 1/2/6 for
/// tomato; we copy whatever's there). Filter by platform so changing
/// kol_platform from douyin to tomato later only seeds tomato rows.
async fn seed_default_submission_config(
    pool: &crate::db::DbPool,
    profile_id: uuid::Uuid,
    kol_platform: Option<&str>,
) {
    let platform = match kol_platform {
        Some(p @ ("tomato" | "qimao")) => p,
        _ => return, // douyin / none — no submission config needed
    };

    if let Err(e) = sqlx::query(
        r#"INSERT INTO kol_submission_config
               (profile_id, platform, alias_type, enabled, daily_limit)
           SELECT $1, platform, alias_type, enabled, daily_limit
             FROM kol_submission_config_defaults
            WHERE platform = $2
           ON CONFLICT (profile_id, platform, alias_type) DO NOTHING"#,
    )
    .bind(profile_id)
    .bind(platform)
    .execute(pool)
    .await
    {
        tracing::warn!("seed_default_submission_config {profile_id}: {e}");
    }
}

/// Partial update. Each field in the body is optional; only supplied fields
/// are touched. Double-Option (e.g. `Option<Option<String>>`) distinguishes
/// "unset to null" (explicit `null`) from "leave alone" (field omitted).
pub async fn update(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
    body: web::Json<UpdateProfileRequest>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let b = body.into_inner();

    // sqlx::QueryBuilder is the cleanest way to build a dynamic UPDATE
    // without a heterogeneous parameter array.
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new("UPDATE browser_profiles SET ");
    let mut first = true;
    let mut add_separator = |qb: &mut sqlx::QueryBuilder<sqlx::Postgres>, first: &mut bool| {
        if *first {
            *first = false;
        } else {
            qb.push(", ");
        }
    };

    if let Some(v) = &b.name {
        add_separator(&mut qb, &mut first);
        qb.push("name = ").push_bind(v.clone());
    }
    if let Some(v) = &b.version {
        add_separator(&mut qb, &mut first);
        qb.push("version = ").push_bind(v.clone());
    }
    if let Some(v) = &b.release_type {
        add_separator(&mut qb, &mut first);
        qb.push("release_type = ").push_bind(v.clone());
    }
    if let Some(v) = &b.proxy_id {
        add_separator(&mut qb, &mut first);
        qb.push("proxy_id = ").push_bind(v.clone());
    }
    if let Some(v) = &b.vpn_id {
        add_separator(&mut qb, &mut first);
        qb.push("vpn_id = ").push_bind(v.clone());
    }
    if let Some(v) = &b.group_id {
        add_separator(&mut qb, &mut first);
        qb.push("group_id = ").push_bind(v.clone());
    }
    if let Some(v) = &b.extension_group_id {
        add_separator(&mut qb, &mut first);
        qb.push("extension_group_id = ").push_bind(v.clone());
    }
    if let Some(v) = &b.tags {
        add_separator(&mut qb, &mut first);
        qb.push("tags = ").push_bind(v.clone());
    }
    if let Some(v) = &b.note {
        add_separator(&mut qb, &mut first);
        qb.push("note = ").push_bind(v.clone());
    }
    if let Some(v) = &b.camoufox_config {
        add_separator(&mut qb, &mut first);
        qb.push("camoufox_config = ").push_bind(v.clone());
    }
    if let Some(v) = &b.wayfern_config {
        add_separator(&mut qb, &mut first);
        qb.push("wayfern_config = ").push_bind(v.clone());
    }
    if let Some(v) = &b.sync_mode {
        add_separator(&mut qb, &mut first);
        qb.push("sync_mode = ").push_bind(v.clone());
    }
    if let Some(v) = &b.encryption_salt {
        add_separator(&mut qb, &mut first);
        qb.push("encryption_salt = ").push_bind(v.clone());
    }
    if let Some(v) = &b.last_sync {
        add_separator(&mut qb, &mut first);
        qb.push("last_sync = ").push_bind(*v);
    }
    if let Some(v) = &b.last_launch {
        add_separator(&mut qb, &mut first);
        qb.push("last_launch = ").push_bind(*v);
    }
    if let Some(v) = &b.host_os {
        add_separator(&mut qb, &mut first);
        qb.push("host_os = ").push_bind(v.clone());
    }
    if let Some(v) = b.ephemeral {
        add_separator(&mut qb, &mut first);
        qb.push("ephemeral = ").push_bind(v);
    }
    if let Some(v) = &b.proxy_bypass_rules {
        add_separator(&mut qb, &mut first);
        qb.push("proxy_bypass_rules = ").push_bind(v.clone());
    }
    if let Some(v) = &b.dns_blocklist {
        add_separator(&mut qb, &mut first);
        qb.push("dns_blocklist = ").push_bind(v.clone());
    }
    if let Some(v) = &b.kol_platform {
        add_separator(&mut qb, &mut first);
        qb.push("kol_platform = ").push_bind(v.clone());
    }
    if let Some(v) = &b.qimao_identifier {
        add_separator(&mut qb, &mut first);
        qb.push("qimao_identifier = ").push_bind(v.clone());
    }
    // Only clear the server-managed token when the credential actually
    // changes. The client sends a full-object PATCH on every sync even
    // when nothing changed, so checking for equality prevents a spurious
    // token invalidation on every profile save.
    // SQL SET expressions evaluate the OLD column value (before the
    // update), so `qimao_credential = $new` in the CASE compares against
    // the pre-update row — safe and atomic.
    if let Some(v) = &b.qimao_credential {
        add_separator(&mut qb, &mut first);
        qb.push("qimao_credential = ").push_bind(v.clone());
        qb.push(", qimao_token = CASE WHEN qimao_credential = ")
            .push_bind(v.clone())
            .push(" THEN qimao_token ELSE NULL END");
        qb.push(", qimao_token_refreshed_at = CASE WHEN qimao_credential = ")
            .push_bind(v.clone())
            .push(" THEN qimao_token_refreshed_at ELSE NULL END");
        qb.push(", qimao_token_last_error = CASE WHEN qimao_credential = ")
            .push_bind(v.clone())
            .push(" THEN qimao_token_last_error ELSE NULL END");
    }

    if first {
        return Err(AppError::BadRequest("no fields to update".into()));
    }

    qb.push(" WHERE id = ").push_bind(id);
    qb.push(" AND user_id = ").push_bind(user.0.sub);
    qb.push(
        " RETURNING id, user_id, name, browser, version, release_type, proxy_id, vpn_id,
                    group_id, extension_group_id, tags, note, camoufox_config,
                    wayfern_config, sync_mode, encryption_salt, last_sync, last_launch,
                    host_os, ephemeral, proxy_bypass_rules, created_by_id,
                    created_by_email, dns_blocklist, kol_platform,
                  qimao_identifier, qimao_credential, qimao_token,
                  qimao_token_refreshed_at, qimao_token_last_error,
                  created_at, updated_at",
    );

    let row = qb
        .build_query_as::<BrowserProfile>()
        .fetch_optional(pool.get_ref())
        .await?
        .ok_or_else(|| AppError::NotFound(format!("profile {id}")))?;

    // If kol_platform was just set/changed to tomato/qimao, seed the
    // per-profile rows from defaults. ON CONFLICT DO NOTHING preserves
    // any existing rows (e.g. when user toggles tomato → douyin → tomato,
    // their previously-edited config is kept).
    if b.kol_platform.is_some() {
        seed_default_submission_config(pool.get_ref(), row.id, row.kol_platform.as_deref()).await;
        crate::services::cache::invalidate_submission_config();
    }

    Ok(HttpResponse::Ok().json(row))
}

pub async fn delete(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let res = sqlx::query("DELETE FROM browser_profiles WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user.0.sub)
        .execute(pool.get_ref())
        .await?;
    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("profile {id}")));
    }
    // kol_submission_config has ON DELETE CASCADE, so the config rows are
    // already gone. Invalidate the cache so next enqueue doesn't see stale data.
    crate::services::cache::invalidate_submission_config();
    Ok(HttpResponse::NoContent().finish())
}

#[derive(Debug, Deserialize)]
pub struct DouyinStatePayload {
    /// Mirrors the strings reported by content.js:
    ///   "authenticated" / "unauthenticated" / "unknown"
    pub state: String,
    #[serde(default)]
    pub url: Option<String>,
}

/// `POST /api/profiles/{id}/douyin_state` — Tauri client forwards the
/// browser-extension's login-state pings here. Server stamps the row
/// and clears the offline_notified_at flag on transition back to
/// `authenticated` so the next "unauthenticated" event triggers a
/// fresh email. The dispatcher worker is the consumer of these
/// timestamps.
///
/// Auth: any logged-in user can update profiles they own. Cross-user
/// updates are rejected via the WHERE user_id check.
pub async fn set_douyin_state(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
    body: web::Json<DouyinStatePayload>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let state = body.state.trim().to_string();
    if !matches!(
        state.as_str(),
        "authenticated" | "unauthenticated" | "unknown"
    ) {
        return Err(AppError::BadRequest(format!(
            "invalid state: {state} (expect authenticated/unauthenticated/unknown)"
        )));
    }

    // On transition back to `authenticated`, clear the
    // notified-at flag so the next disconnection retriggers a fresh
    // notification. For unauthenticated/unknown we leave the flag as-is —
    // it's only set on send and only cleared on recovery.
    let res = sqlx::query(
        r#"UPDATE browser_profiles
           SET douyin_login_state            = $1,
               douyin_login_state_updated_at = NOW(),
               douyin_login_state_url        = $2,
               douyin_offline_notified_at    = CASE
                   WHEN $1 = 'authenticated' THEN NULL
                   ELSE douyin_offline_notified_at
               END
           WHERE id = $3 AND user_id = $4"#,
    )
    .bind(&state)
    .bind(&body.url)
    .bind(id)
    .bind(user.0.sub)
    .execute(pool.get_ref())
    .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::NotFound(format!("profile {id}")));
    }
    Ok(HttpResponse::Ok().json(json!({ "ok": true })))
}

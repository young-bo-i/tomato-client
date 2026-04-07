use actix_web::{web, HttpResponse};
use uuid::Uuid;
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::middleware::auth::UserId;
use crate::models::profile::*;

pub async fn create_profile(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<CreateProfileRequest>,
) -> AppResult<HttpResponse> {
    let id = Uuid::new_v4();
    let browser_type = body.browser_type.as_deref().unwrap_or("chromium");
    let fingerprint = body.fingerprint_config.clone().unwrap_or(serde_json::json!({}));

    sqlx::query(
        r#"INSERT INTO browser_profile (id, account_id, name, browser_type, fingerprint_config, proxy_config)
         VALUES ($1, $2, $3, $4, $5, $6)"#,
    )
    .bind(id)
    .bind(user.0)
    .bind(&body.name)
    .bind(browser_type)
    .bind(&fingerprint)
    .bind(&body.proxy_config)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": { "id": id }
    })))
}

pub async fn list_profiles(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let profiles = sqlx::query_as::<_, BrowserProfile>(
        "SELECT * FROM browser_profile WHERE account_id = $1 AND is_deleted = FALSE ORDER BY created_at DESC",
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": profiles,
    })))
}

pub async fn get_profile(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<Uuid>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let profile = sqlx::query_as::<_, BrowserProfile>(
        "SELECT * FROM browser_profile WHERE id = $1 AND account_id = $2 AND is_deleted = FALSE",
    )
    .bind(id)
    .bind(user.0)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound("Profile not found".into()))?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": profile,
    })))
}

pub async fn update_profile(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<Uuid>,
    body: web::Json<UpdateProfileRequest>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();

    if let Some(ref name) = body.name {
        sqlx::query(
            "UPDATE browser_profile SET name = $1, updated_at = NOW() WHERE id = $2 AND account_id = $3",
        )
        .bind(name)
        .bind(id)
        .bind(user.0)
        .execute(pool.get_ref())
        .await?;
    }
    if let Some(ref fp) = body.fingerprint_config {
        sqlx::query(
            "UPDATE browser_profile SET fingerprint_config = $1, updated_at = NOW() WHERE id = $2 AND account_id = $3",
        )
        .bind(fp)
        .bind(id)
        .bind(user.0)
        .execute(pool.get_ref())
        .await?;
    }
    if let Some(ref proxy) = body.proxy_config {
        sqlx::query(
            "UPDATE browser_profile SET proxy_config = $1, updated_at = NOW() WHERE id = $2 AND account_id = $3",
        )
        .bind(proxy)
        .bind(id)
        .bind(user.0)
        .execute(pool.get_ref())
        .await?;
    }
    if let Some(ref meta) = body.metadata {
        sqlx::query(
            "UPDATE browser_profile SET metadata = $1, updated_at = NOW() WHERE id = $2 AND account_id = $3",
        )
        .bind(meta)
        .bind(id)
        .bind(user.0)
        .execute(pool.get_ref())
        .await?;
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn delete_profile(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<Uuid>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    sqlx::query(
        "UPDATE browser_profile SET is_deleted = TRUE, updated_at = NOW() WHERE id = $1 AND account_id = $2",
    )
    .bind(id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

/// Upload profile data archive (cookies, localStorage, fingerprint data)
pub async fn sync_upload(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<Uuid>,
    body: actix_web::web::Bytes,
) -> AppResult<HttpResponse> {
    let profile_id = path.into_inner();

    // Verify ownership
    let exists = sqlx::query(
        "SELECT id FROM browser_profile WHERE id = $1 AND account_id = $2 AND is_deleted = FALSE",
    )
    .bind(profile_id)
    .bind(user.0)
    .fetch_optional(pool.get_ref())
    .await?;

    if exists.is_none() {
        return Err(AppError::NotFound("Profile not found".into()));
    }

    // Store archive data - in production, store in S3/MinIO
    let hash = format!("{:x}", md5_hash(&body));
    let storage_path = format!("profiles/{}/{}.tar.zst", user.0, profile_id);

    // TODO: Write body to object storage (S3/MinIO)
    // For now, record the archive metadata
    sqlx::query(
        r#"INSERT INTO profile_archive (profile_id, file_hash, file_size, storage_path)
         VALUES ($1, $2, $3, $4)"#,
    )
    .bind(profile_id)
    .bind(&hash)
    .bind(body.len() as i64)
    .bind(&storage_path)
    .execute(pool.get_ref())
    .await?;

    sqlx::query(
        "UPDATE browser_profile SET last_sync_at = NOW(), updated_at = NOW() WHERE id = $1",
    )
    .bind(profile_id)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": { "hash": hash, "size": body.len() }
    })))
}

pub async fn sync_download(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<Uuid>,
) -> AppResult<HttpResponse> {
    let profile_id = path.into_inner();

    let archive = sqlx::query_as::<_, ProfileArchive>(
        r#"SELECT pa.* FROM profile_archive pa
         JOIN browser_profile bp ON bp.id = pa.profile_id
         WHERE pa.profile_id = $1 AND bp.account_id = $2 AND bp.is_deleted = FALSE
         ORDER BY pa.created_at DESC LIMIT 1"#,
    )
    .bind(profile_id)
    .bind(user.0)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound("No sync data found".into()))?;

    // TODO: Read from object storage and stream response
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": {
            "storage_path": archive.storage_path,
            "file_hash": archive.file_hash,
            "file_size": archive.file_size,
        }
    })))
}

pub async fn sync_status(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<Uuid>,
) -> AppResult<HttpResponse> {
    let profile_id = path.into_inner();

    let profile = sqlx::query_as::<_, BrowserProfile>(
        "SELECT * FROM browser_profile WHERE id = $1 AND account_id = $2 AND is_deleted = FALSE",
    )
    .bind(profile_id)
    .bind(user.0)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound("Profile not found".into()))?;

    let archive = sqlx::query_as::<_, ProfileArchive>(
        "SELECT * FROM profile_archive WHERE profile_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(profile_id)
    .fetch_optional(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": ProfileSyncStatus {
            profile_id: profile.id,
            last_sync_at: profile.last_sync_at,
            file_hash: archive.as_ref().map(|a| a.file_hash.clone()),
            file_size: archive.as_ref().map(|a| a.file_size),
        }
    })))
}

fn md5_hash(data: &[u8]) -> u128 {
    // Simple hash for file deduplication (not crypto)
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    data.hash(&mut hasher);
    hasher.finish() as u128
}

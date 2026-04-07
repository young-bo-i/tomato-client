use actix_web::{web, HttpResponse};
use crate::db::DbPool;
use crate::errors::AppResult;
use crate::middleware::auth::UserId;
use crate::models::common_setting::*;
use sqlx::Row;

pub async fn get_all_settings(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let settings = sqlx::query_as::<_, crate::models::common_setting::CommonSetting>(
        "SELECT * FROM common_setting WHERE account_id = $1 AND is_deleted = FALSE",
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": settings,
    })))
}

pub async fn save_platform_types(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<SavePlatformRequest>,
) -> AppResult<HttpResponse> {
    let value = serde_json::to_string(&body.open_types)
        .map_err(|e| crate::errors::AppError::Internal(e.to_string()))?;

    sqlx::query(
        r#"INSERT INTO common_setting (account_id, kol_id, scene, setting_value)
         VALUES ($1, $2, 'OpenBrushPlatform', $3)
         ON CONFLICT (id) DO UPDATE SET setting_value = $3, updated_at = NOW()"#,
    )
    .bind(user.0)
    .bind(body.kol_id)
    .bind(&value)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn save_type_limit(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<SaveLimitRequest>,
) -> AppResult<HttpResponse> {
    let value = serde_json::json!({
        "platform": body.platform,
        "limit": body.limit,
    }).to_string();

    sqlx::query(
        r#"INSERT INTO common_setting (account_id, kol_id, scene, setting_value)
         VALUES ($1, $2, 'BrushLimit', $3)
         ON CONFLICT (id) DO UPDATE SET setting_value = $3, updated_at = NOW()"#,
    )
    .bind(user.0)
    .bind(body.kol_id)
    .bind(&value)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn get_dom_config(
    pool: web::Data<DbPool>,
    _user: UserId,
    path: web::Path<String>,
) -> AppResult<HttpResponse> {
    let dom_type = path.into_inner();
    let config = sqlx::query(
        "SELECT selectors FROM dom_config WHERE dom_type = $1",
    )
    .bind(&dom_type)
    .fetch_optional(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": config.map(|c| c.get::<serde_json::Value, _>("selectors")),
    })))
}

pub async fn update_dom_config(
    pool: web::Data<DbPool>,
    _user: UserId,
    body: web::Json<UpdateDomRequest>,
) -> AppResult<HttpResponse> {
    sqlx::query(
        r#"INSERT INTO dom_config (dom_type, selectors)
         VALUES ($1, $2)
         ON CONFLICT (dom_type) DO UPDATE SET selectors = $2, updated_at = NOW()"#,
    )
    .bind(&body.dom_type)
    .bind(&body.selectors)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn get_income_notice(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let notices = sqlx::query_as::<_, crate::models::income::IncomeNotice>(
        "SELECT * FROM income_notice WHERE account_id = $1 AND is_deleted = FALSE",
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": notices,
    })))
}

pub async fn set_income_notice(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<IncomeNoticeSettingRequest>,
) -> AppResult<HttpResponse> {
    // Soft delete existing entries
    sqlx::query(
        "UPDATE income_notice SET is_deleted = TRUE WHERE account_id = $1",
    )
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    // Insert new entries
    for email in &body.emails {
        sqlx::query(
            "INSERT INTO income_notice (account_id, email, has_child) VALUES ($1, $2, $3)",
        )
        .bind(user.0)
        .bind(email)
        .bind(body.has_child)
        .execute(pool.get_ref())
        .await?;
    }

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn add_notice_email(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<serde_json::Value>,
) -> AppResult<HttpResponse> {
    let email = body.get("email").and_then(|v| v.as_str()).unwrap_or("");

    sqlx::query(
        "INSERT INTO income_notice (account_id, email, has_child) VALUES ($1, $2, FALSE)",
    )
    .bind(user.0)
    .bind(email)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn set_notice_has_child(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<serde_json::Value>,
) -> AppResult<HttpResponse> {
    let has_child = body.get("has_child").and_then(|v| v.as_bool()).unwrap_or(false);

    sqlx::query(
        "UPDATE income_notice SET has_child = $1 WHERE account_id = $2 AND is_deleted = FALSE",
    )
    .bind(has_child)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn get_third_party_limit(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let kol_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM kol_account WHERE account_id = $1 AND is_deleted = FALSE"
    )
    .bind(user.0)
    .fetch_one(pool.get_ref())
    .await?;

    let douyin_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM douyin_account WHERE account_id = $1 AND is_deleted = FALSE"
    )
    .bind(user.0)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": {
            "kol_count": kol_count.0,
            "douyin_count": douyin_count.0,
        }
    })))
}

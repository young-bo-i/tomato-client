use actix_web::{web, HttpResponse};
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::middleware::auth::UserId;
use crate::models::douyin_account::*;

pub async fn submit_storage_state(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<SubmitDouYinRequest>,
) -> AppResult<HttpResponse> {
    let id: (i32,) = sqlx::query_as(
        r#"INSERT INTO douyin_account (account_id, storage_state, nickname, remark)
         VALUES ($1, $2, $3, $4) RETURNING id"#,
    )
    .bind(user.0)
    .bind(&body.storage_state)
    .bind(&body.nickname)
    .bind(&body.remark)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": { "id": id.0 }
    })))
}

pub async fn update_storage_state(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<UpdateDouYinRequest>,
) -> AppResult<HttpResponse> {
    sqlx::query(
        r#"UPDATE douyin_account SET storage_state = $1, updated_at = NOW()
         WHERE id = $2 AND account_id = $3 AND is_deleted = FALSE"#,
    )
    .bind(&body.storage_state)
    .bind(body.id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn get_accounts(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let accounts = sqlx::query_as::<_, DouYinAccount>(
        r#"SELECT * FROM douyin_account WHERE account_id = $1 AND is_deleted = FALSE"#,
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": accounts,
    })))
}

pub async fn get_base_accounts(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let accounts = sqlx::query_as::<_, DouYinAccountInfo>(
        r#"SELECT id, account_id, nickname, remark, status, created_at
         FROM douyin_account WHERE account_id = $1 AND is_deleted = FALSE"#,
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": accounts,
    })))
}

pub async fn get_by_id(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let account = sqlx::query_as::<_, DouYinAccount>(
        "SELECT * FROM douyin_account WHERE id = $1 AND account_id = $2 AND is_deleted = FALSE",
    )
    .bind(id)
    .bind(user.0)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound("DouYin account not found".into()))?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": account,
    })))
}

pub async fn delete_account(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    sqlx::query(
        "UPDATE douyin_account SET is_deleted = TRUE, updated_at = NOW() WHERE id = $1 AND account_id = $2",
    )
    .bind(id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn set_status(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
    body: web::Json<serde_json::Value>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let status = body.get("status").and_then(|v| v.as_i64()).unwrap_or(1) as i16;

    sqlx::query(
        "UPDATE douyin_account SET status = $1, updated_at = NOW() WHERE id = $2 AND account_id = $3",
    )
    .bind(status)
    .bind(id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn update_remark(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
    body: web::Json<serde_json::Value>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let remark = body.get("remark").and_then(|v| v.as_str()).unwrap_or("");

    sqlx::query(
        "UPDATE douyin_account SET remark = $1, updated_at = NOW() WHERE id = $2 AND account_id = $3",
    )
    .bind(remark)
    .bind(id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

use actix_web::{web, HttpResponse};
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::middleware::auth::UserId;
use crate::models::kol_account::*;

pub async fn submit_cookies(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<SubmitCookiesRequest>,
) -> AppResult<HttpResponse> {
    let id: (i32,) = sqlx::query_as(
        r#"INSERT INTO kol_account (account_id, cookies, uid, identity_name, remark)
         VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(user.0)
    .bind(&body.cookies)
    .bind(&body.uid)
    .bind(&body.identity_name)
    .bind(&body.remark)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": { "id": id.0 }
    })))
}

pub async fn update_cookies(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<UpdateCookiesRequest>,
) -> AppResult<HttpResponse> {
    sqlx::query(
        r#"UPDATE kol_account SET cookies = $1, updated_at = NOW()
         WHERE id = $2 AND account_id = $3 AND is_deleted = FALSE"#,
    )
    .bind(&body.cookies)
    .bind(body.id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn get_kol_accounts(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let accounts = sqlx::query_as::<_, KolAccountFull>(
        r#"SELECT id, account_id, cookies, uid, identity_name, identity_number,
                  payment_account, mobile, remark, status, created_at
         FROM kol_account WHERE account_id = $1 AND is_deleted = FALSE"#,
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": accounts,
    })))
}

pub async fn get_kol_base_infos(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let accounts = sqlx::query_as::<_, KolAccountInfo>(
        r#"SELECT id, account_id, uid, identity_name, remark, status, created_at
         FROM kol_account WHERE account_id = $1 AND is_deleted = FALSE"#,
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": accounts,
    })))
}

pub async fn get_kol_by_id(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    let account = sqlx::query_as::<_, KolAccountFull>(
        r#"SELECT id, account_id, cookies, uid, identity_name, identity_number,
                  payment_account, mobile, remark, status, created_at
         FROM kol_account WHERE id = $1 AND account_id = $2 AND is_deleted = FALSE"#,
    )
    .bind(id)
    .bind(user.0)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound("KOL account not found".into()))?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": account,
    })))
}

pub async fn delete_kol_account(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let id = path.into_inner();
    sqlx::query(
        "UPDATE kol_account SET is_deleted = TRUE, updated_at = NOW() WHERE id = $1 AND account_id = $2",
    )
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
        "UPDATE kol_account SET remark = $1, updated_at = NOW() WHERE id = $2 AND account_id = $3",
    )
    .bind(remark)
    .bind(id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn get_invite_codes(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let codes = sqlx::query_as::<_, crate::models::invite_code::KolInviteCode>(
        r#"SELECT * FROM kol_invite_code
         WHERE account_id = $1 AND is_deleted = FALSE"#,
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": codes,
    })))
}

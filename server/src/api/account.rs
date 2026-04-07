use actix_web::{web, HttpResponse};
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::middleware::auth::{JwtConfig, UserId};
use crate::models::account::{AccountInfo, CreateAccountRequest};

pub async fn get_account_info(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let account = sqlx::query_as::<_, AccountInfo>(
        r#"SELECT id, account_name, phone, email, status, parent_id, created_at
         FROM account WHERE id = $1 AND is_deleted = FALSE"#,
    )
    .bind(user.0)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::NotFound("Account not found".into()))?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": account,
    })))
}

pub async fn create_sub_account(
    pool: web::Data<DbPool>,
    user: UserId,
    body: web::Json<CreateAccountRequest>,
) -> AppResult<HttpResponse> {
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    use argon2::password_hash::rand_core::OsRng;

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| AppError::Internal(e.to_string()))?
        .to_string();

    let id: (i32,) = sqlx::query_as(
        r#"INSERT INTO account (account_name, password_hash, phone, email, parent_id)
         VALUES ($1, $2, $3, $4, $5) RETURNING id"#,
    )
    .bind(&body.account_name)
    .bind(&password_hash)
    .bind(&body.phone)
    .bind(&body.email)
    .bind(user.0)
    .fetch_one(pool.get_ref())
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(ref db_err) if db_err.is_unique_violation() => {
            AppError::BadRequest("Account name already exists".into())
        }
        _ => AppError::Database(e),
    })?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": { "id": id.0 }
    })))
}

pub async fn get_all_sub_accounts(
    pool: web::Data<DbPool>,
    user: UserId,
) -> AppResult<HttpResponse> {
    let accounts = sqlx::query_as::<_, AccountInfo>(
        r#"SELECT id, account_name, phone, email, status, parent_id, created_at
         FROM account WHERE parent_id = $1 AND is_deleted = FALSE"#,
    )
    .bind(user.0)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": accounts,
    })))
}

pub async fn renew_account(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let target_id = path.into_inner();
    sqlx::query(
        "UPDATE account SET status = 1, updated_at = NOW() WHERE id = $1 AND parent_id = $2",
    )
    .bind(target_id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn disable_account(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let target_id = path.into_inner();
    sqlx::query(
        "UPDATE account SET status = 0, updated_at = NOW() WHERE id = $1 AND parent_id = $2",
    )
    .bind(target_id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

pub async fn enable_account(
    pool: web::Data<DbPool>,
    user: UserId,
    path: web::Path<i32>,
) -> AppResult<HttpResponse> {
    let target_id = path.into_inner();
    sqlx::query(
        "UPDATE account SET status = 1, updated_at = NOW() WHERE id = $1 AND parent_id = $2",
    )
    .bind(target_id)
    .bind(user.0)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(serde_json::json!({"success": true})))
}

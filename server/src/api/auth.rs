use actix_web::{web, HttpResponse};
use crate::db::DbPool;
use crate::errors::{AppError, AppResult};
use crate::middleware::auth::JwtConfig;
use crate::models::account::{LoginRequest, LoginResponse};

pub async fn login(
    pool: web::Data<DbPool>,
    jwt: web::Data<JwtConfig>,
    body: web::Json<LoginRequest>,
) -> AppResult<HttpResponse> {
    let account = sqlx::query_as::<_, crate::models::account::Account>(
        r#"SELECT * FROM account
         WHERE (account_name = $1 OR phone = $1 OR email = $1)
           AND is_deleted = FALSE AND status = 1"#,
    )
    .bind(&body.account)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid credentials".into()))?;

    // Verify password
    let parsed_hash = argon2::PasswordHash::new(&account.password_hash)
        .map_err(|_| AppError::Internal("Password hash error".into()))?;

    use argon2::PasswordVerifier;
    argon2::Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed_hash)
        .map_err(|_| AppError::Unauthorized("Invalid credentials".into()))?;

    let token = jwt
        .generate_token(account.id)
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": LoginResponse {
            token,
            account_id: account.id,
            account_name: account.account_name,
        }
    })))
}

pub async fn health_check() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "message": "ok"
    }))
}

pub async fn get_version() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "data": env!("CARGO_PKG_VERSION")
    }))
}
